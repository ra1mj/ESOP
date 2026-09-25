#![cfg(feature = "zenoh")]

use esop_ebpf_agent::{
    EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity, MAX_INCIDENT_EVIDENCE,
    RecommendedAction, RuntimeEvidence, RuntimeIncident,
};
use esop_proto::CURRENT_SCHEMA_VERSION;
use esop_proto::v1::IncidentSeverity as ProtoSeverity;
use esop_zenoh_gateway::runtime_incident::{
    IncidentAdapterError, IncidentContractError, project_runtime_incident,
    validate_runtime_incident_message,
};

fn evidence(id: u64, timestamp_ns: u64, cycle_seq: u64) -> RuntimeEvidence {
    RuntimeEvidence {
        evidence_id: id,
        boot_id: 0x11,
        agent_epoch: 0x22,
        timestamp_ns,
        cycle_seq,
        transition_seq: 91,
        pid: 123,
        tid: 124,
        cpu: 5,
        irq: 3,
        netdev_ifindex: 7,
        observed_value: u64::from(u32::MAX) + id,
        threshold: 42,
        duration_ns: 1_500,
        count: 4,
        domain: EvidenceDomain::KernelNetwork,
        kind: EvidenceKind::NetworkDrop,
        severity: IncidentSeverity::Error,
        detail: 9,
    }
}

fn incident() -> RuntimeIncident {
    let mut evidence_records = [RuntimeEvidence::EMPTY; MAX_INCIDENT_EVIDENCE];
    evidence_records[0] = evidence(1, 1_100, 40);
    evidence_records[1] = evidence(2, 1_200, 41);
    RuntimeIncident {
        incident_id: 0x33,
        boot_id: 0x11,
        agent_epoch: 0x22,
        code: IncidentCode::HostNicDrop,
        severity: IncidentSeverity::Error,
        recommended_action: RecommendedAction::ControlledStop,
        confidence_percent: 75,
        first_seen_ns: 1_000,
        last_seen_ns: 1_300,
        evidence_window_ns: 500,
        cycle_first: 40,
        cycle_last: 41,
        transition_seq: 91,
        pid: 123,
        tid: 124,
        cpu: 5,
        irq: 3,
        netdev_ifindex: 7,
        observed_value: u64::from(u32::MAX) + 99,
        threshold: 42,
        count: 8,
        lost_events: 2,
        evidence_count: 2,
        reserved: [0; 3],
        evidence: evidence_records,
    }
}

#[test]
fn projection_preserves_identity_provenance_full_width_values_and_legacy_fields() {
    let projected = project_runtime_incident(&incident()).unwrap();
    assert_eq!(
        projected.incident_id,
        "esop-0000000000000011-0000000000000022-0000000000000033"
    );
    assert_eq!(projected.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(projected.severity, ProtoSeverity::Error as i32);
    assert_eq!(projected.reason_code, IncidentCode::HostNicDrop as u32);
    assert_eq!(projected.window_start_ns, 1_000);
    assert_eq!(projected.window_end_ns, 1_300);
    assert_eq!(projected.cycle_sequence, 41);
    assert_eq!(projected.transition_sequence, 91);
    assert_eq!(projected.lost_event_count, 2);
    assert_eq!(projected.affected_component, "host.network");
    assert_eq!(projected.suggested_action, "controlled_stop");
    assert_eq!(projected.boot_id, 0x11);
    assert_eq!(projected.agent_epoch, 0x22);
    assert_eq!(projected.confidence_percent, 75);
    assert_eq!(projected.cycle_first, 40);
    assert_eq!(projected.cycle_last, 41);
    assert_eq!(projected.evidence_window_ns, 500);
    assert_eq!(projected.pid, 123);
    assert_eq!(projected.tid, 124);
    assert_eq!(projected.cpu, 5);
    assert_eq!(projected.irq, 3);
    assert_eq!(projected.netdev_ifindex, 7);
    assert_eq!(projected.observed_value, u64::from(u32::MAX) + 99);
    assert_eq!(projected.threshold, 42);
    assert_eq!(projected.event_count, 8);
    assert_eq!(projected.evidence.len(), 2);

    let first = &projected.evidence[0];
    assert_eq!(first.value, u32::MAX);
    assert_eq!(first.observed_value, u64::from(u32::MAX) + 1);
    assert_eq!(first.evidence_id, 1);
    assert_eq!(first.agent_epoch, 0x22);
    assert_eq!(first.transition_sequence, 91);
    assert_eq!(first.domain, EvidenceDomain::KernelNetwork as u32);
    assert_eq!(first.severity, ProtoSeverity::Error as i32);
    assert_eq!(first.irq, 3);
    assert_eq!(first.netdev_ifindex, 7);
    assert_eq!(first.threshold, 42);
    assert_eq!(first.duration_ns, 1_500);
    assert_eq!(first.count, 4);
    assert_eq!(first.detail, 9);
    assert_eq!(first.attach_point, 0);
    validate_runtime_incident_message(&projected, 0x11).unwrap();
}

#[test]
fn incident_id_is_stable_and_disambiguates_boot_epoch_and_local_id() {
    let source = incident();
    let first = project_runtime_incident(&source).unwrap().incident_id;
    assert_eq!(
        project_runtime_incident(&source).unwrap().incident_id,
        first
    );

    let mut changed = source;
    changed.incident_id += 1;
    assert_ne!(
        project_runtime_incident(&changed).unwrap().incident_id,
        first
    );

    changed = source;
    changed.boot_id += 1;
    for evidence in &mut changed.evidence[..usize::from(changed.evidence_count)] {
        evidence.boot_id = changed.boot_id;
    }
    assert_ne!(
        project_runtime_incident(&changed).unwrap().incident_id,
        first
    );

    changed = source;
    changed.agent_epoch += 1;
    for evidence in &mut changed.evidence[..usize::from(changed.evidence_count)] {
        evidence.agent_epoch = changed.agent_epoch;
    }
    assert_ne!(
        project_runtime_incident(&changed).unwrap().incident_id,
        first
    );
}

#[test]
fn stable_enum_mappings_cover_every_incident_code_action_and_severity() {
    for (code, component) in [
        (IncidentCode::HostSchedulerStall, "host.scheduler"),
        (IncidentCode::HostIrqStorm, "host.irq"),
        (IncidentCode::HostNicDrop, "host.network"),
        (IncidentCode::HostPageFault, "host.memory"),
        (IncidentCode::HostOom, "host.memory"),
        (IncidentCode::UserComponentExit, "user.component"),
        (IncidentCode::HostCpuThrottle, "host.cpu"),
        (IncidentCode::GatewayStall, "user.zenoh"),
        (IncidentCode::ObservabilityDegraded, "observer.agent"),
        (IncidentCode::HostPortStall, "user.esop.raw_port"),
    ] {
        let mut source = incident();
        source.code = code;
        assert_eq!(
            project_runtime_incident(&source)
                .unwrap()
                .affected_component,
            component
        );
    }

    for (action, name) in [
        (RecommendedAction::ContinueObserve, "continue_observe"),
        (
            RecommendedAction::DegradeHostObservation,
            "degrade_host_observation",
        ),
        (RecommendedAction::ControlledStop, "controlled_stop"),
        (RecommendedAction::LatchFault, "latch_fault"),
    ] {
        let mut source = incident();
        source.recommended_action = action;
        assert_eq!(
            project_runtime_incident(&source).unwrap().suggested_action,
            name
        );
    }

    for (severity, expected) in [
        (IncidentSeverity::Info, ProtoSeverity::Info),
        (IncidentSeverity::Warning, ProtoSeverity::Warning),
        (IncidentSeverity::Error, ProtoSeverity::Error),
        (IncidentSeverity::Critical, ProtoSeverity::Critical),
    ] {
        let mut source = incident();
        source.severity = severity;
        assert_eq!(
            project_runtime_incident(&source).unwrap().severity,
            expected as i32
        );
    }
}

#[test]
fn malformed_agent_incidents_are_rejected_before_projection() {
    let mut value = incident();
    value.incident_id = 0;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::MissingIncidentId)
    );

    value = incident();
    value.boot_id = 0;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::MissingBootId)
    );

    value = incident();
    value.agent_epoch = 0;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::MissingAgentEpoch)
    );

    value = incident();
    value.confidence_percent = 101;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::InvalidConfidence)
    );

    value = incident();
    value.first_seen_ns = value.last_seen_ns + 1;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::InvalidTimeWindow)
    );

    value = incident();
    value.cycle_first = value.cycle_last + 1;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::InvalidCycleWindow)
    );

    value = incident();
    value.evidence_count = 0;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::InvalidEvidenceCount)
    );

    value = incident();
    value.evidence_count = MAX_INCIDENT_EVIDENCE as u8 + 1;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::InvalidEvidenceCount)
    );

    value = incident();
    value.evidence[0].evidence_id = 0;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::MissingEvidenceId)
    );

    value = incident();
    value.evidence[0].boot_id += 1;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::EvidenceBootMismatch)
    );

    value = incident();
    value.evidence[0].agent_epoch += 1;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::EvidenceEpochMismatch)
    );

    value = incident();
    value.evidence[0].timestamp_ns = value.last_seen_ns + 1;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::EvidenceTimestampOutOfRange)
    );

    value = incident();
    value.evidence[0].cycle_seq = 0;
    assert_eq!(
        project_runtime_incident(&value),
        Err(IncidentAdapterError::EvidenceCycleOutOfRange)
    );
}

#[test]
fn malformed_protobuf_incidents_are_rejected_before_query_encoding() {
    let projected = project_runtime_incident(&incident()).unwrap();

    let mut value = projected.clone();
    value.incident_id.clear();
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::MissingIncidentId)
    );

    value = projected.clone();
    value.boot_id = 0;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::MissingBootId)
    );

    assert_eq!(
        validate_runtime_incident_message(&projected, 0x12),
        Err(IncidentContractError::BootMismatch)
    );

    value = projected.clone();
    value.agent_epoch = 0;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::MissingAgentEpoch)
    );

    value = projected.clone();
    value.confidence_percent = 101;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::InvalidConfidence)
    );

    value = projected.clone();
    value.window_start_ns = value.window_end_ns + 1;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::InvalidTimeWindow)
    );

    value = projected.clone();
    value.cycle_first = value.cycle_last + 1;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::InvalidCycleWindow)
    );

    value = projected.clone();
    value.evidence.clear();
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::InvalidEvidenceCount)
    );

    value = projected.clone();
    while value.evidence.len() <= MAX_INCIDENT_EVIDENCE {
        value.evidence.push(value.evidence[0]);
    }
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::InvalidEvidenceCount)
    );

    value = projected.clone();
    value.evidence[0].evidence_id = 0;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::MissingEvidenceId)
    );

    value = projected.clone();
    value.evidence[0].boot_id += 1;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::EvidenceBootMismatch)
    );

    value = projected.clone();
    value.evidence[0].agent_epoch += 1;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::EvidenceEpochMismatch)
    );

    value = projected.clone();
    value.evidence[0].timestamp_ns = value.window_end_ns + 1;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::EvidenceTimestampOutOfRange)
    );

    value = projected;
    value.evidence[0].cycle_sequence = 0;
    assert_eq!(
        validate_runtime_incident_message(&value, 0x11),
        Err(IncidentContractError::EvidenceCycleOutOfRange)
    );
}
