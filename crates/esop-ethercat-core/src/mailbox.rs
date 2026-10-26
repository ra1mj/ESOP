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
use crate::diag::{CoeEmergencyEvent, CoeEmergencyQueue, EmergencySink};
use crate::mapping::{
    ESC_SYNC_MANAGER_BASE, ESC_SYNC_MANAGER_STRIDE, FmmuConfig, SYNC_MANAGER_STATUS_OFFSET,
};
use crate::op_only::MAX_ESC_SYNC_MANAGERS;
use crate::registers::fixed_address;
use crate::rx_index::RxWorkingCounterPolicy;

pub const MAILBOX_HEADER_LEN: usize = 6;
pub const MAX_MAILBOX_BYTES: usize = MAX_CONTROL_PAYLOAD;
pub const MAILBOX_FULL_STATUS_MASK: u8 = 0x08;
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
    pub send_sync_manager: u8,
    pub receive_sync_manager: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxDirection {
    Send,
    Receive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxConfigError {
    AddressZero(MailboxDirection),
    CapacityTooSmall(MailboxDirection),
    CapacityExceeded(MailboxDirection),
    AddressRangeOverflow(MailboxDirection),
    AddressRangeOverlap,
    InvalidStatusBit,
    InvalidSyncManager,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxReceiveSyncManagerError {
    IndexOutOfBounds(u8),
    AddressMismatch { expected: u16, actual: u16 },
    CapacityMismatch { expected: u16, actual: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxStatusBit {
    pub address: u16,
    pub mask: u8,
    pub active_high: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxMappedStatusBit {
    pub domain_id: u8,
    pub domain_bit_offset: u32,
    pub max_age_cycles: u32,
    pub fmmu: FmmuConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxMappedStatusError {
    ZeroMaximumAge,
    LogicalAddressOverflow,
    InvalidFmmu,
    StatusBitMismatch,
    InputOutOfBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxMappedStatusUnavailable {
    DomainBusy,
    MissingDomain,
    InvalidQuality,
    NeverCommitted,
    Stale,
    InputOutOfBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxMappedStatusObservation {
    Active,
    Inactive,
    Unavailable(MailboxMappedStatusUnavailable),
}

impl MailboxMappedStatusBit {
    pub const fn new(
        domain_id: u8,
        domain_bit_offset: u32,
        max_age_cycles: u32,
        fmmu: FmmuConfig,
    ) -> Self {
        Self {
            domain_id,
            domain_bit_offset,
            max_age_cycles,
            fmmu,
        }
    }

    pub fn validate(
        self,
        domain_logical_start: u32,
        status_bit: MailboxStatusBit,
    ) -> Result<(), MailboxMappedStatusError> {
        if self.max_age_cycles == 0 {
            return Err(MailboxMappedStatusError::ZeroMaximumAge);
        }
        let logical_start = domain_logical_start
            .checked_add(self.domain_bit_offset / 8)
            .ok_or(MailboxMappedStatusError::LogicalAddressOverflow)?;
        let logical_bit = (self.domain_bit_offset % 8) as u8;
        if self.fmmu.length != 1
            || self.fmmu.logical_start != logical_start
            || self.fmmu.logical_start_bit != logical_bit
            || self.fmmu.logical_end_bit != logical_bit
            || self.fmmu.fmmu_type != 1
            || !self.fmmu.enable
        {
            return Err(MailboxMappedStatusError::InvalidFmmu);
        }
        if status_bit.mask.count_ones() != 1
            || !status_bit.active_high
            || self.fmmu.physical_start != status_bit.address
            || self.fmmu.physical_start_bit != status_bit.mask.trailing_zeros() as u8
        {
            return Err(MailboxMappedStatusError::StatusBitMismatch);
        }
        Ok(())
    }

    pub fn read(self, input: &[u8]) -> Result<bool, MailboxMappedStatusError> {
        let byte = usize::try_from(self.domain_bit_offset / 8)
            .map_err(|_| MailboxMappedStatusError::InputOutOfBounds)?;
        let bit = (self.domain_bit_offset % 8) as u8;
        let value = input
            .get(byte)
            .ok_or(MailboxMappedStatusError::InputOutOfBounds)?;
        Ok(value & (1 << bit) != 0)
    }
}

impl MailboxStatusBit {
    pub const fn new(address: u16, mask: u8, active_high: bool) -> Self {
        Self {
            address,
            mask,
            active_high,
        }
    }

    pub const fn sync_manager_mailbox_full(sync_manager: u8) -> Self {
        Self::new(
            ESC_SYNC_MANAGER_BASE
                + sync_manager as u16 * ESC_SYNC_MANAGER_STRIDE
                + SYNC_MANAGER_STATUS_OFFSET,
            MAILBOX_FULL_STATUS_MASK,
            true,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxReceiveSyncManager {
    pub index: u8,
    pub physical_start: u16,
    pub length: u16,
    pub control: u8,
}

impl MailboxReceiveSyncManager {
    pub const fn new(index: u8, physical_start: u16, length: u16, control: u8) -> Self {
        Self {
            index,
            physical_start,
            length,
            control,
        }
    }

    pub const fn status_bit(self) -> MailboxStatusBit {
        MailboxStatusBit::sync_manager_mailbox_full(self.index)
    }

    pub fn validate_for(
        self,
        mailbox: MailboxConfig,
    ) -> Result<(), MailboxReceiveSyncManagerError> {
        if usize::from(self.index) >= MAX_ESC_SYNC_MANAGERS {
            return Err(MailboxReceiveSyncManagerError::IndexOutOfBounds(self.index));
        }
        if self.physical_start != mailbox.receive_address {
            return Err(MailboxReceiveSyncManagerError::AddressMismatch {
                expected: mailbox.receive_address,
                actual: self.physical_start,
            });
        }
        if self.length != mailbox.receive_capacity {
            return Err(MailboxReceiveSyncManagerError::CapacityMismatch {
                expected: mailbox.receive_capacity,
                actual: self.length,
            });
        }
        Ok(())
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
            send_sync_manager: 0,
            receive_sync_manager: 1,
        }
    }

    pub const fn with_retry_policy(mut self, retry_policy: MailboxRetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub const fn with_status_bit(mut self, status_bit: MailboxStatusBit) -> Self {
        self.status_bit = Some(status_bit);
        if status_bit.active_high
            && status_bit.mask == MAILBOX_FULL_STATUS_MASK
            && status_bit.address >= ESC_SYNC_MANAGER_BASE + SYNC_MANAGER_STATUS_OFFSET
        {
            let relative = status_bit.address - ESC_SYNC_MANAGER_BASE - SYNC_MANAGER_STATUS_OFFSET;
            if relative % ESC_SYNC_MANAGER_STRIDE == 0 {
                self.receive_sync_manager = (relative / ESC_SYNC_MANAGER_STRIDE) as u8;
            }
        }
        self
    }

    pub const fn with_sync_managers(mut self, send: u8, receive: u8) -> Self {
        self.send_sync_manager = send;
        self.receive_sync_manager = receive;
        self
    }

    pub const fn receive_status_bit(self) -> MailboxStatusBit {
        match self.status_bit {
            Some(status) => status,
            None => MailboxStatusBit::sync_manager_mailbox_full(self.receive_sync_manager),
        }
    }

    /// Compare the physical mailbox layout that the standard SII header can
    /// represent. Runtime polling, timeout, retry and status-bit policy are
    /// intentionally excluded.
    pub const fn has_same_layout(self, other: Self) -> bool {
        self.send_address == other.send_address
            && self.send_capacity == other.send_capacity
            && self.receive_address == other.receive_address
            && self.receive_capacity == other.receive_capacity
    }

    pub fn validate(self) -> Result<(), MailboxConfigError> {
        if usize::from(self.send_sync_manager) >= MAX_ESC_SYNC_MANAGERS
            || usize::from(self.receive_sync_manager) >= MAX_ESC_SYNC_MANAGERS
            || self.send_sync_manager == self.receive_sync_manager
        {
            return Err(MailboxConfigError::InvalidSyncManager);
        }
        let send = mailbox_range(
            self.send_address,
            self.send_capacity,
            MailboxDirection::Send,
        )?;
        let receive = mailbox_range(
            self.receive_address,
            self.receive_capacity,
            MailboxDirection::Receive,
        )?;
        if send.0 < receive.1 && receive.0 < send.1 {
            return Err(MailboxConfigError::AddressRangeOverlap);
        }
        if self
            .status_bit
            .is_some_and(|status_bit| status_bit.mask == 0)
        {
            return Err(MailboxConfigError::InvalidStatusBit);
        }
        Ok(())
    }
}

fn mailbox_range(
    address: u16,
    capacity: u16,
    direction: MailboxDirection,
) -> Result<(u32, u32), MailboxConfigError> {
    if address == 0 {
        return Err(MailboxConfigError::AddressZero(direction));
    }
    if usize::from(capacity) < MAILBOX_HEADER_LEN {
        return Err(MailboxConfigError::CapacityTooSmall(direction));
    }
    if usize::from(capacity) > MAX_MAILBOX_BYTES {
        return Err(MailboxConfigError::CapacityExceeded(direction));
    }
    let start = u32::from(address);
    let end = start + u32::from(capacity);
    if end > u32::from(u16::MAX) + 1 {
        return Err(MailboxConfigError::AddressRangeOverflow(direction));
    }
    Ok((start, end))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxPhase {
    Idle,
    CheckingSendStatus,
    Sending,
    CheckingStatus,
    Polling,
    ReadingRepeatActivation,
    WritingRepeatRequest,
    WaitingRepeatAck,
    PollingRepeat,
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
    pub working_counter_policy: RxWorkingCounterPolicy,
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
    StatusUnavailable,
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
    MissingMappedStatusBinding,
    BufferTooSmall,
    LengthOutOfBounds,
    HeaderTruncated,
    UnknownProtocol,
    ProtocolMismatch,
    CounterOutOfBounds,
    CounterMismatch,
    EmergencyUnconsumed,
    TransportNotOwned,
    RemoteError(MailboxErrorResponse),
    MalformedErrorResponse,
    Control(ControlError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxStatusSource {
    PollTime,
    Direct,
    Mapped,
}

/// Canonical EtherCAT mailbox error body (error type, error detail).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxErrorResponse {
    pub error_type: u16,
    pub detail: u16,
}

impl MailboxErrorResponse {
    pub fn parse(payload: &[u8]) -> Result<Self, MailboxError> {
        if payload.len() != 4 {
            return Err(MailboxError::MalformedErrorResponse);
        }
        Ok(Self {
            error_type: u16::from_le_bytes([payload[0], payload[1]]),
            detail: u16::from_le_bytes([payload[2], payload[3]]),
        })
    }
}

/// Linear ownership token for one physical slave's mailbox transport state.
/// It cannot be copied or cloned; transfer it when configuration yields to
/// runtime requests, and return it before explicit reconfiguration.
#[derive(Debug, Eq, PartialEq)]
pub struct MailboxTransportState {
    station_address: u16,
    next_counter: u8,
    last_delivered_counter: Option<u8>,
    next_token: u8,
    next_datagram_index: u8,
}

impl MailboxTransportState {
    pub const fn station_address(&self) -> Option<u16> {
        if self.station_address == 0 {
            None
        } else {
            Some(self.station_address)
        }
    }
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
    status_source: MailboxStatusSource,
    transport_owned: bool,
    last_delivered_counter: Option<u8>,
    repeat_activation: u8,
    idle_receive: bool,
    emergency_queue: CoeEmergencyQueue<8>,
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
            status_source: MailboxStatusSource::PollTime,
            transport_owned: true,
            last_delivered_counter: None,
            repeat_activation: 0,
            idle_receive: false,
            emergency_queue: CoeEmergencyQueue::new(),
        }
    }

    pub const fn without_transport() -> Self {
        let mut controller = Self::new();
        controller.transport_owned = false;
        controller
    }

    pub const fn phase(&self) -> MailboxPhase {
        self.phase
    }

    pub const fn station_address(&self) -> Option<u16> {
        if self.station_address == 0 {
            None
        } else {
            Some(self.station_address)
        }
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

    pub const fn status_source(&self) -> MailboxStatusSource {
        self.status_source
    }

    /// Side-effect-free scheduler readiness. A due mapped-status observation
    /// also needs admission even though it does not create a wire action yet.
    pub fn is_action_due(&self, now_ns: u64) -> bool {
        self.transport_owned
            && !matches!(
                self.phase,
                MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted
            )
            && (self.pending.is_some()
                || now_ns >= self.configuration_deadline_ns
                || now_ns >= self.poll_due_ns)
    }

    pub fn pop_emergency(&self) -> Option<CoeEmergencyEvent> {
        self.emergency_queue.pop()
    }

    pub fn lost_emergencies(&self) -> usize {
        self.emergency_queue.lost_events()
    }

    pub const fn owns_transport(&self) -> bool {
        self.transport_owned
    }

    /// Abort caller-owned logical work only after its wire request was
    /// completed/released. Preserve the physical TX/RX transport sequence.
    pub fn abort_transaction(&mut self, error: MailboxError) -> Result<(), MailboxError> {
        if self.pending.is_some() {
            return Err(MailboxError::Busy);
        }
        if matches!(
            self.phase,
            MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted
        ) {
            return Ok(());
        }
        self.last_error = Some(error);
        self.phase = MailboxPhase::Faulted;
        self.idle_receive = false;
        Ok(())
    }

    pub fn release_transport_state(&mut self) -> Result<MailboxTransportState, MailboxError> {
        if !self.transport_owned {
            return Err(MailboxError::TransportNotOwned);
        }
        if self.pending.is_some()
            || !matches!(
                self.phase,
                MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted
            )
        {
            return Err(MailboxError::Busy);
        }
        self.transport_owned = false;
        Ok(MailboxTransportState {
            station_address: self.station_address,
            next_counter: self.next_counter,
            last_delivered_counter: self.last_delivered_counter,
            next_token: self.next_token,
            next_datagram_index: self.next_datagram_index,
        })
    }

    pub fn adopt_transport_state(
        &mut self,
        state: MailboxTransportState,
    ) -> Result<(), MailboxError> {
        if self.transport_owned || self.pending.is_some() {
            return Err(MailboxError::Busy);
        }
        self.next_counter = state.next_counter;
        self.station_address = state.station_address;
        self.last_delivered_counter = state.last_delivered_counter;
        self.next_token = state.next_token;
        self.next_datagram_index = state.next_datagram_index;
        self.transport_owned = true;
        Ok(())
    }

    pub fn adopt_transport_state_for(
        &mut self,
        station_address: u16,
        state: MailboxTransportState,
    ) -> Result<(), MailboxError> {
        if station_address == 0
            || state
                .station_address()
                .is_some_and(|station| station != station_address)
        {
            return Err(MailboxError::InvalidConfiguration);
        }
        self.adopt_transport_state(state)?;
        self.station_address = station_address;
        Ok(())
    }

    pub(crate) fn transaction_matches(
        &self,
        station_address: u16,
        generation: u16,
        protocol: MailboxProtocol,
        payload: &[u8],
    ) -> bool {
        self.phase != MailboxPhase::Idle
            && payload.len() <= MAX_MAILBOX_BYTES - MAILBOX_HEADER_LEN
            && self.station_address == station_address
            && self.generation == generation
            && self.protocol == protocol
            && self.send_frame[..2] == (payload.len() as u16).to_le_bytes()
            && self.send_frame[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + payload.len()] == *payload
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
        let status_source = MailboxStatusSource::Direct;
        self.start_impl(
            config,
            station_address,
            generation,
            now_ns,
            protocol,
            payload,
            status_source,
        )
    }

    pub fn start_with_mapped_status(
        &mut self,
        config: MailboxConfig,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        protocol: MailboxProtocol,
        payload: &[u8],
    ) -> Result<(), MailboxError> {
        if !self.transport_owned {
            return Err(MailboxError::TransportNotOwned);
        }
        if config.status_bit.is_none() {
            return Err(MailboxError::InvalidConfiguration);
        }
        self.start_impl(
            config,
            station_address,
            generation,
            now_ns,
            protocol,
            payload,
            MailboxStatusSource::Mapped,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_impl(
        &mut self,
        config: MailboxConfig,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        protocol: MailboxProtocol,
        payload: &[u8],
        status_source: MailboxStatusSource,
    ) -> Result<(), MailboxError> {
        if !self.transport_owned {
            return Err(MailboxError::TransportNotOwned);
        }
        if station_address == 0
            || (self.station_address != 0 && self.station_address != station_address)
        {
            return Err(MailboxError::InvalidConfiguration);
        }
        if !matches!(
            self.phase,
            MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted
        ) {
            return Err(MailboxError::Busy);
        }
        if config.validate().is_err()
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
        self.send_len = config.send_capacity as usize;
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
        self.last_error = None;
        self.retry_count = 0;
        self.last_retry_error = None;
        self.discarded_frames = 0;
        self.status_source = status_source;
        self.idle_receive = false;
        self.phase = MailboxPhase::CheckingSendStatus;
        Ok(())
    }

    /// Poll unsolicited messages without transmitting an SDO request.
    pub fn start_receive(
        &mut self,
        config: MailboxConfig,
        station_address: u16,
        generation: u16,
        now_ns: u64,
    ) -> Result<(), MailboxError> {
        self.start_receive_impl(config, station_address, generation, now_ns, false)
    }

    pub fn start_receive_with_mapped_status(
        &mut self,
        config: MailboxConfig,
        station_address: u16,
        generation: u16,
        now_ns: u64,
    ) -> Result<(), MailboxError> {
        self.start_receive_impl(config, station_address, generation, now_ns, true)
    }

    fn start_receive_impl(
        &mut self,
        config: MailboxConfig,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        mapped: bool,
    ) -> Result<(), MailboxError> {
        // Reuse validation/reset but preserve the next TX counter: receiving
        // unsolicited messages must not consume the transmit sequence.
        let counter = self.next_counter;
        if mapped {
            self.start_with_mapped_status(
                config,
                station_address,
                generation,
                now_ns,
                MailboxProtocol::CoE,
                &[],
            )
        } else {
            self.start(
                config,
                station_address,
                generation,
                now_ns,
                MailboxProtocol::CoE,
                &[],
            )
        }?;
        self.next_counter = counter;
        self.idle_receive = true;
        self.phase = MailboxPhase::CheckingStatus;
        Ok(())
    }

    pub fn finish_idle_receive(&mut self) -> Result<(), MailboxError> {
        if !self.idle_receive
            || self.pending.is_some()
            || self.phase != MailboxPhase::CheckingStatus
        {
            return Err(MailboxError::Busy);
        }
        self.phase = MailboxPhase::Idle;
        self.idle_receive = false;
        Ok(())
    }

    pub fn mapped_status_due(&self, now_ns: u64) -> bool {
        self.status_source == MailboxStatusSource::Mapped
            && self.phase == MailboxPhase::CheckingStatus
            && self.pending.is_none()
            && now_ns >= self.poll_due_ns
    }

    pub fn observe_mapped_status(
        &mut self,
        observation: MailboxMappedStatusObservation,
        now_ns: u64,
    ) -> Result<MailboxProgress, MailboxError> {
        if !self.mapped_status_due(now_ns) {
            return Err(MailboxError::InvalidConfiguration);
        }
        if now_ns >= self.configuration_deadline_ns {
            return self.fail(MailboxError::Timeout);
        }
        match observation {
            MailboxMappedStatusObservation::Active => {
                self.phase = MailboxPhase::Polling;
                self.poll_due_ns = now_ns;
                Ok(MailboxProgress::Advanced)
            }
            MailboxMappedStatusObservation::Inactive => {
                self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                Ok(MailboxProgress::NoMessage)
            }
            MailboxMappedStatusObservation::Unavailable(_) => {
                self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                Ok(MailboxProgress::StatusUnavailable)
            }
        }
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
        if now_ns < self.poll_due_ns {
            return Ok(None);
        }

        let (operation, address, read_len, payload, write_len) = match self.phase {
            MailboxPhase::CheckingSendStatus => (
                RegisterOperation::Read,
                fixed_address(
                    self.station_address,
                    MailboxStatusBit::sync_manager_mailbox_full(self.config.send_sync_manager)
                        .address,
                ),
                1,
                [0; MAX_MAILBOX_BYTES],
                0,
            ),
            MailboxPhase::Sending => (
                RegisterOperation::Write,
                fixed_address(self.station_address, self.config.send_address),
                self.send_len as u16,
                self.send_frame,
                self.send_len as u8,
            ),
            MailboxPhase::CheckingStatus => {
                if self.status_source == MailboxStatusSource::Mapped {
                    return Ok(None);
                }
                let status_bit = self.config.receive_status_bit();
                (
                    RegisterOperation::Read,
                    fixed_address(self.station_address, status_bit.address),
                    1,
                    [0; MAX_MAILBOX_BYTES],
                    0,
                )
            }
            MailboxPhase::Polling | MailboxPhase::PollingRepeat => (
                RegisterOperation::Read,
                fixed_address(self.station_address, self.config.receive_address),
                self.config.receive_capacity,
                [0; MAX_MAILBOX_BYTES],
                0,
            ),
            MailboxPhase::ReadingRepeatActivation => (
                RegisterOperation::Read,
                fixed_address(
                    self.station_address,
                    ESC_SYNC_MANAGER_BASE
                        + self.config.receive_sync_manager as u16 * ESC_SYNC_MANAGER_STRIDE
                        + 6,
                ),
                1,
                [0; MAX_MAILBOX_BYTES],
                0,
            ),
            MailboxPhase::WritingRepeatRequest => {
                let mut bytes = [0; MAX_MAILBOX_BYTES];
                bytes[0] = self.repeat_activation;
                (
                    RegisterOperation::Write,
                    fixed_address(
                        self.station_address,
                        ESC_SYNC_MANAGER_BASE
                            + self.config.receive_sync_manager as u16 * ESC_SYNC_MANAGER_STRIDE
                            + 6,
                    ),
                    1,
                    bytes,
                    1,
                )
            }
            MailboxPhase::WaitingRepeatAck => (
                RegisterOperation::Read,
                fixed_address(
                    self.station_address,
                    ESC_SYNC_MANAGER_BASE
                        + self.config.receive_sync_manager as u16 * ESC_SYNC_MANAGER_STRIDE
                        + 7,
                ),
                1,
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
            working_counter_policy: if matches!(
                self.phase,
                MailboxPhase::Sending | MailboxPhase::Polling | MailboxPhase::PollingRepeat
            ) {
                RxWorkingCounterPolicy::ZeroOrOne
            } else {
                RxWorkingCounterPolicy::Exact
            },
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
        pool.acquire_with_response_len_and_wkc_policy(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
            action.working_counter_policy,
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
        if !action
            .working_counter_policy
            .accepts(action.expected_wkc, working_counter)
        {
            return self.retry_or_fail(MailboxError::UnexpectedWorkingCounter, now_ns);
        }
        if payload.len() != action.datagram_len() {
            return self.retry_or_fail(MailboxError::PayloadLengthMismatch, now_ns);
        }

        let progress = match self.phase {
            MailboxPhase::CheckingSendStatus => {
                self.pending = None;
                if payload[0] & MAILBOX_FULL_STATUS_MASK != 0 {
                    self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                    return Ok(MailboxProgress::NoMessage);
                }
                self.phase = MailboxPhase::Sending;
                self.poll_due_ns = now_ns;
                MailboxProgress::Advanced
            }
            MailboxPhase::Sending => {
                if working_counter == 0 {
                    self.phase = MailboxPhase::CheckingSendStatus;
                    self.pending = None;
                    self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                    return Ok(MailboxProgress::NoMessage);
                }
                self.phase = if self.status_source == MailboxStatusSource::PollTime {
                    MailboxPhase::Polling
                } else {
                    MailboxPhase::CheckingStatus
                };
                self.poll_due_ns = now_ns;
                MailboxProgress::Advanced
            }
            MailboxPhase::CheckingStatus => {
                if self.status_source != MailboxStatusSource::Direct {
                    return self.fail(MailboxError::InvalidConfiguration);
                }
                let status_bit = self.config.receive_status_bit();
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
            MailboxPhase::ReadingRepeatActivation => {
                self.repeat_activation = payload[0] ^ 0x02;
                self.phase = MailboxPhase::WritingRepeatRequest;
                self.poll_due_ns = now_ns;
                MailboxProgress::Advanced
            }
            MailboxPhase::WritingRepeatRequest => {
                self.phase = MailboxPhase::WaitingRepeatAck;
                self.poll_due_ns = now_ns;
                MailboxProgress::Advanced
            }
            MailboxPhase::WaitingRepeatAck => {
                self.pending = None;
                if payload[0] & 0x02 != self.repeat_activation & 0x02 {
                    self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                    return Ok(MailboxProgress::NoMessage);
                }
                self.phase = MailboxPhase::PollingRepeat;
                self.poll_due_ns = now_ns;
                MailboxProgress::Advanced
            }
            MailboxPhase::Polling | MailboxPhase::PollingRepeat => {
                if working_counter == 0 {
                    self.pending = None;
                    self.phase = MailboxPhase::CheckingStatus;
                    self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                    return Ok(MailboxProgress::NoMessage);
                }
                let frame = match MailboxFrame::parse(payload) {
                    Ok(frame) => frame,
                    Err(error) => return self.retry_or_fail(error, now_ns),
                };
                if frame.header.length == 0 {
                    self.pending = None;
                    self.phase = MailboxPhase::CheckingStatus;
                    self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                    return Ok(MailboxProgress::NoMessage);
                }
                // Counters are diagnostic state, not message identities. Even
                // a recovered new message may reuse the previous nonzero
                // counter. The exact pending action/generation is consumed
                // once, and the shared RX owner rejects duplicated frames.
                if frame.header.protocol == MailboxProtocol::Error {
                    let error = match MailboxErrorResponse::parse(frame.payload) {
                        Ok(error) => error,
                        Err(error) => return self.fail(error),
                    };
                    return self.fail(MailboxError::RemoteError(error));
                }
                if frame.header.protocol == MailboxProtocol::CoE {
                    if let Ok(emergency) = CoeEmergency::parse(frame.payload) {
                        let event = CoeEmergencyEvent::new(
                            now_ns,
                            self.station_address,
                            self.generation,
                            frame.header.counter,
                            emergency,
                        );
                        if let Some(sink) = emergency_sink {
                            let _ = sink.record(event);
                        } else {
                            let _ = self.emergency_queue.record(event);
                        }
                        self.last_delivered_counter = Some(frame.header.counter);
                        self.pending = None;
                        self.phase = MailboxPhase::CheckingStatus;
                        self.poll_due_ns = now_ns.saturating_add(self.config.poll_interval_ns);
                        self.discarded_frames = self.discarded_frames.saturating_add(1);
                        return Ok(MailboxProgress::EmergencyConsumed);
                    }
                }
                if frame.header.protocol != self.protocol {
                    return self.retry_or_fail(MailboxError::ProtocolMismatch, now_ns);
                }
                self.last_delivered_counter = Some(frame.header.counter);
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
                if request.action_datagram_index() != action.datagram_index
                    || request.generation != action.generation
                    || request.address != action.address
                    || request.response_length != action.datagram_len()
                    || request.length != action.datagram_len()
                    || request.operation != action.operation
                    || request.deadline_ns != action.deadline_ns
                    || request.working_counter_policy() != action.working_counter_policy
                {
                    let _ = pool.release(handle);
                    return self.fail(MailboxError::ActionMismatch);
                }
                let mut response = [0; MAX_MAILBOX_BYTES];
                response[..request.length].copy_from_slice(request.payload());
                (request.generation, request.actual_wkc, response)
            }
            Some(request) if request.state == RequestState::Failed => {
                if request.action_datagram_index() != action.datagram_index
                    || request.generation != action.generation
                    || request.address != action.address
                    || request.response_length != action.datagram_len()
                    || request.deadline_ns != action.deadline_ns
                    || request.operation != action.operation
                    || request.working_counter_policy() != action.working_counter_policy
                {
                    return self.fail(MailboxError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                let _ = pool.release(handle);
                if error == ControlError::WorkingCounterMismatch {
                    return self.retry_or_fail(MailboxError::UnexpectedWorkingCounter, now_ns);
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                if error == ControlError::TransmitFailed {
                    return self.retry_or_fail(MailboxError::Control(error), now_ns);
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
            if matches!(
                self.phase,
                MailboxPhase::Polling | MailboxPhase::PollingRepeat
            ) {
                self.phase = MailboxPhase::ReadingRepeatActivation;
            }
            Ok(MailboxProgress::RetryScheduled)
        } else {
            self.fail(error)
        }
    }

    fn fail<T>(&mut self, error: MailboxError) -> Result<T, MailboxError> {
        if matches!(self.phase, MailboxPhase::Complete | MailboxPhase::Faulted) {
            return Err(error);
        }
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

    fn ready_send(controller: &mut MailboxController, now: u64) -> MailboxAction {
        let status = controller.next_action(now).unwrap().unwrap();
        assert_eq!(controller.phase(), MailboxPhase::CheckingSendStatus);
        assert_eq!(
            status.address,
            fixed_address(status.station_address, 0x0805)
        );
        controller
            .accept(status, status.generation, &[0], 1, now)
            .unwrap();
        controller.next_action(now).unwrap().unwrap()
    }

    fn ready_receive(controller: &mut MailboxController, now: u64) -> MailboxAction {
        for _ in 0..5 {
            let action = controller.next_action(now).unwrap().unwrap();
            let value = match controller.phase() {
                MailboxPhase::CheckingStatus => 8,
                MailboxPhase::ReadingRepeatActivation => 1,
                MailboxPhase::WritingRepeatRequest => action.payload()[0],
                MailboxPhase::WaitingRepeatAck => controller.repeat_activation,
                _ => return action,
            };
            controller
                .accept(action, action.generation, &[value], 1, now)
                .unwrap();
        }
        panic!("mailbox did not reach the receive memory action")
    }

    #[test]
    fn mailbox_config_validation_rejects_invalid_ranges_transactionally() {
        assert_eq!(
            MailboxConfig::new(0x1000, 32, 0x1100, 64).validate(),
            Ok(())
        );
        assert_eq!(
            MailboxConfig::new(0, 32, 0x1100, 32).validate(),
            Err(MailboxConfigError::AddressZero(MailboxDirection::Send))
        );
        assert_eq!(
            MailboxConfig::new(0x1000, 5, 0x1100, 32).validate(),
            Err(MailboxConfigError::CapacityTooSmall(MailboxDirection::Send))
        );
        assert_eq!(
            MailboxConfig::new(0x1000, 32, 0x1100, 129).validate(),
            Err(MailboxConfigError::CapacityExceeded(
                MailboxDirection::Receive
            ))
        );
        assert_eq!(
            MailboxConfig::new(0xFFF0, 17, 0x1000, 32).validate(),
            Err(MailboxConfigError::AddressRangeOverflow(
                MailboxDirection::Send
            ))
        );
        assert_eq!(
            MailboxConfig::new(0x1000, 32, 0x1010, 32).validate(),
            Err(MailboxConfigError::AddressRangeOverlap)
        );
        assert_eq!(
            MailboxConfig::new(0x1000, 32, 0x1100, 32)
                .with_status_bit(MailboxStatusBit::new(0x1200, 0, true))
                .validate(),
            Err(MailboxConfigError::InvalidStatusBit)
        );
    }

    #[test]
    fn mailbox_layout_comparison_ignores_runtime_policy() {
        let expected = MailboxConfig::new(0x1000, 32, 0x1100, 64);
        let mut policy_variant = expected
            .with_retry_policy(MailboxRetryPolicy::new(3, 5_000))
            .with_status_bit(MailboxStatusBit::new(0x1200, 0x08, false));
        policy_variant.poll_interval_ns = 99;
        policy_variant.timeout_ns = 100;
        policy_variant.request_timeout_ns = 101;

        assert!(expected.has_same_layout(policy_variant));
        assert!(!expected.has_same_layout(MailboxConfig::new(0x1000, 31, 0x1100, 64)));
    }

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

        let send = ready_send(&mut controller, 1);
        assert_eq!(send.operation, RegisterOperation::Write);
        assert_eq!(send.write_len, 32);
        controller.accept(send, 7, &[0; 32], 1, 2).unwrap();

        let poll = ready_receive(&mut controller, 3);
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
    fn receive_counter_is_independent_of_the_transmit_counter() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = ready_send(&mut controller, 1);
        controller.accept(send, 7, &[0; 32], 1, 2).unwrap();
        let poll = ready_receive(&mut controller, 3);
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
            Ok(MailboxProgress::Complete)
        );
        assert_eq!(controller.phase(), MailboxPhase::Complete);
        assert_eq!(controller.response().unwrap().0.counter, 7);
    }

    #[test]
    fn mailbox_retries_protocol_mismatch_without_changing_transaction_counter() {
        let config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_retry_policy(MailboxRetryPolicy::new(2, 10));
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = ready_send(&mut controller, 1);
        controller.accept(send, 7, &[0; 32], 1, 2).unwrap();

        let poll = ready_receive(&mut controller, 3);
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

        let retry = ready_receive(&mut controller, 14);
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
        let send = ready_send(&mut controller, 1);
        controller.accept(send, 7, &[0; 32], 1, 2).unwrap();

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
        let poll = ready_receive(&mut controller, 3);
        assert_eq!(
            controller.accept(poll, 7, &wrong, 1, 4),
            Ok(MailboxProgress::RetryScheduled)
        );
        let retry = ready_receive(&mut controller, 14);
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
        let send = ready_send(&mut controller, 1);
        controller.accept(send, 7, &[0; 32], 1, 2).unwrap();
        let poll = ready_receive(&mut controller, 3);

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
        assert_eq!(controller.phase(), MailboxPhase::CheckingStatus);
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
        let send = ready_send(&mut controller, 1);
        controller.accept(send, 7, &[0; 32], 1, 2).unwrap();

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
    fn mapped_status_binding_validates_and_reads_the_canonical_fmmu_bit() {
        let binding = MailboxMappedStatusBit::new(
            1,
            257,
            4,
            FmmuConfig {
                index: 2,
                logical_start: 0x1020,
                length: 1,
                logical_start_bit: 1,
                logical_end_bit: 1,
                physical_start: 0x0815,
                physical_start_bit: 3,
                fmmu_type: 1,
                enable: true,
            },
        );
        let status = MailboxStatusBit::sync_manager_mailbox_full(2);

        assert_eq!(binding.validate(0x1000, status), Ok(()));
        let mut input = [0; 33];
        assert_eq!(binding.read(&input), Ok(false));
        input[32] = 0x02;
        assert_eq!(binding.read(&input), Ok(true));

        let mut tampered = binding;
        tampered.fmmu.physical_start_bit = 2;
        assert_eq!(
            tampered.validate(0x1000, status),
            Err(MailboxMappedStatusError::StatusBitMismatch)
        );
        assert_eq!(
            MailboxMappedStatusBit::new(1, 264, 4, binding.fmmu).read(&input),
            Err(MailboxMappedStatusError::InputOutOfBounds)
        );
    }

    #[test]
    fn mapped_status_never_emits_a_direct_read_and_gates_the_mailbox_read() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_status_bit(MailboxStatusBit::sync_manager_mailbox_full(1));
        config.poll_interval_ns = 10;
        let mut controller = MailboxController::new();
        controller
            .start_with_mapped_status(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        assert_eq!(controller.status_source(), MailboxStatusSource::Mapped);

        let send = ready_send(&mut controller, 1);
        controller.accept(send, 7, &[0; 32], 1, 2).unwrap();
        assert_eq!(controller.phase(), MailboxPhase::CheckingStatus);
        assert!(controller.next_action(3).unwrap().is_none());
        assert_eq!(
            controller.observe_mapped_status(MailboxMappedStatusObservation::Inactive, 3),
            Ok(MailboxProgress::NoMessage)
        );
        assert!(controller.next_action(4).unwrap().is_none());

        assert_eq!(
            controller.observe_mapped_status(
                MailboxMappedStatusObservation::Unavailable(
                    MailboxMappedStatusUnavailable::InvalidQuality,
                ),
                13,
            ),
            Ok(MailboxProgress::StatusUnavailable)
        );
        assert!(controller.next_action(14).unwrap().is_none());

        assert_eq!(
            controller.observe_mapped_status(MailboxMappedStatusObservation::Active, 23),
            Ok(MailboxProgress::Advanced)
        );
        let poll = controller.next_action(23).unwrap().unwrap();
        assert_eq!(poll.operation, RegisterOperation::Read);
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
    fn receive_sync_manager_derives_and_validates_the_canonical_status_bit() {
        let mailbox = MailboxConfig::new(0x1000, 32, 0x1100, 64);
        let receive = MailboxReceiveSyncManager::new(1, 0x1100, 64, 0x22);
        assert_eq!(
            receive.status_bit(),
            MailboxStatusBit::new(0x080d, MAILBOX_FULL_STATUS_MASK, true)
        );
        assert_eq!(receive.validate_for(mailbox), Ok(()));
        assert_eq!(
            MailboxReceiveSyncManager::new(16, 0x1100, 64, 0x22).validate_for(mailbox),
            Err(MailboxReceiveSyncManagerError::IndexOutOfBounds(16))
        );
        assert_eq!(
            MailboxReceiveSyncManager::new(1, 0x1200, 64, 0x22).validate_for(mailbox),
            Err(MailboxReceiveSyncManagerError::AddressMismatch {
                expected: 0x1100,
                actual: 0x1200,
            })
        );
        assert_eq!(
            MailboxReceiveSyncManager::new(1, 0x1100, 32, 0x22).validate_for(mailbox),
            Err(MailboxReceiveSyncManagerError::CapacityMismatch {
                expected: 64,
                actual: 32,
            })
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
        let action = ready_send(&mut controller, 1);
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.get_mut(handle)
            .unwrap()
            .build_frame(&mut frame, [0; 6], [0; 6])
            .unwrap();
        assert_eq!(
            pool.complete(handle, action.generation, action.address, &[0; 32], 2,),
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

    #[test]
    fn expired_control_response_retries_then_faults_without_leaking_pool_slots() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_retry_policy(MailboxRetryPolicy::new(1, 2));
        config.request_timeout_ns = 10;
        config.timeout_ns = 100;
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];

        let action = controller.next_action(1).unwrap().unwrap();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        pool.build_into_buffer(handle, &mut frame, [0; 6], [0; 6])
            .unwrap();
        assert!(pool.expire_in_flight(action.deadline_ns).is_empty());
        let expiry = pool.expire_in_flight(action.deadline_ns + 1);
        assert_eq!(expiry.handles().next(), Some(handle));
        assert_eq!(
            pool.get(handle).unwrap().last_error(),
            Some(ControlError::Timeout)
        );
        assert_eq!(
            controller.accept_completed(&mut pool, handle, action.deadline_ns + 1),
            Ok(MailboxProgress::RetryScheduled)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(controller.last_retry_error(), Some(MailboxError::Timeout));
        assert_eq!(controller.retry_count(), 1);
        assert!(
            controller
                .next_action(action.deadline_ns + 2)
                .unwrap()
                .is_none()
        );

        let retry = controller
            .next_action(action.deadline_ns + 3)
            .unwrap()
            .unwrap();
        let retry_handle = controller.enqueue_pending(&mut pool).unwrap();
        pool.build_into_buffer(retry_handle, &mut frame, [0; 6], [0; 6])
            .unwrap();
        assert_eq!(pool.expire_in_flight(retry.deadline_ns + 1).count(), 1);
        assert_eq!(
            controller.accept_completed(&mut pool, retry_handle, retry.deadline_ns + 1),
            Err(MailboxError::Timeout)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(controller.phase(), MailboxPhase::Faulted);
        assert_eq!(controller.last_error(), Some(MailboxError::Timeout));
    }

    #[test]
    fn expired_foreign_control_handle_cannot_advance_or_release_mailbox_action() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        config.request_timeout_ns = 10;
        let mut controller = MailboxController::new();
        controller
            .start(config, 0x1000, 7, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let foreign = pool
            .acquire(
                44,
                7,
                action.address,
                action.operation,
                &[0; 7],
                action.deadline_ns,
            )
            .unwrap();
        pool.build_into_buffer(
            foreign,
            &mut [0; crate::wire::MAX_ETHERNET_FRAME_LEN],
            [0; 6],
            [0; 6],
        )
        .unwrap();
        assert_eq!(pool.expire_in_flight(action.deadline_ns + 1).count(), 1);
        assert_eq!(
            controller.accept_completed(&mut pool, foreign, action.deadline_ns + 1),
            Err(MailboxError::ActionMismatch)
        );
        assert_eq!(
            pool.get(foreign).unwrap().last_error(),
            Some(ControlError::Timeout)
        );
        assert_eq!(pool.in_use(), 1);
    }

    #[test]
    fn outgoing_mailbox_waits_for_empty_sm_then_writes_the_terminal_capacity_byte() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        config.poll_interval_ns = 10;
        let mut controller = MailboxController::new();
        controller
            .start(config, 1, 2, 0, MailboxProtocol::CoE, &[0xAA, 0xBB])
            .unwrap();
        let busy = controller.next_action(1).unwrap().unwrap();
        assert_eq!(busy.address, fixed_address(1, 0x0805));
        assert_eq!(
            controller.accept(busy, 2, &[8], 1, 1),
            Ok(MailboxProgress::NoMessage)
        );
        assert!(controller.next_action(10).unwrap().is_none());
        let ready = controller.next_action(11).unwrap().unwrap();
        controller.accept(ready, 2, &[0], 1, 11).unwrap();
        let write = controller.next_action(11).unwrap().unwrap();
        assert_eq!(write.payload().len(), 32);
        assert_eq!(&write.payload()[..8], &[2, 0, 0, 0, 0, 0x13, 0xAA, 0xBB]);
        assert!(write.payload()[8..].iter().all(|byte| *byte == 0));
        assert_eq!(write.payload()[31], 0);
    }

    #[test]
    fn zero_wkc_on_mailbox_memory_is_an_explicit_busy_or_empty_observation() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        config.poll_interval_ns = 1;
        let mut controller = MailboxController::new();
        controller
            .start(config, 1, 2, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = ready_send(&mut controller, 1);
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        assert_eq!(
            pool.get(handle).unwrap().working_counter_policy(),
            RxWorkingCounterPolicy::ZeroOrOne
        );
        pool.release(handle).unwrap();
        assert_eq!(
            controller.accept(send, 2, &[0; 32], 0, 1),
            Ok(MailboxProgress::NoMessage)
        );
        assert_eq!(controller.phase(), MailboxPhase::CheckingSendStatus);
        let send = ready_send(&mut controller, 2);
        assert_eq!(send.payload()[5], 0x13);
        controller.accept(send, 2, &[0; 32], 1, 2).unwrap();
        let poll = ready_receive(&mut controller, 3);
        assert_eq!(
            poll.working_counter_policy,
            RxWorkingCounterPolicy::ZeroOrOne
        );
        assert_eq!(
            controller.accept(poll, 2, &[0; 32], 0, 3),
            Ok(MailboxProgress::NoMessage)
        );
        assert_eq!(controller.phase(), MailboxPhase::CheckingStatus);
        assert_eq!(controller.retry_count(), 0);
        assert_eq!(controller.last_error(), None);
    }

    #[test]
    fn lost_consumed_read_toggles_repeat_then_waits_for_matching_ack_before_reread() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_retry_policy(MailboxRetryPolicy::new(3, 0));
        config.timeout_ns = 100;
        config.request_timeout_ns = 5;
        config.poll_interval_ns = 1;
        let mut controller = MailboxController::new();
        controller
            .start(config, 1, 2, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = ready_send(&mut controller, 1);
        controller.accept(send, 2, &[0; 32], 1, 1).unwrap();
        let read = ready_receive(&mut controller, 2);
        assert_eq!(
            controller.timeout(read, read.deadline_ns),
            Ok(MailboxProgress::RetryScheduled)
        );
        assert_eq!(controller.phase(), MailboxPhase::ReadingRepeatActivation);
        let activation = controller.next_action(7).unwrap().unwrap();
        assert_eq!(activation.address, fixed_address(1, 0x080E));
        controller.accept(activation, 2, &[0xA5], 1, 7).unwrap();
        let repeat = controller.next_action(7).unwrap().unwrap();
        assert_eq!(repeat.operation, RegisterOperation::Write);
        assert_eq!(repeat.payload(), &[0xA7]);
        controller.accept(repeat, 2, &[0xA7], 1, 7).unwrap();
        let ack = controller.next_action(7).unwrap().unwrap();
        assert_eq!(ack.address, fixed_address(1, 0x080F));
        assert_eq!(
            controller.accept(ack, 2, &[0xA5], 1, 7),
            Ok(MailboxProgress::NoMessage)
        );
        assert_eq!(controller.phase(), MailboxPhase::WaitingRepeatAck);
        assert!(controller.next_action(7).unwrap().is_none());
        let ack = controller.next_action(8).unwrap().unwrap();
        controller.accept(ack, 2, &[0xA7], 1, 8).unwrap();
        let repeated = controller.next_action(8).unwrap().unwrap();
        assert_eq!(controller.phase(), MailboxPhase::PollingRepeat);
        assert_eq!(repeated.address, fixed_address(1, 0x1100));
        assert!(repeated.deadline_ns <= 100);
        let mut bytes = [0; 32];
        bytes[..7].copy_from_slice(&[1, 0, 0, 0, 0, 0x03, 0xAB]);
        assert_eq!(
            controller.accept(repeated, 2, &bytes, 1, 8),
            Ok(MailboxProgress::Complete)
        );
        assert_eq!(controller.response().unwrap().1, &[0xAB]);
    }

    #[test]
    fn mailbox_error_body_is_preserved_as_a_terminal_remote_result() {
        let mut controller = MailboxController::new();
        controller
            .start(
                MailboxConfig::new(0x1000, 32, 0x1100, 32),
                1,
                2,
                0,
                MailboxProtocol::CoE,
                &[1],
            )
            .unwrap();
        let send = ready_send(&mut controller, 1);
        controller.accept(send, 2, &[0; 32], 1, 1).unwrap();
        let read = ready_receive(&mut controller, 2);
        let mut bytes = [0; 32];
        bytes[..10].copy_from_slice(&[4, 0, 0, 0, 0, 0x70, 1, 0, 4, 0]);
        let error = MailboxError::RemoteError(MailboxErrorResponse {
            error_type: 1,
            detail: 4,
        });
        assert_eq!(controller.accept(read, 2, &bytes, 1, 2), Err(error));
        assert_eq!(controller.last_error(), Some(error));
        assert_eq!(controller.phase(), MailboxPhase::Faulted);
        assert_eq!(controller.response(), None);
        assert_eq!(
            MailboxErrorResponse::parse(&[1, 0]),
            Err(MailboxError::MalformedErrorResponse)
        );
    }

    #[test]
    fn receive_counters_may_repeat_between_fresh_transactions_while_tx_wraps() {
        let mut controller = MailboxController::new();
        for (ordinal, receive_counter) in [7, 0, 4, 4, 1, 0, 6, 2].into_iter().enumerate() {
            let now = ordinal as u64 * 10;
            controller
                .start(
                    MailboxConfig::new(0x1000, 32, 0x1100, 32),
                    1,
                    2,
                    now,
                    MailboxProtocol::CoE,
                    &[1],
                )
                .unwrap();
            let send = ready_send(&mut controller, now);
            assert_eq!(send.payload()[5] >> 4, ordinal as u8 % 7 + 1);
            controller.accept(send, 2, &[0; 32], 1, now).unwrap();
            let read = ready_receive(&mut controller, now);
            let mut bytes = [0; 32];
            bytes[..7].copy_from_slice(&[1, 0, 0, 0, 0, (receive_counter << 4) | 3, 0xAB]);
            assert_eq!(
                controller.accept(read, 2, &bytes, 1, now),
                Ok(MailboxProgress::Complete)
            );
            assert_eq!(controller.response().unwrap().1, &[0xAB]);
        }
    }

    #[test]
    fn recovered_new_response_may_reuse_the_previously_delivered_counter() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
            .with_retry_policy(MailboxRetryPolicy::new(2, 0));
        config.poll_interval_ns = 1;
        config.request_timeout_ns = 5;
        let mut controller = MailboxController::new();
        let mut bytes = [0; 32];
        bytes[..7].copy_from_slice(&[1, 0, 0, 0, 0, 0x43, 0xAB]);
        for ordinal in 0..2 {
            let now = ordinal * 20;
            controller
                .start(config, 1, 2, now, MailboxProtocol::CoE, &[1])
                .unwrap();
            let send = ready_send(&mut controller, now);
            controller.accept(send, 2, &[0; 32], 1, now).unwrap();
            let mut read = ready_receive(&mut controller, now);
            if ordinal == 1 {
                controller.timeout(read, read.deadline_ns).unwrap();
                read = ready_receive(&mut controller, now + 5);
                assert_eq!(controller.phase(), MailboxPhase::PollingRepeat);
            }
            assert_eq!(
                controller.accept(read, 2, &bytes, 1, now + 5),
                Ok(MailboxProgress::Complete)
            );
            assert_eq!(controller.response().unwrap().0.counter, 4);
        }
    }

    #[test]
    fn duplicated_terminal_delivery_cannot_replace_a_completed_mailbox_result() {
        let mut controller = MailboxController::new();
        controller
            .start(
                MailboxConfig::new(0x1000, 32, 0x1100, 32),
                1,
                2,
                0,
                MailboxProtocol::CoE,
                &[1],
            )
            .unwrap();
        let send = ready_send(&mut controller, 0);
        controller.accept(send, 2, &[0; 32], 1, 0).unwrap();
        let read = ready_receive(&mut controller, 0);
        let mut bytes = [0; 32];
        bytes[..7].copy_from_slice(&[1, 0, 0, 0, 0, 3, 0xAB]);
        controller.accept(read, 2, &bytes, 1, 0).unwrap();
        bytes[6] = 0xCD;
        assert_eq!(
            controller.accept(read, 2, &bytes, 1, 0),
            Err(MailboxError::ActionMismatch)
        );
        assert_eq!(
            controller.timeout(read, read.deadline_ns),
            Err(MailboxError::ActionMismatch)
        );
        assert_eq!(controller.phase(), MailboxPhase::Complete);
        assert_eq!(controller.last_error(), None);
        assert_eq!(controller.response().unwrap().1, &[0xAB]);
    }

    #[test]
    fn sinkless_emergency_queue_overflow_does_not_block_the_sdo_response() {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        config.poll_interval_ns = 1;
        let mut controller = MailboxController::new();
        controller
            .start(config, 1, 2, 0, MailboxProtocol::CoE, &[1])
            .unwrap();
        let send = ready_send(&mut controller, 0);
        controller.accept(send, 2, &[0; 32], 1, 0).unwrap();
        let mut bytes = [0; 32];
        bytes[..16].copy_from_slice(&[10, 0, 0, 0, 0, 3, 0, 0x10, 0x10, 0x23, 0x81, 1, 2, 3, 4, 5]);
        for now in 1..=10 {
            let read = ready_receive(&mut controller, now);
            assert_eq!(
                controller.accept(read, 2, &bytes, 1, now),
                Ok(MailboxProgress::EmergencyConsumed)
            );
        }
        assert!(controller.lost_emergencies() > 0);
        let read = ready_receive(&mut controller, 11);
        bytes.fill(0);
        bytes[..7].copy_from_slice(&[1, 0, 0, 0, 0, 3, 0xAB]);
        assert_eq!(
            controller.accept(read, 2, &bytes, 1, 11),
            Ok(MailboxProgress::Complete)
        );
        assert_eq!(controller.response().unwrap().1, &[0xAB]);
        let mut delivered = 0;
        while let Some(event) = controller.pop_emergency() {
            assert_eq!(event.error_code, 0x2310);
            delivered += 1;
        }
        assert_eq!(delivered + controller.lost_emergencies(), 10);
    }
}
