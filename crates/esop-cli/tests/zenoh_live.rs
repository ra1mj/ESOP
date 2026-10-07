#[path = "support/command.rs"]
mod command;
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
use esop_proto::v1::{QueryReply, QueryRequest};
use esop_proto::{CURRENT_SCHEMA_VERSION, Message};
use esop_zenoh_gateway::KeySpace;
use esop_zenoh_gateway::procbuf_adapter::ProcBufProjector;
use esop_zenoh_gateway::runtime::ZenohGateway;
use esop_zenoh_gateway::runtime_incident::project_runtime_incident;
use support::{Router, client_config};
use zenoh::Wait;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "requires zenohd 1.10.1; run make test-zenoh"]
async fn actual_binary_runs_all_views_and_finite_watch() {
    let router = Router::new();
    let space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
    let config = client_config(&router.endpoint());
    let provider = ZenohGateway::open(space, config.clone()).await.unwrap();
    let state = projected_state(space);
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
                ..QueryReply::default()
            })
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    for (view, expected) in [
        (vec!["status"], "health=ok"),
        (vec!["domain", "list"], "0 6 6 yes yes"),
        (vec!["dc"], "dc=locked offset_ns=42"),
        (vec!["lifecycle"], "permit=yes"),
        (vec!["incident", "list"], "incidents=none"),
        (vec!["doctor"], "healthy"),
    ] {
        let output = command::run(&config.to_string(), &view).await;
        let text = command::assert_exit(&output, 0);
        assert!(text.contains(expected), "{view:?}: {text}");
        let mut watch = vec!["watch"];
        watch.extend_from_slice(&view);
        watch.extend_from_slice(&["--interval-ms", "50", "--iterations", "2"]);
        let output = command::run(&config.to_string(), &watch).await;
        let text = command::assert_exit(&output, 0);
        assert!(text.contains(expected), "{watch:?}: {text}");
    }
    command::assert_exit(
        &command::run(&config.to_string(), &["--boot-id", "8", "status"]).await,
        6,
    );
    command::assert_exit(
        &command::run_with_stdout(&config.to_string(), &["status"], true).await,
        0,
    );
    provider.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "requires zenohd 1.10.1; run make test-zenoh"]
async fn actual_binary_rejects_bad_replies_and_bounds_slow_queries() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let router = Router::new();
    let space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
    let config = client_config(&router.endpoint());
    let provider = ZenohGateway::open(space, config.clone()).await.unwrap();
    let state = projected_state(space);
    let mode = Arc::new(AtomicUsize::new(0));
    let held_queries = Arc::new(std::sync::Mutex::new(Vec::new()));
    let selected = mode.clone();
    let held = held_queries.clone();
    provider
        .serve_queries(move |query| {
            let mode = selected.load(Ordering::SeqCst);
            if mode == 9 {
                held.lock().unwrap().push(query);
                return;
            }
            let mut reply = QueryReply {
                robot_id: "robot_01".to_owned(),
                boot_id: 7,
                schema_version: CURRENT_SCHEMA_VERSION,
                states: vec![state.clone()],
                ..QueryReply::default()
            };
            if mode == 10 {
                let payload = query.payload().unwrap().to_bytes();
                let request = QueryRequest::decode(payload.as_ref()).unwrap();
                if request.after_sequence != 0 {
                    reply.boot_id = 8;
                    reply.states[0].boot_id = 8;
                    reply.states[0].sequence += 1;
                }
            }
            match mode {
                0 => reply.states[0].operational.as_mut().unwrap().link_up = false,
                2 => reply.schema_version = 999,
                3 => reply.robot_id = "other_robot".to_owned(),
                4 => reply.boot_id = 8,
                5 => reply.states[0].operational.as_mut().unwrap().domains[0].domain = 1,
                6 => reply.states[0].lifecycle.as_mut().unwrap().state = 999,
                7 => reply.states.clear(),
                8 => reply.states[0].sequence = 0,
                _ => {}
            }
            let bytes = if mode == 1 {
                vec![0xff]
            } else {
                reply.encode_to_vec()
            };
            query.reply(query.key_expr().clone(), bytes).wait().unwrap();
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let output = command::run(&config.to_string(), &["doctor"]).await;
    assert!(command::assert_exit(&output, 3).contains("result=degraded"));
    for (scenario, exit) in [
        (1, 6),
        (2, 6),
        (3, 6),
        (4, 6),
        (5, 7),
        (6, 7),
        (7, 7),
        (8, 6),
        (9, 5),
    ] {
        mode.store(scenario, Ordering::SeqCst);
        let output = command::run(
            &config.to_string(),
            &["--boot-id", "7", "--timeout-ms", "300", "status"],
        )
        .await;
        command::assert_exit(&output, exit);
    }
    held_queries.lock().unwrap().clear();
    for scenario in [10, 11] {
        mode.store(scenario, Ordering::SeqCst);
        // A provider reboot or repeated State must fail the already-bound
        // watch rather than silently following a new boot or stale sequence.
        command::assert_exit(
            &command::run(
                &config.to_string(),
                &["watch", "--interval-ms", "50", "--iterations", "2"],
            )
            .await,
            6,
        );
    }
    provider.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "requires zenohd 1.10.1; run make test-zenoh"]
async fn watch_surfaces_incidents_without_a_new_state() {
    let router = Router::new();
    let space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
    let config = client_config(&router.endpoint());
    let provider = ZenohGateway::open(space, config.clone()).await.unwrap();
    let state = projected_state(space);
    let incident = project_runtime_incident(&runtime_incident()).unwrap();
    provider
        .serve_typed_queries(7, move |request| {
            let initial = request.after_sequence == 0;
            Ok(QueryReply {
                robot_id: request.robot_id.clone(),
                boot_id: request.boot_id,
                schema_version: CURRENT_SCHEMA_VERSION,
                states: initial.then(|| state.clone()).into_iter().collect(),
                incidents: (!initial).then(|| incident.clone()).into_iter().collect(),
                ..QueryReply::default()
            })
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    for view in ["doctor", "status"] {
        let output = command::run(
            &config.to_string(),
            &["watch", view, "--interval-ms", "50", "--iterations", "2"],
        )
        .await;
        let text = command::assert_exit(&output, if view == "doctor" { 3 } else { 0 });
        assert!(
            text.contains("severity=error"),
            "new incident was hidden: {text}"
        );
        if view == "doctor" {
            assert!(text.ends_with("result=degraded\n"), "{text}");
        }
    }
    provider.close().await.unwrap();
}
