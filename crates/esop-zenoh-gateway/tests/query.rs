#![cfg(feature = "zenoh")]

use esop_proto::v1::{QueryReply, QueryRequest, RobotState, RuntimeEvidence, RuntimeIncident};
use esop_proto::{CURRENT_SCHEMA_VERSION, Message};
use esop_zenoh_gateway::runtime::{
    MAX_QUERY_RECORDS, QueryAdapterError, decode_query_payload, encode_query_reply,
};
use esop_zenoh_gateway::{KeySpace, MAX_PAYLOAD_BYTES, RouteError};

fn space() -> KeySpace {
    KeySpace::new(b"fleet_a", b"robot_01").unwrap()
}

fn query_request() -> QueryRequest {
    QueryRequest {
        robot_id: "robot_01".into(),
        boot_id: 7,
        after_sequence: 10,
        limit: 2,
        schema_version: CURRENT_SCHEMA_VERSION,
    }
}

fn reply() -> QueryReply {
    QueryReply {
        robot_id: "robot_01".into(),
        boot_id: 7,
        schema_version: CURRENT_SCHEMA_VERSION,
        states: vec![RobotState {
            robot_id: "robot_01".into(),
            boot_id: 7,
            sequence: 11,
            schema_version: CURRENT_SCHEMA_VERSION,
            ..Default::default()
        }],
        incidents: vec![RuntimeIncident {
            schema_version: CURRENT_SCHEMA_VERSION,
            ..Default::default()
        }],
        truncated: true,
    }
}

#[test]
fn v1_query_round_trip_preserves_cursor_and_result() {
    let request = query_request();
    let decoded = decode_query_payload(space(), 7, &request.encode_to_vec()).unwrap();
    assert_eq!(decoded, request);
    let encoded = encode_query_reply(&decoded, &reply()).unwrap();
    assert_eq!(QueryReply::decode(encoded.as_slice()).unwrap(), reply());
}

#[test]
fn rejects_malformed_cross_robot_stale_boot_and_unbounded_requests() {
    assert!(matches!(
        decode_query_payload(space(), 7, &[]),
        Err(QueryAdapterError::Payload(RouteError::EmptyPayload))
    ));
    assert!(matches!(
        decode_query_payload(space(), 7, &[0xff]),
        Err(QueryAdapterError::Decode(_))
    ));
    assert!(matches!(
        decode_query_payload(space(), 7, &vec![0; MAX_PAYLOAD_BYTES + 1]),
        Err(QueryAdapterError::Payload(RouteError::PayloadTooLarge))
    ));
    let mut request = query_request();
    request.schema_version = 0;
    assert!(matches!(
        decode_query_payload(space(), 7, &request.encode_to_vec()),
        Err(QueryAdapterError::Schema(_))
    ));
    request = query_request();
    request.robot_id = "robot_02".into();
    assert!(matches!(
        decode_query_payload(space(), 7, &request.encode_to_vec()),
        Err(QueryAdapterError::RobotMismatch)
    ));
    request = query_request();
    request.boot_id = 6;
    assert!(matches!(
        decode_query_payload(space(), 7, &request.encode_to_vec()),
        Err(QueryAdapterError::BootMismatch)
    ));
    for limit in [0, MAX_QUERY_RECORDS + 1] {
        request = query_request();
        request.limit = limit;
        assert!(matches!(
            decode_query_payload(space(), 7, &request.encode_to_vec()),
            Err(QueryAdapterError::LimitOutOfRange)
        ));
    }
}

#[test]
fn rejects_reply_contract_violations_before_publication() {
    let request = query_request();
    let mut response = reply();
    response.schema_version = 0;
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::Schema(_))
    ));
    response = reply();
    response.robot_id = "robot_02".into();
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::RobotMismatch)
    ));
    response = reply();
    response.boot_id = 8;
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::BootMismatch)
    ));
    response = reply();
    response.states[0].schema_version = 0;
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::Schema(_))
    ));
    response = reply();
    response.states[0].robot_id = "robot_02".into();
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::RobotMismatch)
    ));
    response = reply();
    response.states[0].boot_id = 8;
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::BootMismatch)
    ));
    response = reply();
    response.states[0].sequence = request.after_sequence;
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::StaleState)
    ));
    response = reply();
    response.incidents[0].schema_version = 0;
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::Schema(_))
    ));
    response = reply();
    response.incidents[0].evidence.push(RuntimeEvidence {
        boot_id: 8,
        ..Default::default()
    });
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::BootMismatch)
    ));
    response = reply();
    response.states.push(response.states[0].clone());
    response.incidents.clear();
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::StaleState)
    ));
    response = reply();
    response.states.push(response.states[0].clone());
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::TooManyRecords)
    ));
    response = reply();
    response.incidents[0].suggested_action = "x".repeat(MAX_PAYLOAD_BYTES);
    assert!(matches!(
        encode_query_reply(&request, &response),
        Err(QueryAdapterError::Payload(RouteError::PayloadTooLarge))
    ));
}
