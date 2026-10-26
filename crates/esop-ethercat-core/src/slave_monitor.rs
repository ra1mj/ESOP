//! Bounded runtime AL/DL observations over the shared control transport.
//!
//! AL and DL are staged as one sample. Only a complete pair becomes public;
//! failures invalidate the station immediately. This observer never writes AL,
//! rescans, reconfigures, or grants motion permission.
use crate::{
    AL_STATUS_WITH_CODE_LEN, AlStatus, ControlError, ControlRequest, ESC_AL_STATUS, ESC_DL_STATUS,
    ESC_DL_STATUS_LEN, EthercatState, RegisterOperation, RequestState, RxAddressPolicy,
    RxWorkingCounterPolicy, StartupAlFault, fixed_address,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveMonitorConfig {
    pub position: u16,
    pub station_address: u16,
    pub required_al_state: EthercatState,
    /// Raw ESC DL Status bits that must all be set (link bits are 4..7).
    pub required_dl_mask: u16,
}

impl SlaveMonitorConfig {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        required_al_state: EthercatState::Unknown,
        required_dl_mask: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveMonitorActionKind {
    AlStatus,
    DataLinkStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveMonitorAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub sample_sequence: u64,
    pub position: u16,
    pub station_address: u16,
    pub kind: SlaveMonitorActionKind,
    pub address: u32,
    pub length: usize,
    pub deadline_ns: u64,
}

impl SlaveMonitorAction {
    pub fn matches_request(self, request: &ControlRequest) -> bool {
        request.matches_action(
            self.datagram_index,
            self.generation,
            self.address,
            RegisterOperation::Read,
            &[],
            self.length,
            self.deadline_ns,
        ) && request.expected_wkc() == 1
            && request.working_counter_policy() == RxWorkingCounterPolicy::Exact
            && request.address_policy() == RxAddressPolicy::Exact
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveMonitorError {
    InvalidCapacity,
    InvalidTiming,
    InvalidStation,
    DuplicateStation,
    InvalidRequiredState,
    NoPendingAction,
    ActionMismatch,
    StartupAlFault(StartupAlFault),
    Control(ControlError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveMonitorProgress {
    AlStaged(u16),
    SamplePublished(u16),
    StationFailed(u16, SlaveMonitorError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveMonitorSnapshot {
    pub position: u16,
    pub station_address: u16,
    pub sample_sequence: u64,
    pub known: bool,
    /// Age starts at the earlier AL observation, not the later DL completion.
    pub observed_at_ns: u64,
    pub completed_at_ns: u64,
    pub al_status: AlStatus,
    pub dl_status: u16,
    pub al_generation: u16,
    pub dl_generation: u16,
    pub error: Option<SlaveMonitorError>,
}

impl SlaveMonitorSnapshot {
    const EMPTY: Self = Self {
        position: 0,
        station_address: 0,
        sample_sequence: 0,
        known: false,
        observed_at_ns: 0,
        completed_at_ns: 0,
        al_status: AlStatus::new(0, 0),
        dl_status: 0,
        al_generation: 0,
        dl_generation: 0,
        error: None,
    };

    pub const fn age_ns(self, now_ns: u64) -> Option<u64> {
        if self.known && now_ns >= self.observed_at_ns {
            Some(now_ns - self.observed_at_ns)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveMonitorQuality {
    pub stations: usize,
    /// Port-monotonic time at which the booleans below were evaluated.
    pub sampled_at_ns: u64,
    /// Inclusive freshness bound of the earliest expiring complete AL/DL pair.
    /// Unknown stations carry no freshness authority.
    pub fresh_until_ns: Option<u64>,
    pub known: bool,
    pub fresh: bool,
    pub al_good: bool,
    pub data_link_good: bool,
    pub fault_count: u64,
    pub lost_events: u64,
}

impl SlaveMonitorQuality {
    /// Quality at `sampled_at_ns`; use `is_good_at` when consuming it later.
    pub const fn is_good(self) -> bool {
        self.known && self.fresh && self.al_good && self.data_link_good
    }

    pub fn is_good_at(self, now_ns: u64) -> bool {
        self.is_good()
            && now_ns >= self.sampled_at_ns
            && self.fresh_until_ns.is_some_and(|until| now_ns <= until)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlaveMonitorEventKind {
    SamplePublished,
    AlStateFault(AlStatus),
    DataLinkFault(u16),
    ReadFailed(SlaveMonitorError),
    StartupAlFault(StartupAlFault),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlaveMonitorEvent {
    pub timestamp_ns: u64,
    pub position: u16,
    pub station_address: u16,
    /// Read events retain the exact action, generation and absolute deadline.
    /// Startup faults retain their original immutable source record instead.
    pub action: Option<SlaveMonitorAction>,
    pub kind: SlaveMonitorEventKind,
}

/// Object-safe scheduler boundary; request allocation and release stay with
/// the common production scheduler, so this service has no private wire owner.
pub trait SlaveMonitoringService {
    fn is_due(&self, now_ns: u64) -> bool;
    fn pending(&self) -> Option<SlaveMonitorAction>;
    fn next_action(&mut self, now_ns: u64) -> Option<SlaveMonitorAction>;
    fn accept_completed(
        &mut self,
        request: &ControlRequest,
        now_ns: u64,
    ) -> Result<SlaveMonitorProgress, SlaveMonitorError>;
    fn quality(&self, now_ns: u64) -> SlaveMonitorQuality;
    fn observe_startup_fault(&mut self, fault: StartupAlFault, now_ns: u64);
}

pub struct SlaveMonitorController<const SLAVES: usize, const EVENTS: usize = 64> {
    configs: [SlaveMonitorConfig; SLAVES],
    snapshots: [SlaveMonitorSnapshot; SLAVES],
    count: usize,
    cursor: usize,
    datagram_index: u8,
    cadence_ns: u64,
    max_age_ns: u64,
    request_timeout_ns: u64,
    due_at_ns: u64,
    next_generation: u16,
    next_token: u8,
    sample_sequence: u64,
    pending: Option<SlaveMonitorAction>,
    staged_al: Option<(AlStatus, u64, u16)>,
    events: [Option<SlaveMonitorEvent>; EVENTS],
    event_head: usize,
    event_len: usize,
    lost_events: u64,
    fault_count: u64,
    last_startup_fault: Option<StartupAlFault>,
}

impl<const SLAVES: usize, const EVENTS: usize> SlaveMonitorController<SLAVES, EVENTS> {
    pub fn new(
        datagram_index: u8,
        configs: &[SlaveMonitorConfig],
        cadence_ns: u64,
        max_age_ns: u64,
        request_timeout_ns: u64,
        now_ns: u64,
    ) -> Result<Self, SlaveMonitorError> {
        if configs.is_empty() || configs.len() > SLAVES {
            return Err(SlaveMonitorError::InvalidCapacity);
        }
        if cadence_ns == 0
            || max_age_ns == 0
            || request_timeout_ns == 0
            || now_ns.checked_add(request_timeout_ns).is_none()
        {
            return Err(SlaveMonitorError::InvalidTiming);
        }
        for (index, config) in configs.iter().enumerate() {
            if config.station_address == 0 {
                return Err(SlaveMonitorError::InvalidStation);
            }
            if config.required_al_state == EthercatState::Unknown {
                return Err(SlaveMonitorError::InvalidRequiredState);
            }
            if configs[..index].iter().any(|other| {
                other.station_address == config.station_address || other.position == config.position
            }) {
                return Err(SlaveMonitorError::DuplicateStation);
            }
        }
        let mut monitor = Self {
            configs: [SlaveMonitorConfig::EMPTY; SLAVES],
            snapshots: [SlaveMonitorSnapshot::EMPTY; SLAVES],
            count: configs.len(),
            cursor: 0,
            datagram_index,
            cadence_ns,
            max_age_ns,
            request_timeout_ns,
            due_at_ns: now_ns,
            next_generation: 1,
            next_token: 1,
            sample_sequence: 0,
            pending: None,
            staged_al: None,
            events: [None; EVENTS],
            event_head: 0,
            event_len: 0,
            lost_events: 0,
            fault_count: 0,
            last_startup_fault: None,
        };
        monitor.configs[..configs.len()].copy_from_slice(configs);
        for (snapshot, config) in monitor.snapshots.iter_mut().zip(configs) {
            snapshot.position = config.position;
            snapshot.station_address = config.station_address;
        }
        Ok(monitor)
    }

    pub fn snapshots(&self) -> &[SlaveMonitorSnapshot] {
        &self.snapshots[..self.count]
    }
    pub const fn lost_events(&self) -> u64 {
        self.lost_events
    }
    pub const fn fault_count(&self) -> u64 {
        self.fault_count
    }

    pub fn pop_event(&mut self) -> Option<SlaveMonitorEvent> {
        if self.event_len == 0 {
            return None;
        }
        let event = self.events[self.event_head].take();
        self.event_head = (self.event_head + 1) % EVENTS;
        self.event_len -= 1;
        event
    }

    fn record(&mut self, event: SlaveMonitorEvent) {
        if self.event_len == EVENTS {
            self.lost_events = self.lost_events.saturating_add(1);
            return;
        }
        self.events[(self.event_head + self.event_len) % EVENTS] = Some(event);
        self.event_len += 1;
    }

    fn finish_action(&mut self, now_ns: u64, advance_station: bool) {
        self.pending = None;
        self.due_at_ns = now_ns.saturating_add(self.cadence_ns);
        if advance_station {
            self.staged_al = None;
            self.cursor = (self.cursor + 1) % self.count;
        }
    }

    fn fail_station(
        &mut self,
        action: SlaveMonitorAction,
        error: SlaveMonitorError,
        now_ns: u64,
    ) -> SlaveMonitorProgress {
        let snapshot = &mut self.snapshots[self.cursor];
        snapshot.known = false;
        snapshot.error = Some(error);
        self.fault_count = self.fault_count.saturating_add(1);
        self.record(SlaveMonitorEvent {
            timestamp_ns: now_ns,
            position: action.position,
            station_address: action.station_address,
            action: Some(action),
            kind: SlaveMonitorEventKind::ReadFailed(error),
        });
        self.finish_action(now_ns, true);
        SlaveMonitorProgress::StationFailed(action.position, error)
    }
}

impl<const SLAVES: usize, const EVENTS: usize> SlaveMonitoringService
    for SlaveMonitorController<SLAVES, EVENTS>
{
    fn is_due(&self, now_ns: u64) -> bool {
        self.pending.is_some() || now_ns >= self.due_at_ns
    }
    fn pending(&self) -> Option<SlaveMonitorAction> {
        self.pending
    }

    fn next_action(&mut self, now_ns: u64) -> Option<SlaveMonitorAction> {
        if let Some(action) = self.pending {
            if now_ns > action.deadline_ns {
                self.fail_station(
                    action,
                    SlaveMonitorError::Control(ControlError::Timeout),
                    now_ns,
                );
                return None;
            }
            return self.pending;
        }
        if now_ns < self.due_at_ns {
            return None;
        }
        let config = self.configs[self.cursor];
        let (kind, register, length) = if self.staged_al.is_some() {
            (
                SlaveMonitorActionKind::DataLinkStatus,
                ESC_DL_STATUS,
                ESC_DL_STATUS_LEN,
            )
        } else {
            self.sample_sequence = self.sample_sequence.wrapping_add(1);
            (
                SlaveMonitorActionKind::AlStatus,
                ESC_AL_STATUS,
                AL_STATUS_WITH_CODE_LEN,
            )
        };
        let action = SlaveMonitorAction {
            token: self.next_token,
            datagram_index: self.datagram_index,
            generation: self.next_generation,
            sample_sequence: self.sample_sequence,
            position: config.position,
            station_address: config.station_address,
            kind,
            address: fixed_address(config.station_address, register),
            length: length as usize,
            deadline_ns: now_ns.saturating_add(self.request_timeout_ns),
        };
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.pending = Some(action);
        Some(action)
    }

    fn accept_completed(
        &mut self,
        request: &ControlRequest,
        now_ns: u64,
    ) -> Result<SlaveMonitorProgress, SlaveMonitorError> {
        let action = self.pending.ok_or(SlaveMonitorError::NoPendingAction)?;
        if !action.matches_request(request) {
            return Ok(self.fail_station(action, SlaveMonitorError::ActionMismatch, now_ns));
        }
        let error = if now_ns > action.deadline_ns {
            Some(ControlError::Timeout)
        } else if request.state == RequestState::Failed {
            Some(request.last_error().unwrap_or(ControlError::InvalidState))
        } else if request.state != RequestState::Complete {
            return Err(SlaveMonitorError::Control(ControlError::InvalidState));
        } else if request.actual_wkc != 1 {
            Some(ControlError::WorkingCounterMismatch)
        } else {
            None
        };
        if let Some(error) = error {
            return Ok(self.fail_station(action, SlaveMonitorError::Control(error), now_ns));
        }
        let payload = request.payload();
        match action.kind {
            SlaveMonitorActionKind::AlStatus => {
                let status = AlStatus::new(
                    u16::from_le_bytes([payload[0], payload[1]]),
                    u16::from_le_bytes([payload[4], payload[5]]),
                );
                let config = self.configs[self.cursor];
                if status.error || status.code != 0 || status.state != config.required_al_state {
                    self.fault_count = self.fault_count.saturating_add(1);
                    self.record(SlaveMonitorEvent {
                        timestamp_ns: now_ns,
                        position: action.position,
                        station_address: action.station_address,
                        action: Some(action),
                        kind: SlaveMonitorEventKind::AlStateFault(status),
                    });
                }
                self.staged_al = Some((status, now_ns, action.generation));
                self.finish_action(now_ns, false);
                Ok(SlaveMonitorProgress::AlStaged(action.position))
            }
            SlaveMonitorActionKind::DataLinkStatus => {
                let (al_status, observed_at_ns, al_generation) =
                    self.staged_al.ok_or(SlaveMonitorError::ActionMismatch)?;
                let dl_status = u16::from_le_bytes([payload[0], payload[1]]);
                let config = self.configs[self.cursor];
                self.snapshots[self.cursor] = SlaveMonitorSnapshot {
                    position: action.position,
                    station_address: action.station_address,
                    sample_sequence: action.sample_sequence,
                    known: true,
                    observed_at_ns,
                    completed_at_ns: now_ns,
                    al_status,
                    dl_status,
                    al_generation,
                    dl_generation: action.generation,
                    error: None,
                };
                let kind = if dl_status & config.required_dl_mask != config.required_dl_mask {
                    SlaveMonitorEventKind::DataLinkFault(dl_status)
                } else {
                    SlaveMonitorEventKind::SamplePublished
                };
                if kind != SlaveMonitorEventKind::SamplePublished {
                    self.fault_count = self.fault_count.saturating_add(1);
                }
                self.record(SlaveMonitorEvent {
                    timestamp_ns: now_ns,
                    position: action.position,
                    station_address: action.station_address,
                    action: Some(action),
                    kind,
                });
                self.finish_action(now_ns, true);
                Ok(SlaveMonitorProgress::SamplePublished(action.position))
            }
        }
    }

    fn quality(&self, now_ns: u64) -> SlaveMonitorQuality {
        let mut quality = SlaveMonitorQuality {
            stations: self.count,
            sampled_at_ns: now_ns,
            fresh_until_ns: Some(u64::MAX),
            known: true,
            fresh: true,
            al_good: true,
            data_link_good: true,
            fault_count: self.fault_count,
            lost_events: self.lost_events,
        };
        for (snapshot, config) in self.snapshots().iter().zip(&self.configs[..self.count]) {
            quality.fresh_until_ns = quality
                .fresh_until_ns
                .zip(
                    snapshot
                        .known
                        .then_some(snapshot.observed_at_ns.saturating_add(self.max_age_ns)),
                )
                .map(|(earliest, station)| earliest.min(station));
            quality.known &= snapshot.known;
            quality.fresh &= snapshot
                .age_ns(now_ns)
                .is_some_and(|age| age <= self.max_age_ns);
            quality.al_good &= snapshot.known
                && !snapshot.al_status.error
                && snapshot.al_status.code == 0
                && snapshot.al_status.state == config.required_al_state;
            quality.data_link_good &= snapshot.known
                && snapshot.dl_status & config.required_dl_mask == config.required_dl_mask;
        }
        if let Some((status, _, _)) = self.staged_al {
            quality.al_good &= !status.error
                && status.code == 0
                && status.state == self.configs[self.cursor].required_al_state;
        }
        quality
    }

    fn observe_startup_fault(&mut self, fault: StartupAlFault, now_ns: u64) {
        if self.last_startup_fault == Some(fault) {
            return;
        }
        self.last_startup_fault = Some(fault);
        self.fault_count = self.fault_count.saturating_add(1);
        for snapshot in &mut self.snapshots[..self.count] {
            if snapshot.station_address == fault.station_address {
                snapshot.known = false;
                snapshot.error = Some(SlaveMonitorError::StartupAlFault(fault));
            }
        }
        self.record(SlaveMonitorEvent {
            timestamp_ns: now_ns,
            position: fault.position,
            station_address: fault.station_address,
            action: None,
            kind: SlaveMonitorEventKind::StartupAlFault(fault),
        });
    }
}
