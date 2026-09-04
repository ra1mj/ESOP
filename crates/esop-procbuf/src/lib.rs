#![no_std]

//! Fixed-layout real-time ABI between ESOP's RT owner and supervisor domain.
//!
//! The buffer deliberately contains only fixed-size records. Command and
//! state snapshots use two pages with explicit page ownership, so a reader
//! never observes a partially written page and a writer never waits for a
//! reader. The event ring is a bounded SPSC channel with observable overflow.

use core::cell::UnsafeCell;
use core::mem::size_of;
use core::sync::atomic::{AtomicU32, Ordering};

pub const ABI_MAGIC: u32 = 0x4553_4F50;
pub const ABI_VERSION: u16 = 1;

const PAGE_FREE: u32 = 0;
const PAGE_WRITING: u32 = 1;
const PAGE_PUBLISHED: u32 = 2;
const PAGE_READING: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderError {
    MagicMismatch,
    AbiVersionMismatch,
    HeaderSizeMismatch,
    LayoutHashMismatch,
    RobotIdMismatch,
    BootIdMismatch,
    RegionSizeMismatch,
    CapacityMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcBufDimensions {
    pub axes: u16,
    pub io_channels: u16,
    pub domains: u16,
    pub event_capacity: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ProcBufHeader {
    pub magic: u32,
    pub abi_version: u16,
    pub header_bytes: u16,
    pub layout_hash: u64,
    pub robot_id: u64,
    pub boot_id: u64,
    pub region_bytes: u32,
    pub axes: u16,
    pub io_channels: u16,
    pub domains: u16,
    pub event_capacity: u16,
    pub reserved: u32,
}

impl ProcBufHeader {
    pub const fn new(
        layout_hash: u64,
        robot_id: u64,
        boot_id: u64,
        region_bytes: u32,
        dimensions: ProcBufDimensions,
    ) -> Self {
        Self {
            magic: ABI_MAGIC,
            abi_version: ABI_VERSION,
            header_bytes: size_of::<Self>() as u16,
            layout_hash,
            robot_id,
            boot_id,
            region_bytes,
            axes: dimensions.axes,
            io_channels: dimensions.io_channels,
            domains: dimensions.domains,
            event_capacity: dimensions.event_capacity,
            reserved: 0,
        }
    }

    pub fn validate<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &self,
        robot_id: u64,
        boot_id: u64,
    ) -> Result<(), HeaderError> {
        if self.magic != ABI_MAGIC {
            return Err(HeaderError::MagicMismatch);
        }
        if self.abi_version != ABI_VERSION {
            return Err(HeaderError::AbiVersionMismatch);
        }
        if self.header_bytes as usize != size_of::<Self>() {
            return Err(HeaderError::HeaderSizeMismatch);
        }
        if self.layout_hash != layout_hash::<AXES, IO, DOMAINS, EVENTS>() {
            return Err(HeaderError::LayoutHashMismatch);
        }
        if self.robot_id != robot_id {
            return Err(HeaderError::RobotIdMismatch);
        }
        if self.boot_id != boot_id {
            return Err(HeaderError::BootIdMismatch);
        }
        if self.region_bytes as usize != size_of::<ProcBuf<AXES, IO, DOMAINS, EVENTS>>() {
            return Err(HeaderError::RegionSizeMismatch);
        }
        if self.axes as usize != AXES
            || self.io_channels as usize != IO
            || self.domains as usize != DOMAINS
            || self.event_capacity as usize != EVENTS
        {
            return Err(HeaderError::CapacityMismatch);
        }
        Ok(())
    }
}

/// A deterministic hash of the public ABI shape. It is intentionally not a
/// cryptographic hash; it detects layout/capacity mismatches at attachment.
pub const fn layout_hash<
    const AXES: usize,
    const IO: usize,
    const DOMAINS: usize,
    const EVENTS: usize,
>() -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    hash = hash_bytes(hash, ABI_VERSION as u64);
    hash = hash_bytes(hash, AXES as u64);
    hash = hash_bytes(hash, IO as u64);
    hash = hash_bytes(hash, DOMAINS as u64);
    hash = hash_bytes(hash, EVENTS as u64);
    hash = hash_bytes(hash, size_of::<ProcBufHeader>() as u64);
    hash = hash_bytes(hash, size_of::<CommandPage<AXES, IO>>() as u64);
    hash = hash_bytes(hash, size_of::<StatePage<AXES, IO, DOMAINS>>() as u64);
    hash = hash_bytes(hash, size_of::<ProcBufEvent>() as u64);
    hash_bytes(hash, size_of::<ProcBuf<AXES, IO, DOMAINS, EVENTS>>() as u64)
}

const fn hash_bytes(mut hash: u64, value: u64) -> u64 {
    let mut shift = 0;
    while shift < 64 {
        hash ^= (value >> shift) & 0xFF;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        shift += 8;
    }
    hash
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ControlMode {
    Csp = 8,
    Csv = 9,
    Cst = 10,
    Unknown = 255,
}

impl ControlMode {
    pub const fn from_raw(value: u8) -> Self {
        match value {
            8 => Self::Csp,
            9 => Self::Csv,
            10 => Self::Cst,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct JointCommand {
    pub position: f64,
    pub velocity: f64,
    pub torque: f64,
    pub max_velocity: f64,
    pub max_torque: f64,
}

impl JointCommand {
    pub const EMPTY: Self = Self {
        position: 0.0,
        velocity: 0.0,
        torque: 0.0,
        max_velocity: 0.0,
        max_torque: 0.0,
    };

    fn finite(self) -> bool {
        self.position.is_finite()
            && self.velocity.is_finite()
            && self.torque.is_finite()
            && self.max_velocity.is_finite()
            && self.max_torque.is_finite()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct IoCommand {
    pub output_bits: u32,
    pub output_mask: u32,
}

impl IoCommand {
    pub const EMPTY: Self = Self {
        output_bits: 0,
        output_mask: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct CommandPage<const AXES: usize, const IO: usize> {
    pub boot_id: u64,
    pub sequence: u64,
    pub deadline_ns: u64,
    pub source_id: u64,
    pub permit_epoch: u64,
    pub permit_expires_at_ns: u64,
    pub axis_mask: u32,
    pub requested_mode: ControlMode,
    pub motion_enable_request: u8,
    pub authority: u8,
    pub reserved: u8,
    pub axes: [JointCommand; AXES],
    pub io: [IoCommand; IO],
}

impl<const AXES: usize, const IO: usize> CommandPage<AXES, IO> {
    pub const fn new(boot_id: u64) -> Self {
        Self {
            boot_id,
            sequence: 0,
            deadline_ns: 0,
            source_id: 0,
            permit_epoch: 0,
            permit_expires_at_ns: 0,
            axis_mask: 0,
            requested_mode: ControlMode::Unknown,
            motion_enable_request: 0,
            authority: 0,
            reserved: 0,
            axes: [JointCommand::EMPTY; AXES],
            io: [IoCommand::EMPTY; IO],
        }
    }

    fn well_formed(self) -> bool {
        self.sequence != 0
            && self.requested_mode != ControlMode::Unknown
            && self.motion_enable_request <= 1
            && self.axes.iter().copied().all(JointCommand::finite)
            && (self.motion_enable_request == 0 || self.axis_mask != 0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPublishError {
    BootMismatch,
    InvalidCommand,
    NoFreePage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandReadError {
    Unavailable,
    BootMismatch,
    Replayed,
    Expired,
    PermitExpired,
    InvalidCommand,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandSnapshot<const AXES: usize, const IO: usize> {
    pub publish_sequence: u64,
    pub command: CommandPage<AXES, IO>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct JointState {
    pub position: f64,
    pub velocity: f64,
    pub torque: f64,
    pub following_error: f64,
    pub statusword: u16,
    pub controlword: u16,
    pub drive_state: u8,
    pub actual_mode: u8,
    pub quality: u8,
    pub reserved: u8,
}

impl JointState {
    pub const EMPTY: Self = Self {
        position: 0.0,
        velocity: 0.0,
        torque: 0.0,
        following_error: 0.0,
        statusword: 0,
        controlword: 0,
        drive_state: 255,
        actual_mode: 255,
        quality: 0,
        reserved: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct IoState {
    pub input_bits: u32,
    pub quality: u32,
}

impl IoState {
    pub const EMPTY: Self = Self {
        input_bits: 0,
        quality: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DomainQuality {
    pub expected_wkc: u16,
    pub actual_wkc: u16,
    pub valid: u8,
    pub complete: u8,
    pub reserved: u16,
    pub last_valid_cycle: u64,
    pub input_age_cycles: u64,
}

impl DomainQuality {
    pub const EMPTY: Self = Self {
        expected_wkc: 0,
        actual_wkc: 0,
        valid: 0,
        complete: 0,
        reserved: 0,
        last_valid_cycle: 0,
        input_age_cycles: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct QualityPage<const DOMAINS: usize> {
    pub sequence: u64,
    pub link_up: u8,
    pub al_state: u8,
    pub dc_locked: u8,
    pub reserved: u8,
    pub fault_bitmap: u32,
    pub command_age_cycles: u64,
    pub deadline_misses: u64,
    pub dc_offset_ns: i64,
    pub domains: [DomainQuality; DOMAINS],
}

impl<const DOMAINS: usize> QualityPage<DOMAINS> {
    pub const fn new() -> Self {
        Self {
            sequence: 0,
            link_up: 0,
            al_state: 0,
            dc_locked: 0,
            reserved: 0,
            fault_bitmap: 0,
            command_age_cycles: 0,
            deadline_misses: 0,
            dc_offset_ns: 0,
            domains: [DomainQuality::EMPTY; DOMAINS],
        }
    }
}

impl<const DOMAINS: usize> Default for QualityPage<DOMAINS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct LifecycleSummary {
    pub state: u8,
    pub stop_action: u8,
    pub gates_ready: u8,
    pub motion_permit: u8,
    pub gate_mask: u16,
    pub reserved: u16,
    pub first_blocking_code: u32,
    pub latched_fault_code: u32,
    pub transition_sequence: u64,
    pub recovery_count: u64,
}

impl LifecycleSummary {
    pub const EMPTY: Self = Self {
        state: 0,
        stop_action: 0,
        gates_ready: 0,
        motion_permit: 0,
        gate_mask: 0,
        reserved: 0,
        first_blocking_code: 0,
        latched_fault_code: 0,
        transition_sequence: 0,
        recovery_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RuntimeObservation {
    pub latest_incident_id: u64,
    pub agent_epoch: u64,
    pub observed_at_ns: u64,
    pub observation_window_ns: u64,
    pub lost_events: u32,
    pub incident_count: u32,
    pub health: u8,
    pub reserved: [u8; 7],
}

impl RuntimeObservation {
    pub const EMPTY: Self = Self {
        latest_incident_id: 0,
        agent_epoch: 0,
        observed_at_ns: 0,
        observation_window_ns: 0,
        lost_events: 0,
        incident_count: 0,
        health: 0,
        reserved: [0; 7],
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct StatePage<const AXES: usize, const IO: usize, const DOMAINS: usize> {
    pub boot_id: u64,
    pub sequence: u64,
    pub ecat_time_ns: u64,
    pub monotonic_time_ns: u64,
    pub axes: [JointState; AXES],
    pub io: [IoState; IO],
    pub quality: QualityPage<DOMAINS>,
    pub lifecycle: LifecycleSummary,
    pub runtime_observation: RuntimeObservation,
}

impl<const AXES: usize, const IO: usize, const DOMAINS: usize> StatePage<AXES, IO, DOMAINS> {
    pub const fn new(boot_id: u64) -> Self {
        Self {
            boot_id,
            sequence: 0,
            ecat_time_ns: 0,
            monotonic_time_ns: 0,
            axes: [JointState::EMPTY; AXES],
            io: [IoState::EMPTY; IO],
            quality: QualityPage::new(),
            lifecycle: LifecycleSummary::EMPTY,
            runtime_observation: RuntimeObservation::EMPTY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatePublishError {
    BootMismatch,
    ZeroSequence,
    NoFreePage,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateSnapshot<const AXES: usize, const IO: usize, const DOMAINS: usize> {
    pub publish_sequence: u64,
    pub state: StatePage<AXES, IO, DOMAINS>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EventSeverity {
    Info = 1,
    Warning = 2,
    Error = 3,
    Fault = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ProcBufEvent {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub source: u16,
    pub severity: EventSeverity,
    pub code: u16,
    pub axis_or_device: u16,
    pub value: u32,
    pub aux: u32,
}

impl ProcBufEvent {
    pub const EMPTY: Self = Self {
        sequence: 0,
        timestamp_ns: 0,
        source: 0,
        severity: EventSeverity::Info,
        code: 0,
        axis_or_device: 0,
        value: 0,
        aux: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventPushError {
    InvalidCapacity,
    Full,
}

/// Fixed-capacity SPSC event ring embedded in the ProcBuf region.
#[repr(C)]
pub struct EventRing<const EVENTS: usize> {
    events: UnsafeCell<[ProcBufEvent; EVENTS]>,
    head: AtomicU32,
    tail: AtomicU32,
    lost_events: AtomicU32,
}

// SAFETY: one producer owns head and one consumer owns tail; event records are
// copied only after release/acquire publication.
unsafe impl<const EVENTS: usize> Sync for EventRing<EVENTS> {}
unsafe impl<const EVENTS: usize> Send for EventRing<EVENTS> {}

impl<const EVENTS: usize> EventRing<EVENTS> {
    pub const fn new() -> Self {
        if EVENTS == 0 || !EVENTS.is_power_of_two() {
            panic!("ProcBuf event capacity must be a non-zero power of two");
        }
        Self {
            events: UnsafeCell::new([ProcBufEvent::EMPTY; EVENTS]),
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            lost_events: AtomicU32::new(0),
        }
    }

    pub fn push(&self, event: ProcBufEvent) -> Result<(), EventPushError> {
        if EVENTS == 0 || !EVENTS.is_power_of_two() {
            return Err(EventPushError::InvalidCapacity);
        }
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= EVENTS as u32 {
            self.lost_events.fetch_add(1, Ordering::Relaxed);
            return Err(EventPushError::Full);
        }
        let index = (head as usize) & (EVENTS - 1);
        // SAFETY: the producer owns this slot until the release head store.
        unsafe { (*self.events.get())[index] = event };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    pub fn pop(&self) -> Option<ProcBufEvent> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let index = (tail as usize) & (EVENTS - 1);
        // SAFETY: acquire observes the fully published event slot.
        let event = unsafe { (*self.events.get())[index] };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(event)
    }

    pub fn pending(&self) -> usize {
        (self
            .head
            .load(Ordering::Acquire)
            .wrapping_sub(self.tail.load(Ordering::Acquire)) as usize)
            .min(EVENTS)
    }

    pub fn lost_events(&self) -> u32 {
        self.lost_events.load(Ordering::Acquire)
    }
}

impl<const EVENTS: usize> Default for EventRing<EVENTS> {
    fn default() -> Self {
        Self::new()
    }
}

/// Two-page channel with explicit writer/reader page ownership.
#[repr(C)]
struct DoublePage<T: Copy> {
    pages: [UnsafeCell<T>; 2],
    page_state: [AtomicU32; 2],
    published: AtomicU32,
}

// SAFETY: the state machine prevents a writer from modifying a page while it
// is being copied by the single reader.
unsafe impl<T: Copy + Send> Sync for DoublePage<T> {}
unsafe impl<T: Copy + Send> Send for DoublePage<T> {}

impl<T: Copy> DoublePage<T> {
    const fn new(initial: T) -> Self {
        Self {
            pages: [UnsafeCell::new(initial), UnsafeCell::new(initial)],
            page_state: [AtomicU32::new(PAGE_FREE), AtomicU32::new(PAGE_FREE)],
            published: AtomicU32::new(0),
        }
    }

    fn publish(&self, value: T) -> Result<u64, ()> {
        let token = self.published.load(Ordering::Acquire);
        let current_index = (token & 1) as usize;
        let candidate = if token == 0 { 0 } else { current_index ^ 1 };
        let free_page = self.page_state[candidate].compare_exchange(
            PAGE_FREE,
            PAGE_WRITING,
            Ordering::Acquire,
            Ordering::Relaxed,
        );
        let stale_page = if free_page.is_err() {
            self.page_state[candidate].compare_exchange(
                PAGE_PUBLISHED,
                PAGE_WRITING,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
        } else {
            Ok(PAGE_FREE)
        };
        if stale_page.is_err() {
            return Err(());
        }

        // SAFETY: PAGE_WRITING is exclusively owned by this producer.
        unsafe { *self.pages[candidate].get() = value };
        let mut sequence = (token >> 1).wrapping_add(1);
        if sequence == 0 {
            sequence = 1;
        }
        let next_token = (sequence << 1) | candidate as u32;
        self.page_state[candidate].store(PAGE_PUBLISHED, Ordering::Release);
        self.published.store(next_token, Ordering::Release);
        Ok(sequence as u64)
    }

    fn read(&self) -> Option<(u64, T)> {
        let token = self.published.load(Ordering::Acquire);
        if token == 0 {
            return None;
        }
        let index = (token & 1) as usize;
        let stale = index ^ 1;
        let _ = self.page_state[stale].compare_exchange(
            PAGE_PUBLISHED,
            PAGE_FREE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        if self.page_state[index]
            .compare_exchange(
                PAGE_PUBLISHED,
                PAGE_READING,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_err()
        {
            return None;
        }
        // SAFETY: PAGE_READING prevents the producer from reusing this page.
        let value = unsafe { *self.pages[index].get() };
        self.page_state[index].store(PAGE_FREE, Ordering::Release);
        Some(((token >> 1) as u64, value))
    }
}

#[repr(C)]
pub struct ProcBuf<const AXES: usize, const IO: usize, const DOMAINS: usize, const EVENTS: usize> {
    header: ProcBufHeader,
    command: DoublePage<CommandPage<AXES, IO>>,
    state: DoublePage<StatePage<AXES, IO, DOMAINS>>,
    events: EventRing<EVENTS>,
}

// SAFETY: all mutable shared fields are guarded by their own atomic
// ownership protocol; the header is immutable after construction.
unsafe impl<const AXES: usize, const IO: usize, const DOMAINS: usize, const EVENTS: usize> Sync
    for ProcBuf<AXES, IO, DOMAINS, EVENTS>
{
}
unsafe impl<const AXES: usize, const IO: usize, const DOMAINS: usize, const EVENTS: usize> Send
    for ProcBuf<AXES, IO, DOMAINS, EVENTS>
{
}

impl<const AXES: usize, const IO: usize, const DOMAINS: usize, const EVENTS: usize>
    ProcBuf<AXES, IO, DOMAINS, EVENTS>
{
    pub const fn new(robot_id: u64, boot_id: u64) -> Self {
        let layout_hash = layout_hash::<AXES, IO, DOMAINS, EVENTS>();
        let region_bytes = size_of::<Self>() as u32;
        Self {
            header: ProcBufHeader::new(
                layout_hash,
                robot_id,
                boot_id,
                region_bytes,
                ProcBufDimensions {
                    axes: AXES as u16,
                    io_channels: IO as u16,
                    domains: DOMAINS as u16,
                    event_capacity: EVENTS as u16,
                },
            ),
            command: DoublePage::new(CommandPage::new(boot_id)),
            state: DoublePage::new(StatePage::new(boot_id)),
            events: EventRing::new(),
        }
    }

    pub const fn header(&self) -> ProcBufHeader {
        self.header
    }

    pub fn validate_header(&self, robot_id: u64, boot_id: u64) -> Result<(), HeaderError> {
        self.header
            .validate::<AXES, IO, DOMAINS, EVENTS>(robot_id, boot_id)
    }

    pub fn publish_command(
        &self,
        command: CommandPage<AXES, IO>,
    ) -> Result<u64, CommandPublishError> {
        if command.boot_id != self.header.boot_id {
            return Err(CommandPublishError::BootMismatch);
        }
        if !command.well_formed() {
            return Err(CommandPublishError::InvalidCommand);
        }
        self.command
            .publish(command)
            .map_err(|_| CommandPublishError::NoFreePage)
    }

    /// Read one complete command and advance the caller-owned replay floor.
    /// The floor belongs to the RT owner and must not be shared by writers.
    pub fn read_command(
        &self,
        now_ns: u64,
        last_sequence: &mut u64,
    ) -> Result<CommandSnapshot<AXES, IO>, CommandReadError> {
        let (publish_sequence, command) =
            self.command.read().ok_or(CommandReadError::Unavailable)?;
        if command.boot_id != self.header.boot_id {
            return Err(CommandReadError::BootMismatch);
        }
        if command.sequence <= *last_sequence {
            return Err(CommandReadError::Replayed);
        }
        *last_sequence = command.sequence;
        if !command.well_formed() {
            return Err(CommandReadError::InvalidCommand);
        }
        if command.deadline_ns <= now_ns {
            return Err(CommandReadError::Expired);
        }
        if command.motion_enable_request != 0 && command.permit_expires_at_ns <= now_ns {
            return Err(CommandReadError::PermitExpired);
        }
        Ok(CommandSnapshot {
            publish_sequence,
            command,
        })
    }

    pub fn publish_state(
        &self,
        state: StatePage<AXES, IO, DOMAINS>,
    ) -> Result<u64, StatePublishError> {
        if state.boot_id != self.header.boot_id {
            return Err(StatePublishError::BootMismatch);
        }
        if state.sequence == 0 {
            return Err(StatePublishError::ZeroSequence);
        }
        self.state
            .publish(state)
            .map_err(|_| StatePublishError::NoFreePage)
    }

    pub fn read_state(&self) -> Option<StateSnapshot<AXES, IO, DOMAINS>> {
        self.state
            .read()
            .map(|(publish_sequence, state)| StateSnapshot {
                publish_sequence,
                state,
            })
    }

    pub fn record_event(&self, event: ProcBufEvent) -> Result<(), EventPushError> {
        self.events.push(event)
    }

    pub fn pop_event(&self) -> Option<ProcBufEvent> {
        self.events.pop()
    }

    pub fn pending_events(&self) -> usize {
        self.events.pending()
    }

    pub fn lost_events(&self) -> u32 {
        self.events.lost_events()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestBuf = ProcBuf<2, 1, 2, 2>;

    fn command(boot_id: u64, sequence: u64, deadline_ns: u64) -> CommandPage<2, 1> {
        CommandPage {
            boot_id,
            sequence,
            deadline_ns,
            source_id: 7,
            permit_epoch: 1,
            permit_expires_at_ns: deadline_ns,
            axis_mask: 0x03,
            requested_mode: ControlMode::Csp,
            motion_enable_request: 1,
            authority: 1,
            reserved: 0,
            axes: [JointCommand::EMPTY; 2],
            io: [IoCommand::EMPTY; 1],
        }
    }

    #[test]
    fn header_rejects_wrong_layout_identity_and_boot() {
        let buffer = TestBuf::new(42, 9);
        assert_eq!(buffer.validate_header(42, 9), Ok(()));
        assert_eq!(
            buffer.validate_header(41, 9),
            Err(HeaderError::RobotIdMismatch)
        );
        assert_eq!(
            buffer.validate_header(42, 10),
            Err(HeaderError::BootIdMismatch)
        );
        let mut header = buffer.header();
        header.layout_hash ^= 1;
        assert_eq!(
            header.validate::<2, 1, 2, 2>(42, 9),
            Err(HeaderError::LayoutHashMismatch)
        );
    }

    #[test]
    fn command_channel_rejects_invalid_stale_and_replayed_commands() {
        let buffer = TestBuf::new(42, 9);
        assert_eq!(
            buffer.publish_command(command(8, 1, 100)),
            Err(CommandPublishError::BootMismatch)
        );
        let mut invalid = command(9, 1, 100);
        invalid.requested_mode = ControlMode::Unknown;
        assert_eq!(
            buffer.publish_command(invalid),
            Err(CommandPublishError::InvalidCommand)
        );
        assert_eq!(buffer.publish_command(command(9, 1, 100)), Ok(1));
        let mut floor = 0;
        let snapshot = buffer.read_command(50, &mut floor).unwrap();
        assert_eq!(snapshot.command.sequence, 1);
        assert_eq!(floor, 1);

        assert_eq!(
            buffer.read_command(50, &mut floor),
            Err(CommandReadError::Unavailable)
        );
        assert_eq!(buffer.publish_command(command(9, 1, 100)), Ok(2));
        assert_eq!(
            buffer.read_command(50, &mut floor),
            Err(CommandReadError::Replayed)
        );
        assert_eq!(buffer.publish_command(command(9, 2, 50)), Ok(3));
        assert_eq!(
            buffer.read_command(50, &mut floor),
            Err(CommandReadError::Expired)
        );
    }

    #[test]
    fn command_writer_can_replace_an_unconsumed_stale_page_without_waiting() {
        let buffer = TestBuf::new(42, 9);
        assert_eq!(buffer.publish_command(command(9, 1, 100)), Ok(1));
        assert_eq!(buffer.publish_command(command(9, 2, 100)), Ok(2));
        assert_eq!(buffer.publish_command(command(9, 3, 100)), Ok(3));
        let mut floor = 0;
        let snapshot = buffer.read_command(50, &mut floor).unwrap();
        assert_eq!(snapshot.command.sequence, 3);
    }

    #[test]
    fn state_channel_publishes_complete_snapshot_only() {
        let buffer = TestBuf::new(42, 9);
        assert!(buffer.read_state().is_none());
        let mut state = StatePage::new(9);
        state.sequence = 11;
        state.monotonic_time_ns = 1234;
        state.quality.link_up = 1;
        state.quality.domains[0].expected_wkc = 4;
        assert_eq!(buffer.publish_state(state), Ok(1));
        let snapshot = buffer.read_state().unwrap();
        assert_eq!(snapshot.publish_sequence, 1);
        assert_eq!(snapshot.state.sequence, 11);
        assert_eq!(snapshot.state.quality.domains[0].expected_wkc, 4);
        assert_eq!(
            buffer.publish_state(StatePage::new(8)),
            Err(StatePublishError::BootMismatch)
        );
        assert_eq!(
            buffer.publish_state(StatePage::new(9)),
            Err(StatePublishError::ZeroSequence)
        );
    }

    #[test]
    fn event_ring_is_bounded_and_reports_overflow() {
        let buffer = TestBuf::new(42, 9);
        for sequence in 1..=2 {
            assert_eq!(
                buffer.record_event(ProcBufEvent {
                    sequence,
                    timestamp_ns: sequence * 10,
                    source: 1,
                    severity: EventSeverity::Warning,
                    code: 7,
                    axis_or_device: 0,
                    value: 0,
                    aux: 0,
                }),
                Ok(())
            );
        }
        assert_eq!(
            buffer.record_event(ProcBufEvent::EMPTY),
            Err(EventPushError::Full)
        );
        assert_eq!(buffer.pending_events(), 2);
        assert_eq!(buffer.lost_events(), 1);
        assert_eq!(buffer.pop_event().unwrap().sequence, 1);
        assert_eq!(buffer.pop_event().unwrap().sequence, 2);
        assert!(buffer.pop_event().is_none());
    }
}
