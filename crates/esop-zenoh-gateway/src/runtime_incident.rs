//! Host-only projection from bounded eBPF agent incidents to Protobuf v1.

use esop_ebpf_agent::{
    IncidentCode, IncidentSeverity as AgentSeverity, MAX_INCIDENT_EVIDENCE, RecommendedAction,
    RuntimeEvidence as AgentEvidence, RuntimeIncident as AgentIncident,
};
use esop_proto::CURRENT_SCHEMA_VERSION;
use esop_proto::v1::{
    IncidentSeverity as ProtoSeverity, RuntimeEvidence as ProtoEvidence,
    RuntimeIncident as ProtoIncident,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IncidentAdapterError {
    MissingIncidentId,
    MissingBootId,
    MissingAgentEpoch,
    InvalidConfidence,
    InvalidTimeWindow,
    InvalidCycleWindow,
    InvalidEvidenceCount,
    MissingEvidenceId,
    EvidenceBootMismatch,
    EvidenceEpochMismatch,
    EvidenceTimestampOutOfRange,
    EvidenceCycleOutOfRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IncidentContractError {
    MissingIncidentId,
    MissingBootId,
    BootMismatch,
    MissingAgentEpoch,
    InvalidConfidence,
    InvalidTimeWindow,
    InvalidCycleWindow,
    InvalidEvidenceCount,
    MissingEvidenceId,
    EvidenceBootMismatch,
    EvidenceEpochMismatch,
    EvidenceTimestampOutOfRange,
    EvidenceCycleOutOfRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowError {
    Time,
    Cycle,
}

pub fn project_runtime_incident(
    incident: &AgentIncident,
) -> Result<ProtoIncident, IncidentAdapterError> {
    validate_agent_incident(incident)?;
    let evidence = incident.evidence[..usize::from(incident.evidence_count)]
        .iter()
        .map(project_evidence)
        .collect();

    Ok(ProtoIncident {
        incident_id: format!(
            "esop-{:016x}-{:016x}-{:016x}",
            incident.boot_id, incident.agent_epoch, incident.incident_id
        ),
        severity: severity(incident.severity),
        reason_code: incident.code as u32,
        window_start_ns: incident.first_seen_ns,
        window_end_ns: incident.last_seen_ns,
        cycle_sequence: incident.cycle_last,
        transition_sequence: incident.transition_seq,
        evidence,
        lost_event_count: incident.lost_events,
        affected_component: affected_component(incident.code).into(),
        suggested_action: suggested_action(incident.recommended_action).into(),
        schema_version: CURRENT_SCHEMA_VERSION,
        boot_id: incident.boot_id,
        agent_epoch: incident.agent_epoch,
        confidence_percent: u32::from(incident.confidence_percent),
        cycle_first: incident.cycle_first,
        cycle_last: incident.cycle_last,
        evidence_window_ns: incident.evidence_window_ns,
        pid: u64::from(incident.pid),
        tid: u64::from(incident.tid),
        cpu: u32::from(incident.cpu),
        irq: u32::from(incident.irq),
        netdev_ifindex: incident.netdev_ifindex,
        observed_value: incident.observed_value,
        threshold: incident.threshold,
        event_count: incident.count,
    })
}

pub fn validate_runtime_incident_message(
    incident: &ProtoIncident,
    expected_boot_id: u64,
) -> Result<(), IncidentContractError> {
    if incident.incident_id.is_empty() {
        return Err(IncidentContractError::MissingIncidentId);
    }
    if incident.boot_id == 0 {
        return Err(IncidentContractError::MissingBootId);
    }
    if incident.boot_id != expected_boot_id {
        return Err(IncidentContractError::BootMismatch);
    }
    if incident.agent_epoch == 0 {
        return Err(IncidentContractError::MissingAgentEpoch);
    }
    if incident.confidence_percent > 100 {
        return Err(IncidentContractError::InvalidConfidence);
    }
    validate_windows(
        incident.window_start_ns,
        incident.window_end_ns,
        incident.cycle_first,
        incident.cycle_last,
    )
    .map_err(|error| match error {
        WindowError::Time => IncidentContractError::InvalidTimeWindow,
        WindowError::Cycle => IncidentContractError::InvalidCycleWindow,
    })?;
    if incident.evidence.is_empty() || incident.evidence.len() > MAX_INCIDENT_EVIDENCE {
        return Err(IncidentContractError::InvalidEvidenceCount);
    }
    for evidence in &incident.evidence {
        if evidence.evidence_id == 0 {
            return Err(IncidentContractError::MissingEvidenceId);
        }
        if evidence.boot_id != incident.boot_id {
            return Err(IncidentContractError::EvidenceBootMismatch);
        }
        if evidence.agent_epoch != incident.agent_epoch {
            return Err(IncidentContractError::EvidenceEpochMismatch);
        }
        if evidence.timestamp_ns < incident.window_start_ns
            || evidence.timestamp_ns > incident.window_end_ns
        {
            return Err(IncidentContractError::EvidenceTimestampOutOfRange);
        }
        if incident.cycle_first != 0
            && (evidence.cycle_sequence < incident.cycle_first
                || evidence.cycle_sequence > incident.cycle_last)
        {
            return Err(IncidentContractError::EvidenceCycleOutOfRange);
        }
    }
    Ok(())
}

fn validate_agent_incident(incident: &AgentIncident) -> Result<(), IncidentAdapterError> {
    if incident.incident_id == 0 {
        return Err(IncidentAdapterError::MissingIncidentId);
    }
    if incident.boot_id == 0 {
        return Err(IncidentAdapterError::MissingBootId);
    }
    if incident.agent_epoch == 0 {
        return Err(IncidentAdapterError::MissingAgentEpoch);
    }
    if incident.confidence_percent > 100 {
        return Err(IncidentAdapterError::InvalidConfidence);
    }
    validate_windows(
        incident.first_seen_ns,
        incident.last_seen_ns,
        incident.cycle_first,
        incident.cycle_last,
    )
    .map_err(|error| match error {
        WindowError::Time => IncidentAdapterError::InvalidTimeWindow,
        WindowError::Cycle => IncidentAdapterError::InvalidCycleWindow,
    })?;
    if incident.evidence_count == 0 || usize::from(incident.evidence_count) > MAX_INCIDENT_EVIDENCE
    {
        return Err(IncidentAdapterError::InvalidEvidenceCount);
    }
    for evidence in &incident.evidence[..usize::from(incident.evidence_count)] {
        validate_agent_evidence(incident, evidence)?;
    }
    Ok(())
}

fn validate_windows(
    first_seen_ns: u64,
    last_seen_ns: u64,
    cycle_first: u64,
    cycle_last: u64,
) -> Result<(), WindowError> {
    if first_seen_ns > last_seen_ns {
        return Err(WindowError::Time);
    }
    if (cycle_first == 0) != (cycle_last == 0) || (cycle_first != 0 && cycle_first > cycle_last) {
        return Err(WindowError::Cycle);
    }
    Ok(())
}

fn validate_agent_evidence(
    incident: &AgentIncident,
    evidence: &AgentEvidence,
) -> Result<(), IncidentAdapterError> {
    if evidence.evidence_id == 0 {
        return Err(IncidentAdapterError::MissingEvidenceId);
    }
    if evidence.boot_id != incident.boot_id {
        return Err(IncidentAdapterError::EvidenceBootMismatch);
    }
    if evidence.agent_epoch != incident.agent_epoch {
        return Err(IncidentAdapterError::EvidenceEpochMismatch);
    }
    if evidence.timestamp_ns < incident.first_seen_ns
        || evidence.timestamp_ns > incident.last_seen_ns
    {
        return Err(IncidentAdapterError::EvidenceTimestampOutOfRange);
    }
    if incident.cycle_first != 0
        && (evidence.cycle_seq < incident.cycle_first || evidence.cycle_seq > incident.cycle_last)
    {
        return Err(IncidentAdapterError::EvidenceCycleOutOfRange);
    }
    Ok(())
}

fn project_evidence(evidence: &AgentEvidence) -> ProtoEvidence {
    ProtoEvidence {
        kind: evidence.kind as u32,
        timestamp_ns: evidence.timestamp_ns,
        pid: u64::from(evidence.pid),
        tid: u64::from(evidence.tid),
        cpu: u32::from(evidence.cpu),
        attach_point: 0,
        value: evidence.observed_value.min(u64::from(u32::MAX)) as u32,
        cycle_sequence: evidence.cycle_seq,
        boot_id: evidence.boot_id,
        evidence_id: evidence.evidence_id,
        agent_epoch: evidence.agent_epoch,
        transition_sequence: evidence.transition_seq,
        domain: evidence.domain as u32,
        severity: severity(evidence.severity),
        irq: u32::from(evidence.irq),
        netdev_ifindex: evidence.netdev_ifindex,
        observed_value: evidence.observed_value,
        threshold: evidence.threshold,
        duration_ns: evidence.duration_ns,
        count: evidence.count,
        detail: u32::from(evidence.detail),
    }
}

const fn severity(value: AgentSeverity) -> i32 {
    match value {
        AgentSeverity::Info => ProtoSeverity::Info as i32,
        AgentSeverity::Warning => ProtoSeverity::Warning as i32,
        AgentSeverity::Error => ProtoSeverity::Error as i32,
        AgentSeverity::Critical => ProtoSeverity::Critical as i32,
    }
}

const fn suggested_action(value: RecommendedAction) -> &'static str {
    match value {
        RecommendedAction::ContinueObserve => "continue_observe",
        RecommendedAction::DegradeHostObservation => "degrade_host_observation",
        RecommendedAction::ControlledStop => "controlled_stop",
        RecommendedAction::LatchFault => "latch_fault",
    }
}

const fn affected_component(value: IncidentCode) -> &'static str {
    match value {
        IncidentCode::HostSchedulerStall => "host.scheduler",
        IncidentCode::HostIrqStorm => "host.irq",
        IncidentCode::HostNicDrop => "host.network",
        IncidentCode::HostPageFault | IncidentCode::HostOom => "host.memory",
        IncidentCode::UserComponentExit => "user.component",
        IncidentCode::HostCpuThrottle => "host.cpu",
        IncidentCode::GatewayStall => "user.zenoh",
        IncidentCode::ObservabilityDegraded => "observer.agent",
        IncidentCode::HostPortStall => "user.esop.raw_port",
    }
}
