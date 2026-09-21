//! Project validated, current-cycle CiA 402 feedback into stop evidence.

use esop_profile_cia402::{
    CONTROLWORD_DISABLE_VOLTAGE, CONTROLWORD_ENABLE_OPERATION, Cia402AxisBank, Cia402Output,
    Cia402PdoInputs, Cia402Target, DriveRequest, DriveState, OperatingMode,
};

use crate::{AxisCycleDecision, AxisDirective, StopAction, StopFeedback};

/// Apply this guard cycle's per-axis decision to a CiA 402 bank. Hold and
/// RampToZero require product-validated target generators; without one they
/// fail closed as Disable rather than replaying a stale Enable or setpoint.
pub fn step_axis_bank<const AXES: usize>(
    bank: &mut Cia402AxisBank<AXES>,
    decision: &AxisCycleDecision<'_>,
    statuswords: [u16; AXES],
    requests: [DriveRequest; AXES],
) -> [Cia402Output; AXES] {
    let stop_requests = core::array::from_fn(|index| match decision.axis(index) {
        AxisDirective::Stop(StopAction::QuickStop) => DriveRequest::QuickStop,
        AxisDirective::Inhibit
        | AxisDirective::EnableAllowed
        | AxisDirective::Stop(StopAction::Disable | StopAction::Hold | StopAction::RampToZero) => {
            DriveRequest::Disable
        }
    });
    bank.step_with_axis_stop(
        statuswords,
        requests,
        decision.permitted_axis_mask(),
        stop_requests,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ControlledStopPhase {
    Idle = 0,
    Applying = 1,
    ZeroCommanded = 2,
    DisableRequested = 3,
}

/// Product-frozen raw-unit bounds for one controlled stop axis.
///
/// These values are not inferred from the drive. They must be derived from the
/// configured PDO scaling, mechanics, cycle time, and HIL qualification for the
/// exact product combination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlledStopLimits {
    pub max_velocity_step: u32,
    pub max_torque_step: u16,
    pub max_stationary_velocity: u32,
    pub max_zero_torque: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlledStopError {
    NotControlledAction,
    StopSequenceReplayed,
    CycleReplayed,
    ActionChanged,
    ModeChanged,
    LimitsChanged,
    UnsupportedMode,
    ModeNotConfirmed,
    MissingPosition,
    MissingVelocity,
    MissingTorque,
    InvalidLimits,
}

/// One bounded stop command. `target=None` always means fail-closed Disable;
/// normal motion permission is never implied by an enabled controlled target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlledStopCommand {
    pub controlword: u16,
    pub mode: OperatingMode,
    pub target: Option<Cia402Target>,
    pub action: StopAction,
    pub phase: ControlledStopPhase,
}

impl ControlledStopCommand {
    pub const fn controlled(self) -> bool {
        self.controlword == CONTROLWORD_ENABLE_OPERATION && self.target.is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlledStopPreview {
    next: ControlledStopPlanner,
    command: ControlledStopCommand,
}

impl ControlledStopPreview {
    pub const fn command(self) -> ControlledStopCommand {
        self.command
    }
}

/// Fixed-state generator for product-qualified Hold and RampToZero targets.
///
/// `preview` is transactional: callers must commit the preview only after the
/// complete EtherCAT frame has been accepted by the port. A failed build or TX
/// can retry from the unchanged planner without advancing the ramp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlledStopPlanner {
    initialized: bool,
    stop_sequence: u64,
    last_cycle: u64,
    action: StopAction,
    mode: OperatingMode,
    limits: ControlledStopLimits,
    phase: ControlledStopPhase,
    hold_position: i32,
}

impl ControlledStopPlanner {
    pub const fn new() -> Self {
        Self {
            initialized: false,
            stop_sequence: 0,
            last_cycle: 0,
            action: StopAction::Disable,
            mode: OperatingMode::Unknown,
            limits: ControlledStopLimits {
                max_velocity_step: 0,
                max_torque_step: 0,
                max_stationary_velocity: 0,
                max_zero_torque: 0,
            },
            phase: ControlledStopPhase::Idle,
            hold_position: 0,
        }
    }

    pub const fn phase(&self) -> ControlledStopPhase {
        self.phase
    }

    pub const fn stop_sequence(&self) -> Option<u64> {
        if self.initialized {
            Some(self.stop_sequence)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn preview(
        &self,
        stop_sequence: u64,
        cycle: u64,
        action: StopAction,
        mode: OperatingMode,
        inputs: Cia402PdoInputs,
        limits: ControlledStopLimits,
    ) -> Result<ControlledStopPreview, ControlledStopError> {
        let mut next = *self;
        let command = next.advance(stop_sequence, cycle, action, mode, inputs, limits)?;
        Ok(ControlledStopPreview { next, command })
    }

    pub fn commit(&mut self, preview: ControlledStopPreview) {
        *self = preview.next;
    }

    #[allow(clippy::too_many_arguments)]
    fn advance(
        &mut self,
        stop_sequence: u64,
        cycle: u64,
        action: StopAction,
        mode: OperatingMode,
        inputs: Cia402PdoInputs,
        limits: ControlledStopLimits,
    ) -> Result<ControlledStopCommand, ControlledStopError> {
        if !matches!(action, StopAction::Hold | StopAction::RampToZero) {
            return Err(ControlledStopError::NotControlledAction);
        }
        if self.initialized {
            if stop_sequence < self.stop_sequence {
                return Err(ControlledStopError::StopSequenceReplayed);
            }
            if stop_sequence == self.stop_sequence {
                if cycle <= self.last_cycle {
                    return Err(ControlledStopError::CycleReplayed);
                }
                if action != self.action {
                    return Err(ControlledStopError::ActionChanged);
                }
                if mode != self.mode {
                    return Err(ControlledStopError::ModeChanged);
                }
                if limits != self.limits {
                    return Err(ControlledStopError::LimitsChanged);
                }
            } else {
                *self = Self::new();
            }
        }

        if !self.initialized {
            self.initialize(stop_sequence, cycle, action, mode, inputs, limits)?;
        } else {
            self.last_cycle = cycle;
        }

        if DriveState::from_statusword(inputs.statusword) != DriveState::OperationEnabled {
            self.phase = ControlledStopPhase::DisableRequested;
            return Ok(self.disable_command());
        }
        if inputs.actual_mode != mode {
            return Err(ControlledStopError::ModeNotConfirmed);
        }

        match action {
            StopAction::Hold => self.hold_command(inputs, limits),
            StopAction::RampToZero => self.ramp_command(inputs, limits),
            StopAction::QuickStop | StopAction::Disable => {
                Err(ControlledStopError::NotControlledAction)
            }
        }
    }

    fn initialize(
        &mut self,
        stop_sequence: u64,
        cycle: u64,
        action: StopAction,
        mode: OperatingMode,
        inputs: Cia402PdoInputs,
        limits: ControlledStopLimits,
    ) -> Result<(), ControlledStopError> {
        validate_limits(action, mode, limits)?;
        let hold_position = if action == StopAction::Hold
            && DriveState::from_statusword(inputs.statusword) == DriveState::OperationEnabled
        {
            inputs
                .actual_position
                .ok_or(ControlledStopError::MissingPosition)?
        } else {
            0
        };
        self.initialized = true;
        self.stop_sequence = stop_sequence;
        self.last_cycle = cycle;
        self.action = action;
        self.mode = mode;
        self.limits = limits;
        self.phase = ControlledStopPhase::Idle;
        self.hold_position = hold_position;
        Ok(())
    }

    fn hold_command(
        &mut self,
        inputs: Cia402PdoInputs,
        limits: ControlledStopLimits,
    ) -> Result<ControlledStopCommand, ControlledStopError> {
        if self.mode != OperatingMode::Csp {
            return Err(ControlledStopError::UnsupportedMode);
        }
        let velocity = inputs
            .actual_velocity
            .ok_or(ControlledStopError::MissingVelocity)?;
        if self.phase == ControlledStopPhase::Applying
            && velocity.unsigned_abs() <= limits.max_stationary_velocity
        {
            self.phase = ControlledStopPhase::DisableRequested;
            return Ok(self.disable_command());
        }
        self.phase = ControlledStopPhase::Applying;
        Ok(self.target_command(Cia402Target::Position(self.hold_position)))
    }

    fn ramp_command(
        &mut self,
        inputs: Cia402PdoInputs,
        limits: ControlledStopLimits,
    ) -> Result<ControlledStopCommand, ControlledStopError> {
        let velocity = inputs
            .actual_velocity
            .ok_or(ControlledStopError::MissingVelocity)?;
        if self.phase == ControlledStopPhase::ZeroCommanded
            && velocity.unsigned_abs() <= limits.max_stationary_velocity
            && (self.mode != OperatingMode::Cst
                || inputs
                    .actual_torque
                    .is_some_and(|torque| torque.unsigned_abs() <= limits.max_zero_torque))
        {
            self.phase = ControlledStopPhase::DisableRequested;
            return Ok(self.disable_command());
        }

        let target = match self.mode {
            OperatingMode::Csv => {
                Cia402Target::Velocity(toward_zero_i32(velocity, limits.max_velocity_step))
            }
            OperatingMode::Cst => {
                let torque = inputs
                    .actual_torque
                    .ok_or(ControlledStopError::MissingTorque)?;
                Cia402Target::Torque(toward_zero_i16(torque, limits.max_torque_step))
            }
            OperatingMode::Csp | OperatingMode::Unknown => {
                return Err(ControlledStopError::UnsupportedMode);
            }
        };
        self.phase = match target {
            Cia402Target::Velocity(0) | Cia402Target::Torque(0) => {
                ControlledStopPhase::ZeroCommanded
            }
            Cia402Target::Position(_) | Cia402Target::Velocity(_) | Cia402Target::Torque(_) => {
                ControlledStopPhase::Applying
            }
        };
        Ok(self.target_command(target))
    }

    const fn target_command(&self, target: Cia402Target) -> ControlledStopCommand {
        ControlledStopCommand {
            controlword: CONTROLWORD_ENABLE_OPERATION,
            mode: self.mode,
            target: Some(target),
            action: self.action,
            phase: self.phase,
        }
    }

    const fn disable_command(&self) -> ControlledStopCommand {
        ControlledStopCommand {
            controlword: CONTROLWORD_DISABLE_VOLTAGE,
            mode: self.mode,
            target: None,
            action: self.action,
            phase: self.phase,
        }
    }
}

impl Default for ControlledStopPlanner {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_limits(
    action: StopAction,
    mode: OperatingMode,
    limits: ControlledStopLimits,
) -> Result<(), ControlledStopError> {
    match (action, mode) {
        (StopAction::Hold, OperatingMode::Csp) => Ok(()),
        (StopAction::RampToZero, OperatingMode::Csv) if limits.max_velocity_step != 0 => Ok(()),
        (StopAction::RampToZero, OperatingMode::Cst) if limits.max_torque_step != 0 => Ok(()),
        (StopAction::RampToZero, OperatingMode::Csv | OperatingMode::Cst) => {
            Err(ControlledStopError::InvalidLimits)
        }
        (StopAction::Hold | StopAction::RampToZero, _) => Err(ControlledStopError::UnsupportedMode),
        (StopAction::QuickStop | StopAction::Disable, _) => {
            Err(ControlledStopError::NotControlledAction)
        }
    }
}

fn toward_zero_i32(value: i32, step: u32) -> i32 {
    let value = i64::from(value);
    let step = i64::from(step);
    if value.unsigned_abs() <= step as u64 {
        0
    } else if value > 0 {
        (value - step) as i32
    } else {
        (value + step) as i32
    }
}

fn toward_zero_i16(value: i16, step: u16) -> i16 {
    let value = i32::from(value);
    let step = i32::from(step);
    if value.unsigned_abs() <= step as u32 {
        0
    } else if value > 0 {
        (value - step) as i16
    } else {
        (value + step) as i16
    }
}

/// Call only after the input Domain has passed complete-frame, WKC and age
/// checks. A missing velocity sample or unknown drive state cannot confirm a stop.
pub fn stop_feedback_from_cia402(
    cycle: u64,
    axis_index: u8,
    inputs: Cia402PdoInputs,
    max_stationary_velocity: u32,
) -> Option<StopFeedback> {
    let axis_bit = 1u32.checked_shl(axis_index as u32)?;
    let drive_state = DriveState::from_statusword(inputs.statusword);
    let stationary = inputs
        .actual_velocity
        .is_some_and(|velocity| velocity.unsigned_abs() <= max_stationary_velocity);
    let non_enabled = !matches!(
        drive_state,
        DriveState::OperationEnabled | DriveState::FaultReactionActive | DriveState::Unknown
    );
    Some(StopFeedback {
        cycle,
        observed_axis_mask: axis_bit,
        stationary_axis_mask: if stationary { axis_bit } else { 0 },
        non_enabled_axis_mask: if non_enabled { axis_bit } else { 0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(statusword: u16, actual_velocity: Option<i32>) -> Cia402PdoInputs {
        Cia402PdoInputs {
            statusword,
            actual_mode: OperatingMode::Csp,
            error_code: 0,
            actual_position: None,
            actual_velocity,
            actual_torque: None,
            following_error: None,
        }
    }

    fn controlled_inputs(
        mode: OperatingMode,
        position: Option<i32>,
        velocity: Option<i32>,
        torque: Option<i16>,
    ) -> Cia402PdoInputs {
        Cia402PdoInputs {
            statusword: 0x0027,
            actual_mode: mode,
            error_code: 0,
            actual_position: position,
            actual_velocity: velocity,
            actual_torque: torque,
            following_error: None,
        }
    }

    const CONTROLLED_LIMITS: ControlledStopLimits = ControlledStopLimits {
        max_velocity_step: 30,
        max_torque_step: 4,
        max_stationary_velocity: 2,
        max_zero_torque: 1,
    };

    #[test]
    fn missing_motion_feedback_and_active_drive_cannot_confirm_stop() {
        let disabled = stop_feedback_from_cia402(7, 1, inputs(0x0040, None), 3).unwrap();
        assert_eq!(disabled.observed_axis_mask, 2);
        assert_eq!(disabled.stationary_axis_mask, 0);
        assert_eq!(disabled.non_enabled_axis_mask, 2);

        let active = stop_feedback_from_cia402(7, 1, inputs(0x0027, Some(0)), 3).unwrap();
        assert_eq!(active.non_enabled_axis_mask, 0);
        assert_eq!(active.stationary_axis_mask, 2);
        let unknown = stop_feedback_from_cia402(7, 1, inputs(0xFFFF, Some(0)), 3).unwrap();
        assert_eq!(unknown.non_enabled_axis_mask, 0);
        assert_eq!(
            stop_feedback_from_cia402(7, 32, inputs(0x0040, Some(0)), 3),
            None
        );
    }

    #[test]
    fn velocity_threshold_is_inclusive_and_safe_for_minimum_i32() {
        let within = stop_feedback_from_cia402(8, 31, inputs(0x0040, Some(-3)), 3).unwrap();
        assert_eq!(within.stationary_axis_mask, 1u32 << 31);
        let moving = stop_feedback_from_cia402(8, 31, inputs(0x0040, Some(i32::MIN)), 3).unwrap();
        assert_eq!(moving.stationary_axis_mask, 0);
    }

    #[test]
    fn hold_preview_is_transactional_and_locks_the_first_verified_position() {
        let mut planner = ControlledStopPlanner::new();
        let first = planner
            .preview(
                4,
                10,
                StopAction::Hold,
                OperatingMode::Csp,
                controlled_inputs(OperatingMode::Csp, Some(120), Some(20), None),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(planner.phase(), ControlledStopPhase::Idle);
        assert_eq!(
            first.command(),
            ControlledStopCommand {
                controlword: CONTROLWORD_ENABLE_OPERATION,
                mode: OperatingMode::Csp,
                target: Some(Cia402Target::Position(120)),
                action: StopAction::Hold,
                phase: ControlledStopPhase::Applying,
            }
        );
        planner.commit(first);

        let moving = planner
            .preview(
                4,
                11,
                StopAction::Hold,
                OperatingMode::Csp,
                controlled_inputs(OperatingMode::Csp, Some(145), Some(5), None),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(moving.command().target, Some(Cia402Target::Position(120)));
        planner.commit(moving);

        let stationary = planner
            .preview(
                4,
                12,
                StopAction::Hold,
                OperatingMode::Csp,
                controlled_inputs(OperatingMode::Csp, Some(121), Some(2), None),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(stationary.command().target, None);
        assert_eq!(
            stationary.command().phase,
            ControlledStopPhase::DisableRequested
        );
    }

    #[test]
    fn csv_ramp_is_bounded_from_current_feedback_and_disables_after_zero_response() {
        let mut planner = ControlledStopPlanner::new();
        for (cycle, actual, expected) in [(20, 100, 70), (21, 65, 35), (22, 20, 0)] {
            let preview = planner
                .preview(
                    7,
                    cycle,
                    StopAction::RampToZero,
                    OperatingMode::Csv,
                    controlled_inputs(OperatingMode::Csv, None, Some(actual), None),
                    CONTROLLED_LIMITS,
                )
                .unwrap();
            assert_eq!(
                preview.command().target,
                Some(Cia402Target::Velocity(expected))
            );
            planner.commit(preview);
        }
        assert_eq!(planner.phase(), ControlledStopPhase::ZeroCommanded);

        let disable = planner
            .preview(
                7,
                23,
                StopAction::RampToZero,
                OperatingMode::Csv,
                controlled_inputs(OperatingMode::Csv, None, Some(-2), None),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(disable.command().controlword, CONTROLWORD_DISABLE_VOLTAGE);
        assert_eq!(disable.command().target, None);
    }

    #[test]
    fn cst_ramp_requires_zero_torque_and_stationary_velocity_before_disable() {
        let mut planner = ControlledStopPlanner::new();
        let zero = planner
            .preview(
                9,
                30,
                StopAction::RampToZero,
                OperatingMode::Cst,
                controlled_inputs(OperatingMode::Cst, None, Some(1), Some(-3)),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(zero.command().target, Some(Cia402Target::Torque(0)));
        planner.commit(zero);

        let retained = planner
            .preview(
                9,
                31,
                StopAction::RampToZero,
                OperatingMode::Cst,
                controlled_inputs(OperatingMode::Cst, None, Some(1), Some(2)),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(retained.command().target, Some(Cia402Target::Torque(0)));
        planner.commit(retained);

        let disable = planner
            .preview(
                9,
                32,
                StopAction::RampToZero,
                OperatingMode::Cst,
                controlled_inputs(OperatingMode::Cst, None, Some(0), Some(1)),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(
            disable.command().phase,
            ControlledStopPhase::DisableRequested
        );
    }

    #[test]
    fn controlled_stop_rejects_unqualified_modes_limits_and_replays() {
        let planner = ControlledStopPlanner::new();
        assert_eq!(
            planner.preview(
                1,
                1,
                StopAction::Hold,
                OperatingMode::Csv,
                controlled_inputs(OperatingMode::Csv, Some(1), Some(0), None),
                CONTROLLED_LIMITS,
            ),
            Err(ControlledStopError::UnsupportedMode)
        );
        assert_eq!(
            planner.preview(
                1,
                1,
                StopAction::RampToZero,
                OperatingMode::Csv,
                controlled_inputs(OperatingMode::Csv, None, Some(10), None),
                ControlledStopLimits {
                    max_velocity_step: 0,
                    ..CONTROLLED_LIMITS
                },
            ),
            Err(ControlledStopError::InvalidLimits)
        );

        let first = planner
            .preview(
                2,
                5,
                StopAction::RampToZero,
                OperatingMode::Csv,
                controlled_inputs(OperatingMode::Csv, None, Some(i32::MIN), None),
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(
            first.command().target,
            Some(Cia402Target::Velocity(i32::MIN + 30))
        );
        let mut committed = planner;
        committed.commit(first);
        assert_eq!(
            committed.preview(
                2,
                5,
                StopAction::RampToZero,
                OperatingMode::Csv,
                controlled_inputs(OperatingMode::Csv, None, Some(0), None),
                CONTROLLED_LIMITS,
            ),
            Err(ControlledStopError::CycleReplayed)
        );
        assert_eq!(
            committed.preview(
                1,
                6,
                StopAction::RampToZero,
                OperatingMode::Csv,
                controlled_inputs(OperatingMode::Csv, None, Some(0), None),
                CONTROLLED_LIMITS,
            ),
            Err(ControlledStopError::StopSequenceReplayed)
        );
        assert_eq!(
            committed.preview(
                2,
                6,
                StopAction::RampToZero,
                OperatingMode::Csv,
                controlled_inputs(OperatingMode::Csv, None, Some(0), None),
                ControlledStopLimits {
                    max_velocity_step: CONTROLLED_LIMITS.max_velocity_step + 1,
                    ..CONTROLLED_LIMITS
                },
            ),
            Err(ControlledStopError::LimitsChanged)
        );
    }

    #[test]
    fn already_disabled_hold_does_not_require_motion_feedback() {
        let planner = ControlledStopPlanner::new();
        let preview = planner
            .preview(
                3,
                7,
                StopAction::Hold,
                OperatingMode::Csp,
                Cia402PdoInputs {
                    statusword: 0x0040,
                    actual_mode: OperatingMode::Unknown,
                    error_code: 0,
                    actual_position: None,
                    actual_velocity: None,
                    actual_torque: None,
                    following_error: None,
                },
                CONTROLLED_LIMITS,
            )
            .unwrap();
        assert_eq!(preview.command().controlword, CONTROLWORD_DISABLE_VOLTAGE);
        assert_eq!(preview.command().target, None);
        assert_eq!(
            preview.command().phase,
            ControlledStopPhase::DisableRequested
        );
    }
}
