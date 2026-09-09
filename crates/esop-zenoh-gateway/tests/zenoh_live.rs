#![cfg(feature = "zenoh")]

mod support;

use std::sync::mpsc;
use std::time::Duration;

use esop_command_gateway::{CommandIngress, IngressPolicy};
use esop_lifecycle_guard::{GateId, GuardPolicy, LifecycleAction, LifecycleGuard, StopAction};
use esop_proto::Message;
use esop_proto::v1::{DiagnosticEvent, MotionCommand, RobotState, RuntimeIncident};
use esop_zenoh_gateway::runtime::{ConnectionState, ZenohGateway, decode_command_payload};
use esop_zenoh_gateway::{KeySpace, RouteKind};
use support::{Router, client_config};
use zenoh::Wait;

fn route_key(space: KeySpace, kind: RouteKind) -> String {
    let mut buffer = [0; esop_zenoh_gateway::MAX_ZENOH_KEY_BYTES];
    let length = space.write_key(kind, &mut buffer).expect("route key fits");
    String::from_utf8(buffer[..length].to_vec()).expect("route key is UTF-8")
}

fn command_payload() -> Vec<u8> {
    command_payload_with(1, 10_000)
}

fn command_payload_with(sequence: u64, deadline_ns: u64) -> Vec<u8> {
    command_payload_with_epoch(sequence, deadline_ns, 1)
}

fn command_payload_with_epoch(sequence: u64, deadline_ns: u64, permit_epoch: u64) -> Vec<u8> {
    MotionCommand {
        robot_id: "robot_01".to_owned(),
        boot_id: 7,
        source_id: 42,
        permit_epoch,
        sequence,
        deadline_ns,
        axis_mask: 0x03,
        authority: 2,
        policy_version: 9,
        ..MotionCommand::default()
    }
    .encode_to_vec()
}

fn wait_for_command(
    observer: &zenoh::Session,
    key: &str,
    payload: &[u8],
    receiver: &mpsc::Receiver<Vec<u8>>,
) -> Vec<u8> {
    for _ in 0..20 {
        observer
            .put(key, payload.to_vec())
            .wait()
            .expect("command publishes through router");
        if let Ok(received) = receiver.recv_timeout(Duration::from_millis(500)) {
            return received;
        }
    }
    panic!("command did not reach gateway");
}

fn ingress() -> CommandIngress {
    CommandIngress::new(
        7,
        IngressPolicy {
            authorized_sources: [42, 0, 0, 0],
            authorized_source_count: 1,
            minimum_authority: 2,
            reserved: [0; 2],
            permit_policy_version: 9,
            allowed_axis_mask: 0x03,
            max_ttl_ns: 100_000,
            rate_window_ns: 1_000,
            max_commands_per_window: 2,
            reserved_tail: [0; 6],
        },
    )
}

#[test]
#[ignore = "requires zenohd 1.10.1; run make test-zenoh"]
fn router_round_trip_covers_gateway_contracts() {
    tokio::runtime::Runtime::new()
        .expect("Tokio runtime starts")
        .block_on(async {
            let router = Router::new();
            let space = KeySpace::new(b"fleet_a", b"robot_01").expect("valid key space");
            let gateway = ZenohGateway::open(space, client_config(&router.endpoint()))
                .await
                .expect("gateway session opens");
            let observer = zenoh::open(client_config(&router.endpoint()))
                .await
                .expect("observer session opens");

            let (state_tx, state_rx) = mpsc::channel();
            observer
                .declare_subscriber(route_key(space, RouteKind::State))
                .callback(move |sample| {
                    let _ = state_tx.send((
                        sample.payload().to_bytes().into_owned(),
                        sample.priority(),
                        sample.congestion_control(),
                        sample.express(),
                    ));
                })
                .background()
                .await
                .expect("state subscriber declares");
            let mut state_received = None;
            for _ in 0..10 {
                gateway
                    .publish_state(&RobotState {
                        robot_id: "robot_01".to_owned(),
                        boot_id: 7,
                        sequence: 1,
                        ..RobotState::default()
                    })
                    .await
                    .expect("state publishes through router");
                if let Ok(payload) = state_rx.recv_timeout(Duration::from_millis(500)) {
                    state_received = Some(payload);
                    break;
                }
            }
            let (state_bytes, state_priority, state_congestion, state_express) =
                state_received.expect("state reaches observer");
            let state_payload =
                RobotState::decode(state_bytes.as_slice()).expect("state payload decodes");
            assert_eq!(state_payload.robot_id, "robot_01");
            assert_eq!(state_payload.sequence, 1);
            assert_eq!(state_priority, zenoh::qos::Priority::Data);
            assert_eq!(state_congestion, zenoh::qos::CongestionControl::Drop);
            assert!(!state_express);

            let (event_tx, event_rx) = mpsc::channel();
            observer
                .declare_subscriber(route_key(space, RouteKind::Event))
                .callback(move |sample| {
                    let _ = event_tx.send(sample.payload().to_bytes().into_owned());
                })
                .background()
                .await
                .expect("event subscriber declares");
            let (incident_tx, incident_rx) = mpsc::channel();
            observer
                .declare_subscriber(route_key(space, RouteKind::Diagnostic))
                .callback(move |sample| {
                    let _ = incident_tx.send(sample.payload().to_bytes().into_owned());
                })
                .background()
                .await
                .expect("incident subscriber declares");
            std::thread::sleep(Duration::from_millis(250));
            gateway
                .publish_event(&DiagnosticEvent {
                    sequence: 2,
                    timestamp_ns: 2_000,
                    code: 0x1001,
                    ..DiagnosticEvent::default()
                })
                .await
                .expect("event publishes through router");
            gateway
                .publish_incident(&RuntimeIncident {
                    incident_id: "inc-1".to_owned(),
                    reason_code: 0x2001,
                    ..RuntimeIncident::default()
                })
                .await
                .expect("incident publishes through router");
            assert_eq!(
                DiagnosticEvent::decode(
                    event_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("event reaches observer")
                        .as_slice()
                )
                .expect("event payload decodes")
                .code,
                0x1001
            );
            assert_eq!(
                RuntimeIncident::decode(
                    incident_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("incident reaches observer")
                        .as_slice()
                )
                .expect("incident payload decodes")
                .incident_id,
                "inc-1"
            );

            let (command_tx, command_rx) = mpsc::channel();
            gateway
                .subscribe_commands(move |sample| {
                    let _ = command_tx.send(sample.payload().to_bytes().into_owned());
                })
                .await
                .expect("command subscriber declares");
            std::thread::sleep(Duration::from_millis(250));
            let command = command_payload();
            let payload = wait_for_command(
                &observer,
                &route_key(space, RouteKind::Command),
                &command,
                &command_rx,
            );
            let mut ingress = ingress();
            let permit = gateway
                .admit_authenticated_command(&mut ingress, &payload, 42, 1_000)
                .expect("received command passes fixed ingress");
            assert_eq!(permit.source_id, 42);
            assert_eq!(ingress.audit_count(), 1);
            assert_eq!(
                decode_command_payload(space, &payload)
                    .expect("command decodes")
                    .sequence,
                1
            );

            gateway
                .serve_queries(|query| {
                    query
                        .reply(query.key_expr().clone(), "query-reply")
                        .wait()
                        .expect("query reply sends");
                })
                .await
                .expect("queryable declares");
            std::thread::sleep(Duration::from_millis(250));
            let replies = observer
                .get(route_key(space, RouteKind::Query))
                .payload("query-request")
                .await
                .expect("query dispatches through router");
            let reply = replies
                .recv_async()
                .await
                .expect("query reply arrives")
                .into_result()
                .expect("query reply is data");
            assert_eq!(reply.payload().to_bytes().as_ref(), b"query-reply");

            gateway.close().await.expect("gateway closes");
            assert_eq!(gateway.health().state(), ConnectionState::Disconnected);
            assert!(matches!(
                gateway
                    .publish_event(&DiagnosticEvent {
                        sequence: 3,
                        ..DiagnosticEvent::default()
                    })
                    .await,
                Err(esop_zenoh_gateway::runtime::RuntimeError::Zenoh(_))
            ));
            assert_eq!(gateway.health().state(), ConnectionState::Disconnected);
            observer.close().await.expect("observer closes");
        });
}

#[test]
#[ignore = "requires zenohd 1.10.1; run make test-zenoh"]
fn router_restart_is_observable_and_cannot_rearm_motion() {
    tokio::runtime::Runtime::new()
        .expect("Tokio runtime starts")
        .block_on(async {
            let mut router = Router::new();
            let space = KeySpace::new(b"fleet_a", b"robot_01").expect("valid key space");
            let gateway = ZenohGateway::open(space, client_config(&router.endpoint()))
                .await
                .expect("gateway session opens");
            let observer = zenoh::open(client_config(&router.endpoint()))
                .await
                .expect("observer session opens");
            let (command_tx, command_rx) = mpsc::channel();
            gateway
                .subscribe_commands(move |sample| {
                    let _ = command_tx.send(sample.payload().to_bytes().into_owned());
                })
                .await
                .expect("command subscriber declares");
            std::thread::sleep(Duration::from_millis(250));

            let command_key = route_key(space, RouteKind::Command);
            let old_command = command_payload();
            let received = wait_for_command(&observer, &command_key, &old_command, &command_rx);
            let mut ingress = ingress();
            let permit = gateway
                .admit_authenticated_command(&mut ingress, &received, 42, 1_000)
                .expect("initial command is admitted");

            let mut guard = LifecycleGuard::new(
                GateId::Link.bit(),
                7,
                GuardPolicy {
                    enter_good_cycles: 1,
                    exit_bad_cycles: 1,
                    max_age_cycles: 1,
                    stop_action: StopAction::QuickStop,
                    authorized_source_id: 42,
                    minimum_authority: 2,
                    permit_policy_version: 9,
                },
            );
            guard.update_gate(GateId::Link, true, 1, 0);
            assert_eq!(
                guard.request_rearm(permit, 1, 1_000),
                Ok(LifecycleAction::EnableAllowed)
            );

            router.stop();
            let disconnected = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if gateway.refresh_health().await == ConnectionState::Disconnected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await;
            assert!(disconnected.is_ok(), "router loss becomes observable");
            assert_eq!(
                guard.cycle(2, 10_001),
                LifecycleAction::Stop(StopAction::QuickStop)
            );

            router.start();
            let connected = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if gateway.refresh_health().await == ConnectionState::Connected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await;
            assert!(connected.is_ok(), "router recovery becomes observable");

            let replayed = wait_for_command(&observer, &command_key, &old_command, &command_rx);
            assert!(matches!(
                gateway.admit_authenticated_command(&mut ingress, &replayed, 42, 20_000),
                Err(esop_zenoh_gateway::runtime::CommandAdapterError::Policy(
                    esop_command_gateway::IngressError::DeadlineExpired
                ))
            ));
            assert_eq!(
                guard.cycle(3, 20_001),
                LifecycleAction::Stop(StopAction::QuickStop)
            );

            let renewed = command_payload_with_epoch(2, 30_000, 2);
            let renewed_payload = wait_for_command(&observer, &command_key, &renewed, &command_rx);
            let renewed_permit = gateway
                .admit_authenticated_command(&mut ingress, &renewed_payload, 42, 20_000)
                .expect("new command is admitted after recovery");
            assert_eq!(
                guard.cycle(4, 20_001),
                LifecycleAction::Stop(StopAction::QuickStop)
            );
            guard.acknowledge_stopped(4).expect("stop is acknowledged");
            guard.update_gate(GateId::Link, true, 5, 0);
            assert_eq!(
                guard.request_rearm(renewed_permit, 5, 20_000),
                Ok(LifecycleAction::EnableAllowed)
            );
            gateway.close().await.expect("gateway closes");
            observer.close().await.expect("observer closes");
        });
}
