//! Bounded EtherCAT State Machine transition controller.
//!
//! One controller performs one legal state transition at a time. If a slave
//! needs several transitions (INIT -> PREOP -> SAFEOP -> OP), the caller
//! restarts the controller for the next step after each successful result.

use crate::control::{ControlError, ControlRequestPool, RegisterOperation, RequestHandle};
use crate::registers::{AL_STATUS_WITH_CODE_LEN, ESC_AL_CONTROL, ESC_AL_STATUS, fixed_address};
use crate::slave::{AL_ERROR_FLAG, AlStatus, EthercatState, next_state};

const ACTION_PAYLOAD_LEN: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlPhase {
    Idle,
    WritingControl,
    ReadingStatus,
    WritingErrorAcknowledge,
    ReadingErrorAcknowledge,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlError {
    Busy,
    NotStarted,
    NoPendingAction,
    TokenMismatch,
    GenerationMismatch,
    InvalidTransition,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    AlErrorCode(u16),
    InvalidResponse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlProgress {
    ControlWritten,
    Polling,
    Reached(EthercatState),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlErrorAcknowledgePolicy {
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlErrorAcknowledgeStatus {
    NotAttempted,
    InProgress,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlFaultRecord {
    pub status: AlStatus,
    pub requested_state: EthercatState,
    pub acknowledgement: AlErrorAcknowledgeStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub station_address: u16,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; ACTION_PAYLOAD_LEN],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl AlAction {
    pub fn payload(&self) -> &[u8] {
        &self.write_payload[..self.write_len as usize]
    }

    pub const fn datagram_len(&self) -> usize {
        let read_len = self.read_len as usize;
        let write_len = self.write_len as usize;
        if read_len > write_len {
            read_len
        } else {
            write_len
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlTransitionRequest {
    pub station_address: u16,
    pub current_state: EthercatState,
    pub requested_state: EthercatState,
    pub generation: u16,
    pub now_ns: u64,
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
}

pub struct AlTransitionController {
    phase: AlPhase,
    generation: u16,
    station_address: u16,
    current_state: EthercatState,
    requested_state: EthercatState,
    expected_state: EthercatState,
    observed_status: AlStatus,
    scan_deadline_ns: u64,
    request_timeout_ns: u64,
    pending: Option<AlAction>,
    error_acknowledge_policy: AlErrorAcknowledgePolicy,
    fault_record: Option<AlFaultRecord>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<AlError>,
}

impl AlTransitionController {
    pub const fn new() -> Self {
        Self {
            phase: AlPhase::Idle,
            generation: 0,
            station_address: 0,
            current_state: EthercatState::Unknown,
            requested_state: EthercatState::Unknown,
            expected_state: EthercatState::Unknown,
            observed_status: AlStatus {
                state: EthercatState::Unknown,
                error: false,
                raw: 0,
                code: 0,
            },
            scan_deadline_ns: 0,
            request_timeout_ns: 0,
            pending: None,
            error_acknowledge_policy: AlErrorAcknowledgePolicy::Disabled,
            fault_record: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> AlPhase {
        self.phase
    }

    pub const fn expected_state(&self) -> EthercatState {
        self.expected_state
    }

    pub const fn observed_status(&self) -> AlStatus {
        self.observed_status
    }

    pub const fn pending(&self) -> Option<AlAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<AlError> {
        self.last_error
    }

    pub const fn fault_record(&self) -> Option<AlFaultRecord> {
        self.fault_record
    }

    pub fn start(&mut self, request: AlTransitionRequest) -> Result<(), AlError> {
        let status = AlStatus::new(request.current_state as u16, 0);
        self.start_with_status(request, status, AlErrorAcknowledgePolicy::Disabled)
    }

    pub fn start_with_status(
        &mut self,
        request: AlTransitionRequest,
        observed_status: AlStatus,
        error_acknowledge_policy: AlErrorAcknowledgePolicy,
    ) -> Result<(), AlError> {
        if !matches!(
            self.phase,
            AlPhase::Idle | AlPhase::Complete | AlPhase::Faulted
        ) {
            return Err(AlError::Busy);
        }
        if matches!(request.current_state, EthercatState::Unknown)
            || matches!(request.requested_state, EthercatState::Unknown)
            || matches!(observed_status.state, EthercatState::Unknown)
            || observed_status.state != request.current_state
        {
            return Err(AlError::InvalidTransition);
        }
        let expected_state = if request.current_state == request.requested_state {
            request.current_state
        } else {
            next_state(request.current_state, request.requested_state)
                .ok_or(AlError::InvalidTransition)?
        };
        self.generation = request.generation;
        self.station_address = request.station_address;
        self.current_state = request.current_state;
        self.requested_state = request.requested_state;
        self.expected_state = expected_state;
        self.observed_status = observed_status;
        self.scan_deadline_ns = request.now_ns.saturating_add(request.timeout_ns);
        self.request_timeout_ns = request.request_timeout_ns;
        self.pending = None;
        self.error_acknowledge_policy = error_acknowledge_policy;
        self.fault_record = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        if observed_status.error {
            return self.begin_error_acknowledgement(observed_status);
        }
        self.phase = if request.current_state == request.requested_state {
            AlPhase::Complete
        } else {
            AlPhase::WritingControl
        };
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<AlAction>, AlError> {
        if self.phase == AlPhase::Idle {
            return Err(AlError::NotStarted);
        }
        if matches!(self.phase, AlPhase::Complete | AlPhase::Faulted) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.scan_deadline_ns {
            self.fail(AlError::Timeout);
            return Err(AlError::Timeout);
        }

        let (operation, address, read_len, payload, write_len) = match self.phase {
            AlPhase::WritingControl => (
                RegisterOperation::Write,
                fixed_address(self.station_address, ESC_AL_CONTROL),
                0,
                (self.expected_state as u16).to_le_bytes(),
                2,
            ),
            AlPhase::WritingErrorAcknowledge => {
                let fault = self.fault_record.ok_or(AlError::InvalidResponse)?;
                (
                    RegisterOperation::Write,
                    fixed_address(self.station_address, ESC_AL_CONTROL),
                    0,
                    ((fault.status.state as u16) | AL_ERROR_FLAG).to_le_bytes(),
                    2,
                )
            }
            AlPhase::ReadingStatus | AlPhase::ReadingErrorAcknowledge => (
                RegisterOperation::Read,
                fixed_address(self.station_address, ESC_AL_STATUS),
                AL_STATUS_WITH_CODE_LEN,
                [0; ACTION_PAYLOAD_LEN],
                0,
            ),
            AlPhase::Idle | AlPhase::Complete | AlPhase::Faulted => return Ok(None),
        };
        let deadline_ns = now_ns
            .saturating_add(self.request_timeout_ns)
            .min(self.scan_deadline_ns);
        let action = AlAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            operation,
            address,
            read_len,
            write_payload: payload,
            write_len,
            deadline_ns,
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
        token: u8,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<AlProgress, AlError> {
        let action = self.pending.ok_or(AlError::NoPendingAction)?;
        if action.token != token {
            return Err(AlError::TokenMismatch);
        }
        if action.generation != generation {
            self.fail(AlError::GenerationMismatch);
            return Err(AlError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            self.fail(AlError::Timeout);
            return Err(AlError::Timeout);
        }
        if working_counter != action.expected_wkc {
            self.fail(AlError::UnexpectedWorkingCounter);
            return Err(AlError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.read_len as usize {
            self.fail(AlError::PayloadLengthMismatch);
            return Err(AlError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            AlPhase::WritingControl => {
                self.phase = AlPhase::ReadingStatus;
                AlProgress::ControlWritten
            }
            AlPhase::WritingErrorAcknowledge => {
                self.phase = AlPhase::ReadingErrorAcknowledge;
                AlProgress::ControlWritten
            }
            AlPhase::ReadingStatus => {
                let status = decode_status(payload);
                self.observed_status = status;
                if status.error {
                    self.pending = None;
                    self.begin_error_acknowledgement(status)?;
                    return Ok(AlProgress::Polling);
                }
                if status.state == self.expected_state {
                    self.phase = AlPhase::Complete;
                    AlProgress::Reached(status.state)
                } else {
                    AlProgress::Polling
                }
            }
            AlPhase::ReadingErrorAcknowledge => {
                let status = decode_status(payload);
                self.observed_status = status;
                if status.error {
                    AlProgress::Polling
                } else {
                    let mut fault = self.fault_record.ok_or(AlError::InvalidResponse)?;
                    if status.state != fault.status.state {
                        self.fail(AlError::InvalidResponse);
                        return Err(AlError::InvalidResponse);
                    }
                    fault.acknowledgement = AlErrorAcknowledgeStatus::Complete;
                    self.fault_record = Some(fault);
                    let error = AlError::AlErrorCode(fault.status.code);
                    self.fail(error);
                    return Err(error);
                }
            }
            AlPhase::Idle | AlPhase::Complete | AlPhase::Faulted => {
                return Err(AlError::InvalidResponse);
            }
        };
        self.pending = None;
        Ok(progress)
    }

    pub fn timeout(&mut self, token: u8, now_ns: u64) -> Result<(), AlError> {
        let action = self.pending.ok_or(AlError::NoPendingAction)?;
        if action.token != token {
            return Err(AlError::TokenMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(AlError::Timeout);
        }
        self.fail(AlError::Timeout);
        Err(AlError::Timeout)
    }

    fn fail(&mut self, error: AlError) {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = AlPhase::Faulted;
    }

    fn begin_error_acknowledgement(&mut self, status: AlStatus) -> Result<(), AlError> {
        if self.fault_record.is_none() {
            self.fault_record = Some(AlFaultRecord {
                status,
                requested_state: self.requested_state,
                acknowledgement: AlErrorAcknowledgeStatus::NotAttempted,
            });
        }
        if self.error_acknowledge_policy == AlErrorAcknowledgePolicy::Disabled {
            let error = AlError::AlErrorCode(status.code);
            self.fail(error);
            return Err(error);
        }
        let mut fault = self.fault_record.ok_or(AlError::InvalidResponse)?;
        fault.acknowledgement = AlErrorAcknowledgeStatus::InProgress;
        self.fault_record = Some(fault);
        self.phase = AlPhase::WritingErrorAcknowledge;
        Ok(())
    }
}

fn decode_status(payload: &[u8]) -> AlStatus {
    let raw = u16::from_le_bytes([payload[0], payload[1]]);
    let code = u16::from_le_bytes([payload[4], payload[5]]);
    AlStatus::new(raw, code)
}

impl Default for AlTransitionController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: EthercatState, code: u16) -> [u8; 6] {
        let mut bytes = [0; 6];
        bytes[0..2].copy_from_slice(&(state as u16).to_le_bytes());
        bytes[4..6].copy_from_slice(&code.to_le_bytes());
        bytes
    }

    #[test]
    fn controller_performs_one_legal_step_and_polls_until_reached() {
        let mut controller = AlTransitionController::new();
        controller
            .start(AlTransitionRequest {
                station_address: 0x1000,
                current_state: EthercatState::Init,
                requested_state: EthercatState::Op,
                generation: 9,
                now_ns: 0,
                timeout_ns: 1_000,
                request_timeout_ns: 100,
            })
            .unwrap();
        assert_eq!(controller.expected_state(), EthercatState::PreOp);

        let write = controller.next_action(1).unwrap().unwrap();
        assert_eq!(write.operation, RegisterOperation::Write);
        assert_eq!(write.address as u16, ESC_AL_CONTROL);
        assert_eq!(write.payload(), &[0x02, 0x00]);
        controller.accept(write.token, 9, &[], 1, 2).unwrap();

        let read = controller.next_action(3).unwrap().unwrap();
        assert_eq!(read.address as u16, ESC_AL_STATUS);
        assert_eq!(
            controller.accept(read.token, 9, &status(EthercatState::Init, 0), 1, 4),
            Ok(AlProgress::Polling)
        );
        let read = controller.next_action(5).unwrap().unwrap();
        assert_eq!(
            controller.accept(read.token, 9, &status(EthercatState::PreOp, 0), 1, 6),
            Ok(AlProgress::Reached(EthercatState::PreOp))
        );
        assert_eq!(controller.phase(), AlPhase::Complete);
    }

    #[test]
    fn al_error_latches_fault_and_rejects_auto_recovery() {
        let mut controller = AlTransitionController::new();
        controller
            .start(AlTransitionRequest {
                station_address: 0x1000,
                current_state: EthercatState::SafeOp,
                requested_state: EthercatState::Op,
                generation: 1,
                now_ns: 0,
                timeout_ns: 1_000,
                request_timeout_ns: 100,
            })
            .unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller.accept(write.token, 1, &[], 1, 2).unwrap();
        let read = controller.next_action(3).unwrap().unwrap();
        let mut error_status = status(EthercatState::SafeOp, 0x001B);
        error_status[0] |= 0x10;
        assert_eq!(
            controller.accept(read.token, 1, &error_status, 1, 4),
            Err(AlError::AlErrorCode(0x001B))
        );
        assert_eq!(controller.phase(), AlPhase::Faulted);
        assert_eq!(controller.next_action(5), Ok(None));
        assert_eq!(
            controller.fault_record(),
            Some(AlFaultRecord {
                status: AlStatus::new(0x14, 0x001B),
                requested_state: EthercatState::Op,
                acknowledgement: AlErrorAcknowledgeStatus::NotAttempted,
            })
        );
    }

    #[test]
    fn enabled_policy_acknowledges_error_but_does_not_retry_transition() {
        let mut controller = AlTransitionController::new();
        controller
            .start_with_status(
                AlTransitionRequest {
                    station_address: 0x1000,
                    current_state: EthercatState::SafeOp,
                    requested_state: EthercatState::Op,
                    generation: 2,
                    now_ns: 0,
                    timeout_ns: 1_000,
                    request_timeout_ns: 100,
                },
                AlStatus::new(EthercatState::SafeOp as u16, 0),
                AlErrorAcknowledgePolicy::Enabled,
            )
            .unwrap();

        let transition = controller.next_action(1).unwrap().unwrap();
        assert_eq!(transition.payload(), &[EthercatState::Op as u8, 0]);
        controller.accept(transition.token, 2, &[], 1, 2).unwrap();
        let status_read = controller.next_action(3).unwrap().unwrap();
        let mut error_status = status(EthercatState::SafeOp, 0x001B);
        error_status[0] |= AL_ERROR_FLAG as u8;
        assert_eq!(
            controller.accept(status_read.token, 2, &error_status, 1, 4),
            Ok(AlProgress::Polling)
        );

        let acknowledge = controller.next_action(5).unwrap().unwrap();
        assert_eq!(controller.phase(), AlPhase::WritingErrorAcknowledge);
        assert_eq!(acknowledge.payload(), &[0x14, 0x00]);
        controller.accept(acknowledge.token, 2, &[], 1, 6).unwrap();
        let verify = controller.next_action(7).unwrap().unwrap();
        assert_eq!(
            controller.accept(verify.token, 2, &status(EthercatState::SafeOp, 0), 1, 8,),
            Err(AlError::AlErrorCode(0x001B))
        );
        assert_eq!(controller.phase(), AlPhase::Faulted);
        assert_eq!(
            controller.fault_record().unwrap().acknowledgement,
            AlErrorAcknowledgeStatus::Complete
        );
        assert_eq!(controller.next_action(9), Ok(None));
    }

    #[test]
    fn acknowledgement_timeout_preserves_first_al_fault() {
        let mut controller = AlTransitionController::new();
        let mut initial_status = status(EthercatState::PreOp, 0x0011);
        initial_status[0] |= AL_ERROR_FLAG as u8;
        controller
            .start_with_status(
                AlTransitionRequest {
                    station_address: 0x1000,
                    current_state: EthercatState::PreOp,
                    requested_state: EthercatState::SafeOp,
                    generation: 5,
                    now_ns: 0,
                    timeout_ns: 1_000,
                    request_timeout_ns: 10,
                },
                AlStatus::new(
                    u16::from_le_bytes([initial_status[0], initial_status[1]]),
                    0x0011,
                ),
                AlErrorAcknowledgePolicy::Enabled,
            )
            .unwrap();

        let acknowledge = controller.next_action(1).unwrap().unwrap();
        assert_eq!(acknowledge.payload(), &[0x12, 0x00]);
        assert_eq!(
            controller.timeout(acknowledge.token, acknowledge.deadline_ns),
            Err(AlError::Timeout)
        );
        assert_eq!(controller.last_error(), Some(AlError::Timeout));
        assert_eq!(
            controller.fault_record(),
            Some(AlFaultRecord {
                status: AlStatus::new(0x12, 0x0011),
                requested_state: EthercatState::SafeOp,
                acknowledgement: AlErrorAcknowledgeStatus::InProgress,
            })
        );
    }

    #[test]
    fn invalid_acknowledgement_response_preserves_first_al_fault() {
        let mut controller = AlTransitionController::new();
        controller
            .start_with_status(
                AlTransitionRequest {
                    station_address: 0x1000,
                    current_state: EthercatState::PreOp,
                    requested_state: EthercatState::SafeOp,
                    generation: 6,
                    now_ns: 0,
                    timeout_ns: 1_000,
                    request_timeout_ns: 100,
                },
                AlStatus::new(0x12, 0x0011),
                AlErrorAcknowledgePolicy::Enabled,
            )
            .unwrap();

        let acknowledge = controller.next_action(1).unwrap().unwrap();
        controller.accept(acknowledge.token, 6, &[], 1, 2).unwrap();
        let verify = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(verify.token, 6, &status(EthercatState::SafeOp, 0), 1, 4),
            Err(AlError::InvalidResponse)
        );
        assert_eq!(controller.last_error(), Some(AlError::InvalidResponse));
        assert_eq!(
            controller.fault_record(),
            Some(AlFaultRecord {
                status: AlStatus::new(0x12, 0x0011),
                requested_state: EthercatState::SafeOp,
                acknowledgement: AlErrorAcknowledgeStatus::InProgress,
            })
        );
    }

    #[test]
    fn already_reached_state_is_idempotent_but_unknown_is_rejected() {
        let mut controller = AlTransitionController::new();
        controller
            .start(AlTransitionRequest {
                station_address: 0x1000,
                current_state: EthercatState::PreOp,
                requested_state: EthercatState::PreOp,
                generation: 1,
                now_ns: 0,
                timeout_ns: 100,
                request_timeout_ns: 10,
            })
            .unwrap();
        assert_eq!(controller.phase(), AlPhase::Complete);
        assert_eq!(controller.next_action(1), Ok(None));

        assert_eq!(
            controller.start(AlTransitionRequest {
                station_address: 0x1000,
                current_state: EthercatState::Unknown,
                requested_state: EthercatState::PreOp,
                generation: 1,
                now_ns: 0,
                timeout_ns: 100,
                request_timeout_ns: 10,
            }),
            Err(AlError::InvalidTransition)
        );
    }
}
