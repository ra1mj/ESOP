#![no_std]

pub const MAX_GATES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GateId {
    Platform = 0,
    Configuration = 1,
    Topology = 2,
    Link = 3,
    Domain = 4,
    DistributedClock = 5,
    Drive = 6,
    Command = 7,
    Supervisor = 8,
    Budget = 9,
    ExternalSafety = 10,
    HostObservation = 11,
}

impl GateId {
    pub const fn bit(self) -> u16 {
        1u16 << (self as u8)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StopAction {
    Hold = 0,
    RampToZero = 1,
    QuickStop = 2,
    Disable = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ObservationState {
    Healthy = 0,
    Degraded = 1,
    Failed = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct HostObservation {
    pub boot_id: u64,
    pub agent_epoch: u64,
    pub heartbeat_seq: u64,
    pub observed_at_ns: u64,
    pub state: ObservationState,
    pub reserved: [u8; 7],
    pub attach_mask: u64,
    pub lost_event_count: u32,
    pub incident_count: u32,
    pub fault_code: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostObservationError {
    BootMismatch,
    FutureTimestamp,
    Stale,
    EpochReplayed,
    HeartbeatReplayed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuardPolicy {
    pub enter_good_cycles: u16,
    pub exit_bad_cycles: u16,
    pub max_age_cycles: u64,
    pub stop_action: StopAction,
}

/// Cross-layer quality facts collected by the cycle owner. Each field is an
/// observation only; the guard remains the sole authority for motion enable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CyclicQuality {
    pub coe_ready: bool,
    pub distributed_clock_locked: bool,
    pub drive_ready: bool,
    pub domain_valid: bool,
    pub wkc_valid: bool,
    pub cycle_within_budget: bool,
}

impl GuardPolicy {
    pub const fn conservative() -> Self {
        Self {
            enter_good_cycles: 3,
            exit_bad_cycles: 2,
            max_age_cycles: 1,
            stop_action: StopAction::QuickStop,
        }
    }

    const fn normalized(self) -> Self {
        Self {
            enter_good_cycles: if self.enter_good_cycles == 0 {
                1
            } else {
                self.enter_good_cycles
            },
            exit_bad_cycles: if self.exit_bad_cycles == 0 {
                1
            } else {
                self.exit_bad_cycles
            },
            max_age_cycles: self.max_age_cycles,
            stop_action: self.stop_action,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum LifecycleState {
    Qualifying = 0,
    Ready = 1,
    Active = 2,
    Stopping = 3,
    FaultLatched = 4,
    Maintenance = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleAction {
    Hold,
    EnableAllowed,
    Stop(StopAction),
    FaultLatched,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GateStatus {
    pub valid: bool,
    pub qualified: bool,
    pub good_cycles: u16,
    pub bad_cycles: u16,
    pub last_update_cycle: u64,
    pub fault_code: u32,
}

impl GateStatus {
    pub const EMPTY: Self = Self {
        valid: false,
        qualified: false,
        good_cycles: 0,
        bad_cycles: 0,
        last_update_cycle: 0,
        fault_code: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MotionPermit {
    pub boot_id: u64,
    pub permit_epoch: u64,
    pub sequence: u64,
    pub axis_mask: u32,
    pub expires_at_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermitError {
    BootMismatch,
    Expired,
    EmptyAxisMask,
    EpochReplayed,
    SequenceReplayed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    InvalidState,
    NotReady,
    Permit(PermitError),
}

pub struct LifecycleGuard {
    policy: GuardPolicy,
    required_mask: u16,
    gates: [GateStatus; MAX_GATES],
    boot_id: u64,
    permit: Option<MotionPermit>,
    permit_epoch: u64,
    last_permit_sequence: u64,
    state: LifecycleState,
    state_since_cycle: u64,
    first_fault_code: u32,
    host_observation: Option<HostObservation>,
    host_observation_epoch: u64,
    host_heartbeat_seq: u64,
}

impl LifecycleGuard {
    pub const fn new(required_mask: u16, boot_id: u64, policy: GuardPolicy) -> Self {
        Self {
            policy: policy.normalized(),
            required_mask,
            gates: [GateStatus::EMPTY; MAX_GATES],
            boot_id,
            permit: None,
            permit_epoch: 0,
            last_permit_sequence: 0,
            state: LifecycleState::Qualifying,
            state_since_cycle: 0,
            first_fault_code: 0,
            host_observation: None,
            host_observation_epoch: 0,
            host_heartbeat_seq: 0,
        }
    }

    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    pub const fn state_since_cycle(&self) -> u64 {
        self.state_since_cycle
    }

    pub const fn required_mask(&self) -> u16 {
        self.required_mask
    }

    pub const fn first_fault_code(&self) -> u32 {
        self.first_fault_code
    }

    pub const fn host_observation(&self) -> Option<HostObservation> {
        self.host_observation
    }

    pub const fn gate(&self, gate: GateId) -> GateStatus {
        self.gates[gate as usize]
    }

    pub fn update_gate(&mut self, gate: GateId, valid: bool, cycle: u64, fault_code: u32) {
        let status = &mut self.gates[gate as usize];
        status.last_update_cycle = cycle;
        if valid {
            let was_valid = status.valid;
            status.valid = true;
            status.good_cycles = if was_valid {
                status.good_cycles.saturating_add(1)
            } else {
                1
            };
            status.bad_cycles = 0;
            status.fault_code = 0;
            if status.good_cycles >= self.policy.enter_good_cycles {
                status.qualified = true;
            }
        } else {
            let was_valid = status.valid;
            status.valid = false;
            status.good_cycles = 0;
            status.bad_cycles = if was_valid {
                1
            } else {
                status.bad_cycles.saturating_add(1)
            };
            status.fault_code = fault_code;
            if status.bad_cycles >= self.policy.exit_bad_cycles {
                status.qualified = false;
                if self.first_fault_code == 0 {
                    self.first_fault_code = fault_code;
                }
            }
        }
    }

    /// Project the cycle owner's quality snapshot into the corresponding
    /// lifecycle gates using stable fault-code namespaces.
    pub fn update_cyclic_quality(&mut self, quality: CyclicQuality, cycle: u64) {
        self.update_gate(GateId::Configuration, quality.coe_ready, cycle, 0x434F_0001);
        self.update_gate(
            GateId::DistributedClock,
            quality.distributed_clock_locked,
            cycle,
            0x4443_0001,
        );
        self.update_gate(GateId::Drive, quality.drive_ready, cycle, 0x4452_0001);
        self.update_gate(GateId::Domain, quality.domain_valid, cycle, 0x444F_0001);
        self.update_gate(GateId::Link, quality.wkc_valid, cycle, 0x574B_0001);
        self.update_gate(
            GateId::Budget,
            quality.cycle_within_budget,
            cycle,
            0x4255_0001,
        );
    }

    /// Accept one fixed-size Linux/eBPF observation heartbeat and project it
    /// into the dedicated host-observation gate. This is evidence only: it
    /// cannot grant a permit or alter an active state directly.
    pub fn update_host_observation(
        &mut self,
        observation: HostObservation,
        cycle: u64,
        now_ns: u64,
        max_age_ns: u64,
    ) -> Result<(), HostObservationError> {
        if observation.boot_id != self.boot_id {
            return self.reject_host_observation(cycle, HostObservationError::BootMismatch);
        }
        if observation.observed_at_ns > now_ns {
            return self.reject_host_observation(cycle, HostObservationError::FutureTimestamp);
        }
        if now_ns.saturating_sub(observation.observed_at_ns) > max_age_ns {
            return self.reject_host_observation(cycle, HostObservationError::Stale);
        }
        if self.host_observation.is_some() {
            if observation.agent_epoch < self.host_observation_epoch {
                return self.reject_host_observation(cycle, HostObservationError::EpochReplayed);
            }
            if observation.agent_epoch == self.host_observation_epoch
                && observation.heartbeat_seq <= self.host_heartbeat_seq
            {
                return self
                    .reject_host_observation(cycle, HostObservationError::HeartbeatReplayed);
            }
        }

        self.host_observation_epoch = observation.agent_epoch;
        self.host_heartbeat_seq = observation.heartbeat_seq;
        self.host_observation = Some(observation);
        let valid = observation.state == ObservationState::Healthy;
        let fault_code = if valid {
            0
        } else if observation.fault_code != 0 {
            observation.fault_code
        } else {
            match observation.state {
                ObservationState::Healthy => 0,
                ObservationState::Degraded => 0x484F_1001,
                ObservationState::Failed => 0x484F_1002,
            }
        };
        self.update_gate(GateId::HostObservation, valid, cycle, fault_code);
        Ok(())
    }

    pub fn accept_permit(&mut self, permit: MotionPermit, now_ns: u64) -> Result<(), PermitError> {
        if permit.boot_id != self.boot_id {
            return Err(PermitError::BootMismatch);
        }
        if permit.expires_at_ns <= now_ns {
            return Err(PermitError::Expired);
        }
        if permit.axis_mask == 0 {
            return Err(PermitError::EmptyAxisMask);
        }
        if permit.permit_epoch < self.permit_epoch {
            return Err(PermitError::EpochReplayed);
        }
        if permit.permit_epoch == self.permit_epoch && permit.sequence <= self.last_permit_sequence
        {
            return Err(PermitError::SequenceReplayed);
        }
        self.permit_epoch = permit.permit_epoch;
        self.last_permit_sequence = permit.sequence;
        self.permit = Some(permit);
        Ok(())
    }

    pub fn revoke_permit(&mut self) {
        self.permit = None;
        self.permit_epoch = self.permit_epoch.saturating_add(1);
        self.last_permit_sequence = 0;
    }

    pub fn set_maintenance(&mut self, enabled: bool, cycle: u64) {
        if enabled {
            self.revoke_permit();
            self.transition(LifecycleState::Maintenance, cycle);
        } else if self.state == LifecycleState::Maintenance {
            self.transition(LifecycleState::Qualifying, cycle);
        }
    }

    pub fn acknowledge_stopped(&mut self, cycle: u64) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Stopping {
            return Err(LifecycleError::InvalidState);
        }
        self.revoke_permit();
        self.transition(LifecycleState::Ready, cycle);
        Ok(())
    }

    pub fn latch_fault(&mut self, code: u32, cycle: u64) {
        self.revoke_permit();
        self.first_fault_code = code;
        self.transition(LifecycleState::FaultLatched, cycle);
    }

    pub fn clear_fault(&mut self, cycle: u64) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::FaultLatched {
            return Err(LifecycleError::InvalidState);
        }
        self.first_fault_code = 0;
        self.transition(LifecycleState::Qualifying, cycle);
        Ok(())
    }

    pub fn request_rearm(
        &mut self,
        permit: MotionPermit,
        cycle: u64,
        now_ns: u64,
    ) -> Result<LifecycleAction, LifecycleError> {
        if !matches!(
            self.state,
            LifecycleState::Ready | LifecycleState::Qualifying
        ) {
            return Err(LifecycleError::InvalidState);
        }
        self.accept_permit(permit, now_ns)
            .map_err(LifecycleError::Permit)?;
        if !self.gates_ready(cycle) {
            return Err(LifecycleError::NotReady);
        }
        self.transition(LifecycleState::Active, cycle);
        Ok(LifecycleAction::EnableAllowed)
    }

    pub fn cycle(&mut self, cycle: u64, now_ns: u64) -> LifecycleAction {
        if self.state == LifecycleState::Maintenance {
            return LifecycleAction::Hold;
        }
        if self.state == LifecycleState::FaultLatched {
            return LifecycleAction::FaultLatched;
        }

        let permit_current = self.permit_current(now_ns);
        match self.state {
            LifecycleState::Qualifying | LifecycleState::Ready => {
                if self.gates_ready(cycle) && permit_current {
                    self.transition(LifecycleState::Ready, cycle);
                } else {
                    self.transition(LifecycleState::Qualifying, cycle);
                }
                LifecycleAction::Hold
            }
            LifecycleState::Active => {
                if self.gates_active(cycle) && permit_current {
                    LifecycleAction::EnableAllowed
                } else {
                    self.transition(LifecycleState::Stopping, cycle);
                    LifecycleAction::Stop(self.policy.stop_action)
                }
            }
            LifecycleState::Stopping => LifecycleAction::Stop(self.policy.stop_action),
            LifecycleState::Maintenance | LifecycleState::FaultLatched => unreachable!(),
        }
    }

    fn permit_current(&self, now_ns: u64) -> bool {
        self.permit
            .map(|permit| {
                permit.boot_id == self.boot_id
                    && permit.axis_mask != 0
                    && permit.expires_at_ns > now_ns
            })
            .unwrap_or(false)
    }

    fn gates_ready(&self, cycle: u64) -> bool {
        self.required_mask == 0
            || (0..MAX_GATES).all(|index| {
                let bit = 1u16 << index;
                bit & self.required_mask == 0 || self.gate_ready(self.gates[index], cycle)
            })
    }

    fn gates_active(&self, cycle: u64) -> bool {
        self.required_mask == 0
            || (0..MAX_GATES).all(|index| {
                let bit = 1u16 << index;
                bit & self.required_mask == 0 || self.gate_active(self.gates[index], cycle)
            })
    }

    fn gate_ready(&self, status: GateStatus, cycle: u64) -> bool {
        status.qualified
            && status.valid
            && cycle.saturating_sub(status.last_update_cycle) <= self.policy.max_age_cycles
    }

    fn gate_active(&self, status: GateStatus, cycle: u64) -> bool {
        status.qualified
            && cycle.saturating_sub(status.last_update_cycle) <= self.policy.max_age_cycles
    }

    fn transition(&mut self, state: LifecycleState, cycle: u64) {
        if self.state != state {
            self.state = state;
            self.state_since_cycle = cycle;
        }
    }

    fn reject_host_observation(
        &mut self,
        cycle: u64,
        error: HostObservationError,
    ) -> Result<(), HostObservationError> {
        self.update_gate(
            GateId::HostObservation,
            false,
            cycle,
            host_observation_error_code(error),
        );
        Err(error)
    }
}

const fn host_observation_error_code(error: HostObservationError) -> u32 {
    match error {
        HostObservationError::BootMismatch => 0x484F_0001,
        HostObservationError::FutureTimestamp => 0x484F_0002,
        HostObservationError::Stale => 0x484F_0003,
        HostObservationError::EpochReplayed => 0x484F_0004,
        HostObservationError::HeartbeatReplayed => 0x484F_0005,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclic_quality_projects_runtime_gates() {
        let required = GateId::Configuration.bit()
            | GateId::DistributedClock.bit()
            | GateId::Drive.bit()
            | GateId::Domain.bit()
            | GateId::Link.bit()
            | GateId::Budget.bit();
        let policy = GuardPolicy {
            enter_good_cycles: 1,
            exit_bad_cycles: 1,
            max_age_cycles: 1,
            stop_action: StopAction::QuickStop,
        };
        let mut guard = LifecycleGuard::new(required, 1, policy);
        guard.update_cyclic_quality(
            CyclicQuality {
                coe_ready: true,
                distributed_clock_locked: true,
                drive_ready: true,
                domain_valid: true,
                wkc_valid: true,
                cycle_within_budget: true,
            },
            1,
        );
        guard
            .accept_permit(
                MotionPermit {
                    boot_id: 1,
                    permit_epoch: 1,
                    sequence: 1,
                    axis_mask: 1,
                    expires_at_ns: 100,
                },
                1,
            )
            .unwrap();
        assert_eq!(guard.cycle(1, 1), LifecycleAction::Hold);
        guard.update_cyclic_quality(
            CyclicQuality {
                coe_ready: true,
                distributed_clock_locked: true,
                drive_ready: true,
                domain_valid: true,
                wkc_valid: false,
                cycle_within_budget: true,
            },
            2,
        );
        assert_eq!(guard.gate(GateId::Link).fault_code, 0x574B_0001);
    }

    const POLICY: GuardPolicy = GuardPolicy {
        enter_good_cycles: 2,
        exit_bad_cycles: 2,
        max_age_cycles: 1,
        stop_action: StopAction::QuickStop,
    };

    fn permit(sequence: u64, expires_at_ns: u64) -> MotionPermit {
        MotionPermit {
            boot_id: 10,
            permit_epoch: 1,
            sequence,
            axis_mask: 0x03,
            expires_at_ns,
        }
    }

    fn observation(epoch: u64, heartbeat_seq: u64, state: ObservationState) -> HostObservation {
        HostObservation {
            boot_id: 10,
            agent_epoch: epoch,
            heartbeat_seq,
            observed_at_ns: 100,
            state,
            reserved: [0; 7],
            attach_mask: 0x03,
            lost_event_count: 0,
            incident_count: 0,
            fault_code: if state == ObservationState::Healthy {
                0
            } else {
                0xBEEF
            },
        }
    }

    #[test]
    fn guard_requires_good_window_and_current_permit() {
        let mut guard =
            LifecycleGuard::new(GateId::Platform.bit() | GateId::Link.bit(), 10, POLICY);
        guard.update_gate(GateId::Platform, true, 1, 0);
        guard.update_gate(GateId::Link, true, 1, 0);
        guard.accept_permit(permit(1, 100), 1).unwrap();
        assert_eq!(guard.cycle(1, 1), LifecycleAction::Hold);
        guard.update_gate(GateId::Platform, true, 2, 0);
        guard.update_gate(GateId::Link, true, 2, 0);
        assert_eq!(
            guard.request_rearm(permit(2, 100), 2, 2),
            Ok(LifecycleAction::EnableAllowed)
        );
        assert_eq!(guard.state(), LifecycleState::Active);
    }

    #[test]
    fn active_guard_stops_after_bad_window_and_never_auto_rearms() {
        let mut guard = LifecycleGuard::new(GateId::Link.bit(), 10, POLICY);
        guard.update_gate(GateId::Link, true, 1, 0);
        guard.update_gate(GateId::Link, true, 2, 0);
        guard.request_rearm(permit(1, 100), 2, 2).unwrap();
        guard.update_gate(GateId::Link, false, 3, 0xCAFE);
        assert_eq!(guard.cycle(3, 3), LifecycleAction::EnableAllowed);
        guard.update_gate(GateId::Link, false, 4, 0xCAFE);
        assert_eq!(
            guard.cycle(4, 4),
            LifecycleAction::Stop(StopAction::QuickStop)
        );
        assert_eq!(guard.state(), LifecycleState::Stopping);
        guard.acknowledge_stopped(5).unwrap();
        assert_eq!(guard.cycle(5, 5), LifecycleAction::Hold);
        assert_eq!(guard.state(), LifecycleState::Qualifying);
    }

    #[test]
    fn permit_replay_and_expiry_are_rejected() {
        let mut guard = LifecycleGuard::new(0, 10, POLICY);
        guard.accept_permit(permit(1, 10), 1).unwrap();
        assert_eq!(
            guard.accept_permit(permit(1, 10), 2),
            Err(PermitError::SequenceReplayed)
        );
        assert_eq!(
            guard.accept_permit(permit(2, 10), 10),
            Err(PermitError::Expired)
        );
        assert_eq!(guard.accept_permit(permit(2, 10), 9), Ok(()));
    }

    #[test]
    fn host_observation_heartbeat_is_a_fail_closed_lifecycle_gate() {
        let mut guard = LifecycleGuard::new(GateId::HostObservation.bit(), 10, POLICY);
        guard
            .update_host_observation(observation(1, 1, ObservationState::Healthy), 1, 100, 10)
            .unwrap();
        guard.accept_permit(permit(1, 1_000), 100).unwrap();
        assert_eq!(guard.cycle(1, 100), LifecycleAction::Hold);

        guard
            .update_host_observation(observation(1, 2, ObservationState::Healthy), 2, 100, 10)
            .unwrap();
        assert_eq!(
            guard.request_rearm(permit(2, 1_000), 2, 100),
            Ok(LifecycleAction::EnableAllowed)
        );
        assert_eq!(guard.state(), LifecycleState::Active);

        guard
            .update_host_observation(observation(1, 3, ObservationState::Degraded), 3, 100, 10)
            .unwrap();
        assert_eq!(guard.cycle(3, 100), LifecycleAction::EnableAllowed);
        guard
            .update_host_observation(observation(1, 4, ObservationState::Degraded), 4, 100, 10)
            .unwrap();
        assert_eq!(
            guard.cycle(4, 100),
            LifecycleAction::Stop(StopAction::QuickStop)
        );
        assert_eq!(guard.host_observation().unwrap().heartbeat_seq, 4);
    }

    #[test]
    fn host_observation_rejects_replay_future_and_stale_inputs() {
        let mut guard = LifecycleGuard::new(GateId::HostObservation.bit(), 10, POLICY);
        guard
            .update_host_observation(observation(2, 0, ObservationState::Healthy), 1, 100, 10)
            .unwrap();
        assert_eq!(
            guard.update_host_observation(observation(1, 1, ObservationState::Healthy), 2, 100, 10),
            Err(HostObservationError::EpochReplayed)
        );
        assert_eq!(
            guard.update_host_observation(observation(2, 0, ObservationState::Healthy), 3, 100, 10),
            Err(HostObservationError::HeartbeatReplayed)
        );
        let mut future = observation(2, 5, ObservationState::Healthy);
        future.observed_at_ns = 101;
        assert_eq!(
            guard.update_host_observation(future, 4, 100, 10),
            Err(HostObservationError::FutureTimestamp)
        );
        let mut stale = observation(2, 6, ObservationState::Healthy);
        stale.observed_at_ns = 80;
        assert_eq!(
            guard.update_host_observation(stale, 5, 100, 10),
            Err(HostObservationError::Stale)
        );
        assert!(!guard.gate(GateId::HostObservation).valid);
    }
}
