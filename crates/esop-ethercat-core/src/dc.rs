//! Allocation-free Distributed Clocks configuration and health monitoring.
//!
//! The controller mirrors the non-blocking part of SOEM's DC setup sequence:
//! stop SYNC generation, grant EtherCAT register access, sample local system
//! time, calculate the first trigger, program cycle registers, and activate
//! SYNC0/SYNC1. The caller schedules each action through the normal control
//! request pool, so no wait or allocation is introduced into the cycle path.

use crate::control::{
    ControlError, ControlRequestPool, MAX_CONTROL_PAYLOAD, RegisterOperation, RequestHandle,
    RequestState,
};
use crate::engine::RxDatagramConsumer;
use crate::plan::DatagramPlan;
use crate::registers::{
    ESC_DC_CUC, ESC_DC_CYCLE0, ESC_DC_CYCLE1, ESC_DC_START0, ESC_DC_SYNC_ACTIVATION,
    ESC_DC_SYSTEM_OFFSET, ESC_DC_SYSTEM_TIME, ESC_PORT_COUNT, fixed_address,
};
use crate::rx_index::RxMatch;
use crate::scan::{EscDcRange, ScanPortLink, ScanRecord};
use crate::wire::{Command, DatagramHeader};

pub const DC_SYNC_DELAY_NS: u64 = 100_000_000;
const DC_DOWNSTREAM_PORTS: [usize; 3] = [3, 1, 2];
const DC_REVERSE_PORT_ORDER: [usize; ESC_PORT_COUNT] = [2, 3, 1, 0];
const DC_SYSTEM_TIME_LEN: usize = 8;
const DC_CYCLE_LEN: usize = 4;
const DC_ACTIVATION_LEN: usize = 1;
const DC_MAX_ACTION_PAYLOAD: usize = DC_SYSTEM_TIME_LEN;
const DC_CLOCK_SAMPLE_LEN: usize = 24;
const DC_CLOCK_WRITE_LEN: usize = 12;
const DC_CLOCK_MAX_ACTION_PAYLOAD: usize = DC_CLOCK_SAMPLE_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcTopologyPort {
    pub link: ScanPortLink,
    pub receive_time_ns: Option<u32>,
    pub next_slave_position: Option<u16>,
    pub next_dc_position: Option<u16>,
    pub propagation_delay_ns: Option<u32>,
}

impl DcTopologyPort {
    const EMPTY: Self = Self {
        link: ScanPortLink::NONE,
        receive_time_ns: None,
        next_slave_position: None,
        next_dc_position: None,
        propagation_delay_ns: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcTopologySlave {
    pub position: u16,
    pub station_address: u16,
    pub dc_supported: bool,
    pub system_time_capable: bool,
    pub range: EscDcRange,
    pub ports: [DcTopologyPort; ESC_PORT_COUNT],
    pub transmission_delay_ns: Option<u32>,
}

impl DcTopologySlave {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        dc_supported: false,
        system_time_capable: false,
        range: EscDcRange::Bits32,
        ports: [DcTopologyPort::EMPTY; ESC_PORT_COUNT],
        transmission_delay_ns: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcTopologyError {
    CapacityExceeded,
    DuplicatePosition(u16),
    MissingReceiveTimes(u16),
    TopologyOverrun {
        position: u16,
        port: u8,
    },
    UnreachablePosition(u16),
    InvalidReference(u16),
    DelayUnderflow {
        position: u16,
        port: u8,
        round_trip_ns: u32,
        downstream_round_trip_ns: u32,
    },
    DelayOverflow(u16),
}

#[derive(Clone, Copy)]
struct DcTopologyBuildFrame {
    slave_index: usize,
    next_port: u8,
}

impl DcTopologyBuildFrame {
    const EMPTY: Self = Self {
        slave_index: 0,
        next_port: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcTopology<const MAX_SLAVES: usize> {
    slaves: [DcTopologySlave; MAX_SLAVES],
    len: usize,
    reference_position: Option<u16>,
}

impl<const MAX_SLAVES: usize> DcTopology<MAX_SLAVES> {
    pub const fn empty() -> Self {
        Self {
            slaves: [DcTopologySlave::EMPTY; MAX_SLAVES],
            len: 0,
            reference_position: None,
        }
    }

    pub fn build(
        records: &[ScanRecord],
        reference_position: Option<u16>,
    ) -> Result<Self, DcTopologyError> {
        if records.len() > MAX_SLAVES {
            return Err(DcTopologyError::CapacityExceeded);
        }

        let mut topology = Self::empty();
        topology.len = records.len();
        topology.reference_position = reference_position;

        for (index, record) in records.iter().copied().enumerate() {
            if records[..index]
                .iter()
                .any(|existing| existing.position == record.position)
            {
                return Err(DcTopologyError::DuplicatePosition(record.position));
            }
            if record.dc.supported && record.dc.receive_times.is_none() {
                return Err(DcTopologyError::MissingReceiveTimes(record.position));
            }

            let mut ports = [DcTopologyPort::EMPTY; ESC_PORT_COUNT];
            for port in 0..ESC_PORT_COUNT {
                ports[port].link = record.port_links[port];
                ports[port].receive_time_ns = record.dc.receive_times.map(|times| times[port]);
            }
            topology.slaves[index] = DcTopologySlave {
                position: record.position,
                station_address: record.station_address,
                dc_supported: record.dc.supported,
                system_time_capable: record.dc.can_be_reference_clock(),
                range: record.dc.range,
                ports,
                transmission_delay_ns: None,
            };
        }

        if topology.len == 0 {
            if let Some(reference) = reference_position {
                return Err(DcTopologyError::InvalidReference(reference));
            }
            return Ok(topology);
        }

        topology.build_physical_tree()?;
        topology.calculate_dc_links()?;
        topology.calculate_transmission_delays()?;
        Ok(topology)
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn reference_position(&self) -> Option<u16> {
        self.reference_position
    }

    pub fn slaves(&self) -> &[DcTopologySlave] {
        &self.slaves[..self.len]
    }

    pub fn slave(&self, position: u16) -> Option<&DcTopologySlave> {
        self.position_index(position)
            .map(|index| &self.slaves[index])
    }

    pub fn transmission_delay_ns(&self, position: u16) -> Option<u32> {
        self.slave(position)
            .and_then(|slave| slave.transmission_delay_ns)
    }

    fn build_physical_tree(&mut self) -> Result<(), DcTopologyError> {
        let mut stack = [DcTopologyBuildFrame::EMPTY; MAX_SLAVES];
        stack[0] = DcTopologyBuildFrame {
            slave_index: 0,
            next_port: 0,
        };
        let mut depth = 1;
        let mut next_slave_index = 1;

        while depth > 0 {
            let frame_index = depth - 1;
            let port_cursor = stack[frame_index].next_port as usize;
            if port_cursor >= DC_DOWNSTREAM_PORTS.len() {
                depth -= 1;
                continue;
            }

            stack[frame_index].next_port += 1;
            let slave_index = stack[frame_index].slave_index;
            let port = DC_DOWNSTREAM_PORTS[port_cursor];
            if self.slaves[slave_index].ports[port].link.loop_closed {
                continue;
            }
            if next_slave_index >= self.len {
                return Err(DcTopologyError::TopologyOverrun {
                    position: self.slaves[slave_index].position,
                    port: port as u8,
                });
            }

            let child_index = next_slave_index;
            next_slave_index += 1;
            let parent_position = self.slaves[slave_index].position;
            let child_position = self.slaves[child_index].position;
            self.slaves[slave_index].ports[port].next_slave_position = Some(child_position);
            self.slaves[child_index].ports[0].next_slave_position = Some(parent_position);
            if depth >= MAX_SLAVES {
                return Err(DcTopologyError::CapacityExceeded);
            }
            stack[depth] = DcTopologyBuildFrame {
                slave_index: child_index,
                next_port: 0,
            };
            depth += 1;
        }

        if next_slave_index != self.len {
            return Err(DcTopologyError::UnreachablePosition(
                self.slaves[next_slave_index].position,
            ));
        }
        Ok(())
    }

    fn calculate_dc_links(&mut self) -> Result<(), DcTopologyError> {
        for source_index in 0..self.len {
            if !self.slaves[source_index].dc_supported {
                continue;
            }
            for port in DC_DOWNSTREAM_PORTS {
                let Some(child_position) =
                    self.slaves[source_index].ports[port].next_slave_position
                else {
                    continue;
                };
                let child_index = self
                    .position_index(child_position)
                    .ok_or(DcTopologyError::UnreachablePosition(child_position))?;
                let Some(next_dc_index) = self.find_first_dc_in_subtree(child_index)? else {
                    continue;
                };
                let round_trip_ns = self.port_round_trip(source_index, port)?;
                let downstream_round_trip_ns = self.internal_round_trip_sum(next_dc_index)?;
                let delay_ns = round_trip_ns.checked_sub(downstream_round_trip_ns).ok_or(
                    DcTopologyError::DelayUnderflow {
                        position: self.slaves[source_index].position,
                        port: port as u8,
                        round_trip_ns,
                        downstream_round_trip_ns,
                    },
                )? / 2;

                let source_position = self.slaves[source_index].position;
                let next_dc_position = self.slaves[next_dc_index].position;
                self.slaves[source_index].ports[port].next_dc_position = Some(next_dc_position);
                self.slaves[source_index].ports[port].propagation_delay_ns = Some(delay_ns);
                self.slaves[next_dc_index].ports[0].next_dc_position = Some(source_position);
                self.slaves[next_dc_index].ports[0].propagation_delay_ns = Some(delay_ns);
            }
        }
        Ok(())
    }

    fn calculate_transmission_delays(&mut self) -> Result<(), DcTopologyError> {
        let Some(reference_position) = self.reference_position else {
            return Ok(());
        };
        let reference_index = self
            .position_index(reference_position)
            .filter(|index| self.slaves[*index].system_time_capable)
            .ok_or(DcTopologyError::InvalidReference(reference_position))?;

        let mut visited = [false; MAX_SLAVES];
        let mut stack_indices = [0usize; MAX_SLAVES];
        let mut stack_delays = [0u32; MAX_SLAVES];
        let mut depth = 1;
        visited[reference_index] = true;
        self.slaves[reference_index].transmission_delay_ns = Some(0);
        stack_indices[0] = reference_index;

        while depth > 0 {
            depth -= 1;
            let source_index = stack_indices[depth];
            let source_delay = stack_delays[depth];
            for port in 0..ESC_PORT_COUNT {
                let Some(peer_position) = self.slaves[source_index].ports[port].next_dc_position
                else {
                    continue;
                };
                let Some(link_delay) = self.slaves[source_index].ports[port].propagation_delay_ns
                else {
                    continue;
                };
                let peer_index = self
                    .position_index(peer_position)
                    .ok_or(DcTopologyError::UnreachablePosition(peer_position))?;
                if visited[peer_index] {
                    continue;
                }
                let peer_delay =
                    source_delay
                        .checked_add(link_delay)
                        .ok_or(DcTopologyError::DelayOverflow(
                            self.slaves[peer_index].position,
                        ))?;
                visited[peer_index] = true;
                self.slaves[peer_index].transmission_delay_ns = Some(peer_delay);
                if depth >= MAX_SLAVES {
                    return Err(DcTopologyError::CapacityExceeded);
                }
                stack_indices[depth] = peer_index;
                stack_delays[depth] = peer_delay;
                depth += 1;
            }
        }
        Ok(())
    }

    fn find_first_dc_in_subtree(
        &self,
        start_index: usize,
    ) -> Result<Option<usize>, DcTopologyError> {
        let mut stack = [0usize; MAX_SLAVES];
        let mut depth = 1;
        stack[0] = start_index;
        while depth > 0 {
            depth -= 1;
            let slave_index = stack[depth];
            if self.slaves[slave_index].dc_supported {
                return Ok(Some(slave_index));
            }
            for port in DC_DOWNSTREAM_PORTS.iter().rev().copied() {
                let Some(position) = self.slaves[slave_index].ports[port].next_slave_position
                else {
                    continue;
                };
                let child_index = self
                    .position_index(position)
                    .ok_or(DcTopologyError::UnreachablePosition(position))?;
                if depth >= MAX_SLAVES {
                    return Err(DcTopologyError::CapacityExceeded);
                }
                stack[depth] = child_index;
                depth += 1;
            }
        }
        Ok(None)
    }

    fn internal_round_trip_sum(&self, slave_index: usize) -> Result<u32, DcTopologyError> {
        let mut sum = 0u32;
        for port in DC_DOWNSTREAM_PORTS {
            if self.slaves[slave_index].ports[port]
                .next_slave_position
                .is_none()
            {
                continue;
            }
            sum = sum
                .checked_add(self.port_round_trip(slave_index, port)?)
                .ok_or(DcTopologyError::DelayOverflow(
                    self.slaves[slave_index].position,
                ))?;
        }
        Ok(sum)
    }

    fn port_round_trip(&self, slave_index: usize, port: usize) -> Result<u32, DcTopologyError> {
        let slave = &self.slaves[slave_index];
        let previous_port = self.previous_connected_port(slave_index, port);
        let receive_time = slave.ports[port]
            .receive_time_ns
            .ok_or(DcTopologyError::MissingReceiveTimes(slave.position))?;
        let previous_receive_time = slave.ports[previous_port]
            .receive_time_ns
            .ok_or(DcTopologyError::MissingReceiveTimes(slave.position))?;
        Ok(receive_time.wrapping_sub(previous_receive_time))
    }

    fn previous_connected_port(&self, slave_index: usize, mut port: usize) -> usize {
        loop {
            port = DC_REVERSE_PORT_ORDER[port];
            if port == 0
                || self.slaves[slave_index].ports[port]
                    .next_slave_position
                    .is_some()
            {
                return port;
            }
        }
    }

    fn position_index(&self, position: u16) -> Option<usize> {
        self.slaves[..self.len]
            .iter()
            .position(|slave| slave.position == position)
    }
}

impl<const MAX_SLAVES: usize> Default for DcTopology<MAX_SLAVES> {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcClockConfig {
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
}

impl DcClockConfig {
    pub const fn new() -> Self {
        Self {
            timeout_ns: 1_000_000_000,
            request_timeout_ns: 1_000_000,
        }
    }

    pub const fn validate(self) -> Result<(), DcClockError> {
        if self.timeout_ns == 0 || self.request_timeout_ns == 0 {
            return Err(DcClockError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl Default for DcClockConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DcClockActionKind {
    ReadClock = 0,
    WriteOffsetDelay = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcClockAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub position: u16,
    pub station_address: u16,
    pub kind: DcClockActionKind,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; DC_CLOCK_MAX_ACTION_PAYLOAD],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl DcClockAction {
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

    pub const fn response_len(&self) -> usize {
        self.read_len as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcClockPhase {
    Idle,
    ReadingClock,
    WritingOffsetDelay,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcClockProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcClockError {
    Busy,
    NotStarted,
    NoPendingAction,
    InvalidConfiguration,
    DeadlineOverflow,
    MissingReference,
    InvalidReference(u16),
    MissingTransmissionDelay(u16),
    ActionMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    MonotonicTimeRegressed,
    ApplicationTimeOverflow,
    OffsetDeltaOutOfRange,
    Control(ControlError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DcClockPlanEntry {
    position: u16,
    station_address: u16,
    range: EscDcRange,
    transmission_delay_ns: u32,
}

impl DcClockPlanEntry {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        range: EscDcRange::Bits32,
        transmission_delay_ns: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcClockProgrammedSlave {
    pub position: u16,
    pub station_address: u16,
    pub range: EscDcRange,
    pub sampled_system_time_ns: u64,
    pub target_application_time_ns: u64,
    pub old_offset: u64,
    pub new_offset: u64,
    pub transmission_delay_ns: u32,
}

impl DcClockProgrammedSlave {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        range: EscDcRange::Bits32,
        sampled_system_time_ns: 0,
        target_application_time_ns: 0,
        old_offset: 0,
        new_offset: 0,
        transmission_delay_ns: 0,
    };
}

pub struct DcClockController<const MAX_SLAVES: usize> {
    phase: DcClockPhase,
    config: DcClockConfig,
    generation: u16,
    configuration_deadline_ns: u64,
    base_application_time_ns: u64,
    start_monotonic_ns: u64,
    plan: [DcClockPlanEntry; MAX_SLAVES],
    plan_len: usize,
    current_index: usize,
    completed_count: usize,
    programmed: [DcClockProgrammedSlave; MAX_SLAVES],
    published_len: usize,
    pending: Option<DcClockAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<DcClockError>,
}

impl<const MAX_SLAVES: usize> DcClockController<MAX_SLAVES> {
    pub const fn new() -> Self {
        Self {
            phase: DcClockPhase::Idle,
            config: DcClockConfig::new(),
            generation: 0,
            configuration_deadline_ns: 0,
            base_application_time_ns: 0,
            start_monotonic_ns: 0,
            plan: [DcClockPlanEntry::EMPTY; MAX_SLAVES],
            plan_len: 0,
            current_index: 0,
            completed_count: 0,
            programmed: [DcClockProgrammedSlave::EMPTY; MAX_SLAVES],
            published_len: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> DcClockPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<DcClockAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<DcClockError> {
        self.last_error
    }

    pub const fn planned_count(&self) -> usize {
        self.plan_len
    }

    pub const fn completed_count(&self) -> usize {
        self.completed_count
    }

    pub fn programmed_slaves(&self) -> &[DcClockProgrammedSlave] {
        &self.programmed[..self.published_len]
    }

    pub fn programmed_slave(&self, position: u16) -> Option<&DcClockProgrammedSlave> {
        self.programmed_slaves()
            .iter()
            .find(|slave| slave.position == position)
    }

    pub fn start(
        &mut self,
        config: DcClockConfig,
        topology: &DcTopology<MAX_SLAVES>,
        generation: u16,
        application_time_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<(), DcClockError> {
        if !matches!(
            self.phase,
            DcClockPhase::Idle | DcClockPhase::Complete | DcClockPhase::Faulted
        ) {
            return Err(DcClockError::Busy);
        }
        if let Err(error) = config.validate() {
            return self.start_failed(error);
        }
        let configuration_deadline_ns = match monotonic_now_ns.checked_add(config.timeout_ns) {
            Some(deadline) => deadline,
            None => return self.start_failed(DcClockError::DeadlineOverflow),
        };

        let has_system_time_clock = topology
            .slaves()
            .iter()
            .any(|slave| slave.system_time_capable);
        if has_system_time_clock {
            let reference_position = match topology.reference_position() {
                Some(position) => position,
                None => return self.start_failed(DcClockError::MissingReference),
            };
            let Some(reference) = topology.slave(reference_position) else {
                return self.start_failed(DcClockError::InvalidReference(reference_position));
            };
            if !reference.system_time_capable || reference.transmission_delay_ns != Some(0) {
                return self.start_failed(DcClockError::InvalidReference(reference_position));
            }
        }

        let mut plan = [DcClockPlanEntry::EMPTY; MAX_SLAVES];
        let mut plan_len = 0usize;
        for slave in topology.slaves().iter().copied() {
            if !slave.system_time_capable {
                continue;
            }
            let transmission_delay_ns = match slave.transmission_delay_ns {
                Some(delay) => delay,
                None => {
                    return self
                        .start_failed(DcClockError::MissingTransmissionDelay(slave.position));
                }
            };
            plan[plan_len] = DcClockPlanEntry {
                position: slave.position,
                station_address: slave.station_address,
                range: slave.range,
                transmission_delay_ns,
            };
            plan_len += 1;
        }

        self.phase = if plan_len == 0 {
            DcClockPhase::Complete
        } else {
            DcClockPhase::ReadingClock
        };
        self.config = config;
        self.generation = generation;
        self.configuration_deadline_ns = configuration_deadline_ns;
        self.base_application_time_ns = application_time_ns;
        self.start_monotonic_ns = monotonic_now_ns;
        self.plan = plan;
        self.plan_len = plan_len;
        self.current_index = 0;
        self.completed_count = 0;
        self.programmed = [DcClockProgrammedSlave::EMPTY; MAX_SLAVES];
        self.published_len = 0;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<DcClockAction>, DcClockError> {
        if self.phase == DcClockPhase::Idle {
            return Err(DcClockError::NotStarted);
        }
        if matches!(self.phase, DcClockPhase::Complete | DcClockPhase::Faulted) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.configuration_deadline_ns {
            return self.fail(DcClockError::Timeout);
        }
        let request_deadline_ns = match now_ns.checked_add(self.config.request_timeout_ns) {
            Some(deadline) => deadline.min(self.configuration_deadline_ns),
            None => return self.fail(DcClockError::DeadlineOverflow),
        };
        let entry = self.plan[self.current_index];
        let (kind, operation, register, read_len, write_payload, write_len) = match self.phase {
            DcClockPhase::ReadingClock => (
                DcClockActionKind::ReadClock,
                RegisterOperation::Read,
                ESC_DC_SYSTEM_TIME,
                DC_CLOCK_SAMPLE_LEN,
                [0; DC_CLOCK_MAX_ACTION_PAYLOAD],
                0,
            ),
            DcClockPhase::WritingOffsetDelay => {
                let evidence = self.programmed[self.current_index];
                let mut payload = [0; DC_CLOCK_MAX_ACTION_PAYLOAD];
                payload[..8].copy_from_slice(&evidence.new_offset.to_le_bytes());
                payload[8..DC_CLOCK_WRITE_LEN]
                    .copy_from_slice(&entry.transmission_delay_ns.to_le_bytes());
                (
                    DcClockActionKind::WriteOffsetDelay,
                    RegisterOperation::Write,
                    ESC_DC_SYSTEM_OFFSET,
                    0,
                    payload,
                    DC_CLOCK_WRITE_LEN,
                )
            }
            DcClockPhase::Idle | DcClockPhase::Complete | DcClockPhase::Faulted => {
                return Ok(None);
            }
        };
        let action = DcClockAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            position: entry.position,
            station_address: entry.station_address,
            kind,
            operation,
            address: fixed_address(entry.station_address, register),
            read_len: read_len as u16,
            write_payload,
            write_len: write_len as u8,
            deadline_ns: request_deadline_ns,
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
        action: DcClockAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<DcClockProgress, DcClockError> {
        if self.pending != Some(action) {
            return self.fail(DcClockError::ActionMismatch);
        }
        if action.generation != generation {
            return self.fail(DcClockError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(DcClockError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(DcClockError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(DcClockError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            DcClockPhase::ReadingClock => {
                let sampled_system_time_ns = u64::from_le_bytes(match payload[..8].try_into() {
                    Ok(bytes) => bytes,
                    Err(_) => return self.fail(DcClockError::PayloadLengthMismatch),
                });
                let old_offset = u64::from_le_bytes(match payload[16..24].try_into() {
                    Ok(bytes) => bytes,
                    Err(_) => return self.fail(DcClockError::PayloadLengthMismatch),
                });
                let elapsed = match now_ns.checked_sub(self.start_monotonic_ns) {
                    Some(elapsed) => elapsed,
                    None => return self.fail(DcClockError::MonotonicTimeRegressed),
                };
                let target_application_time_ns =
                    match self.base_application_time_ns.checked_add(elapsed) {
                        Some(target) => target,
                        None => return self.fail(DcClockError::ApplicationTimeOverflow),
                    };
                let entry = self.plan[self.current_index];
                let delta = match entry.range {
                    EscDcRange::Bits32 => target_application_time_ns
                        .wrapping_sub(sampled_system_time_ns)
                        as u32 as i32 as i64,
                    EscDcRange::Bits64 => {
                        let delta =
                            target_application_time_ns as i128 - sampled_system_time_ns as i128;
                        if !(i64::MIN as i128..=i64::MAX as i128).contains(&delta) {
                            return self.fail(DcClockError::OffsetDeltaOutOfRange);
                        }
                        delta as i64
                    }
                };
                self.programmed[self.current_index] = DcClockProgrammedSlave {
                    position: entry.position,
                    station_address: entry.station_address,
                    range: entry.range,
                    sampled_system_time_ns,
                    target_application_time_ns,
                    old_offset,
                    new_offset: old_offset.wrapping_add(delta as u64),
                    transmission_delay_ns: entry.transmission_delay_ns,
                };
                self.phase = DcClockPhase::WritingOffsetDelay;
                DcClockProgress::Advanced
            }
            DcClockPhase::WritingOffsetDelay => {
                self.completed_count += 1;
                self.current_index += 1;
                if self.current_index == self.plan_len {
                    self.phase = DcClockPhase::Complete;
                    self.published_len = self.plan_len;
                    DcClockProgress::Complete
                } else {
                    self.phase = DcClockPhase::ReadingClock;
                    DcClockProgress::Advanced
                }
            }
            DcClockPhase::Idle | DcClockPhase::Complete | DcClockPhase::Faulted => {
                return self.fail(DcClockError::NoPendingAction);
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
    ) -> Result<DcClockProgress, DcClockError> {
        let action = match self.pending {
            Some(action) => action,
            None => return self.fail(DcClockError::NoPendingAction),
        };
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
                ) {
                    let _ = pool.release(handle);
                    return self.fail(DcClockError::ActionMismatch);
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
                    return self.fail(DcClockError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(DcClockError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(DcClockError::Control(error));
            }
            Some(_) => return Err(DcClockError::Control(ControlError::InvalidState)),
            None => return self.fail(DcClockError::Control(ControlError::InvalidHandle)),
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
            (Ok(_), Err(error)) => self.fail(DcClockError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(
        &mut self,
        action: DcClockAction,
        now_ns: u64,
    ) -> Result<DcClockProgress, DcClockError> {
        if self.pending != Some(action) {
            return self.fail(DcClockError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(DcClockError::Timeout);
        }
        self.fail(DcClockError::Timeout)
    }

    fn start_failed<T>(&mut self, error: DcClockError) -> Result<T, DcClockError> {
        self.phase = DcClockPhase::Faulted;
        self.plan_len = 0;
        self.current_index = 0;
        self.completed_count = 0;
        self.published_len = 0;
        self.pending = None;
        self.last_error = Some(error);
        Err(error)
    }

    fn fail<T>(&mut self, error: DcClockError) -> Result<T, DcClockError> {
        self.pending = None;
        self.phase = DcClockPhase::Faulted;
        self.published_len = 0;
        self.last_error = Some(error);
        Err(error)
    }
}

impl<const MAX_SLAVES: usize> Default for DcClockController<MAX_SLAVES> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcSyncMode {
    Sync0,
    Sync0Sync1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcConfig {
    pub mode: DcSyncMode,
    pub activate: bool,
    pub cycle_time0_ns: u32,
    pub cycle_time1_ns: u32,
    pub shift_ns: i32,
    pub sync_delay_ns: u64,
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
}

impl DcConfig {
    pub const fn sync0(cycle_time0_ns: u32, shift_ns: i32) -> Self {
        Self {
            mode: DcSyncMode::Sync0,
            activate: true,
            cycle_time0_ns,
            cycle_time1_ns: 0,
            shift_ns,
            sync_delay_ns: DC_SYNC_DELAY_NS,
            timeout_ns: 1_000_000_000,
            request_timeout_ns: 1_000_000,
        }
    }

    pub const fn sync0_sync1(cycle_time0_ns: u32, cycle_time1_ns: u32, shift_ns: i32) -> Self {
        Self {
            mode: DcSyncMode::Sync0Sync1,
            activate: true,
            cycle_time0_ns,
            cycle_time1_ns,
            shift_ns,
            sync_delay_ns: DC_SYNC_DELAY_NS,
            timeout_ns: 1_000_000_000,
            request_timeout_ns: 1_000_000,
        }
    }

    pub const fn validate(&self) -> Result<(), DcError> {
        if self.cycle_time0_ns == 0 || self.timeout_ns == 0 || self.request_timeout_ns == 0 {
            return Err(DcError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DcActionKind {
    DisableSync = 0,
    GrantEthercatAccess = 1,
    ReadSystemTime = 2,
    WriteStartTime = 3,
    WriteCycle0 = 4,
    WriteCycle1 = 5,
    ActivateSync = 6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub station_address: u16,
    pub kind: DcActionKind,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; DC_MAX_ACTION_PAYLOAD],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl DcAction {
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

    pub const fn response_len(&self) -> usize {
        self.read_len as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcPhase {
    Idle,
    DisablingSync,
    GrantingEthercatAccess,
    ReadingSystemTime,
    WritingStartTime,
    WritingCycle0,
    WritingCycle1,
    ActivatingSync,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcError {
    Busy,
    NotStarted,
    NoPendingAction,
    InvalidConfiguration,
    ActionMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    SystemTimeOverflow,
    Control(ControlError),
}

pub struct DcController {
    phase: DcPhase,
    config: DcConfig,
    generation: u16,
    station_address: u16,
    configuration_deadline_ns: u64,
    system_time_ns: u64,
    first_trigger_ns: u64,
    pending: Option<DcAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<DcError>,
}

impl DcController {
    pub const fn new() -> Self {
        Self {
            phase: DcPhase::Idle,
            config: DcConfig::sync0(0, 0),
            generation: 0,
            station_address: 0,
            configuration_deadline_ns: 0,
            system_time_ns: 0,
            first_trigger_ns: 0,
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> DcPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<DcAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<DcError> {
        self.last_error
    }

    pub const fn system_time_ns(&self) -> u64 {
        self.system_time_ns
    }

    pub const fn first_trigger_ns(&self) -> u64 {
        self.first_trigger_ns
    }

    pub fn start(
        &mut self,
        config: DcConfig,
        station_address: u16,
        generation: u16,
        now_ns: u64,
    ) -> Result<(), DcError> {
        if !matches!(
            self.phase,
            DcPhase::Idle | DcPhase::Complete | DcPhase::Faulted
        ) {
            return Err(DcError::Busy);
        }
        config.validate()?;
        self.phase = DcPhase::DisablingSync;
        self.config = config;
        self.station_address = station_address;
        self.generation = generation;
        self.configuration_deadline_ns = now_ns.saturating_add(config.timeout_ns);
        self.system_time_ns = 0;
        self.first_trigger_ns = 0;
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<DcAction>, DcError> {
        if self.phase == DcPhase::Idle {
            return Err(DcError::NotStarted);
        }
        if matches!(self.phase, DcPhase::Complete | DcPhase::Faulted) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.configuration_deadline_ns {
            return self.fail(DcError::Timeout);
        }

        let (kind, operation, register, read_len, payload, write_len) = match self.phase {
            DcPhase::DisablingSync => (
                DcActionKind::DisableSync,
                RegisterOperation::Write,
                ESC_DC_SYNC_ACTIVATION,
                0,
                [0; DC_MAX_ACTION_PAYLOAD],
                DC_ACTIVATION_LEN,
            ),
            DcPhase::GrantingEthercatAccess => (
                DcActionKind::GrantEthercatAccess,
                RegisterOperation::Write,
                ESC_DC_CUC,
                0,
                [0; DC_MAX_ACTION_PAYLOAD],
                DC_ACTIVATION_LEN,
            ),
            DcPhase::ReadingSystemTime => (
                DcActionKind::ReadSystemTime,
                RegisterOperation::Read,
                ESC_DC_SYSTEM_TIME,
                DC_SYSTEM_TIME_LEN,
                [0; DC_MAX_ACTION_PAYLOAD],
                0,
            ),
            DcPhase::WritingStartTime => (
                DcActionKind::WriteStartTime,
                RegisterOperation::Write,
                ESC_DC_START0,
                0,
                self.first_trigger_ns.to_le_bytes(),
                DC_SYSTEM_TIME_LEN,
            ),
            DcPhase::WritingCycle0 => (
                DcActionKind::WriteCycle0,
                RegisterOperation::Write,
                ESC_DC_CYCLE0,
                0,
                u32_payload(self.config.cycle_time0_ns),
                DC_CYCLE_LEN,
            ),
            DcPhase::WritingCycle1 => (
                DcActionKind::WriteCycle1,
                RegisterOperation::Write,
                ESC_DC_CYCLE1,
                0,
                u32_payload(self.config.cycle_time1_ns),
                DC_CYCLE_LEN,
            ),
            DcPhase::ActivatingSync => (
                DcActionKind::ActivateSync,
                RegisterOperation::Write,
                ESC_DC_SYNC_ACTIVATION,
                0,
                [self.activation_value(), 0, 0, 0, 0, 0, 0, 0],
                DC_ACTIVATION_LEN,
            ),
            DcPhase::Idle | DcPhase::Complete | DcPhase::Faulted => return Ok(None),
        };
        let action = DcAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            kind,
            operation,
            address: fixed_address(self.station_address, register),
            read_len: read_len as u16,
            write_payload: payload,
            write_len: write_len as u8,
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
        action: DcAction,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<DcProgress, DcError> {
        if self.pending != Some(action) {
            return self.fail(DcError::ActionMismatch);
        }
        if action.generation != generation {
            return self.fail(DcError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(DcError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(DcError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.response_len() {
            return self.fail(DcError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            DcPhase::DisablingSync => {
                self.phase = if self.config.activate {
                    DcPhase::GrantingEthercatAccess
                } else {
                    DcPhase::Complete
                };
                if self.phase == DcPhase::Complete {
                    DcProgress::Complete
                } else {
                    DcProgress::Advanced
                }
            }
            DcPhase::GrantingEthercatAccess => {
                self.phase = DcPhase::ReadingSystemTime;
                DcProgress::Advanced
            }
            DcPhase::ReadingSystemTime => {
                let bytes: [u8; DC_SYSTEM_TIME_LEN] = match payload.try_into() {
                    Ok(bytes) => bytes,
                    Err(_) => return self.fail(DcError::PayloadLengthMismatch),
                };
                self.system_time_ns = u64::from_le_bytes(bytes);
                self.first_trigger_ns = match self.calculate_first_trigger() {
                    Ok(first_trigger_ns) => first_trigger_ns,
                    Err(error) => return self.fail(error),
                };
                self.phase = DcPhase::WritingStartTime;
                DcProgress::Advanced
            }
            DcPhase::WritingStartTime => {
                self.phase = DcPhase::WritingCycle0;
                DcProgress::Advanced
            }
            DcPhase::WritingCycle0 => {
                self.phase = if self.config.mode == DcSyncMode::Sync0Sync1 {
                    DcPhase::WritingCycle1
                } else {
                    DcPhase::ActivatingSync
                };
                DcProgress::Advanced
            }
            DcPhase::WritingCycle1 => {
                self.phase = DcPhase::ActivatingSync;
                DcProgress::Advanced
            }
            DcPhase::ActivatingSync => {
                self.phase = DcPhase::Complete;
                DcProgress::Complete
            }
            DcPhase::Idle | DcPhase::Complete | DcPhase::Faulted => {
                return self.fail(DcError::NoPendingAction);
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
    ) -> Result<DcProgress, DcError> {
        let action = match self.pending {
            Some(action) => action,
            None => return self.fail(DcError::NoPendingAction),
        };
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
                ) {
                    let _ = pool.release(handle);
                    return self.fail(DcError::ActionMismatch);
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
                    return self.fail(DcError::ActionMismatch);
                }
                let error = request.last_error().unwrap_or(ControlError::InvalidState);
                if let Err(release_error) = pool.release(handle) {
                    return self.fail(DcError::Control(release_error));
                }
                if error == ControlError::Timeout {
                    return self.timeout(action, now_ns);
                }
                return self.fail(DcError::Control(error));
            }
            Some(_) => return Err(DcError::Control(ControlError::InvalidState)),
            None => return self.fail(DcError::Control(ControlError::InvalidHandle)),
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
            (Ok(_), Err(error)) => self.fail(DcError::Control(error)),
            (Err(error), _) => Err(error),
        }
    }

    pub fn timeout(&mut self, action: DcAction, now_ns: u64) -> Result<DcProgress, DcError> {
        if self.pending != Some(action) {
            return self.fail(DcError::ActionMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(DcError::Timeout);
        }
        self.fail(DcError::Timeout)
    }

    fn activation_value(&self) -> u8 {
        if !self.config.activate {
            0
        } else {
            match self.config.mode {
                DcSyncMode::Sync0 => 0x03,
                DcSyncMode::Sync0Sync1 => 0x07,
            }
        }
    }

    fn true_cycle_ns(&self) -> u64 {
        match self.config.mode {
            DcSyncMode::Sync0 => self.config.cycle_time0_ns as u64,
            DcSyncMode::Sync0Sync1 => {
                let cycle0 = self.config.cycle_time0_ns as u64;
                ((self.config.cycle_time1_ns as u64 / cycle0) + 1) * cycle0
            }
        }
    }

    fn calculate_first_trigger(&self) -> Result<u64, DcError> {
        let cycle_ns = self.true_cycle_ns();
        let base = self
            .system_time_ns
            .checked_add(self.config.sync_delay_ns)
            .ok_or(DcError::SystemTimeOverflow)?;
        let rounded = (base / cycle_ns)
            .checked_add(1)
            .and_then(|periods| periods.checked_mul(cycle_ns))
            .ok_or(DcError::SystemTimeOverflow)?;
        let trigger = rounded as i128 + self.config.shift_ns as i128;
        if !(0..=u64::MAX as i128).contains(&trigger) {
            return Err(DcError::SystemTimeOverflow);
        }
        Ok(trigger as u64)
    }

    fn fail<T>(&mut self, error: DcError) -> Result<T, DcError> {
        self.pending = None;
        self.phase = DcPhase::Faulted;
        self.last_error = Some(error);
        Err(error)
    }
}

impl Default for DcController {
    fn default() -> Self {
        Self::new()
    }
}

/// Static cyclic reference-clock synchronization slot.
///
/// The slot is added to a `FramePlan` as an FRMW datagram. `prepare` updates
/// only its eight-byte application-time field in the caller-owned process
/// image, and `complete` turns the returned reference-clock time into a DC
/// monitor sample. A slot permits one in-flight generation, matching the
/// master's one RX-index entry per datagram index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcCyclicConfig {
    pub reference_station: u16,
    pub datagram_index: u8,
    pub payload_offset: usize,
    pub expected_wkc: u16,
}

impl DcCyclicConfig {
    pub const fn new(reference_station: u16, datagram_index: u8, payload_offset: usize) -> Self {
        Self {
            reference_station,
            datagram_index,
            payload_offset,
            expected_wkc: 1,
        }
    }

    pub const fn datagram_plan(&self) -> DatagramPlan {
        DatagramPlan {
            command: Command::Frmw,
            index: self.datagram_index,
            address: fixed_address(self.reference_station, ESC_DC_SYSTEM_TIME),
            payload_offset: self.payload_offset,
            payload_len: DC_SYSTEM_TIME_LEN,
            expected_wkc: self.expected_wkc,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcCyclicError {
    Busy,
    MissingResponse,
    InvalidConfiguration,
    ProcessImageOutOfBounds,
    ApplicationTimeRegressed,
    UnexpectedDatagram,
    GenerationMismatch,
    PayloadLengthMismatch,
    WorkingCounterMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DcCyclicPending {
    generation: u16,
    application_time_ns: u64,
}

pub struct DcCyclicSync {
    config: DcCyclicConfig,
    monitor: DcMonitor,
    pending: Option<DcCyclicPending>,
    has_last_application_time: bool,
    last_application_time_ns: u64,
    last_reference_time_ns: u64,
    last_sync_cycle: u64,
    sync_count: u64,
    unlock_count: u64,
    last_error: Option<DcCyclicError>,
}

impl DcCyclicSync {
    pub const fn new(config: DcCyclicConfig, monitor: DcMonitor) -> Self {
        Self {
            config,
            monitor,
            pending: None,
            has_last_application_time: false,
            last_application_time_ns: 0,
            last_reference_time_ns: 0,
            last_sync_cycle: 0,
            sync_count: 0,
            unlock_count: 0,
            last_error: None,
        }
    }

    pub const fn config(&self) -> DcCyclicConfig {
        self.config
    }

    pub const fn datagram_plan(&self) -> DatagramPlan {
        self.config.datagram_plan()
    }

    pub const fn pending_generation(&self) -> Option<u16> {
        match self.pending {
            Some(pending) => Some(pending.generation),
            None => None,
        }
    }

    pub const fn monitor(&self) -> &DcMonitor {
        &self.monitor
    }

    pub fn monitor_mut(&mut self) -> &mut DcMonitor {
        &mut self.monitor
    }

    pub const fn last_application_time_ns(&self) -> u64 {
        self.last_application_time_ns
    }

    pub const fn last_reference_time_ns(&self) -> u64 {
        self.last_reference_time_ns
    }

    pub const fn last_sync_cycle(&self) -> u64 {
        self.last_sync_cycle
    }

    pub const fn sync_count(&self) -> u64 {
        self.sync_count
    }

    pub const fn unlock_count(&self) -> u64 {
        self.unlock_count
    }

    pub const fn last_error(&self) -> Option<DcCyclicError> {
        self.last_error
    }

    /// Stage the next application's DC time in the process-image region owned
    /// by this FRMW datagram. The supplied value must be monotonic.
    pub fn prepare(
        &mut self,
        generation: u16,
        application_time_ns: u64,
        process_image: &mut [u8],
    ) -> Result<(), DcCyclicError> {
        if self.config.expected_wkc == 0 {
            return self.fail(DcCyclicError::InvalidConfiguration);
        }
        if self.pending.is_some() {
            return self.fail(DcCyclicError::Busy);
        }
        if self.has_last_application_time && application_time_ns < self.last_application_time_ns {
            return self.fail(DcCyclicError::ApplicationTimeRegressed);
        }
        let end = match self.config.payload_offset.checked_add(DC_SYSTEM_TIME_LEN) {
            Some(end) => end,
            None => return self.fail(DcCyclicError::ProcessImageOutOfBounds),
        };
        if end > process_image.len() {
            return self.fail(DcCyclicError::ProcessImageOutOfBounds);
        }
        process_image[self.config.payload_offset..end]
            .copy_from_slice(&application_time_ns.to_le_bytes());
        self.pending = Some(DcCyclicPending {
            generation,
            application_time_ns,
        });
        self.last_application_time_ns = application_time_ns;
        self.has_last_application_time = true;
        self.last_error = None;
        Ok(())
    }

    pub fn complete(
        &mut self,
        cycle: u64,
        received_at_ns: u64,
        completion: RxMatch,
        header: DatagramHeader,
        payload: &[u8],
    ) -> Result<(), DcCyclicError> {
        let pending = match self.pending {
            Some(pending) => pending,
            None => return self.fail(DcCyclicError::Busy),
        };
        let expected = self.config.datagram_plan();
        if header.index != expected.index
            || header.command != expected.command
            || header.address != expected.address
        {
            return self.fail(DcCyclicError::UnexpectedDatagram);
        }
        if completion.generation != pending.generation {
            return self.fail(DcCyclicError::GenerationMismatch);
        }
        if completion.working_counter != self.config.expected_wkc {
            return self.fail(DcCyclicError::WorkingCounterMismatch);
        }
        if header.length as usize != DC_SYSTEM_TIME_LEN {
            return self.fail(DcCyclicError::PayloadLengthMismatch);
        }
        let reference_time: [u8; DC_SYSTEM_TIME_LEN] = match payload.try_into() {
            Ok(bytes) => bytes,
            Err(_) => return self.fail(DcCyclicError::PayloadLengthMismatch),
        };

        let previous_state = self.monitor.state();
        let reference_time_ns = u64::from_le_bytes(reference_time);
        self.monitor.observe(
            received_at_ns,
            reference_time_ns,
            pending.application_time_ns,
        );
        if self.monitor.state() == DcLockState::Unlocked && previous_state != DcLockState::Unlocked
        {
            self.unlock_count = self.unlock_count.saturating_add(1);
        }
        self.last_reference_time_ns = reference_time_ns;
        self.last_sync_cycle = cycle;
        self.sync_count = self.sync_count.saturating_add(1);
        self.pending = None;
        self.last_error = None;
        Ok(())
    }

    /// Close an RX generation even if its FRMW response was lost or rejected
    /// by the master. A missing sample must not keep the next prepare busy or
    /// allow the previous cycle's lock to count as current evidence.
    pub fn finish_receive(&mut self, cycle: u64, generation: u16) -> Result<(), DcCyclicError> {
        if self.pending_generation() == Some(generation) {
            return self.fail(DcCyclicError::MissingResponse);
        }
        if self.pending.is_none() && self.last_sync_cycle == cycle && self.last_error.is_none() {
            return Ok(());
        }
        if let Some(error) = self.last_error {
            return Err(error);
        }
        self.fail(DcCyclicError::GenerationMismatch)
    }

    fn fail<T>(&mut self, error: DcCyclicError) -> Result<T, DcCyclicError> {
        self.pending = None;
        self.last_error = Some(error);
        Err(error)
    }
}

impl RxDatagramConsumer for DcCyclicSync {
    fn accept(
        &mut self,
        cycle: u64,
        received_at_ns: u64,
        completion: RxMatch,
        header: DatagramHeader,
        payload: &[u8],
    ) -> bool {
        if header.index != self.config.datagram_index {
            return false;
        }
        self.complete(cycle, received_at_ns, completion, header, payload)
            .is_ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DcLockState {
    Unknown,
    Locking,
    Locked,
    Degraded,
    Unlocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DcSample {
    pub timestamp_ns: u64,
    pub reference_time_ns: u64,
    pub local_time_ns: u64,
    pub offset_ns: i64,
    pub jitter_ns: u64,
    pub within_limits: bool,
}

pub struct DcMonitor {
    state: DcLockState,
    max_offset_ns: u64,
    max_jitter_ns: u64,
    lock_good_cycles: u16,
    unlock_bad_cycles: u16,
    good_cycles: u16,
    bad_cycles: u16,
    sample_count: u64,
    offset_ns: i64,
    jitter_ns: u64,
    last_sample_timestamp_ns: u64,
    previous_offset_ns: i64,
    has_previous: bool,
}

impl DcMonitor {
    pub const fn new(
        max_offset_ns: u64,
        max_jitter_ns: u64,
        lock_good_cycles: u16,
        unlock_bad_cycles: u16,
    ) -> Self {
        Self {
            state: DcLockState::Unknown,
            max_offset_ns,
            max_jitter_ns,
            lock_good_cycles: if lock_good_cycles == 0 {
                1
            } else {
                lock_good_cycles
            },
            unlock_bad_cycles: if unlock_bad_cycles == 0 {
                1
            } else {
                unlock_bad_cycles
            },
            good_cycles: 0,
            bad_cycles: 0,
            sample_count: 0,
            offset_ns: 0,
            jitter_ns: 0,
            last_sample_timestamp_ns: 0,
            previous_offset_ns: 0,
            has_previous: false,
        }
    }

    pub const fn state(&self) -> DcLockState {
        self.state
    }

    pub fn is_locked(&self) -> bool {
        matches!(self.state, DcLockState::Locked)
    }

    pub const fn offset_ns(&self) -> i64 {
        self.offset_ns
    }

    pub const fn jitter_ns(&self) -> u64 {
        self.jitter_ns
    }

    pub const fn good_cycles(&self) -> u16 {
        self.good_cycles
    }

    pub const fn bad_cycles(&self) -> u16 {
        self.bad_cycles
    }

    pub const fn sample_count(&self) -> u64 {
        self.sample_count
    }

    pub const fn last_sample_timestamp_ns(&self) -> u64 {
        self.last_sample_timestamp_ns
    }

    pub fn observe(
        &mut self,
        timestamp_ns: u64,
        reference_time_ns: u64,
        local_time_ns: u64,
    ) -> DcSample {
        let offset_ns = saturating_i128_to_i64(local_time_ns as i128 - reference_time_ns as i128);
        let jitter_ns = if self.has_previous {
            absolute_difference(offset_ns, self.previous_offset_ns)
        } else {
            0
        };
        let within_limits =
            absolute_i64(offset_ns) <= self.max_offset_ns && jitter_ns <= self.max_jitter_ns;

        self.offset_ns = offset_ns;
        self.jitter_ns = jitter_ns;
        self.last_sample_timestamp_ns = timestamp_ns;
        self.sample_count = self.sample_count.saturating_add(1);
        self.previous_offset_ns = offset_ns;
        self.has_previous = true;
        if within_limits {
            self.good_cycles = self.good_cycles.saturating_add(1);
            self.bad_cycles = 0;
            if self.good_cycles >= self.lock_good_cycles {
                self.state = DcLockState::Locked;
            } else {
                self.state = DcLockState::Locking;
            }
        } else {
            self.bad_cycles = self.bad_cycles.saturating_add(1);
            self.good_cycles = 0;
            self.state = if self.bad_cycles >= self.unlock_bad_cycles {
                DcLockState::Unlocked
            } else {
                DcLockState::Degraded
            };
        }

        DcSample {
            timestamp_ns,
            reference_time_ns,
            local_time_ns,
            offset_ns,
            jitter_ns,
            within_limits,
        }
    }

    pub fn reset(&mut self) {
        self.state = DcLockState::Unknown;
        self.good_cycles = 0;
        self.bad_cycles = 0;
        self.sample_count = 0;
        self.offset_ns = 0;
        self.jitter_ns = 0;
        self.last_sample_timestamp_ns = 0;
        self.previous_offset_ns = 0;
        self.has_previous = false;
    }
}

impl Default for DcMonitor {
    fn default() -> Self {
        Self::new(1_000, 500, 3, 3)
    }
}

fn absolute_i64(value: i64) -> u64 {
    if value == i64::MIN {
        i64::MAX as u64 + 1
    } else if value < 0 {
        (-value) as u64
    } else {
        value as u64
    }
}

fn absolute_difference(left: i64, right: i64) -> u64 {
    let difference = left as i128 - right as i128;
    if difference < 0 {
        (-difference) as u64
    } else {
        difference as u64
    }
}

fn saturating_i128_to_i64(value: i128) -> i64 {
    if value > i64::MAX as i128 {
        i64::MAX
    } else if value < i64::MIN as i128 {
        i64::MIN
    } else {
        value as i64
    }
}

fn u32_payload(value: u32) -> [u8; DC_MAX_ACTION_PAYLOAD] {
    let mut payload = [0; DC_MAX_ACTION_PAYLOAD];
    payload[..DC_CYCLE_LEN].copy_from_slice(&value.to_le_bytes());
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{EscDcRange, ScanDcCapabilities};
    use crate::slave::{AlStatus, EthercatState};

    fn topology_links(open_ports: &[usize]) -> [ScanPortLink; ESC_PORT_COUNT] {
        let mut links = [ScanPortLink {
            link_up: false,
            loop_closed: true,
            signal_detected: false,
        }; ESC_PORT_COUNT];
        for port in open_ports.iter().copied() {
            links[port].loop_closed = false;
        }
        links
    }

    fn topology_record(
        position: u16,
        open_ports: &[usize],
        dc_supported: bool,
        system_time_capable: bool,
        receive_times: [u32; ESC_PORT_COUNT],
    ) -> ScanRecord {
        ScanRecord {
            position,
            station_address: 0x1000 + position,
            esc_type: 0,
            revision: 0,
            build: 0,
            fmmu_count: 0,
            sync_manager_count: 0,
            ram_size: 0,
            port_descriptor: 0,
            dl_status: 0,
            port_links: topology_links(open_ports),
            dc: ScanDcCapabilities {
                raw_features: if dc_supported {
                    crate::ESC_FEATURE_DC_SUPPORTED
                } else {
                    0
                },
                fmmu_bit_operation: false,
                supported: dc_supported,
                range: EscDcRange::Bits32,
                has_system_time: system_time_capable,
                system_time: system_time_capable.then_some(1),
                receive_times: dc_supported.then_some(receive_times),
            },
            device_emulation: false,
            al_status: AlStatus::new(EthercatState::Init as u16, 0),
            online: true,
        }
    }

    fn clock_payload(system_time_ns: u64, old_offset: u64) -> [u8; DC_CLOCK_SAMPLE_LEN] {
        let mut payload = [0; DC_CLOCK_SAMPLE_LEN];
        payload[..8].copy_from_slice(&system_time_ns.to_le_bytes());
        payload[16..24].copy_from_slice(&old_offset.to_le_bytes());
        payload
    }

    #[test]
    fn dc_topology_builds_branched_physical_tree_in_port_order() {
        let records = [
            topology_record(10, &[3, 1], false, false, [0; 4]),
            topology_record(11, &[3], false, false, [0; 4]),
            topology_record(12, &[], false, false, [0; 4]),
            topology_record(13, &[], false, false, [0; 4]),
        ];
        let topology = DcTopology::<4>::build(&records, None).unwrap();

        assert_eq!(topology.len(), 4);
        assert_eq!(
            topology.slave(10).unwrap().ports[3].next_slave_position,
            Some(11)
        );
        assert_eq!(
            topology.slave(11).unwrap().ports[0].next_slave_position,
            Some(10)
        );
        assert_eq!(
            topology.slave(11).unwrap().ports[3].next_slave_position,
            Some(12)
        );
        assert_eq!(
            topology.slave(12).unwrap().ports[0].next_slave_position,
            Some(11)
        );
        assert_eq!(
            topology.slave(10).unwrap().ports[1].next_slave_position,
            Some(13)
        );
        assert_eq!(
            topology.slave(13).unwrap().ports[0].next_slave_position,
            Some(10)
        );
    }

    #[test]
    fn dc_topology_uses_delay_only_source_and_non_first_reference() {
        let records = [
            topology_record(0, &[3], true, false, [0, 0, 0, 200]),
            topology_record(1, &[], true, true, [0; 4]),
        ];
        let topology = DcTopology::<2>::build(&records, Some(1)).unwrap();

        assert_eq!(topology.reference_position(), Some(1));
        assert_eq!(topology.transmission_delay_ns(1), Some(0));
        assert_eq!(topology.transmission_delay_ns(0), Some(100));
        assert_eq!(
            topology.slave(0).unwrap().ports[3].next_dc_position,
            Some(1)
        );
        assert_eq!(
            topology.slave(0).unwrap().ports[3].propagation_delay_ns,
            Some(100)
        );
        assert_eq!(
            topology.slave(1).unwrap().ports[0].next_dc_position,
            Some(0)
        );
    }

    #[test]
    fn dc_topology_receive_time_delta_wraps_at_u32_boundary() {
        let records = [
            topology_record(0, &[3], true, true, [u32::MAX - 15, 0, 0, 16]),
            topology_record(1, &[], true, false, [0; 4]),
        ];
        let topology = DcTopology::<2>::build(&records, Some(0)).unwrap();

        assert_eq!(
            topology.slave(0).unwrap().ports[3].propagation_delay_ns,
            Some(16)
        );
        assert_eq!(topology.transmission_delay_ns(1), Some(16));
    }

    #[test]
    fn dc_topology_leaves_disconnected_dc_branch_unmeasurable() {
        let records = [
            topology_record(0, &[3, 1], false, false, [0; 4]),
            topology_record(1, &[], true, true, [0; 4]),
            topology_record(2, &[], true, true, [0; 4]),
        ];
        let topology = DcTopology::<3>::build(&records, Some(1)).unwrap();

        assert_eq!(topology.transmission_delay_ns(1), Some(0));
        assert_eq!(topology.transmission_delay_ns(2), None);
        assert_eq!(topology.slave(1).unwrap().ports[0].next_dc_position, None);
        assert_eq!(topology.slave(2).unwrap().ports[0].next_dc_position, None);
    }

    #[test]
    fn dc_topology_rejects_invalid_structure_and_missing_times() {
        let open_root = [topology_record(0, &[3], false, false, [0; 4])];
        assert_eq!(
            DcTopology::<1>::build(&open_root, None),
            Err(DcTopologyError::TopologyOverrun {
                position: 0,
                port: 3,
            })
        );

        let unreachable = [
            topology_record(0, &[], false, false, [0; 4]),
            topology_record(1, &[], false, false, [0; 4]),
        ];
        assert_eq!(
            DcTopology::<2>::build(&unreachable, None),
            Err(DcTopologyError::UnreachablePosition(1))
        );
        assert_eq!(
            DcTopology::<1>::build(&unreachable, None),
            Err(DcTopologyError::CapacityExceeded)
        );

        let duplicate = [
            topology_record(0, &[3], false, false, [0; 4]),
            topology_record(0, &[], false, false, [0; 4]),
        ];
        assert_eq!(
            DcTopology::<2>::build(&duplicate, None),
            Err(DcTopologyError::DuplicatePosition(0))
        );

        let mut missing = topology_record(0, &[], true, true, [0; 4]);
        missing.dc.receive_times = None;
        assert_eq!(
            DcTopology::<1>::build(&[missing], Some(0)),
            Err(DcTopologyError::MissingReceiveTimes(0))
        );
    }

    #[test]
    fn dc_topology_rejects_delay_underflow_and_overflow() {
        let underflow = [
            topology_record(0, &[3], true, true, [0, 0, 0, 10]),
            topology_record(1, &[3], true, false, [0, 0, 0, 20]),
            topology_record(2, &[], false, false, [0; 4]),
        ];
        assert_eq!(
            DcTopology::<3>::build(&underflow, Some(0)),
            Err(DcTopologyError::DelayUnderflow {
                position: 0,
                port: 3,
                round_trip_ns: 10,
                downstream_round_trip_ns: 20,
            })
        );

        let aggregate_overflow = [
            topology_record(0, &[3], true, true, [0, 0, 0, 100]),
            topology_record(
                1,
                &[3, 1],
                true,
                false,
                [0, u32::MAX - 31, 0, u32::MAX - 15],
            ),
            topology_record(2, &[], false, false, [0; 4]),
            topology_record(3, &[], false, false, [0; 4]),
        ];
        assert_eq!(
            DcTopology::<4>::build(&aggregate_overflow, Some(0)),
            Err(DcTopologyError::DelayOverflow(1))
        );

        let mut cumulative = DcTopology::<3>::empty();
        cumulative.len = 3;
        cumulative.reference_position = Some(0);
        for (index, slave) in cumulative.slaves.iter_mut().enumerate() {
            slave.position = index as u16;
            slave.dc_supported = true;
        }
        cumulative.slaves[0].system_time_capable = true;
        cumulative.slaves[0].ports[3].next_dc_position = Some(1);
        cumulative.slaves[0].ports[3].propagation_delay_ns = Some(u32::MAX);
        cumulative.slaves[1].ports[0].next_dc_position = Some(0);
        cumulative.slaves[1].ports[0].propagation_delay_ns = Some(u32::MAX);
        cumulative.slaves[1].ports[3].next_dc_position = Some(2);
        cumulative.slaves[1].ports[3].propagation_delay_ns = Some(1);
        cumulative.slaves[2].ports[0].next_dc_position = Some(1);
        cumulative.slaves[2].ports[0].propagation_delay_ns = Some(1);
        assert_eq!(
            cumulative.calculate_transmission_delays(),
            Err(DcTopologyError::DelayOverflow(2))
        );
    }

    #[test]
    fn dc_topology_rejects_reference_without_system_time() {
        let records = [topology_record(4, &[], true, false, [0; 4])];
        assert_eq!(
            DcTopology::<1>::build(&records, Some(4)),
            Err(DcTopologyError::InvalidReference(4))
        );
    }

    #[test]
    fn dc_clock_programs_all_system_time_slaves_in_topology_order() {
        let mut reference = topology_record(0, &[3], true, true, [0, 0, 0, 200]);
        reference.dc.range = EscDcRange::Bits64;
        let secondary = topology_record(1, &[], true, true, [0; 4]);
        let topology = DcTopology::<2>::build(&[reference, secondary], Some(0)).unwrap();
        let mut controller = DcClockController::new();
        controller
            .start(DcClockConfig::new(), &topology, 7, 1_000, 0)
            .unwrap();

        assert_eq!(controller.planned_count(), 2);
        assert!(controller.programmed_slaves().is_empty());
        let read_reference = controller.next_action(1).unwrap().unwrap();
        assert_eq!(read_reference.kind, DcClockActionKind::ReadClock);
        assert_eq!(read_reference.position, 0);
        assert_eq!(read_reference.station_address, 0x1000);
        assert_eq!(
            read_reference.address,
            fixed_address(0x1000, ESC_DC_SYSTEM_TIME)
        );
        assert_eq!(read_reference.response_len(), DC_CLOCK_SAMPLE_LEN);
        assert_eq!(
            controller.accept(
                read_reference,
                read_reference.generation,
                &clock_payload(900, 5),
                1,
                10,
            ),
            Ok(DcClockProgress::Advanced)
        );

        let write_reference = controller.next_action(11).unwrap().unwrap();
        assert_eq!(write_reference.kind, DcClockActionKind::WriteOffsetDelay);
        assert_eq!(
            write_reference.address,
            fixed_address(0x1000, ESC_DC_SYSTEM_OFFSET)
        );
        assert_eq!(write_reference.payload().len(), DC_CLOCK_WRITE_LEN);
        assert_eq!(
            u64::from_le_bytes(write_reference.payload()[..8].try_into().unwrap()),
            115
        );
        assert_eq!(
            u32::from_le_bytes(write_reference.payload()[8..12].try_into().unwrap()),
            0
        );
        assert_eq!(
            controller.accept(write_reference, write_reference.generation, &[], 1, 12,),
            Ok(DcClockProgress::Advanced)
        );
        assert_eq!(controller.completed_count(), 1);
        assert!(controller.programmed_slaves().is_empty());

        let read_secondary = controller.next_action(13).unwrap().unwrap();
        assert_eq!(read_secondary.position, 1);
        assert_eq!(
            controller.accept(
                read_secondary,
                read_secondary.generation,
                &clock_payload(1_000, 20),
                1,
                14,
            ),
            Ok(DcClockProgress::Advanced)
        );
        let write_secondary = controller.next_action(15).unwrap().unwrap();
        assert_eq!(
            u64::from_le_bytes(write_secondary.payload()[..8].try_into().unwrap()),
            34
        );
        assert_eq!(
            u32::from_le_bytes(write_secondary.payload()[8..12].try_into().unwrap()),
            100
        );
        assert_eq!(
            controller.accept(write_secondary, write_secondary.generation, &[], 1, 16,),
            Ok(DcClockProgress::Complete)
        );

        assert_eq!(controller.phase(), DcClockPhase::Complete);
        assert_eq!(controller.completed_count(), 2);
        assert_eq!(controller.programmed_slaves().len(), 2);
        assert_eq!(
            controller.programmed_slave(0),
            Some(&DcClockProgrammedSlave {
                position: 0,
                station_address: 0x1000,
                range: EscDcRange::Bits64,
                sampled_system_time_ns: 900,
                target_application_time_ns: 1_010,
                old_offset: 5,
                new_offset: 115,
                transmission_delay_ns: 0,
            })
        );
    }

    #[test]
    fn dc_clock_applies_32_bit_wrap_and_64_bit_negative_correction() {
        let mut bits32 = topology_record(0, &[], true, true, [0; 4]);
        bits32.dc.range = EscDcRange::Bits32;
        let topology32 = DcTopology::<1>::build(&[bits32], Some(0)).unwrap();
        let mut controller32 = DcClockController::new();
        controller32
            .start(DcClockConfig::new(), &topology32, 1, 2, 100)
            .unwrap();
        let read32 = controller32.next_action(100).unwrap().unwrap();
        controller32
            .accept(
                read32,
                read32.generation,
                &clock_payload(u32::MAX as u64 - 1, 10),
                1,
                100,
            )
            .unwrap();
        let write32 = controller32.next_action(101).unwrap().unwrap();
        assert_eq!(
            u64::from_le_bytes(write32.payload()[..8].try_into().unwrap()),
            14
        );

        let mut bits64 = topology_record(0, &[], true, true, [0; 4]);
        bits64.dc.range = EscDcRange::Bits64;
        let topology64 = DcTopology::<1>::build(&[bits64], Some(0)).unwrap();
        let mut controller64 = DcClockController::new();
        controller64
            .start(DcClockConfig::new(), &topology64, 2, 100, 0)
            .unwrap();
        let read64 = controller64.next_action(0).unwrap().unwrap();
        controller64
            .accept(read64, read64.generation, &clock_payload(150, 1_000), 1, 0)
            .unwrap();
        let write64 = controller64.next_action(1).unwrap().unwrap();
        assert_eq!(
            u64::from_le_bytes(write64.payload()[..8].try_into().unwrap()),
            950
        );
    }

    #[test]
    fn dc_clock_rejects_incomplete_topology_before_emitting_actions() {
        let no_reference = [topology_record(0, &[], true, true, [0; 4])];
        let topology = DcTopology::<1>::build(&no_reference, None).unwrap();
        let mut controller = DcClockController::new();
        assert_eq!(
            controller.start(DcClockConfig::new(), &topology, 1, 0, 0),
            Err(DcClockError::MissingReference)
        );
        assert_eq!(controller.phase(), DcClockPhase::Faulted);
        assert_eq!(controller.pending(), None);
        assert!(controller.programmed_slaves().is_empty());

        let disconnected = [
            topology_record(0, &[3, 1], false, false, [0; 4]),
            topology_record(1, &[], true, true, [0; 4]),
            topology_record(2, &[], true, true, [0; 4]),
        ];
        let topology = DcTopology::<3>::build(&disconnected, Some(1)).unwrap();
        let mut controller = DcClockController::<3>::new();
        assert_eq!(
            controller.start(DcClockConfig::new(), &topology, 2, 0, 0),
            Err(DcClockError::MissingTransmissionDelay(2))
        );
        assert_eq!(controller.planned_count(), 0);

        let empty = DcTopology::<0>::empty();
        let mut empty_controller = DcClockController::new();
        empty_controller
            .start(DcClockConfig::new(), &empty, 3, 0, 0)
            .unwrap();
        assert_eq!(empty_controller.phase(), DcClockPhase::Complete);
        assert_eq!(empty_controller.next_action(0), Ok(None));
    }

    #[test]
    fn dc_clock_fault_hides_partial_batch_and_restart_clears_progress() {
        let records = [
            topology_record(0, &[3], true, true, [0, 0, 0, 200]),
            topology_record(1, &[], true, true, [0; 4]),
        ];
        let topology = DcTopology::<2>::build(&records, Some(0)).unwrap();
        let mut controller = DcClockController::new();
        controller
            .start(DcClockConfig::new(), &topology, 1, 100, 0)
            .unwrap();
        let read = controller.next_action(0).unwrap().unwrap();
        controller
            .accept(read, read.generation, &clock_payload(100, 0), 1, 0)
            .unwrap();
        let write = controller.next_action(1).unwrap().unwrap();
        controller
            .accept(write, write.generation, &[], 1, 1)
            .unwrap();
        assert_eq!(controller.completed_count(), 1);

        let second = controller.next_action(2).unwrap().unwrap();
        assert_eq!(
            controller.accept(second, second.generation, &clock_payload(100, 0), 0, 2),
            Err(DcClockError::UnexpectedWorkingCounter)
        );
        assert_eq!(controller.completed_count(), 1);
        assert!(controller.programmed_slaves().is_empty());
        assert_eq!(
            controller.last_error(),
            Some(DcClockError::UnexpectedWorkingCounter)
        );

        controller
            .start(DcClockConfig::new(), &topology, 2, 200, 10)
            .unwrap();
        assert_eq!(controller.completed_count(), 0);
        assert!(controller.programmed_slaves().is_empty());
        assert_eq!(controller.phase(), DcClockPhase::ReadingClock);
    }

    #[test]
    fn dc_clock_latches_time_and_offset_arithmetic_failures() {
        let mut record = topology_record(0, &[], true, true, [0; 4]);
        record.dc.range = EscDcRange::Bits64;
        let topology = DcTopology::<1>::build(&[record], Some(0)).unwrap();

        let mut regressed = DcClockController::new();
        regressed
            .start(DcClockConfig::new(), &topology, 1, 0, 10)
            .unwrap();
        let action = regressed.next_action(10).unwrap().unwrap();
        assert_eq!(
            regressed.accept(action, action.generation, &clock_payload(0, 0), 1, 9),
            Err(DcClockError::MonotonicTimeRegressed)
        );
        assert_eq!(regressed.phase(), DcClockPhase::Faulted);

        let mut overflow = DcClockController::new();
        overflow
            .start(DcClockConfig::new(), &topology, 2, u64::MAX, 0)
            .unwrap();
        let action = overflow.next_action(0).unwrap().unwrap();
        assert_eq!(
            overflow.accept(action, action.generation, &clock_payload(0, 0), 1, 1),
            Err(DcClockError::ApplicationTimeOverflow)
        );

        let mut out_of_range = DcClockController::new();
        out_of_range
            .start(DcClockConfig::new(), &topology, 3, u64::MAX, 0)
            .unwrap();
        let action = out_of_range.next_action(0).unwrap().unwrap();
        assert_eq!(
            out_of_range.accept(action, action.generation, &clock_payload(0, 0), 1, 0),
            Err(DcClockError::OffsetDeltaOutOfRange)
        );
    }

    #[test]
    fn dc_clock_rejects_action_generation_shape_and_deadline_mismatches() {
        let record = topology_record(0, &[], true, true, [0; 4]);
        let topology = DcTopology::<1>::build(&[record], Some(0)).unwrap();

        let mut wrong_action = DcClockController::new();
        wrong_action
            .start(DcClockConfig::new(), &topology, 1, 0, 0)
            .unwrap();
        let action = wrong_action.next_action(0).unwrap().unwrap();
        let mut substituted = action;
        substituted.token = substituted.token.wrapping_add(1);
        assert_eq!(
            wrong_action.accept(substituted, action.generation, &clock_payload(0, 0), 1, 0),
            Err(DcClockError::ActionMismatch)
        );

        let mut wrong_generation = DcClockController::new();
        wrong_generation
            .start(DcClockConfig::new(), &topology, 2, 0, 0)
            .unwrap();
        let action = wrong_generation.next_action(0).unwrap().unwrap();
        assert_eq!(
            wrong_generation.accept(
                action,
                action.generation.wrapping_add(1),
                &clock_payload(0, 0),
                1,
                0,
            ),
            Err(DcClockError::GenerationMismatch)
        );

        let mut wrong_length = DcClockController::new();
        wrong_length
            .start(DcClockConfig::new(), &topology, 3, 0, 0)
            .unwrap();
        let action = wrong_length.next_action(0).unwrap().unwrap();
        assert_eq!(
            wrong_length.accept(action, action.generation, &[0; 23], 1, 0),
            Err(DcClockError::PayloadLengthMismatch)
        );

        let mut timed_out = DcClockController::new();
        timed_out
            .start(
                DcClockConfig {
                    timeout_ns: 100,
                    request_timeout_ns: 5,
                },
                &topology,
                4,
                0,
                0,
            )
            .unwrap();
        let action = timed_out.next_action(0).unwrap().unwrap();
        assert_eq!(
            timed_out.accept(action, action.generation, &clock_payload(0, 0), 1, 6),
            Err(DcClockError::Timeout)
        );

        let mut deadline_overflow = DcClockController::new();
        deadline_overflow
            .start(
                DcClockConfig {
                    timeout_ns: 5,
                    request_timeout_ns: 20,
                },
                &topology,
                5,
                0,
                u64::MAX - 10,
            )
            .unwrap();
        assert_eq!(
            deadline_overflow.next_action(u64::MAX - 9),
            Err(DcClockError::DeadlineOverflow)
        );
        assert_eq!(deadline_overflow.phase(), DcClockPhase::Faulted);
    }

    #[test]
    fn controller_follows_soem_dc_sync0_sequence() {
        let mut controller = DcController::new();
        controller
            .start(DcConfig::sync0(1_000_000, 0), 0x1001, 7, 0)
            .unwrap();

        let disable = controller.next_action(1).unwrap().unwrap();
        assert_eq!(disable.kind, DcActionKind::DisableSync);
        assert_eq!(
            disable.address,
            fixed_address(0x1001, ESC_DC_SYNC_ACTIVATION)
        );
        assert_eq!(disable.payload(), &[0]);
        controller.accept(disable, 7, &[], 1, 2).unwrap();

        let access = controller.next_action(3).unwrap().unwrap();
        assert_eq!(access.kind, DcActionKind::GrantEthercatAccess);
        assert_eq!(access.address, fixed_address(0x1001, ESC_DC_CUC));
        controller.accept(access, 7, &[], 1, 4).unwrap();

        let read = controller.next_action(5).unwrap().unwrap();
        assert_eq!(read.kind, DcActionKind::ReadSystemTime);
        assert_eq!(read.response_len(), 8);
        let system_time = 1_234_567_890u64;
        controller
            .accept(read, 7, &system_time.to_le_bytes(), 1, 6)
            .unwrap();
        assert_eq!(controller.system_time_ns(), system_time);
        assert_eq!(controller.first_trigger_ns(), 1_335_000_000);

        let start = controller.next_action(7).unwrap().unwrap();
        assert_eq!(start.kind, DcActionKind::WriteStartTime);
        assert_eq!(start.payload(), &1_335_000_000u64.to_le_bytes());
        controller.accept(start, 7, &[], 1, 8).unwrap();

        let cycle = controller.next_action(9).unwrap().unwrap();
        assert_eq!(cycle.kind, DcActionKind::WriteCycle0);
        assert_eq!(cycle.payload(), &1_000_000u32.to_le_bytes());
        controller.accept(cycle, 7, &[], 1, 10).unwrap();

        let activate = controller.next_action(11).unwrap().unwrap();
        assert_eq!(activate.kind, DcActionKind::ActivateSync);
        assert_eq!(activate.payload(), &[0x03]);
        assert_eq!(
            controller.accept(activate, 7, &[], 1, 12),
            Ok(DcProgress::Complete)
        );
        assert_eq!(controller.phase(), DcPhase::Complete);
    }

    #[test]
    fn controller_programs_sync1_and_uses_combined_cycle_for_start_time() {
        let mut controller = DcController::new();
        controller
            .start(DcConfig::sync0_sync1(1_000_000, 2_000_000, 0), 1, 2, 0)
            .unwrap();

        for timestamp in [1u64, 3] {
            let action = controller.next_action(timestamp).unwrap().unwrap();
            controller.accept(action, 2, &[], 1, timestamp + 1).unwrap();
        }
        let read = controller.next_action(5).unwrap().unwrap();
        assert_eq!(read.kind, DcActionKind::ReadSystemTime);
        controller
            .accept(read, 2, &3_100_000u64.to_le_bytes(), 1, 7)
            .unwrap();
        assert_eq!(controller.first_trigger_ns(), 105_000_000);

        let start = controller.next_action(8).unwrap().unwrap();
        controller.accept(start, 2, &[], 1, 9).unwrap();
        let cycle0 = controller.next_action(10).unwrap().unwrap();
        controller.accept(cycle0, 2, &[], 1, 11).unwrap();
        let cycle1 = controller.next_action(12).unwrap().unwrap();
        assert_eq!(cycle1.kind, DcActionKind::WriteCycle1);
        assert_eq!(cycle1.payload(), &2_000_000u32.to_le_bytes());
        controller.accept(cycle1, 2, &[], 1, 13).unwrap();
        let activate = controller.next_action(14).unwrap().unwrap();
        assert_eq!(activate.payload(), &[0x07]);
    }

    #[test]
    fn controller_can_be_driven_from_the_control_request_pool() {
        let mut controller = DcController::new();
        controller
            .start(DcConfig::sync0(1_000_000, 0), 0x1000, 9, 0)
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        let request = pool.get_mut(handle).unwrap();
        request.state = RequestState::InFlight;
        pool.complete(
            handle,
            action.generation,
            action.address,
            action.payload(),
            1,
        )
        .unwrap();
        assert_eq!(
            controller.accept_completed(&mut pool, handle, 2),
            Ok(DcProgress::Advanced)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn expired_control_request_reaches_dc_controller_as_timeout() {
        let mut controller = DcController::new();
        controller
            .start(DcConfig::sync0(1_000_000, 0), 0x1000, 9, 0)
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();
        let handle = controller.enqueue_pending(&mut pool).unwrap();
        let mut frame = [0; crate::wire::MAX_ETHERNET_FRAME_LEN];
        pool.build_into_buffer(handle, &mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
        let expired_at = action.deadline_ns.saturating_add(1);
        assert!(pool.expire_in_flight(expired_at).contains(handle));

        assert_eq!(
            controller.accept_completed(&mut pool, handle, expired_at),
            Err(DcError::Timeout)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(controller.phase(), DcPhase::Faulted);
    }

    #[test]
    fn monitor_requires_good_cycles_and_latches_unlocked_hysteresis() {
        let mut monitor = DcMonitor::new(50, 10, 2, 2);
        monitor.observe(1, 1_000, 1_020);
        assert_eq!(monitor.state(), DcLockState::Locking);
        assert!(monitor.observe(2, 2_000, 2_020).within_limits);
        assert!(monitor.is_locked());
        assert_eq!(monitor.offset_ns(), 20);
        assert_eq!(monitor.jitter_ns(), 0);

        monitor.observe(3, 3_000, 3_200);
        assert_eq!(monitor.state(), DcLockState::Degraded);
        monitor.observe(4, 4_000, 4_200);
        assert_eq!(monitor.state(), DcLockState::Unlocked);
        assert_eq!(monitor.bad_cycles(), 2);

        monitor.observe(5, 5_000, 5_020);
        assert_eq!(monitor.state(), DcLockState::Unlocked);
        monitor.observe(6, 6_000, 6_020);
        assert_eq!(monitor.state(), DcLockState::Locking);
        monitor.observe(7, 7_000, 7_020);
        assert!(monitor.is_locked());
    }

    #[test]
    fn invalid_cycle_configuration_fails_closed() {
        let mut controller = DcController::new();
        let mut config = DcConfig::sync0(0, 0);
        config.timeout_ns = 1;
        config.request_timeout_ns = 1;
        assert_eq!(
            controller.start(config, 1, 1, 0),
            Err(DcError::InvalidConfiguration)
        );
    }

    #[test]
    fn cyclic_sync_updates_application_time_and_observes_reference_clock() {
        let config = DcCyclicConfig::new(0x1000, 13, 3);
        let monitor = DcMonitor::new(50, 10, 1, 2);
        let mut sync = DcCyclicSync::new(config, monitor);
        let mut process_image = [0u8; 11];

        sync.prepare(7, 1_020, &mut process_image).unwrap();
        assert_eq!(&process_image[3..11], &1_020u64.to_le_bytes());
        assert_eq!(sync.pending_generation(), Some(7));

        let mut plan = crate::plan::FramePlan::<2>::new();
        plan.push(DatagramPlan {
            command: Command::Lrw,
            index: 12,
            address: 0,
            payload_offset: 0,
            payload_len: 3,
            expected_wkc: 1,
        })
        .unwrap();
        plan.push(sync.datagram_plan()).unwrap();
        let mut frame = [0u8; crate::wire::MAX_ETHERNET_FRAME_LEN];
        let length = plan
            .build(&mut frame, [0xFF; 6], [1, 2, 3, 4, 5, 6], &process_image)
            .unwrap();
        let view = crate::wire::FrameView::parse(&frame[..length]).unwrap();
        let dc_datagram = view.datagrams().nth(1).unwrap().unwrap();
        assert_eq!(dc_datagram.header.command, Command::Frmw);
        assert_eq!(dc_datagram.header.index, 13);
        assert_eq!(
            dc_datagram.header.address,
            fixed_address(0x1000, ESC_DC_SYSTEM_TIME)
        );
        assert_eq!(dc_datagram.payload, &1_020u64.to_le_bytes());

        sync.complete(
            42,
            100,
            RxMatch {
                slot_id: 1,
                generation: 7,
                working_counter: 1,
            },
            DatagramHeader {
                command: Command::Frmw,
                index: 13,
                address: fixed_address(0x1000, ESC_DC_SYSTEM_TIME),
                length: 8,
                last: true,
            },
            &1_000u64.to_le_bytes(),
        )
        .unwrap();

        assert_eq!(sync.last_application_time_ns(), 1_020);
        assert_eq!(sync.last_reference_time_ns(), 1_000);
        assert_eq!(sync.last_sync_cycle(), 42);
        assert_eq!(sync.sync_count(), 1);
        assert_eq!(sync.monitor().state(), DcLockState::Locked);
        assert_eq!(sync.monitor().offset_ns(), 20);
    }

    #[test]
    fn consumer_mux_routes_domain_and_dc_by_datagram_index() {
        let mut domain = crate::domain::Domain::<4, 1>::new(0);
        domain
            .add_segment(crate::domain::DomainSegment {
                datagram_index: 12,
                input_offset: 0,
                len: 2,
                expected_wkc: 1,
            })
            .unwrap();
        domain.begin_receive(7).unwrap();

        let config = DcCyclicConfig::new(0x1000, 13, 2);
        let monitor = DcMonitor::new(50, 10, 1, 2);
        let mut sync = DcCyclicSync::new(config, monitor);
        let mut process_image = [0u8; 10];
        sync.prepare(7, 1_020, &mut process_image).unwrap();

        let mut mux = crate::engine::RxConsumerMux::new(domain, sync);
        assert!(mux.accept(
            42,
            100,
            RxMatch {
                slot_id: 0,
                generation: 7,
                working_counter: 1,
            },
            DatagramHeader {
                command: Command::Lrw,
                index: 12,
                address: 0,
                length: 2,
                last: false,
            },
            &[0xAA, 0x55],
        ));
        assert!(mux.accept(
            42,
            100,
            RxMatch {
                slot_id: 1,
                generation: 7,
                working_counter: 1,
            },
            DatagramHeader {
                command: Command::Frmw,
                index: 13,
                address: fixed_address(0x1000, ESC_DC_SYSTEM_TIME),
                length: 8,
                last: true,
            },
            &1_000u64.to_le_bytes(),
        ));

        assert!(mux.first_mut().finish_receive(7, 42).unwrap());
        assert_eq!(mux.first().input(), &[0xAA, 0x55, 0, 0]);
        assert!(mux.second().monitor().is_locked());
    }

    #[test]
    fn cyclic_sync_rejects_regressed_application_time() {
        let mut sync = DcCyclicSync::new(
            DcCyclicConfig::new(0x1000, 13, 0),
            DcMonitor::new(50, 10, 1, 2),
        );
        let mut process_image = [0u8; 8];
        sync.prepare(1, 100, &mut process_image).unwrap();
        sync.complete(
            1,
            10,
            RxMatch {
                slot_id: 1,
                generation: 1,
                working_counter: 1,
            },
            DatagramHeader {
                command: Command::Frmw,
                index: 13,
                address: fixed_address(0x1000, ESC_DC_SYSTEM_TIME),
                length: 8,
                last: true,
            },
            &100u64.to_le_bytes(),
        )
        .unwrap();

        assert_eq!(
            sync.prepare(2, 99, &mut process_image),
            Err(DcCyclicError::ApplicationTimeRegressed)
        );
        assert_eq!(
            sync.last_error(),
            Some(DcCyclicError::ApplicationTimeRegressed)
        );
    }
}
