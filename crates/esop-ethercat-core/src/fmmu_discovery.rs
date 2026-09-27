//! Bounded discovery of the standard ESC FMMU register bank.
//!
//! Discovery records the live register pages reported by the ESC scan. It does
//! not adopt their logical addresses as desired configuration; mapping remains
//! master-owned and may use the verified bank only as a complete reset bound.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::mapping::{ESC_FMMU_BASE, ESC_FMMU_STRIDE, FMMU_IMAGE_LEN, MAX_ESC_FMMUS};
use crate::registers::fixed_address;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FmmuRegisterDescriptor {
    raw: [u8; FMMU_IMAGE_LEN],
}

impl FmmuRegisterDescriptor {
    pub const RESET: Self = Self {
        raw: [0; FMMU_IMAGE_LEN],
    };

    pub const fn from_raw(raw: [u8; FMMU_IMAGE_LEN]) -> Self {
        Self { raw }
    }

    pub const fn raw(self) -> [u8; FMMU_IMAGE_LEN] {
        self.raw
    }

    pub const fn logical_start(self) -> u32 {
        u32::from_le_bytes([self.raw[0], self.raw[1], self.raw[2], self.raw[3]])
    }

    pub const fn length(self) -> u16 {
        u16::from_le_bytes([self.raw[4], self.raw[5]])
    }

    pub const fn logical_start_bit(self) -> u8 {
        self.raw[6]
    }

    pub const fn logical_end_bit(self) -> u8 {
        self.raw[7]
    }

    pub const fn physical_start(self) -> u16 {
        u16::from_le_bytes([self.raw[8], self.raw[9]])
    }

    pub const fn physical_start_bit(self) -> u8 {
        self.raw[10]
    }

    pub const fn fmmu_type(self) -> u8 {
        self.raw[11]
    }

    pub const fn enabled(self) -> bool {
        self.raw[12] & 1 != 0
    }
}

impl Default for FmmuRegisterDescriptor {
    fn default() -> Self {
        Self::RESET
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FmmuRegisterBank {
    position: u16,
    station_address: u16,
    descriptor_count: u8,
    descriptors: [FmmuRegisterDescriptor; MAX_ESC_FMMUS],
}

impl FmmuRegisterBank {
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

    pub fn descriptors(&self) -> &[FmmuRegisterDescriptor] {
        &self.descriptors[..self.descriptor_count as usize]
    }

    pub fn descriptor(&self, index: u8) -> Option<FmmuRegisterDescriptor> {
        self.descriptors().get(index as usize).copied()
    }

    pub(crate) const fn from_parts(
        position: u16,
        station_address: u16,
        descriptor_count: u8,
        descriptors: [FmmuRegisterDescriptor; MAX_ESC_FMMUS],
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
pub enum FmmuRegisterDiscoveryPhase {
    Idle,
    Reading,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FmmuRegisterDiscoveryAction {
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

impl FmmuRegisterDiscoveryAction {
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
pub enum FmmuRegisterDiscoveryProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FmmuRegisterDiscoveryError {
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

pub struct FmmuRegisterDiscoveryController {
    phase: FmmuRegisterDiscoveryPhase,
    generation: u16,
    position: u16,
    station_address: u16,
    descriptor_count: u8,
    descriptor_index: u8,
    descriptors: [FmmuRegisterDescriptor; MAX_ESC_FMMUS],
    discovery_deadline_ns: u64,
    request_timeout_ns: u64,
    pending: Option<FmmuRegisterDiscoveryAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<FmmuRegisterDiscoveryError>,
}

impl FmmuRegisterDiscoveryController {
    pub const fn new() -> Self {
        Self {
            phase: FmmuRegisterDiscoveryPhase::Idle,
            generation: 0,
            position: 0,
            station_address: 0,
            descriptor_count: 0,
            descriptor_index: 0,
            descriptors: [FmmuRegisterDescriptor::RESET; MAX_ESC_FMMUS],
            discovery_deadline_ns: 0,
            request_timeout_ns: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> FmmuRegisterDiscoveryPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<FmmuRegisterDiscoveryAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<FmmuRegisterDiscoveryError> {
        self.last_error
    }

    pub fn bank(&self) -> Option<FmmuRegisterBank> {
        if self.phase != FmmuRegisterDiscoveryPhase::Complete {
            return None;
        }
        Some(FmmuRegisterBank::from_parts(
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
    ) -> Result<(), FmmuRegisterDiscoveryError> {
        if !matches!(
            self.phase,
            FmmuRegisterDiscoveryPhase::Idle
                | FmmuRegisterDiscoveryPhase::Complete
                | FmmuRegisterDiscoveryPhase::Faulted
        ) {
            return Err(FmmuRegisterDiscoveryError::Busy);
        }
        if descriptor_count as usize > MAX_ESC_FMMUS {
            return self.fail(FmmuRegisterDiscoveryError::CapacityExceeded);
        }

        self.generation = generation;
        self.position = position;
        self.station_address = station_address;
        self.descriptor_count = descriptor_count;
        self.descriptor_index = 0;
        self.descriptors = [FmmuRegisterDescriptor::RESET; MAX_ESC_FMMUS];
        self.discovery_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        self.phase = if descriptor_count == 0 {
            FmmuRegisterDiscoveryPhase::Complete
        } else {
            FmmuRegisterDiscoveryPhase::Reading
        };
        Ok(())
    }

    pub fn next_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<FmmuRegisterDiscoveryAction>, FmmuRegisterDiscoveryError> {
        if self.phase == FmmuRegisterDiscoveryPhase::Idle {
            return Err(FmmuRegisterDiscoveryError::NotStarted);
        }
        if matches!(
            self.phase,
            FmmuRegisterDiscoveryPhase::Complete | FmmuRegisterDiscoveryPhase::Faulted
        ) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.discovery_deadline_ns {
            return self.fail(FmmuRegisterDiscoveryError::Timeout);
        }

        let register = ESC_FMMU_BASE
            .checked_add(
                u16::from(self.descriptor_index)
                    .checked_mul(ESC_FMMU_STRIDE)
                    .ok_or(FmmuRegisterDiscoveryError::RegisterAddressOverflow)?,
            )
            .ok_or(FmmuRegisterDiscoveryError::RegisterAddressOverflow)?;
        let action = FmmuRegisterDiscoveryAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            position: self.position,
            station_address: self.station_address,
            descriptor_index: self.descriptor_index,
            operation: RegisterOperation::Read,
            address: fixed_address(self.station_address, register),
            read_len: FMMU_IMAGE_LEN as u16,
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
        action: FmmuRegisterDiscoveryAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<FmmuRegisterDiscoveryProgress, FmmuRegisterDiscoveryError> {
        if self.pending != Some(action) {
            return self.fail(FmmuRegisterDiscoveryError::ActionMismatch);
        }
        if generation != action.generation {
            return self.fail(FmmuRegisterDiscoveryError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(FmmuRegisterDiscoveryError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(FmmuRegisterDiscoveryError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(FmmuRegisterDiscoveryError::PayloadLengthMismatch);
        }

        let mut raw = [0; FMMU_IMAGE_LEN];
        raw.copy_from_slice(payload);
        self.descriptors[self.descriptor_index as usize] = FmmuRegisterDescriptor::from_raw(raw);
        self.descriptor_index += 1;
        self.pending = None;
        if self.descriptor_index == self.descriptor_count {
            self.phase = FmmuRegisterDiscoveryPhase::Complete;
            Ok(FmmuRegisterDiscoveryProgress::Complete)
        } else {
            Ok(FmmuRegisterDiscoveryProgress::Advanced)
        }
    }

    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<FmmuRegisterDiscoveryProgress, FmmuRegisterDiscoveryError> {
        let action = self
            .pending
            .ok_or(FmmuRegisterDiscoveryError::NoPendingAction)?;
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
                    return self.fail(FmmuRegisterDiscoveryError::ActionMismatch);
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
                    return self.fail(FmmuRegisterDiscoveryError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(FmmuRegisterDiscoveryError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(FmmuRegisterDiscoveryError::Control(error));
            }
            Some(_) => {
                return Err(FmmuRegisterDiscoveryError::Control(
                    ControlError::InvalidState,
                ));
            }
            None => {
                return self.fail(FmmuRegisterDiscoveryError::Control(
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
            (Ok(_), Err(error)) => self.fail(FmmuRegisterDiscoveryError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: FmmuRegisterDiscoveryAction,
        now_ns: u64,
    ) -> Result<FmmuRegisterDiscoveryProgress, FmmuRegisterDiscoveryError> {
        if self.pending != Some(action) {
            return self.fail(FmmuRegisterDiscoveryError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(FmmuRegisterDiscoveryError::Timeout);
        }
        self.fail(FmmuRegisterDiscoveryError::Timeout)
    }

    fn fail<T>(
        &mut self,
        error: FmmuRegisterDiscoveryError,
    ) -> Result<T, FmmuRegisterDiscoveryError> {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = FmmuRegisterDiscoveryPhase::Faulted;
        Err(error)
    }
}

impl Default for FmmuRegisterDiscoveryController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(logical_start: u32, physical_start: u16, fmmu_type: u8) -> [u8; 16] {
        let mut raw = [0; 16];
        raw[0..4].copy_from_slice(&logical_start.to_le_bytes());
        raw[4..6].copy_from_slice(&8u16.to_le_bytes());
        raw[6] = 1;
        raw[7] = 6;
        raw[8..10].copy_from_slice(&physical_start.to_le_bytes());
        raw[10] = 2;
        raw[11] = fmmu_type;
        raw[12] = 1;
        raw
    }

    #[test]
    fn discovers_ordered_fmmu_registers_and_decodes_fields() {
        let mut controller = FmmuRegisterDiscoveryController::new();
        controller.start(3, 0x1003, 2, 7, 0, 1_000, 100).unwrap();
        assert_eq!(controller.bank(), None);

        let first = controller.next_action(1).unwrap().unwrap();
        assert_eq!(first.address, fixed_address(0x1003, ESC_FMMU_BASE));
        assert_eq!(first.descriptor_index, 0);
        controller
            .accept(first, 7, &descriptor(0x1122_3344, 0x5566, 2), 1, 2)
            .unwrap();
        assert_eq!(controller.bank(), None);

        let second = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            second.address,
            fixed_address(0x1003, ESC_FMMU_BASE + ESC_FMMU_STRIDE)
        );
        assert_eq!(
            controller.accept(second, 7, &descriptor(0x7788_9900, 0xAABB, 1), 1, 4),
            Ok(FmmuRegisterDiscoveryProgress::Complete)
        );

        let bank = controller.bank().unwrap();
        assert_eq!(bank.position(), 3);
        assert_eq!(bank.station_address(), 0x1003);
        assert_eq!(bank.descriptor_count(), 2);
        let decoded = bank.descriptor(0).unwrap();
        assert_eq!(decoded.logical_start(), 0x1122_3344);
        assert_eq!(decoded.length(), 8);
        assert_eq!(decoded.logical_start_bit(), 1);
        assert_eq!(decoded.logical_end_bit(), 6);
        assert_eq!(decoded.physical_start(), 0x5566);
        assert_eq!(decoded.physical_start_bit(), 2);
        assert_eq!(decoded.fmmu_type(), 2);
        assert!(decoded.enabled());
    }

    #[test]
    fn zero_count_completes_without_action_and_capacity_is_bounded() {
        let mut controller = FmmuRegisterDiscoveryController::new();
        controller.start(0, 0x1000, 0, 1, 0, 10, 5).unwrap();
        assert_eq!(controller.phase(), FmmuRegisterDiscoveryPhase::Complete);
        assert_eq!(controller.next_action(1), Ok(None));
        assert!(controller.bank().unwrap().is_empty());

        assert_eq!(
            controller.start(0, 0x1000, 17, 2, 0, 10, 5),
            Err(FmmuRegisterDiscoveryError::CapacityExceeded)
        );
        assert_eq!(controller.phase(), FmmuRegisterDiscoveryPhase::Faulted);
        assert_eq!(controller.bank(), None);
        assert_eq!(
            controller.last_error(),
            Some(FmmuRegisterDiscoveryError::CapacityExceeded)
        );

        controller.start(2, 0x1002, 1, 3, 20, 100, 10).unwrap();
        let action = controller.next_action(21).unwrap().unwrap();
        assert_eq!(
            controller.accept(action, 3, &[0; FMMU_IMAGE_LEN], 1, 22),
            Ok(FmmuRegisterDiscoveryProgress::Complete)
        );
        assert_eq!(controller.bank().unwrap().position(), 2);
    }

    #[test]
    fn control_pool_completion_preserves_action_ownership() {
        let mut controller = FmmuRegisterDiscoveryController::new();
        controller.start(0, 0x1000, 1, 9, 0, 1_000, 100).unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        let request = pool.get_mut(handle).unwrap();
        request.payload_mut()[..16].copy_from_slice(&descriptor(0x2000, 0x1000, 2));
        request.actual_wkc = 1;
        request.state = RequestState::Complete;
        assert_eq!(
            controller.accept_completed(&mut pool, handle, 2),
            Ok(FmmuRegisterDiscoveryProgress::Complete)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(
            controller.bank().unwrap().descriptor(0).unwrap().raw()[12],
            1
        );

        let mut substituted = FmmuRegisterDiscoveryController::new();
        substituted.start(0, 0x1000, 1, 9, 0, 1_000, 100).unwrap();
        let mut wrong = substituted.next_action(1).unwrap().unwrap();
        wrong.descriptor_index = 1;
        assert_eq!(
            substituted.accept(wrong, 9, &[0; 16], 1, 2),
            Err(FmmuRegisterDiscoveryError::ActionMismatch)
        );
        assert_eq!(substituted.bank(), None);
        assert_eq!(action.operation, RegisterOperation::Read);
    }

    #[test]
    fn fmmu_discovery_faults_without_partial_bank() {
        for (payload, wkc, generation, expected) in [
            (
                &[0u8; 15][..],
                1,
                4,
                FmmuRegisterDiscoveryError::PayloadLengthMismatch,
            ),
            (
                &[0u8; 16][..],
                0,
                4,
                FmmuRegisterDiscoveryError::UnexpectedWorkingCounter,
            ),
            (
                &[0u8; 16][..],
                1,
                5,
                FmmuRegisterDiscoveryError::GenerationMismatch,
            ),
        ] {
            let mut controller = FmmuRegisterDiscoveryController::new();
            controller.start(0, 0x1000, 2, 4, 0, 1_000, 100).unwrap();
            let action = controller.next_action(1).unwrap().unwrap();
            assert_eq!(
                controller.accept(action, generation, payload, wkc, 2),
                Err(expected)
            );
            assert_eq!(controller.bank(), None);
        }

        let mut timed_out = FmmuRegisterDiscoveryController::new();
        timed_out.start(0, 0x1000, 1, 4, 0, 1_000, 10).unwrap();
        let action = timed_out.next_action(1).unwrap().unwrap();
        assert_eq!(
            timed_out.timeout(action, action.deadline_ns),
            Err(FmmuRegisterDiscoveryError::Timeout)
        );
        assert_eq!(timed_out.bank(), None);
    }
}
