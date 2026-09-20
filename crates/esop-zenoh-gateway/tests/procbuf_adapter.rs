#![cfg(feature = "zenoh")]

use esop_lifecycle_guard::{LifecycleState as GuardState, StopAction as GuardStop};
use esop_procbuf::{EventSeverity as ProcSeverity, HeaderError, ProcBuf, ProcBufEvent, StatePage};
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
    state.lifecycle.gate_mask = 0x025;
    state.lifecycle.first_blocking_code = 0x102;
    state.lifecycle.latched_fault_code = 0x204;
    state.lifecycle.transition_sequence = 8;
    state.lifecycle.recovery_count = 2;
    state
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
    assert_eq!(
        (lifecycle.ready_gate_mask, lifecycle.transition_sequence),
        (0x025, 8)
    );
    assert_eq!(
        (lifecycle.first_blocking_code, lifecycle.latched_fault_code),
        (0x102, 0x204)
    );
    assert_eq!(lifecycle.recovery_count, 2);
    assert!(projected.quality.is_none());
    assert!(projected.events.is_empty());
    assert!(projected.encoded_len() <= MAX_PAYLOAD_BYTES);
    assert!(projector.read_state(&buffer).unwrap().is_none());
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
