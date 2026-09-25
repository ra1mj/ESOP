//! Fixed-size projection for the optional ProcBuf ABI boundary.

use crate::{CyclicQuality, LifecycleSnapshot, MotionPermit};
use esop_procbuf::{CommandPage, CyclicQualityMask, LifecycleSummary, QualityFact, StatePage};

use crate::{LifecycleGuard, LifecycleState, MAX_MOTION_AXES, STOP_TIMEOUT_FAULT_CODE, StopAction};
use esop_procbuf::{EventPushError, EventSeverity, HeaderError, ProcBuf, ProcBufEvent};

#[cfg(feature = "cia402")]
use crate::{AxisCycleDecision, AxisDirective, StopFeedback};
#[cfg(feature = "cia402")]
use esop_procbuf::{AxisStopEvidence, ControlMode, JointStateQuality};
#[cfg(feature = "cia402")]
use esop_profile_cia402::{
    CONTROLWORD_DISABLE_VOLTAGE, CONTROLWORD_QUICK_STOP, Cia402Output, Cia402PdoError,
    Cia402PdoField, Cia402PdoMap, Cia402Target, CyclicLimits, DriveState, OperatingMode,
};

#[cfg(all(feature = "cia402", feature = "ethercat"))]
use crate::ethercat::ControlledStopFrameReport;
#[cfg(all(feature = "cia402", feature = "ethercat"))]
use esop_profile_cia402::CONTROLWORD_ENABLE_OPERATION;

#[cfg(feature = "ethercat")]
use crate::ethercat::{
    OtherCycleFacts, ScheduledDomainQuality, cyclic_quality_from_domains,
    cyclic_quality_from_schedule,
};
#[cfg(feature = "ethercat")]
use esop_ethercat_core::{
    CycleReport, DcCyclicSync, DomainQuality as EthercatDomainQuality, ScheduleTable,
};

/// Reconstruct the exact admitted permit carried by a validated ProcBuf
/// command. Call this only after `ProcBuf::read_command`; disabled pages do not
/// grant a permit.
pub const fn motion_permit_from_command<const AXES: usize, const IO: usize>(
    command: &CommandPage<AXES, IO>,
) -> Option<MotionPermit> {
    if command.motion_enable_request == 0 {
        return None;
    }
    Some(MotionPermit {
        boot_id: command.boot_id,
        source_id: command.source_id,
        permit_epoch: command.permit_epoch,
        sequence: command.sequence,
        expires_at_ns: command.permit_expires_at_ns,
        axis_mask: command.axis_mask,
        authority: command.authority,
        reserved: [0; 3],
        policy_version: command.policy_version,
    })
}

/// Frozen product conversion and mechanical policy for one CiA 402 axis.
/// Signed scales carry the product's axis direction. The raw position offset
/// applies only to absolute CSP positions.
#[cfg(feature = "cia402")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cia402AxisCommandPolicy {
    pub position_units_per_radian: f64,
    pub velocity_units_per_radian_per_second: f64,
    pub torque_units_per_newton_metre: f64,
    pub position_offset: i32,
    pub min_position_radians: f64,
    pub max_position_radians: f64,
    pub max_velocity_radians_per_second: f64,
    pub max_torque_newton_metres: f64,
    pub max_position_step_radians: f64,
}

#[cfg(feature = "cia402")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cia402AxisCommandPolicyError {
    NonFiniteScale,
    ZeroScale,
    NonFinitePositionBounds,
    ReversedPositionBounds,
    NonFiniteLimit,
    NegativeLimit,
}

#[cfg(feature = "cia402")]
impl Cia402AxisCommandPolicy {
    pub fn validate(self) -> Result<(), Cia402AxisCommandPolicyError> {
        let scales = [
            self.position_units_per_radian,
            self.velocity_units_per_radian_per_second,
            self.torque_units_per_newton_metre,
        ];
        if scales.iter().any(|value| !value.is_finite()) {
            return Err(Cia402AxisCommandPolicyError::NonFiniteScale);
        }
        if scales.contains(&0.0) {
            return Err(Cia402AxisCommandPolicyError::ZeroScale);
        }
        if !self.min_position_radians.is_finite() || !self.max_position_radians.is_finite() {
            return Err(Cia402AxisCommandPolicyError::NonFinitePositionBounds);
        }
        if self.min_position_radians > self.max_position_radians {
            return Err(Cia402AxisCommandPolicyError::ReversedPositionBounds);
        }
        let limits = [
            self.max_velocity_radians_per_second,
            self.max_torque_newton_metres,
            self.max_position_step_radians,
        ];
        if limits.iter().any(|value| !value.is_finite()) {
            return Err(Cia402AxisCommandPolicyError::NonFiniteLimit);
        }
        if limits.iter().any(|value| *value < 0.0) {
            return Err(Cia402AxisCommandPolicyError::NegativeLimit);
        }
        Ok(())
    }
}

#[cfg(feature = "cia402")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcBufCia402CommandError {
    InvalidCommand,
    MotionDisabled,
    Expired,
    AxisCapacityExceeded,
    PermitMismatch,
    UnsupportedMode,
    ModeMismatch(usize),
    InvalidCyclePeriod,
    InvalidPolicy(usize, Cia402AxisCommandPolicyError),
    VelocityCapExceeded(usize),
    TorqueCapExceeded(usize),
    PositionOutOfBounds(usize),
    VelocityOutOfBounds(usize),
    TorqueOutOfBounds(usize),
    RawTargetOverflow(usize),
    RawLimitOverflow(usize),
}

/// Transactionally prepared, fixed-size raw command. Identity is retained so
/// the EtherCAT execution boundary can reject a superseded lifecycle permit.
#[cfg(feature = "cia402")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedCia402Command<const AXES: usize> {
    permit: MotionPermit,
    command_sequence: u64,
    deadline_ns: u64,
    axis_mask: u32,
    mode: OperatingMode,
    targets: [Option<Cia402Target>; AXES],
    limits: [CyclicLimits; AXES],
}

#[cfg(feature = "cia402")]
impl<const AXES: usize> PreparedCia402Command<AXES> {
    pub const fn permit(&self) -> MotionPermit {
        self.permit
    }

    pub const fn command_sequence(&self) -> u64 {
        self.command_sequence
    }

    pub const fn deadline_ns(&self) -> u64 {
        self.deadline_ns
    }

    pub const fn axis_mask(&self) -> u32 {
        self.axis_mask
    }

    pub const fn mode(&self) -> OperatingMode {
        self.mode
    }

    pub const fn targets(&self) -> &[Option<Cia402Target>; AXES] {
        &self.targets
    }

    pub const fn limits(&self) -> &[CyclicLimits; AXES] {
        &self.limits
    }
}

/// Validate and stage one SI-valued ProcBuf command without mutating the
/// lifecycle guard, setpoint guards, process image, frame pool, or port.
#[cfg(feature = "cia402")]
pub fn prepare_cia402_command<const AXES: usize, const IO: usize>(
    command: &CommandPage<AXES, IO>,
    guard: &LifecycleGuard,
    modes: &[OperatingMode; AXES],
    policies: &[Cia402AxisCommandPolicy; AXES],
    cycle_period_ns: u64,
    now_ns: u64,
) -> Result<PreparedCia402Command<AXES>, ProcBufCia402CommandError> {
    if !command.is_well_formed() {
        return Err(ProcBufCia402CommandError::InvalidCommand);
    }
    if AXES == 0 || AXES > MAX_MOTION_AXES {
        return Err(ProcBufCia402CommandError::AxisCapacityExceeded);
    }
    if cycle_period_ns == 0 {
        return Err(ProcBufCia402CommandError::InvalidCyclePeriod);
    }
    if command.motion_enable_request == 0 {
        return Err(ProcBufCia402CommandError::MotionDisabled);
    }
    if command.deadline_ns <= now_ns || command.permit_expires_at_ns <= now_ns {
        return Err(ProcBufCia402CommandError::Expired);
    }
    let Some(permit) = motion_permit_from_command(command) else {
        return Err(ProcBufCia402CommandError::MotionDisabled);
    };
    if guard.permit() != Some(permit) {
        return Err(ProcBufCia402CommandError::PermitMismatch);
    }
    let mode = match command.requested_mode {
        ControlMode::Csp => OperatingMode::Csp,
        ControlMode::Csv => OperatingMode::Csv,
        ControlMode::Cst => OperatingMode::Cst,
        ControlMode::Unknown => return Err(ProcBufCia402CommandError::UnsupportedMode),
    };
    let cycle_seconds = cycle_period_ns as f64 / 1_000_000_000.0;
    let mut targets = [None; AXES];
    let mut limits = [CyclicLimits {
        max_position_step: 0.0,
        max_velocity: 0.0,
        max_torque: 0.0,
    }; AXES];

    for axis in 0..AXES {
        if command.axis_mask & (1u32 << axis) == 0 {
            continue;
        }
        if modes[axis] != mode {
            return Err(ProcBufCia402CommandError::ModeMismatch(axis));
        }
        let policy = policies[axis];
        policy
            .validate()
            .map_err(|error| ProcBufCia402CommandError::InvalidPolicy(axis, error))?;
        let joint = command.axes[axis];
        if joint.max_velocity > policy.max_velocity_radians_per_second {
            return Err(ProcBufCia402CommandError::VelocityCapExceeded(axis));
        }
        if joint.max_torque > policy.max_torque_newton_metres {
            return Err(ProcBufCia402CommandError::TorqueCapExceeded(axis));
        }

        targets[axis] = Some(match mode {
            OperatingMode::Csp => {
                if joint.position < policy.min_position_radians
                    || joint.position > policy.max_position_radians
                {
                    return Err(ProcBufCia402CommandError::PositionOutOfBounds(axis));
                }
                Cia402Target::Position(
                    rounded_i32(
                        joint.position * policy.position_units_per_radian
                            + f64::from(policy.position_offset),
                    )
                    .ok_or(ProcBufCia402CommandError::RawTargetOverflow(axis))?,
                )
            }
            OperatingMode::Csv => {
                if joint.velocity.abs() > joint.max_velocity
                    || joint.velocity.abs() > policy.max_velocity_radians_per_second
                {
                    return Err(ProcBufCia402CommandError::VelocityOutOfBounds(axis));
                }
                Cia402Target::Velocity(
                    rounded_i32(joint.velocity * policy.velocity_units_per_radian_per_second)
                        .ok_or(ProcBufCia402CommandError::RawTargetOverflow(axis))?,
                )
            }
            OperatingMode::Cst => {
                if joint.torque.abs() > joint.max_torque
                    || joint.torque.abs() > policy.max_torque_newton_metres
                {
                    return Err(ProcBufCia402CommandError::TorqueOutOfBounds(axis));
                }
                Cia402Target::Torque(
                    rounded_i16(joint.torque * policy.torque_units_per_newton_metre)
                        .ok_or(ProcBufCia402CommandError::RawTargetOverflow(axis))?,
                )
            }
            OperatingMode::Unknown => unreachable!(),
        });

        let position_step = policy
            .max_position_step_radians
            .min(joint.max_velocity * cycle_seconds)
            * policy.position_units_per_radian.abs();
        let velocity_limit = joint.max_velocity * policy.velocity_units_per_radian_per_second.abs();
        let torque_limit = joint.max_torque * policy.torque_units_per_newton_metre.abs();
        limits[axis] = CyclicLimits {
            max_position_step: floored_limit(position_step, f64::from(i32::MAX))
                .ok_or(ProcBufCia402CommandError::RawLimitOverflow(axis))?,
            max_velocity: floored_limit(velocity_limit, f64::from(i32::MAX))
                .ok_or(ProcBufCia402CommandError::RawLimitOverflow(axis))?,
            max_torque: floored_limit(torque_limit, f64::from(i16::MAX))
                .ok_or(ProcBufCia402CommandError::RawLimitOverflow(axis))?,
        };
    }

    Ok(PreparedCia402Command {
        permit,
        command_sequence: command.sequence,
        deadline_ns: command.deadline_ns,
        axis_mask: command.axis_mask,
        mode,
        targets,
        limits,
    })
}

#[cfg(feature = "cia402")]
fn rounded_i32(value: f64) -> Option<i32> {
    if !value.is_finite() {
        return None;
    }
    let rounded = if value >= 0.0 {
        (value + 0.5) as i64
    } else {
        (value - 0.5) as i64
    };
    i32::try_from(rounded).ok()
}

#[cfg(feature = "cia402")]
fn rounded_i16(value: f64) -> Option<i16> {
    if !value.is_finite() {
        return None;
    }
    let rounded = if value >= 0.0 {
        (value + 0.5) as i64
    } else {
        (value - 0.5) as i64
    };
    i16::try_from(rounded).ok()
}

#[cfg(feature = "cia402")]
fn floored_limit(value: f64, maximum: f64) -> Option<f64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let floored = (value as u64) as f64;
    (floored <= maximum).then_some(floored)
}

#[cfg(feature = "cia402")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cia402FeedbackError {
    AxisCapacityExceeded,
    InvalidPolicy(usize, Cia402AxisCommandPolicyError),
    Pdo(usize, Cia402PdoError),
    NonFiniteValue(usize, Cia402PdoField),
}

/// Transactionally project verified CiA 402 inputs and accepted outputs into
/// the State page. Unverified input retains old values but clears freshness;
/// accepted controlwords remain independently observable as TX evidence.
#[cfg(feature = "cia402")]
pub fn cia402_feedback_to_procbuf<const AXES: usize, const IO: usize, const DOMAINS: usize>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    image: &[u8],
    maps: &[Cia402PdoMap; AXES],
    modes: &[OperatingMode; AXES],
    policies: &[Cia402AxisCommandPolicy; AXES],
    input_current: bool,
    accepted_outputs: Option<&[Cia402Output; AXES]>,
) -> Result<(), Cia402FeedbackError> {
    if AXES > MAX_MOTION_AXES {
        return Err(Cia402FeedbackError::AxisCapacityExceeded);
    }

    let mut axes = state.axes;
    for (axis, joint) in axes.iter_mut().enumerate() {
        joint.quality = 0;
        if input_current {
            let policy = policies[axis];
            policy
                .validate()
                .map_err(|error| Cia402FeedbackError::InvalidPolicy(axis, error))?;
            let inputs = maps[axis]
                .read_inputs_for(image, modes[axis])
                .map_err(|error| Cia402FeedbackError::Pdo(axis, error))?;
            let drive_state = DriveState::from_statusword(inputs.statusword);

            joint.statusword = inputs.statusword;
            joint.error_code = inputs.error_code;
            joint.drive_state = drive_state as u8;
            joint.actual_mode = inputs.actual_mode as u8;
            joint.quality = JointStateQuality::CURRENT_INPUT;
            if inputs.actual_mode == modes[axis] {
                joint.quality |= JointStateQuality::MODE_CONFIRMED;
            }
            if drive_state.is_operation_enabled() {
                joint.quality |= JointStateQuality::OPERATION_ENABLED;
            }
            if !drive_state.is_fault() && inputs.error_code == 0 {
                joint.quality |= JointStateQuality::FAULT_FREE;
            }

            if let Some(raw) = inputs.actual_position {
                joint.position = finite_feedback_value(
                    (f64::from(raw) - f64::from(policy.position_offset))
                        / policy.position_units_per_radian,
                    axis,
                    Cia402PdoField::ActualPosition,
                )?;
                joint.quality |= JointStateQuality::POSITION_VALID;
            }
            if let Some(raw) = inputs.actual_velocity {
                joint.velocity = finite_feedback_value(
                    f64::from(raw) / policy.velocity_units_per_radian_per_second,
                    axis,
                    Cia402PdoField::ActualVelocity,
                )?;
                joint.quality |= JointStateQuality::VELOCITY_VALID;
            }
            if let Some(raw) = inputs.actual_torque {
                joint.torque = finite_feedback_value(
                    f64::from(raw) / policy.torque_units_per_newton_metre,
                    axis,
                    Cia402PdoField::ActualTorque,
                )?;
                joint.quality |= JointStateQuality::TORQUE_VALID;
            }
            if let Some(raw) = inputs.following_error {
                joint.following_error = finite_feedback_value(
                    f64::from(raw) / policy.position_units_per_radian,
                    axis,
                    Cia402PdoField::FollowingError,
                )?;
                joint.quality |= JointStateQuality::FOLLOWING_ERROR_VALID;
            }
        }

        if let Some(outputs) = accepted_outputs {
            joint.controlword = outputs[axis].controlword;
        }
    }
    state.axes = axes;
    Ok(())
}

#[cfg(feature = "cia402")]
fn finite_feedback_value(
    value: f64,
    axis: usize,
    field: Cia402PdoField,
) -> Result<f64, Cia402FeedbackError> {
    value
        .is_finite()
        .then_some(value)
        .ok_or(Cia402FeedbackError::NonFiniteValue(axis, field))
}

#[cfg(feature = "ethercat")]
use esop_procbuf::DomainQuality as ProcBufDomainQuality;

/// Write the raw cycle owner's facts into the unpublished State page. Bind
/// their sequence to that same page so the reader can reject stale quality.
pub fn cyclic_quality_to_procbuf<const AXES: usize, const IO: usize, const DOMAINS: usize>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    quality: CyclicQuality,
) {
    let observations = [
        (QualityFact::Platform, quality.platform_ready),
        (QualityFact::Configuration, quality.coe_ready),
        (QualityFact::Topology, quality.topology_valid),
        (
            QualityFact::DistributedClock,
            quality.distributed_clock_locked,
        ),
        (QualityFact::Drive, quality.drive_ready),
        (QualityFact::Domain, quality.domain_valid),
        (QualityFact::Wkc, quality.wkc_valid),
        (QualityFact::Command, quality.command_current),
        (QualityFact::Supervisor, quality.supervisor_healthy),
        (QualityFact::ExternalSafety, quality.external_safety_clear),
        (QualityFact::CycleBudget, quality.cycle_within_budget),
    ];
    let mut good_mask = 0;
    for (fact, good) in observations {
        if good {
            good_mask |= fact.bit();
        }
    }
    state.quality.cyclic = CyclicQualityMask {
        known_mask: QualityFact::ALL_MASK,
        good_mask,
    };
    state.quality.sequence = state.sequence;
}

/// Project all configured Domain slots while qualifying only the Domains
/// scheduled in this cycle. Slot order and `due` come from the frozen Domain
/// registry/schedule; the writer must finish every due receive before calling.
/// The returned facts must also be supplied to the guard for this cycle.
/// AL state, command age, deadline-miss count, and fault bitmap remain owned
/// by their respective producers and are left untouched in `state`.
#[cfg(feature = "ethercat")]
pub fn ethercat_cycle_to_procbuf<const AXES: usize, const IO: usize, const DOMAINS: usize>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    report: CycleReport,
    domains: &[EthercatDomainQuality; DOMAINS],
    due: &[bool; DOMAINS],
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    let quality = cyclic_quality_from_domains(
        report,
        domains
            .iter()
            .zip(due.iter())
            .filter_map(|(domain, is_due)| (*is_due).then_some(domain)),
        dc,
        other,
    );
    project_ethercat_diagnostics(state, report, domains.iter().copied(), dc, quality);
    quality
}

/// Project schedule-bound Domain evidence, including ticks without any due
/// Domain. Every slot must match the frozen schedule in order and ID. A bad or
/// missing snapshot publishes a blocked Domain/WKC gate, not a healthy one.
#[cfg(feature = "ethercat")]
pub fn scheduled_ethercat_cycle_to_procbuf<
    const AXES: usize,
    const IO: usize,
    const DOMAINS: usize,
    const SLOTS: usize,
>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    report: CycleReport,
    schedule: &ScheduleTable<DOMAINS, SLOTS>,
    domains: &[ScheduledDomainQuality; DOMAINS],
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    let quality = cyclic_quality_from_schedule(report, schedule, domains, dc, other);
    project_ethercat_diagnostics(
        state,
        report,
        domains.iter().map(|entry| entry.quality),
        dc,
        quality,
    );
    quality
}

#[cfg(feature = "ethercat")]
fn project_ethercat_diagnostics<const AXES: usize, const IO: usize, const DOMAINS: usize>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    report: CycleReport,
    domains: impl Iterator<Item = EthercatDomainQuality>,
    dc: &DcCyclicSync,
    quality: CyclicQuality,
) {
    if quality.distributed_clock_locked && dc.last_sync_cycle() == report.cycle {
        state.ecat_time_ns = dc.last_reference_time_ns();
    }
    state.quality.link_up = (!report.link_down) as u8;
    state.quality.dc_locked = quality.distributed_clock_locked as u8;
    // Offset is the last observed sample; dc_locked signals whether it is
    // fresh and qualified in this cycle.
    state.quality.dc_offset_ns = dc.monitor().offset_ns();
    for (destination, source) in state.quality.domains.iter_mut().zip(domains) {
        *destination = ProcBufDomainQuality {
            expected_wkc: source.expected_wkc,
            actual_wkc: source.actual_wkc,
            valid: source.valid as u8,
            complete: source.complete as u8,
            reserved: 0,
            last_valid_cycle: source.last_valid_cycle,
            input_age_cycles: source
                .input_age_cycles
                .max(report.cycle.saturating_sub(source.last_valid_cycle)),
        };
    }
    cyclic_quality_to_procbuf(state, quality);
}

/// `transition_time_ns` must come from the recorded transition's monotonic
/// timestamp, not from the current cycle when publishing a later snapshot.
pub fn lifecycle_to_procbuf(
    snapshot: LifecycleSnapshot,
    transition_time_ns: u64,
) -> LifecycleSummary {
    LifecycleSummary {
        state: snapshot.state as u8,
        stop_action: snapshot.stop_action as u8,
        gates_ready: ((snapshot.ready_gate_mask & snapshot.required_gate_mask)
            == snapshot.required_gate_mask) as u8,
        motion_permit: snapshot.motion_permit_current as u8,
        required_gate_mask: snapshot.required_gate_mask,
        valid_gate_mask: snapshot.valid_gate_mask,
        qualified_gate_mask: snapshot.qualified_gate_mask,
        ready_gate_mask: snapshot.ready_gate_mask,
        first_blocking_code: snapshot.first_blocking_code,
        latched_fault_code: snapshot.latched_fault_code,
        permit_epoch: snapshot.permit_epoch,
        permit_expires_at_ns: snapshot.permit_expires_at_ns,
        transition_sequence: snapshot.transition_sequence,
        transition_cycle: snapshot.transition_cycle,
        transition_time_ns,
        recovery_count: snapshot.recovery_count,
        permit_audit_sequence: snapshot.permit_audit_sequence,
    }
}

/// Source/code reserved for lifecycle diagnostics and per-axis stop deadline
/// escalation. Timeout `value` holds
/// the full fault code; `sequence` matches the lifecycle transition sequence.
/// `aux` packs requested/issued Protobuf action values in the low two bytes,
/// prior stop issuance in bit 16 and the FaultLatched state in the high byte.
pub const LIFECYCLE_EVENT_SOURCE: u16 = 0x4D4C;
pub const STOP_TIMEOUT_EVENT_SOURCE: u16 = LIFECYCLE_EVENT_SOURCE;
pub const STOP_TIMEOUT_EVENT_CODE: u16 = 1;
pub const LIFECYCLE_TRANSITION_EVENT_CODE: u16 = 2;
pub const LIFECYCLE_EVENT_NO_AXIS: u16 = u16::MAX;

fn stop_timeout_axes_fit<const AXES: usize>(mask: u32) -> bool {
    AXES <= MAX_MOTION_AXES && (AXES == MAX_MOTION_AXES || mask & !((1u32 << AXES) - 1) == 0)
}

/// The RT owner retains this cursor across cycles; a new boot needs a new
/// cursor. `next_sequence` is the first transition not yet written to ProcBuf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleEventCursor {
    boot_id: u64,
    next_sequence: u64,
}

impl LifecycleEventCursor {
    pub const fn new(guard: &LifecycleGuard) -> Self {
        Self {
            boot_id: guard.boot_id,
            next_sequence: 1,
        }
    }

    pub const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Explicitly acknowledge transitions overwritten before they could be
    /// published. Call only after reporting the returned lost count.
    pub fn acknowledge_history_loss(
        &mut self,
        guard: &LifecycleGuard,
    ) -> Result<u64, LifecycleEventError> {
        if self.boot_id != guard.boot_id {
            return Err(LifecycleEventError::BootMismatch);
        }
        if self.next_sequence > guard.transition_sequence.saturating_add(1) {
            return Err(LifecycleEventError::CursorAhead);
        }
        let oldest = guard
            .transition_at(0)
            .map(|record| record.sequence)
            .unwrap_or(guard.transition_sequence.saturating_add(1));
        let missed = oldest.saturating_sub(self.next_sequence);
        self.next_sequence = self.next_sequence.max(oldest);
        Ok(missed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleEventError {
    Header(HeaderError),
    BootMismatch,
    CursorAhead,
    HistoryOverrun { missed: u64 },
    Ring(EventPushError),
    StopTimeout(StopTimeoutEventError),
}

/// Publish all available lifecycle transitions before per-axis timeout events.
/// Both kinds use their transition sequence for State-page correlation. The
/// timestamp is the monotonic time of emission, not a reconstructed historical
/// transition time. A full ring keeps the failed transition/axis pending.
pub fn lifecycle_events_to_procbuf<
    const AXES: usize,
    const IO: usize,
    const DOMAINS: usize,
    const EVENTS: usize,
>(
    guard: &mut LifecycleGuard,
    buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    cursor: &mut LifecycleEventCursor,
    timestamp_ns: u64,
) -> Result<usize, LifecycleEventError> {
    if cursor.boot_id != guard.boot_id {
        return Err(LifecycleEventError::BootMismatch);
    }
    if cursor.next_sequence > guard.transition_sequence.saturating_add(1) {
        return Err(LifecycleEventError::CursorAhead);
    }
    let oldest = guard.transition_at(0);
    if let Some(record) = oldest {
        if cursor.next_sequence < record.sequence {
            return Err(LifecycleEventError::HistoryOverrun {
                missed: record.sequence - cursor.next_sequence,
            });
        }
    }
    if cursor.next_sequence == guard.transition_sequence.saturating_add(1)
        && guard.pending_stop_timeout_events_mask == 0
    {
        return Ok(0);
    }
    buffer
        .validate_header(buffer.header().robot_id, guard.boot_id)
        .map_err(LifecycleEventError::Header)?;
    if guard.pending_stop_timeout_events_mask != 0
        && !stop_timeout_axes_fit::<AXES>(
            guard
                .stop_timeout_record
                .map_or(0, |record| record.axis_mask),
        )
    {
        return Err(LifecycleEventError::StopTimeout(
            StopTimeoutEventError::AxisCapacityExceeded,
        ));
    }

    let mut written = 0;
    for index in 0..guard.transition_count() {
        let Some(transition) = guard.transition_at(index) else {
            continue;
        };
        if transition.sequence < cursor.next_sequence {
            continue;
        }
        let severity = match transition.to {
            LifecycleState::Stopping | LifecycleState::Maintenance => EventSeverity::Warning,
            LifecycleState::FaultLatched => EventSeverity::Fault,
            LifecycleState::Qualifying | LifecycleState::Ready | LifecycleState::Active => {
                EventSeverity::Info
            }
        };
        buffer
            .record_event(ProcBufEvent {
                sequence: transition.sequence,
                timestamp_ns,
                source: LIFECYCLE_EVENT_SOURCE,
                severity,
                code: LIFECYCLE_TRANSITION_EVENT_CODE,
                axis_or_device: LIFECYCLE_EVENT_NO_AXIS,
                value: transition.fault_code,
                aux: transition.from as u32 | ((transition.to as u32) << 8),
            })
            .map_err(LifecycleEventError::Ring)?;
        cursor.next_sequence = transition.sequence.saturating_add(1);
        written += 1;
    }
    let escalated = stop_timeout_events_to_procbuf(guard, buffer, timestamp_ns)
        .map_err(LifecycleEventError::StopTimeout)?;
    Ok(written + escalated)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopTimeoutEventError {
    Header(HeaderError),
    AxisCapacityExceeded,
    Ring(EventPushError),
}

/// Emit each armed axis's timeout escalation once, with retry on event-ring
/// overflow. The cycle owner supplies a monotonic timestamp and calls this
/// after evaluating the guard; a partial write leaves only unwritten axes
/// pending. Events are diagnostic, never stop confirmation.
pub fn stop_timeout_events_to_procbuf<
    const AXES: usize,
    const IO: usize,
    const DOMAINS: usize,
    const EVENTS: usize,
>(
    guard: &mut LifecycleGuard,
    buffer: &ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    timestamp_ns: u64,
) -> Result<usize, StopTimeoutEventError> {
    let Some(record) = guard.stop_timeout_record else {
        return Ok(0);
    };
    if guard.pending_stop_timeout_events_mask == 0 {
        return Ok(0);
    }
    buffer
        .validate_header(buffer.header().robot_id, guard.boot_id)
        .map_err(StopTimeoutEventError::Header)?;
    if !stop_timeout_axes_fit::<AXES>(record.axis_mask) {
        return Err(StopTimeoutEventError::AxisCapacityExceeded);
    }
    let mut written = 0;
    for axis in 0..MAX_MOTION_AXES {
        let bit = 1u32 << axis;
        if guard.pending_stop_timeout_events_mask & bit == 0 {
            continue;
        }
        let requested = record.actions[axis] as u32 + 1;
        buffer
            .record_event(ProcBufEvent {
                sequence: record.transition_sequence,
                timestamp_ns,
                source: STOP_TIMEOUT_EVENT_SOURCE,
                severity: EventSeverity::Fault,
                code: STOP_TIMEOUT_EVENT_CODE,
                axis_or_device: axis as u16,
                value: STOP_TIMEOUT_FAULT_CODE,
                aux: requested
                    | ((StopAction::Disable as u32 + 1) << 8)
                    | ((record.first_issued_cycle.is_some() as u32) << 16)
                    | ((LifecycleState::FaultLatched as u32) << 24),
            })
            .map_err(StopTimeoutEventError::Ring)?;
        guard.pending_stop_timeout_events_mask &= !bit;
        written += 1;
    }
    Ok(written)
}

#[cfg(feature = "cia402")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AxisEvidenceError {
    AxisCapacityExceeded,
    CycleMismatch,
    FeedbackBeforeStop,
    InvalidFeedback,
    UnsafeOutput(usize),
}

/// Stage evidence from the same cycle's guard decision and CiA 402 outputs.
/// Feedback must come from quality-checked current-cycle inputs; booleans are
/// proof of stationary/non-enabled only when feedback_valid is set.
#[cfg(feature = "cia402")]
pub fn axis_stops_to_procbuf<const AXES: usize, const IO: usize, const DOMAINS: usize>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    decision: &AxisCycleDecision<'_>,
    outputs: &[Cia402Output; AXES],
    feedback: Option<StopFeedback>,
) -> Result<(), AxisEvidenceError> {
    if AXES > MAX_MOTION_AXES {
        return Err(AxisEvidenceError::AxisCapacityExceeded);
    }
    if state.sequence == 0 || state.sequence != decision.cycle() {
        return Err(AxisEvidenceError::CycleMismatch);
    }
    let axes_mask = if AXES == MAX_MOTION_AXES {
        u32::MAX
    } else {
        (1u32 << AXES) - 1
    };
    let stopping_mask = decision.stopping_axis_mask();
    if stopping_mask & !axes_mask != 0 || decision.permitted_axis_mask() & !axes_mask != 0 {
        return Err(AxisEvidenceError::AxisCapacityExceeded);
    }
    if let Some(sample) = feedback {
        if sample.cycle != state.sequence {
            return Err(AxisEvidenceError::CycleMismatch);
        }
        if sample.observed_axis_mask & !stopping_mask != 0
            || (sample.stationary_axis_mask | sample.non_enabled_axis_mask)
                & !sample.observed_axis_mask
                != 0
        {
            return Err(AxisEvidenceError::InvalidFeedback);
        }
        if sample.observed_axis_mask != 0
            && !decision
                .stop_issued_cycle()
                .is_some_and(|issued| sample.cycle > issued)
        {
            return Err(AxisEvidenceError::FeedbackBeforeStop);
        }
    }

    let mut evidence = [AxisStopEvidence::EMPTY; AXES];
    for (axis, output) in outputs.iter().enumerate() {
        match decision.axis(axis) {
            AxisDirective::Stop(requested) => {
                let issued = match output.controlword {
                    CONTROLWORD_QUICK_STOP => StopAction::QuickStop,
                    CONTROLWORD_DISABLE_VOLTAGE => StopAction::Disable,
                    _ => return Err(AxisEvidenceError::UnsafeOutput(axis)),
                };
                let expected = if requested == StopAction::QuickStop {
                    StopAction::QuickStop
                } else {
                    StopAction::Disable
                };
                if issued != expected || output.motion_allowed || output.fault_reset_pulse {
                    return Err(AxisEvidenceError::UnsafeOutput(axis));
                }
                let bit = 1u32 << axis;
                let observed = feedback.is_some_and(|sample| sample.observed_axis_mask & bit != 0);
                evidence[axis] = AxisStopEvidence {
                    request_cycle: state.sequence,
                    feedback_cycle: if observed { state.sequence } else { 0 },
                    requested_action: requested as u8 + 1,
                    issued_action: if decision.stop_transmitted() {
                        issued as u8 + 1
                    } else {
                        0
                    },
                    feedback_valid: observed as u8,
                    stationary: feedback
                        .is_some_and(|sample| observed && sample.stationary_axis_mask & bit != 0)
                        as u8,
                    non_enabled: feedback
                        .is_some_and(|sample| observed && sample.non_enabled_axis_mask & bit != 0)
                        as u8,
                    reserved: [0; 3],
                };
            }
            AxisDirective::Inhibit => {
                if output.controlword != CONTROLWORD_DISABLE_VOLTAGE
                    || output.motion_allowed
                    || output.fault_reset_pulse
                {
                    return Err(AxisEvidenceError::UnsafeOutput(axis));
                }
            }
            AxisDirective::EnableAllowed => {}
        }
    }
    state.axis_stops = evidence;
    Ok(())
}

/// Stage stop evidence for a frame submitted by
/// `submit_controlled_stopping_frame`.
///
/// A controlled target records Hold or RampToZero as the issued action even
/// though CiA 402 keeps Operation Enabled while applying the target. Once the
/// planner requests Disable, the issued action becomes Disable. The report is
/// accepted only when its command and output agree for every stopping axis.
#[cfg(all(feature = "cia402", feature = "ethercat"))]
pub fn controlled_axis_stops_to_procbuf<
    const AXES: usize,
    const IO: usize,
    const DOMAINS: usize,
>(
    state: &mut StatePage<AXES, IO, DOMAINS>,
    decision: &AxisCycleDecision<'_>,
    report: &ControlledStopFrameReport<AXES>,
    feedback: Option<StopFeedback>,
) -> Result<(), AxisEvidenceError> {
    if AXES > MAX_MOTION_AXES {
        return Err(AxisEvidenceError::AxisCapacityExceeded);
    }
    if state.sequence == 0 || state.sequence != decision.cycle() {
        return Err(AxisEvidenceError::CycleMismatch);
    }
    let axes_mask = if AXES == MAX_MOTION_AXES {
        u32::MAX
    } else {
        (1u32 << AXES) - 1
    };
    let stopping_mask = decision.stopping_axis_mask();
    if stopping_mask & !axes_mask != 0 || decision.permitted_axis_mask() & !axes_mask != 0 {
        return Err(AxisEvidenceError::AxisCapacityExceeded);
    }
    if let Some(sample) = feedback {
        if sample.cycle != state.sequence {
            return Err(AxisEvidenceError::CycleMismatch);
        }
        if sample.observed_axis_mask & !stopping_mask != 0
            || (sample.stationary_axis_mask | sample.non_enabled_axis_mask)
                & !sample.observed_axis_mask
                != 0
        {
            return Err(AxisEvidenceError::InvalidFeedback);
        }
        if sample.observed_axis_mask != 0
            && !decision
                .stop_issued_cycle()
                .is_some_and(|issued| sample.cycle > issued)
        {
            return Err(AxisEvidenceError::FeedbackBeforeStop);
        }
    }

    let mut evidence = [AxisStopEvidence::EMPTY; AXES];
    for (axis, evidence_slot) in evidence.iter_mut().enumerate() {
        let output = report.outputs[axis];
        let command = report.controlled_commands[axis];
        match decision.axis(axis) {
            AxisDirective::Stop(requested @ (StopAction::Hold | StopAction::RampToZero)) => {
                let Some(command) = command else {
                    return Err(AxisEvidenceError::UnsafeOutput(axis));
                };
                if command.action != requested
                    || command.controlword != output.controlword
                    || output.motion_allowed
                    || output.fault_reset_pulse
                {
                    return Err(AxisEvidenceError::UnsafeOutput(axis));
                }
                let issued =
                    if command.controlled() && output.controlword == CONTROLWORD_ENABLE_OPERATION {
                        requested
                    } else if command.target.is_none()
                        && output.controlword == CONTROLWORD_DISABLE_VOLTAGE
                    {
                        StopAction::Disable
                    } else {
                        return Err(AxisEvidenceError::UnsafeOutput(axis));
                    };
                let bit = 1u32 << axis;
                let observed = feedback.is_some_and(|sample| sample.observed_axis_mask & bit != 0);
                *evidence_slot = AxisStopEvidence {
                    request_cycle: state.sequence,
                    feedback_cycle: if observed { state.sequence } else { 0 },
                    requested_action: requested as u8 + 1,
                    issued_action: if decision.stop_transmitted() {
                        issued as u8 + 1
                    } else {
                        0
                    },
                    feedback_valid: observed as u8,
                    stationary: feedback
                        .is_some_and(|sample| observed && sample.stationary_axis_mask & bit != 0)
                        as u8,
                    non_enabled: feedback
                        .is_some_and(|sample| observed && sample.non_enabled_axis_mask & bit != 0)
                        as u8,
                    reserved: [0; 3],
                };
            }
            AxisDirective::Stop(requested @ (StopAction::QuickStop | StopAction::Disable)) => {
                if command.is_some() {
                    return Err(AxisEvidenceError::UnsafeOutput(axis));
                }
                let expected = if requested == StopAction::QuickStop {
                    CONTROLWORD_QUICK_STOP
                } else {
                    CONTROLWORD_DISABLE_VOLTAGE
                };
                if output.controlword != expected
                    || output.motion_allowed
                    || output.fault_reset_pulse
                {
                    return Err(AxisEvidenceError::UnsafeOutput(axis));
                }
                let bit = 1u32 << axis;
                let observed = feedback.is_some_and(|sample| sample.observed_axis_mask & bit != 0);
                *evidence_slot = AxisStopEvidence {
                    request_cycle: state.sequence,
                    feedback_cycle: if observed { state.sequence } else { 0 },
                    requested_action: requested as u8 + 1,
                    issued_action: if decision.stop_transmitted() {
                        requested as u8 + 1
                    } else {
                        0
                    },
                    feedback_valid: observed as u8,
                    stationary: feedback
                        .is_some_and(|sample| observed && sample.stationary_axis_mask & bit != 0)
                        as u8,
                    non_enabled: feedback
                        .is_some_and(|sample| observed && sample.non_enabled_axis_mask & bit != 0)
                        as u8,
                    reserved: [0; 3],
                };
            }
            AxisDirective::Inhibit => {
                if command.is_some()
                    || output.controlword != CONTROLWORD_DISABLE_VOLTAGE
                    || output.motion_allowed
                    || output.fault_reset_pulse
                {
                    return Err(AxisEvidenceError::UnsafeOutput(axis));
                }
            }
            AxisDirective::EnableAllowed => return Err(AxisEvidenceError::UnsafeOutput(axis)),
        }
    }
    state.axis_stops = evidence;
    Ok(())
}

#[cfg(all(test, feature = "cia402"))]
mod command_tests {
    use super::*;
    use crate::GuardPolicy;
    use esop_procbuf::{IoCommand, JointCommand};

    const POLICY: Cia402AxisCommandPolicy = Cia402AxisCommandPolicy {
        position_units_per_radian: -100.0,
        velocity_units_per_radian_per_second: -20.0,
        torque_units_per_newton_metre: 10.0,
        position_offset: 10,
        min_position_radians: -2.0,
        max_position_radians: 2.0,
        max_velocity_radians_per_second: 3.0,
        max_torque_newton_metres: 2.0,
        max_position_step_radians: 0.15,
    };

    fn command(mode: ControlMode, sequence: u64) -> CommandPage<1, 1> {
        CommandPage {
            boot_id: 9,
            sequence,
            deadline_ns: 2_000,
            source_id: 7,
            permit_epoch: 3,
            permit_expires_at_ns: 2_000,
            axis_mask: 1,
            requested_mode: mode,
            motion_enable_request: 1,
            authority: 2,
            reserved: [0; 3],
            policy_version: 4,
            axes: [JointCommand {
                position: 1.234,
                velocity: 1.26,
                torque: -1.25,
                max_velocity: 2.0,
                max_torque: 1.5,
            }],
            io: [IoCommand::EMPTY],
        }
    }

    fn guard_for(command: &CommandPage<1, 1>) -> LifecycleGuard {
        let mut guard = LifecycleGuard::new(
            0,
            command.boot_id,
            GuardPolicy {
                enter_good_cycles: 1,
                exit_bad_cycles: 1,
                max_age_cycles: 1,
                stop_timeout_cycles: 1,
                stop_action: StopAction::Disable,
                authorized_source_id: command.source_id,
                minimum_authority: 1,
                permit_policy_version: command.policy_version,
                allowed_axis_mask: 1,
            },
        );
        guard
            .accept_permit(motion_permit_from_command(command).unwrap(), 1_000)
            .unwrap();
        guard
    }

    #[test]
    fn prepares_all_modes_with_signed_scales_offset_and_conservative_limits() {
        for (control_mode, operating_mode, expected) in [
            (
                ControlMode::Csp,
                OperatingMode::Csp,
                Cia402Target::Position(-113),
            ),
            (
                ControlMode::Csv,
                OperatingMode::Csv,
                Cia402Target::Velocity(-25),
            ),
            (
                ControlMode::Cst,
                OperatingMode::Cst,
                Cia402Target::Torque(-13),
            ),
        ] {
            let command = command(control_mode, operating_mode as u64);
            let guard = guard_for(&command);
            let prepared = prepare_cia402_command(
                &command,
                &guard,
                &[operating_mode],
                &[POLICY],
                100_000_000,
                1_000,
            )
            .unwrap();

            assert_eq!(prepared.permit(), guard.permit().unwrap());
            assert_eq!(prepared.targets(), &[Some(expected)]);
            assert_eq!(
                prepared.limits(),
                &[CyclicLimits {
                    max_position_step: 15.0,
                    max_velocity: 40.0,
                    max_torque: 15.0,
                }]
            );
        }
    }

    #[test]
    fn preparation_rejects_identity_mode_policy_bounds_and_overflow_without_mutation() {
        let command = command(ControlMode::Csp, 8);
        let guard = guard_for(&command);
        let before = guard.permit();

        let mut wrong_permit = command;
        wrong_permit.sequence += 1;
        assert_eq!(
            prepare_cia402_command(
                &wrong_permit,
                &guard,
                &[OperatingMode::Csp],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::PermitMismatch)
        );
        assert_eq!(
            prepare_cia402_command(
                &command,
                &guard,
                &[OperatingMode::Csv],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::ModeMismatch(0))
        );

        let mut outside = command;
        outside.axes[0].position = 3.0;
        assert_eq!(
            prepare_cia402_command(
                &outside,
                &guard,
                &[OperatingMode::Csp],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::PositionOutOfBounds(0))
        );

        let mut invalid_policy = POLICY;
        invalid_policy.position_units_per_radian = 0.0;
        assert_eq!(
            prepare_cia402_command(
                &command,
                &guard,
                &[OperatingMode::Csp],
                &[invalid_policy],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::InvalidPolicy(
                0,
                Cia402AxisCommandPolicyError::ZeroScale,
            ))
        );

        let mut overflow_policy = POLICY;
        overflow_policy.position_units_per_radian = 1.0e20;
        assert_eq!(
            prepare_cia402_command(
                &command,
                &guard,
                &[OperatingMode::Csp],
                &[overflow_policy],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::RawTargetOverflow(0))
        );
        assert_eq!(
            prepare_cia402_command(&command, &guard, &[OperatingMode::Csp], &[POLICY], 0, 1_000,),
            Err(ProcBufCia402CommandError::InvalidCyclePeriod)
        );
        assert_eq!(guard.permit(), before);
    }

    #[test]
    fn preparation_rejects_caps_expiry_disabled_motion_and_raw_limit_overflow() {
        let csp = command(ControlMode::Csp, 11);
        let guard = guard_for(&csp);

        let mut velocity_cap = csp;
        velocity_cap.axes[0].max_velocity = 4.0;
        assert_eq!(
            prepare_cia402_command(
                &velocity_cap,
                &guard,
                &[OperatingMode::Csp],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::VelocityCapExceeded(0))
        );

        let mut torque_cap = csp;
        torque_cap.axes[0].max_torque = 3.0;
        assert_eq!(
            prepare_cia402_command(
                &torque_cap,
                &guard,
                &[OperatingMode::Csp],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::TorqueCapExceeded(0))
        );

        let mut csv = command(ControlMode::Csv, 12);
        csv.axes[0].velocity = 2.5;
        csv.axes[0].max_velocity = 2.0;
        let csv_guard = guard_for(&csv);
        assert_eq!(
            prepare_cia402_command(
                &csv,
                &csv_guard,
                &[OperatingMode::Csv],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::VelocityOutOfBounds(0))
        );

        let mut cst = command(ControlMode::Cst, 13);
        cst.axes[0].torque = 1.75;
        cst.axes[0].max_torque = 1.5;
        let cst_guard = guard_for(&cst);
        assert_eq!(
            prepare_cia402_command(
                &cst,
                &cst_guard,
                &[OperatingMode::Cst],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::TorqueOutOfBounds(0))
        );

        let mut disabled = csp;
        disabled.motion_enable_request = 0;
        assert_eq!(
            prepare_cia402_command(
                &disabled,
                &guard,
                &[OperatingMode::Csp],
                &[POLICY],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::MotionDisabled)
        );
        assert_eq!(
            prepare_cia402_command(
                &csp,
                &guard,
                &[OperatingMode::Csp],
                &[POLICY],
                100_000_000,
                2_000,
            ),
            Err(ProcBufCia402CommandError::Expired)
        );

        let mut limit_overflow = POLICY;
        limit_overflow.velocity_units_per_radian_per_second = 1.0e20;
        assert_eq!(
            prepare_cia402_command(
                &csp,
                &guard,
                &[OperatingMode::Csp],
                &[limit_overflow],
                100_000_000,
                1_000,
            ),
            Err(ProcBufCia402CommandError::RawLimitOverflow(0))
        );
    }
}

#[cfg(all(test, feature = "cia402", feature = "ethercat"))]
mod feedback_tests {
    use super::*;
    use esop_ethercat_core::PdoEntry;
    use esop_procbuf::{JointState, JointStateQuality};

    const POLICY: Cia402AxisCommandPolicy = Cia402AxisCommandPolicy {
        position_units_per_radian: -100.0,
        velocity_units_per_radian_per_second: -20.0,
        torque_units_per_newton_metre: 10.0,
        position_offset: 10,
        min_position_radians: -10.0,
        max_position_radians: 10.0,
        max_velocity_radians_per_second: 20.0,
        max_torque_newton_metres: 20.0,
        max_position_step_radians: 1.0,
    };

    fn field_entry(
        field: Cia402PdoField,
        bit_offset: usize,
        bit_length: u8,
        signed: bool,
    ) -> PdoEntry {
        PdoEntry {
            index: field.object_index(),
            subindex: 0,
            bit_offset,
            bit_length,
            signed,
            direction: field.direction(),
        }
    }

    fn map_at(base: usize) -> Cia402PdoMap {
        let mut map = Cia402PdoMap::new();
        for (field, offset, length, signed) in [
            (Cia402PdoField::Statusword, 0, 16, false),
            (Cia402PdoField::ModeDisplay, 16, 8, true),
            (Cia402PdoField::ErrorCode, 24, 16, false),
            (Cia402PdoField::ActualPosition, 40, 32, true),
            (Cia402PdoField::ActualVelocity, 72, 32, true),
            (Cia402PdoField::ActualTorque, 104, 16, true),
            (Cia402PdoField::FollowingError, 120, 32, true),
            (Cia402PdoField::Controlword, 152, 16, false),
            (Cia402PdoField::ModeOfOperation, 168, 8, true),
            (Cia402PdoField::TargetPosition, 176, 32, true),
            (Cia402PdoField::TargetVelocity, 208, 32, true),
            (Cia402PdoField::TargetTorque, 240, 16, true),
        ] {
            map.set_entry(field, field_entry(field, base + offset, length, signed));
        }
        map
    }

    #[derive(Clone, Copy)]
    struct TestFeedback {
        statusword: u16,
        error_code: u16,
        position: i32,
        velocity: i32,
        torque: i16,
        following_error: i32,
    }

    fn write_feedback(map: &Cia402PdoMap, image: &mut [u8], feedback: TestFeedback) {
        map.entry(Cia402PdoField::Statusword)
            .unwrap()
            .write_unsigned(image, u64::from(feedback.statusword))
            .unwrap();
        map.entry(Cia402PdoField::ModeDisplay)
            .unwrap()
            .write_signed(image, i64::from(OperatingMode::Csp.raw()))
            .unwrap();
        map.entry(Cia402PdoField::ErrorCode)
            .unwrap()
            .write_unsigned(image, u64::from(feedback.error_code))
            .unwrap();
        for (field, value) in [
            (Cia402PdoField::ActualPosition, i64::from(feedback.position)),
            (Cia402PdoField::ActualVelocity, i64::from(feedback.velocity)),
            (Cia402PdoField::ActualTorque, i64::from(feedback.torque)),
            (
                Cia402PdoField::FollowingError,
                i64::from(feedback.following_error),
            ),
        ] {
            map.entry(field)
                .unwrap()
                .write_signed(image, value)
                .unwrap();
        }
    }

    fn output(controlword: u16) -> Cia402Output {
        Cia402Output {
            state: DriveState::OperationEnabled,
            statusword: 0x0027,
            controlword,
            operation_enabled: true,
            motion_allowed: true,
            fault_reset_pulse: false,
        }
    }

    #[test]
    fn projects_signed_si_feedback_quality_faults_and_accepted_controlword() {
        let map = map_at(0);
        let mut image = [0_u8; 32];
        let mut feedback = TestFeedback {
            statusword: 0x0027,
            error_code: 0,
            position: -113,
            velocity: -25,
            torque: -13,
            following_error: -7,
        };
        write_feedback(&map, &mut image, feedback);
        let mut state = StatePage::<1, 0, 1>::new(9);

        cia402_feedback_to_procbuf(
            &mut state,
            &image,
            &[map],
            &[OperatingMode::Csp],
            &[POLICY],
            true,
            Some(&[output(0x000F)]),
        )
        .unwrap();

        let joint = state.axes[0];
        assert!((joint.position - 1.23).abs() < f64::EPSILON);
        assert!((joint.velocity - 1.25).abs() < f64::EPSILON);
        assert!((joint.torque + 1.3).abs() < f64::EPSILON);
        assert!((joint.following_error - 0.07).abs() < f64::EPSILON);
        assert_eq!(joint.statusword, 0x0027);
        assert_eq!(joint.controlword, 0x000F);
        assert_eq!(joint.error_code, 0);
        assert_eq!(joint.drive_state, DriveState::OperationEnabled as u8);
        assert_eq!(joint.actual_mode, OperatingMode::Csp as u8);
        assert_eq!(joint.quality, u8::MAX);

        feedback.statusword = 0x0008;
        feedback.error_code = 0x2310;
        write_feedback(&map, &mut image, feedback);
        cia402_feedback_to_procbuf(
            &mut state,
            &image,
            &[map],
            &[OperatingMode::Csp],
            &[POLICY],
            true,
            None,
        )
        .unwrap();
        assert_eq!(state.axes[0].drive_state, DriveState::Fault as u8);
        assert_eq!(state.axes[0].error_code, 0x2310);
        assert_eq!(state.axes[0].quality & JointStateQuality::FAULT_FREE, 0);
        assert_eq!(
            state.axes[0].quality & JointStateQuality::OPERATION_ENABLED,
            0
        );
    }

    #[test]
    fn stale_and_unmapped_feedback_retain_values_but_clear_validity() {
        let full_map = map_at(0);
        let mut sparse_map = full_map;
        sparse_map.clear_entry(Cia402PdoField::ActualVelocity);
        sparse_map.clear_entry(Cia402PdoField::ActualTorque);
        sparse_map.clear_entry(Cia402PdoField::FollowingError);
        let mut image = [0_u8; 32];
        write_feedback(
            &full_map,
            &mut image,
            TestFeedback {
                statusword: 0x0027,
                error_code: 0,
                position: -113,
                velocity: -25,
                torque: -13,
                following_error: -7,
            },
        );
        let mut state = StatePage::<1, 0, 1>::new(9);
        state.axes[0] = JointState {
            position: 9.0,
            velocity: 8.0,
            torque: 7.0,
            following_error: 6.0,
            statusword: 5,
            controlword: 4,
            error_code: 3,
            drive_state: 2,
            actual_mode: 1,
            quality: u8::MAX,
            reserved: 0,
        };

        cia402_feedback_to_procbuf(
            &mut state,
            &image,
            &[sparse_map],
            &[OperatingMode::Csp],
            &[POLICY],
            true,
            None,
        )
        .unwrap();
        assert!((state.axes[0].position - 1.23).abs() < f64::EPSILON);
        assert_eq!(state.axes[0].velocity, 8.0);
        assert_eq!(state.axes[0].torque, 7.0);
        assert_eq!(state.axes[0].following_error, 6.0);
        assert_ne!(state.axes[0].quality & JointStateQuality::POSITION_VALID, 0);
        assert_eq!(
            state.axes[0].quality
                & (JointStateQuality::VELOCITY_VALID
                    | JointStateQuality::TORQUE_VALID
                    | JointStateQuality::FOLLOWING_ERROR_VALID),
            0
        );

        let retained = state.axes[0];
        cia402_feedback_to_procbuf(
            &mut state,
            &[],
            &[Cia402PdoMap::new()],
            &[OperatingMode::Unknown],
            &[POLICY],
            false,
            Some(&[output(0x0002)]),
        )
        .unwrap();
        assert_eq!(state.axes[0].quality, 0);
        assert_eq!(state.axes[0].controlword, 0x0002);
        assert_eq!(state.axes[0].position, retained.position);
        assert_eq!(state.axes[0].statusword, retained.statusword);
        assert_eq!(state.axes[0].error_code, retained.error_code);
    }

    #[test]
    fn multi_axis_failure_does_not_publish_a_partial_snapshot() {
        let maps = [map_at(0), map_at(256)];
        let mut image = [0_u8; 64];
        write_feedback(
            &maps[0],
            &mut image,
            TestFeedback {
                statusword: 0x0027,
                error_code: 0,
                position: -113,
                velocity: -25,
                torque: -13,
                following_error: -7,
            },
        );
        write_feedback(
            &maps[1],
            &mut image,
            TestFeedback {
                statusword: 0x0027,
                error_code: 0,
                position: -213,
                velocity: -45,
                torque: -23,
                following_error: -9,
            },
        );
        let mut state = StatePage::<2, 0, 1>::new(9);
        state.axes[0].position = 10.0;
        state.axes[1].position = 20.0;
        let before = state.axes;
        let mut invalid = POLICY;
        invalid.position_units_per_radian = 0.0;

        assert_eq!(
            cia402_feedback_to_procbuf(
                &mut state,
                &image,
                &maps,
                &[OperatingMode::Csp; 2],
                &[POLICY, invalid],
                true,
                None,
            ),
            Err(Cia402FeedbackError::InvalidPolicy(
                1,
                Cia402AxisCommandPolicyError::ZeroScale,
            ))
        );
        assert_eq!(state.axes, before);
    }
}

#[cfg(all(test, feature = "ethercat"))]
mod tests {
    use super::*;
    use esop_ethercat_core::wire::{Command, DatagramHeader};
    use esop_ethercat_core::{DcCyclicConfig, DcMonitor, RxMatch};

    #[test]
    fn scheduled_domains_only_qualify_but_all_domain_ages_advance() {
        let mut dc = DcCyclicSync::new(
            DcCyclicConfig::new(0x1000, 13, 0),
            DcMonitor::new(50, 10, 1, 2),
        );
        let mut image = [0; 8];
        dc.prepare(1, 120, &mut image).unwrap();
        let plan = dc.datagram_plan();
        dc.complete(
            5,
            100,
            RxMatch {
                slot_id: 0,
                generation: 1,
                working_counter: 1,
            },
            DatagramHeader {
                command: Command::Frmw,
                index: plan.index,
                address: plan.address,
                length: 8,
                last: true,
            },
            &100u64.to_le_bytes(),
        )
        .unwrap();
        let report = CycleReport {
            cycle: 5,
            received_frames: 1,
            received_bytes: 64,
            parsed_datagrams: 2,
            unmatched_datagrams: 0,
            corrupt_frames: 0,
            wkc_mismatches: 0,
            timed_out_datagrams: 0,
            consumer_rejections: 0,
            budget_exhausted: false,
            link_down: false,
        };
        let domains = [
            EthercatDomainQuality {
                expected_wkc: 2,
                actual_wkc: 2,
                valid: true,
                complete: true,
                last_valid_cycle: 5,
                input_age_cycles: 0,
            },
            EthercatDomainQuality {
                expected_wkc: 1,
                actual_wkc: 1,
                valid: true,
                complete: true,
                last_valid_cycle: 3,
                input_age_cycles: 0,
            },
        ];
        let other = OtherCycleFacts {
            platform_ready: true,
            coe_ready: true,
            topology_valid: true,
            drive_ready: true,
            command_current: true,
            supervisor_healthy: true,
            external_safety_clear: true,
            deadline_met: true,
        };
        let mut state = StatePage::<0, 0, 2>::new(7);
        state.sequence = 77;
        state.quality.al_state = 8;
        state.quality.command_age_cycles = 9;
        state.quality.deadline_misses = 2;
        state.quality.fault_bitmap = 3;

        let good =
            ethercat_cycle_to_procbuf(&mut state, report, &domains, &[true, false], &dc, other);
        assert!(good.domain_valid && good.wkc_valid && good.distributed_clock_locked);
        assert_eq!(state.quality.sequence, 77);
        assert!(state.quality.cyclic.complete());
        assert!(state.quality.cyclic.good(QualityFact::Domain));
        assert_eq!(state.quality.link_up, 1);
        assert_eq!(state.quality.dc_locked, 1);
        assert_eq!(state.quality.dc_offset_ns, 20);
        assert_eq!(state.ecat_time_ns, 100);
        assert_eq!(state.quality.domains[0].expected_wkc, 2);
        assert_eq!(state.quality.domains[0].input_age_cycles, 0);
        assert_eq!(state.quality.domains[1].last_valid_cycle, 3);
        assert_eq!(state.quality.domains[1].input_age_cycles, 2);
        assert_eq!(state.quality.al_state, 8);
        assert_eq!(state.quality.command_age_cycles, 9);
        assert_eq!(state.quality.deadline_misses, 2);
        assert_eq!(state.quality.fault_bitmap, 3);

        let missed =
            ethercat_cycle_to_procbuf(&mut state, report, &domains, &[true, true], &dc, other);
        assert!(!missed.domain_valid && !missed.wkc_valid);
        assert!(!state.quality.cyclic.good(QualityFact::Domain));
        let none_due =
            ethercat_cycle_to_procbuf(&mut state, report, &domains, &[false, false], &dc, other);
        assert!(!none_due.domain_valid && !none_due.wkc_valid);
        let link_down = ethercat_cycle_to_procbuf(
            &mut state,
            CycleReport {
                link_down: true,
                ..report
            },
            &domains,
            &[true, false],
            &dc,
            other,
        );
        assert!(!link_down.wkc_valid);
        assert_eq!(state.quality.link_up, 0);
        assert_eq!(state.quality.dc_locked, 0);
        let stale_dc = ethercat_cycle_to_procbuf(
            &mut state,
            CycleReport { cycle: 6, ..report },
            &domains,
            &[true, false],
            &dc,
            other,
        );
        assert!(!stale_dc.distributed_clock_locked);
        assert_eq!(state.quality.dc_locked, 0);
        assert_eq!(state.quality.dc_offset_ns, 20);
        assert_eq!(state.ecat_time_ns, 100);
    }
}
