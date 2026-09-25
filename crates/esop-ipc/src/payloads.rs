//! Bounded host-domain adapters between ProcBuf, Protobuf and IPC frames.

use core::str;

use esop_command_gateway::{CommandIngress, ExternalMotionCommand, IngressError};
use esop_lifecycle_guard::MotionPermit;
use esop_procbuf::{
    CommandPage, CommandPublishError as ProcBufPublishError, ControlMode, CyclicQualityMask,
    EventSeverity as ProcSeverity, HeaderError, IoCommand, JointCommand, ProcBuf, ProcBufEvent,
    ProcBufHeader, QualityFact, StateSnapshot,
};
use esop_proto::v1::{
    AxisStopEvidence, DiagnosticEvent, EventSeverity, IoState, JointState, LifecycleState,
    LifecycleSummary, MotionCommand, QualitySummary, RobotState, StopAction,
};
use esop_proto::{
    CURRENT_SCHEMA_VERSION, Message, SchemaCompatibilityError, validate_schema_version,
};

use crate::{FrameError, IpcFrame, IpcHeader, MAX_PAYLOAD_BYTES, MessageKind};

pub const MAX_ROBOT_ID_BYTES: usize = 64;
pub const QUALITY_KNOWN_SHIFT: u32 = 0;
pub const QUALITY_GOOD_SHIFT: u32 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RobotIdError {
    Empty,
    TooLong,
    InvalidCharacter,
}

/// Fixed-capacity textual robot identity used by the Protobuf contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RobotId {
    bytes: [u8; MAX_ROBOT_ID_BYTES],
    len: u8,
}

impl RobotId {
    pub fn new(value: &str) -> Result<Self, RobotIdError> {
        let value = value.as_bytes();
        if value.is_empty() {
            return Err(RobotIdError::Empty);
        }
        if value.len() > MAX_ROBOT_ID_BYTES {
            return Err(RobotIdError::TooLong);
        }
        if value.iter().any(
            |byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'),
        ) {
            return Err(RobotIdError::InvalidCharacter);
        }

        let mut bytes = [0; MAX_ROBOT_ID_BYTES];
        bytes[..value.len()].copy_from_slice(value);
        Ok(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    pub fn as_str(&self) -> &str {
        str::from_utf8(self.as_bytes()).expect("RobotId accepts ASCII bytes only")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    Header(HeaderError),
    Frame(FrameError),
    InvalidLifecycleState(u8),
    InvalidStopAction(u8),
    InvalidAxisStopEvidence(usize),
    NonFiniteJoint(usize),
    InvalidQualityMask,
    ReplayedState,
    PayloadTooLarge,
}

/// Sole supervisor-side reader for one ProcBuf state page and event ring.
pub struct ProcBufProjector {
    robot_id: RobotId,
    numeric_robot_id: u64,
    boot_id: u64,
    last_state_sequence: u64,
}

impl ProcBufProjector {
    pub const fn new(robot_id: RobotId, numeric_robot_id: u64, boot_id: u64) -> Self {
        Self {
            robot_id,
            numeric_robot_id,
            boot_id,
            last_state_sequence: 0,
        }
    }

    pub fn read_state<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &mut self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<Option<RobotState>, ProjectionError> {
        let Some(projected) = self.take_state(buffer)? else {
            return Ok(None);
        };
        self.last_state_sequence = projected.message.sequence;
        Ok(Some(projected.message))
    }

    pub fn read_state_frame<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &mut self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
        source_id: u64,
    ) -> Result<Option<IpcFrame>, ProjectionError> {
        self.validate_frame_identity(buffer.header(), source_id)?;
        let Some(projected) = self.take_state(buffer)? else {
            return Ok(None);
        };
        let header = IpcHeader::new(
            MessageKind::State,
            CURRENT_SCHEMA_VERSION,
            projected.header.layout_hash,
            projected.header.robot_id,
            projected.message.boot_id,
            source_id,
            projected.message.sequence,
            projected.message.monotonic_time_ns,
            projected.quality_bits,
        );
        let frame = encode_message(header, &projected.message)?;
        self.last_state_sequence = projected.message.sequence;
        Ok(Some(frame))
    }

    pub fn pop_event<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &mut self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<Option<DiagnosticEvent>, ProjectionError> {
        self.validate_buffer(buffer)?;
        let Some(event) = buffer.pop_event() else {
            return Ok(None);
        };
        let message = self.project_event(event);
        validate_message_size(&message)?;
        Ok(Some(message))
    }

    pub fn pop_event_frame<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &mut self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
        source_id: u64,
    ) -> Result<Option<IpcFrame>, ProjectionError> {
        let procbuf_header = buffer.header();
        self.validate_frame_identity(procbuf_header, source_id)?;
        self.validate_buffer(buffer)?;
        let Some(event) = buffer.pop_event() else {
            return Ok(None);
        };
        let message = self.project_event(event);
        let header = IpcHeader::new(
            MessageKind::Event,
            CURRENT_SCHEMA_VERSION,
            procbuf_header.layout_hash,
            procbuf_header.robot_id,
            message.boot_id,
            source_id,
            message.sequence,
            message.timestamp_ns,
            0,
        );
        encode_message(header, &message).map(Some)
    }

    fn take_state<const AXES: usize, const IO: usize, const DOMAINS: usize, const EVENTS: usize>(
        &self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<Option<ProjectedState>, ProjectionError> {
        self.validate_buffer(buffer)?;
        let Some(snapshot) = buffer.read_state() else {
            return Ok(None);
        };
        if snapshot.state.sequence <= self.last_state_sequence {
            return Err(ProjectionError::ReplayedState);
        }
        let (message, quality_bits) = self.project_state(snapshot)?;
        validate_message_size(&message)?;
        Ok(Some(ProjectedState {
            header: buffer.header(),
            message,
            quality_bits,
        }))
    }

    fn validate_buffer<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        &self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<(), ProjectionError> {
        buffer
            .validate_header(self.numeric_robot_id, self.boot_id)
            .map_err(ProjectionError::Header)
    }

    fn validate_frame_identity(
        &self,
        header: ProcBufHeader,
        source_id: u64,
    ) -> Result<(), ProjectionError> {
        let error = if header.layout_hash == 0 {
            Some(FrameError::ZeroLayoutHash)
        } else if self.numeric_robot_id == 0 {
            Some(FrameError::ZeroRobotId)
        } else if self.boot_id == 0 {
            Some(FrameError::ZeroBootId)
        } else if source_id == 0 {
            Some(FrameError::ZeroSourceId)
        } else {
            None
        };
        match error {
            Some(error) => Err(ProjectionError::Frame(error)),
            None => Ok(()),
        }
    }

    fn project_event(&self, event: ProcBufEvent) -> DiagnosticEvent {
        let severity = match event.severity {
            ProcSeverity::Info => EventSeverity::Info,
            ProcSeverity::Warning => EventSeverity::Warning,
            ProcSeverity::Error => EventSeverity::Error,
            ProcSeverity::Fault => EventSeverity::Critical,
        };
        DiagnosticEvent {
            sequence: event.sequence,
            timestamp_ns: event.timestamp_ns,
            source: u32::from(event.source),
            severity: severity as i32,
            code: u32::from(event.code),
            axis_or_device: u32::from(event.axis_or_device),
            value: event.value,
            aux: event.aux,
            boot_id: self.boot_id,
            schema_version: CURRENT_SCHEMA_VERSION,
        }
    }

    fn project_state<const AXES: usize, const IO: usize, const DOMAINS: usize>(
        &self,
        snapshot: StateSnapshot<AXES, IO, DOMAINS>,
    ) -> Result<(RobotState, u64), ProjectionError> {
        let state = snapshot.state;
        if state.boot_id != self.boot_id {
            return Err(ProjectionError::Header(HeaderError::BootIdMismatch));
        }
        let lifecycle_state = match state.lifecycle.state {
            0 => LifecycleState::Qualifying,
            1 => LifecycleState::Ready,
            2 => LifecycleState::Active,
            3 => LifecycleState::Stopping,
            4 => LifecycleState::FaultLatched,
            5 => LifecycleState::Maintenance,
            value => return Err(ProjectionError::InvalidLifecycleState(value)),
        };
        let stop_action = match state.lifecycle.stop_action {
            0 => StopAction::Hold,
            1 => StopAction::RampToZero,
            2 => StopAction::QuickStop,
            3 => StopAction::Disable,
            value => return Err(ProjectionError::InvalidStopAction(value)),
        };
        let mut axis_stops = Vec::new();
        for (axis, evidence) in state.axis_stops.iter().enumerate() {
            if evidence.request_cycle != state.sequence {
                continue;
            }
            let expected_issued = if evidence.requested_action == StopAction::QuickStop as u8 {
                StopAction::QuickStop as u8
            } else {
                StopAction::Disable as u8
            };
            if !(1..=4).contains(&evidence.requested_action)
                || (evidence.issued_action != StopAction::Unspecified as u8
                    && evidence.issued_action != expected_issued)
                || evidence.feedback_valid > 1
                || evidence.stationary > 1
                || evidence.non_enabled > 1
                || (evidence.feedback_valid == 0
                    && (evidence.feedback_cycle != 0
                        || evidence.stationary != 0
                        || evidence.non_enabled != 0))
                || (evidence.feedback_valid == 1 && evidence.feedback_cycle != state.sequence)
                || evidence.reserved != [0; 3]
            {
                return Err(ProjectionError::InvalidAxisStopEvidence(axis));
            }
            axis_stops.push(AxisStopEvidence {
                axis: axis as u32,
                requested_action: i32::from(evidence.requested_action),
                issued_action: i32::from(evidence.issued_action),
                feedback_observed: evidence.feedback_valid != 0,
                stationary: evidence.stationary != 0,
                non_enabled: evidence.non_enabled != 0,
                request_cycle: evidence.request_cycle,
                feedback_cycle: evidence.feedback_cycle,
            });
        }

        let (quality, quality_bits) = project_quality(
            state.sequence,
            state.quality.sequence,
            state.quality.cyclic,
            state.lifecycle.first_blocking_code,
        )?;

        let mut joints = Vec::with_capacity(AXES);
        for (axis, joint) in state.axes.iter().enumerate() {
            if !joint.position.is_finite()
                || !joint.velocity.is_finite()
                || !joint.torque.is_finite()
                || !joint.following_error.is_finite()
            {
                return Err(ProjectionError::NonFiniteJoint(axis));
            }
            joints.push(JointState {
                axis: axis as u32,
                position: joint.position,
                velocity: joint.velocity,
                torque: joint.torque,
                following_error: joint.following_error,
                statusword: u32::from(joint.statusword),
                controlword: u32::from(joint.controlword),
                drive_state: u32::from(joint.drive_state),
                actual_mode: u32::from(joint.actual_mode),
                quality: u32::from(joint.quality),
                drive_error_code: u32::from(joint.error_code),
            });
        }
        let io = state
            .io
            .iter()
            .enumerate()
            .map(|(channel, value)| IoState {
                channel: channel as u32,
                input_bits: value.input_bits,
                quality: value.quality,
            })
            .collect();
        Ok((
            RobotState {
                robot_id: self.robot_id.as_str().to_owned(),
                boot_id: state.boot_id,
                sequence: state.sequence,
                monotonic_time_ns: state.monotonic_time_ns,
                ecat_time_ns: state.ecat_time_ns,
                lifecycle: Some(LifecycleSummary {
                    state: lifecycle_state as i32,
                    stop_action: stop_action as i32,
                    required_gate_mask: u32::from(state.lifecycle.required_gate_mask),
                    valid_gate_mask: u32::from(state.lifecycle.valid_gate_mask),
                    qualified_gate_mask: u32::from(state.lifecycle.qualified_gate_mask),
                    ready_gate_mask: u32::from(state.lifecycle.ready_gate_mask),
                    first_blocking_code: state.lifecycle.first_blocking_code,
                    latched_fault_code: state.lifecycle.latched_fault_code,
                    motion_permit_current: state.lifecycle.motion_permit != 0,
                    permit_epoch: state.lifecycle.permit_epoch,
                    permit_expires_at_ns: state.lifecycle.permit_expires_at_ns,
                    transition_sequence: state.lifecycle.transition_sequence,
                    transition_cycle: state.lifecycle.transition_cycle,
                    recovery_count: state.lifecycle.recovery_count,
                    permit_audit_sequence: state.lifecycle.permit_audit_sequence,
                    axis_stops,
                }),
                quality,
                joints,
                io,
                events: Vec::new(),
                schema_version: CURRENT_SCHEMA_VERSION,
            },
            quality_bits,
        ))
    }
}

struct ProjectedState {
    header: ProcBufHeader,
    message: RobotState,
    quality_bits: u64,
}

fn project_quality(
    state_sequence: u64,
    quality_sequence: u64,
    facts: CyclicQualityMask,
    first_fault_code: u32,
) -> Result<(Option<QualitySummary>, u64), ProjectionError> {
    if quality_sequence != state_sequence {
        return Ok((None, 0));
    }
    if !facts.well_formed() {
        return Err(ProjectionError::InvalidQualityMask);
    }
    let quality_bits = u64::from(facts.known_mask) << QUALITY_KNOWN_SHIFT
        | u64::from(facts.good_mask) << QUALITY_GOOD_SHIFT;
    let quality = facts.complete().then(|| QualitySummary {
        platform_ready: facts.good(QualityFact::Platform),
        configuration_ready: facts.good(QualityFact::Configuration),
        topology_valid: facts.good(QualityFact::Topology),
        distributed_clock_locked: facts.good(QualityFact::DistributedClock),
        drive_ready: facts.good(QualityFact::Drive),
        domain_valid: facts.good(QualityFact::Domain),
        wkc_valid: facts.good(QualityFact::Wkc),
        command_current: facts.good(QualityFact::Command),
        supervisor_healthy: facts.good(QualityFact::Supervisor),
        external_safety_clear: facts.good(QualityFact::ExternalSafety),
        cycle_within_budget: facts.good(QualityFact::CycleBudget),
        first_fault_code,
    });
    Ok((quality, quality_bits))
}

fn validate_message_size(message: &impl Message) -> Result<(), ProjectionError> {
    if message.encoded_len() > MAX_PAYLOAD_BYTES {
        Err(ProjectionError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

fn encode_message(header: IpcHeader, message: &impl Message) -> Result<IpcFrame, ProjectionError> {
    validate_message_size(message)?;
    let payload = message.encode_to_vec();
    IpcFrame::new(header, &payload).map_err(ProjectionError::Frame)
}

#[derive(Debug)]
pub enum CommandDecodeError {
    EmptyPayload,
    PayloadTooLarge { maximum: usize, actual: usize },
    Decode(esop_proto::DecodeError),
    RobotMismatch,
    Schema(SchemaCompatibilityError),
    AuthorityOutOfRange,
    IdentityMismatch,
}

impl From<esop_proto::DecodeError> for CommandDecodeError {
    fn from(error: esop_proto::DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// Decode only the policy-bearing fields shared by hosted transports.
pub fn decode_motion_command(
    robot_id: RobotId,
    payload: &[u8],
) -> Result<ExternalMotionCommand, CommandDecodeError> {
    let command = decode_motion_command_message(payload)?;
    map_motion_command(robot_id, &command)
}

fn decode_motion_command_message(payload: &[u8]) -> Result<MotionCommand, CommandDecodeError> {
    if payload.is_empty() {
        return Err(CommandDecodeError::EmptyPayload);
    }
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(CommandDecodeError::PayloadTooLarge {
            maximum: MAX_PAYLOAD_BYTES,
            actual: payload.len(),
        });
    }
    MotionCommand::decode(payload).map_err(CommandDecodeError::Decode)
}

pub fn validate_authenticated_source(
    command: ExternalMotionCommand,
    authenticated_source_id: u64,
) -> Result<ExternalMotionCommand, CommandDecodeError> {
    if command.source_id == authenticated_source_id {
        Ok(command)
    } else {
        Err(CommandDecodeError::IdentityMismatch)
    }
}

fn map_motion_command(
    robot_id: RobotId,
    command: &MotionCommand,
) -> Result<ExternalMotionCommand, CommandDecodeError> {
    validate_schema_version(command.schema_version).map_err(CommandDecodeError::Schema)?;
    if command.robot_id.as_bytes() != robot_id.as_bytes() {
        return Err(CommandDecodeError::RobotMismatch);
    }
    let authority =
        u8::try_from(command.authority).map_err(|_| CommandDecodeError::AuthorityOutOfRange)?;
    Ok(ExternalMotionCommand {
        boot_id: command.boot_id,
        source_id: command.source_id,
        permit_epoch: command.permit_epoch,
        sequence: command.sequence,
        deadline_ns: command.deadline_ns,
        axis_mask: command.axis_mask,
        authority,
        reserved: [0; 3],
        policy_version: command.policy_version,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JointTargetField {
    Position,
    Velocity,
    Torque,
    MaxVelocity,
    MaxTorque,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandTargetError {
    AxisCapacityTooLarge { maximum: usize, actual: usize },
    UnsupportedMode(u32),
    AxisMaskOutsideCapacity { axis_mask: u32, capacity: usize },
    TooManyTargets { maximum: usize, actual: usize },
    AxisOutOfRange { axis: u32, capacity: usize },
    DuplicateAxis(u32),
    AxisNotSelected(u32),
    MissingAxis(u32),
    NonFinite { axis: u32, field: JointTargetField },
    NegativeLimit { axis: u32, field: JointTargetField },
}

#[derive(Debug)]
pub enum CommandPrepareError {
    Identity(CommandIdentityError),
    Decode(CommandDecodeError),
    BootMismatch { expected: u64, actual: u64 },
    Target(CommandTargetError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPublicationError {
    RobotMismatch { expected: u64, actual: u64 },
    BootMismatch { expected: u64, actual: u64 },
    LayoutMismatch { expected: u64, actual: u64 },
    Header(HeaderError),
    Publish(ProcBufPublishError),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedProcBufCommand<const AXES: usize, const IO: usize> {
    identity: CommandFrameIdentity,
    command: ExternalMotionCommand,
    requested_mode: ControlMode,
    axes: [JointCommand; AXES],
    io: [IoCommand; IO],
}

impl<const AXES: usize, const IO: usize> PreparedProcBufCommand<AXES, IO> {
    pub const fn policy_command(&self) -> ExternalMotionCommand {
        self.command
    }

    pub fn admit(
        self,
        ingress: &mut CommandIngress,
        now_ns: u64,
    ) -> Result<AdmittedProcBufCommand<AXES, IO>, IngressError> {
        let permit = ingress.admit(self.command, now_ns)?;
        let page = CommandPage {
            boot_id: permit.boot_id,
            sequence: permit.sequence,
            deadline_ns: permit.expires_at_ns,
            source_id: permit.source_id,
            permit_epoch: permit.permit_epoch,
            permit_expires_at_ns: permit.expires_at_ns,
            axis_mask: permit.axis_mask,
            requested_mode: self.requested_mode,
            motion_enable_request: 1,
            authority: permit.authority,
            reserved: [0; 3],
            policy_version: permit.policy_version,
            axes: self.axes,
            io: self.io,
        };
        Ok(AdmittedProcBufCommand {
            identity: self.identity,
            permit,
            page,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdmittedProcBufCommand<const AXES: usize, const IO: usize> {
    identity: CommandFrameIdentity,
    permit: MotionPermit,
    page: CommandPage<AXES, IO>,
}

impl<const AXES: usize, const IO: usize> AdmittedProcBufCommand<AXES, IO> {
    pub const fn permit(&self) -> MotionPermit {
        self.permit
    }

    pub const fn page(&self) -> &CommandPage<AXES, IO> {
        &self.page
    }

    pub fn publish<const DOMAINS: usize, const EVENTS: usize>(
        &self,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<u64, CommandPublicationError> {
        let header = buffer.header();
        if header.robot_id != self.identity.numeric_robot_id {
            return Err(CommandPublicationError::RobotMismatch {
                expected: self.identity.numeric_robot_id,
                actual: header.robot_id,
            });
        }
        if header.boot_id != self.identity.boot_id {
            return Err(CommandPublicationError::BootMismatch {
                expected: self.identity.boot_id,
                actual: header.boot_id,
            });
        }
        if header.layout_hash != self.identity.layout_hash {
            return Err(CommandPublicationError::LayoutMismatch {
                expected: self.identity.layout_hash,
                actual: header.layout_hash,
            });
        }
        buffer
            .validate_header(self.identity.numeric_robot_id, self.identity.boot_id)
            .map_err(CommandPublicationError::Header)?;
        buffer
            .publish_command(self.page)
            .map_err(CommandPublicationError::Publish)
    }
}

pub fn prepare_motion_command_for_procbuf<
    const AXES: usize,
    const IO: usize,
    const DOMAINS: usize,
    const EVENTS: usize,
>(
    robot_id: RobotId,
    buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    payload: &[u8],
    authenticated_source_id: Option<u64>,
) -> Result<PreparedProcBufCommand<AXES, IO>, CommandPrepareError> {
    let identity = CommandFrameIdentity::for_procbuf(robot_id, buffer)
        .map_err(CommandPrepareError::Identity)?;
    let message = decode_motion_command_message(payload).map_err(CommandPrepareError::Decode)?;
    let command = map_motion_command(robot_id, &message).map_err(CommandPrepareError::Decode)?;
    if command.boot_id != identity.boot_id {
        return Err(CommandPrepareError::BootMismatch {
            expected: identity.boot_id,
            actual: command.boot_id,
        });
    }
    let command = match authenticated_source_id {
        Some(source_id) => validate_authenticated_source(command, source_id)
            .map_err(CommandPrepareError::Decode)?,
        None => command,
    };
    prepare_decoded_command(identity, command, &message).map_err(CommandPrepareError::Target)
}

fn prepare_decoded_command<const AXES: usize, const IO: usize>(
    identity: CommandFrameIdentity,
    command: ExternalMotionCommand,
    message: &MotionCommand,
) -> Result<PreparedProcBufCommand<AXES, IO>, CommandTargetError> {
    if AXES > 32 {
        return Err(CommandTargetError::AxisCapacityTooLarge {
            maximum: 32,
            actual: AXES,
        });
    }
    let requested_mode = match message.requested_mode {
        8 => ControlMode::Csp,
        9 => ControlMode::Csv,
        10 => ControlMode::Cst,
        mode => return Err(CommandTargetError::UnsupportedMode(mode)),
    };
    let capacity_mask = if AXES == 32 {
        u32::MAX
    } else {
        (1u32 << AXES) - 1
    };
    if command.axis_mask & !capacity_mask != 0 {
        return Err(CommandTargetError::AxisMaskOutsideCapacity {
            axis_mask: command.axis_mask,
            capacity: AXES,
        });
    }
    if message.joints.len() > AXES {
        return Err(CommandTargetError::TooManyTargets {
            maximum: AXES,
            actual: message.joints.len(),
        });
    }

    let mut axes = [JointCommand::EMPTY; AXES];
    let mut seen_mask = 0u32;
    for target in &message.joints {
        let index =
            usize::try_from(target.axis).map_err(|_| CommandTargetError::AxisOutOfRange {
                axis: target.axis,
                capacity: AXES,
            })?;
        if index >= AXES {
            return Err(CommandTargetError::AxisOutOfRange {
                axis: target.axis,
                capacity: AXES,
            });
        }
        let bit = 1u32 << index;
        if seen_mask & bit != 0 {
            return Err(CommandTargetError::DuplicateAxis(target.axis));
        }
        if command.axis_mask & bit == 0 {
            return Err(CommandTargetError::AxisNotSelected(target.axis));
        }
        validate_joint_target(target.axis, target)?;
        seen_mask |= bit;
        axes[index] = JointCommand {
            position: target.position,
            velocity: target.velocity,
            torque: target.torque,
            max_velocity: target.max_velocity,
            max_torque: target.max_torque,
        };
    }
    let missing_mask = command.axis_mask & !seen_mask;
    if missing_mask != 0 {
        return Err(CommandTargetError::MissingAxis(
            missing_mask.trailing_zeros(),
        ));
    }

    Ok(PreparedProcBufCommand {
        identity,
        command,
        requested_mode,
        axes,
        io: [IoCommand::EMPTY; IO],
    })
}

fn validate_joint_target(
    axis: u32,
    target: &esop_proto::v1::JointTarget,
) -> Result<(), CommandTargetError> {
    for (field, value) in [
        (JointTargetField::Position, target.position),
        (JointTargetField::Velocity, target.velocity),
        (JointTargetField::Torque, target.torque),
        (JointTargetField::MaxVelocity, target.max_velocity),
        (JointTargetField::MaxTorque, target.max_torque),
    ] {
        if !value.is_finite() {
            return Err(CommandTargetError::NonFinite { axis, field });
        }
    }
    if target.max_velocity < 0.0 {
        return Err(CommandTargetError::NegativeLimit {
            axis,
            field: JointTargetField::MaxVelocity,
        });
    }
    if target.max_torque < 0.0 {
        return Err(CommandTargetError::NegativeLimit {
            axis,
            field: JointTargetField::MaxTorque,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandIdentityError {
    ZeroRobotId,
    ZeroBootId,
    ZeroLayoutHash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandFrameIdentity {
    robot_id: RobotId,
    numeric_robot_id: u64,
    boot_id: u64,
    layout_hash: u64,
}

impl CommandFrameIdentity {
    pub fn new(
        robot_id: RobotId,
        numeric_robot_id: u64,
        boot_id: u64,
        layout_hash: u64,
    ) -> Result<Self, CommandIdentityError> {
        if numeric_robot_id == 0 {
            return Err(CommandIdentityError::ZeroRobotId);
        }
        if boot_id == 0 {
            return Err(CommandIdentityError::ZeroBootId);
        }
        if layout_hash == 0 {
            return Err(CommandIdentityError::ZeroLayoutHash);
        }
        Ok(Self {
            robot_id,
            numeric_robot_id,
            boot_id,
            layout_hash,
        })
    }

    pub fn for_procbuf<
        const AXES: usize,
        const IO: usize,
        const DOMAINS: usize,
        const EVENTS: usize,
    >(
        robot_id: RobotId,
        buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    ) -> Result<Self, CommandIdentityError> {
        let header = buffer.header();
        Self::new(
            robot_id,
            header.robot_id,
            header.boot_id,
            header.layout_hash,
        )
    }
}

#[derive(Debug)]
pub enum CommandFrameError {
    Frame(FrameError),
    Decode(CommandDecodeError),
    WrongKind(MessageKind),
    EnvelopeSchema(SchemaCompatibilityError),
    LayoutMismatch { expected: u64, actual: u64 },
    RobotMismatch { expected: u64, actual: u64 },
    BootMismatch { expected: u64, actual: u64 },
    SourceMismatch { envelope: u64, payload: u64 },
    SequenceMismatch { envelope: u64, payload: u64 },
    Policy(IngressError),
}

#[derive(Debug)]
pub enum ProcBufCommandFrameError {
    Frame(CommandFrameError),
    Target(CommandTargetError),
    Policy(IngressError),
}

pub fn encode_command_frame(
    identity: CommandFrameIdentity,
    command: &MotionCommand,
    monotonic_time_ns: u64,
) -> Result<IpcFrame, CommandFrameError> {
    let mapped =
        map_motion_command(identity.robot_id, command).map_err(CommandFrameError::Decode)?;
    if mapped.boot_id != identity.boot_id {
        return Err(CommandFrameError::BootMismatch {
            expected: identity.boot_id,
            actual: mapped.boot_id,
        });
    }
    let payload_len = command.encoded_len();
    if payload_len > MAX_PAYLOAD_BYTES {
        return Err(CommandFrameError::Decode(
            CommandDecodeError::PayloadTooLarge {
                maximum: MAX_PAYLOAD_BYTES,
                actual: payload_len,
            },
        ));
    }
    let payload = command.encode_to_vec();
    let header = IpcHeader::new(
        MessageKind::Command,
        CURRENT_SCHEMA_VERSION,
        identity.layout_hash,
        identity.numeric_robot_id,
        mapped.boot_id,
        mapped.source_id,
        mapped.sequence,
        monotonic_time_ns,
        0,
    );
    IpcFrame::new(header, &payload).map_err(CommandFrameError::Frame)
}

pub fn decode_command_frame(
    identity: CommandFrameIdentity,
    frame: &IpcFrame,
    authenticated_source_id: Option<u64>,
) -> Result<ExternalMotionCommand, CommandFrameError> {
    decode_command_frame_message(identity, frame, authenticated_source_id)
        .map(|(_, command)| command)
}

fn decode_command_frame_message(
    identity: CommandFrameIdentity,
    frame: &IpcFrame,
    authenticated_source_id: Option<u64>,
) -> Result<(MotionCommand, ExternalMotionCommand), CommandFrameError> {
    let header = frame.header();
    if header.kind != MessageKind::Command {
        return Err(CommandFrameError::WrongKind(header.kind));
    }
    validate_schema_version(header.payload_schema_version)
        .map_err(CommandFrameError::EnvelopeSchema)?;
    if header.layout_hash != identity.layout_hash {
        return Err(CommandFrameError::LayoutMismatch {
            expected: identity.layout_hash,
            actual: header.layout_hash,
        });
    }
    if header.robot_id != identity.numeric_robot_id {
        return Err(CommandFrameError::RobotMismatch {
            expected: identity.numeric_robot_id,
            actual: header.robot_id,
        });
    }
    if header.boot_id != identity.boot_id {
        return Err(CommandFrameError::BootMismatch {
            expected: identity.boot_id,
            actual: header.boot_id,
        });
    }
    let message =
        decode_motion_command_message(frame.payload()).map_err(CommandFrameError::Decode)?;
    let command =
        map_motion_command(identity.robot_id, &message).map_err(CommandFrameError::Decode)?;
    if command.boot_id != header.boot_id {
        return Err(CommandFrameError::BootMismatch {
            expected: header.boot_id,
            actual: command.boot_id,
        });
    }
    if command.source_id != header.source_id {
        return Err(CommandFrameError::SourceMismatch {
            envelope: header.source_id,
            payload: command.source_id,
        });
    }
    if command.sequence != header.sequence {
        return Err(CommandFrameError::SequenceMismatch {
            envelope: header.sequence,
            payload: command.sequence,
        });
    }
    match authenticated_source_id {
        Some(source_id) => validate_authenticated_source(command, source_id)
            .map(|command| (message, command))
            .map_err(CommandFrameError::Decode),
        None => Ok((message, command)),
    }
}

pub fn prepare_procbuf_command_frame<const AXES: usize, const IO: usize>(
    identity: CommandFrameIdentity,
    frame: &IpcFrame,
    authenticated_source_id: Option<u64>,
) -> Result<PreparedProcBufCommand<AXES, IO>, ProcBufCommandFrameError> {
    let (message, command) = decode_command_frame_message(identity, frame, authenticated_source_id)
        .map_err(ProcBufCommandFrameError::Frame)?;
    prepare_decoded_command(identity, command, &message).map_err(ProcBufCommandFrameError::Target)
}

pub fn admit_procbuf_command_frame<const AXES: usize, const IO: usize>(
    identity: CommandFrameIdentity,
    ingress: &mut CommandIngress,
    frame: &IpcFrame,
    authenticated_source_id: Option<u64>,
    now_ns: u64,
) -> Result<AdmittedProcBufCommand<AXES, IO>, ProcBufCommandFrameError> {
    prepare_procbuf_command_frame(identity, frame, authenticated_source_id)?
        .admit(ingress, now_ns)
        .map_err(ProcBufCommandFrameError::Policy)
}

pub fn admit_command_frame(
    identity: CommandFrameIdentity,
    ingress: &mut CommandIngress,
    frame: &IpcFrame,
    authenticated_source_id: Option<u64>,
    now_ns: u64,
) -> Result<MotionPermit, CommandFrameError> {
    let command = decode_command_frame(identity, frame, authenticated_source_id)?;
    ingress
        .admit(command, now_ns)
        .map_err(CommandFrameError::Policy)
}
