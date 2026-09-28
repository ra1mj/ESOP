mod support;

use std::time::Duration;

use esop_cli::{View, render};
use esop_ebpf_agent::{
    EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity, MAX_INCIDENT_EVIDENCE,
    RecommendedAction, RuntimeEvidence, RuntimeIncident,
};
use esop_procbuf::{
    CyclicQualityMask, DomainQuality, ProcBuf, QualityFact, RuntimeObservation, StatePage,
};
use esop_proto::CURRENT_SCHEMA_VERSION;
use esop_proto::v1::{QueryReply, QueryRequest};
use esop_zenoh_gateway::KeySpace;
use esop_zenoh_gateway::procbuf_adapter::ProcBufProjector;
use esop_zenoh_gateway::runtime::ZenohGateway;
use esop_zenoh_gateway::runtime_incident::project_runtime_incident;
use support::{Router, client_config};

fn runtime_incident() -> RuntimeIncident {
    let mut evidence = [RuntimeEvidence::EMPTY; MAX_INCIDENT_EVIDENCE];
    evidence[0] = RuntimeEvidence {
        evidence_id: 17,
        boot_id: 7,
        agent_epoch: 3,
        timestamp_ns: 2_500,
        cycle_seq: 11,
        transition_seq: 4,
        pid: 123,
        tid: 124,
        cpu: 5,
        irq: 0,
        netdev_ifindex: 0,
        observed_value: 40_000_000,
        threshold: 25_000_000,
        duration_ns: 40_000_000,
        count: 1,
        domain: EvidenceDomain::UserZenoh,
        kind: EvidenceKind::GatewayStall,
        severity: IncidentSeverity::Error,
        detail: 2,
    };
    RuntimeIncident {
        incident_id: 9,
        boot_id: 7,
        agent_epoch: 3,
        code: IncidentCode::GatewayStall,
        severity: IncidentSeverity::Error,
        recommended_action: RecommendedAction::ControlledStop,
        confidence_percent: 75,
        first_seen_ns: 2_400,
        last_seen_ns: 2_600,
        evidence_window_ns: 1_000,
        cycle_first: 11,
        cycle_last: 11,
        transition_seq: 4,
        pid: 123,
        tid: 124,
        cpu: 5,
        irq: 0,
        netdev_ifindex: 0,
        observed_value: 40_000_000,
        threshold: 25_000_000,
        count: 1,
        lost_events: 0,
        evidence_count: 1,
        reserved: [0; 3],
        evidence,
    }
}

fn projected_state(space: KeySpace) -> esop_proto::v1::RobotState {
    let buffer = ProcBuf::<1, 1, 1, 4>::new(42, 7);
    let mut state = StatePage::new(7);
    state.sequence = 11;
    state.monotonic_time_ns = 2_500;
    state.ecat_time_ns = 2_450;
    state.quality.sequence = state.sequence;
    state.quality.link_up = 1;
    state.quality.al_state = 8;
    state.quality.dc_locked = 1;
    state.quality.dc_offset_ns = 42;
    state.quality.cyclic = CyclicQualityMask {
        known_mask: QualityFact::ALL_MASK,
        good_mask: QualityFact::ALL_MASK,
    };
    state.quality.domains[0] = DomainQuality {
        expected_wkc: 6,
        actual_wkc: 6,
        valid: 1,
        complete: 1,
        consecutive_wkc_mismatches: 0,
        last_valid_cycle: 11,
        input_age_cycles: 0,
    };
    state.lifecycle.state = 2;
    state.lifecycle.stop_action = 0;
    state.lifecycle.gates_ready = 1;
    state.lifecycle.motion_permit = 1;
    state.lifecycle.required_gate_mask = 0x07ff;
    state.lifecycle.valid_gate_mask = 0x07ff;
    state.lifecycle.qualified_gate_mask = 0x07ff;
    state.lifecycle.ready_gate_mask = 0x07ff;
    state.lifecycle.permit_epoch = 3;
    state.lifecycle.permit_expires_at_ns = 10_000;
    state.lifecycle.transition_sequence = 4;
    state.lifecycle.transition_cycle = 10;
    state.runtime_observation = RuntimeObservation {
        latest_incident_id: 9,
        agent_epoch: 3,
        observed_at_ns: 2_500,
        observation_window_ns: 1_000,
        lost_events: 0,
        incident_count: 1,
        health: 0,
        reserved: [0; 7],
    };
    buffer.publish_state(state).expect("publish state");
    ProcBufProjector::new(space, 42, 7)
        .read_state(&buffer)
        .expect("project state")
        .expect("state is available")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "requires zenohd 1.10.1; run make test-zenoh"]
async fn query_and_render_projected_runtime_status() {
    let router = Router::new();
    let space = KeySpace::new(b"fleet_a", b"robot_01").expect("valid key space");
    let provider = ZenohGateway::open(space, client_config(&router.endpoint()))
        .await
        .expect("provider opens");
    let state = projected_state(space);
    let incident = project_runtime_incident(&runtime_incident()).expect("project incident");
    provider
        .serve_typed_queries(7, move |request| {
            Ok(QueryReply {
                robot_id: request.robot_id.clone(),
                boot_id: request.boot_id,
                schema_version: CURRENT_SCHEMA_VERSION,
                states: (state.sequence > request.after_sequence)
                    .then(|| state.clone())
                    .into_iter()
                    .collect(),
                incidents: vec![incident.clone()],
                ..QueryReply::default()
            })
        })
        .await
        .expect("query provider starts");
    tokio::time::sleep(Duration::from_millis(250)).await;

    let client = ZenohGateway::open(space, client_config(&router.endpoint()))
        .await
        .expect("client opens");
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        client.query_typed(&QueryRequest {
            robot_id: "robot_01".to_owned(),
            boot_id: 0,
            after_sequence: 0,
            limit: 2,
            schema_version: CURRENT_SCHEMA_VERSION,
        }),
    )
    .await
    .expect("query is bounded")
    .expect("query succeeds");

    let status = render(View::Status, &reply).expect("status renders");
    assert!(!status.healthy, "error incident degrades aggregate status");
    assert!(status.text.contains("dc=locked offset_ns=42"));
    assert!(
        status
            .text
            .contains("ebpf=healthy epoch=3 incidents=1 lost=0")
    );
    assert!(status.text.contains("incidents=1"));

    let incidents = render(View::IncidentList, &reply).expect("incidents render");
    assert!(!incidents.healthy);
    assert!(incidents.text.contains("severity=error"));
    assert!(incidents.text.contains("action=controlled_stop"));

    client.close().await.expect("client closes");
    provider.close().await.expect("provider closes");
}
