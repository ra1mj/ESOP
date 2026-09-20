#![cfg(feature = "zenoh")]

use esop_lifecycle_guard::{
    AxisStopPolicy, CyclicQuality, GateId, GuardPolicy, LifecycleGuard,
    LifecycleState as GuardState, MotionPermit, PermitError, STOP_TIMEOUT_FAULT_CODE,
    StopAction as GuardStop, StopFeedback,
    cia402::step_axis_bank,
    procbuf::{
        AxisEvidenceError, STOP_TIMEOUT_EVENT_CODE, STOP_TIMEOUT_EVENT_SOURCE,
        StopTimeoutEventError, axis_stops_to_procbuf, cyclic_quality_to_procbuf,
        lifecycle_to_procbuf, stop_timeout_events_to_procbuf,
    },
};
use esop_procbuf::{
    AxisStopEvidence as RawAxisStopEvidence, EventPushError, EventSeverity as ProcSeverity,
    HeaderError, ProcBuf, ProcBufEvent, QualityFact, StatePage,
};
use esop_profile_cia402::{Cia402AxisBank, DriveRequest};
use esop_proto::v1::{EventSeverity, LifecycleState, StopAction};
use esop_proto::{CURRENT_SCHEMA_VERSION, Message};
use esop_zenoh_gateway::procbuf_adapter::{ProcBufProjector, ProjectionError};
use esop_zenoh_gateway::{KeySpace, MAX_PAYLOAD_BYTES};

type TestBuf = ProcBuf<2, 1, 1, 4>;

fn projector() -> ProcBufProjector {
    ProcBufProjector::new(KeySpace::new(b"fleet_a", b"robot_01").unwrap(), 42, 7)
}

fn state(sequence: u64) -> StatePage<2, 1, 1> {
    let mut state = StatePage::new(7);
    state.sequence = sequence;
    state.monotonic_time_ns = 123_000;
    state.ecat_time_ns = 122_000;
    state.axes[0].position = 1.5;
    state.axes[0].velocity = 0.25;
    state.axes[0].statusword = 0x27;
    state.axes[0].quality = 1;
    state.io[0].input_bits = 0x03;
    state.io[0].quality = 2;
    state.lifecycle.state = GuardState::Stopping as u8;
    state.lifecycle.stop_action = GuardStop::QuickStop as u8;
    state.lifecycle.required_gate_mask = 0x027;
    state.lifecycle.valid_gate_mask = 0x025;
    state.lifecycle.qualified_gate_mask = 0x005;
    state.lifecycle.ready_gate_mask = 0x025;
    state.lifecycle.first_blocking_code = 0x102;
    state.lifecycle.latched_fault_code = 0x204;
    state.lifecycle.permit_epoch = 3;
    state.lifecycle.permit_expires_at_ns = 250_000;
    state.lifecycle.transition_sequence = 8;
    state.lifecycle.transition_cycle = 7;
    state.lifecycle.recovery_count = 2;
    state.lifecycle.permit_audit_sequence = 4;
    state
}

fn quality_facts() -> CyclicQuality {
    CyclicQuality {
        platform_ready: true,
        coe_ready: false,
        topology_valid: true,
        distributed_clock_locked: false,
        drive_ready: true,
        domain_valid: false,
        wkc_valid: true,
        command_current: false,
        supervisor_healthy: true,
        external_safety_clear: false,
        cycle_within_budget: true,
    }
}

#[test]
fn maps_one_complete_procbuf_state_without_inventing_missing_quality() {
    let buffer = TestBuf::new(42, 7);
    let mut projector = projector();
    assert!(projector.read_state(&buffer).unwrap().is_none());
    buffer.publish_state(state(9)).unwrap();
    let projected = projector.read_state(&buffer).unwrap().unwrap();
    assert_eq!(projected.robot_id, "robot_01");
    assert_eq!((projected.boot_id, projected.sequence), (7, 9));
    assert_eq!(
        (projected.monotonic_time_ns, projected.ecat_time_ns),
        (123_000, 122_000)
    );
    assert_eq!(projected.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(projected.joints.len(), 2);
    assert_eq!(
        (projected.joints[0].axis, projected.joints[0].position),
        (0, 1.5)
    );
    assert_eq!(projected.joints[0].statusword, 0x27);
    assert_eq!(
        (projected.io[0].channel, projected.io[0].input_bits),
        (0, 3)
    );
    let lifecycle = projected.lifecycle.as_ref().unwrap();
    assert_eq!(lifecycle.state, LifecycleState::Stopping as i32);
    assert_eq!(lifecycle.stop_action, StopAction::QuickStop as i32);
    assert_eq!(lifecycle.required_gate_mask, 0x027);
    assert_eq!(lifecycle.valid_gate_mask, 0x025);
    assert_eq!(lifecycle.qualified_gate_mask, 0x005);
    assert_eq!(
        (lifecycle.ready_gate_mask, lifecycle.transition_sequence),
        (0x025, 8)
    );
    assert_eq!(
        (lifecycle.first_blocking_code, lifecycle.latched_fault_code),
        (0x102, 0x204)
    );
    assert_eq!(lifecycle.permit_epoch, 3);
    assert_eq!(lifecycle.permit_expires_at_ns, 250_000);
    assert_eq!(lifecycle.transition_cycle, 7);
    assert_eq!(lifecycle.recovery_count, 2);
    assert_eq!(lifecycle.permit_audit_sequence, 4);
    assert!(projected.quality.is_none());
    assert!(projected.events.is_empty());
    assert!(projected.encoded_len() <= MAX_PAYLOAD_BYTES);
    assert!(projector.read_state(&buffer).unwrap().is_none());
}

#[test]
fn guard_snapshot_flows_through_procbuf_to_protobuf_without_losing_gate_or_audit_facts() {
    let required = GateId::Platform.bit() | GateId::ExternalSafety.bit();
    let mut guard = LifecycleGuard::new(
        required,
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            exit_bad_cycles: 1,
            max_age_cycles: 1,
            stop_timeout_cycles: 1_000,
            stop_action: GuardStop::QuickStop,
            authorized_source_id: 11,
            minimum_authority: 1,
            permit_policy_version: 1,
            allowed_axis_mask: 0x03,
        },
    );
    guard.update_gate(GateId::Platform, true, 4, 0);
    guard.update_gate(GateId::ExternalSafety, false, 4, 0x5341_0001);
    let permit = MotionPermit {
        boot_id: 7,
        source_id: 11,
        permit_epoch: 3,
        sequence: 1,
        axis_mask: 3,
        expires_at_ns: 250_000,
        authority: 1,
        reserved: [0; 3],
        policy_version: 1,
    };
    guard.accept_permit(permit, 123_000).unwrap();
    assert_eq!(
        guard.accept_permit(permit, 123_001),
        Err(PermitError::SequenceReplayed)
    );
    let snapshot = guard.snapshot(4, 123_001);
    let buffer = TestBuf::new(42, 7);
    let mut state = state(10);
    state.lifecycle = lifecycle_to_procbuf(snapshot, 0);
    buffer.publish_state(state).unwrap();
    let projected = projector().read_state(&buffer).unwrap().unwrap();
    let lifecycle = projected.lifecycle.unwrap();
    assert_eq!(lifecycle.required_gate_mask, u32::from(required));
    assert_eq!(lifecycle.valid_gate_mask, u32::from(GateId::Platform.bit()));
    assert_eq!(
        lifecycle.qualified_gate_mask,
        u32::from(GateId::Platform.bit())
    );
    assert_eq!(lifecycle.ready_gate_mask, u32::from(GateId::Platform.bit()));
    assert_eq!(lifecycle.first_blocking_code, 0x5341_0001);
    assert_eq!(lifecycle.permit_epoch, 3);
    assert_eq!(lifecycle.permit_expires_at_ns, 250_000);
    assert_eq!(lifecycle.permit_audit_sequence, 1);
    assert_eq!(
        lifecycle.motion_permit_current,
        snapshot.motion_permit_current
    );
}

#[test]
fn per_axis_stop_request_issued_control_and_observed_feedback_survive_projection() {
    let policy = GuardPolicy {
        enter_good_cycles: 1,
        allowed_axis_mask: 0b11,
        ..GuardPolicy::conservative()
    };
    let actions = AxisStopPolicy::uniform(GuardStop::QuickStop)
        .with_action(1, GuardStop::Hold)
        .unwrap();
    let mut guard =
        LifecycleGuard::new_with_axis_stop_policy(GateId::Link.bit(), 7, policy, actions);
    let mut bank = Cia402AxisBank::<2>::new();
    guard.update_gate(GateId::Link, true, 1, 0);
    guard
        .request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                axis_mask: 0b11,
                expires_at_ns: 100,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            1,
            1,
        )
        .unwrap();
    guard.update_gate(GateId::Link, false, 2, 0xCAFE);
    let buffer = TestBuf::new(42, 7);
    let mut projector = projector();
    let mut initial = state(2);
    {
        let decision = guard.cycle_axes(2, 2);
        let outputs = step_axis_bank(&mut bank, &decision, [0x0027; 2], [DriveRequest::Enable; 2]);
        axis_stops_to_procbuf(&mut initial, &decision, &outputs, None).unwrap();
    }
    initial.lifecycle = lifecycle_to_procbuf(guard.snapshot(2, 2), 2);
    buffer.publish_state(initial).unwrap();
    let first = projector.read_state(&buffer).unwrap().unwrap();
    assert!(
        first
            .lifecycle
            .unwrap()
            .axis_stops
            .iter()
            .all(|stop| !stop.feedback_observed)
    );

    guard.update_gate(GateId::Link, false, 3, 0xCAFE);
    let decision = guard.cycle_axes(3, 3);
    let outputs = step_axis_bank(
        &mut bank,
        &decision,
        [0x0040, 0x0027],
        [DriveRequest::Enable; 2],
    );
    let mut state = state(3);
    let feedback = StopFeedback {
        cycle: 3,
        observed_axis_mask: 0b11,
        stationary_axis_mask: 0b01,
        non_enabled_axis_mask: 0b01,
    };
    axis_stops_to_procbuf(&mut state, &decision, &outputs, Some(feedback)).unwrap();
    state.lifecycle = lifecycle_to_procbuf(guard.snapshot(3, 3), 3);
    buffer.publish_state(state).unwrap();
    let projected = projector.read_state(&buffer).unwrap().unwrap();
    let stops = projected.lifecycle.unwrap().axis_stops;
    assert_eq!(stops.len(), 2);
    assert_eq!(
        (
            stops[0].axis,
            stops[0].requested_action,
            stops[0].issued_action
        ),
        (
            0,
            StopAction::QuickStop as i32,
            StopAction::QuickStop as i32
        )
    );
    assert_eq!(
        (
            stops[1].axis,
            stops[1].requested_action,
            stops[1].issued_action
        ),
        (1, StopAction::Hold as i32, StopAction::Disable as i32)
    );
    assert_eq!((stops[0].request_cycle, stops[0].feedback_cycle), (3, 3));
    assert!(stops[0].feedback_observed && stops[0].stationary && stops[0].non_enabled);
    assert!(stops[1].feedback_observed && !stops[1].stationary && !stops[1].non_enabled);
}

#[test]
fn stale_and_unsafe_axis_stop_evidence_cannot_be_published_as_current() {
    let mut guard = LifecycleGuard::new_with_axis_stop_policy(
        0,
        7,
        GuardPolicy {
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
        AxisStopPolicy::uniform(GuardStop::QuickStop),
    );
    guard
        .request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                axis_mask: 1,
                expires_at_ns: 100,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            1,
            1,
        )
        .unwrap();
    guard.set_maintenance(true, 2);
    let decision = guard.cycle_axes(2, 2);
    let mut bank = Cia402AxisBank::<2>::new();
    let outputs = step_axis_bank(&mut bank, &decision, [0x0027; 2], [DriveRequest::Enable; 2]);
    let mut page = state(2);
    let initial = page.axis_stops;
    let stale = StopFeedback {
        cycle: 1,
        observed_axis_mask: 1,
        stationary_axis_mask: 1,
        non_enabled_axis_mask: 1,
    };
    assert_eq!(
        axis_stops_to_procbuf(&mut page, &decision, &outputs, Some(stale)),
        Err(AxisEvidenceError::CycleMismatch)
    );
    assert_eq!(page.axis_stops, initial);
    let premature = StopFeedback {
        cycle: 2,
        observed_axis_mask: 1,
        stationary_axis_mask: 1,
        non_enabled_axis_mask: 1,
    };
    assert_eq!(
        axis_stops_to_procbuf(&mut page, &decision, &outputs, Some(premature)),
        Err(AxisEvidenceError::FeedbackBeforeStop)
    );
    assert_eq!(page.axis_stops, initial);
    let outside = StopFeedback {
        cycle: 2,
        observed_axis_mask: 2,
        stationary_axis_mask: 0,
        non_enabled_axis_mask: 0,
    };
    assert_eq!(
        axis_stops_to_procbuf(&mut page, &decision, &outputs, Some(outside)),
        Err(AxisEvidenceError::InvalidFeedback)
    );
    assert_eq!(page.axis_stops, initial);
    let mut unsafe_outputs = outputs;
    unsafe_outputs[0].controlword = 0x000f;
    assert_eq!(
        axis_stops_to_procbuf(&mut page, &decision, &unsafe_outputs, None),
        Err(AxisEvidenceError::UnsafeOutput(0))
    );
    assert_eq!(page.axis_stops, initial);
    page.sequence = 3;
    assert_eq!(
        axis_stops_to_procbuf(&mut page, &decision, &outputs, None),
        Err(AxisEvidenceError::CycleMismatch)
    );
    page.sequence = 2;
    axis_stops_to_procbuf(&mut page, &decision, &outputs, None).unwrap();
    assert_eq!(
        page.axis_stops[0].requested_action,
        StopAction::Disable as u8
    );
    assert_eq!(page.axis_stops[0].feedback_valid, 0);
    assert_eq!(page.axis_stops[0].feedback_cycle, 0);
    assert_eq!(page.axis_stops[1], RawAxisStopEvidence::EMPTY);
    let mut oversized = StatePage::<33, 0, 0>::new(7);
    oversized.sequence = 2;
    assert_eq!(
        axis_stops_to_procbuf(&mut oversized, &decision, &[outputs[0]; 33], None),
        Err(AxisEvidenceError::AxisCapacityExceeded)
    );

    let buffer = TestBuf::new(42, 7);
    let mut old = state(4);
    old.axis_stops[0] = RawAxisStopEvidence {
        request_cycle: 3,
        requested_action: 3,
        issued_action: 3,
        ..RawAxisStopEvidence::EMPTY
    };
    buffer.publish_state(old).unwrap();
    assert!(
        projector()
            .read_state(&buffer)
            .unwrap()
            .unwrap()
            .lifecycle
            .unwrap()
            .axis_stops
            .is_empty()
    );
}

#[test]
fn raw_cyclic_quality_projects_all_observed_facts_without_using_qualified_gate_masks() {
    let buffer = TestBuf::new(42, 7);
    let facts = quality_facts();
    let mut state = state(10);
    cyclic_quality_to_procbuf(&mut state, facts);
    buffer.publish_state(state).unwrap();

    let projected = projector().read_state(&buffer).unwrap().unwrap();
    let quality = projected.quality.unwrap();
    assert!(quality.platform_ready);
    assert!(!quality.configuration_ready);
    assert!(quality.topology_valid);
    assert!(!quality.distributed_clock_locked);
    assert!(quality.drive_ready);
    assert!(!quality.domain_valid);
    assert!(quality.wkc_valid);
    assert!(!quality.command_current);
    assert!(quality.supervisor_healthy);
    assert!(!quality.external_safety_clear);
    assert!(quality.cycle_within_budget);
    assert_eq!(quality.first_fault_code, 0x102);
    assert_eq!(projected.lifecycle.unwrap().ready_gate_mask, 0x025);
}

#[test]
fn unknown_stale_and_invalid_quality_never_appears_as_complete() {
    let buffer = TestBuf::new(42, 7);
    let mut projector = projector();

    let mut partial = state(10);
    partial.quality.sequence = 10;
    partial.quality.cyclic.known_mask = QualityFact::Platform.bit();
    partial.quality.cyclic.good_mask = QualityFact::Platform.bit();
    buffer.publish_state(partial).unwrap();
    assert!(
        projector
            .read_state(&buffer)
            .unwrap()
            .unwrap()
            .quality
            .is_none()
    );

    let mut stale = state(11);
    cyclic_quality_to_procbuf(&mut stale, quality_facts());
    stale.quality.sequence = 10;
    buffer.publish_state(stale).unwrap();
    assert!(
        projector
            .read_state(&buffer)
            .unwrap()
            .unwrap()
            .quality
            .is_none()
    );

    let mut invalid = state(12);
    cyclic_quality_to_procbuf(&mut invalid, quality_facts());
    invalid.quality.cyclic.good_mask |= 1 << 15;
    buffer.publish_state(invalid).unwrap();
    assert_eq!(
        projector.read_state(&buffer),
        Err(ProjectionError::InvalidQualityMask)
    );

    let mut all_bad = state(13);
    let facts = CyclicQuality {
        platform_ready: false,
        coe_ready: false,
        topology_valid: false,
        distributed_clock_locked: false,
        drive_ready: false,
        domain_valid: false,
        wkc_valid: false,
        command_current: false,
        supervisor_healthy: false,
        external_safety_clear: false,
        cycle_within_budget: false,
    };
    cyclic_quality_to_procbuf(&mut all_bad, facts);
    buffer.publish_state(all_bad).unwrap();
    let quality = projector
        .read_state(&buffer)
        .unwrap()
        .unwrap()
        .quality
        .unwrap();
    assert!(!quality.platform_ready && !quality.wkc_valid && !quality.external_safety_clear);
}

#[test]
fn checks_header_before_consumption_and_rejects_replay_or_invalid_state() {
    let buffer = TestBuf::new(42, 7);
    buffer.publish_state(state(9)).unwrap();
    let space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
    let mut wrong_robot = ProcBufProjector::new(space, 43, 7);
    assert!(matches!(
        wrong_robot.read_state(&buffer),
        Err(ProjectionError::Header(HeaderError::RobotIdMismatch))
    ));
    let mut wrong_boot = ProcBufProjector::new(space, 42, 8);
    assert!(matches!(
        wrong_boot.read_state(&buffer),
        Err(ProjectionError::Header(HeaderError::BootIdMismatch))
    ));
    let mut projector = projector();
    assert_eq!(projector.read_state(&buffer).unwrap().unwrap().sequence, 9);
    buffer.publish_state(state(9)).unwrap();
    assert!(matches!(
        projector.read_state(&buffer),
        Err(ProjectionError::ReplayedState)
    ));
    let mut invalid = state(10);
    invalid.lifecycle.state = 255;
    buffer.publish_state(invalid).unwrap();
    assert!(matches!(
        projector.read_state(&buffer),
        Err(ProjectionError::InvalidLifecycleState(255))
    ));
    invalid = state(11);
    invalid.lifecycle.stop_action = 255;
    buffer.publish_state(invalid).unwrap();
    assert!(matches!(
        projector.read_state(&buffer),
        Err(ProjectionError::InvalidStopAction(255))
    ));
    invalid = state(12);
    invalid.axes[1].following_error = f64::NAN;
    buffer.publish_state(invalid).unwrap();
    assert!(matches!(
        projector.read_state(&buffer),
        Err(ProjectionError::NonFiniteJoint(1))
    ));
    buffer.publish_state(state(13)).unwrap();
    assert_eq!(projector.read_state(&buffer).unwrap().unwrap().sequence, 13);
    let mut stale_feedback = state(14);
    stale_feedback.axis_stops[0] = RawAxisStopEvidence {
        request_cycle: 14,
        feedback_cycle: 13,
        requested_action: StopAction::QuickStop as u8,
        issued_action: StopAction::QuickStop as u8,
        feedback_valid: 1,
        ..RawAxisStopEvidence::EMPTY
    };
    buffer.publish_state(stale_feedback).unwrap();
    assert_eq!(
        projector.read_state(&buffer),
        Err(ProjectionError::InvalidAxisStopEvidence(0))
    );
    let mut inconsistent = state(15);
    inconsistent.axis_stops[0] = RawAxisStopEvidence {
        request_cycle: 15,
        requested_action: StopAction::QuickStop as u8,
        issued_action: StopAction::Hold as u8,
        ..RawAxisStopEvidence::EMPTY
    };
    buffer.publish_state(inconsistent).unwrap();
    assert_eq!(
        projector.read_state(&buffer),
        Err(ProjectionError::InvalidAxisStopEvidence(0))
    );
}

#[test]
fn refuses_state_that_cannot_fit_the_transport_payload() {
    let buffer = ProcBuf::<128, 0, 0, 2>::new(42, 7);
    let mut state = StatePage::<128, 0, 0>::new(7);
    state.sequence = 1;
    for joint in &mut state.axes {
        joint.position = 1.0;
        joint.velocity = 2.0;
        joint.torque = 3.0;
        joint.following_error = 4.0;
    }
    buffer.publish_state(state).unwrap();
    assert!(matches!(
        projector().read_state(&buffer),
        Err(ProjectionError::PayloadTooLarge)
    ));
}

#[test]
fn maps_fixed_event_ring_to_versioned_diagnostic_event() {
    let buffer = TestBuf::new(42, 7);
    let mut projector = projector();
    assert!(projector.pop_event(&buffer).unwrap().is_none());
    buffer
        .record_event(ProcBufEvent {
            sequence: 16,
            timestamp_ns: 150_000,
            source: 8,
            severity: ProcSeverity::Fault,
            code: 0x22,
            axis_or_device: 1,
            value: 6,
            aux: 7,
        })
        .unwrap();
    let event = projector.pop_event(&buffer).unwrap().unwrap();
    assert_eq!(
        (event.sequence, event.timestamp_ns, event.boot_id),
        (16, 150_000, 7)
    );
    assert_eq!(
        (event.source, event.code, event.axis_or_device),
        (8, 0x22, 1)
    );
    assert_eq!((event.value, event.aux), (6, 7));
    assert_eq!(event.severity, EventSeverity::Critical as i32);
    assert_eq!(event.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(projector.pop_event(&buffer).unwrap().is_none());
}

#[test]
fn stop_timeout_escalations_retain_original_axes_and_retry_full_event_ring() {
    let policy = GuardPolicy {
        enter_good_cycles: 1,
        stop_timeout_cycles: 2,
        allowed_axis_mask: 0b11,
        ..GuardPolicy::conservative()
    };
    let actions = AxisStopPolicy::uniform(GuardStop::QuickStop)
        .with_action(1, GuardStop::RampToZero)
        .unwrap();
    let mut guard =
        LifecycleGuard::new_with_axis_stop_policy(GateId::Link.bit(), 7, policy, actions);
    let mut bank = Cia402AxisBank::<2>::new();
    let buffer = ProcBuf::<2, 0, 0, 1>::new(42, 7);
    let mut projector = projector();
    guard.update_gate(GateId::Link, true, 1, 0);
    guard
        .request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                axis_mask: 0b11,
                expires_at_ns: 100,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            1,
            1,
        )
        .unwrap();
    guard.update_gate(GateId::Link, false, 2, 0xCAFE);
    assert_eq!(guard.cycle_axes(2, 2).stopping_axis_mask(), 0b11);

    let decision = guard.cycle_axes(4, 4);
    assert_eq!(
        decision.action(),
        esop_lifecycle_guard::LifecycleAction::FaultLatched
    );
    let outputs = step_axis_bank(&mut bank, &decision, [0x0027; 2], [DriveRequest::Enable; 2]);
    assert!(outputs.iter().all(|output| {
        output.controlword == esop_profile_cia402::CONTROLWORD_DISABLE_VOLTAGE
            && !output.motion_allowed
    }));
    let mut page = StatePage::<2, 0, 0>::new(7);
    page.sequence = 4;
    axis_stops_to_procbuf(&mut page, &decision, &outputs, None).unwrap();
    assert_eq!(page.axis_stops, [RawAxisStopEvidence::EMPTY; 2]);
    page.lifecycle = lifecycle_to_procbuf(guard.snapshot(4, 400), 400);
    assert_eq!(page.lifecycle.first_blocking_code, 0xCAFE);
    assert_eq!(page.lifecycle.latched_fault_code, STOP_TIMEOUT_FAULT_CODE);
    buffer.publish_state(page).unwrap();
    let state = projector.read_state(&buffer).unwrap().unwrap();
    assert!(state.lifecycle.unwrap().axis_stops.is_empty());

    let record = guard.stop_timeout_record().unwrap();
    assert_eq!(
        (record.cycle, record.axis_mask, record.first_issued_cycle),
        (4, 0b11, Some(2))
    );
    assert_eq!(record.requested_action(0), Some(GuardStop::QuickStop));
    assert_eq!(record.requested_action(1), Some(GuardStop::RampToZero));
    assert_eq!(record.requested_action(2), None);

    let wrong_boot = ProcBuf::<2, 0, 0, 1>::new(42, 8);
    assert_eq!(
        stop_timeout_events_to_procbuf(&mut guard, &wrong_boot, 400),
        Err(StopTimeoutEventError::Header(HeaderError::BootIdMismatch))
    );
    let too_few_axes = ProcBuf::<1, 0, 0, 1>::new(42, 7);
    assert_eq!(
        stop_timeout_events_to_procbuf(&mut guard, &too_few_axes, 400),
        Err(StopTimeoutEventError::AxisCapacityExceeded)
    );
    assert_eq!(wrong_boot.pending_events(), 0);
    assert_eq!(too_few_axes.pending_events(), 0);

    assert_eq!(
        stop_timeout_events_to_procbuf(&mut guard, &buffer, 401),
        Err(StopTimeoutEventError::Ring(EventPushError::Full))
    );
    let first = projector.pop_event(&buffer).unwrap().unwrap();
    assert_eq!(first.boot_id, 7);
    assert_eq!(first.sequence, record.transition_sequence);
    assert_eq!(first.timestamp_ns, 401);
    assert_eq!(
        (first.source, first.code),
        (
            u32::from(STOP_TIMEOUT_EVENT_SOURCE),
            u32::from(STOP_TIMEOUT_EVENT_CODE)
        )
    );
    assert_eq!(first.axis_or_device, 0);
    assert_eq!(first.severity, EventSeverity::Critical as i32);
    assert_eq!(first.value, STOP_TIMEOUT_FAULT_CODE);
    assert_eq!(first.aux, 3 | (4 << 8) | (1 << 16) | (4 << 24));
    assert_eq!(buffer.lost_events(), 1);

    assert_eq!(
        stop_timeout_events_to_procbuf(&mut guard, &buffer, 402),
        Ok(1)
    );
    let second = projector.pop_event(&buffer).unwrap().unwrap();
    assert_eq!(second.sequence, record.transition_sequence);
    assert_eq!(second.axis_or_device, 1);
    assert_eq!(second.aux, 2 | (4 << 8) | (1 << 16) | (4 << 24));
    assert_eq!(
        stop_timeout_events_to_procbuf(&mut guard, &buffer, 403),
        Ok(0)
    );
    assert!(projector.pop_event(&buffer).unwrap().is_none());
}
