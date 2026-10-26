#![no_std]
//! Independent bounded virtual EtherCAT bus.
//!
//! Only Ethernet bytes, monotonically advanced virtual time and explicit faults
//! enter the model. No master expectation, controller or completion API is used.
mod esc;
pub mod fixtures;

use esc::Esc;
pub use esc::{MAX_OBJECT_BYTES, SlaveConfig, SlaveKind, SlaveSnapshot};
use esop_ethercat_core::wire::MAX_ETHERNET_FRAME_LEN;
use esop_ethercat_core::{EthercatPort, LinkState, RxPoll};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimError {
    InvalidConfiguration,
    InvalidFrame,
    TimeRegression,
    TimeOverflow,
    QueueFull,
    LinkDown,
    SlaveOutOfBounds,
    ObjectCapacity,
    MailboxFull,
}

/// One-shot response faults apply to the next accepted frame. Persistent
/// faults remain until explicitly replaced with their healthy value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    DropNextResponse,
    DelayNextResponse(u64),
    DuplicateNextResponse,
    CorruptNextResponse { offset: usize, xor: u8 },
    WorkingCounterNextResponse { datagram: usize, value: u16 },
    Link(bool),
    SlaveLink { slave: usize, up: bool },
    Al { slave: usize, code: u16 },
    Drive { slave: usize, code: u16 },
    DcOffset { slave: usize, nanoseconds: i64 },
    RepeatAck { slave: usize, enabled: bool },
    MailboxCounter { slave: usize, next: u8 },
}

#[derive(Clone, Copy)]
struct QueuedFrame {
    bytes: [u8; MAX_ETHERNET_FRAME_LEN],
    len: usize,
    ready_ns: u64,
    order: u64,
}
impl QueuedFrame {
    const EMPTY: Self = Self {
        bytes: [0; MAX_ETHERNET_FRAME_LEN],
        len: 0,
        ready_ns: 0,
        order: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BusStatistics {
    pub transmitted: u64,
    pub received: u64,
    pub dropped: u64,
    pub malformed: u64,
    pub queue_pressure: u64,
}

/// Capacity is frozen at construction. Creating large topologies on small
/// thread stacks should use caller-owned/static storage.
pub struct VirtualBus<const SLAVES: usize, const RX: usize = 8> {
    slaves: [Esc; SLAVES],
    queue: [QueuedFrame; RX],
    now_ns: u64,
    link_up: bool,
    next_fault: Option<Fault>,
    statistics: BusStatistics,
    next_order: u64,
}

impl<const SLAVES: usize, const RX: usize> VirtualBus<SLAVES, RX> {
    pub fn new(configs: [SlaveConfig; SLAVES]) -> Result<Self, SimError> {
        if SLAVES == 0 || SLAVES > u16::MAX as usize || RX == 0 {
            return Err(SimError::InvalidConfiguration);
        }
        for config in configs {
            config.validate()?;
        }
        let mut bus = Self {
            slaves: configs.map(Esc::new),
            queue: [QueuedFrame::EMPTY; RX],
            now_ns: 0,
            link_up: true,
            next_fault: None,
            statistics: BusStatistics::default(),
            next_order: 0,
        };
        for (index, slave) in bus.slaves.iter_mut().enumerate() {
            slave.set_downstream_link(index + 1 < SLAVES);
        }
        Ok(bus)
    }

    pub fn advance_to(&mut self, time_ns: u64) -> Result<(), SimError> {
        if time_ns < self.now_ns {
            return Err(SimError::TimeRegression);
        }
        let elapsed = time_ns - self.now_ns;
        self.now_ns = time_ns;
        for slave in &mut self.slaves {
            slave.tick(time_ns, elapsed);
        }
        Ok(())
    }

    pub fn advance_by(&mut self, nanoseconds: u64) -> Result<(), SimError> {
        self.advance_to(
            self.now_ns
                .checked_add(nanoseconds)
                .ok_or(SimError::TimeOverflow)?,
        )
    }

    pub const fn statistics(&self) -> BusStatistics {
        self.statistics
    }

    pub fn slave(&self, index: usize) -> Option<SlaveSnapshot> {
        self.slaves.get(index).map(Esc::snapshot)
    }

    /// Initialize device-side application objects, independently of master
    /// traffic. Small objects and eight 1 KiB application objects per ESC are bounded.
    pub fn set_object(
        &mut self,
        slave: usize,
        index: u16,
        subindex: u8,
        data: &[u8],
    ) -> Result<(), SimError> {
        self.slaves
            .get_mut(slave)
            .ok_or(SimError::SlaveOutOfBounds)?
            .set_object(index, subindex, data)
    }

    pub fn emergency(
        &mut self,
        slave: usize,
        error_code: u16,
        error_register: u8,
        data: [u8; 5],
    ) -> Result<(), SimError> {
        self.slaves
            .get_mut(slave)
            .ok_or(SimError::SlaveOutOfBounds)?
            .emergency(error_code, error_register, data)
    }

    pub fn inject(&mut self, fault: Fault) -> Result<(), SimError> {
        match fault {
            Fault::Link(up) => self.link_up = up,
            Fault::SlaveLink { slave, up } => {
                self.slaves
                    .get_mut(slave)
                    .ok_or(SimError::SlaveOutOfBounds)?
                    .online = up
            }
            Fault::Al { slave, code } => self
                .slaves
                .get_mut(slave)
                .ok_or(SimError::SlaveOutOfBounds)?
                .al_fault(code),
            Fault::Drive { slave, code } => self
                .slaves
                .get_mut(slave)
                .ok_or(SimError::SlaveOutOfBounds)?
                .drive_fault(code),
            Fault::DcOffset { slave, nanoseconds } => {
                self.slaves
                    .get_mut(slave)
                    .ok_or(SimError::SlaveOutOfBounds)?
                    .dc_fault = nanoseconds
            }
            Fault::RepeatAck { slave, enabled } => {
                self.slaves
                    .get_mut(slave)
                    .ok_or(SimError::SlaveOutOfBounds)?
                    .repeat_ack = enabled
            }
            Fault::MailboxCounter { slave, next } => {
                self.slaves
                    .get_mut(slave)
                    .ok_or(SimError::SlaveOutOfBounds)?
                    .counter = next & 7
            }
            _ => self.next_fault = Some(fault),
        }
        Ok(())
    }

    fn enqueue(&mut self, mut frame: QueuedFrame) -> Result<(), SimError> {
        if let Some(slot) = self.queue.iter_mut().find(|slot| slot.len == 0) {
            frame.order = self.next_order;
            self.next_order = self.next_order.saturating_add(1);
            *slot = frame;
            Ok(())
        } else {
            self.statistics.queue_pressure = self.statistics.queue_pressure.saturating_add(1);
            Err(SimError::QueueFull)
        }
    }

    fn process(&mut self, frame: &mut [u8]) -> Result<(), SimError> {
        let end = validate_frame(frame)?;
        let mut offset = 16;
        while offset < end {
            let command = frame[offset];
            let mut adp = u16::from_le_bytes([frame[offset + 2], frame[offset + 3]]);
            let ado = u16::from_le_bytes([frame[offset + 4], frame[offset + 5]]);
            let logical = u32::from_le_bytes(
                frame[offset + 2..offset + 6]
                    .try_into()
                    .map_err(|_| SimError::InvalidFrame)?,
            );
            let len = (u16::from_le_bytes([frame[offset + 6], frame[offset + 7]]) & 0x7ff) as usize;
            let mut wkc = u16::from_le_bytes([frame[offset + 10 + len], frame[offset + 11 + len]]);
            let data = &mut frame[offset + 10..offset + 10 + len];
            let mut original = [0u8; MAX_ETHERNET_FRAME_LEN];
            original[..len].copy_from_slice(data);
            for slave in &mut self.slaves {
                // A broken serial link disconnects all downstream devices.
                if !slave.online {
                    break;
                }
                let auto = matches!(command, 1..=3 | 13);
                let broadcast = matches!(command, 7..=9);
                let selected = if auto {
                    adp == 0
                } else if broadcast {
                    true
                } else {
                    slave.station() == adp
                };
                if matches!(command, 10..=12) {
                    wkc = wkc.wrapping_add(slave.logical(
                        command,
                        logical,
                        data,
                        &original[..len],
                        self.now_ns,
                    ));
                } else if matches!(command, 13 | 14) {
                    if selected {
                        if slave.read(ado, data, self.now_ns) {
                            wkc = wkc.wrapping_add(1);
                        }
                    } else if slave.write(ado, data, self.now_ns) {
                        wkc = wkc.wrapping_add(1);
                    }
                } else if selected {
                    match command {
                        1 | 4 => {
                            if slave.read(ado, data, self.now_ns) {
                                wkc = wkc.wrapping_add(1);
                            }
                        }
                        2 | 5 | 8 => {
                            if slave.write(ado, data, self.now_ns) {
                                wkc = wkc.wrapping_add(1);
                            }
                        }
                        3 | 6 | 9 => {
                            if command == 9 {
                                let mut read = [0u8; MAX_ETHERNET_FRAME_LEN];
                                if slave.read(ado, &mut read[..len], self.now_ns) {
                                    for (dst, src) in data.iter_mut().zip(read) {
                                        *dst |= src;
                                    }
                                    wkc = wkc.wrapping_add(1);
                                }
                            } else if slave.read(ado, data, self.now_ns) {
                                wkc = wkc.wrapping_add(1);
                            }
                            if slave.write(ado, &original[..len], self.now_ns) {
                                wkc = wkc.wrapping_add(2);
                            }
                        }
                        7 => {
                            let mut read = [0u8; MAX_ETHERNET_FRAME_LEN];
                            if slave.read(ado, &mut read[..len], self.now_ns) {
                                for (dst, src) in data.iter_mut().zip(read) {
                                    *dst |= src;
                                }
                                wkc = wkc.wrapping_add(1);
                            }
                        }
                        _ => {}
                    }
                }
                if auto || broadcast {
                    adp = adp.wrapping_add(1);
                }
            }
            if matches!(command, 1..=3 | 7..=9 | 13) {
                frame[offset + 2..offset + 4].copy_from_slice(&adp.to_le_bytes());
            }
            frame[offset + 10 + len..offset + 12 + len].copy_from_slice(&wkc.to_le_bytes());
            offset += 12 + len;
        }
        Ok(())
    }
}

impl<const SLAVES: usize, const RX: usize> EthercatPort for VirtualBus<SLAVES, RX> {
    type Error = SimError;
    fn link_state(&self) -> LinkState {
        if self.link_up {
            LinkState::Up
        } else {
            LinkState::Down
        }
    }
    fn now_ns(&self) -> u64 {
        self.now_ns
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), SimError> {
        if !self.link_up {
            return Err(SimError::LinkDown);
        }
        if validate_frame(frame).is_err() {
            self.statistics.malformed = self.statistics.malformed.saturating_add(1);
            return Err(SimError::InvalidFrame);
        }
        let duplicate = matches!(self.next_fault, Some(Fault::DuplicateNextResponse));
        let required = if duplicate { 2 } else { 1 };
        if self.queue.iter().filter(|q| q.len == 0).count() < required
            && !matches!(self.next_fault, Some(Fault::DropNextResponse))
        {
            self.statistics.queue_pressure = self.statistics.queue_pressure.saturating_add(1);
            return Err(SimError::QueueFull);
        }
        let mut response = QueuedFrame::EMPTY;
        response.bytes[..frame.len()].copy_from_slice(frame);
        response.len = frame.len();
        response.ready_ns = self.now_ns;
        self.process(&mut response.bytes[..response.len])?;
        self.statistics.transmitted = self.statistics.transmitted.saturating_add(1);
        match self.next_fault.take() {
            Some(Fault::DropNextResponse) => {
                self.statistics.dropped = self.statistics.dropped.saturating_add(1);
                return Ok(());
            }
            Some(Fault::DelayNextResponse(delay)) => {
                response.ready_ns = self.now_ns.saturating_add(delay)
            }
            Some(Fault::CorruptNextResponse { offset, xor }) => {
                if offset < response.len {
                    response.bytes[offset] ^= xor;
                }
            }
            Some(Fault::WorkingCounterNextResponse { datagram, value }) => {
                let mut cursor = 16;
                for current in 0..=datagram {
                    if cursor + 12 > response.len {
                        break;
                    }
                    let len = (u16::from_le_bytes([
                        response.bytes[cursor + 6],
                        response.bytes[cursor + 7],
                    ]) & 0x7ff) as usize;
                    if current == datagram {
                        response.bytes[cursor + 10 + len..cursor + 12 + len]
                            .copy_from_slice(&value.to_le_bytes());
                    }
                    cursor += 12 + len;
                }
            }
            _ => {}
        }
        self.enqueue(response)?;
        if duplicate {
            self.enqueue(response)?;
        }
        Ok(())
    }
    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, SimError> {
        if !self.link_up {
            return Ok(RxPoll::LinkDown);
        }
        let next = self
            .queue
            .iter()
            .enumerate()
            .filter(|(_, q)| q.len != 0 && q.ready_ns <= self.now_ns)
            .min_by_key(|(_, q)| (q.ready_ns, q.order))
            .map(|(i, _)| i);
        if let Some(index) = next {
            let queued = &mut self.queue[index];
            let len = queued.len;
            destination[..len].copy_from_slice(&queued.bytes[..len]);
            queued.len = 0;
            self.statistics.received = self.statistics.received.saturating_add(1);
            Ok(RxPoll::Frame(len))
        } else {
            Ok(RxPoll::Empty)
        }
    }
}

// Validate the complete chained datagram envelope before touching ESC state.
fn validate_frame(frame: &[u8]) -> Result<usize, SimError> {
    if frame.len() < 28 || frame.len() > MAX_ETHERNET_FRAME_LEN || frame[12..14] != [0x88, 0xa4] {
        return Err(SimError::InvalidFrame);
    }
    let header = u16::from_le_bytes([frame[14], frame[15]]);
    if header & 0xf800 != 0x1000 {
        return Err(SimError::InvalidFrame);
    }
    let end = 16 + (header & 0x7ff) as usize;
    if end > frame.len() {
        return Err(SimError::InvalidFrame);
    }
    let mut cursor = 16;
    while cursor < end {
        if cursor + 12 > end || frame[cursor] > 14 {
            return Err(SimError::InvalidFrame);
        }
        let flags = u16::from_le_bytes([frame[cursor + 6], frame[cursor + 7]]);
        let next = cursor + 12 + (flags & 0x7ff) as usize;
        if next > end || flags & 0x3800 != 0 || (flags & 0x8000 != 0) != (next < end) {
            return Err(SimError::InvalidFrame);
        }
        cursor = next;
    }
    if cursor != end || cursor == 16 {
        return Err(SimError::InvalidFrame);
    }
    Ok(end)
}
