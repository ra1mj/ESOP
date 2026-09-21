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
    CycleReport, DcCyclicSync, Domain, EthercatMaster, EthercatPort, FramePlan, FramePlanSet,
    ScheduleTable, ScheduledDomainBank, ScheduledReceiveReport, wire::Command,
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
    InvalidAuxiliaryOutputs,
    ReceiveMismatch,
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
    /// Accepted auxiliary frames, even if a later auxiliary frame failed.
    pub auxiliary_frames_sent: usize,
    pub auxiliary_failure: Option<AuxiliaryOutputFailure<E>>,
    /// `None` for caller-owned deadline facts; checked entries sample the port
    /// after TX and before publishing State/events.
    pub post_tx_deadline_met: Option<bool>,
    /// Checked entries sample again after attempting State and event publication.
    /// An overrun here revokes motion authority and attempts a corrected State.
    /// It cannot retract a previously published optimistic State snapshot.
    pub post_publication_deadline_met: Option<bool>,
    /// Result of the corrective State publication, only attempted when a new
    /// deadline miss is first observed after State/event publication.
    pub deadline_correction_publish: Option<Result<u64, StatePublishError>>,
    /// Corrective events are attempted only after the corrected State succeeds.
    pub deadline_correction_events: Option<Result<usize, LifecycleEventError>>,
    /// An active frame was accepted before a deadline miss was observed. Its RX
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

/// A frozen safe image for one non-motion Domain. The caller must prove that
/// every writable output in this image is safe even while motion is stopped.
pub struct AuxiliaryOutputEntry<'a, const FRAMES: usize, const DATAGRAMS: usize> {
    pub id: u8,
    pub image: &'a [u8],
    pub plans: &'a FramePlanSet<FRAMES, DATAGRAMS>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuxiliaryOutputPlanError {
    InvalidSchedule,
    MissingDomain(u8),
    UnexpectedMotionOutput,
    InvalidPlan(u8),
    DuplicateIndex(u8),
    OverlappingWrite(u8, u8),
}

#[derive(Debug)]
pub struct AuxiliaryOutputFailure<E> {
    pub domain_id: u8,
    pub frame_index: usize,
    pub error: StopFrameError<E>,
}

struct AuxiliaryOutputReport<E> {
    sent_frames: usize,
    failure: Option<AuxiliaryOutputFailure<E>>,
}

/// Bind the frozen schedule, verified receive Domains, safe images, and split
/// TX plans once at activation. No allocation or plan search is needed per tick.
pub struct ScheduledAuxiliaryOutputs<
    'a,
    const DOMAINS: usize,
    const SCHEDULE_SLOTS: usize,
    const FRAMES: usize,
    const DATAGRAMS: usize,
> {
    schedule: &'a ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
    motion_domain_id: u8,
    motion_plan: FramePlan<DATAGRAMS>,
    entries: [Option<AuxiliaryOutputEntry<'a, FRAMES, DATAGRAMS>>; DOMAINS],
}

impl<
    'a,
    const DOMAINS: usize,
    const SCHEDULE_SLOTS: usize,
    const FRAMES: usize,
    const DATAGRAMS: usize,
> ScheduledAuxiliaryOutputs<'a, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>
{
    pub fn new(
        bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        schedule: &'a ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        motion_domain_id: u8,
        motion_plan: &FramePlan<DATAGRAMS>,
        entries: [Option<AuxiliaryOutputEntry<'a, FRAMES, DATAGRAMS>>; DOMAINS],
    ) -> Result<Self, AuxiliaryOutputPlanError> {
        if !bank.uses_schedule(schedule)
            || schedule.domain_count() != DOMAINS
            || !schedule
                .domains()
                .iter()
                .any(|domain| domain.id == motion_domain_id)
            || motion_plan.is_empty()
            || !bank.matches_frame_plan(motion_domain_id, motion_plan)
        {
            return Err(AuxiliaryOutputPlanError::InvalidSchedule);
        }
        let mut indices = [false; 256];
        for datagram in motion_plan.datagrams() {
            indices[datagram.index as usize] = true;
        }
        for (slot, configured) in schedule.domains().iter().enumerate() {
            let Some(entry) = entries[slot].as_ref() else {
                if configured.id == motion_domain_id {
                    continue;
                }
                return Err(AuxiliaryOutputPlanError::MissingDomain(configured.id));
            };
            if configured.id == motion_domain_id {
                return Err(AuxiliaryOutputPlanError::UnexpectedMotionOutput);
            }
            if entry.id != configured.id
                || entry.plans.is_empty()
                || !bank.matches_frame_plans(entry.id, entry.plans)
            {
                return Err(AuxiliaryOutputPlanError::InvalidPlan(configured.id));
            }
            for plan in entry.plans.plans() {
                for datagram in plan.datagrams() {
                    if !matches!(datagram.command, Command::Lrd | Command::Lwr | Command::Lrw)
                        || datagram.payload_len == 0
                        || datagram.expected_wkc == 0
                        || datagram
                            .payload_offset
                            .checked_add(datagram.payload_len)
                            .is_none_or(|end| end > entry.image.len())
                    {
                        return Err(AuxiliaryOutputPlanError::InvalidPlan(entry.id));
                    }
                    if indices[datagram.index as usize] {
                        return Err(AuxiliaryOutputPlanError::DuplicateIndex(datagram.index));
                    }
                    indices[datagram.index as usize] = true;
                    if !matches!(datagram.command, Command::Lwr | Command::Lrw) {
                        continue;
                    }
                    for motion in motion_plan.datagrams() {
                        if writable_overlap(datagram, motion) {
                            return Err(AuxiliaryOutputPlanError::OverlappingWrite(
                                entry.id,
                                motion_domain_id,
                            ));
                        }
                    }
                    for earlier in entry.plans.plans().iter().flat_map(|plan| plan.datagrams()) {
                        if earlier.index != datagram.index && writable_overlap(datagram, earlier) {
                            return Err(AuxiliaryOutputPlanError::OverlappingWrite(
                                entry.id, entry.id,
                            ));
                        }
                    }
                    for prior in entries[..slot].iter().flatten() {
                        for earlier in prior.plans.plans().iter().flat_map(|plan| plan.datagrams())
                        {
                            if writable_overlap(datagram, earlier) {
                                return Err(AuxiliaryOutputPlanError::OverlappingWrite(
                                    entry.id, prior.id,
                                ));
                            }
                        }
                    }
                }
            }
        }
        Ok(Self {
            schedule,
            motion_domain_id,
            motion_plan: *motion_plan,
            entries,
        })
    }

    fn submit_due<P: EthercatPort, const SLOTS: usize, const MTU: usize>(
        &self,
        cycle: u64,
        master: &mut EthercatMaster<SLOTS, MTU>,
        port: &mut P,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: Option<u64>,
    ) -> AuxiliaryOutputReport<P::Error> {
        let due = self
            .schedule
            .due_mask(((cycle - 1) % u64::from(self.schedule.hyperperiod_ticks())) as u32);
        let mut sent_frames = 0;
        for entry in self.entries.iter().flatten() {
            if due & (1u64 << entry.id) == 0 {
                continue;
            }
            for (frame_index, plan) in entry.plans.plans().iter().enumerate() {
                let now_ns = port.now_ns();
                let result = if now_ns >= rx_deadline_ns
                    || cycle_deadline_ns.is_some_and(|deadline| now_ns >= deadline)
                {
                    Err(StopFrameError::InvalidDeadline)
                } else {
                    master.reap_expired_rx_before_tx(now_ns);
                    match master.acquire_frame(generation, rx_deadline_ns) {
                        Err(error) => Err(StopFrameError::FramePool(error)),
                        Ok(handle) => {
                            match master.build_and_arm_frame_from_plan(handle, plan, entry.image) {
                                Err(error) => {
                                    let _ = master.release_unarmed_frame(handle);
                                    Err(StopFrameError::Build(error))
                                }
                                Ok(_) => master
                                    .submit_frame(port, handle)
                                    .map_err(StopFrameError::Transmit),
                            }
                        }
                    }
                };
                if let Err(error) = result {
                    return AuxiliaryOutputReport {
                        sent_frames,
                        failure: Some(AuxiliaryOutputFailure {
                            domain_id: entry.id,
                            frame_index,
                            error,
                        }),
                    };
                }
                sent_frames += 1;
            }
        }
        AuxiliaryOutputReport {
            sent_frames,
            failure: None,
        }
    }
}

fn writable_overlap(
    left: &esop_ethercat_core::DatagramPlan,
    right: &esop_ethercat_core::DatagramPlan,
) -> bool {
    matches!(right.command, Command::Lwr | Command::Lrw)
        && left.address < right.address.saturating_add(right.payload_len as u32)
        && right.address < left.address.saturating_add(left.payload_len as u32)
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
    /// State publication. A miss after publication attempts a correction, but
    /// cannot revoke a snapshot already observed by a concurrent reader.
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

    /// Submit each due safe auxiliary Domain before considering active motion
    /// output. An auxiliary failure stops remaining auxiliary TX, revokes
    /// motion authority, and still attempts the motion Domain's stop frame.
    #[allow(clippy::too_many_arguments)]
    pub fn run_scheduled_with_outputs_until<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &mut self,
        schedule: &ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        domains: &[ScheduledDomainQuality; DOMAINS],
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner_with_outputs(
            Some((schedule, domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
            }),
            Some(cycle_deadline_ns),
            Some(outputs),
        )
    }

    /// Bind the output decision to the same finalized shared RX report that
    /// supplied the motion Domain. The bank supplies the frozen Domain order;
    /// callers cannot substitute a stale or rearranged quality array. This
    /// still leaves DC/control TX and the final post-publication deadline with
    /// the outer cycle owner.
    #[allow(clippy::too_many_arguments)]
    pub fn run_received_with_outputs_until<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        received: &ScheduledReceiveReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        if !domain_bank.uses_schedule(outputs.schedule)
            || !domain_bank.confirms_receive(received)
            || self.report != received.report
            || !domain_bank
                .domain::<BYTES, SEGMENTS>(motion_domain_id)
                .is_some_and(|domain| core::ptr::eq(domain, self.domain))
            || match received.dc_result {
                Ok(()) => {
                    self.dc.last_sync_cycle() != received.report.cycle
                        || self.dc.last_error().is_some()
                }
                Err(error) => self.dc.last_error() != Some(error),
            }
        {
            return Err(StopCycleError::ReceiveMismatch);
        }
        let domains = core::array::from_fn(|slot| ScheduledDomainQuality {
            id: outputs.schedule.domains()[slot].id,
            quality: received.qualities[slot],
        });
        self.run_scheduled_with_outputs_until(
            outputs.schedule,
            &domains,
            motion_domain_id,
            outputs,
            targets,
            guards,
            limits,
            cycle_deadline_ns,
        )
    }

    fn run_inner<const SCHEDULE_SLOTS: usize>(
        &mut self,
        scheduled: Option<ScheduledInputs<'_, DOMAINS, SCHEDULE_SLOTS>>,
        motion: Option<MotionInputs<'_, AXES>>,
        cycle_deadline_ns: Option<u64>,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner_with_outputs::<SCHEDULE_SLOTS, 1>(scheduled, motion, cycle_deadline_ns, None)
    }

    fn run_inner_with_outputs<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &mut self,
        scheduled: Option<ScheduledInputs<'_, DOMAINS, SCHEDULE_SLOTS>>,
        motion: Option<MotionInputs<'_, AXES>>,
        cycle_deadline_ns: Option<u64>,
        outputs: Option<&ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>>,
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
        if let Some(outputs) = outputs {
            let Some((schedule, _, motion_domain_id)) = scheduled else {
                return Err(StopCycleError::InvalidAuxiliaryOutputs);
            };
            if !core::ptr::eq(outputs.schedule, schedule)
                || outputs.motion_domain_id != motion_domain_id
                || outputs.motion_plan != *self.plan
            {
                return Err(StopCycleError::InvalidAuxiliaryOutputs);
            }
        }
        if cycle_deadline_ns == Some(0) {
            return Err(StopCycleError::InvalidCycleDeadline);
        }

        let previous_transition_sequence = self.guard.transition_sequence;
        let mut decision_now_ns = cycle_deadline_ns
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
        let mut auxiliary_frames_sent = 0;
        let mut auxiliary_failure = None;
        if let Some(outputs) = outputs {
            // A failed pre-TX deadline must not emit auxiliary outputs either.
            if quality.cycle_within_budget {
                let sent = outputs.submit_due(
                    self.report.cycle,
                    self.master,
                    self.port,
                    self.next_generation,
                    self.deadline_ns,
                    cycle_deadline_ns,
                );
                auxiliary_frames_sent = sent.sent_frames;
                auxiliary_failure = sent.failure;
            }
            decision_now_ns = self.port.now_ns().max(decision_now_ns);
            if cycle_deadline_ns.is_some_and(|deadline| decision_now_ns >= deadline) {
                quality.cycle_within_budget = false;
                cyclic_quality_to_procbuf(self.state, quality);
            }
        }
        self.guard.update_cyclic_quality(quality, self.report.cycle);
        if cycle_deadline_ns.is_some() && !quality.cycle_within_budget {
            self.guard.latch_fault(0x4255_0001, self.report.cycle);
        }
        if auxiliary_failure.is_some() {
            self.guard.latch_fault(0x5458_0002, self.report.cycle);
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
        let mut deadline_correction_publish = None;
        let mut deadline_correction_events = None;
        let post_publication_deadline_met = if let Some(deadline) = cycle_deadline_ns {
            let final_now_ns = self.port.now_ns().max(publish_now_ns);
            if final_now_ns >= deadline && quality.cycle_within_budget {
                quality.cycle_within_budget = false;
                cyclic_quality_to_procbuf(self.state, quality);
                self.guard
                    .update_gate(GateId::Budget, false, self.report.cycle, 0x4255_0001);
                self.guard.latch_fault(0x4255_0001, self.report.cycle);
                if matches!(action, LifecycleAction::EnableAllowed) && transmission.is_ok() {
                    active_tx_before_deadline_miss = true;
                    let stop_decision = self.guard.cycle_axes(self.report.cycle, final_now_ns);
                    action = stop_decision.action();
                    let stop_outputs = step_axis_bank(
                        self.bank,
                        &stop_decision,
                        statuswords,
                        [DriveRequest::Disable; AXES],
                    );
                    // No stop frame was sent: record the request, not issuance.
                    axis_stops_to_procbuf(self.state, &stop_decision, &stop_outputs, None)
                        .map_err(StopCycleError::Evidence)?;
                } else if matches!(action, LifecycleAction::Hold) {
                    action = self
                        .guard
                        .cycle_axes(self.report.cycle, final_now_ns)
                        .action();
                }
                self.state.lifecycle = lifecycle_to_procbuf(
                    self.guard.snapshot(self.report.cycle, final_now_ns),
                    if self.guard.transition_sequence != self.state.lifecycle.transition_sequence {
                        final_now_ns
                    } else {
                        self.state.lifecycle.transition_time_ns
                    },
                );
                self.state.monotonic_time_ns = final_now_ns;
                let corrected = self.buffer.publish_state(*self.state);
                deadline_correction_events = corrected.as_ref().ok().map(|_| {
                    lifecycle_events_to_procbuf(
                        self.guard,
                        self.buffer,
                        self.event_cursor,
                        final_now_ns,
                    )
                });
                deadline_correction_publish = Some(corrected);
            }
            Some(quality.cycle_within_budget && final_now_ns < deadline)
        } else {
            None
        };
        Ok(StopCycleOutcome {
            action,
            active_failure,
            auxiliary_frames_sent,
            auxiliary_failure,
            post_tx_deadline_met,
            post_publication_deadline_met,
            deadline_correction_publish,
            deadline_correction_events,
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
