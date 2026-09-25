#![cfg(all(unix, feature = "payloads"))]

use esop_command_gateway::{CommandIngress, IngressPolicy};
use esop_ipc::payloads::{
    CommandDecodeError, CommandFrameError, CommandFrameIdentity, CommandPublicationError,
    CommandTargetError, JointTargetField, ProcBufCommandFrameError, ProcBufProjector, RobotId,
    RobotIdError, admit_command_frame, admit_procbuf_command_frame, decode_motion_command,
    encode_command_frame,
};
use esop_ipc::{IpcFrame, IpcHeader, MAX_PAYLOAD_BYTES, MessageKind, UnixDatagramEndpoint};
use esop_lifecycle_guard::{
    GuardPolicy, LifecycleGuard, StopAction, procbuf::motion_permit_from_command,
};
use esop_procbuf::{
    ControlMode, EventSeverity as ProcSeverity, IoCommand, JointCommand, ProcBuf, ProcBufEvent,
    QualityFact, StatePage,
};
use esop_proto::v1::{DiagnosticEvent, JointTarget, MotionCommand, RobotState};
use esop_proto::{CURRENT_SCHEMA_VERSION, Message};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

type TestBuf = ProcBuf<2, 1, 1, 8>;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let suffix = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("esop-ipc-payloads-{}-{suffix}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn robot_id() -> RobotId {
    RobotId::new("robot_01").unwrap()
}

fn endpoints(directory: &TestDirectory) -> (UnixDatagramEndpoint, UnixDatagramEndpoint) {
    let producer_path = directory.socket("producer.sock");
    let supervisor_path = directory.socket("supervisor.sock");
    let producer = UnixDatagramEndpoint::bind(&producer_path, &supervisor_path).unwrap();
    let supervisor = UnixDatagramEndpoint::bind(&supervisor_path, &producer_path).unwrap();
    (producer, supervisor)
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
            max_ttl_ns: 100,
            rate_window_ns: 1_000,
            max_commands_per_window: 2,
            reserved_tail: [0; 6],
        },
    )
}

fn command(sequence: u64) -> MotionCommand {
    MotionCommand {
        robot_id: "robot_01".to_owned(),
        boot_id: 7,
        source_id: 42,
        permit_epoch: 3,
        sequence,
        deadline_ns: 200,
        axis_mask: 0x03,
        authority: 2,
        policy_version: 9,
        schema_version: CURRENT_SCHEMA_VERSION,
        ..MotionCommand::default()
    }
}

fn targeted_command(sequence: u64, requested_mode: u32) -> MotionCommand {
    MotionCommand {
        requested_mode,
        joints: vec![
            JointTarget {
                axis: 0,
                position: 1.0,
                velocity: 2.0,
                torque: 3.0,
                max_velocity: 4.0,
                max_torque: 5.0,
            },
            JointTarget {
                axis: 1,
                position: 11.0,
                velocity: 12.0,
                torque: 13.0,
                max_velocity: 14.0,
                max_torque: 15.0,
            },
        ],
        ..command(sequence)
    }
}

fn strict_target_error(
    ingress: &mut CommandIngress,
    command: &MotionCommand,
) -> ProcBufCommandFrameError {
    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let frame = encode_command_frame(identity, command, 100).unwrap();
    let error =
        admit_procbuf_command_frame::<2, 1>(identity, ingress, &frame, Some(42), 100).unwrap_err();
    assert_eq!(ingress.audit_count(), 0);
    error
}

fn raw_command_frame(
    buffer: &TestBuf,
    command: &MotionCommand,
    layout_hash: u64,
    numeric_robot_id: u64,
    boot_id: u64,
    source_id: u64,
    sequence: u64,
) -> IpcFrame {
    IpcFrame::new(
        IpcHeader::new(
            MessageKind::Command,
            CURRENT_SCHEMA_VERSION,
            if layout_hash == 0 {
                buffer.header().layout_hash
            } else {
                layout_hash
            },
            numeric_robot_id,
            boot_id,
            source_id,
            sequence,
            100,
            0,
        ),
        &command.encode_to_vec(),
    )
    .unwrap()
}

#[test]
fn robot_identity_is_bounded_and_uses_the_same_identifier_alphabet_as_zenoh() {
    assert_eq!(RobotId::new(""), Err(RobotIdError::Empty));
    assert_eq!(
        RobotId::new("robot/01"),
        Err(RobotIdError::InvalidCharacter)
    );
    assert_eq!(RobotId::new(&"r".repeat(65)), Err(RobotIdError::TooLong));
    assert_eq!(robot_id().as_str(), "robot_01");
}

#[test]
fn shared_command_decoder_enforces_the_transport_payload_bound() {
    assert!(matches!(
        decode_motion_command(robot_id(), &[]),
        Err(CommandDecodeError::EmptyPayload)
    ));
    let oversized = vec![0; MAX_PAYLOAD_BYTES + 1];
    assert!(matches!(
        decode_motion_command(robot_id(), &oversized),
        Err(CommandDecodeError::PayloadTooLarge {
            maximum: MAX_PAYLOAD_BYTES,
            actual
        }) if actual == MAX_PAYLOAD_BYTES + 1
    ));
}

#[test]
fn state_and_event_frames_cross_real_unix_datagrams_with_bound_identity() {
    let directory = TestDirectory::new();
    let (producer, supervisor) = endpoints(&directory);
    let buffer = TestBuf::new(41, 7);
    let mut projector = ProcBufProjector::new(robot_id(), 41, 7);

    let mut state = StatePage::new(7);
    state.sequence = 9;
    state.monotonic_time_ns = 123_000;
    state.ecat_time_ns = 122_000;
    state.axes[0].position = 1.5;
    state.axes[0].error_code = 0x2310;
    state.quality.sequence = 9;
    state.quality.cyclic.known_mask = QualityFact::ALL_MASK;
    state.quality.cyclic.good_mask = QualityFact::Platform.bit() | QualityFact::Wkc.bit();
    buffer.publish_state(state).unwrap();

    let state_frame = projector.read_state_frame(&buffer, 17).unwrap().unwrap();
    producer.send(&state_frame).unwrap();
    let received = supervisor.receive().unwrap();
    assert_eq!(received.header().kind, MessageKind::State);
    assert_eq!(
        (
            received.header().robot_id,
            received.header().boot_id,
            received.header().source_id,
            received.header().sequence,
            received.header().monotonic_time_ns,
        ),
        (41, 7, 17, 9, 123_000)
    );
    assert_eq!(
        received.header().quality_bits,
        u64::from(QualityFact::ALL_MASK)
            | (u64::from(QualityFact::Platform.bit() | QualityFact::Wkc.bit()) << 16)
    );
    let decoded = RobotState::decode(received.payload()).unwrap();
    assert_eq!(
        (decoded.robot_id.as_str(), decoded.boot_id, decoded.sequence),
        ("robot_01", 7, 9)
    );
    assert_eq!(decoded.joints[0].position, 1.5);
    assert_eq!(decoded.joints[0].drive_error_code, 0x2310);
    let quality = decoded.quality.unwrap();
    assert!(quality.platform_ready && quality.wkc_valid);
    assert!(!quality.drive_ready && !quality.command_current);

    buffer
        .record_event(ProcBufEvent {
            sequence: 10,
            timestamp_ns: 124_000,
            source: 8,
            severity: ProcSeverity::Fault,
            code: 0x22,
            axis_or_device: 1,
            value: 6,
            aux: 7,
        })
        .unwrap();
    let event_frame = projector.pop_event_frame(&buffer, 17).unwrap().unwrap();
    producer.send(&event_frame).unwrap();
    let received = supervisor.receive().unwrap();
    assert_eq!(received.header().kind, MessageKind::Event);
    assert_eq!(
        (
            received.header().robot_id,
            received.header().source_id,
            received.header().sequence,
            received.header().monotonic_time_ns,
        ),
        (41, 17, 10, 124_000)
    );
    let decoded = DiagnosticEvent::decode(received.payload()).unwrap();
    assert_eq!(
        (decoded.boot_id, decoded.source, decoded.code),
        (7, 8, 0x22)
    );
}

#[test]
fn command_frame_crosses_socket_and_enters_the_existing_ingress_policy() {
    let directory = TestDirectory::new();
    let (producer, supervisor) = endpoints(&directory);
    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let frame = encode_command_frame(identity, &command(1), 100).unwrap();

    producer.send(&frame).unwrap();
    let received = supervisor.receive().unwrap();
    let mut ingress = ingress();
    let permit = admit_command_frame(identity, &mut ingress, &received, Some(42), 100).unwrap();
    assert_eq!(
        (
            permit.boot_id,
            permit.source_id,
            permit.permit_epoch,
            permit.sequence,
            permit.axis_mask,
        ),
        (7, 42, 3, 1, 0x03)
    );
    assert_eq!(ingress.audit_count(), 1);
}

#[test]
fn strict_target_mapping_preserves_modes_permit_and_empty_slots() {
    for (raw_mode, expected_mode) in [
        (8, ControlMode::Csp),
        (9, ControlMode::Csv),
        (10, ControlMode::Cst),
    ] {
        let buffer = TestBuf::new(41, 7);
        let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
        let frame = encode_command_frame(identity, &targeted_command(1, raw_mode), 100).unwrap();
        let mut ingress = ingress();
        let admitted =
            admit_procbuf_command_frame::<2, 1>(identity, &mut ingress, &frame, Some(42), 100)
                .unwrap();
        let permit = admitted.permit();
        let page = admitted.page();
        assert_eq!(page.requested_mode, expected_mode);
        assert_eq!(page.motion_enable_request, 1);
        assert_eq!(page.boot_id, permit.boot_id);
        assert_eq!(page.source_id, permit.source_id);
        assert_eq!(page.permit_epoch, permit.permit_epoch);
        assert_eq!(page.sequence, permit.sequence);
        assert_eq!(page.deadline_ns, permit.expires_at_ns);
        assert_eq!(page.permit_expires_at_ns, permit.expires_at_ns);
        assert_eq!(page.axis_mask, permit.axis_mask);
        assert_eq!(page.authority, permit.authority);
        assert_eq!(page.policy_version, permit.policy_version);
        assert_eq!(
            page.axes[0],
            JointCommand {
                position: 1.0,
                velocity: 2.0,
                torque: 3.0,
                max_velocity: 4.0,
                max_torque: 5.0,
            }
        );
        assert_eq!(page.io, [IoCommand::EMPTY; 1]);
        assert_eq!(ingress.audit_count(), 1);
    }

    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let mut partial = targeted_command(1, 8);
    partial.axis_mask = 0x01;
    partial.joints.pop();
    let frame = encode_command_frame(identity, &partial, 100).unwrap();
    let mut ingress = ingress();
    let admitted =
        admit_procbuf_command_frame::<2, 1>(identity, &mut ingress, &frame, Some(42), 100).unwrap();
    assert_eq!(admitted.page().axes[1], JointCommand::EMPTY);
    assert_eq!(admitted.page().io, [IoCommand::EMPTY; 1]);
}

#[test]
fn every_structural_target_failure_precedes_ingress_mutation() {
    let mut ingress = ingress();
    let mut invalid = targeted_command(1, 7);
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::UnsupportedMode(7))
    ));

    invalid = targeted_command(1, 8);
    invalid.axis_mask = 0x07;
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::AxisMaskOutsideCapacity {
            axis_mask: 0x07,
            capacity: 2
        })
    ));

    invalid = targeted_command(1, 8);
    invalid.joints.push(JointTarget {
        axis: 2,
        ..JointTarget::default()
    });
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::TooManyTargets {
            maximum: 2,
            actual: 3
        })
    ));

    invalid = targeted_command(1, 8);
    invalid.joints[1].axis = 2;
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::AxisOutOfRange {
            axis: 2,
            capacity: 2
        })
    ));

    invalid = targeted_command(1, 8);
    invalid.joints[1].axis = 0;
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::DuplicateAxis(0))
    ));

    invalid = targeted_command(1, 8);
    invalid.axis_mask = 0x01;
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::AxisNotSelected(1))
    ));

    invalid = targeted_command(1, 8);
    invalid.joints.pop();
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::MissingAxis(1))
    ));

    invalid = targeted_command(1, 8);
    invalid.joints[0].position = f64::NAN;
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::NonFinite {
            axis: 0,
            field: JointTargetField::Position
        })
    ));

    invalid = targeted_command(1, 8);
    invalid.joints[0].max_velocity = -1.0;
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::NegativeLimit {
            axis: 0,
            field: JointTargetField::MaxVelocity
        })
    ));

    invalid = targeted_command(1, 8);
    invalid.joints[0].max_torque = -1.0;
    assert!(matches!(
        strict_target_error(&mut ingress, &invalid),
        ProcBufCommandFrameError::Target(CommandTargetError::NegativeLimit {
            axis: 0,
            field: JointTargetField::MaxTorque
        })
    ));

    let buffer = ProcBuf::<33, 1, 1, 8>::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let frame = encode_command_frame(identity, &targeted_command(1, 8), 100).unwrap();
    assert!(matches!(
        admit_procbuf_command_frame::<33, 1>(identity, &mut ingress, &frame, Some(42), 100),
        Err(ProcBufCommandFrameError::Target(
            CommandTargetError::AxisCapacityTooLarge {
                maximum: 32,
                actual: 33
            }
        ))
    ));
    assert_eq!(ingress.audit_count(), 0);

    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let frame = encode_command_frame(identity, &targeted_command(1, 8), 100).unwrap();
    assert!(
        admit_procbuf_command_frame::<2, 1>(identity, &mut ingress, &frame, Some(42), 100).is_ok()
    );
    assert_eq!(ingress.audit_count(), 1);
}

#[test]
fn admitted_command_stays_retryable_and_bound_to_its_procbuf() {
    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let frame = encode_command_frame(identity, &targeted_command(1, 8), 100).unwrap();
    let mut ingress = ingress();
    let admitted =
        admit_procbuf_command_frame::<2, 1>(identity, &mut ingress, &frame, Some(42), 100).unwrap();
    assert_eq!(ingress.audit_count(), 1);

    let wrong_robot = TestBuf::new(42, 7);
    assert_eq!(
        admitted.publish(&wrong_robot),
        Err(CommandPublicationError::RobotMismatch {
            expected: 41,
            actual: 42
        })
    );
    let wrong_boot = TestBuf::new(41, 8);
    assert_eq!(
        admitted.publish(&wrong_boot),
        Err(CommandPublicationError::BootMismatch {
            expected: 7,
            actual: 8
        })
    );
    let wrong_layout = ProcBuf::<2, 1, 2, 8>::new(41, 7);
    assert!(matches!(
        admitted.publish(&wrong_layout),
        Err(CommandPublicationError::LayoutMismatch { .. })
    ));

    assert_eq!(admitted.publish(&buffer), Ok(1));
    assert_eq!(ingress.audit_count(), 1);
}

#[test]
fn real_unix_command_reaches_procbuf_and_lifecycle_acceptance() {
    let directory = TestDirectory::new();
    let (producer, supervisor) = endpoints(&directory);
    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let frame = encode_command_frame(identity, &targeted_command(1, 10), 100).unwrap();
    producer.send(&frame).unwrap();
    let received = supervisor.receive().unwrap();

    let mut ingress = ingress();
    let admitted =
        admit_procbuf_command_frame::<2, 1>(identity, &mut ingress, &received, Some(42), 100)
            .unwrap();
    admitted.publish(&buffer).unwrap();

    let mut floor = 0;
    let snapshot = buffer.read_command(101, &mut floor).unwrap();
    assert_eq!(snapshot.command.requested_mode, ControlMode::Cst);
    assert_eq!(snapshot.command.axes, admitted.page().axes);
    assert_eq!(snapshot.command.io, [IoCommand::EMPTY; 1]);
    let permit = motion_permit_from_command(&snapshot.command).unwrap();
    assert_eq!(permit, admitted.permit());

    let mut guard = LifecycleGuard::new(
        0,
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            exit_bad_cycles: 1,
            max_age_cycles: 1,
            stop_timeout_cycles: 10,
            stop_action: StopAction::QuickStop,
            authorized_source_id: 42,
            minimum_authority: 2,
            permit_policy_version: 9,
            allowed_axis_mask: 0x03,
        },
    );
    guard.accept_permit(permit, 101).unwrap();
    assert_eq!(guard.permit(), Some(permit));
}

#[test]
fn envelope_payload_mismatch_is_rejected_before_ingress_state_changes() {
    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let command = command(1);
    let mismatched = IpcFrame::new(
        IpcHeader::new(
            MessageKind::Command,
            CURRENT_SCHEMA_VERSION,
            buffer.header().layout_hash,
            41,
            7,
            43,
            1,
            100,
            0,
        ),
        &command.encode_to_vec(),
    )
    .unwrap();
    let mut ingress = ingress();
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &mismatched, None, 100),
        Err(CommandFrameError::SourceMismatch {
            envelope: 43,
            payload: 42
        })
    ));
    assert_eq!(ingress.audit_count(), 0);

    let valid = encode_command_frame(identity, &command, 100).unwrap();
    assert!(admit_command_frame(identity, &mut ingress, &valid, Some(42), 100).is_ok());
    assert_eq!(ingress.audit_count(), 1);
}

#[test]
fn every_command_identity_mismatch_is_rejected_before_policy() {
    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let command = command(1);
    let mut ingress = ingress();

    let wrong_layout = raw_command_frame(
        &buffer,
        &command,
        buffer.header().layout_hash ^ 1,
        41,
        7,
        42,
        1,
    );
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &wrong_layout, None, 100),
        Err(CommandFrameError::LayoutMismatch { .. })
    ));

    let wrong_robot = raw_command_frame(&buffer, &command, 0, 43, 7, 42, 1);
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &wrong_robot, None, 100),
        Err(CommandFrameError::RobotMismatch { .. })
    ));

    let wrong_boot = raw_command_frame(&buffer, &command, 0, 41, 8, 42, 1);
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &wrong_boot, None, 100),
        Err(CommandFrameError::BootMismatch { .. })
    ));

    let wrong_sequence = raw_command_frame(&buffer, &command, 0, 41, 7, 42, 2);
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &wrong_sequence, None, 100),
        Err(CommandFrameError::SequenceMismatch {
            envelope: 2,
            payload: 1
        })
    ));

    let wrong_text_robot = raw_command_frame(
        &buffer,
        &MotionCommand {
            robot_id: "robot_02".to_owned(),
            ..command.clone()
        },
        0,
        41,
        7,
        42,
        1,
    );
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &wrong_text_robot, None, 100),
        Err(CommandFrameError::Decode(CommandDecodeError::RobotMismatch))
    ));

    let valid = encode_command_frame(identity, &command, 100).unwrap();
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &valid, Some(43), 100),
        Err(CommandFrameError::Decode(
            CommandDecodeError::IdentityMismatch
        ))
    ));
    assert_eq!(ingress.audit_count(), 0);
}

#[test]
fn command_frame_rejects_wrong_kind_and_envelope_schema_before_policy() {
    let buffer = TestBuf::new(41, 7);
    let identity = CommandFrameIdentity::for_procbuf(robot_id(), &buffer).unwrap();
    let payload = command(1).encode_to_vec();
    let wrong_kind = IpcFrame::new(
        IpcHeader::new(
            MessageKind::State,
            CURRENT_SCHEMA_VERSION,
            buffer.header().layout_hash,
            41,
            7,
            42,
            1,
            100,
            0,
        ),
        &payload,
    )
    .unwrap();
    let wrong_schema = IpcFrame::new(
        IpcHeader::new(
            MessageKind::Command,
            2,
            buffer.header().layout_hash,
            41,
            7,
            42,
            1,
            100,
            0,
        ),
        &payload,
    )
    .unwrap();
    let mut ingress = ingress();
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &wrong_kind, None, 100),
        Err(CommandFrameError::WrongKind(MessageKind::State))
    ));
    assert!(matches!(
        admit_command_frame(identity, &mut ingress, &wrong_schema, None, 100),
        Err(CommandFrameError::EnvelopeSchema(_))
    ));
    assert_eq!(ingress.audit_count(), 0);
}
