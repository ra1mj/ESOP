#![no_std]

//! Fixed-size runtime evidence and incident correlation for the Linux
//! eBPF observation boundary.
//!
//! Kernel programs and the CO-RE loader are platform adapters. They publish
//! [`RuntimeEvidence`] records into this bounded protocol; this crate does
//! not grant motion permission and has no EtherCAT write path.

use esop_lifecycle_guard::{HostObservation, ObservationState};

pub const MAX_INCIDENT_EVIDENCE: usize = 8;
pub const CAPABILITY_BTF: u64 = 1 << 0;
pub const CAPABILITY_RINGBUF: u64 = 1 << 1;
pub const CAPABILITY_VERIFIER: u64 = 1 << 2;
pub const CAPABILITY_PERMISSION: u64 = 1 << 3;

const FAULT_AGENT_CAPABILITY: u32 = 0x4542_1001;
const FAULT_AGENT_LOAD: u32 = 0x4542_1002;
const FAULT_AGENT_ATTACH: u32 = 0x4542_1003;
const FAULT_INCIDENT_DEGRADED: u32 = 0x4542_2001;
const FAULT_INCIDENT_LATCHED: u32 = 0x4542_2002;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CapabilitySnapshot {
    pub kernel_release_hash: u64,
    pub attach_mask: u64,
    pub required_attach_mask: u64,
    pub capability_mask: u64,
    pub missing_capabilities: u64,
    pub reserved: [u8; 8],
}

impl CapabilitySnapshot {
    pub const EMPTY: Self = Self {
        kernel_release_hash: 0,
        attach_mask: 0,
        required_attach_mask: 0,
        capability_mask: 0,
        missing_capabilities: 0,
        reserved: [0; 8],
    };

    pub const fn new(
        kernel_release_hash: u64,
        attach_mask: u64,
        required_attach_mask: u64,
        capability_mask: u64,
    ) -> Self {
        let missing_capabilities =
            (CAPABILITY_BTF | CAPABILITY_RINGBUF | CAPABILITY_VERIFIER | CAPABILITY_PERMISSION)
                & !capability_mask;
        Self {
            kernel_release_hash,
            attach_mask,
            required_attach_mask,
            capability_mask,
            missing_capabilities,
            reserved: [0; 8],
        }
    }

    pub const fn attach_ready(self) -> bool {
        self.missing_capabilities == 0
            && self.attach_mask & self.required_attach_mask == self.required_attach_mask
    }

    pub const fn hard_load_failure(self) -> bool {
        self.capability_mask & (CAPABILITY_VERIFIER | CAPABILITY_PERMISSION)
            != (CAPABILITY_VERIFIER | CAPABILITY_PERMISSION)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AgentState {
    Starting = 0,
    Healthy = 1,
    Degraded = 2,
    Failed = 3,
    Restarting = 4,
    Stopped = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EvidenceDomain {
    KernelScheduler = 0,
    KernelIrq = 1,
    KernelNetwork = 2,
    KernelMemory = 3,
    KernelProcess = 4,
    UserEsop = 5,
    UserRos = 6,
    UserZenoh = 7,
    Correlator = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EvidenceKind {
    SchedulerRunqueueLatency = 0,
    IrqCpuTime = 1,
    NetworkDrop = 2,
    PageFault = 3,
    OomKill = 4,
    ProcessExit = 5,
    CpuThrottle = 6,
    GatewayStall = 7,
    AgentCapabilityFailure = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IncidentCode {
    HostSchedulerStall = 0,
    HostIrqStorm = 1,
    HostNicDrop = 2,
    HostPageFault = 3,
    HostOom = 4,
    UserComponentExit = 5,
    HostCpuThrottle = 6,
    GatewayStall = 7,
    ObservabilityDegraded = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IncidentSeverity {
    Info = 0,
    Warning = 1,
    Error = 2,
    Critical = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RecommendedAction {
    ContinueObserve = 0,
    DegradeHostObservation = 1,
    ControlledStop = 2,
    LatchFault = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RuntimeEvidence {
    pub evidence_id: u64,
    pub boot_id: u64,
    pub agent_epoch: u64,
    pub timestamp_ns: u64,
    pub cycle_seq: u64,
    pub transition_seq: u64,
    pub pid: u32,
    pub tid: u32,
    pub cpu: u16,
    pub irq: u16,
    pub netdev_ifindex: u32,
    pub observed_value: u64,
    pub threshold: u64,
    pub duration_ns: u64,
    pub count: u32,
    pub domain: EvidenceDomain,
    pub kind: EvidenceKind,
    pub severity: IncidentSeverity,
    pub reserved: u8,
}

impl RuntimeEvidence {
    pub const EMPTY: Self = Self {
        evidence_id: 0,
        boot_id: 0,
        agent_epoch: 0,
        timestamp_ns: 0,
        cycle_seq: 0,
        transition_seq: 0,
        pid: 0,
        tid: 0,
        cpu: 0,
        irq: 0,
        netdev_ifindex: 0,
        observed_value: 0,
        threshold: 0,
        duration_ns: 0,
        count: 0,
        domain: EvidenceDomain::Correlator,
        kind: EvidenceKind::AgentCapabilityFailure,
        severity: IncidentSeverity::Info,
        reserved: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CycleContext {
    pub boot_id: u64,
    pub cycle_seq: u64,
    pub transition_seq: u64,
    pub timestamp_ns: u64,
    pub deadline_miss: u8,
    pub wkc_bad: u8,
    pub dc_bad: u8,
    pub reserved: u8,
    pub expected_wkc: u16,
    pub actual_wkc: u16,
    pub dc_offset_ns: i64,
    pub command_age_ns: u64,
    pub input_age_cycles: u64,
}

impl CycleContext {
    pub const EMPTY: Self = Self {
        boot_id: 0,
        cycle_seq: 0,
        transition_seq: 0,
        timestamp_ns: 0,
        deadline_miss: 0,
        wkc_bad: 0,
        dc_bad: 0,
        reserved: 0,
        expected_wkc: 0,
        actual_wkc: 0,
        dc_offset_ns: 0,
        command_age_ns: 0,
        input_age_cycles: 0,
    };

    pub const fn has_transport_risk(self) -> bool {
        self.deadline_miss != 0 || self.wkc_bad != 0 || self.dc_bad != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RuntimeIncident {
    pub incident_id: u64,
    pub boot_id: u64,
    pub agent_epoch: u64,
    pub code: IncidentCode,
    pub severity: IncidentSeverity,
    pub recommended_action: RecommendedAction,
    pub confidence_percent: u8,
    pub first_seen_ns: u64,
    pub last_seen_ns: u64,
    pub evidence_window_ns: u64,
    pub cycle_first: u64,
    pub cycle_last: u64,
    pub transition_seq: u64,
    pub pid: u32,
    pub tid: u32,
    pub cpu: u16,
    pub irq: u16,
    pub netdev_ifindex: u32,
    pub observed_value: u64,
    pub threshold: u64,
    pub count: u32,
    pub lost_events: u32,
    pub evidence_count: u8,
    pub reserved: [u8; 3],
    pub evidence: [RuntimeEvidence; MAX_INCIDENT_EVIDENCE],
}

impl RuntimeIncident {
    const EMPTY: Self = Self {
        incident_id: 0,
        boot_id: 0,
        agent_epoch: 0,
        code: IncidentCode::ObservabilityDegraded,
        severity: IncidentSeverity::Info,
        recommended_action: RecommendedAction::ContinueObserve,
        confidence_percent: 0,
        first_seen_ns: 0,
        last_seen_ns: 0,
        evidence_window_ns: 0,
        cycle_first: 0,
        cycle_last: 0,
        transition_seq: 0,
        pid: 0,
        tid: 0,
        cpu: 0,
        irq: 0,
        netdev_ifindex: 0,
        observed_value: 0,
        threshold: 0,
        count: 0,
        lost_events: 0,
        evidence_count: 0,
        reserved: [0; 3],
        evidence: [RuntimeEvidence::EMPTY; MAX_INCIDENT_EVIDENCE],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorrelatorError {
    BootMismatch,
    EvidenceEpochMismatch,
    CycleWindowExpired,
}

pub struct IncidentCorrelator<const INCIDENTS: usize> {
    boot_id: u64,
    agent_epoch: u64,
    window_ns: u64,
    next_incident_id: u64,
    context: CycleContext,
    incidents: [RuntimeIncident; INCIDENTS],
    head: usize,
    len: usize,
    dropped_incidents: u32,
}

impl<const INCIDENTS: usize> IncidentCorrelator<INCIDENTS> {
    pub const fn new(boot_id: u64, agent_epoch: u64, window_ns: u64) -> Self {
        Self {
            boot_id,
            agent_epoch,
            window_ns,
            next_incident_id: 1,
            context: CycleContext::EMPTY,
            incidents: [RuntimeIncident::EMPTY; INCIDENTS],
            head: 0,
            len: 0,
            dropped_incidents: 0,
        }
    }

    pub const fn context(&self) -> CycleContext {
        self.context
    }

    pub const fn dropped_incidents(&self) -> u32 {
        self.dropped_incidents
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn observe_cycle(&mut self, context: CycleContext) -> Result<(), CorrelatorError> {
        if context.boot_id != self.boot_id {
            return Err(CorrelatorError::BootMismatch);
        }
        self.context = context;
        Ok(())
    }

    pub fn ingest(
        &mut self,
        evidence: RuntimeEvidence,
    ) -> Result<Option<RuntimeIncident>, CorrelatorError> {
        if evidence.boot_id != self.boot_id {
            return Err(CorrelatorError::BootMismatch);
        }
        if evidence.agent_epoch != self.agent_epoch {
            return Err(CorrelatorError::EvidenceEpochMismatch);
        }

        let correlated = self.correlates_with_cycle(evidence);
        let Some((code, severity, action, confidence)) = classify(evidence, correlated) else {
            return Ok(None);
        };

        if let Some(index) = self.find_merge_target(code, evidence) {
            let incident = self.merge_incident(index, evidence, severity, action, confidence);
            return Ok(Some(incident));
        }

        let context = self.context;
        let cycle = if evidence.cycle_seq != 0 {
            evidence.cycle_seq
        } else {
            context.cycle_seq
        };
        let transition_seq = if evidence.transition_seq != 0 {
            evidence.transition_seq
        } else {
            context.transition_seq
        };
        let mut incident = RuntimeIncident {
            incident_id: self.next_incident_id,
            boot_id: self.boot_id,
            agent_epoch: self.agent_epoch,
            code,
            severity,
            recommended_action: action,
            confidence_percent: confidence,
            first_seen_ns: evidence.timestamp_ns,
            last_seen_ns: evidence.timestamp_ns,
            evidence_window_ns: self.window_ns,
            cycle_first: cycle,
            cycle_last: cycle,
            transition_seq,
            pid: evidence.pid,
            tid: evidence.tid,
            cpu: evidence.cpu,
            irq: evidence.irq,
            netdev_ifindex: evidence.netdev_ifindex,
            observed_value: evidence.observed_value,
            threshold: evidence.threshold,
            count: evidence.count,
            lost_events: 0,
            evidence_count: 1,
            reserved: [0; 3],
            evidence: [RuntimeEvidence::EMPTY; MAX_INCIDENT_EVIDENCE],
        };
        incident.evidence[0] = evidence;
        self.next_incident_id = self.next_incident_id.wrapping_add(1).max(1);
        self.push_incident(incident);
        Ok(Some(incident))
    }

    pub fn latest(&self) -> Option<RuntimeIncident> {
        if self.len == 0 {
            return None;
        }
        let index = (self.head + INCIDENTS - 1) % INCIDENTS;
        Some(self.incidents[index])
    }

    /// Remove and return the oldest retained incident.
    ///
    /// This is the bounded consumer side of the incident ring. A publisher
    /// can drain it to IPC or a recorder without exposing the backing array.
    pub fn pop(&mut self) -> Option<RuntimeIncident> {
        if self.len == 0 {
            return None;
        }
        let index = (self.head + INCIDENTS - self.len) % INCIDENTS;
        let incident = self.incidents[index];
        self.len -= 1;
        Some(incident)
    }

    /// Start a new agent epoch and discard incidents from the old process.
    pub fn restart(&mut self, agent_epoch: u64) {
        self.agent_epoch = agent_epoch;
        self.next_incident_id = 1;
        self.context = CycleContext::EMPTY;
        self.head = 0;
        self.len = 0;
        self.dropped_incidents = 0;
    }

    fn find_merge_target(&self, code: IncidentCode, evidence: RuntimeEvidence) -> Option<usize> {
        for offset in 0..self.len {
            let index = (self.head + INCIDENTS - 1 - offset) % INCIDENTS;
            let existing = self.incidents[index];
            if existing.code != code
                || existing.boot_id != evidence.boot_id
                || existing.agent_epoch != evidence.agent_epoch
            {
                continue;
            }
            if existing.pid != 0 && evidence.pid != 0 && existing.pid != evidence.pid {
                continue;
            }
            if existing.tid != 0 && evidence.tid != 0 && existing.tid != evidence.tid {
                continue;
            }
            if existing.netdev_ifindex != 0
                && evidence.netdev_ifindex != 0
                && existing.netdev_ifindex != evidence.netdev_ifindex
            {
                continue;
            }
            if existing.last_seen_ns.abs_diff(evidence.timestamp_ns) <= self.window_ns {
                return Some(index);
            }
        }
        None
    }

    fn merge_incident(
        &mut self,
        index: usize,
        evidence: RuntimeEvidence,
        severity: IncidentSeverity,
        action: RecommendedAction,
        confidence: u8,
    ) -> RuntimeIncident {
        let incident = &mut self.incidents[index];
        incident.first_seen_ns = incident.first_seen_ns.min(evidence.timestamp_ns);
        incident.last_seen_ns = incident.last_seen_ns.max(evidence.timestamp_ns);
        incident.cycle_first = merge_first_nonzero(incident.cycle_first, evidence.cycle_seq);
        incident.cycle_last = incident.cycle_last.max(evidence.cycle_seq);
        if incident.transition_seq == 0 {
            incident.transition_seq = evidence.transition_seq;
        }
        incident.severity = max_severity(incident.severity, severity);
        incident.recommended_action = max_action(incident.recommended_action, action);
        incident.confidence_percent = incident.confidence_percent.max(confidence);
        incident.observed_value = incident.observed_value.max(evidence.observed_value);
        incident.threshold = incident.threshold.max(evidence.threshold);
        incident.count = incident.count.saturating_add(evidence.count);
        if incident.evidence_count < MAX_INCIDENT_EVIDENCE as u8 {
            incident.evidence[incident.evidence_count as usize] = evidence;
            incident.evidence_count += 1;
        } else {
            incident.lost_events = incident.lost_events.saturating_add(1);
        }
        *incident
    }

    fn correlates_with_cycle(&self, evidence: RuntimeEvidence) -> bool {
        if self.context.cycle_seq == 0 || !self.context.has_transport_risk() {
            return false;
        }
        if evidence.cycle_seq != 0 {
            return evidence.cycle_seq == self.context.cycle_seq;
        }
        let age = evidence.timestamp_ns.abs_diff(self.context.timestamp_ns);
        age <= self.window_ns
    }

    fn push_incident(&mut self, incident: RuntimeIncident) {
        if INCIDENTS == 0 {
            self.dropped_incidents = self.dropped_incidents.saturating_add(1);
            return;
        }
        self.incidents[self.head] = incident;
        self.head = (self.head + 1) % INCIDENTS;
        if self.len < INCIDENTS {
            self.len += 1;
        } else {
            self.dropped_incidents = self.dropped_incidents.saturating_add(1);
        }
    }
}

fn merge_first_nonzero(current: u64, incoming: u64) -> u64 {
    match (current, incoming) {
        (0, value) => value,
        (current, 0) => current,
        (current, incoming) => current.min(incoming),
    }
}

fn max_severity(left: IncidentSeverity, right: IncidentSeverity) -> IncidentSeverity {
    if (right as u8) > left as u8 {
        right
    } else {
        left
    }
}

fn max_action(left: RecommendedAction, right: RecommendedAction) -> RecommendedAction {
    if (right as u8) > left as u8 {
        right
    } else {
        left
    }
}

fn classify(
    evidence: RuntimeEvidence,
    correlated: bool,
) -> Option<(IncidentCode, IncidentSeverity, RecommendedAction, u8)> {
    let over_threshold = evidence.observed_value > evidence.threshold;
    let enough_count = evidence.count > 0 && evidence.count >= evidence.threshold as u32;
    match evidence.kind {
        EvidenceKind::SchedulerRunqueueLatency if over_threshold && correlated => Some((
            IncidentCode::HostSchedulerStall,
            IncidentSeverity::Error,
            RecommendedAction::ControlledStop,
            70,
        )),
        EvidenceKind::IrqCpuTime if over_threshold && correlated => Some((
            IncidentCode::HostIrqStorm,
            IncidentSeverity::Error,
            RecommendedAction::ControlledStop,
            70,
        )),
        EvidenceKind::NetworkDrop if enough_count && correlated => Some((
            IncidentCode::HostNicDrop,
            IncidentSeverity::Error,
            RecommendedAction::ControlledStop,
            75,
        )),
        EvidenceKind::PageFault if correlated => Some((
            IncidentCode::HostPageFault,
            IncidentSeverity::Warning,
            RecommendedAction::DegradeHostObservation,
            65,
        )),
        EvidenceKind::OomKill => Some((
            IncidentCode::HostOom,
            IncidentSeverity::Critical,
            RecommendedAction::LatchFault,
            100,
        )),
        EvidenceKind::ProcessExit => Some((
            IncidentCode::UserComponentExit,
            IncidentSeverity::Critical,
            RecommendedAction::LatchFault,
            100,
        )),
        EvidenceKind::CpuThrottle if correlated => Some((
            IncidentCode::HostCpuThrottle,
            IncidentSeverity::Error,
            RecommendedAction::DegradeHostObservation,
            70,
        )),
        EvidenceKind::GatewayStall if correlated => Some((
            IncidentCode::GatewayStall,
            IncidentSeverity::Error,
            RecommendedAction::ControlledStop,
            75,
        )),
        EvidenceKind::AgentCapabilityFailure => Some((
            IncidentCode::ObservabilityDegraded,
            IncidentSeverity::Warning,
            RecommendedAction::DegradeHostObservation,
            100,
        )),
        _ => None,
    }
}

pub struct AgentHealth {
    boot_id: u64,
    agent_epoch: u64,
    heartbeat_seq: u64,
    state: AgentState,
    attach_mask: u64,
    lost_event_count: u32,
    incident_count: u32,
    fault_code: u32,
    last_event_ns: u64,
}

impl AgentHealth {
    pub const fn new(boot_id: u64, agent_epoch: u64) -> Self {
        Self {
            boot_id,
            agent_epoch,
            heartbeat_seq: 0,
            state: AgentState::Starting,
            attach_mask: 0,
            lost_event_count: 0,
            incident_count: 0,
            fault_code: 0,
            last_event_ns: 0,
        }
    }

    pub const fn state(&self) -> AgentState {
        self.state
    }

    pub const fn attach_mask(&self) -> u64 {
        self.attach_mask
    }

    pub const fn lost_event_count(&self) -> u32 {
        self.lost_event_count
    }

    pub fn set_capabilities(&mut self, attach_mask: u64, required_mask: u64, btf_ok: bool) {
        self.attach_mask = attach_mask;
        if !btf_ok || attach_mask & required_mask != required_mask {
            self.state = AgentState::Degraded;
            self.fault_code = FAULT_AGENT_CAPABILITY;
        } else {
            self.state = AgentState::Healthy;
            self.fault_code = 0;
        }
    }

    /// Apply a loader preflight result without making the agent own the
    /// kernel-loading implementation. Missing observation points degrade the
    /// evidence quality; verifier or permission failures make loading unsafe.
    pub fn set_capability_snapshot(&mut self, snapshot: CapabilitySnapshot) {
        self.attach_mask = snapshot.attach_mask;
        if snapshot.hard_load_failure() {
            self.state = AgentState::Failed;
            self.fault_code = FAULT_AGENT_LOAD;
        } else if !snapshot.attach_ready() {
            self.state = AgentState::Degraded;
            self.fault_code = if snapshot.missing_capabilities != 0 {
                FAULT_AGENT_CAPABILITY
            } else {
                FAULT_AGENT_ATTACH
            };
        } else {
            self.state = AgentState::Healthy;
            self.fault_code = 0;
        }
    }

    pub fn record_event_loss(&mut self, count: u32) {
        self.lost_event_count = self.lost_event_count.saturating_add(count);
        if self.state == AgentState::Healthy {
            self.state = AgentState::Degraded;
        }
    }

    pub fn record_incident(&mut self) {
        self.incident_count = self.incident_count.saturating_add(1);
    }

    /// Project a correlator result into the observer's health lease.
    ///
    /// This changes only the evidence state exported through
    /// `HostObservation`; it does not control EtherCAT or motion directly.
    pub fn observe_incident(&mut self, incident: RuntimeIncident) {
        self.record_incident();
        match incident.recommended_action {
            RecommendedAction::ContinueObserve => {}
            RecommendedAction::DegradeHostObservation | RecommendedAction::ControlledStop => {
                if self.state != AgentState::Failed {
                    self.state = AgentState::Degraded;
                    self.fault_code = FAULT_INCIDENT_DEGRADED;
                }
            }
            RecommendedAction::LatchFault => {
                self.state = AgentState::Failed;
                self.fault_code = FAULT_INCIDENT_LATCHED;
            }
        }
    }

    pub fn restart(&mut self, agent_epoch: u64) {
        self.agent_epoch = agent_epoch;
        self.heartbeat_seq = 0;
        self.state = AgentState::Restarting;
        self.last_event_ns = 0;
    }

    pub fn heartbeat(&mut self, now_ns: u64) -> HostObservation {
        self.heartbeat_seq = self.heartbeat_seq.wrapping_add(1).max(1);
        self.last_event_ns = now_ns;
        HostObservation {
            boot_id: self.boot_id,
            agent_epoch: self.agent_epoch,
            heartbeat_seq: self.heartbeat_seq,
            observed_at_ns: now_ns,
            state: match self.state {
                AgentState::Healthy => ObservationState::Healthy,
                AgentState::Starting | AgentState::Degraded | AgentState::Restarting => {
                    ObservationState::Degraded
                }
                AgentState::Failed | AgentState::Stopped => ObservationState::Failed,
            },
            reserved: [0; 7],
            attach_mask: self.attach_mask,
            lost_event_count: self.lost_event_count,
            incident_count: self.incident_count,
            fault_code: self.fault_code,
        }
    }
}

/// Fixed-size runtime facade that connects evidence ingestion, incident
/// correlation, and the host-observation lease without granting motion
/// control to the observation agent.
pub struct RuntimeAgent<const INCIDENTS: usize> {
    health: AgentHealth,
    correlator: IncidentCorrelator<INCIDENTS>,
}

impl<const INCIDENTS: usize> RuntimeAgent<INCIDENTS> {
    pub const fn new(boot_id: u64, agent_epoch: u64, window_ns: u64) -> Self {
        Self {
            health: AgentHealth::new(boot_id, agent_epoch),
            correlator: IncidentCorrelator::new(boot_id, agent_epoch, window_ns),
        }
    }

    pub const fn health(&self) -> &AgentHealth {
        &self.health
    }

    pub const fn health_mut(&mut self) -> &mut AgentHealth {
        &mut self.health
    }

    pub const fn correlator(&self) -> &IncidentCorrelator<INCIDENTS> {
        &self.correlator
    }

    pub fn correlator_mut(&mut self) -> &mut IncidentCorrelator<INCIDENTS> {
        &mut self.correlator
    }

    pub fn observe_cycle(&mut self, context: CycleContext) -> Result<(), CorrelatorError> {
        self.correlator.observe_cycle(context)
    }

    pub fn ingest(
        &mut self,
        evidence: RuntimeEvidence,
    ) -> Result<Option<RuntimeIncident>, CorrelatorError> {
        let incident = self.correlator.ingest(evidence)?;
        if let Some(incident) = incident {
            self.health.observe_incident(incident);
            Ok(Some(incident))
        } else {
            Ok(None)
        }
    }

    pub fn heartbeat(&mut self, now_ns: u64) -> HostObservation {
        self.health.heartbeat(now_ns)
    }

    pub fn pop_incident(&mut self) -> Option<RuntimeIncident> {
        self.correlator.pop()
    }

    pub fn restart(&mut self, agent_epoch: u64) {
        self.health.restart(agent_epoch);
        self.correlator.restart(agent_epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(kind: EvidenceKind, timestamp_ns: u64) -> RuntimeEvidence {
        RuntimeEvidence {
            evidence_id: 7,
            boot_id: 11,
            agent_epoch: 3,
            timestamp_ns,
            cycle_seq: 42,
            transition_seq: 9,
            pid: 100,
            tid: 101,
            cpu: 2,
            irq: 0,
            netdev_ifindex: 4,
            observed_value: 20,
            threshold: 10,
            duration_ns: 20,
            count: 20,
            domain: EvidenceDomain::KernelScheduler,
            kind,
            severity: IncidentSeverity::Error,
            reserved: 0,
        }
    }

    #[test]
    fn correlates_scheduler_stall_with_cycle_risk() {
        let mut correlator = IncidentCorrelator::<2>::new(11, 3, 1_000);
        correlator
            .observe_cycle(CycleContext {
                boot_id: 11,
                cycle_seq: 42,
                transition_seq: 9,
                timestamp_ns: 1_000,
                deadline_miss: 1,
                ..CycleContext::EMPTY
            })
            .unwrap();
        let incident = correlator
            .ingest(evidence(EvidenceKind::SchedulerRunqueueLatency, 1_100))
            .unwrap()
            .unwrap();
        assert_eq!(incident.code, IncidentCode::HostSchedulerStall);
        assert_eq!(incident.cycle_first, 42);
        assert_eq!(
            incident.recommended_action,
            RecommendedAction::ControlledStop
        );
    }

    #[test]
    fn soft_evidence_outside_risk_window_is_not_called_a_root_cause() {
        let mut correlator = IncidentCorrelator::<2>::new(11, 3, 1_000);
        correlator
            .observe_cycle(CycleContext {
                boot_id: 11,
                cycle_seq: 42,
                timestamp_ns: 1_000,
                deadline_miss: 0,
                ..CycleContext::EMPTY
            })
            .unwrap();
        assert_eq!(
            correlator
                .ingest(evidence(EvidenceKind::SchedulerRunqueueLatency, 1_100))
                .unwrap(),
            None
        );
    }

    #[test]
    fn hard_facts_create_incidents_without_cycle_context() {
        let mut correlator = IncidentCorrelator::<2>::new(11, 3, 1_000);
        let incident = correlator
            .ingest(evidence(EvidenceKind::ProcessExit, 2_000))
            .unwrap()
            .unwrap();
        assert_eq!(incident.code, IncidentCode::UserComponentExit);
        assert_eq!(incident.confidence_percent, 100);
        assert_eq!(incident.recommended_action, RecommendedAction::LatchFault);
    }

    #[test]
    fn incident_ring_is_bounded_and_counts_overwrite() {
        let mut correlator = IncidentCorrelator::<1>::new(11, 3, 1_000);
        for evidence_id in 1..=3 {
            let mut current = evidence(EvidenceKind::OomKill, evidence_id * 2_000);
            current.evidence_id = evidence_id;
            current.pid = evidence_id as u32;
            correlator.ingest(current).unwrap();
        }
        assert_eq!(correlator.len(), 1);
        assert_eq!(correlator.dropped_incidents(), 2);
        assert_eq!(correlator.latest().unwrap().incident_id, 3);
    }

    #[test]
    fn same_problem_is_aggregated_within_the_bounded_window() {
        let mut correlator = IncidentCorrelator::<2>::new(11, 3, 1_000);
        let mut first = evidence(EvidenceKind::ProcessExit, 2_000);
        first.evidence_id = 1;
        let first_incident = correlator.ingest(first).unwrap().unwrap();

        let mut second = evidence(EvidenceKind::ProcessExit, 2_500);
        second.evidence_id = 2;
        let merged = correlator.ingest(second).unwrap().unwrap();

        assert_eq!(merged.incident_id, first_incident.incident_id);
        assert_eq!(merged.evidence_count, 2);
        assert_eq!(merged.first_seen_ns, 2_000);
        assert_eq!(merged.last_seen_ns, 2_500);
        assert_eq!(correlator.len(), 1);
        assert_eq!(
            correlator.pop().unwrap().incident_id,
            first_incident.incident_id
        );
        assert!(correlator.pop().is_none());
    }

    #[test]
    fn agent_health_degrades_on_capability_gap_and_exports_heartbeat() {
        let mut health = AgentHealth::new(11, 3);
        health.set_capabilities(0b01, 0b11, true);
        health.record_event_loss(2);
        let observation = health.heartbeat(5_000);
        assert_eq!(health.state(), AgentState::Degraded);
        assert_eq!(observation.boot_id, 11);
        assert_eq!(observation.agent_epoch, 3);
        assert_eq!(observation.heartbeat_seq, 1);
        assert_eq!(observation.state, ObservationState::Degraded);
        assert_eq!(observation.lost_event_count, 2);
    }

    #[test]
    fn agent_restart_changes_epoch_and_never_reuses_heartbeat_sequence() {
        let mut health = AgentHealth::new(11, 3);
        health.set_capabilities(0b11, 0b11, true);
        assert_eq!(health.heartbeat(1).heartbeat_seq, 1);
        health.restart(4);
        let observation = health.heartbeat(2);
        assert_eq!(observation.agent_epoch, 4);
        assert_eq!(observation.heartbeat_seq, 1);
        assert_eq!(observation.state, ObservationState::Degraded);
    }

    #[test]
    fn capability_snapshot_distinguishes_degraded_observation_from_load_failure() {
        let mut health = AgentHealth::new(11, 3);
        health.set_capability_snapshot(CapabilitySnapshot::new(
            0xAA,
            0b01,
            0b11,
            CAPABILITY_BTF | CAPABILITY_RINGBUF | CAPABILITY_VERIFIER | CAPABILITY_PERMISSION,
        ));
        assert_eq!(health.state(), AgentState::Degraded);

        health.set_capability_snapshot(CapabilitySnapshot::new(
            0xAA,
            0b11,
            0b11,
            CAPABILITY_BTF | CAPABILITY_RINGBUF | CAPABILITY_VERIFIER,
        ));
        assert_eq!(health.state(), AgentState::Failed);

        health.set_capability_snapshot(CapabilitySnapshot::new(
            0xAA,
            0b11,
            0b11,
            CAPABILITY_BTF | CAPABILITY_RINGBUF | CAPABILITY_VERIFIER | CAPABILITY_PERMISSION,
        ));
        assert_eq!(health.state(), AgentState::Healthy);
    }

    #[test]
    fn stale_boot_or_epoch_evidence_is_rejected() {
        let mut correlator = IncidentCorrelator::<1>::new(11, 3, 100);
        let mut wrong_boot = evidence(EvidenceKind::OomKill, 1);
        wrong_boot.boot_id = 12;
        assert_eq!(
            correlator.ingest(wrong_boot),
            Err(CorrelatorError::BootMismatch)
        );
        let mut wrong_epoch = evidence(EvidenceKind::OomKill, 1);
        wrong_epoch.agent_epoch = 4;
        assert_eq!(
            correlator.ingest(wrong_epoch),
            Err(CorrelatorError::EvidenceEpochMismatch)
        );
    }

    #[test]
    fn incident_action_degrades_or_latches_the_observation_lease() {
        let mut health = AgentHealth::new(11, 3);
        health.set_capabilities(0b11, 0b11, true);

        let degraded = RuntimeIncident {
            recommended_action: RecommendedAction::ControlledStop,
            ..RuntimeIncident::EMPTY
        };
        health.observe_incident(degraded);
        let observation = health.heartbeat(10);
        assert_eq!(observation.state, ObservationState::Degraded);
        assert_eq!(observation.fault_code, FAULT_INCIDENT_DEGRADED);
        assert_eq!(observation.incident_count, 1);

        let latched = RuntimeIncident {
            recommended_action: RecommendedAction::LatchFault,
            ..RuntimeIncident::EMPTY
        };
        health.observe_incident(latched);
        let observation = health.heartbeat(20);
        assert_eq!(observation.state, ObservationState::Failed);
        assert_eq!(observation.fault_code, FAULT_INCIDENT_LATCHED);
        assert_eq!(observation.incident_count, 2);
    }

    #[test]
    fn runtime_agent_routes_incidents_to_health_and_consumer_queue() {
        let mut agent = RuntimeAgent::<2>::new(11, 3, 1_000);
        agent
            .health_mut()
            .set_capabilities(CAPABILITY_BTF | CAPABILITY_RINGBUF, 0b11, true);
        agent
            .observe_cycle(CycleContext {
                boot_id: 11,
                cycle_seq: 42,
                timestamp_ns: 1_000,
                deadline_miss: 1,
                ..CycleContext::EMPTY
            })
            .unwrap();

        let incident = agent
            .ingest(evidence(EvidenceKind::SchedulerRunqueueLatency, 1_100))
            .unwrap()
            .unwrap();
        let observation = agent.heartbeat(1_200);
        assert_eq!(incident.code, IncidentCode::HostSchedulerStall);
        assert_eq!(observation.state, ObservationState::Degraded);
        assert_eq!(observation.incident_count, 1);
        assert_eq!(
            agent.pop_incident().unwrap().incident_id,
            incident.incident_id
        );
        assert!(agent.pop_incident().is_none());
    }
}
