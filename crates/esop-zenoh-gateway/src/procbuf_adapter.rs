//! Host-only, single-reader projection from the fixed ProcBuf ABI into v1 messages.

use esop_procbuf::{
    EventSeverity as ProcSeverity, HeaderError, ProcBuf, ProcBufEvent, QualityFact, StateSnapshot,
};
use esop_proto::v1::{
    AxisStopEvidence, DiagnosticEvent, EventSeverity, IoState, JointState, LifecycleState,
    LifecycleSummary, QualitySummary, RobotState, StopAction,
};
use esop_proto::{CURRENT_SCHEMA_VERSION, Message};

use crate::{KeySpace, MAX_PAYLOAD_BYTES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    Header(HeaderError),
    InvalidLifecycleState(u8),
    InvalidStopAction(u8),
    InvalidAxisStopEvidence(usize),
    InvalidRobotId,
    NonFiniteJoint(usize),
    InvalidQualityMask,
    ReplayedState,
    PayloadTooLarge,
}

/// This is the sole supervisor-side reader of one ProcBuf state page and event
/// ring. Callers may share the resulting owned messages with publishers and
/// query handlers, but must not create competing readers for the same region.
pub struct ProcBufProjector {
    key_space: KeySpace,
    numeric_robot_id: u64,
    boot_id: u64,
    last_state_sequence: u64,
}

impl ProcBufProjector {
    pub const fn new(key_space: KeySpace, numeric_robot_id: u64, boot_id: u64) -> Self {
        Self {
            key_space,
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
        buffer
            .validate_header(self.numeric_robot_id, self.boot_id)
            .map_err(ProjectionError::Header)?;
        let Some(snapshot) = buffer.read_state() else {
            return Ok(None);
        };
        if snapshot.state.sequence <= self.last_state_sequence {
            return Err(ProjectionError::ReplayedState);
        }
        let message = self.project_state(snapshot)?;
        if message.encoded_len() > MAX_PAYLOAD_BYTES {
            return Err(ProjectionError::PayloadTooLarge);
        }
        self.last_state_sequence = message.sequence;
        Ok(Some(message))
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
        buffer
            .validate_header(self.numeric_robot_id, self.boot_id)
            .map_err(ProjectionError::Header)?;
        Ok(buffer.pop_event().map(|event| self.project_event(event)))
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
    ) -> Result<RobotState, ProjectionError> {
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
        let quality = if state.quality.sequence == state.sequence {
            let facts = state.quality.cyclic;
            if !facts.well_formed() {
                return Err(ProjectionError::InvalidQualityMask);
            }
            facts.complete().then(|| QualitySummary {
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
                first_fault_code: state.lifecycle.first_blocking_code,
            })
        } else {
            None
        };
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
        Ok(RobotState {
            robot_id: std::str::from_utf8(self.key_space.robot())
                .map_err(|_| ProjectionError::InvalidRobotId)?
                .to_owned(),
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
        })
    }
}
