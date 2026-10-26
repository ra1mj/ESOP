//! Fixed-capacity caller-driven runtime CoE requests.
//!
//! A verified physical slave has exactly one mailbox transport owner. Logical
//! requests retain FIFO ownership on that slave while completed wire actions
//! rotate across slaves. The production scheduler owns all in-flight frames.

use crate::coe::{BoundedSdoTransfer, SdoAccess, SdoAccessPolicy, SdoError, SdoProgress};
use crate::mailbox::{
    MAILBOX_HEADER_LEN, MailboxConfig, MailboxController, MailboxError, MailboxMappedStatusBit,
    MailboxPhase, MailboxProtocol, MailboxTransportState,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdoRequestHandle {
    slot: u16,
    generation: u32,
}

impl SdoRequestHandle {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoRequestPhase {
    Queued,
    Active,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoRequestError {
    InvalidConfiguration,
    SlaveNotBound,
    AlreadyBound,
    QueueFull,
    InvalidHandle,
    Busy,
    NotComplete,
    SequenceExhausted,
    Timeout,
    Sdo(SdoError),
    Mailbox(MailboxError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdoRequestStatus {
    pub handle: SdoRequestHandle,
    pub phase: SdoRequestPhase,
    pub station_address: u16,
    pub deadline_ns: u64,
    pub error: Option<SdoRequestError>,
}

/// Object-safe scheduler binding; request submission/results remain typed on
/// the concrete bank. `finish_wire` must run after every completed or failed
/// control action, before selecting another slave.
pub trait RuntimeSdoService {
    fn is_due(&self, _now_ns: u64) -> bool {
        self.has_work()
    }
    fn prepare(&mut self, now_ns: u64) -> bool;
    fn current_mailbox(&self) -> Option<&MailboxController>;
    fn current_mailbox_mut(&mut self) -> Option<&mut MailboxController>;
    fn current_mapped_status(&self) -> Option<MailboxMappedStatusBit> {
        None
    }
    fn finish_wire(&mut self, now_ns: u64);
    fn has_work(&self) -> bool;
    fn last_terminal_status(&self) -> Option<SdoRequestStatus> {
        None
    }
    fn pop_terminal_status(&mut self) -> Option<SdoRequestStatus> {
        None
    }
    fn lost_terminal_statuses(&self) -> u64 {
        0
    }
}

struct RequestSlot<const OBJECT_BYTES: usize> {
    occupied: bool,
    generation: u32,
    ticket: u64,
    slave: usize,
    station_address: u16,
    deadline_ns: u64,
    phase: SdoRequestPhase,
    error: Option<SdoRequestError>,
    transfer: BoundedSdoTransfer<OBJECT_BYTES>,
}

impl<const OBJECT_BYTES: usize> RequestSlot<OBJECT_BYTES> {
    const fn new() -> Self {
        Self {
            occupied: false,
            generation: 1,
            ticket: 0,
            slave: 0,
            station_address: 0,
            deadline_ns: 0,
            phase: SdoRequestPhase::Queued,
            error: None,
            transfer: BoundedSdoTransfer::new(),
        }
    }
}

struct SlaveTransport {
    bound: bool,
    station_address: u16,
    wire_generation: u16,
    config: MailboxConfig,
    mailbox: MailboxController,
    active: Option<usize>,
    idle_due_ns: u64,
    idle_poll: bool,
    mapped_status: Option<MailboxMappedStatusBit>,
    started: bool,
}

impl SlaveTransport {
    const fn new() -> Self {
        Self {
            bound: false,
            station_address: 0,
            wire_generation: 0,
            config: MailboxConfig::new(0, 0, 0, 0),
            mailbox: MailboxController::without_transport(),
            active: None,
            idle_due_ns: 0,
            idle_poll: false,
            mapped_status: None,
            started: false,
        }
    }
}

pub struct SdoRequestBank<
    const SLAVES: usize,
    const REQUESTS: usize = 8,
    const OBJECT_BYTES: usize = 1024,
> {
    slaves: [SlaveTransport; SLAVES],
    requests: [RequestSlot<OBJECT_BYTES>; REQUESTS],
    selected: Option<usize>,
    next_slave: usize,
    next_ticket: u64,
    last_terminal: Option<SdoRequestStatus>,
    terminal_events: [Option<SdoRequestStatus>; REQUESTS],
    terminal_head: usize,
    terminal_len: usize,
    lost_terminal_events: u64,
}

impl<const SLAVES: usize, const REQUESTS: usize, const OBJECT_BYTES: usize>
    SdoRequestBank<SLAVES, REQUESTS, OBJECT_BYTES>
{
    pub const fn new() -> Self {
        Self {
            slaves: [const { SlaveTransport::new() }; SLAVES],
            requests: [const { RequestSlot::new() }; REQUESTS],
            selected: None,
            next_slave: 0,
            next_ticket: 1,
            last_terminal: None,
            terminal_events: [None; REQUESTS],
            terminal_head: 0,
            terminal_len: 0,
            lost_terminal_events: 0,
        }
    }

    /// Bind once after the caller verified this slave's startup/mailbox plan.
    /// Use `bind_verified_slave_with_transport` when configuration already
    /// used the mailbox; it preserves that physical transport's sequence.
    pub fn bind_verified_slave(
        &mut self,
        slot: usize,
        station_address: u16,
        config: MailboxConfig,
    ) -> Result<(), SdoRequestError> {
        let mut initial = MailboxController::new();
        let state = initial
            .release_transport_state()
            .map_err(SdoRequestError::Mailbox)?;
        self.bind_verified_slave_with_transport(slot, station_address, config, state)
    }

    pub fn bind_verified_slave_with_transport(
        &mut self,
        slot: usize,
        station_address: u16,
        config: MailboxConfig,
        state: MailboxTransportState,
    ) -> Result<(), SdoRequestError> {
        if slot >= SLAVES
            || station_address == 0
            || config.validate().is_err()
            || usize::from(config.send_capacity) < MAILBOX_HEADER_LEN + 10
            || usize::from(config.receive_capacity) < MAILBOX_HEADER_LEN + 10
        {
            return Err(SdoRequestError::InvalidConfiguration);
        }
        if self.slaves[slot].bound
            || self
                .slaves
                .iter()
                .any(|slave| slave.bound && slave.station_address == station_address)
        {
            return Err(SdoRequestError::AlreadyBound);
        }
        let slave = &mut self.slaves[slot];
        slave
            .mailbox
            .adopt_transport_state_for(station_address, state)
            .map_err(SdoRequestError::Mailbox)?;
        slave.bound = true;
        slave.station_address = station_address;
        slave.config = config;
        Ok(())
    }

    pub fn take_transport_state(
        &mut self,
        station_address: u16,
    ) -> Result<MailboxTransportState, SdoRequestError> {
        let index = self.slave_index(station_address)?;
        if self.selected == Some(index)
            || self.slaves[index].active.is_some()
            || self.requests.iter().any(|request| {
                request.occupied
                    && request.slave == index
                    && matches!(
                        request.phase,
                        SdoRequestPhase::Queued | SdoRequestPhase::Active
                    )
            })
        {
            return Err(SdoRequestError::Busy);
        }
        self.slaves[index]
            .mailbox
            .release_transport_state()
            .map_err(SdoRequestError::Mailbox)
    }

    pub fn return_transport_state(
        &mut self,
        station_address: u16,
        state: MailboxTransportState,
    ) -> Result<(), SdoRequestError> {
        let index = self.slave_index(station_address)?;
        self.slaves[index]
            .mailbox
            .adopt_transport_state_for(station_address, state)
            .map_err(SdoRequestError::Mailbox)
    }

    pub fn mailbox(&self, station_address: u16) -> Option<&MailboxController> {
        self.slave_index(station_address)
            .ok()
            .map(|index| &self.slaves[index].mailbox)
    }

    /// Freeze an already product-verified input mapping before this transport
    /// admits any work. Unavailable/stale observations never fall back to an
    /// unbudgeted direct register read.
    pub fn bind_verified_mapped_status(
        &mut self,
        station_address: u16,
        mapping: MailboxMappedStatusBit,
    ) -> Result<(), SdoRequestError> {
        let index = self.slave_index(station_address)?;
        let slave = &mut self.slaves[index];
        if slave.started || slave.mapped_status.is_some() {
            return Err(SdoRequestError::Busy);
        }
        let status = slave
            .config
            .status_bit
            .ok_or(SdoRequestError::InvalidConfiguration)?;
        let base = mapping
            .fmmu
            .logical_start
            .checked_sub(mapping.domain_bit_offset / 8)
            .ok_or(SdoRequestError::InvalidConfiguration)?;
        mapping
            .validate(base, status)
            .map_err(|_| SdoRequestError::InvalidConfiguration)?;
        slave.mapped_status = Some(mapping);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_upload(
        &mut self,
        station_address: u16,
        index: u16,
        subindex: u8,
        access: SdoAccess,
        policy: SdoAccessPolicy,
        now_ns: u64,
        deadline_ns: u64,
    ) -> Result<SdoRequestHandle, SdoRequestError> {
        self.submit(
            station_address,
            index,
            subindex,
            None,
            access,
            policy,
            now_ns,
            deadline_ns,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_download(
        &mut self,
        station_address: u16,
        index: u16,
        subindex: u8,
        data: &[u8],
        access: SdoAccess,
        policy: SdoAccessPolicy,
        now_ns: u64,
        deadline_ns: u64,
    ) -> Result<SdoRequestHandle, SdoRequestError> {
        self.submit(
            station_address,
            index,
            subindex,
            Some(data),
            access,
            policy,
            now_ns,
            deadline_ns,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn submit(
        &mut self,
        station_address: u16,
        index: u16,
        subindex: u8,
        data: Option<&[u8]>,
        access: SdoAccess,
        policy: SdoAccessPolicy,
        now_ns: u64,
        deadline_ns: u64,
    ) -> Result<SdoRequestHandle, SdoRequestError> {
        let slave = self.slave_index(station_address)?;
        if !self.slaves[slave].mailbox.owns_transport() {
            return Err(SdoRequestError::Busy);
        }
        if deadline_ns <= now_ns {
            return Err(SdoRequestError::Timeout);
        }
        let slot = self
            .requests
            .iter()
            .position(|request| !request.occupied && request.generation != 0)
            .ok_or(SdoRequestError::QueueFull)?;
        let slot_number = u16::try_from(slot).map_err(|_| SdoRequestError::InvalidConfiguration)?;
        let next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or(SdoRequestError::SequenceExhausted)?;
        let payload_capacity =
            usize::from(self.slaves[slave].config.send_capacity) - MAILBOX_HEADER_LEN;
        let mut transfer =
            BoundedSdoTransfer::with_mailbox_payload_capacity(policy, payload_capacity)
                .map_err(SdoRequestError::Sdo)?;
        match data {
            Some(data) => transfer.start_download(index, subindex, data, access),
            None => transfer.start_upload(index, subindex, access),
        }
        .map_err(SdoRequestError::Sdo)?;
        let request = &mut self.requests[slot];
        request.occupied = true;
        request.ticket = self.next_ticket;
        request.slave = slave;
        request.station_address = station_address;
        request.deadline_ns = deadline_ns;
        request.phase = SdoRequestPhase::Queued;
        request.error = None;
        request.transfer = transfer;
        self.next_ticket = next_ticket;
        self.slaves[slave].started = true;
        Ok(SdoRequestHandle {
            slot: slot_number,
            generation: request.generation,
        })
    }

    pub fn status(&self, handle: SdoRequestHandle) -> Result<SdoRequestStatus, SdoRequestError> {
        let request = self.request(handle)?;
        Ok(SdoRequestStatus {
            handle,
            phase: request.phase,
            station_address: request.station_address,
            deadline_ns: request.deadline_ns,
            error: request.error,
        })
    }

    pub fn result(&self, handle: SdoRequestHandle) -> Result<&[u8], SdoRequestError> {
        let request = self.request(handle)?;
        match request.phase {
            SdoRequestPhase::Complete => Ok(request.transfer.data()),
            SdoRequestPhase::Failed => Err(request.error.unwrap_or(SdoRequestError::NotComplete)),
            _ => Err(SdoRequestError::NotComplete),
        }
    }

    pub fn release(&mut self, handle: SdoRequestHandle) -> Result<(), SdoRequestError> {
        let request = self.request(handle)?;
        if !matches!(
            request.phase,
            SdoRequestPhase::Complete | SdoRequestPhase::Failed
        ) {
            return Err(SdoRequestError::Busy);
        }
        let request = &mut self.requests[handle.slot()];
        request.occupied = false;
        // Never wrap a handle generation into one that can name an old slot.
        request.generation = request.generation.checked_add(1).unwrap_or(0);
        Ok(())
    }

    fn request(
        &self,
        handle: SdoRequestHandle,
    ) -> Result<&RequestSlot<OBJECT_BYTES>, SdoRequestError> {
        self.requests
            .get(handle.slot())
            .filter(|request| {
                request.occupied
                    && request.generation != 0
                    && request.generation == handle.generation
            })
            .ok_or(SdoRequestError::InvalidHandle)
    }

    fn slave_index(&self, station_address: u16) -> Result<usize, SdoRequestError> {
        self.slaves
            .iter()
            .position(|slave| slave.bound && slave.station_address == station_address)
            .ok_or(SdoRequestError::SlaveNotBound)
    }

    fn fail_request(&mut self, slot: usize, error: SdoRequestError) {
        let request = &mut self.requests[slot];
        let _ = request.transfer.terminate(match error {
            SdoRequestError::Sdo(error) => error,
            _ => SdoError::Timeout,
        });
        request.phase = SdoRequestPhase::Failed;
        request.error = Some(error);
        let status = SdoRequestStatus {
            handle: SdoRequestHandle {
                slot: slot as u16,
                generation: request.generation,
            },
            phase: request.phase,
            station_address: request.station_address,
            deadline_ns: request.deadline_ns,
            error: request.error,
        };
        if self.slaves[request.slave].active == Some(slot) {
            self.slaves[request.slave].active = None;
            let _ = self.slaves[request.slave]
                .mailbox
                .abort_transaction(MailboxError::Timeout);
        }
        self.record_terminal(status);
    }

    fn record_terminal(&mut self, status: SdoRequestStatus) {
        self.last_terminal = Some(status);
        if self.terminal_len == REQUESTS {
            self.lost_terminal_events = self.lost_terminal_events.saturating_add(1);
            return;
        }
        let index = (self.terminal_head + self.terminal_len) % REQUESTS;
        self.terminal_events[index] = Some(status);
        self.terminal_len += 1;
    }

    fn start_wire_request(
        &mut self,
        slave: usize,
        slot: usize,
        now_ns: u64,
    ) -> Result<(), SdoRequestError> {
        let request = &self.requests[slot];
        let mut config = self.slaves[slave].config;
        config.timeout_ns = config
            .timeout_ns
            .min(request.deadline_ns.saturating_sub(now_ns));
        let transport = &mut self.slaves[slave];
        transport.wire_generation = transport.wire_generation.wrapping_add(1);
        let payload = request
            .transfer
            .request()
            .ok_or(SdoRequestError::NotComplete)?;
        if transport.mapped_status.is_some() {
            transport.mailbox.start_with_mapped_status(
                config,
                transport.station_address,
                transport.wire_generation,
                now_ns,
                MailboxProtocol::CoE,
                payload,
            )
        } else {
            transport.mailbox.start(
                config,
                transport.station_address,
                transport.wire_generation,
                now_ns,
                MailboxProtocol::CoE,
                payload,
            )
        }
        .map_err(SdoRequestError::Mailbox)
    }
}

impl<const SLAVES: usize, const REQUESTS: usize, const OBJECT_BYTES: usize> RuntimeSdoService
    for SdoRequestBank<SLAVES, REQUESTS, OBJECT_BYTES>
{
    fn is_due(&self, now_ns: u64) -> bool {
        self.slaves.iter().enumerate().any(|(index, slave)| {
            if !slave.bound || !slave.mailbox.owns_transport() {
                return false;
            }
            let queued = self.requests.iter().any(|request| {
                request.occupied
                    && request.slave == index
                    && request.phase == SdoRequestPhase::Queued
            });
            if slave.active.is_some() || slave.idle_poll {
                slave.mailbox.is_action_due(now_ns)
                    || self.requests.iter().any(|request| {
                        request.occupied
                            && request.slave == index
                            && matches!(
                                request.phase,
                                SdoRequestPhase::Queued | SdoRequestPhase::Active
                            )
                            && now_ns >= request.deadline_ns
                    })
            } else {
                queued || now_ns >= slave.idle_due_ns
            }
        })
    }

    fn prepare(&mut self, now_ns: u64) -> bool {
        if let Some(index) = self.selected {
            // The scheduler must finish ownership before rotating a pending wire action.
            if self.slaves[index].mailbox.pending().is_some() {
                return true;
            }
            self.finish_wire(now_ns);
        }
        for slot in 0..REQUESTS {
            if self.requests[slot].occupied
                && matches!(
                    self.requests[slot].phase,
                    SdoRequestPhase::Queued | SdoRequestPhase::Active
                )
                && now_ns >= self.requests[slot].deadline_ns
                && self.slaves[self.requests[slot].slave]
                    .mailbox
                    .pending()
                    .is_none()
            {
                self.fail_request(slot, SdoRequestError::Timeout);
            }
        }
        // Prefer queued/active work. Idle Emergency polling is lower priority.
        // Its expiry can release a station that already has queued work, so
        // make one bounded work pass afterward before reporting no action.
        for idle in [false, true, false] {
            for offset in 0..SLAVES {
                let index = (self.next_slave + offset) % SLAVES;
                if !self.slaves[index].bound || !self.slaves[index].mailbox.owns_transport() {
                    continue;
                }
                if self.slaves[index].active.is_none() && !self.slaves[index].idle_poll && !idle {
                    let next = self
                        .requests
                        .iter()
                        .enumerate()
                        .filter(|(_, request)| {
                            request.occupied
                                && request.slave == index
                                && request.phase == SdoRequestPhase::Queued
                        })
                        .min_by_key(|(_, request)| request.ticket)
                        .map(|(slot, _)| slot);
                    if let Some(slot) = next {
                        self.slaves[index].active = Some(slot);
                        self.requests[slot].phase = SdoRequestPhase::Active;
                        if let Err(error) = self.start_wire_request(index, slot, now_ns) {
                            self.fail_request(slot, error);
                            continue;
                        }
                    }
                }
                if idle {
                    if self.slaves[index].active.is_some()
                        || now_ns < self.slaves[index].idle_due_ns
                    {
                        continue;
                    }
                    if matches!(
                        self.slaves[index].mailbox.phase(),
                        MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted
                    ) {
                        let slave = &mut self.slaves[index];
                        slave.started = true;
                        slave.wire_generation = slave.wire_generation.wrapping_add(1);
                        let started = if slave.mapped_status.is_some() {
                            slave.mailbox.start_receive_with_mapped_status(
                                slave.config,
                                slave.station_address,
                                slave.wire_generation,
                                now_ns,
                            )
                        } else {
                            slave.mailbox.start_receive(
                                slave.config,
                                slave.station_address,
                                slave.wire_generation,
                                now_ns,
                            )
                        };
                        if started.is_err() {
                            continue;
                        }
                        slave.idle_poll = true;
                    }
                } else if self.slaves[index].active.is_none() {
                    continue;
                }
                match self.slaves[index].mailbox.next_action(now_ns) {
                    Ok(Some(_)) => {
                        self.selected = Some(index);
                        return true;
                    }
                    Ok(None) if self.slaves[index].mailbox.mapped_status_due(now_ns) => {
                        self.selected = Some(index);
                        return true;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(slot) = self.slaves[index].active {
                            self.fail_request(slot, SdoRequestError::Mailbox(error));
                        } else {
                            // An idle receive can expire while mandatory work
                            // owns the scheduler. Release its logical ownership
                            // so queued requests do not inherit a faulted poll.
                            self.slaves[index].idle_poll = false;
                        }
                        self.slaves[index].idle_due_ns =
                            now_ns.saturating_add(self.slaves[index].config.poll_interval_ns);
                    }
                }
            }
        }
        false
    }

    fn current_mailbox(&self) -> Option<&MailboxController> {
        self.selected.map(|index| &self.slaves[index].mailbox)
    }

    fn current_mailbox_mut(&mut self) -> Option<&mut MailboxController> {
        self.selected.map(|index| &mut self.slaves[index].mailbox)
    }

    fn current_mapped_status(&self) -> Option<MailboxMappedStatusBit> {
        self.selected
            .and_then(|index| self.slaves[index].mapped_status)
    }

    fn finish_wire(&mut self, now_ns: u64) {
        let Some(index) = self.selected.take() else {
            return;
        };
        if self.slaves[index].mailbox.pending().is_some() {
            self.selected = Some(index);
            return;
        }
        self.next_slave = if SLAVES == 0 { 0 } else { (index + 1) % SLAVES };
        if let Some(slot) = self.slaves[index].active {
            let phase = self.slaves[index].mailbox.phase();
            if phase == MailboxPhase::Faulted {
                let error = self.slaves[index]
                    .mailbox
                    .last_error()
                    .unwrap_or(MailboxError::NoPendingAction);
                self.fail_request(slot, SdoRequestError::Mailbox(error));
            } else if phase == MailboxPhase::Complete {
                let response = self.slaves[index]
                    .mailbox
                    .response()
                    .map(|(_, payload)| payload);
                let result = response
                    .ok_or(SdoError::Truncated)
                    .and_then(|payload| self.requests[slot].transfer.accept_response(payload));
                match result {
                    Ok(SdoProgress::Complete) => {
                        self.requests[slot].phase = SdoRequestPhase::Complete;
                        self.record_terminal(SdoRequestStatus {
                            handle: SdoRequestHandle {
                                slot: slot as u16,
                                generation: self.requests[slot].generation,
                            },
                            phase: SdoRequestPhase::Complete,
                            station_address: self.requests[slot].station_address,
                            deadline_ns: self.requests[slot].deadline_ns,
                            error: None,
                        });
                        self.slaves[index].active = None;
                    }
                    Ok(SdoProgress::Advanced) => {
                        if now_ns >= self.requests[slot].deadline_ns {
                            self.fail_request(slot, SdoRequestError::Timeout);
                        } else if let Err(error) = self.start_wire_request(index, slot, now_ns) {
                            self.fail_request(slot, error);
                        }
                    }
                    Err(error) => self.fail_request(slot, SdoRequestError::Sdo(error)),
                }
            }
        } else if self.slaves[index].idle_poll {
            let phase = self.slaves[index].mailbox.phase();
            if phase == MailboxPhase::CheckingStatus {
                let _ = self.slaves[index].mailbox.finish_idle_receive();
            }
            if matches!(
                self.slaves[index].mailbox.phase(),
                MailboxPhase::Idle | MailboxPhase::Complete | MailboxPhase::Faulted
            ) {
                self.slaves[index].idle_poll = false;
                self.slaves[index].idle_due_ns =
                    now_ns.saturating_add(self.slaves[index].config.poll_interval_ns);
            }
        }
    }

    fn has_work(&self) -> bool {
        self.selected.is_some()
            || self
                .slaves
                .iter()
                .any(|slave| slave.bound && slave.mailbox.owns_transport())
    }

    fn last_terminal_status(&self) -> Option<SdoRequestStatus> {
        self.last_terminal
    }

    fn pop_terminal_status(&mut self) -> Option<SdoRequestStatus> {
        if self.terminal_len == 0 {
            return None;
        }
        let event = self.terminal_events[self.terminal_head].take();
        self.terminal_head = (self.terminal_head + 1) % REQUESTS;
        self.terminal_len -= 1;
        event
    }

    fn lost_terminal_statuses(&self) -> u64 {
        self.lost_terminal_events
    }
}

impl<const SLAVES: usize, const REQUESTS: usize, const OBJECT_BYTES: usize> Default
    for SdoRequestBank<SLAVES, REQUESTS, OBJECT_BYTES>
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::{MailboxHeader, MailboxProgress};

    fn config() -> MailboxConfig {
        let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
        config.poll_interval_ns = 1;
        config.request_timeout_ns = 100;
        config
    }

    fn upload<const S: usize, const R: usize>(
        bank: &mut SdoRequestBank<S, R>,
        station: u16,
        index: u16,
        deadline: u64,
    ) -> SdoRequestHandle {
        bank.submit_upload(
            station,
            index,
            0,
            SdoAccess::Single,
            SdoAccessPolicy::new(false),
            0,
            deadline,
        )
        .unwrap()
    }

    fn response(body: &[u8], counter: u8) -> [u8; 32] {
        let mut bytes = [0; 32];
        MailboxHeader {
            length: body.len() as u16,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::CoE,
            counter,
        }
        .encode(&mut bytes)
        .unwrap();
        bytes[6..6 + body.len()].copy_from_slice(body);
        bytes
    }

    /// A small independent test peer: record each submitted object and answer
    /// with its index as four little-endian bytes. It never reads transfer state.
    fn drive_one<const S: usize, const R: usize>(
        bank: &mut SdoRequestBank<S, R>,
        now: u64,
        indices: &mut [u16; S],
        counters: &mut [u8; S],
        abort_station: Option<u16>,
    ) -> (u16, MailboxPhase) {
        assert!(bank.prepare(now));
        let mailbox = bank.current_mailbox_mut().unwrap();
        let phase = mailbox.phase();
        let action = mailbox.pending().unwrap();
        let station = action.station_address;
        let slave = usize::from(station - 1);
        let mut bytes = [0; 128];
        let len = action.datagram_len();
        match phase {
            MailboxPhase::CheckingSendStatus => {}
            MailboxPhase::Sending => {
                bytes[..len].copy_from_slice(action.payload());
                assert_eq!(len, 32);
                assert_eq!(&action.payload()[6..9], &[0, 0x20, 0x40]);
                indices[slave] = u16::from_le_bytes([action.payload()[9], action.payload()[10]]);
                counters[slave] = action.payload()[5] >> 4;
            }
            MailboxPhase::CheckingStatus => bytes[0] = 8,
            MailboxPhase::Polling => {
                let [low, high] = indices[slave].to_le_bytes();
                let body = if abort_station == Some(station) {
                    [0, 0x30, 0x80, low, high, 0, 0, 0, 2, 6]
                } else {
                    [0, 0x30, 0x43, low, high, 0, low, high, 0, 0]
                };
                // Zero and non-sequential receive counters are independent
                // of the outgoing counter captured above.
                bytes[..32].copy_from_slice(&response(&body, if station == 1 { 0 } else { 7 }));
            }
            _ => panic!("unexpected phase {phase:?}"),
        }
        mailbox
            .accept(action, action.generation, &bytes[..len], 1, now)
            .unwrap();
        bank.finish_wire(now);
        (station, phase)
    }

    #[test]
    fn eight_requests_complete_fifo_per_slave_with_wire_action_fairness() {
        let mut bank = SdoRequestBank::<2>::new();
        bank.bind_verified_slave(0, 1, config()).unwrap();
        bank.bind_verified_slave(1, 2, config()).unwrap();
        let handles = core::array::from_fn::<_, 8, _>(|slot| {
            upload(
                &mut bank,
                if slot < 4 { 1 } else { 2 },
                0x2000 + slot as u16,
                1_000,
            )
        });
        assert_eq!(
            bank.submit_upload(
                1,
                0x3000,
                0,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                0,
                1_000
            ),
            Err(SdoRequestError::QueueFull)
        );
        assert_eq!(bank.release(handles[0]), Err(SdoRequestError::Busy));
        let mut indices = [0; 2];
        let mut counters = [0; 2];
        let mut observed = [[0; 4]; 2];
        let mut writes = [0; 2];
        for tick in 1..=32 {
            let (station, phase) = drive_one(&mut bank, tick, &mut indices, &mut counters, None);
            assert_eq!(station, if tick % 2 == 1 { 1 } else { 2 });
            if phase == MailboxPhase::Sending {
                let slave = usize::from(station - 1);
                observed[slave][writes[slave]] = indices[slave];
                writes[slave] += 1;
            }
        }
        assert_eq!(
            observed,
            [
                [0x2000, 0x2001, 0x2002, 0x2003],
                [0x2004, 0x2005, 0x2006, 0x2007]
            ]
        );
        for (slot, handle) in handles.iter().copied().enumerate() {
            assert_eq!(
                bank.status(handle).unwrap().phase,
                SdoRequestPhase::Complete
            );
            assert_eq!(
                bank.result(handle).unwrap(),
                (0x2000u32 + slot as u32).to_le_bytes()
            );
        }
        let terminal = bank.last_terminal_status().unwrap();
        assert_eq!(terminal.handle, handles[7]);
        assert_eq!(terminal.phase, SdoRequestPhase::Complete);
        bank.release(handles[0]).unwrap();
        let replacement = upload(&mut bank, 1, 0x3000, 2_000);
        assert_eq!(replacement.slot(), handles[0].slot());
        assert_ne!(replacement.generation(), handles[0].generation());
        assert_eq!(bank.status(handles[0]), Err(SdoRequestError::InvalidHandle));
        assert_eq!(bank.last_terminal_status(), Some(terminal));
    }

    #[test]
    fn request_abort_is_local_and_releases_same_slave_fifo_ownership() {
        let mut bank = SdoRequestBank::<2, 3>::new();
        bank.bind_verified_slave(0, 1, config()).unwrap();
        bank.bind_verified_slave(1, 2, config()).unwrap();
        let aborted = upload(&mut bank, 1, 0x2000, 1_000);
        let same_slave = upload(&mut bank, 1, 0x2001, 1_000);
        let other_slave = upload(&mut bank, 2, 0x2002, 1_000);
        let mut indices = [0; 2];
        let mut counters = [0; 2];
        for tick in 1..=12 {
            drive_one(
                &mut bank,
                tick,
                &mut indices,
                &mut counters,
                if tick <= 8 { Some(1) } else { None },
            );
        }
        assert_eq!(
            bank.result(aborted),
            Err(SdoRequestError::Sdo(SdoError::Abort(0x0602_0000)))
        );
        assert_eq!(bank.result(same_slave).unwrap(), &[1, 0x20, 0, 0]);
        assert_eq!(bank.result(other_slave).unwrap(), &[2, 0x20, 0, 0]);
        assert_eq!(counters[0], 2);
    }

    #[test]
    fn queued_timeout_does_not_clear_another_request_on_the_same_slave() {
        let mut bank = SdoRequestBank::<1, 2>::new();
        bank.bind_verified_slave(0, 1, config()).unwrap();
        let first = upload(&mut bank, 1, 0x2000, 100);
        let queued = upload(&mut bank, 1, 0x2001, 2);
        let mut indices = [0];
        let mut counters = [0];
        for tick in [1, 3, 4, 5] {
            drive_one(&mut bank, tick, &mut indices, &mut counters, None);
        }
        assert_eq!(bank.result(first).unwrap(), &[0, 0x20, 0, 0]);
        assert_eq!(bank.result(queued), Err(SdoRequestError::Timeout));
        assert_eq!(counters[0], 1);
    }

    #[test]
    fn physical_transport_handoff_preserves_counter_and_revokes_the_old_owner() {
        let mut startup = MailboxController::new();
        startup.start_receive(config(), 1, 1, 0).unwrap();
        let status = startup.next_action(0).unwrap().unwrap();
        assert_eq!(
            startup.accept(status, 1, &[0], 1, 0),
            Ok(MailboxProgress::NoMessage)
        );
        startup.finish_idle_receive().unwrap();
        let token = startup.release_transport_state().unwrap();
        assert_eq!(
            startup.start(config(), 1, 1, 0, MailboxProtocol::CoE, &[1]),
            Err(MailboxError::TransportNotOwned)
        );
        let mut bank = SdoRequestBank::<1, 1>::new();
        bank.bind_verified_slave_with_transport(0, 1, config(), token)
            .unwrap();
        let handle = upload(&mut bank, 1, 0x2000, 100);
        let mut indices = [0];
        let mut counters = [0];
        for tick in 1..=4 {
            drive_one(&mut bank, tick, &mut indices, &mut counters, None);
        }
        assert!(bank.result(handle).is_ok());
        let token = bank.take_transport_state(1).unwrap();
        assert_eq!(
            bank.submit_upload(
                1,
                0x2001,
                0,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                5,
                100
            ),
            Err(SdoRequestError::Busy)
        );
        startup.adopt_transport_state(token).unwrap();
        startup
            .start(config(), 1, 1, 5, MailboxProtocol::CoE, &[1])
            .unwrap();
        let status = startup.next_action(5).unwrap().unwrap();
        startup.accept(status, 1, &[0], 1, 5).unwrap();
        let send = startup.next_action(5).unwrap().unwrap();
        assert_eq!(send.payload()[5] >> 4, 2);
    }

    #[test]
    fn idle_emergency_is_drained_without_transmitting_or_consuming_tx_counter() {
        let mut bank = SdoRequestBank::<1, 1>::new();
        bank.bind_verified_slave(0, 1, config()).unwrap();
        assert!(bank.prepare(0));
        let mailbox = bank.current_mailbox_mut().unwrap();
        let status = mailbox.pending().unwrap();
        mailbox
            .accept(status, status.generation, &[8], 1, 0)
            .unwrap();
        bank.finish_wire(0);
        assert!(bank.prepare(1));
        let mailbox = bank.current_mailbox_mut().unwrap();
        let read = mailbox.pending().unwrap();
        let emergency = response(&[0, 0x10, 0x10, 0x23, 0x81, 1, 2, 3, 4, 5], 0);
        assert_eq!(
            mailbox.accept(read, read.generation, &emergency, 1, 1),
            Ok(MailboxProgress::EmergencyConsumed)
        );
        bank.finish_wire(1);
        assert_eq!(
            bank.mailbox(1).unwrap().pop_emergency().unwrap().error_code,
            0x2310
        );
        let _handle = upload(&mut bank, 1, 0x2000, 100);
        let mut indices = [0];
        let mut counters = [0];
        drive_one(&mut bank, 2, &mut indices, &mut counters, None);
        drive_one(&mut bank, 3, &mut indices, &mut counters, None);
        assert_eq!(counters[0], 1);
    }

    #[test]
    fn idle_timeout_releases_poll_ownership_before_a_new_same_slave_request() {
        let mut bank = SdoRequestBank::<1, 1>::new();
        let mut config = config();
        config.timeout_ns = 10;
        bank.bind_verified_slave(0, 1, config).unwrap();
        assert!(bank.prepare(0));
        let mailbox = bank.current_mailbox_mut().unwrap();
        let status = mailbox.pending().unwrap();
        mailbox
            .accept(status, status.generation, &[8], 1, 0)
            .unwrap();
        bank.finish_wire(0);
        assert!(!bank.prepare(11));
        assert_eq!(bank.mailbox(1).unwrap().phase(), MailboxPhase::Faulted);
        let handle = bank
            .submit_upload(
                1,
                0x2000,
                0,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                12,
                100,
            )
            .unwrap();
        assert!(bank.is_due(12));
        let mut indices = [0];
        let mut counters = [0];
        for now in 12..=15 {
            drive_one(&mut bank, now, &mut indices, &mut counters, None);
        }
        assert_eq!(bank.result(handle).unwrap(), &[0, 0x20, 0, 0]);
        assert_eq!(bank.status(handle).unwrap().deadline_ns, 100);
        assert_eq!(bank.pop_terminal_status().unwrap().handle, handle);
        assert_eq!(bank.pop_terminal_status(), None);
    }

    #[test]
    fn eight_queued_expirations_remain_observable_as_distinct_terminal_events() {
        let mut bank = SdoRequestBank::<1>::new();
        bank.bind_verified_slave(0, 1, config()).unwrap();
        let handles =
            core::array::from_fn::<_, 8, _>(|slot| upload(&mut bank, 1, 0x2000 + slot as u16, 1));
        // Simulate higher-priority scheduler work keeping this bank unselected
        // until every queued absolute deadline has passed.
        assert!(bank.is_due(2));
        assert!(bank.current_mailbox().is_none());
        bank.prepare(2);
        for handle in handles {
            let event = bank.pop_terminal_status().unwrap();
            assert_eq!(event.handle, handle);
            assert_eq!(event.phase, SdoRequestPhase::Failed);
            assert_eq!(event.error, Some(SdoRequestError::Timeout));
            assert_eq!(bank.status(handle).unwrap(), event);
        }
        assert_eq!(bank.pop_terminal_status(), None);
        assert_eq!(bank.lost_terminal_statuses(), 0);
    }

    #[test]
    fn terminal_event_overflow_is_counted_without_blocking_or_duplicate_delivery() {
        let mut bank = SdoRequestBank::<1, 1>::new();
        bank.bind_verified_slave(0, 1, config()).unwrap();
        let first = upload(&mut bank, 1, 0x2000, 100);
        let mut indices = [0];
        let mut counters = [0];
        for tick in 1..=4 {
            drive_one(&mut bank, tick, &mut indices, &mut counters, None);
        }
        bank.finish_wire(4);
        bank.release(first).unwrap();
        let second = bank
            .submit_upload(
                1,
                0x2001,
                0,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                4,
                100,
            )
            .unwrap();
        for tick in 5..=8 {
            drive_one(&mut bank, tick, &mut indices, &mut counters, None);
        }
        assert_eq!(bank.result(second).unwrap(), &[1, 0x20, 0, 0]);
        assert_eq!(bank.last_terminal_status().unwrap().handle, second);
        assert_eq!(bank.lost_terminal_statuses(), 1);
        let event = bank.pop_terminal_status().unwrap();
        assert_eq!(event.handle, first);
        assert_eq!(event.phase, SdoRequestPhase::Complete);
        assert_eq!(bank.pop_terminal_status(), None);
        bank.finish_wire(8);
        assert_eq!(bank.pop_terminal_status(), None);
        assert_eq!(bank.lost_terminal_statuses(), 1);
    }
}
