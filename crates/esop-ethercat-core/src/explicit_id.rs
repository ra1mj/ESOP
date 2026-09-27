//! Bounded EtherCAT Requesting ID transaction controller.
//!
//! The controller is caller-driven and performs the INIT-only sequence through
//! the normal control request pool: request the ID, poll ID Loaded, then read
//! the 16-bit value from the AL Status Code register.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::registers::{
    AL_ID_LOADED_FLAG, AL_ID_REQUEST_FLAG, ESC_AL_CONTROL, ESC_AL_STATUS, ESC_AL_STATUS_CODE,
    fixed_address,
};
use crate::slave::{AL_ERROR_FLAG, EthercatState};

const REQUESTING_ID_REGISTER_LEN: u16 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestingIdPhase {
    Idle,
    WritingRequest,
    ReadingStatus,
    ReadingValue,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestingIdRequest {
    pub station_address: u16,
    pub current_state: EthercatState,
    pub generation: u16,
    pub now_ns: u64,
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestingIdAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub station_address: u16,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; REQUESTING_ID_REGISTER_LEN as usize],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl RequestingIdAction {
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
pub enum RequestingIdProgress {
    RequestWritten,
    Polling,
    ValueRead(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestingIdError {
    Busy,
    NotStarted,
    NoPendingAction,
    InvalidState(EthercatState),
    AlErrorIndication(u16),
    ActionMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    Control(ControlError),
}

pub struct RequestingIdController {
    phase: RequestingIdPhase,
    generation: u16,
    station_address: u16,
    operation_deadline_ns: u64,
    request_timeout_ns: u64,
    pending: Option<RequestingIdAction>,
    value: Option<u16>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<RequestingIdError>,
}

impl RequestingIdController {
    pub const fn new() -> Self {
        Self {
            phase: RequestingIdPhase::Idle,
            generation: 0,
            station_address: 0,
            operation_deadline_ns: 0,
            request_timeout_ns: 0,
            pending: None,
            value: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> RequestingIdPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<RequestingIdAction> {
        self.pending
    }

    pub const fn value(&self) -> Option<u16> {
        if matches!(self.phase, RequestingIdPhase::Complete) {
            self.value
        } else {
            None
        }
    }

    pub const fn last_error(&self) -> Option<RequestingIdError> {
        self.last_error
    }

    pub fn start(&mut self, request: RequestingIdRequest) -> Result<(), RequestingIdError> {
        if !matches!(
            self.phase,
            RequestingIdPhase::Idle | RequestingIdPhase::Complete | RequestingIdPhase::Faulted
        ) {
            return Err(RequestingIdError::Busy);
        }
        if request.current_state != EthercatState::Init {
            return self.fail(RequestingIdError::InvalidState(request.current_state));
        }

        self.generation = request.generation;
        self.station_address = request.station_address;
        self.operation_deadline_ns = request.now_ns.saturating_add(request.timeout_ns);
        self.request_timeout_ns = request.request_timeout_ns;
        self.pending = None;
        self.value = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        self.phase = RequestingIdPhase::WritingRequest;
        Ok(())
    }

    pub fn next_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<RequestingIdAction>, RequestingIdError> {
        if self.phase == RequestingIdPhase::Idle {
            return Err(RequestingIdError::NotStarted);
        }
        if matches!(
            self.phase,
            RequestingIdPhase::Complete | RequestingIdPhase::Faulted
        ) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.operation_deadline_ns {
            return self.fail(RequestingIdError::Timeout);
        }

        let (operation, register, read_len, write_payload, write_len) = match self.phase {
            RequestingIdPhase::WritingRequest => (
                RegisterOperation::Write,
                ESC_AL_CONTROL,
                0,
                ((EthercatState::Init as u16) | AL_ID_REQUEST_FLAG).to_le_bytes(),
                REQUESTING_ID_REGISTER_LEN as u8,
            ),
            RequestingIdPhase::ReadingStatus => (
                RegisterOperation::Read,
                ESC_AL_STATUS,
                REQUESTING_ID_REGISTER_LEN,
                [0; REQUESTING_ID_REGISTER_LEN as usize],
                0,
            ),
            RequestingIdPhase::ReadingValue => (
                RegisterOperation::Read,
                ESC_AL_STATUS_CODE,
                REQUESTING_ID_REGISTER_LEN,
                [0; REQUESTING_ID_REGISTER_LEN as usize],
                0,
            ),
            RequestingIdPhase::Idle | RequestingIdPhase::Complete | RequestingIdPhase::Faulted => {
                return Ok(None);
            }
        };
        let action = RequestingIdAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            operation,
            address: fixed_address(self.station_address, register),
            read_len,
            write_payload,
            write_len,
            deadline_ns: now_ns
                .saturating_add(self.request_timeout_ns)
                .min(self.operation_deadline_ns),
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
        action: RequestingIdAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<RequestingIdProgress, RequestingIdError> {
        if self.pending != Some(action) {
            return self.fail(RequestingIdError::ActionMismatch);
        }
        if generation != action.generation {
            return self.fail(RequestingIdError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(RequestingIdError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(RequestingIdError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(RequestingIdError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            RequestingIdPhase::WritingRequest => {
                self.phase = RequestingIdPhase::ReadingStatus;
                RequestingIdProgress::RequestWritten
            }
            RequestingIdPhase::ReadingStatus => {
                let raw = u16::from_le_bytes([payload[0], payload[1]]);
                let state = EthercatState::from_al_status(raw);
                if state != EthercatState::Init {
                    return self.fail(RequestingIdError::InvalidState(state));
                }
                if raw & AL_ERROR_FLAG != 0 {
                    return self.fail(RequestingIdError::AlErrorIndication(raw));
                }
                if raw & AL_ID_LOADED_FLAG != 0 {
                    self.phase = RequestingIdPhase::ReadingValue;
                }
                RequestingIdProgress::Polling
            }
            RequestingIdPhase::ReadingValue => {
                let value = u16::from_le_bytes([payload[0], payload[1]]);
                self.value = Some(value);
                self.phase = RequestingIdPhase::Complete;
                RequestingIdProgress::ValueRead(value)
            }
            RequestingIdPhase::Idle | RequestingIdPhase::Complete | RequestingIdPhase::Faulted => {
                return self.fail(RequestingIdError::NoPendingAction);
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
    ) -> Result<RequestingIdProgress, RequestingIdError> {
        let action = self.pending.ok_or(RequestingIdError::NoPendingAction)?;
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
                    return self.fail(RequestingIdError::ActionMismatch);
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
                    return self.fail(RequestingIdError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(RequestingIdError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(RequestingIdError::Control(error));
            }
            Some(_) => return Err(RequestingIdError::Control(ControlError::InvalidState)),
            None => {
                return self.fail(RequestingIdError::Control(ControlError::InvalidHandle));
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
            (Ok(_), Err(error)) => self.fail(RequestingIdError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: RequestingIdAction,
        now_ns: u64,
    ) -> Result<RequestingIdProgress, RequestingIdError> {
        if self.pending != Some(action) {
            return self.fail(RequestingIdError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(RequestingIdError::Timeout);
        }
        self.fail(RequestingIdError::Timeout)
    }

    fn fail<T>(&mut self, error: RequestingIdError) -> Result<T, RequestingIdError> {
        if self.last_error.is_none() {
            self.last_error = Some(error);
        }
        self.pending = None;
        self.value = None;
        self.phase = RequestingIdPhase::Faulted;
        Err(self.last_error.unwrap_or(error))
    }
}

impl Default for RequestingIdController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(now_ns: u64) -> RequestingIdRequest {
        RequestingIdRequest {
            station_address: 0x1000,
            current_state: EthercatState::Init,
            generation: 7,
            now_ns,
            timeout_ns: 1_000,
            request_timeout_ns: 100,
        }
    }

    #[test]
    fn requests_polls_and_publishes_the_value() {
        let mut controller = RequestingIdController::new();
        controller.start(request(0)).unwrap();
        assert_eq!(controller.value(), None);

        let write = controller.next_action(1).unwrap().unwrap();
        assert_eq!(write.operation, RegisterOperation::Write);
        assert_eq!(write.address, fixed_address(0x1000, ESC_AL_CONTROL));
        assert_eq!(write.payload(), &[0x21, 0x00]);
        assert_eq!(
            controller.accept(write, 7, &[], 1, 2),
            Ok(RequestingIdProgress::RequestWritten)
        );

        let first_poll = controller.next_action(3).unwrap().unwrap();
        assert_eq!(first_poll.address, fixed_address(0x1000, ESC_AL_STATUS));
        assert_eq!(
            controller.accept(first_poll, 7, &[0x01, 0x00], 1, 4),
            Ok(RequestingIdProgress::Polling)
        );
        assert_eq!(controller.value(), None);

        let loaded_poll = controller.next_action(5).unwrap().unwrap();
        assert_eq!(
            controller.accept(loaded_poll, 7, &[0x21, 0x00], 1, 6),
            Ok(RequestingIdProgress::Polling)
        );
        let value = controller.next_action(7).unwrap().unwrap();
        assert_eq!(value.address, fixed_address(0x1000, ESC_AL_STATUS_CODE));
        assert_eq!(
            controller.accept(value, 7, &[0x41, 0x00], 1, 8),
            Ok(RequestingIdProgress::ValueRead(0x0041))
        );
        assert_eq!(controller.phase(), RequestingIdPhase::Complete);
        assert_eq!(controller.value(), Some(0x0041));
    }

    #[test]
    fn rejects_invalid_state_shape_wkc_generation_action_and_timeout() {
        let mut invalid = RequestingIdController::new();
        assert_eq!(
            invalid.start(RequestingIdRequest {
                current_state: EthercatState::PreOp,
                ..request(0)
            }),
            Err(RequestingIdError::InvalidState(EthercatState::PreOp))
        );

        let mut shape = RequestingIdController::new();
        shape.start(request(0)).unwrap();
        let write = shape.next_action(1).unwrap().unwrap();
        assert_eq!(
            shape.accept(write, 7, &[0], 1, 2),
            Err(RequestingIdError::PayloadLengthMismatch)
        );

        let mut wkc = RequestingIdController::new();
        wkc.start(request(0)).unwrap();
        let write = wkc.next_action(1).unwrap().unwrap();
        assert_eq!(
            wkc.accept(write, 7, &[], 0, 2),
            Err(RequestingIdError::UnexpectedWorkingCounter)
        );

        let mut generation = RequestingIdController::new();
        generation.start(request(0)).unwrap();
        let write = generation.next_action(1).unwrap().unwrap();
        assert_eq!(
            generation.accept(write, 8, &[], 1, 2),
            Err(RequestingIdError::GenerationMismatch)
        );

        let mut action = RequestingIdController::new();
        action.start(request(0)).unwrap();
        let mut write = action.next_action(1).unwrap().unwrap();
        write.address = fixed_address(0x1000, ESC_AL_STATUS);
        assert_eq!(
            action.accept(write, 7, &[], 1, 2),
            Err(RequestingIdError::ActionMismatch)
        );

        let mut timeout = RequestingIdController::new();
        timeout.start(request(0)).unwrap();
        assert_eq!(timeout.next_action(1_000), Err(RequestingIdError::Timeout));
        assert_eq!(timeout.phase(), RequestingIdPhase::Faulted);
    }

    #[test]
    fn restart_clears_value_and_first_fault_is_retained() {
        let mut controller = RequestingIdController::new();
        controller.start(request(0)).unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller.accept(write, 7, &[], 1, 2).unwrap();
        let poll = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(poll, 7, &[0x02, 0x00], 1, 4),
            Err(RequestingIdError::InvalidState(EthercatState::PreOp))
        );
        assert_eq!(
            controller.last_error(),
            Some(RequestingIdError::InvalidState(EthercatState::PreOp))
        );
        assert_eq!(
            controller.accept(poll, 7, &[0x01, 0x00], 1, 5),
            Err(RequestingIdError::InvalidState(EthercatState::PreOp))
        );

        controller.start(request(10)).unwrap();
        assert_eq!(controller.last_error(), None);
        assert_eq!(controller.value(), None);
        assert_eq!(controller.phase(), RequestingIdPhase::WritingRequest);
    }
}
