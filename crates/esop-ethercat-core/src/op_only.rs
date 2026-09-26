//! Bounded SyncManager activation control for ESI/SII `OpOnly` outputs.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::mapping::{ESC_SYNC_MANAGER_BASE, ESC_SYNC_MANAGER_STRIDE};
use crate::registers::fixed_address;

pub const MAX_ESC_SYNC_MANAGERS: usize = 16;
pub const SYNC_MANAGER_ACTIVATION_OFFSET: u16 = 6;
pub const SYNC_MANAGER_ENABLE_FLAG: u8 = 1 << 0;
pub const SYNC_MANAGER_OP_ONLY_FLAG: u8 = 1 << 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpOnlyProfileError {
    IndexOutOfRange,
    DuplicateSyncManager,
    MissingOpOnlyFlag,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpOnlySyncManagerProfile {
    mask: u16,
    activation: [u8; MAX_ESC_SYNC_MANAGERS],
}

impl OpOnlySyncManagerProfile {
    pub const EMPTY: Self = Self {
        mask: 0,
        activation: [0; MAX_ESC_SYNC_MANAGERS],
    };

    pub const fn new() -> Self {
        Self::EMPTY
    }

    pub const fn from_raw(mask: u16, activation: [u8; MAX_ESC_SYNC_MANAGERS]) -> Self {
        Self { mask, activation }
    }

    pub const fn mask(self) -> u16 {
        self.mask
    }

    pub const fn activation_templates(self) -> [u8; MAX_ESC_SYNC_MANAGERS] {
        self.activation
    }

    pub const fn is_empty(self) -> bool {
        self.mask == 0
    }

    pub const fn contains(self, index: u8) -> bool {
        index < MAX_ESC_SYNC_MANAGERS as u8 && self.mask & (1u16 << index) != 0
    }

    pub const fn activation_template(self, index: u8) -> Option<u8> {
        if self.contains(index) {
            Some(self.activation[index as usize])
        } else {
            None
        }
    }

    pub fn add(&mut self, index: u8, activation: u8) -> Result<(), OpOnlyProfileError> {
        if index >= MAX_ESC_SYNC_MANAGERS as u8 {
            return Err(OpOnlyProfileError::IndexOutOfRange);
        }
        let bit = 1u16 << index;
        if self.mask & bit != 0 {
            return Err(OpOnlyProfileError::DuplicateSyncManager);
        }
        if activation & SYNC_MANAGER_OP_ONLY_FLAG == 0 {
            return Err(OpOnlyProfileError::MissingOpOnlyFlag);
        }
        self.mask |= bit;
        self.activation[index as usize] = activation;
        Ok(())
    }

    pub fn validate(self) -> Result<(), OpOnlyProfileError> {
        for index in 0..MAX_ESC_SYNC_MANAGERS as u8 {
            if self.contains(index)
                && self.activation[index as usize] & SYNC_MANAGER_OP_ONLY_FLAG == 0
            {
                return Err(OpOnlyProfileError::MissingOpOnlyFlag);
            }
        }
        Ok(())
    }

    pub const fn activation_for(self, index: u8, enabled: bool) -> Option<u8> {
        let template = match self.activation_template(index) {
            Some(template) => template,
            None => return None,
        };
        Some(if enabled {
            template | SYNC_MANAGER_ENABLE_FLAG
        } else {
            template & !SYNC_MANAGER_ENABLE_FLAG
        })
    }
}

impl Default for OpOnlySyncManagerProfile {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpOnlySyncManagerPhase {
    Idle,
    Writing,
    Verifying,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpOnlySyncManagerAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub station_address: u16,
    pub sync_manager: u8,
    pub enabled: bool,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; 1],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl OpOnlySyncManagerAction {
    pub fn payload(&self) -> &[u8] {
        &self.write_payload[..self.write_len as usize]
    }

    pub const fn datagram_len(self) -> usize {
        let read_len = self.read_len as usize;
        let write_len = self.write_len as usize;
        if read_len > write_len {
            read_len
        } else {
            write_len
        }
    }

    pub const fn response_len(self) -> usize {
        self.read_len as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpOnlySyncManagerProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpOnlySyncManagerError {
    Busy,
    NotStarted,
    NoPendingAction,
    ActionMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    ReadbackMismatch,
    Timeout,
    Profile(OpOnlyProfileError),
    Control(ControlError),
}

pub struct OpOnlySyncManagerController {
    phase: OpOnlySyncManagerPhase,
    generation: u16,
    station_address: u16,
    deadline_ns: u64,
    request_timeout_ns: u64,
    profile: OpOnlySyncManagerProfile,
    enabled: bool,
    sync_manager: u8,
    pending: Option<OpOnlySyncManagerAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<OpOnlySyncManagerError>,
}

impl OpOnlySyncManagerController {
    pub const fn new() -> Self {
        Self {
            phase: OpOnlySyncManagerPhase::Idle,
            generation: 0,
            station_address: 0,
            deadline_ns: 0,
            request_timeout_ns: 0,
            profile: OpOnlySyncManagerProfile::EMPTY,
            enabled: false,
            sync_manager: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> OpOnlySyncManagerPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<OpOnlySyncManagerAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<OpOnlySyncManagerError> {
        self.last_error
    }

    pub fn start(
        &mut self,
        station_address: u16,
        generation: u16,
        deadline_ns: u64,
        request_timeout_ns: u64,
        profile: OpOnlySyncManagerProfile,
        enabled: bool,
    ) -> Result<(), OpOnlySyncManagerError> {
        if !matches!(
            self.phase,
            OpOnlySyncManagerPhase::Idle
                | OpOnlySyncManagerPhase::Complete
                | OpOnlySyncManagerPhase::Faulted
        ) {
            return Err(OpOnlySyncManagerError::Busy);
        }
        profile
            .validate()
            .map_err(OpOnlySyncManagerError::Profile)?;
        self.generation = generation;
        self.station_address = station_address;
        self.deadline_ns = deadline_ns;
        self.request_timeout_ns = request_timeout_ns;
        self.profile = profile;
        self.enabled = enabled;
        self.sync_manager = 0;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        self.phase = if profile.is_empty() {
            OpOnlySyncManagerPhase::Complete
        } else {
            self.sync_manager = next_sync_manager(profile, 0).unwrap_or(0);
            OpOnlySyncManagerPhase::Writing
        };
        Ok(())
    }

    pub fn next_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<OpOnlySyncManagerAction>, OpOnlySyncManagerError> {
        if self.phase == OpOnlySyncManagerPhase::Idle {
            return Err(OpOnlySyncManagerError::NotStarted);
        }
        if matches!(
            self.phase,
            OpOnlySyncManagerPhase::Complete | OpOnlySyncManagerPhase::Faulted
        ) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.deadline_ns {
            return self.fail(OpOnlySyncManagerError::Timeout);
        }

        let expected = self
            .profile
            .activation_for(self.sync_manager, self.enabled)
            .ok_or(OpOnlySyncManagerError::Profile(
                OpOnlyProfileError::IndexOutOfRange,
            ))?;
        let operation = if self.phase == OpOnlySyncManagerPhase::Writing {
            RegisterOperation::Write
        } else {
            RegisterOperation::Read
        };
        let action = OpOnlySyncManagerAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            sync_manager: self.sync_manager,
            enabled: self.enabled,
            operation,
            address: fixed_address(
                self.station_address,
                ESC_SYNC_MANAGER_BASE
                    + self.sync_manager as u16 * ESC_SYNC_MANAGER_STRIDE
                    + SYNC_MANAGER_ACTIVATION_OFFSET,
            ),
            read_len: u16::from(operation == RegisterOperation::Read),
            write_payload: [expected],
            write_len: u8::from(operation == RegisterOperation::Write),
            deadline_ns: now_ns
                .saturating_add(self.request_timeout_ns)
                .min(self.deadline_ns),
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
        action: OpOnlySyncManagerAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<OpOnlySyncManagerProgress, OpOnlySyncManagerError> {
        if self.pending != Some(action) {
            return self.fail(OpOnlySyncManagerError::ActionMismatch);
        }
        if generation != action.generation {
            return self.fail(OpOnlySyncManagerError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(OpOnlySyncManagerError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(OpOnlySyncManagerError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(OpOnlySyncManagerError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            OpOnlySyncManagerPhase::Writing => {
                self.phase = OpOnlySyncManagerPhase::Verifying;
                OpOnlySyncManagerProgress::Advanced
            }
            OpOnlySyncManagerPhase::Verifying => {
                let expected = self
                    .profile
                    .activation_for(self.sync_manager, self.enabled)
                    .ok_or(OpOnlySyncManagerError::Profile(
                        OpOnlyProfileError::IndexOutOfRange,
                    ))?;
                if payload != [expected] {
                    return self.fail(OpOnlySyncManagerError::ReadbackMismatch);
                }
                if let Some(next) = next_sync_manager(self.profile, self.sync_manager + 1) {
                    self.sync_manager = next;
                    self.phase = OpOnlySyncManagerPhase::Writing;
                    OpOnlySyncManagerProgress::Advanced
                } else {
                    self.phase = OpOnlySyncManagerPhase::Complete;
                    OpOnlySyncManagerProgress::Complete
                }
            }
            OpOnlySyncManagerPhase::Idle
            | OpOnlySyncManagerPhase::Complete
            | OpOnlySyncManagerPhase::Faulted => {
                return self.fail(OpOnlySyncManagerError::NoPendingAction);
            }
        };
        self.pending = None;
        Ok(progress)
    }

    pub fn accept_completed<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
    ) -> Result<OpOnlySyncManagerProgress, OpOnlySyncManagerError> {
        let action = self
            .pending
            .ok_or(OpOnlySyncManagerError::NoPendingAction)?;
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
                    return self.fail(OpOnlySyncManagerError::ActionMismatch);
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
                    return self.fail(OpOnlySyncManagerError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(OpOnlySyncManagerError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(OpOnlySyncManagerError::Control(error));
            }
            Some(_) => return Err(OpOnlySyncManagerError::Control(ControlError::InvalidState)),
            None => {
                return self.fail(OpOnlySyncManagerError::Control(ControlError::InvalidHandle));
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
            (Ok(_), Err(error)) => self.fail(OpOnlySyncManagerError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: OpOnlySyncManagerAction,
        now_ns: u64,
    ) -> Result<OpOnlySyncManagerProgress, OpOnlySyncManagerError> {
        if self.pending != Some(action) {
            return self.fail(OpOnlySyncManagerError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(OpOnlySyncManagerError::Timeout);
        }
        self.fail(OpOnlySyncManagerError::Timeout)
    }

    fn fail<T>(&mut self, error: OpOnlySyncManagerError) -> Result<T, OpOnlySyncManagerError> {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = OpOnlySyncManagerPhase::Faulted;
        Err(error)
    }
}

impl Default for OpOnlySyncManagerController {
    fn default() -> Self {
        Self::new()
    }
}

fn next_sync_manager(profile: OpOnlySyncManagerProfile, start: u8) -> Option<u8> {
    (start..MAX_ESC_SYNC_MANAGERS as u8).find(|index| profile.contains(*index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RequestState;

    fn profile() -> OpOnlySyncManagerProfile {
        let mut profile = OpOnlySyncManagerProfile::new();
        profile
            .add(2, SYNC_MANAGER_OP_ONLY_FLAG | SYNC_MANAGER_ENABLE_FLAG)
            .unwrap();
        profile
            .add(
                5,
                SYNC_MANAGER_OP_ONLY_FLAG | SYNC_MANAGER_ENABLE_FLAG | 0x20,
            )
            .unwrap();
        profile
    }

    #[test]
    fn profile_preserves_activation_flags_and_rejects_invalid_entries() {
        let mut value = OpOnlySyncManagerProfile::new();
        assert_eq!(
            value.add(16, SYNC_MANAGER_OP_ONLY_FLAG),
            Err(OpOnlyProfileError::IndexOutOfRange)
        );
        assert_eq!(
            value.add(2, SYNC_MANAGER_ENABLE_FLAG),
            Err(OpOnlyProfileError::MissingOpOnlyFlag)
        );
        value
            .add(
                2,
                SYNC_MANAGER_OP_ONLY_FLAG | SYNC_MANAGER_ENABLE_FLAG | 0x20,
            )
            .unwrap();
        assert_eq!(value.activation_for(2, false), Some(0x28));
        assert_eq!(value.activation_for(2, true), Some(0x29));
        assert_eq!(
            value.add(2, SYNC_MANAGER_OP_ONLY_FLAG),
            Err(OpOnlyProfileError::DuplicateSyncManager)
        );
    }

    #[test]
    fn controller_disables_and_verifies_every_profile_entry() {
        let mut controller = OpOnlySyncManagerController::new();
        controller
            .start(0x1000, 7, 1_000, 100, profile(), false)
            .unwrap();

        let write = controller.next_action(1).unwrap().unwrap();
        assert_eq!(write.sync_manager, 2);
        assert_eq!(write.address, 0x1000_0816);
        assert_eq!(write.payload(), &[SYNC_MANAGER_OP_ONLY_FLAG]);
        controller.accept(write, 7, &[], 1, 2).unwrap();
        let read = controller.next_action(3).unwrap().unwrap();
        assert_eq!(read.operation, RegisterOperation::Read);
        controller
            .accept(read, 7, &[SYNC_MANAGER_OP_ONLY_FLAG], 1, 4)
            .unwrap();

        let write = controller.next_action(5).unwrap().unwrap();
        assert_eq!(write.sync_manager, 5);
        assert_eq!(write.payload(), &[0x28]);
        controller.accept(write, 7, &[], 1, 6).unwrap();
        let read = controller.next_action(7).unwrap().unwrap();
        assert_eq!(
            controller.accept(read, 7, &[0x28], 1, 8),
            Ok(OpOnlySyncManagerProgress::Complete)
        );
        assert_eq!(controller.phase(), OpOnlySyncManagerPhase::Complete);
    }

    #[test]
    fn controller_faults_on_readback_or_deadline_failure() {
        let mut controller = OpOnlySyncManagerController::new();
        controller
            .start(0x1000, 7, 20, 10, profile(), true)
            .unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller.accept(write, 7, &[], 1, 2).unwrap();
        let read = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(read, 7, &[0], 1, 4),
            Err(OpOnlySyncManagerError::ReadbackMismatch)
        );

        controller
            .start(0x1000, 8, 5, 10, profile(), false)
            .unwrap();
        assert_eq!(
            controller.next_action(5),
            Err(OpOnlySyncManagerError::Timeout)
        );
    }

    #[test]
    fn controller_rejects_generation_wkc_and_response_length_mismatches() {
        let mut controller = OpOnlySyncManagerController::new();
        controller
            .start(0x1000, 7, 100, 10, profile(), false)
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        assert_eq!(
            controller.accept(action, 8, &[], 1, 2),
            Err(OpOnlySyncManagerError::GenerationMismatch)
        );

        controller
            .start(0x1000, 7, 100, 10, profile(), false)
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        assert_eq!(
            controller.accept(action, 7, &[], 0, 2),
            Err(OpOnlySyncManagerError::UnexpectedWorkingCounter)
        );

        controller
            .start(0x1000, 7, 100, 10, profile(), false)
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        assert_eq!(
            controller.accept(action, 7, &[0], 1, 2),
            Err(OpOnlySyncManagerError::PayloadLengthMismatch)
        );
    }

    #[test]
    fn completed_control_requests_drive_exact_write_and_readback() {
        let mut single = OpOnlySyncManagerProfile::new();
        single
            .add(2, SYNC_MANAGER_OP_ONLY_FLAG | SYNC_MANAGER_ENABLE_FLAG)
            .unwrap();
        let mut controller = OpOnlySyncManagerController::new();
        controller.start(0x1000, 7, 100, 10, single, false).unwrap();
        controller.next_action(1).unwrap().unwrap();

        let mut pool = ControlRequestPool::<1>::new();
        let write = controller.enqueue_pending(&mut pool).unwrap();
        assert_eq!(pool.get(write).unwrap().length, 1);
        pool.get_mut(write).unwrap().state = RequestState::Complete;
        pool.get_mut(write).unwrap().actual_wkc = 1;
        assert_eq!(
            controller.accept_completed(&mut pool, write, 2),
            Ok(OpOnlySyncManagerProgress::Advanced)
        );
        assert_eq!(pool.in_use(), 0);

        controller.next_action(3).unwrap().unwrap();
        let read = controller.enqueue_pending(&mut pool).unwrap();
        let request = pool.get_mut(read).unwrap();
        request.state = RequestState::Complete;
        request.actual_wkc = 1;
        request.payload_mut()[0] = SYNC_MANAGER_OP_ONLY_FLAG;
        assert_eq!(
            controller.accept_completed(&mut pool, read, 4),
            Ok(OpOnlySyncManagerProgress::Complete)
        );
        assert_eq!(pool.in_use(), 0);
    }
}
