//! Fixed-capacity EtherCAT lifecycle cycle branches.
//!
//! The caller owns RX, Domain completion, and command admission. These
//! branches consume a finished Domain and a real receive
//! report, then publish TX and its evidence for that cycle.

use crate::cia402::step_axis_bank;
use crate::ethercat::{
    OtherCycleFacts, ScheduledDomainQuality, StopFrameError, submit_active_frame,
    submit_inhibited_frame, submit_stopping_frame, verified_ethercat_stop_feedback,
};
use crate::procbuf::{
    AxisEvidenceError, LifecycleEventCursor, LifecycleEventError, axis_stops_to_procbuf,
    cyclic_quality_to_procbuf, ethercat_cycle_to_procbuf, lifecycle_events_to_procbuf,
    lifecycle_to_procbuf, scheduled_ethercat_cycle_to_procbuf,
};
use crate::{
    CyclicQuality, GateId, LifecycleAction, LifecycleError, LifecycleGuard, MAX_MOTION_AXES,
    StopFeedback,
};
use esop_ethercat_core::{
    CycleReport, DcCyclicSync, Domain, EthercatMaster, EthercatPort, FramePlan, ScheduleTable,
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
    ScheduleRequired,
    InvalidSchedule,
    MotionDomainMismatch,
    InvalidCycleDeadline,
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
    /// `None` for caller-owned deadline facts; checked entries sample the port
    /// after TX and before publishing State/events.
    pub post_tx_deadline_met: Option<bool>,
    /// An active frame was accepted before a post-TX miss was observed. Its RX
    /// index may still be armed; `transmission` is that active submission, not
    /// proof that a stop frame was sent. The next cycle must send the stop.
    pub active_tx_before_deadline_miss: bool,
    pub quality: CyclicQuality,
    pub feedback: Option<StopFeedback>,
    pub acknowledged: bool,
    pub transmission: Result<usize, StopFrameError<E>>,
    pub state_publish: Result<u64, StatePublishError>,
    pub event_publish: Option<Result<usize, LifecycleEventError>>,
}

/// All non-CiA 402 outputs in `safe_process_image` must be independently
/// checked by the caller. The caller must supply actual quality for every
/// scheduled Domain, including failed or missing due receives. Start the next
/// receive only *after* this branch inspects its input.
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
    const DOMAINS: usize = 1,
> {
    pub guard: &'a mut LifecycleGuard,
    pub bank: &'a mut Cia402AxisBank<AXES>,
    pub master: &'a mut EthercatMaster<SLOTS, MTU>,
    pub port: &'a mut P,
    pub domain: &'a Domain<BYTES, SEGMENTS>,
    pub dc: &'a DcCyclicSync,
    pub buffer: &'a ProcBuf<AXES, IO, DOMAINS, EVENTS>,
    pub event_cursor: &'a mut LifecycleEventCursor,
    pub state: &'a mut StatePage<AXES, IO, DOMAINS>,
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

type ScheduledInputs<'a, const DOMAINS: usize, const SLOTS: usize> = (
    &'a ScheduleTable<DOMAINS, SLOTS>,
    &'a [ScheduledDomainQuality; DOMAINS],
    u8,
);

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
    const DOMAINS: usize,
> StopCycleContext<'_, P, AXES, IO, BYTES, SEGMENTS, DATAGRAMS, SLOTS, MTU, EVENTS, DOMAINS>
{
    /// Project the completed RX cycle, submit a stop or inhibited frame, then
    /// publish causally matched State and transition events. `NotStopping`
    /// means this stop-only entry needs `run_with_motion` for an active cycle;
    /// no frame or State is published on that path.
    pub fn run(&mut self) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner::<1>(None, None, None)
    }

    /// As `run`, with an absolute deadline on the same monotonic clock as the
    /// port. A miss before TX inhibits output; a miss during TX faults before
    /// State publication. This does not measure subsequent event publication.
    pub fn run_until(
        &mut self,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner::<1>(None, None, Some(cycle_deadline_ns))
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
        self.run_inner::<1>(
            None,
            Some(MotionInputs {
                targets,
                guards,
                limits,
            }),
            None,
        )
    }

    /// As `run_with_motion`, checking the actual port clock before and after
    /// the output submission. A late accepted active TX is reported explicitly
    /// and motion authority is revoked; it is not mislabeled as a stop TX.
    pub fn run_with_motion_until(
        &mut self,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner::<1>(
            None,
            Some(MotionInputs {
                targets,
                guards,
                limits,
            }),
            Some(cycle_deadline_ns),
        )
    }

    /// Use the frozen schedule to qualify every configured Domain. The caller
    /// supplies actual snapshots for non-motion Domains, including misses.
    /// The motion Domain must be due every cycle, and its reported quality
    /// must equal the actual Domain consumed for CiA 402 input and TX.
    pub fn run_scheduled<const SCHEDULE_SLOTS: usize>(
        &mut self,
        schedule: &ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        domains: &[ScheduledDomainQuality; DOMAINS],
        motion_domain_id: u8,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner(Some((schedule, domains, motion_domain_id)), None, None)
    }

    /// Combine frozen multi-Domain quality and a measured post-TX deadline.
    pub fn run_scheduled_until<const SCHEDULE_SLOTS: usize>(
        &mut self,
        schedule: &ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        domains: &[ScheduledDomainQuality; DOMAINS],
        motion_domain_id: u8,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner(
            Some((schedule, domains, motion_domain_id)),
            None,
            Some(cycle_deadline_ns),
        )
    }

    /// As `run_scheduled`, including the active CiA 402 motion branch.
    pub fn run_scheduled_with_motion<const SCHEDULE_SLOTS: usize>(
        &mut self,
        schedule: &ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        domains: &[ScheduledDomainQuality; DOMAINS],
        motion_domain_id: u8,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner(
            Some((schedule, domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
            }),
            None,
        )
    }

    /// As `run_scheduled_with_motion`, with a measured post-TX deadline.
    #[allow(clippy::too_many_arguments)]
    pub fn run_scheduled_with_motion_until<const SCHEDULE_SLOTS: usize>(
        &mut self,
        schedule: &ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        domains: &[ScheduledDomainQuality; DOMAINS],
        motion_domain_id: u8,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner(
            Some((schedule, domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
            }),
            Some(cycle_deadline_ns),
        )
    }

    fn run_inner<const SCHEDULE_SLOTS: usize>(
        &mut self,
        scheduled: Option<ScheduledInputs<'_, DOMAINS, SCHEDULE_SLOTS>>,
        motion: Option<MotionInputs<'_, AXES>>,
        cycle_deadline_ns: Option<u64>,
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

        if let Some((schedule, domains, motion_domain_id)) = scheduled {
            if schedule.domain_count() != DOMAINS
                || !schedule
                    .domains()
                    .iter()
                    .zip(domains)
                    .all(|(configured, observed)| configured.id == observed.id)
            {
                return Err(StopCycleError::InvalidSchedule);
            }
            let Some(position) = schedule.domains().iter().position(|configured| {
                configured.id == motion_domain_id
                    && configured.period_ticks == 1
                    && configured.phase_ticks == 0
            }) else {
                return Err(StopCycleError::MotionDomainMismatch);
            };
            if domains[position].quality != self.domain.quality() {
                return Err(StopCycleError::MotionDomainMismatch);
            }
        } else if DOMAINS != 1 {
            return Err(StopCycleError::ScheduleRequired);
        }
        if cycle_deadline_ns == Some(0) {
            return Err(StopCycleError::InvalidCycleDeadline);
        }

        let previous_transition_sequence = self.guard.transition_sequence;
        let decision_now_ns = cycle_deadline_ns
            .map(|_| self.port.now_ns().max(self.now_ns))
            .unwrap_or(self.now_ns);
        let mut other = self.other;
        if cycle_deadline_ns.is_some_and(|deadline| decision_now_ns >= deadline) {
            other.deadline_met = false;
        }
        let mut quality = if let Some((schedule, domains, _)) = scheduled {
            scheduled_ethercat_cycle_to_procbuf(
                self.state,
                self.report,
                schedule,
                domains,
                self.dc,
                other,
            )
        } else {
            ethercat_cycle_to_procbuf(
                self.state,
                self.report,
                &[self.domain.quality(); DOMAINS],
                &[true; DOMAINS],
                self.dc,
                other,
            )
        };
        self.guard.update_cyclic_quality(quality, self.report.cycle);
        if cycle_deadline_ns.is_some() && !quality.cycle_within_budget {
            self.guard.latch_fault(0x4255_0001, self.report.cycle);
        }
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
            let mut decision = self.guard.cycle_axes(self.report.cycle, decision_now_ns);
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
            let mut stop_decision = self.guard.cycle_axes(self.report.cycle, decision_now_ns);
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
        let mut active_tx_before_deadline_miss = false;
        let publish_now_ns = cycle_deadline_ns
            .map(|_| self.port.now_ns().max(decision_now_ns))
            .unwrap_or(self.now_ns);
        let post_tx_deadline_met = cycle_deadline_ns
            .map(|deadline| quality.cycle_within_budget && publish_now_ns < deadline);
        if post_tx_deadline_met == Some(false) {
            quality.cycle_within_budget = false;
            cyclic_quality_to_procbuf(self.state, quality);
            self.guard
                .update_gate(GateId::Budget, false, self.report.cycle, 0x4255_0001);
            self.guard.latch_fault(0x4255_0001, self.report.cycle);
            if matches!(action, LifecycleAction::EnableAllowed) && transmission.is_ok() {
                active_tx_before_deadline_miss = true;
                let stop_decision = self.guard.cycle_axes(self.report.cycle, publish_now_ns);
                action = stop_decision.action();
                let stop_outputs = step_axis_bank(
                    self.bank,
                    &stop_decision,
                    statuswords,
                    [DriveRequest::Disable; AXES],
                );
                axis_stops_to_procbuf(self.state, &stop_decision, &stop_outputs, None)
                    .map_err(StopCycleError::Evidence)?;
                stop_mask = stop_decision.stopping_axis_mask();
            } else if matches!(action, LifecycleAction::Hold) {
                action = self
                    .guard
                    .cycle_axes(self.report.cycle, publish_now_ns)
                    .action();
            }
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
            self.guard.snapshot(self.report.cycle, publish_now_ns),
            if self.guard.transition_sequence != previous_transition_sequence {
                publish_now_ns
            } else {
                self.transition_time_ns
            },
        );
        if cycle_deadline_ns.is_some() {
            self.state.monotonic_time_ns = publish_now_ns;
        }
        let state_publish = self.buffer.publish_state(*self.state);
        let event_publish = state_publish.as_ref().ok().map(|_| {
            lifecycle_events_to_procbuf(self.guard, self.buffer, self.event_cursor, publish_now_ns)
        });
        Ok(StopCycleOutcome {
            action,
            active_failure,
            post_tx_deadline_met,
            active_tx_before_deadline_miss,
            quality,
            feedback,
            acknowledged,
            transmission,
            state_publish,
            event_publish,
        })
    }
}
