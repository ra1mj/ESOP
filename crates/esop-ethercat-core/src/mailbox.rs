//! Allocation-free EtherCAT mailbox transport.
//!
//! The controller owns no thread and performs no waiting. It emits one ESC
//! register action at a time, allowing the caller to schedule mailbox work
//! below the cyclic PDO budget.

use crate::coe::CoeEmergency;
use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::diag::{CoeEmergencyEvent, EmergencySink};
use crate::registers::fixed_address;

pub const MAILBOX_HEADER_LEN: usize = 6;
pub const MAX_MAILBOX_BYTES: usize = MAX_CONTROL_PAYLOAD;
const MAILBOX_LENGTH_MASK: u16 = 0x07FF;
const MAILBOX_COUNTER_MASK: u8 = 0x07;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MailboxProtocol {
    Error = 0x00,
    AoE = 0x01,
    EoE = 0x02,
    CoE = 0x03,
    FoE = 0x04,
    SoE = 0x05,
    VoE = 0x0F,
}

impl MailboxProtocol {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0x00 => Self::Error,
            0x01 => Self::AoE,
            0x02 => Self::EoE,
            0x03 => Self::CoE,
            0x04 => Self::FoE,
            0x05 => Self::SoE,
            0x0F => Self::VoE,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxHeader {
    pub length: u16,
    pub address: u16,
    pub priority: u8,
    pub protocol: MailboxProtocol,
    pub counter: u8,
}

impl MailboxHeader {
    pub fn encode(&self, dst: &mut [u8]) -> Result<(), MailboxError> {
        if dst.len() < MAILBOX_HEADER_LEN {
            return Err(MailboxError::BufferTooSmall);
        }
        if self.length > MAILBOX_LENGTH_MASK {
            return Err(MailboxError::LengthOutOfBounds);
        }
        if self.counter > MAILBOX_COUNTER_MASK {
            return Err(MailboxError::CounterOutOfBounds);
        }
        dst[0..2].copy_from_slice(&self.length.to_le_bytes());
        dst[2..4].copy_from_slice(&self.address.to_le_bytes());
        dst[4] = self.priority;
        dst[5] = self.protocol as u8 | (self.counter << 4);
        Ok(())
    }

    pub fn decode(src: &[u8]) -> Result<Self, MailboxError> {
        if src.len() < MAILBOX_HEADER_LEN {
            return Err(MailboxError::HeaderTruncated);
        }
        let type_counter = src[5];
        let protocol =
            MailboxProtocol::from_u8(type_counter & 0x0F).ok_or(MailboxError::UnknownProtocol)?;
        Ok(Self {
            length: u16::from_le_bytes([src[0], src[1]]) & MAILBOX_LENGTH_MASK,
            address: u16::from_le_bytes([src[2], src[3]]),
            priority: src[4],
            protocol,
            counter: (type_counter >> 4) & MAILBOX_COUNTER_MASK,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxFrame<'a> {
    pub header: MailboxHeader,
    pub payload: &'a [u8],
}

impl<'a> MailboxFrame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, MailboxError> {
        let header = MailboxHeader::decode(bytes)?;
        let end = MAILBOX_HEADER_LEN
            .checked_add(header.length as usize)
            .ok_or(MailboxError::LengthOutOfBounds)?;
        if end > bytes.len() {
            return Err(MailboxError::LengthOutOfBounds);
        }
        Ok(Self {
            header,
            payload: &bytes[MAILBOX_HEADER_LEN..end],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxConfig {
    pub send_address: u16,
    pub send_capacity: u16,
    pub receive_address: u16,
    pub receive_capacity: u16,
    pub poll_interval_ns: u64,
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
    pub retry_policy: MailboxRetryPolicy,
    pub status_bit: Option<MailboxStatusBit>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxStatusBit {
    pub address: u16,
    pub mask: u8,
    pub active_high: bool,
}

impl MailboxStatusBit {
    pub const fn new(address: u16, mask: u8, active_high: bool) -> Self {
        Self {
            address,
            mask,
            active_high,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxRetryPolicy {
    pub max_retries: u8,
    pub retry_delay_ns: u64,
}

impl MailboxRetryPolicy {
    pub const fn disabled() -> Self {
        Self {
            max_retries: 0,
            retry_delay_ns: 0,
        }
    }

    pub const fn new(max_retries: u8, retry_delay_ns: u64) -> Self {
        Self {
            max_retries,
            retry_delay_ns,
        }
    }
}

impl MailboxConfig {
    pub const fn new(
        send_address: u16,
        send_capacity: u16,
        receive_address: u16,
        receive_capacity: u16,
    ) -> Self {
        Self {
            send_address,
            send_capacity,
            receive_address,
            receive_capacity,
            poll_interval_ns: 1_000,
            timeout_ns: 1_000_000_000,
            request_timeout_ns: 1_000_000,
            retry_policy: MailboxRetryPolicy::disabled(),
            status_bit: None,
        }
    }

    pub const fn with_retry_policy(mut self, retry_policy: MailboxRetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub const fn with_status_bit(mut self, status_bit: MailboxStatusBit) -> Self {
        self.status_bit = Some(status_bit);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxPhase {
    Idle,
    Sending,
    CheckingStatus,
    Polling,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub station_address: u16,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; MAX_MAILBOX_BYTES],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl MailboxAction {
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
pub enum MailboxProgress {
    Advanced,
    NoMessage,
    RetryScheduled,
    EmergencyConsumed,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxError {
    Busy,
    NotStarted,
    NoPendingAction,
    ActionMismatch,
    TokenMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    InvalidConfiguration,
    BufferTooSmall,
    LengthOutOfBounds,
    HeaderTruncated,
    UnknownProtocol,
    ProtocolMismatch,
    CounterOutOfBounds,
    CounterMismatch,
    EmergencyUnconsumed,
    Control(ControlError),
}

pub struct MailboxController {
    phase: MailboxPhase,
    config: MailboxConfig,
    generation: u16,
    station_address: u16,
    configuration_deadline_ns: u64,
    protocol: MailboxProtocol,
    counter: u8,
    next_counter: u8,
    send_frame: [u8; MAX_MAILBOX_BYTES],
    send_len: usize,
    response: [u8; MAX_MAILBOX_BYTES],
    response_len: usize,
    response_header: Option<MailboxHeader>,
    poll_due_ns: u64,
    pending: Option<MailboxAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<MailboxError>,
    retry_count: u8,
    last_retry_error: Option<MailboxError>,
    discarded_frames: u32,
}

impl MailboxController {
    pub const fn new() -> Self {
        Self {
            phase: MailboxPhase::Idle,
            config: MailboxConfig::new(0, 0, 0, 0),
            generation: 0,
            station_address: 0,
            configuration_deadline_ns: 0,
            protocol: MailboxProtocol::Error,
            counter: 0,
            next_counter: 1,
            send_frame: [0; MAX_MAILBOX_BYTES],
            send_len: 0,
            response: [0; MAX_MAILBOX_BYTES],
            response_len: 0,
            response_header: None,
            poll_due_ns: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
            retry_count: 0,
            last_retry_error: None,
            discarded_frames: 0,
        }
    }

    pub const fn phase(&self) -> MailboxPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<MailboxAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<MailboxError> {
        self.last_error
    }

    pub const fn retry_count(&self) -> u8 {
        self.retry_count
    }

    pub const fn last_retry_error(&self) -> Option<MailboxError> {
        self.last_retry_error
    }

    pub const fn discarded_frames(&self) -> u32 {
        self.discarded_frames
    }

    pub fn response(&self) -> Option<(&MailboxHeader, &[u8])> {
        if self.phase != MailboxPhase::Complete {
            return None;
        }
        self.response_header
            .as_ref()
            .map(|header| (header, &self.response[..self.response_len]))
    }

    pub fn start(
        &mut self,
        config: MailboxConfig,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        protocol: MailboxProtocol,
        payload: &[u8],
    ) -> Result<(), MailboxError> {
        if !matches!(
            self.phase,
            MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted
        ) {
            return Err(MailboxError::Busy);
        }
        if (config.send_capacity as usize) > MAX_MAILBOX_BYTES
            || (config.receive_capacity as usize) > MAX_MAILBOX_BYTES
            || (config.receive_capacity as usize) < MAILBOX_HEADER_LEN
            || config
                .status_bit
                .is_some_and(|status_bit| status_bit.mask == 0)
            || payload.len() + MAILBOX_HEADER_LEN > (config.send_capacity as usize)
            || payload.len() + MAILBOX_HEADER_LEN > MAX_MAILBOX_BYTES
        {
            return Err(MailboxError::InvalidConfiguration);
        }
        let counter = self.next_counter;
        self.next_counter = if counter >= MAILBOX_COUNTER_MASK {
            1
        } else {
            counter + 1
        };
        let header = MailboxHeader {
            length: payload.len() as u16,
            address: 0,
            priority: 0,
            protocol,
            counter,
        };
        self.send_frame.fill(0);
        header.encode(&mut self.send_frame)?;
        self.send_frame[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + payload.len()]
            .copy_from_slice(payload);
        self.send_len = MAILBOX_HEADER_LEN + payload.len();
        self.config = config;
        self.station_address = station_address;
        self.generation = generation;
        self.configuration_deadline_ns = now_ns.saturating_add(config.timeout_ns);
        self.protocol = protocol;
        self.counter = counter;
        self.response.fill(0);
        self.response_len = 0;
        self.response_header = None;
        self.poll_due_ns = now_ns;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        self.retry_count = 0;
        self.last_retry_error = None;
        self.discarded_frames = 0;
        self.phase = MailboxPhase::Sending;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<MailboxAction>, MailboxError> {
        if self.phase == MailboxPhase::Idle {
            return Err(MailboxError::NotStarted);
        }
        if matches!(self.phase, MailboxPhase::Complete | MailboxPhase::Faulted) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.configuration_deadline_ns {
            return self.fail(MailboxError::Timeout);
        }
        if matches!(
            self.phase,
            MailboxPhase::CheckingStatus | MailboxPhase::Polling
        ) && now_ns < self.poll_due_ns
        {
            return Ok(None);
        }

        let (operation, address, read_len, payload, write_len) = match self.phase {
            MailboxPhase::Sending => (
                RegisterOperation::Write,
                fixed_address(self.station_address, self.config.send_address),
                self.send_len as u16,
                self.send_frame,
                self.send_len as u8,
            ),
            MailboxPhase::CheckingStatus => {
                let status_bit = self
                    .config
                    .status_bit
                    .ok_or(MailboxError::InvalidConfiguration)?;
                (
                    RegisterOperation::Read,
                    fixed_address(self.station_address, status_bit.address),
                    1,
                    [0; MAX_MAILBOX_BYTES],
                    0,
                )
            }
            MailboxPhase::Polling => (
                RegisterOperation::Read,
                fixed_address(self.station_address, self.config.receive_address),
                self.config.receive_capacity,
                [0; MAX_MAILBOX_BYTES],
                0,
            ),
            MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted => return Ok(None),
        };
        let action = MailboxAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            operation,
            address,
            read_len,
            write_payload: payload,
            write_len,
            deadline_ns: now_ns
                .saturating_add(self.config.request_timeout_ns)
                .min(self.configuration_deadline_ns),
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
        action: MailboxAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<MailboxProgress, MailboxError> {
        self.accept_impl(action, generation, payload, working_counter, now_ns, None)
    }

    pub fn accept_with_emergency_sink(
        &mut self,
        action: MailboxAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
        emergency_sink: &dyn EmergencySink,
    ) -> Result<MailboxProgress, MailboxError> {
        self.accept_impl(
            action,
            generation,
            payload,
            working_counter,
            now_ns,
            Some(emergency_sink),
        )
    }

    fn accept_impl(
        &mut self,
        action: MailboxAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
        emergency_sink: Option<&dyn EmergencySink>,
    ) -> Result<MailboxProgress, MailboxError> {
        if self.pending != Some(action) {
            return self.fail(MailboxError::ActionMismatch);
        }
        if action.generation != generation {
            return self.fail(MailboxError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.retry_or_fail(MailboxError::Timeout, now_ns);
        }
        if working_counter != action.expected_wkc {
            return self.retry_or_fail(MailboxError::UnexpectedWorkingCounter, now_ns);
        }
        if payload.len() != action.datagram_len() {
            return self.retry_or_fail(MailboxError::PayloadLengthMismatch, now_ns);
        }

        let progress = match self.phase {
            MailboxPhase::Sending => {
                self.phase = if self.config.status_bit.is_some() {
                    MailboxPhase::CheckingStatus
                } else {
                    MailboxPhase::Polling
                };
                self.poll_due_ns = now_ns;
                MailboxProgress::Advanced
            }
            MailboxPhase::CheckingStatus => {
                let status_bit = self
                    .config
                    .status_bit
                    .ok_or(MailboxError::InvalidConfiguration)?;
                let active = (payload[0] & status_bit.mask) != 0;
                let active = if status_bit.active_high {
                    active
                } else {
                    !active
                };
                self.pending = None;
                if active {
                    self.phase = MailboxPhase::Polling;
                    self.poll_due_ns = now_ns;
                    return Ok(MailboxProgress::Advanced);
                }
                self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                return Ok(MailboxProgress::NoMessage);
            }
            MailboxPhase::Polling => {
                let frame = match MailboxFrame::parse(payload) {
                    Ok(frame) => frame,
                    Err(error) => return self.retry_or_fail(error, now_ns),
                };
                if frame.header.length == 0 {
                    self.pending = None;
                    self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                    return Ok(MailboxProgress::NoMessage);
                }
                if frame.header.protocol == MailboxProtocol::CoE {
                    if let Ok(emergency) = CoeEmergency::parse(frame.payload) {
                        let Some(emergency_sink) = emergency_sink else {
                            return self.retry_or_fail(MailboxError::EmergencyUnconsumed, now_ns);
                        };
                        let event = CoeEmergencyEvent::new(
                            now_ns,
                            self.station_address,
                            self.generation,
                            frame.header.counter,
                            emergency,
                        );
                        let _ = emergency_sink.record(event);
                        self.pending = None;
                        self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                        self.discarded_frames = self.discarded_frames.saturating_add(1);
                        return Ok(MailboxProgress::EmergencyConsumed);
                    }
                }
                if frame.header.protocol != self.protocol {
                    return self.retry_or_fail(MailboxError::ProtocolMismatch, now_ns);
                }
                if frame.header.counter != self.counter {
                    return self.retry_or_fail(MailboxError::CounterMismatch, now_ns);
                }
                self.response[..frame.payload.len()].copy_from_slice(frame.payload);
                self.response_len = frame.payload.len();
                self.response_header = Some(frame.header);
                self.phase = MailboxPhase::Complete;
                MailboxProgress::Complete
            }
            MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted => {
                return self.fail(MailboxError::NoPendingAction);
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
    ) -> Result<MailboxProgress, MailboxError> {
        self.accept_completed_impl(pool, handle, now_ns, None)
    }

    pub fn accept_completed_with_emergency_sink<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
        emergency_sink: &dyn EmergencySink,
    ) -> Result<MailboxProgress, MailboxError> {
        self.accept_completed_impl(pool, handle, now_ns, Some(emergency_sink))
    }

    fn accept_completed_impl<const REQUESTS: usize>(
        &mut self,
        pool: &mut ControlRequestPool<REQUESTS>,
        handle: RequestHandle,
        now_ns: u64,
        emergency_sink: Option<&dyn EmergencySink>,
    ) -> Result<MailboxProgress, MailboxError> {
        let action = match self.pending {
            Some(action) => action,
            None => return self.fail(MailboxError::NoPendingAction),
        };
        let (generation, actual_wkc, response) = match pool.get(handle) {
            Some(request) if request.state == RequestState::Complete => {
                if request.datagram_index != action.datagram_index
                    || request.generation != action.generation
                    || request.address != action.address
                    || request.response_length != action.datagram_len()
                    || request.length != action.datagram_len()
                {
                    let _ = pool.release(handle);
                    return self.fail(MailboxError::ActionMismatch);
                }
                let mut response = [0; MAX_MAILBOX_BYTES];
                response[..request.length].copy_from_slice(request.payload());
                (request.generation, request.actual_wkc, response)
            }
            Some(request) if request.state == RequestState::Failed => {
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                let _ = pool.release(handle);
                if error == ControlError::WorkingCounterMismatch {
                    return self.retry_or_fail(MailboxError::UnexpectedWorkingCounter, now_ns);
                }
                return self.fail(MailboxError::Control(error));
            }
            Some(_) => return Err(MailboxError::Control(ControlError::InvalidState)),
            None => return self.fail(MailboxError::Control(ControlError::InvalidHandle)),
        };
        let progress = self.accept_impl(
            action,
            generation,
            &response[..action.datagram_len()],
            actual_wkc,
            now_ns,
            emergency_sink,
        );
        let release = pool.release(handle);
        match (progress, release) {
            (Ok(progress), Ok(())) => Ok(progress),
            (Ok(_), Err(error)) => self.fail(MailboxError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: MailboxAction,
        now_ns: u64,
    ) -> Result<MailboxProgress, MailboxError> {
        if self.pending != Some(action) {
            return self.fail(MailboxError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(MailboxError::Timeout);
        }
        self.retry_or_fail(MailboxError::Timeout, now_ns)
    }

    fn retry_or_fail(
        &mut self,
        error: MailboxError,
        now_ns: u64,
    ) -> Result<MailboxProgress, MailboxError> {
        if self.retry_count < self.config.retry_policy.max_retries
            && now_ns < self.configuration_deadline_ns
        {
            self.retry_count = self.retry_count.saturating_add(1);
            self.last_retry_error = Some(error);
            self.pending = None;
            self.poll_due_ns = now_ns.saturating_add(self.config.retry_policy.retry_delay_ns);
            self.discarded_frames = self.discarded_frames.saturating_add(1);
            Ok(MailboxProgress::RetryScheduled)
        } else {
            self.fail(error)
        }
    }

    fn fail<T>(&mut self, error: MailboxError) -> Result<T, MailboxError> {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = MailboxPhase::Faulted;
        Err(error)
    }
}

impl Default for MailboxController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coe::{CoeHeader, CoeService};
    use crate::diag::{CoeEmergencyEvent, CoeEmergencyQueue};

    #[test]
    fn mailbox_header_round_trips_protocol_and_counter() {
        let header = MailboxHeader {
            length: 10,
            address: 0x1234,
            priority: 2,
            protocol: MailboxProtocol::CoE,
            counter: 5,
        };
        let mut bytes = [0; MAILBOX_HEADER_LEN];
        header.encode(&mut bytes).unwrap();
        assert_eq!(MailboxHeader::decode(&bytes), Ok(header));
    }

    #[test]
    fn controller_writes_mailbox_then_polls_and_caches_response() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        config.poll_interval_ns = 10;
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[0xAA, 0xBB])
            .unwrap();

        let send = controller.next_action(1).unwrap().unwrap();
        assert_eq!(send.operation, RegisterOperation::Write);
        assert_eq!(send.write_len, 8);
        controller.accept(send, 7, &[0; 8], 1, 2).unwrap();

        let poll = controller.next_action(3).unwrap().unwrap();
        assert_eq!(poll.operation, RegisterOperation::Read);
        assert_eq!(poll.read_len, 32);
        let mut mailbox = [0; 32];
        let header = MailboxHeader {
            length: 2,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::CoE,
            counter: 1,
        };
        header.encode(&mut mailbox).unwrap();
        mailbox[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + 2].copy_from_slice(&[1, 2]);
        assert_eq!(
            controller.accept(poll, 7, &mailbox, 1, 4),
            Ok(MailboxProgress::Complete)
        );
        assert_eq!(controller.phase(), MailboxPhase::Complete);
        assert_eq!(controller.response().unwrap().1, &[1, 2]);
    }

    #[test]
    fn mailbox_counter_mismatch_is_not_published() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = controller.next_action(1).unwrap().unwrap();
        controller.accept(send, 7, &[0; 7], 1, 2).unwrap();
        let poll = controller.next_action(3).unwrap().unwrap();
        let mut mailbox = [0; 32];
        MailboxHeader {
            length: 1,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::CoE,
            counter: 7,
        }
        .encode(&mut mailbox)
        .unwrap();
        assert_eq!(
            controller.accept(poll, 7, &mailbox, 1, 4),
            Err(MailboxError::CounterMismatch)
        );
        assert_eq!(controller.phase(), MailboxPhase::Faulted);
        assert!(controller.response().is_none());
    }

    #[test]
    fn mailbox_retries_protocol_mismatch_without_changing_transaction_counter() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_retry_policy(MailboxRetryPolicy::new(2, 10));
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = controller.next_action(1).unwrap().unwrap();
        controller.accept(send, 7, &[0; 7], 1, 2).unwrap();

        let poll = controller.next_action(3).unwrap().unwrap();
        let mut wrong = [0; 32];
        MailboxHeader {
            length: 1,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::EoE,
            counter: 1,
        }
        .encode(&mut wrong)
        .unwrap();
        assert_eq!(
            controller.accept(poll, 7, &wrong, 1, 4),
            Ok(MailboxProgress::RetryScheduled)
        );
        assert_eq!(controller.retry_count(), 1);
        assert_eq!(
            controller.last_retry_error(),
            Some(MailboxError::ProtocolMismatch)
        );
        assert_eq!(controller.discarded_frames(), 1);
        assert!(controller.next_action(5).unwrap().is_none());

        let retry = controller.next_action(14).unwrap().unwrap();
        let mut response = [0; 32];
        MailboxHeader {
            length: 1,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::CoE,
            counter: 1,
        }
        .encode(&mut response)
        .unwrap();
        response[MAILBOX_HEADER_LEN] = 0xCC;
        assert_eq!(
            controller.accept(retry, 7, &response, 1, 15),
            Ok(MailboxProgress::Complete)
        );
        assert_eq!(controller.response().unwrap().1, &[0xCC]);
    }

    #[test]
    fn mailbox_retry_budget_exhaustion_latches_fault() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_retry_policy(MailboxRetryPolicy::new(1, 10));
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = controller.next_action(1).unwrap().unwrap();
        controller.accept(send, 7, &[0; 7], 1, 2).unwrap();

        let mut wrong = [0; 32];
        MailboxHeader {
            length: 1,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::EoE,
            counter: 1,
        }
        .encode(&mut wrong)
        .unwrap();
        let poll = controller.next_action(3).unwrap().unwrap();
        assert_eq!(
            controller.accept(poll, 7, &wrong, 1, 4),
            Ok(MailboxProgress::RetryScheduled)
        );
        let retry = controller.next_action(14).unwrap().unwrap();
        assert_eq!(
            controller.accept(retry, 7, &wrong, 1, 15),
            Err(MailboxError::ProtocolMismatch)
        );
        assert_eq!(controller.phase(), MailboxPhase::Faulted);
        assert_eq!(controller.retry_count(), 1);
        assert_eq!(
            controller.last_error(),
            Some(MailboxError::ProtocolMismatch)
        );
    }

    #[test]
    fn emergency_is_delivered_without_completing_the_sdo_transaction() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = controller.next_action(1).unwrap().unwrap();
        controller.accept(send, 7, &[0; 7], 1, 2).unwrap();
        let poll = controller.next_action(3).unwrap().unwrap();

        let mut emergency_frame = [0; 32];
        MailboxHeader {
            length: 10,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::CoE,
            counter: 7,
        }
        .encode(&mut emergency_frame)
        .unwrap();
        CoeHeader {
            number: 0,
            service: CoeService::Emergency,
        }
        .encode(&mut emergency_frame[MAILBOX_HEADER_LEN..])
        .unwrap();
        emergency_frame[MAILBOX_HEADER_LEN + 2..MAILBOX_HEADER_LEN + 4]
            .copy_from_slice(&0x2310u16.to_le_bytes());
        emergency_frame[MAILBOX_HEADER_LEN + 4] = 0x81;
        emergency_frame[MAILBOX_HEADER_LEN + 5..MAILBOX_HEADER_LEN + 10]
            .copy_from_slice(&[9, 8, 7, 6, 5]);

        let queue = CoeEmergencyQueue::<2>::new();
        assert_eq!(
            controller.accept_with_emergency_sink(poll, 7, &emergency_frame, 1, 4, &queue),
            Ok(MailboxProgress::EmergencyConsumed)
        );
        assert_eq!(controller.phase(), MailboxPhase::Polling);
        assert_eq!(controller.discarded_frames(), 1);
        assert_eq!(queue.pending(), 1);
        assert_eq!(
            queue.pop(),
            Some(CoeEmergencyEvent::new(
                4,
                0x1000,
                7,
                7,
                CoeEmergency {
                    error_code: 0x2310,
                    error_register: 0x81,
                    manufacturer_data: [9, 8, 7, 6, 5],
                },
            ))
        );
        assert!(controller.next_action(4).unwrap().is_none());
        assert!(controller.next_action(1_004).unwrap().is_some());
    }

    #[test]
    fn status_bit_polling_avoids_mailbox_read_until_status_is_active() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_status_bit(MailboxStatusBit::new(0x1200, 0x08, true));
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = controller.next_action(1).unwrap().unwrap();
        controller.accept(send, 7, &[0; 7], 1, 2).unwrap();

        let status = controller.next_action(3).unwrap().unwrap();
        assert_eq!(status.operation, RegisterOperation::Read);
        assert_eq!(status.address, fixed_address(0x1000, 0x1200));
        assert_eq!(status.read_len, 1);
        assert_eq!(
            controller.accept(status, 7, &[0], 1, 4),
            Ok(MailboxProgress::NoMessage)
        );
        assert_eq!(controller.phase(), MailboxPhase::CheckingStatus);
        assert!(controller.next_action(5).unwrap().is_none());

        let status = controller.next_action(1_004).unwrap().unwrap();
        assert_eq!(
            controller.accept(status, 7, &[0x08], 1, 1_005),
            Ok(MailboxProgress::Advanced)
        );
        assert_eq!(controller.phase(), MailboxPhase::Polling);
        let poll = controller.next_action(1_006).unwrap().unwrap();
        assert_eq!(poll.address, fixed_address(0x1000, 0x1100));
        assert_eq!(poll.read_len, 32);
    }

    #[test]
    fn status_bit_requires_a_nonzero_mask() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_status_bit(MailboxStatusBit::new(0x1200, 0, true));
        let mut controller = MailboxController::new();
        assert_eq!(
            controller.start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1]),
            Err(MailboxError::InvalidConfiguration)
        );
    }

    #[test]
    fn control_pool_wkc_failure_is_translated_into_a_mailbox_retry() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_retry_policy(MailboxRetryPolicy::new(1, 10));
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0; 6], [0; 6])
            .unwrap();
        assert_eq!(
            pool.complete(handle, action.generation, action.address, &[0; 7], 0,),
            Err(ControlError::WorkingCounterMismatch)
        );
        assert_eq!(
            controller.accept_completed(&mut pool, handle, 2),
            Ok(MailboxProgress::RetryScheduled)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(controller.retry_count(), 1);
        assert_eq!(controller.phase(), MailboxPhase::Sending);
    }
}
