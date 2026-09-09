#![no_std]

pub const MAX_GATES: usize = 16;
pub const MAX_TRANSITIONS: usize = 16;
pub const MAX_PERMIT_AUDITS: usize = 16;

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
    pub authorized_source_id: u64,
    pub minimum_authority: u8,
    pub permit_policy_version: u32,
}

/// Cross-layer quality facts collected by the cycle owner. Each field is an
/// observation only; the guard remains the sole authority for motion enable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CyclicQuality {
    pub platform_ready: bool,
    pub coe_ready: bool,
    pub topology_valid: bool,
    pub distributed_clock_locked: bool,
    pub drive_ready: bool,
    pub domain_valid: bool,
    pub wkc_valid: bool,
    pub command_current: bool,
    pub supervisor_healthy: bool,
    pub external_safety_clear: bool,
    pub cycle_within_budget: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleSnapshot {
    pub state: LifecycleState,
    pub stop_action: StopAction,
    pub state_since_cycle: u64,
    pub required_gate_mask: u16,
    pub valid_gate_mask: u16,
    pub qualified_gate_mask: u16,
    pub ready_gate_mask: u16,
    pub first_blocking_code: u32,
    pub latched_fault_code: u32,
    pub motion_permit_current: bool,
    pub permit_epoch: u64,
    pub permit_expires_at_ns: u64,
    pub transition_sequence: u64,
    pub transition_cycle: u64,
    pub recovery_count: u64,
}

impl GuardPolicy {
    pub const fn conservative() -> Self {
        Self {
            enter_good_cycles: 3,
            exit_bad_cycles: 2,
            max_age_cycles: 1,
            stop_action: StopAction::QuickStop,
            authorized_source_id: 1,
            minimum_authority: 1,
            permit_policy_version: 1,
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
            authorized_source_id: self.authorized_source_id,
            minimum_authority: self.minimum_authority,
            permit_policy_version: self.permit_policy_version,
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

/// A fixed-size audit record for one lifecycle state transition.
///
/// Records are returned in chronological order by [`LifecycleGuard::transition_at`].
/// `cycle` is the RT cycle sequence, which is the guard's monotonic time base.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct LifecycleTransition {
    pub sequence: u64,
    pub cycle: u64,
    pub from: LifecycleState,
    pub to: LifecycleState,
    pub fault_code: u32,
    pub reserved: u32,
}

impl LifecycleTransition {
    pub const EMPTY: Self = Self {
        sequence: 0,
        cycle: 0,
        from: LifecycleState::Qualifying,
        to: LifecycleState::Qualifying,
        fault_code: 0,
        reserved: 0,
    };
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
#[repr(C)]
pub struct MotionPermit {
    pub boot_id: u64,
    pub source_id: u64,
    pub permit_epoch: u64,
    pub sequence: u64,
    pub expires_at_ns: u64,
    pub axis_mask: u32,
    pub authority: u8,
    pub reserved: [u8; 3],
    pub policy_version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PermitError {
    BootMismatch,
    SourceUnauthorized,
    AuthorityInsufficient,
    PolicyVersionMismatch,
    Expired,
    EmptyAxisMask,
    EpochReplayed,
    SequenceReplayed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PermitAudit {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub source_id: u64,
    pub permit_epoch: u64,
    pub permit_sequence: u64,
    pub error: PermitError,
    pub reserved: [u8; 7],
}

impl PermitAudit {
    pub const EMPTY: Self = Self {
        sequence: 0,
        timestamp_ns: 0,
        source_id: 0,
        permit_epoch: 0,
        permit_sequence: 0,
        error: PermitError::BootMismatch,
        reserved: [0; 7],
    };
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
    latched_fault_code: u32,
    recovery_count: u64,
    transitions: [LifecycleTransition; MAX_TRANSITIONS],
    transition_head: usize,
    transition_count: usize,
    transition_sequence: u64,
    host_observation: Option<HostObservation>,
    host_observation_epoch: u64,
    host_heartbeat_seq: u64,
    permit_audits: [PermitAudit; MAX_PERMIT_AUDITS],
    permit_audit_head: usize,
    permit_audit_count: usize,
    permit_audit_sequence: u64,
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
            latched_fault_code: 0,
            recovery_count: 0,
            transitions: [LifecycleTransition::EMPTY; MAX_TRANSITIONS],
            transition_head: 0,
            transition_count: 0,
            transition_sequence: 0,
            host_observation: None,
            host_observation_epoch: 0,
            host_heartbeat_seq: 0,
            permit_audits: [PermitAudit::EMPTY; MAX_PERMIT_AUDITS],
            permit_audit_head: 0,
            permit_audit_count: 0,
            permit_audit_sequence: 0,
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

    pub const fn latched_fault_code(&self) -> u32 {
        self.latched_fault_code
    }

    pub const fn recovery_count(&self) -> u64 {
        self.recovery_count
    }

    pub const fn transition_sequence(&self) -> u64 {
        self.transition_sequence
    }

    pub const fn transition_count(&self) -> usize {
        self.transition_count
    }

    pub const fn permit(&self) -> Option<MotionPermit> {
        self.permit
    }

    pub const fn permit_audit_count(&self) -> usize {
        self.permit_audit_count
    }

    /// Return permit rejections from oldest to newest.
    pub fn permit_audit_at(&self, index: usize) -> Option<PermitAudit> {
        if index >= self.permit_audit_count {
            return None;
        }
        let oldest = (self.permit_audit_head + MAX_PERMIT_AUDITS - self.permit_audit_count)
            % MAX_PERMIT_AUDITS;
        Some(self.permit_audits[(oldest + index) % MAX_PERMIT_AUDITS])
    }

    pub fn valid_gate_mask(&self) -> u16 {
        let mut mask = 0;
        for index in 0..MAX_GATES {
            if self.gates[index].valid {
                mask |= 1u16 << index;
            }
        }
        mask
    }

    pub fn qualified_gate_mask(&self) -> u16 {
        let mut mask = 0;
        for index in 0..MAX_GATES {
            if self.gates[index].qualified {
                mask |= 1u16 << index;
            }
        }
        mask
    }

    pub fn ready_gate_mask(&self, cycle: u64) -> u16 {
        let mut mask = 0;
        for index in 0..MAX_GATES {
            if self.gate_ready(self.gates[index], cycle) {
                mask |= 1u16 << index;
            }
        }
        mask
    }

    pub fn snapshot(&self, cycle: u64, now_ns: u64) -> LifecycleSnapshot {
        let latest = self.transition_at(self.transition_count.saturating_sub(1));
        let permit = self.permit.unwrap_or(MotionPermit {
            boot_id: self.boot_id,
            source_id: 0,
            permit_epoch: self.permit_epoch,
            sequence: 0,
            axis_mask: 0,
            expires_at_ns: 0,
            authority: 0,
            reserved: [0; 3],
            policy_version: 0,
        });
        LifecycleSnapshot {
            state: self.state,
            stop_action: self.policy.stop_action,
            state_since_cycle: self.state_since_cycle,
            required_gate_mask: self.required_mask,
            valid_gate_mask: self.valid_gate_mask(),
            qualified_gate_mask: self.qualified_gate_mask(),
            ready_gate_mask: self.ready_gate_mask(cycle),
            first_blocking_code: self.first_fault_code,
            latched_fault_code: self.latched_fault_code,
            motion_permit_current: self.permit_current(now_ns),
            permit_epoch: permit.permit_epoch,
            permit_expires_at_ns: permit.expires_at_ns,
            transition_sequence: self.transition_sequence,
            transition_cycle: latest.map(|transition| transition.cycle).unwrap_or(0),
            recovery_count: self.recovery_count,
        }
    }

    /// Return a transition from oldest to newest, without exposing ring slots.
    pub fn transition_at(&self, index: usize) -> Option<LifecycleTransition> {
        if index >= self.transition_count {
            return None;
        }
        let oldest =
            (self.transition_head + MAX_TRANSITIONS - self.transition_count) % MAX_TRANSITIONS;
        Some(self.transitions[(oldest + index) % MAX_TRANSITIONS])
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
        self.update_gate(GateId::Platform, quality.platform_ready, cycle, 0x504C_0001);
        self.update_gate(GateId::Configuration, quality.coe_ready, cycle, 0x434F_0001);
        self.update_gate(GateId::Topology, quality.topology_valid, cycle, 0x544F_0001);
        self.update_gate(
            GateId::DistributedClock,
            quality.distributed_clock_locked,
            cycle,
            0x4443_0001,
        );
        self.update_gate(GateId::Drive, quality.drive_ready, cycle, 0x4452_0001);
        self.update_gate(GateId::Domain, quality.domain_valid, cycle, 0x444F_0001);
        self.update_gate(GateId::Link, quality.wkc_valid, cycle, 0x574B_0001);
        self.update_gate(GateId::Command, quality.command_current, cycle, 0x434D_0001);
        self.update_gate(
            GateId::Supervisor,
            quality.supervisor_healthy,
            cycle,
            0x5355_0001,
        );
        self.update_gate(
            GateId::ExternalSafety,
            quality.external_safety_clear,
            cycle,
            0x5341_0001,
        );
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
            return self.reject_permit(permit, now_ns, PermitError::BootMismatch);
        }
        if permit.source_id != self.policy.authorized_source_id {
            return self.reject_permit(permit, now_ns, PermitError::SourceUnauthorized);
        }
        if permit.authority < self.policy.minimum_authority {
            return self.reject_permit(permit, now_ns, PermitError::AuthorityInsufficient);
        }
        if permit.policy_version != self.policy.permit_policy_version {
            return self.reject_permit(permit, now_ns, PermitError::PolicyVersionMismatch);
        }
        if permit.expires_at_ns <= now_ns {
            return self.reject_permit(permit, now_ns, PermitError::Expired);
        }
        if permit.axis_mask == 0 {
            return self.reject_permit(permit, now_ns, PermitError::EmptyAxisMask);
        }
        if permit.permit_epoch < self.permit_epoch {
            return self.reject_permit(permit, now_ns, PermitError::EpochReplayed);
        }
        if permit.permit_epoch == self.permit_epoch && permit.sequence <= self.last_permit_sequence
        {
            return self.reject_permit(permit, now_ns, PermitError::SequenceReplayed);
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
        self.latched_fault_code = code;
        self.transition(LifecycleState::FaultLatched, cycle);
    }

    pub fn clear_fault(&mut self, cycle: u64) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::FaultLatched {
            return Err(LifecycleError::InvalidState);
        }
        let fault_code = self.first_fault_code;
        self.first_fault_code = 0;
        self.latched_fault_code = 0;
        self.transition_with_fault(LifecycleState::Qualifying, cycle, fault_code);
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
        self.recovery_count = self.recovery_count.saturating_add(1);
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
        self.transition_with_fault(state, cycle, self.first_fault_code);
    }

    fn transition_with_fault(&mut self, state: LifecycleState, cycle: u64, fault_code: u32) {
        if self.state != state {
            let sequence = self.transition_sequence.saturating_add(1);
            let previous = self.state;
            self.state = state;
            self.state_since_cycle = cycle;
            self.transition_sequence = sequence;
            self.transitions[self.transition_head] = LifecycleTransition {
                sequence,
                cycle,
                from: previous,
                to: state,
                fault_code,
                reserved: 0,
            };
            self.transition_head = (self.transition_head + 1) % MAX_TRANSITIONS;
            self.transition_count = (self.transition_count + 1).min(MAX_TRANSITIONS);
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

    fn reject_permit(
        &mut self,
        permit: MotionPermit,
        now_ns: u64,
        error: PermitError,
    ) -> Result<(), PermitError> {
        self.permit_audit_sequence = self.permit_audit_sequence.saturating_add(1);
        self.permit_audits[self.permit_audit_head] = PermitAudit {
            sequence: self.permit_audit_sequence,
            timestamp_ns: now_ns,
            source_id: permit.source_id,
            permit_epoch: permit.permit_epoch,
            permit_sequence: permit.sequence,
            error,
            reserved: [0; 7],
        };
        self.permit_audit_head = (self.permit_audit_head + 1) % MAX_PERMIT_AUDITS;
        self.permit_audit_count = (self.permit_audit_count + 1).min(MAX_PERMIT_AUDITS);
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
            | GateId::Platform.bit()
            | GateId::Topology.bit()
            | GateId::DistributedClock.bit()
            | GateId::Drive.bit()
            | GateId::Domain.bit()
            | GateId::Link.bit()
            | GateId::Command.bit()
            | GateId::Supervisor.bit()
            | GateId::ExternalSafety.bit()
            | GateId::Budget.bit();
        let policy = GuardPolicy {
            enter_good_cycles: 1,
            exit_bad_cycles: 1,
            max_age_cycles: 1,
            stop_action: StopAction::QuickStop,
            authorized_source_id: 1,
            minimum_authority: 1,
            permit_policy_version: 1,
        };
        let mut guard = LifecycleGuard::new(required, 1, policy);
        guard.update_cyclic_quality(
            CyclicQuality {
                platform_ready: true,
                coe_ready: true,
                topology_valid: true,
                distributed_clock_locked: true,
                drive_ready: true,
                domain_valid: true,
                wkc_valid: true,
                command_current: true,
                supervisor_healthy: true,
                external_safety_clear: true,
                cycle_within_budget: true,
            },
            1,
        );
        guard
            .accept_permit(
                MotionPermit {
                    boot_id: 1,
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
            )
            .unwrap();
        assert_eq!(guard.cycle(1, 1), LifecycleAction::Hold);
        guard.update_cyclic_quality(
            CyclicQuality {
                platform_ready: true,
                coe_ready: true,
                topology_valid: true,
                distributed_clock_locked: true,
                drive_ready: true,
                domain_valid: true,
                wkc_valid: false,
                command_current: true,
                supervisor_healthy: true,
                external_safety_clear: true,
                cycle_within_budget: true,
            },
            2,
        );
        assert_eq!(guard.gate(GateId::Link).fault_code, 0x574B_0001);
    }

    #[test]
    fn cyclic_quality_projects_each_runtime_safety_gate() {
        let mut guard = LifecycleGuard::new(
            GateId::Platform.bit()
                | GateId::Topology.bit()
                | GateId::Command.bit()
                | GateId::Supervisor.bit()
                | GateId::ExternalSafety.bit(),
            1,
            GuardPolicy {
                enter_good_cycles: 1,
                exit_bad_cycles: 1,
                max_age_cycles: 1,
                stop_action: StopAction::QuickStop,
                authorized_source_id: 1,
                minimum_authority: 1,
                permit_policy_version: 1,
            },
        );
        guard.update_cyclic_quality(
            CyclicQuality {
                platform_ready: false,
                coe_ready: true,
                topology_valid: false,
                distributed_clock_locked: true,
                drive_ready: true,
                domain_valid: true,
                wkc_valid: true,
                command_current: false,
                supervisor_healthy: false,
                external_safety_clear: false,
                cycle_within_budget: true,
            },
            7,
        );
        assert_eq!(guard.gate(GateId::Platform).fault_code, 0x504C_0001);
        assert_eq!(guard.gate(GateId::Topology).fault_code, 0x544F_0001);
        assert_eq!(guard.gate(GateId::Command).fault_code, 0x434D_0001);
        assert_eq!(guard.gate(GateId::Supervisor).fault_code, 0x5355_0001);
        assert_eq!(guard.gate(GateId::ExternalSafety).fault_code, 0x5341_0001);
        assert_eq!(guard.ready_gate_mask(7) & guard.required_mask(), 0);
    }

    const POLICY: GuardPolicy = GuardPolicy {
        enter_good_cycles: 2,
        exit_bad_cycles: 2,
        max_age_cycles: 1,
        stop_action: StopAction::QuickStop,
        authorized_source_id: 1,
        minimum_authority: 1,
        permit_policy_version: 1,
    };

    fn permit(sequence: u64, expires_at_ns: u64) -> MotionPermit {
        MotionPermit {
            boot_id: 10,
            source_id: 1,
            permit_epoch: 1,
            sequence,
            axis_mask: 0x03,
            expires_at_ns,
            authority: 1,
            reserved: [0; 3],
            policy_version: 1,
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
    fn snapshot_exposes_current_gate_permit_and_transition_state() {
        let mut guard = LifecycleGuard::new(
            GateId::Link.bit() | GateId::Drive.bit(),
            10,
            GuardPolicy {
                enter_good_cycles: 1,
                exit_bad_cycles: 1,
                max_age_cycles: 1,
                stop_action: StopAction::QuickStop,
                authorized_source_id: 1,
                minimum_authority: 1,
                permit_policy_version: 1,
            },
        );
        guard.update_gate(GateId::Link, true, 4, 0);
        guard.update_gate(GateId::Drive, true, 4, 0);
        guard.accept_permit(permit(1, 100), 50).unwrap();
        assert_eq!(
            guard.request_rearm(permit(2, 100), 4, 50),
            Ok(LifecycleAction::EnableAllowed)
        );

        let snapshot = guard.snapshot(4, 50);
        assert_eq!(snapshot.state, LifecycleState::Active);
        assert_eq!(snapshot.state_since_cycle, 4);
        assert_eq!(
            snapshot.required_gate_mask,
            GateId::Link.bit() | GateId::Drive.bit()
        );
        assert_eq!(
            snapshot.valid_gate_mask,
            GateId::Link.bit() | GateId::Drive.bit()
        );
        assert_eq!(
            snapshot.qualified_gate_mask,
            GateId::Link.bit() | GateId::Drive.bit()
        );
        assert_eq!(
            snapshot.ready_gate_mask,
            GateId::Link.bit() | GateId::Drive.bit()
        );
        assert!(snapshot.motion_permit_current);
        assert_eq!(snapshot.permit_epoch, 1);
        assert_eq!(snapshot.permit_expires_at_ns, 100);
        assert_eq!(snapshot.transition_sequence, 1);
        assert_eq!(snapshot.transition_cycle, 4);
        assert_eq!(snapshot.recovery_count, 1);
        assert_eq!(guard.latched_fault_code(), 0);

        guard.latch_fault(0xDEAD, 5);
        let fault_snapshot = guard.snapshot(5, 50);
        assert_eq!(fault_snapshot.latched_fault_code, 0xDEAD);
        assert!(!fault_snapshot.motion_permit_current);
        assert_eq!(fault_snapshot.permit_epoch, 2);
        assert_eq!(fault_snapshot.transition_sequence, 2);
        assert_eq!(fault_snapshot.transition_cycle, 5);
    }

    #[test]
    fn stopped_motion_requires_explicit_rearm_and_new_permit() {
        let mut guard = LifecycleGuard::new(
            GateId::Link.bit(),
            10,
            GuardPolicy {
                enter_good_cycles: 1,
                exit_bad_cycles: 1,
                max_age_cycles: 1,
                stop_action: StopAction::QuickStop,
                authorized_source_id: 1,
                minimum_authority: 1,
                permit_policy_version: 1,
            },
        );
        guard.update_gate(GateId::Link, true, 1, 0);
        guard.accept_permit(permit(1, 100), 1).unwrap();
        assert_eq!(
            guard.request_rearm(permit(2, 100), 1, 1),
            Ok(LifecycleAction::EnableAllowed)
        );
        guard.update_gate(GateId::Link, false, 2, 0xCAFE);
        assert_eq!(
            guard.cycle(2, 2),
            LifecycleAction::Stop(StopAction::QuickStop)
        );
        guard.update_gate(GateId::Link, true, 3, 0);
        assert_eq!(
            guard.cycle(3, 3),
            LifecycleAction::Stop(StopAction::QuickStop)
        );
        assert_eq!(guard.acknowledge_stopped(3), Ok(()));
        assert_eq!(guard.cycle(4, 4), LifecycleAction::Hold);
        assert_eq!(
            guard.request_rearm(
                MotionPermit {
                    permit_epoch: 2,
                    ..permit(3, 100)
                },
                4,
                4,
            ),
            Ok(LifecycleAction::EnableAllowed)
        );
        assert_eq!(guard.cycle(4, 4), LifecycleAction::EnableAllowed);
    }

    #[test]
    fn transition_history_captures_order_and_fault_reason() {
        let mut guard = LifecycleGuard::new(
            GateId::Link.bit(),
            10,
            GuardPolicy {
                enter_good_cycles: 1,
                exit_bad_cycles: 1,
                max_age_cycles: 1,
                stop_action: StopAction::QuickStop,
                authorized_source_id: 1,
                minimum_authority: 1,
                permit_policy_version: 1,
            },
        );
        guard.update_gate(GateId::Link, true, 1, 0);
        guard.accept_permit(permit(1, 100), 1).unwrap();
        assert_eq!(guard.cycle(1, 1), LifecycleAction::Hold);
        assert_eq!(
            guard.request_rearm(permit(2, 100), 1, 1),
            Ok(LifecycleAction::EnableAllowed)
        );

        guard.update_gate(GateId::Link, false, 2, 0xCAFE);
        assert_eq!(
            guard.cycle(2, 2),
            LifecycleAction::Stop(StopAction::QuickStop)
        );

        assert_eq!(guard.transition_sequence(), 3);
        assert_eq!(guard.transition_count(), 3);
        assert_eq!(
            guard.transition_at(0),
            Some(LifecycleTransition {
                sequence: 1,
                cycle: 1,
                from: LifecycleState::Qualifying,
                to: LifecycleState::Ready,
                fault_code: 0,
                reserved: 0,
            })
        );
        assert_eq!(
            guard.transition_at(1).map(|transition| transition.to),
            Some(LifecycleState::Active)
        );
        assert_eq!(
            guard.transition_at(2),
            Some(LifecycleTransition {
                sequence: 3,
                cycle: 2,
                from: LifecycleState::Active,
                to: LifecycleState::Stopping,
                fault_code: 0xCAFE,
                reserved: 0,
            })
        );
        assert_eq!(guard.transition_at(3), None);
    }

    #[test]
    fn transition_history_overwrites_oldest_record_at_fixed_capacity() {
        let mut guard = LifecycleGuard::new(0, 10, GuardPolicy::conservative());
        for cycle in 0..(MAX_TRANSITIONS + 2) {
            guard.set_maintenance(true, (cycle * 2 + 1) as u64);
            guard.set_maintenance(false, (cycle * 2 + 2) as u64);
        }

        let total = ((MAX_TRANSITIONS + 2) * 2) as u64;
        assert_eq!(guard.transition_count(), MAX_TRANSITIONS);
        assert_eq!(guard.transition_sequence(), total);
        assert_eq!(
            guard.transition_at(0).unwrap().sequence,
            total - MAX_TRANSITIONS as u64 + 1
        );
        assert_eq!(
            guard.transition_at(MAX_TRANSITIONS - 1).unwrap().sequence,
            total
        );
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
    fn permit_policy_rejections_are_fixed_capacity_audits() {
        let policy = GuardPolicy {
            enter_good_cycles: 1,
            exit_bad_cycles: 1,
            max_age_cycles: 1,
            stop_action: StopAction::QuickStop,
            authorized_source_id: 42,
            minimum_authority: 2,
            permit_policy_version: 9,
        };
        let mut guard = LifecycleGuard::new(0, 10, policy);

        let mut unauthorized = permit(1, 100);
        unauthorized.source_id = 7;
        assert_eq!(
            guard.accept_permit(unauthorized, 11),
            Err(PermitError::SourceUnauthorized)
        );
        let mut insufficient = permit(2, 100);
        insufficient.source_id = 42;
        insufficient.authority = 1;
        assert_eq!(
            guard.accept_permit(insufficient, 12),
            Err(PermitError::AuthorityInsufficient)
        );
        let mut stale_policy = permit(3, 100);
        stale_policy.source_id = 42;
        stale_policy.authority = 2;
        stale_policy.policy_version = 8;
        assert_eq!(
            guard.accept_permit(stale_policy, 13),
            Err(PermitError::PolicyVersionMismatch)
        );

        assert_eq!(guard.permit_audit_count(), 3);
        assert_eq!(
            guard.permit_audit_at(0),
            Some(PermitAudit {
                sequence: 1,
                timestamp_ns: 11,
                source_id: 7,
                permit_epoch: 1,
                permit_sequence: 1,
                error: PermitError::SourceUnauthorized,
                reserved: [0; 7],
            })
        );
        assert_eq!(
            guard.permit_audit_at(2).map(|audit| audit.error),
            Some(PermitError::PolicyVersionMismatch)
        );

        for sequence in 4..=(MAX_PERMIT_AUDITS as u64 + 3) {
            let mut rejected = permit(sequence, 100);
            rejected.source_id = 42;
            rejected.authority = 2;
            rejected.policy_version = 8;
            assert_eq!(
                guard.accept_permit(rejected, sequence),
                Err(PermitError::PolicyVersionMismatch)
            );
        }
        assert_eq!(guard.permit_audit_count(), MAX_PERMIT_AUDITS);
        assert_eq!(guard.permit_audit_at(0).unwrap().sequence, 4);
        assert_eq!(
            guard
                .permit_audit_at(MAX_PERMIT_AUDITS - 1)
                .unwrap()
                .sequence,
            MAX_PERMIT_AUDITS as u64 + 3
        );
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
