//! Fixed-capacity single-Domain EtherCAT lifecycle cycle branches.
//!
//! The caller owns RX, Domain completion, and command admission. These
//! branches consume a finished Domain and a real receive
//! report, then publish TX and its evidence for that cycle.

use crate::cia402::step_axis_bank;
use crate::ethercat::{
    OtherCycleFacts, StopFrameError, submit_active_frame, submit_inhibited_frame,
    submit_stopping_frame, verified_ethercat_stop_feedback,
};
use crate::procbuf::{
    AxisEvidenceError, LifecycleEventCursor, LifecycleEventError, axis_stops_to_procbuf,
    ethercat_cycle_to_procbuf, lifecycle_events_to_procbuf, lifecycle_to_procbuf,
};
use crate::{
    CyclicQuality, LifecycleAction, LifecycleError, LifecycleGuard, MAX_MOTION_AXES, StopFeedback,
};
use esop_ethercat_core::{
    CycleReport, DcCyclicSync, Domain, EthercatMaster, EthercatPort, FramePlan,
};
use esop_procbuf::{HeaderError, ProcBuf, StatePage, StatePublishError};
use esop_profile_cia402::{
    Cia402AxisBank, Cia402PdoMap, Cia402Target, CyclicLimits, CyclicSetpointGuard, DriveRequest,
    OperatingMode,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopCycleError {
    CycleMismatch,
    AxisCapacityExceeded,
    Header(HeaderError),
    NotStopping(LifecycleAction),
    Evidence(AxisEvidenceError),
    Abort(LifecycleError),
}

/// Submission errors do not suppress the State page: stop `issued_action`
/// stays zero for a failed TX, while a verified response to an *earlier*
/// accepted TX may still acknowledge stopping. An active TX failure triggers
/// an immediate stop attempt and never advances the setpoint guard. A failed
/// State publication leaves the caller-owned page intact for a retry;
/// lifecycle events remain pending.
#[derive(Debug)]
pub struct StopCycleOutcome<E> {
    pub action: LifecycleAction,
    /// Original active TX failure if a stop was attempted in its place.
    pub active_failure: Option<StopFrameError<E>>,
    pub quality: CyclicQuality,
    pub feedback: Option<StopFeedback>,
    pub acknowledged: bool,
    pub transmission: Result<usize, StopFrameError<E>>,
    pub state_publish: Result<u64, StatePublishError>,
    pub event_publish: Option<Result<usize, LifecycleEventError>>,
}

/// All non-CiA 402 outputs in `safe_process_image` must be independently
/// checked by the caller. The Domain has already been finished for `report`;
/// start the next Domain receive only *after* this branch inspects its input.
/// `transition_time_ns` is the timestamp of a previously recorded transition;
/// transitions made during `run` use `now_ns` instead.
pub struct StopCycleContext<
    'a,
    P: EthercatPort,
    const AXES: usize,
    const IO: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
    const EVENTS: usize,
> {
    pub guard: &'a mut LifecycleGuard,
    pub bank: &'a mut Cia402AxisBank<AXES>,
    pub master: &'a mut EthercatMaster<SLOTS, MTU>,
    pub port: &'a mut P,
    pub domain: &'a Domain<BYTES, SEGMENTS>,
    pub dc: &'a DcCyclicSync,
    pub buffer: &'a ProcBuf<AXES, IO, 1, EVENTS>,
    pub event_cursor: &'a mut LifecycleEventCursor,
    pub state: &'a mut StatePage<AXES, IO, 1>,
    pub report: CycleReport,
    pub other: OtherCycleFacts,
    pub maps: &'a [Cia402PdoMap; AXES],
    pub modes: &'a [OperatingMode; AXES],
    pub max_stationary_velocities: &'a [u32; AXES],
    pub safe_process_image: &'a [u8; BYTES],
    pub plan: &'a FramePlan<DATAGRAMS>,
    pub next_generation: u16,
    pub deadline_ns: u64,
    pub now_ns: u64,
    pub transition_time_ns: u64,
}

struct MotionInputs<'a, const AXES: usize> {
    targets: &'a [Option<Cia402Target>; AXES],
    guards: &'a mut [CyclicSetpointGuard; AXES],
    limits: &'a [CyclicLimits; AXES],
}

impl<
    P: EthercatPort,
    const AXES: usize,
    const IO: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
    const EVENTS: usize,
> StopCycleContext<'_, P, AXES, IO, BYTES, SEGMENTS, DATAGRAMS, SLOTS, MTU, EVENTS>
{
    /// Project the completed RX cycle, submit a stop or inhibited frame, then
    /// publish causally matched State and transition events. `NotStopping`
    /// means this stop-only entry needs `run_with_motion` for an active cycle;
    /// no frame or State is published on that path.
    pub fn run(&mut self) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner(None)
    }

    /// Run all lifecycle branches. Guards are reset on a new Active
    /// transition, and the enable-operation edge seeds them from current
    /// verified actual feedback. Targets commit only after successful TX.
    pub fn run_with_motion(
        &mut self,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner(Some(MotionInputs {
            targets,
            guards,
            limits,
        }))
    }

    fn run_inner(
        &mut self,
        motion: Option<MotionInputs<'_, AXES>>,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        if self.report.cycle == 0
            || self.state.sequence != self.report.cycle
            || self.master.cycle_number() != self.report.cycle
        {
            return Err(StopCycleError::CycleMismatch);
        }
        if AXES > MAX_MOTION_AXES {
            return Err(StopCycleError::AxisCapacityExceeded);
        }
        let axes_mask = if AXES == MAX_MOTION_AXES {
            u32::MAX
        } else {
            (1u32 << AXES) - 1
        };
        if (self.guard.policy.allowed_axis_mask | self.guard.motion_axes_mask) & !axes_mask != 0 {
            return Err(StopCycleError::AxisCapacityExceeded);
        }
        self.buffer
            .validate_header(self.buffer.header().robot_id, self.guard.boot_id)
            .map_err(StopCycleError::Header)?;
        if self.state.boot_id != self.guard.boot_id {
            return Err(StopCycleError::Header(HeaderError::BootIdMismatch));
        }

        let previous_transition_sequence = self.guard.transition_sequence;
        let quality = ethercat_cycle_to_procbuf(
            self.state,
            self.report,
            &[self.domain.quality()],
            &[true],
            self.dc,
            self.other,
        );
        self.guard.update_cyclic_quality(quality, self.report.cycle);
        let statuswords = core::array::from_fn(|axis| {
            if quality.domain_valid && quality.wkc_valid {
                self.maps[axis]
                    .read_inputs_for(self.domain.input(), self.modes[axis])
                    .map_or(u16::MAX, |input| input.statusword)
            } else {
                u16::MAX
            }
        });
        let activation = (self.guard.boot_id, self.guard.transition_sequence);
        let (mut action, mut transmission, feedback, mut stop_mask) = {
            let mut decision = self.guard.cycle_axes(self.report.cycle, self.now_ns);
            let action = decision.action();
            if matches!(action, LifecycleAction::EnableAllowed) && motion.is_none() {
                return Err(StopCycleError::NotStopping(action));
            }
            let requests = if matches!(action, LifecycleAction::EnableAllowed) {
                [DriveRequest::Enable; AXES]
            } else {
                [DriveRequest::Disable; AXES]
            };
            let outputs = step_axis_bank(self.bank, &decision, statuswords, requests);
            let feedback = if matches!(action, LifecycleAction::Stop(_)) {
                verified_ethercat_stop_feedback(
                    &decision,
                    self.report,
                    self.domain,
                    self.maps,
                    self.modes,
                    self.max_stationary_velocities,
                )
            } else {
                None
            };
            let transmission = match (action, motion) {
                (LifecycleAction::Stop(_), _) => submit_stopping_frame(
                    &mut decision,
                    &outputs,
                    self.maps,
                    self.modes,
                    self.safe_process_image,
                    self.domain,
                    self.plan,
                    self.master,
                    self.port,
                    self.next_generation,
                    self.deadline_ns,
                ),
                (LifecycleAction::EnableAllowed, Some(motion)) => {
                    let mut next_guards = *motion.guards;
                    for guard in &mut next_guards {
                        guard.bind_activation(activation.0, activation.1);
                    }
                    let transmission = submit_active_frame(
                        &decision,
                        self.report,
                        &outputs,
                        motion.targets,
                        &mut next_guards,
                        motion.limits,
                        self.maps,
                        self.modes,
                        self.safe_process_image,
                        self.domain,
                        self.plan,
                        self.master,
                        self.port,
                        self.next_generation,
                        self.deadline_ns,
                    );
                    if transmission.is_ok() {
                        *motion.guards = next_guards;
                    }
                    transmission
                }
                (LifecycleAction::Hold | LifecycleAction::FaultLatched, _) => {
                    submit_inhibited_frame(
                        &decision,
                        &outputs,
                        self.maps,
                        self.modes,
                        self.safe_process_image,
                        self.domain,
                        self.plan,
                        self.master,
                        self.port,
                        self.next_generation,
                        self.deadline_ns,
                    )
                }
                (LifecycleAction::EnableAllowed, None) => {
                    return Err(StopCycleError::NotStopping(LifecycleAction::EnableAllowed));
                }
            };
            if !matches!(action, LifecycleAction::EnableAllowed) || transmission.is_ok() {
                axis_stops_to_procbuf(self.state, &decision, &outputs, feedback)
                    .map_err(StopCycleError::Evidence)?;
            }
            (
                action,
                transmission,
                feedback,
                decision.stopping_axis_mask(),
            )
        };
        let mut active_failure = None;
        if matches!(action, LifecycleAction::EnableAllowed) && transmission.is_err() {
            active_failure = transmission.err();
            self.guard
                .abort_active_cycle(self.report.cycle, 0x5458_0001)
                .map_err(StopCycleError::Abort)?;
            let mut stop_decision = self.guard.cycle_axes(self.report.cycle, self.now_ns);
            action = stop_decision.action();
            let stop_outputs = step_axis_bank(
                self.bank,
                &stop_decision,
                statuswords,
                [DriveRequest::Disable; AXES],
            );
            let stop_transmission = submit_stopping_frame(
                &mut stop_decision,
                &stop_outputs,
                self.maps,
                self.modes,
                self.safe_process_image,
                self.domain,
                self.plan,
                self.master,
                self.port,
                self.next_generation,
                self.deadline_ns,
            );
            axis_stops_to_procbuf(self.state, &stop_decision, &stop_outputs, None)
                .map_err(StopCycleError::Evidence)?;
            stop_mask = stop_decision.stopping_axis_mask();
            transmission = stop_transmission;
        }
        let acknowledged = feedback
            .filter(|proof| {
                proof.observed_axis_mask & stop_mask == stop_mask
                    && proof.stationary_axis_mask & stop_mask == stop_mask
                    && proof.non_enabled_axis_mask & stop_mask == stop_mask
            })
            .is_some_and(|proof| {
                self.guard
                    .acknowledge_stopped(self.report.cycle, proof)
                    .is_ok()
            });
        self.state.lifecycle = lifecycle_to_procbuf(
            self.guard.snapshot(self.report.cycle, self.now_ns),
            if self.guard.transition_sequence != previous_transition_sequence {
                self.now_ns
            } else {
                self.transition_time_ns
            },
        );
        let state_publish = self.buffer.publish_state(*self.state);
        let event_publish = state_publish.as_ref().ok().map(|_| {
            lifecycle_events_to_procbuf(self.guard, self.buffer, self.event_cursor, self.now_ns)
        });
        Ok(StopCycleOutcome {
            action,
            active_failure,
            quality,
            feedback,
            acknowledged,
            transmission,
            state_publish,
            event_publish,
        })
    }
}
