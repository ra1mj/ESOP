//! Bounded discovery of the standard ESC SyncManager register bank.
//!
//! Discovery records every live register page reported by the ESC scan. It
//! does not adopt those pages as desired configuration; product mapping remains
//! authoritative and may use the verified bank only as a complete reset bound.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::mapping::{ESC_SYNC_MANAGER_BASE, ESC_SYNC_MANAGER_STRIDE, SYNC_MANAGER_IMAGE_LEN};
use crate::op_only::MAX_ESC_SYNC_MANAGERS;
use crate::registers::fixed_address;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncManagerRegisterDescriptor {
    raw: [u8; SYNC_MANAGER_IMAGE_LEN],
}

impl SyncManagerRegisterDescriptor {
    pub const RESET: Self = Self {
        raw: [0; SYNC_MANAGER_IMAGE_LEN],
    };

    pub const fn from_raw(raw: [u8; SYNC_MANAGER_IMAGE_LEN]) -> Self {
        Self { raw }
    }

    pub const fn raw(self) -> [u8; SYNC_MANAGER_IMAGE_LEN] {
        self.raw
    }

    pub const fn physical_start(self) -> u16 {
        u16::from_le_bytes([self.raw[0], self.raw[1]])
    }

    pub const fn length(self) -> u16 {
        u16::from_le_bytes([self.raw[2], self.raw[3]])
    }

    pub const fn control(self) -> u8 {
        self.raw[4]
    }

    pub const fn status(self) -> u8 {
        self.raw[5]
    }

    pub const fn activation(self) -> u8 {
        self.raw[6]
    }

    pub const fn enabled(self) -> bool {
        self.raw[6] & 1 != 0
    }

    pub const fn pdi_control(self) -> u8 {
        self.raw[7]
    }
}

impl Default for SyncManagerRegisterDescriptor {
    fn default() -> Self {
        Self::RESET
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncManagerRegisterBank {
    position: u16,
    station_address: u16,
    descriptor_count: u8,
    descriptors: [SyncManagerRegisterDescriptor; MAX_ESC_SYNC_MANAGERS],
}

impl SyncManagerRegisterBank {
    pub const fn position(self) -> u16 {
        self.position
    }

    pub const fn station_address(self) -> u16 {
        self.station_address
    }

    pub const fn descriptor_count(self) -> usize {
        self.descriptor_count as usize
    }

    pub const fn is_empty(self) -> bool {
        self.descriptor_count == 0
    }

    pub fn descriptors(&self) -> &[SyncManagerRegisterDescriptor] {
        &self.descriptors[..self.descriptor_count as usize]
    }

    pub fn descriptor(&self, index: u8) -> Option<SyncManagerRegisterDescriptor> {
        self.descriptors().get(index as usize).copied()
    }

    pub(crate) const fn from_parts(
        position: u16,
        station_address: u16,
        descriptor_count: u8,
        descriptors: [SyncManagerRegisterDescriptor; MAX_ESC_SYNC_MANAGERS],
    ) -> Self {
        Self {
            position,
            station_address,
            descriptor_count,
            descriptors,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncManagerRegisterDiscoveryPhase {
    Idle,
    Reading,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncManagerRegisterDiscoveryAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub position: u16,
    pub station_address: u16,
    pub descriptor_index: u8,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl SyncManagerRegisterDiscoveryAction {
    pub const fn payload(self) -> &'static [u8] {
        &[]
    }

    pub const fn datagram_len(self) -> usize {
        self.read_len as usize
    }

    pub const fn response_len(self) -> usize {
        self.read_len as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncManagerRegisterDiscoveryProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncManagerRegisterDiscoveryError {
    Busy,
    NotStarted,
    NoPendingAction,
    CapacityExceeded,
    RegisterAddressOverflow,
    ActionMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    Control(ControlError),
}

pub struct SyncManagerRegisterDiscoveryController {
    phase: SyncManagerRegisterDiscoveryPhase,
    generation: u16,
    position: u16,
    station_address: u16,
    descriptor_count: u8,
    descriptor_index: u8,
    descriptors: [SyncManagerRegisterDescriptor; MAX_ESC_SYNC_MANAGERS],
    discovery_deadline_ns: u64,
    request_timeout_ns: u64,
    pending: Option<SyncManagerRegisterDiscoveryAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<SyncManagerRegisterDiscoveryError>,
}

impl SyncManagerRegisterDiscoveryController {
    pub const fn new() -> Self {
        Self {
            phase: SyncManagerRegisterDiscoveryPhase::Idle,
            generation: 0,
            position: 0,
            station_address: 0,
            descriptor_count: 0,
            descriptor_index: 0,
            descriptors: [SyncManagerRegisterDescriptor::RESET; MAX_ESC_SYNC_MANAGERS],
            discovery_deadline_ns: 0,
            request_timeout_ns: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> SyncManagerRegisterDiscoveryPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<SyncManagerRegisterDiscoveryAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<SyncManagerRegisterDiscoveryError> {
        self.last_error
    }

    pub fn bank(&self) -> Option<SyncManagerRegisterBank> {
        if self.phase != SyncManagerRegisterDiscoveryPhase::Complete {
            return None;
        }
        Some(SyncManagerRegisterBank::from_parts(
            self.position,
            self.station_address,
            self.descriptor_count,
            self.descriptors,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        position: u16,
        station_address: u16,
        descriptor_count: u8,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
    ) -> Result<(), SyncManagerRegisterDiscoveryError> {
        if !matches!(
            self.phase,
            SyncManagerRegisterDiscoveryPhase::Idle
                | SyncManagerRegisterDiscoveryPhase::Complete
                | SyncManagerRegisterDiscoveryPhase::Faulted
        ) {
            return Err(SyncManagerRegisterDiscoveryError::Busy);
        }
        if descriptor_count as usize > MAX_ESC_SYNC_MANAGERS {
            return self.fail(SyncManagerRegisterDiscoveryError::CapacityExceeded);
        }

        self.generation = generation;
        self.position = position;
        self.station_address = station_address;
        self.descriptor_count = descriptor_count;
        self.descriptor_index = 0;
        self.descriptors = [SyncManagerRegisterDescriptor::RESET; MAX_ESC_SYNC_MANAGERS];
        self.discovery_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        self.phase = if descriptor_count == 0 {
            SyncManagerRegisterDiscoveryPhase::Complete
        } else {
            SyncManagerRegisterDiscoveryPhase::Reading
        };
        Ok(())
    }

    pub fn next_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<SyncManagerRegisterDiscoveryAction>, SyncManagerRegisterDiscoveryError> {
        if self.phase == SyncManagerRegisterDiscoveryPhase::Idle {
            return Err(SyncManagerRegisterDiscoveryError::NotStarted);
        }
        if matches!(
            self.phase,
            SyncManagerRegisterDiscoveryPhase::Complete
                | SyncManagerRegisterDiscoveryPhase::Faulted
        ) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.discovery_deadline_ns {
            return self.fail(SyncManagerRegisterDiscoveryError::Timeout);
        }

        let register = ESC_SYNC_MANAGER_BASE
            .checked_add(
                u16::from(self.descriptor_index)
                    .checked_mul(ESC_SYNC_MANAGER_STRIDE)
                    .ok_or(SyncManagerRegisterDiscoveryError::RegisterAddressOverflow)?,
            )
            .ok_or(SyncManagerRegisterDiscoveryError::RegisterAddressOverflow)?;
        let action = SyncManagerRegisterDiscoveryAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            position: self.position,
            station_address: self.station_address,
            descriptor_index: self.descriptor_index,
            operation: RegisterOperation::Read,
            address: fixed_address(self.station_address, register),
            read_len: SYNC_MANAGER_IMAGE_LEN as u16,
            deadline_ns: now_ns
                .saturating_add(self.request_timeout_ns)
                .min(self.discovery_deadline_ns),
            expected_wkc: 1,
        };
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.next_datagram_index = self.next_datagram_index.wrapping_add(1).max(1);
        self.pending = Some(action);
        Ok(Some(action))
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, ControlError> {
        let action = self.pending.ok_or(ControlError::InvalidState)?;
        pool.acquire_with_response_len(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
        )
    }

    pub fn accept(
        &mut self,
        action: SyncManagerRegisterDiscoveryAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<SyncManagerRegisterDiscoveryProgress, SyncManagerRegisterDiscoveryError> {
        if self.pending != Some(action) {
            return self.fail(SyncManagerRegisterDiscoveryError::ActionMismatch);
        }
        if generation != action.generation {
            return self.fail(SyncManagerRegisterDiscoveryError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(SyncManagerRegisterDiscoveryError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(SyncManagerRegisterDiscoveryError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(SyncManagerRegisterDiscoveryError::PayloadLengthMismatch);
        }

        let mut raw = [0; SYNC_MANAGER_IMAGE_LEN];
        raw.copy_from_slice(payload);
        self.descriptors[self.descriptor_index as usize] =
            SyncManagerRegisterDescriptor::from_raw(raw);
        self.descriptor_index += 1;
        self.pending = None;
        if self.descriptor_index == self.descriptor_count {
            self.phase = SyncManagerRegisterDiscoveryPhase::Complete;
            Ok(SyncManagerRegisterDiscoveryProgress::Complete)
        } else {
            Ok(SyncManagerRegisterDiscoveryProgress::Advanced)
        }
    }

    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<SyncManagerRegisterDiscoveryProgress, SyncManagerRegisterDiscoveryError> {
        let action = self
            .pending
            .ok_or(SyncManagerRegisterDiscoveryError::NoPendingAction)?;
        let (generation, actual_wkc, response) = match pool.get(handle) {
            Some(request) if request.state == RequestState::Complete => {
                if !request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) || request.length < action.response_len()
                {
                    let _ = pool.release(handle);
                    return self.fail(SyncManagerRegisterDiscoveryError::ActionMismatch);
                }
                let mut response = [0; MAX_CONTROL_PAYLOAD];
                response[..request.length].copy_from_slice(request.payload());
                (request.generation, request.actual_wkc, response)
            }
            Some(request) if request.state == RequestState::Failed => {
                if !request.matches_action(
                    action.datagram_index,
                    action.generation,
                    action.address,
                    action.operation,
                    action.payload(),
                    action.datagram_len(),
                    action.deadline_ns,
                ) {
                    let _ = pool.release(handle);
                    return self.fail(SyncManagerRegisterDiscoveryError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(SyncManagerRegisterDiscoveryError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(SyncManagerRegisterDiscoveryError::Control(error));
            }
            Some(_) => {
                return Err(SyncManagerRegisterDiscoveryError::Control(
                    ControlError::InvalidState,
                ));
            }
            None => {
                return self.fail(SyncManagerRegisterDiscoveryError::Control(
                    ControlError::InvalidHandle,
                ));
            }
        };

        let progress = self.accept(
            action,
            generation,
            &response[..action.response_len()],
            actual_wkc,
            now_ns,
        );
        let release = pool.release(handle);
        match (progress, release) {
            (Ok(progress), Ok(())) => Ok(progress),
            (Ok(_), Err(error)) => self.fail(SyncManagerRegisterDiscoveryError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: SyncManagerRegisterDiscoveryAction,
        now_ns: u64,
    ) -> Result<SyncManagerRegisterDiscoveryProgress, SyncManagerRegisterDiscoveryError> {
        if self.pending != Some(action) {
            return self.fail(SyncManagerRegisterDiscoveryError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(SyncManagerRegisterDiscoveryError::Timeout);
        }
        self.fail(SyncManagerRegisterDiscoveryError::Timeout)
    }

    fn fail<T>(
        &mut self,
        error: SyncManagerRegisterDiscoveryError,
    ) -> Result<T, SyncManagerRegisterDiscoveryError> {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = SyncManagerRegisterDiscoveryPhase::Faulted;
        Err(error)
    }
}

impl Default for SyncManagerRegisterDiscoveryController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(start: u16, length: u16, control: u8, activation: u8) -> [u8; 8] {
        let mut raw = [0; 8];
        raw[0..2].copy_from_slice(&start.to_le_bytes());
        raw[2..4].copy_from_slice(&length.to_le_bytes());
        raw[4] = control;
        raw[5] = 0x0C;
        raw[6] = activation;
        raw[7] = 0x55;
        raw
    }

    #[test]
    fn discovers_ordered_sync_manager_registers_and_decodes_fields() {
        let mut controller = SyncManagerRegisterDiscoveryController::new();
        controller.start(3, 0x1003, 2, 7, 0, 1_000, 100).unwrap();
        assert_eq!(controller.bank(), None);

        let first = controller.next_action(1).unwrap().unwrap();
        assert_eq!(first.address, fixed_address(0x1003, ESC_SYNC_MANAGER_BASE));
        assert_eq!(first.descriptor_index, 0);
        controller
            .accept(first, 7, &descriptor(0x1122, 0x3344, 0x26, 0x09), 1, 2)
            .unwrap();
        assert_eq!(controller.bank(), None);

        let second = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            second.address,
            fixed_address(0x1003, ESC_SYNC_MANAGER_BASE + ESC_SYNC_MANAGER_STRIDE)
        );
        assert_eq!(
            controller.accept(second, 7, &descriptor(0x5566, 0x7788, 0x22, 0), 1, 4),
            Ok(SyncManagerRegisterDiscoveryProgress::Complete)
        );

        let bank = controller.bank().unwrap();
        assert_eq!(bank.position(), 3);
        assert_eq!(bank.station_address(), 0x1003);
        assert_eq!(bank.descriptor_count(), 2);
        let decoded = bank.descriptor(0).unwrap();
        assert_eq!(decoded.physical_start(), 0x1122);
        assert_eq!(decoded.length(), 0x3344);
        assert_eq!(decoded.control(), 0x26);
        assert_eq!(decoded.status(), 0x0C);
        assert_eq!(decoded.activation(), 0x09);
        assert!(decoded.enabled());
        assert_eq!(decoded.pdi_control(), 0x55);
    }

    #[test]
    fn zero_count_completes_without_action_and_capacity_is_bounded() {
        let mut controller = SyncManagerRegisterDiscoveryController::new();
        controller.start(0, 0x1000, 0, 1, 0, 10, 5).unwrap();
        assert_eq!(
            controller.phase(),
            SyncManagerRegisterDiscoveryPhase::Complete
        );
        assert_eq!(controller.next_action(1), Ok(None));
        assert!(controller.bank().unwrap().is_empty());

        assert_eq!(
            controller.start(0, 0x1000, (MAX_ESC_SYNC_MANAGERS + 1) as u8, 2, 0, 10, 5,),
            Err(SyncManagerRegisterDiscoveryError::CapacityExceeded)
        );
        assert_eq!(
            controller.phase(),
            SyncManagerRegisterDiscoveryPhase::Faulted
        );
        assert_eq!(controller.bank(), None);
    }

    #[test]
    fn control_pool_completion_preserves_action_ownership() {
        let mut controller = SyncManagerRegisterDiscoveryController::new();
        controller.start(0, 0x1000, 1, 9, 0, 1_000, 100).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        let request = pool.get_mut(handle).unwrap();
        request.payload_mut()[..8].copy_from_slice(&descriptor(0x2000, 32, 0x26, 1));
        request.actual_wkc = 1;
        request.state = RequestState::Complete;
        assert_eq!(
            controller.accept_completed(&mut pool, handle, 2),
            Ok(SyncManagerRegisterDiscoveryProgress::Complete)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(
            controller.bank().unwrap().descriptor(0).unwrap().length(),
            32
        );

        let mut substituted = SyncManagerRegisterDiscoveryController::new();
        substituted.start(0, 0x1000, 1, 9, 0, 1_000, 100).unwrap();
        let mut wrong = substituted.next_action(1).unwrap().unwrap();
        wrong.descriptor_index = 1;
        assert_eq!(
            substituted.accept(wrong, 9, &[0; 8], 1, 2),
            Err(SyncManagerRegisterDiscoveryError::ActionMismatch)
        );
        assert_eq!(substituted.bank(), None);
        assert_eq!(action.operation, RegisterOperation::Read);
    }

    #[test]
    fn discovery_faults_without_partial_bank() {
        for (payload, wkc, generation, expected) in [
            (
                &[0u8; 7][..],
                1,
                4,
                SyncManagerRegisterDiscoveryError::PayloadLengthMismatch,
            ),
            (
                &[0u8; 8][..],
                0,
                4,
                SyncManagerRegisterDiscoveryError::UnexpectedWorkingCounter,
            ),
            (
                &[0u8; 8][..],
                1,
                5,
                SyncManagerRegisterDiscoveryError::GenerationMismatch,
            ),
        ] {
            let mut controller = SyncManagerRegisterDiscoveryController::new();
            controller.start(0, 0x1000, 2, 4, 0, 1_000, 100).unwrap();
            let action = controller.next_action(1).unwrap().unwrap();
            assert_eq!(
                controller.accept(action, generation, payload, wkc, 2),
                Err(expected)
            );
            assert_eq!(controller.bank(), None);
        }

        let mut timed_out = SyncManagerRegisterDiscoveryController::new();
        timed_out.start(0, 0x1000, 1, 4, 0, 1_000, 10).unwrap();
        let action = timed_out.next_action(1).unwrap().unwrap();
        assert_eq!(
            timed_out.timeout(action, action.deadline_ns),
            Err(SyncManagerRegisterDiscoveryError::Timeout)
        );
        assert_eq!(timed_out.bank(), None);

        timed_out.start(0, 0x1000, 1, 6, 20, 1_000, 10).unwrap();
        let restarted = timed_out.next_action(21).unwrap().unwrap();
        assert_eq!(restarted.generation, 6);
        assert_eq!(
            timed_out.accept(restarted, 6, &descriptor(0x1200, 16, 0x22, 1), 1, 22),
            Ok(SyncManagerRegisterDiscoveryProgress::Complete)
        );
        let bank = timed_out.bank().unwrap();
        assert_eq!(bank.descriptor_count(), 1);
        assert_eq!(bank.descriptor(0).unwrap().physical_start(), 0x1200);
    }
}
