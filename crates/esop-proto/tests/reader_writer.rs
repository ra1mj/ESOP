use esop_proto::{Message, v1};

mod baseline {
    include!(concat!(env!("OUT_DIR"), "/baseline/esop.v1.rs"));
}

fn bidirectional<Old, New>(old: Old, new: New)
where
    Old: Message + Default + PartialEq + std::fmt::Debug,
    New: Message + Default + PartialEq + std::fmt::Debug,
{
    assert_eq!(Old::decode(new.encode_to_vec().as_slice()).unwrap(), old);
    assert_eq!(New::decode(old.encode_to_vec().as_slice()).unwrap(), new);
}

// Independent generated types share sample values, not field/tag definitions.
macro_rules! samples {
    ($schema:ident) => {{
        let event = $schema::DiagnosticEvent {
            sequence: 23,
            timestamp_ns: 456,
            source: 2,
            severity: 3,
            code: 12,
            axis_or_device: 1,
            value: 4,
            aux: 5,
            boot_id: 9,
            schema_version: 1,
        };
        let state = $schema::RobotState {
            robot_id: "robot_01".into(),
            boot_id: 9,
            sequence: 21,
            monotonic_time_ns: 1234,
            ecat_time_ns: 1230,
            schema_version: 1,
            lifecycle: Some($schema::LifecycleSummary {
                state: 3,
                stop_action: 4,
                permit_epoch: 8,
                transition_sequence: 19,
                motion_permit_current: true,
                ..Default::default()
            }),
            quality: Some($schema::QualitySummary {
                domain_valid: true,
                wkc_valid: true,
                first_fault_code: 17,
                ..Default::default()
            }),
            joints: vec![$schema::JointState {
                axis: 1,
                position: -1.25,
                velocity: 2.5,
                torque: -3.75,
                statusword: 0x27,
                actual_mode: 8,
                ..Default::default()
            }],
            io: vec![$schema::IoState {
                channel: 2,
                input_bits: 0x55,
                quality: 1,
            }],
            events: vec![event.clone()],
        };
        let command = $schema::MotionCommand {
            robot_id: "robot_01".into(),
            boot_id: 9,
            source_id: 42,
            permit_epoch: 8,
            sequence: 22,
            deadline_ns: 5000,
            axis_mask: 3,
            authority: 2,
            policy_version: 7,
            requested_mode: 8,
            schema_version: 1,
            joints: vec![$schema::JointTarget {
                axis: 1,
                position: -1.25,
                velocity: 2.5,
                torque: -3.75,
                max_velocity: 10.0,
                max_torque: 20.0,
            }],
        };
        let reply = $schema::CommandReply {
            robot_id: "robot_01".into(),
            boot_id: 9,
            command_sequence: 22,
            decision: 2,
            reason_code: 17,
            audit_sequence: 50,
            accepted_at_ns: 1500,
            schema_version: 1,
        };
        let mut evidence = $schema::RuntimeEvidence::default();
        evidence.kind = 3;
        evidence.timestamp_ns = 150;
        evidence.pid = 123;
        evidence.tid = 124;
        evidence.cpu = 2;
        evidence.attach_point = 1;
        evidence.value = 42;
        evidence.cycle_sequence = 21;
        evidence.boot_id = 9;
        let mut incident = $schema::RuntimeIncident::default();
        incident.incident_id = "inc-1".into();
        incident.severity = 4;
        incident.reason_code = 7;
        incident.window_start_ns = 100;
        incident.window_end_ns = 200;
        incident.cycle_sequence = 21;
        incident.transition_sequence = 19;
        incident.lost_event_count = 3;
        incident.affected_component = "supervisor".into();
        incident.suggested_action = "stop".into();
        incident.schema_version = 1;
        incident.evidence = vec![evidence];
        let query = $schema::QueryRequest {
            robot_id: "robot_01".into(),
            boot_id: 9,
            after_sequence: 20,
            limit: 2,
            schema_version: 1,
        };
        let query_reply = $schema::QueryReply {
            robot_id: "robot_01".into(),
            boot_id: 9,
            states: vec![state.clone()],
            incidents: vec![incident.clone()],
            truncated: true,
            schema_version: 1,
        };
        (state, event, command, reply, incident, query, query_reply)
    }};
}

#[test]
fn all_top_level_messages_support_old_new_reader_writer_pairs() {
    let old = samples!(baseline);
    let new = samples!(v1);
    bidirectional(old.0, new.0);
    bidirectional(old.1, new.1);
    bidirectional(old.2, new.2);
    bidirectional(old.3, new.3);
    bidirectional(old.4, new.4);
    bidirectional(old.5, new.5);
    bidirectional(old.6, new.6);
}

#[derive(Clone, PartialEq, Message)]
struct AdditiveState {
    #[prost(string, tag = "1")]
    robot_id: String,
    #[prost(uint32, tag = "13")]
    schema_version: u32,
    #[prost(string, tag = "100")]
    future_label: String,
}

#[test]
fn additive_fields_decode_but_are_not_preserved_by_a_prost_relay() {
    let writer = AdditiveState {
        robot_id: "robot_01".into(),
        schema_version: 1,
        future_label: "new".into(),
    };
    let encoded = writer.encode_to_vec();
    let old = baseline::RobotState::decode(encoded.as_slice()).unwrap();
    let new = v1::RobotState::decode(encoded.as_slice()).unwrap();
    assert_eq!(old.robot_id, writer.robot_id);
    assert_eq!(new.robot_id, writer.robot_id);
    assert_eq!(old.schema_version, 1);
    assert_eq!(new.schema_version, 1);
    for relayed in [old.encode_to_vec(), new.encode_to_vec()] {
        let decoded = AdditiveState::decode(relayed.as_slice()).unwrap();
        assert_eq!(decoded.schema_version, 1);
        assert_eq!(decoded.future_label, "");
    }
}

#[test]
fn unknown_enum_values_remain_unknown_for_both_readers() {
    let bytes = v1::LifecycleSummary {
        state: 99,
        ..Default::default()
    }
    .encode_to_vec();
    let old = baseline::LifecycleSummary::decode(bytes.as_slice()).unwrap();
    let new = v1::LifecycleSummary::decode(bytes.as_slice()).unwrap();
    assert_eq!(old.state, 99);
    assert_eq!(new.state, 99);
    assert!(baseline::LifecycleState::try_from(old.state).is_err());
    assert!(v1::LifecycleState::try_from(new.state).is_err());
}

#[test]
fn additive_axis_stop_evidence_is_visible_to_new_readers_without_changing_legacy_summary() {
    let message = v1::LifecycleSummary {
        state: v1::LifecycleState::Stopping as i32,
        stop_action: v1::StopAction::QuickStop as i32,
        axis_stops: vec![v1::AxisStopEvidence {
            axis: 1,
            requested_action: v1::StopAction::Hold as i32,
            issued_action: v1::StopAction::Disable as i32,
            feedback_observed: true,
            stationary: false,
            non_enabled: false,
            request_cycle: 9,
            feedback_cycle: 9,
        }],
        ..Default::default()
    };
    let bytes = message.encode_to_vec();
    let old = baseline::LifecycleSummary::decode(bytes.as_slice()).unwrap();
    assert_eq!(old.state, message.state);
    assert_eq!(old.stop_action, message.stop_action);
    let projected = v1::LifecycleSummary::decode(bytes.as_slice()).unwrap();
    assert_eq!(projected.axis_stops, message.axis_stops);
    assert!(
        v1::LifecycleSummary::decode(old.encode_to_vec().as_slice())
            .unwrap()
            .axis_stops
            .is_empty()
    );
}

#[test]
fn additive_drive_error_code_is_visible_to_current_readers_only() {
    let message = v1::JointState {
        axis: 2,
        statusword: 0x0008,
        drive_state: 7,
        drive_error_code: 0x2310,
        ..Default::default()
    };
    let bytes = message.encode_to_vec();
    let old = baseline::JointState::decode(bytes.as_slice()).unwrap();
    assert_eq!(old.axis, message.axis);
    assert_eq!(old.statusword, message.statusword);

    let projected = v1::JointState::decode(bytes.as_slice()).unwrap();
    assert_eq!(projected.drive_error_code, 0x2310);
    assert_eq!(
        v1::JointState::decode(old.encode_to_vec().as_slice())
            .unwrap()
            .drive_error_code,
        0
    );
}

#[test]
fn additive_runtime_incident_fields_are_visible_to_current_readers_only() {
    let message = v1::RuntimeIncident {
        incident_id: "esop-boot-epoch-1".into(),
        severity: v1::IncidentSeverity::Error as i32,
        reason_code: 7,
        window_start_ns: 100,
        window_end_ns: 200,
        cycle_sequence: 12,
        transition_sequence: 9,
        lost_event_count: 2,
        affected_component: "host.network".into(),
        suggested_action: "controlled_stop".into(),
        schema_version: 1,
        boot_id: 11,
        agent_epoch: 3,
        confidence_percent: 75,
        cycle_first: 10,
        cycle_last: 12,
        evidence_window_ns: 500,
        pid: 123,
        tid: 124,
        cpu: 5,
        irq: 3,
        netdev_ifindex: 7,
        observed_value: u64::from(u32::MAX) + 9,
        threshold: 42,
        event_count: 4,
        evidence: vec![v1::RuntimeEvidence {
            kind: 2,
            timestamp_ns: 150,
            pid: 123,
            tid: 124,
            cpu: 5,
            attach_point: 0,
            value: u32::MAX,
            cycle_sequence: 11,
            boot_id: 11,
            evidence_id: 99,
            agent_epoch: 3,
            transition_sequence: 9,
            domain: 2,
            severity: v1::IncidentSeverity::Error as i32,
            irq: 3,
            netdev_ifindex: 7,
            observed_value: u64::from(u32::MAX) + 9,
            threshold: 42,
            duration_ns: 1_000,
            count: 4,
            detail: 8,
        }],
    };

    let bytes = message.encode_to_vec();
    let old = baseline::RuntimeIncident::decode(bytes.as_slice()).unwrap();
    assert_eq!(old.incident_id, message.incident_id);
    assert_eq!(old.cycle_sequence, message.cycle_sequence);
    assert_eq!(old.evidence[0].value, u32::MAX);
    assert_eq!(old.evidence[0].boot_id, 11);
    assert_eq!(
        v1::RuntimeIncident::decode(bytes.as_slice()).unwrap(),
        message
    );

    let relayed = v1::RuntimeIncident::decode(old.encode_to_vec().as_slice()).unwrap();
    assert_eq!(relayed.boot_id, 0);
    assert_eq!(relayed.agent_epoch, 0);
    assert_eq!(relayed.evidence[0].evidence_id, 0);
    assert_eq!(relayed.evidence[0].observed_value, 0);
}
