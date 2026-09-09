#![cfg(feature = "zenoh")]

use std::sync::mpsc;
use std::time::Duration;

use esop_command_gateway::{CommandIngress, IngressPolicy};
use esop_proto::Message;
use esop_proto::v1::{DiagnosticEvent, MotionCommand, RobotState, RuntimeIncident};
use esop_zenoh_gateway::runtime::{ConnectionState, ZenohGateway, decode_command_payload};
use esop_zenoh_gateway::{KeySpace, RouteKind};
use zenoh::Wait;

const ROUTER_ENDPOINT: &str = "tcp/127.0.0.1:17447";

fn client_config() -> zenoh::Config {
    zenoh::Config::from_json5(&format!(
        r#"{{mode: "client", connect: {{endpoints: ["{ROUTER_ENDPOINT}"]}}}}"#
    ))
    .expect("valid Zenoh client configuration")
}

fn route_key(space: KeySpace, kind: RouteKind) -> String {
    let mut buffer = [0; esop_zenoh_gateway::MAX_ZENOH_KEY_BYTES];
    let length = space.write_key(kind, &mut buffer).expect("route key fits");
    String::from_utf8(buffer[..length].to_vec()).expect("route key is UTF-8")
}

fn command_payload() -> Vec<u8> {
    MotionCommand {
        robot_id: "robot_01".to_owned(),
        boot_id: 7,
        source_id: 42,
        permit_epoch: 1,
        sequence: 1,
        deadline_ns: 10_000,
        axis_mask: 0x03,
        authority: 2,
        policy_version: 9,
        ..MotionCommand::default()
    }
    .encode_to_vec()
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
#[ignore = "requires zenohd on tcp/127.0.0.1:17447; run make test-zenoh"]
fn router_round_trip_covers_gateway_contracts() {
    tokio::runtime::Runtime::new()
        .expect("Tokio runtime starts")
        .block_on(async {
            let space = KeySpace::new(b"fleet_a", b"robot_01").expect("valid key space");
            let gateway = ZenohGateway::open(space, client_config())
                .await
                .expect("gateway session opens");
            let observer = zenoh::open(client_config())
                .await
                .expect("observer session opens");

            let (state_tx, state_rx) = mpsc::channel();
            observer
                .declare_subscriber(route_key(space, RouteKind::State))
                .callback(move |sample| {
                    let _ = state_tx.send(sample.payload().to_bytes().into_owned());
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
            let state_payload =
                RobotState::decode(state_received.expect("state reaches observer").as_slice())
                    .expect("state payload decodes");
            assert_eq!(state_payload.robot_id, "robot_01");
            assert_eq!(state_payload.sequence, 1);

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
            let mut payload = None;
            for _ in 0..10 {
                observer
                    .put(route_key(space, RouteKind::Command), command.clone())
                    .await
                    .expect("command publishes through router");
                if let Ok(received) = command_rx.recv_timeout(Duration::from_millis(500)) {
                    payload = Some(received);
                    break;
                }
            }
            let payload = payload.expect("command reaches gateway");
            let mut ingress = ingress();
            let permit = gateway
                .admit_command(&mut ingress, &payload, 1_000)
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
