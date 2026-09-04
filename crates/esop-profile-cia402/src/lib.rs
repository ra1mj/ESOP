#![no_std]

//! Allocation-free CiA 402 PDS finite-state handling.
//!
//! This crate only owns profile semantics. EtherCAT framing, PDO layout,
//! Mailbox transport and the motion lifecycle guard remain separate layers.
//! The output is deliberately safe to apply every cycle: an unknown or
//! invalid Statusword never produces an operation-enable Controlword.

pub const CONTROLWORD_DISABLE_VOLTAGE: u16 = 0x0000;
pub const CONTROLWORD_SHUTDOWN: u16 = 0x0006;
pub const CONTROLWORD_SWITCH_ON: u16 = 0x0007;
pub const CONTROLWORD_ENABLE_OPERATION: u16 = 0x000F;
pub const CONTROLWORD_QUICK_STOP: u16 = 0x0002;
pub const CONTROLWORD_FAULT_RESET: u16 = 0x0080;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OperatingMode {
    Csp = 8,
    Csv = 9,
    Cst = 10,
    Unknown = 255,
}

impl OperatingMode {
    pub const fn from_raw(value: i8) -> Self {
        match value {
            8 => Self::Csp,
            9 => Self::Csv,
            10 => Self::Cst,
            _ => Self::Unknown,
        }
    }

    pub const fn raw(self) -> i8 {
        match self {
            Self::Csp => 8,
            Self::Csv => 9,
            Self::Cst => 10,
            Self::Unknown => 0,
        }
    }

    pub const fn is_cyclic(self) -> bool {
        matches!(self, Self::Csp | Self::Csv | Self::Cst)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ModeSwitchState {
    Unconfigured = 0,
    AwaitingConfirmation = 1,
    Ready = 2,
    TimedOut = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeSwitchAction {
    HoldDisabled,
    SetMode,
    Ready,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeRequestError {
    UnsupportedMode,
    ZeroTimeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeSwitchOutput {
    pub state: ModeSwitchState,
    pub requested_mode: OperatingMode,
    pub actual_mode: OperatingMode,
    pub mode_command: i8,
    pub action: ModeSwitchAction,
    pub confirmed: bool,
    pub cyclic_allowed: bool,
}

/// Supervises 0x6060/0x6061 mode changes independently from the PDS FSA.
/// A mode is only usable after consecutive actual-mode confirmations, and
/// cyclic output additionally requires the drive to be operation-enabled.
pub struct ModeSupervisor {
    requested_mode: OperatingMode,
    state: ModeSwitchState,
    confirmation_cycles: u16,
    confirmed_cycles: u16,
    deadline_cycle: u64,
}

impl ModeSupervisor {
    pub const fn new(confirmation_cycles: u16) -> Self {
        Self {
            requested_mode: OperatingMode::Unknown,
            state: ModeSwitchState::Unconfigured,
            confirmation_cycles: if confirmation_cycles == 0 {
                1
            } else {
                confirmation_cycles
            },
            confirmed_cycles: 0,
            deadline_cycle: 0,
        }
    }

    pub const fn state(&self) -> ModeSwitchState {
        self.state
    }

    pub const fn requested_mode(&self) -> OperatingMode {
        self.requested_mode
    }

    pub fn request(
        &mut self,
        mode: OperatingMode,
        cycle: u64,
        timeout_cycles: u64,
    ) -> Result<(), ModeRequestError> {
        if !mode.is_cyclic() {
            return Err(ModeRequestError::UnsupportedMode);
        }
        if timeout_cycles == 0 {
            return Err(ModeRequestError::ZeroTimeout);
        }
        self.requested_mode = mode;
        self.state = ModeSwitchState::AwaitingConfirmation;
        self.confirmed_cycles = 0;
        self.deadline_cycle = cycle.saturating_add(timeout_cycles);
        Ok(())
    }

    pub fn step(
        &mut self,
        actual_mode_raw: i8,
        drive_state: DriveState,
        cycle: u64,
    ) -> ModeSwitchOutput {
        let actual_mode = OperatingMode::from_raw(actual_mode_raw);
        if self.state == ModeSwitchState::Unconfigured {
            return self.output(
                actual_mode,
                drive_state,
                ModeSwitchAction::HoldDisabled,
                false,
            );
        }
        if self.state == ModeSwitchState::TimedOut {
            return self.output(actual_mode, drive_state, ModeSwitchAction::Timeout, false);
        }
        if actual_mode == self.requested_mode {
            self.confirmed_cycles = self.confirmed_cycles.saturating_add(1);
            if self.confirmed_cycles >= self.confirmation_cycles {
                self.state = ModeSwitchState::Ready;
                return self.output(actual_mode, drive_state, ModeSwitchAction::Ready, true);
            }
            return self.output(
                actual_mode,
                drive_state,
                ModeSwitchAction::HoldDisabled,
                false,
            );
        }

        self.confirmed_cycles = 0;
        if cycle >= self.deadline_cycle {
            self.state = ModeSwitchState::TimedOut;
            return self.output(actual_mode, drive_state, ModeSwitchAction::Timeout, false);
        }
        let action = if drive_state.is_operation_enabled() {
            ModeSwitchAction::HoldDisabled
        } else {
            ModeSwitchAction::SetMode
        };
        self.output(actual_mode, drive_state, action, false)
    }

    fn output(
        &self,
        actual_mode: OperatingMode,
        drive_state: DriveState,
        action: ModeSwitchAction,
        confirmed: bool,
    ) -> ModeSwitchOutput {
        ModeSwitchOutput {
            state: self.state,
            requested_mode: self.requested_mode,
            actual_mode,
            mode_command: self.requested_mode.raw(),
            action,
            confirmed,
            cyclic_allowed: confirmed
                && actual_mode == self.requested_mode
                && drive_state.is_operation_enabled(),
        }
    }
}

impl Default for ModeSupervisor {
    fn default() -> Self {
        Self::new(2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CyclicSetpoint {
    pub position: f64,
    pub velocity: f64,
    pub torque: f64,
}

impl CyclicSetpoint {
    pub const ZERO: Self = Self {
        position: 0.0,
        velocity: 0.0,
        torque: 0.0,
    };

    fn finite(self) -> bool {
        self.position.is_finite() && self.velocity.is_finite() && self.torque.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CyclicLimits {
    pub max_position_step: f64,
    pub max_velocity: f64,
    pub max_torque: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CyclicSetpointError {
    ModeNotReady,
    NotSeeded,
    NonFinite,
    InvalidLimits,
    PositionStepExceeded,
    VelocityLimitExceeded,
    TorqueLimitExceeded,
}

/// Guards the first cyclic target and every subsequent setpoint against a
/// mode transition jump or a configured velocity/torque limit violation.
pub struct CyclicSetpointGuard {
    seeded: bool,
    last: CyclicSetpoint,
}

impl CyclicSetpointGuard {
    pub const fn new() -> Self {
        Self {
            seeded: false,
            last: CyclicSetpoint::ZERO,
        }
    }

    pub const fn seeded(&self) -> bool {
        self.seeded
    }

    pub const fn last(&self) -> CyclicSetpoint {
        self.last
    }

    pub fn seed_from_actual(&mut self, actual: CyclicSetpoint) -> Result<(), CyclicSetpointError> {
        if !actual.finite() {
            return Err(CyclicSetpointError::NonFinite);
        }
        self.last = actual;
        self.seeded = true;
        Ok(())
    }

    pub fn validate_and_accept(
        &mut self,
        mode: OperatingMode,
        mode_ready: bool,
        setpoint: CyclicSetpoint,
        limits: CyclicLimits,
    ) -> Result<(), CyclicSetpointError> {
        if !mode.is_cyclic() || !mode_ready {
            return Err(CyclicSetpointError::ModeNotReady);
        }
        if !self.seeded {
            return Err(CyclicSetpointError::NotSeeded);
        }
        if !setpoint.finite() {
            return Err(CyclicSetpointError::NonFinite);
        }
        if !limits.max_position_step.is_finite()
            || !limits.max_velocity.is_finite()
            || !limits.max_torque.is_finite()
            || limits.max_position_step < 0.0
            || limits.max_velocity < 0.0
            || limits.max_torque < 0.0
        {
            return Err(CyclicSetpointError::InvalidLimits);
        }
        if (setpoint.position - self.last.position).abs() > limits.max_position_step {
            return Err(CyclicSetpointError::PositionStepExceeded);
        }
        if setpoint.velocity.abs() > limits.max_velocity {
            return Err(CyclicSetpointError::VelocityLimitExceeded);
        }
        if setpoint.torque.abs() > limits.max_torque {
            return Err(CyclicSetpointError::TorqueLimitExceeded);
        }
        self.last = setpoint;
        Ok(())
    }
}

impl Default for CyclicSetpointGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DriveState {
    NotReadyToSwitchOn = 0,
    SwitchOnDisabled = 1,
    ReadyToSwitchOn = 2,
    SwitchedOn = 3,
    OperationEnabled = 4,
    QuickStopActive = 5,
    FaultReactionActive = 6,
    Fault = 7,
    Unknown = 255,
}

impl DriveState {
    /// Decode the CiA 402 state using the standard Statusword masks.
    pub const fn from_statusword(statusword: u16) -> Self {
        let masked_4f = statusword & 0x004F;
        let masked_6f = statusword & 0x006F;
        match () {
            _ if masked_4f == 0x0000 => Self::NotReadyToSwitchOn,
            _ if masked_4f == 0x0040 => Self::SwitchOnDisabled,
            _ if masked_6f == 0x0021 => Self::ReadyToSwitchOn,
            _ if masked_6f == 0x0023 => Self::SwitchedOn,
            _ if masked_6f == 0x0027 => Self::OperationEnabled,
            _ if masked_6f == 0x0007 => Self::QuickStopActive,
            _ if masked_4f == 0x000F => Self::FaultReactionActive,
            _ if masked_4f == 0x0008 => Self::Fault,
            _ => Self::Unknown,
        }
    }

    pub const fn is_fault(self) -> bool {
        matches!(self, Self::Fault | Self::FaultReactionActive)
    }

    pub const fn is_operation_enabled(self) -> bool {
        matches!(self, Self::OperationEnabled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DriveRequest {
    Disable = 0,
    Enable = 1,
    QuickStop = 2,
    FaultReset = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cia402Output {
    pub state: DriveState,
    pub statusword: u16,
    pub controlword: u16,
    pub operation_enabled: bool,
    pub motion_allowed: bool,
    pub fault_reset_pulse: bool,
}

pub struct Cia402Controller {
    fault_reset_issued: bool,
}

impl Cia402Controller {
    pub const fn new() -> Self {
        Self {
            fault_reset_issued: false,
        }
    }

    pub const fn fault_reset_issued(&self) -> bool {
        self.fault_reset_issued
    }

    /// Produce one cycle's Controlword under an explicit profile request.
    ///
    /// `motion_allowed` is an input from the lifecycle guard. It is an
    /// additional fail-closed constraint: when false, an Enable request is
    /// converted to Disable and can never emit `0x000F`.
    pub fn step(
        &mut self,
        statusword: u16,
        request: DriveRequest,
        motion_allowed: bool,
    ) -> Cia402Output {
        let state = DriveState::from_statusword(statusword);
        if !state.is_fault() {
            self.fault_reset_issued = false;
        }

        let enable_requested = request == DriveRequest::Enable && motion_allowed;
        let (controlword, fault_reset_pulse) = match request {
            DriveRequest::FaultReset if state == DriveState::Fault && !self.fault_reset_issued => {
                self.fault_reset_issued = true;
                (CONTROLWORD_FAULT_RESET, true)
            }
            DriveRequest::FaultReset => (CONTROLWORD_DISABLE_VOLTAGE, false),
            DriveRequest::QuickStop => (CONTROLWORD_QUICK_STOP, false),
            DriveRequest::Enable if enable_requested => (enable_controlword(state), false),
            DriveRequest::Enable | DriveRequest::Disable => (disable_controlword(state), false),
        };

        Cia402Output {
            state,
            statusword,
            controlword,
            operation_enabled: state.is_operation_enabled(),
            motion_allowed: enable_requested && state.is_operation_enabled(),
            fault_reset_pulse,
        }
    }
}

impl Default for Cia402Controller {
    fn default() -> Self {
        Self::new()
    }
}

const fn enable_controlword(state: DriveState) -> u16 {
    match state {
        DriveState::SwitchOnDisabled => CONTROLWORD_SHUTDOWN,
        DriveState::ReadyToSwitchOn => CONTROLWORD_SWITCH_ON,
        DriveState::SwitchedOn | DriveState::OperationEnabled | DriveState::QuickStopActive => {
            CONTROLWORD_ENABLE_OPERATION
        }
        DriveState::NotReadyToSwitchOn
        | DriveState::FaultReactionActive
        | DriveState::Fault
        | DriveState::Unknown => CONTROLWORD_DISABLE_VOLTAGE,
    }
}

const fn disable_controlword(state: DriveState) -> u16 {
    match state {
        DriveState::NotReadyToSwitchOn
        | DriveState::SwitchOnDisabled
        | DriveState::FaultReactionActive
        | DriveState::Fault
        | DriveState::Unknown => CONTROLWORD_DISABLE_VOLTAGE,
        DriveState::ReadyToSwitchOn
        | DriveState::SwitchedOn
        | DriveState::OperationEnabled
        | DriveState::QuickStopActive => CONTROLWORD_DISABLE_VOLTAGE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_standard_states_and_rejects_unknown_combinations() {
        let cases = [
            (0x0000, DriveState::NotReadyToSwitchOn),
            (0x0040, DriveState::SwitchOnDisabled),
            (0x0021, DriveState::ReadyToSwitchOn),
            (0x0023, DriveState::SwitchedOn),
            (0x0027, DriveState::OperationEnabled),
            (0x0007, DriveState::QuickStopActive),
            (0x000F, DriveState::FaultReactionActive),
            (0x0008, DriveState::Fault),
        ];
        for (statusword, expected) in cases {
            assert_eq!(DriveState::from_statusword(statusword), expected);
        }
        assert_eq!(DriveState::from_statusword(0x0001), DriveState::Unknown);
    }

    #[test]
    fn enable_sequence_follows_the_pds_fsa() {
        let mut controller = Cia402Controller::new();
        let sequence = [
            (0x0040, CONTROLWORD_SHUTDOWN),
            (0x0021, CONTROLWORD_SWITCH_ON),
            (0x0023, CONTROLWORD_ENABLE_OPERATION),
            (0x0027, CONTROLWORD_ENABLE_OPERATION),
        ];
        for (statusword, expected_controlword) in sequence {
            let output = controller.step(statusword, DriveRequest::Enable, true);
            assert_eq!(output.controlword, expected_controlword);
            assert!(!output.fault_reset_pulse);
        }
    }

    #[test]
    fn lifecycle_denial_cannot_emit_operation_enable() {
        let mut controller = Cia402Controller::new();
        let output = controller.step(0x0027, DriveRequest::Enable, false);
        assert_eq!(output.controlword, CONTROLWORD_DISABLE_VOLTAGE);
        assert!(!output.motion_allowed);
    }

    #[test]
    fn unknown_statusword_is_fail_closed() {
        let mut controller = Cia402Controller::new();
        let output = controller.step(0x0001, DriveRequest::Enable, true);
        assert_eq!(output.state, DriveState::Unknown);
        assert_eq!(output.controlword, CONTROLWORD_DISABLE_VOLTAGE);
        assert!(!output.motion_allowed);
    }

    #[test]
    fn fault_reset_is_a_single_edge_until_fault_clears() {
        let mut controller = Cia402Controller::new();
        let first = controller.step(0x0008, DriveRequest::FaultReset, false);
        let repeated = controller.step(0x0008, DriveRequest::FaultReset, false);
        assert_eq!(first.controlword, CONTROLWORD_FAULT_RESET);
        assert!(first.fault_reset_pulse);
        assert_eq!(repeated.controlword, CONTROLWORD_DISABLE_VOLTAGE);
        assert!(!repeated.fault_reset_pulse);

        let recovered = controller.step(0x0040, DriveRequest::Disable, false);
        assert!(!recovered.fault_reset_pulse);
        let next_fault = controller.step(0x0008, DriveRequest::FaultReset, false);
        assert_eq!(next_fault.controlword, CONTROLWORD_FAULT_RESET);
        assert!(next_fault.fault_reset_pulse);
    }

    #[test]
    fn mode_switch_stays_disabled_until_actual_mode_is_confirmed() {
        let mut supervisor = ModeSupervisor::new(2);
        supervisor.request(OperatingMode::Csv, 10, 5).unwrap();

        let changing = supervisor.step(OperatingMode::Csp.raw(), DriveState::OperationEnabled, 11);
        assert_eq!(changing.action, ModeSwitchAction::HoldDisabled);
        assert!(!changing.cyclic_allowed);
        assert_eq!(changing.mode_command, OperatingMode::Csv.raw());

        let first_confirmation =
            supervisor.step(OperatingMode::Csv.raw(), DriveState::SwitchedOn, 12);
        assert_eq!(
            first_confirmation.state,
            ModeSwitchState::AwaitingConfirmation
        );
        assert!(!first_confirmation.cyclic_allowed);

        let second_confirmation =
            supervisor.step(OperatingMode::Csv.raw(), DriveState::OperationEnabled, 13);
        assert_eq!(second_confirmation.state, ModeSwitchState::Ready);
        assert!(second_confirmation.confirmed);
        assert!(second_confirmation.cyclic_allowed);
    }

    #[test]
    fn confirmed_mode_does_not_bypass_the_pds_operation_enabled_gate() {
        let mut supervisor = ModeSupervisor::new(1);
        supervisor.request(OperatingMode::Csp, 1, 3).unwrap();
        let confirmed_while_switched_on =
            supervisor.step(OperatingMode::Csp.raw(), DriveState::SwitchedOn, 2);
        assert!(confirmed_while_switched_on.confirmed);
        assert!(!confirmed_while_switched_on.cyclic_allowed);

        let enabled = supervisor.step(OperatingMode::Csp.raw(), DriveState::OperationEnabled, 3);
        assert!(enabled.cyclic_allowed);
    }

    #[test]
    fn mode_switch_times_out_and_unknown_modes_are_rejected() {
        let mut supervisor = ModeSupervisor::new(1);
        assert_eq!(
            supervisor.request(OperatingMode::Unknown, 1, 2),
            Err(ModeRequestError::UnsupportedMode)
        );
        assert_eq!(
            supervisor.request(OperatingMode::Csp, 1, 0),
            Err(ModeRequestError::ZeroTimeout)
        );
        supervisor.request(OperatingMode::Csp, 1, 2).unwrap();
        let output = supervisor.step(OperatingMode::Csv.raw(), DriveState::SwitchedOn, 3);
        assert_eq!(output.state, ModeSwitchState::TimedOut);
        assert_eq!(output.action, ModeSwitchAction::Timeout);
        assert!(!output.cyclic_allowed);
    }

    #[test]
    fn cyclic_setpoint_requires_actual_seed_and_limits_first_target_jump() {
        let mut guard = CyclicSetpointGuard::new();
        let limits = CyclicLimits {
            max_position_step: 0.1,
            max_velocity: 2.0,
            max_torque: 3.0,
        };
        let target = CyclicSetpoint {
            position: 0.05,
            velocity: 1.0,
            torque: 1.0,
        };
        assert_eq!(
            guard.validate_and_accept(OperatingMode::Csp, true, target, limits),
            Err(CyclicSetpointError::NotSeeded)
        );
        guard
            .seed_from_actual(CyclicSetpoint::ZERO)
            .expect("zero actual is a valid seed");
        assert_eq!(
            guard.validate_and_accept(OperatingMode::Csp, true, target, limits),
            Ok(())
        );
        assert_eq!(guard.last(), target);
        assert_eq!(
            guard.validate_and_accept(
                OperatingMode::Csp,
                true,
                CyclicSetpoint {
                    position: 0.2,
                    ..target
                },
                limits,
            ),
            Err(CyclicSetpointError::PositionStepExceeded)
        );
        assert_eq!(
            guard.validate_and_accept(OperatingMode::Csv, false, target, limits,),
            Err(CyclicSetpointError::ModeNotReady)
        );
    }
}
