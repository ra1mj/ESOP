//! Fixed-capacity EtherCAT lifecycle cycle branches.
//!
//! The caller owns RX, Domain completion, and command admission. These
//! branches consume a finished Domain and a real receive
//! report, then publish TX and its evidence for that cycle.

use crate::cia402::{
    ControlledStopLimits, ControlledStopPhase, ControlledStopPlanner, step_axis_bank,
};
use crate::ethercat::{
    ControlledStopFrameReport, OtherCycleFacts, ScheduledControlGate, ScheduledDomainQuality,
    StopFrameError, other_cycle_facts_from_control_cycle, other_cycle_facts_from_mailbox_cycle,
    other_cycle_facts_from_process_tx, other_cycle_facts_from_production_service_cycle,
    submit_active_frame, submit_controlled_stopping_frame, submit_inhibited_frame,
    submit_prepared_active_frame, submit_stopping_frame, verified_ethercat_stop_feedback,
};
use crate::procbuf::{
    AxisEvidenceError, Cia402AxisCommandPolicy, Cia402FeedbackError, LifecycleEventCursor,
    LifecycleEventError, ProcBufCia402CommandError, axis_stops_to_procbuf,
    cia402_feedback_to_procbuf, controlled_axis_stops_to_procbuf, cyclic_quality_to_procbuf,
    ethercat_cycle_to_procbuf, lifecycle_events_to_procbuf, lifecycle_to_procbuf,
    prepare_cia402_command, scheduled_ethercat_cycle_to_procbuf,
};
use crate::{
    CyclicQuality, GateId, LifecycleAction, LifecycleError, LifecycleGuard, MAX_MOTION_AXES,
    MotionPermit, StopFeedback,
};
use esop_ethercat_core::{
    CycleReport, DcCyclicSync, Domain, EthercatMaster, EthercatPort, FramePlan, FramePlanSet,
    ScheduleTable, ScheduledControlCycleReport, ScheduledDomainBank, ScheduledMailboxCycleReport,
    ScheduledProcessInputs, ScheduledProcessTxReport, ScheduledProductionServiceCycleReport,
    ScheduledReceiveReport, ScheduledServiceTxFailure, wire::Command,
};
use esop_procbuf::{CommandPage, HeaderError, ProcBuf, StatePage, StatePublishError};
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
    ProcessCycleMismatch,
    ServiceCycleMismatch,
    Header(HeaderError),
    NotStopping(LifecycleAction),
    Evidence(AxisEvidenceError),
    Feedback(Cia402FeedbackError),
    Command(ProcBufCia402CommandError),
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
    /// The product-controlled path accepted the motion Domain frame. This is
    /// transport evidence only; drive execution still requires later input.
    pub controlled_stop_used: bool,
    /// The controlled runtime is latched to the default fail-closed stop for
    /// this MLG transition sequence.
    pub controlled_stop_fallback: bool,
    /// First controlled-path error that caused this cycle to latch fallback.
    /// The final `transmission` result describes the fallback frame.
    pub controlled_stop_failure: Option<StopFrameError<E>>,
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
    /// Exact process frames now owning the next shared RX generation. Present
    /// only for the scheduled-output entries that can participate in the
    /// stable production-cycle handoff.
    pub process_handoff: Option<ScheduledProcessHandoff>,
    pub quality: CyclicQuality,
    pub feedback: Option<StopFeedback>,
    pub acknowledged: bool,
    pub transmission: Result<usize, StopFrameError<E>>,
    pub state_publish: Result<u64, StatePublishError>,
    pub event_publish: Option<Result<usize, LifecycleEventError>>,
}

/// Immutable evidence that accepted process outputs belong to one future
/// shared RX cycle. A partial handoff must still be drained by that RX cycle;
/// it is not permission to resubmit already armed indices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledProcessHandoff {
    cycle: u64,
    generation: u16,
    rx_deadline_ns: u64,
    due_mask: u64,
    expected_frames: usize,
    sent_frames: usize,
}

impl ScheduledProcessHandoff {
    pub const fn cycle(self) -> u64 {
        self.cycle
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }

    pub const fn rx_deadline_ns(self) -> u64 {
        self.rx_deadline_ns
    }

    pub const fn due_mask(self) -> u64 {
        self.due_mask
    }

    pub const fn expected_frames(self) -> usize {
        self.expected_frames
    }

    pub const fn sent_frames(self) -> usize {
        self.sent_frames
    }

    pub const fn complete(self) -> bool {
        self.sent_frames == self.expected_frames
    }

    fn from_process_tx<E>(report: &ScheduledProcessTxReport<E>) -> Self {
        Self {
            cycle: report.cycle,
            generation: report.generation,
            rx_deadline_ns: report.rx_deadline_ns,
            due_mask: report.due_mask,
            expected_frames: report.expected_frames,
            sent_frames: report.sent_frames,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionPhase {
    PrimingRequired,
    ReceiveArmed,
    OutputPending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduledProductionCycleError {
    InvalidPhase,
    InvalidBinding,
    ProcessMismatch,
    ReceiveMismatch,
    OutputMismatch,
}

/// Final settlement of one production task. A cycle can safely advance to
/// the next RX even when motion was stopped, but task release additionally
/// requires a complete process handoff, final deadline, State, and event
/// publication evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledProductionRelease {
    pub cycle: u64,
    pub next: ScheduledProcessHandoff,
    pub process_complete: bool,
    pub final_deadline_met: bool,
    pub state_published: bool,
    pub events_published: bool,
    pub controlled_stop_used: bool,
    pub controlled_stop_fallback: bool,
}

impl ScheduledProductionRelease {
    pub const fn task_released(self) -> bool {
        self.process_complete
            && self.final_deadline_met
            && self.state_published
            && self.events_published
    }
}

/// Caller-owned, allocation-free state for one product-configured controlled
/// stop path. Limits are frozen at construction. A controlled validation,
/// build, or TX failure latches the current MLG transition sequence to the
/// default Disable/QuickStop path, so later cycles cannot re-enable a Hold or
/// RampToZero target after a fallback frame was already accepted.
pub struct ControlledStopCycleState<const AXES: usize> {
    planners: [ControlledStopPlanner; AXES],
    limits: [ControlledStopLimits; AXES],
    sequence: Option<u64>,
    fallback_latched: bool,
}

impl<const AXES: usize> ControlledStopCycleState<AXES> {
    pub const fn new(limits: [ControlledStopLimits; AXES]) -> Self {
        Self {
            planners: [ControlledStopPlanner::new(); AXES],
            limits,
            sequence: None,
            fallback_latched: false,
        }
    }

    pub const fn sequence(&self) -> Option<u64> {
        self.sequence
    }

    pub const fn fallback_latched(&self) -> bool {
        self.fallback_latched
    }

    pub fn phase(&self, axis: usize) -> Option<ControlledStopPhase> {
        self.planners.get(axis).map(ControlledStopPlanner::phase)
    }

    fn prepare(&mut self, sequence: u64) {
        if self.sequence != Some(sequence) {
            self.planners = [ControlledStopPlanner::new(); AXES];
            self.sequence = Some(sequence);
            self.fallback_latched = false;
        }
    }

    fn latch_fallback(&mut self, sequence: u64) {
        self.prepare(sequence);
        self.planners = [ControlledStopPlanner::new(); AXES];
        self.fallback_latched = true;
    }

    fn parts(
        &mut self,
    ) -> (
        &mut [ControlledStopPlanner; AXES],
        &[ControlledStopLimits; AXES],
    ) {
        (&mut self.planners, &self.limits)
    }
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
    due_mask: u64,
    expected_frames: usize,
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
        receive_cycle: u64,
        master: &mut EthercatMaster<SLOTS, MTU>,
        port: &mut P,
        generation: u16,
        rx_deadline_ns: u64,
        cycle_deadline_ns: Option<u64>,
    ) -> AuxiliaryOutputReport<P::Error> {
        let (due_mask, expected_frames) = self.due_summary(receive_cycle);
        let mut sent_frames = 0;
        for entry in self.entries.iter().flatten() {
            if due_mask & (1u64 << entry.id) == 0 {
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
                        due_mask,
                        expected_frames,
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
            due_mask,
            expected_frames,
            sent_frames,
            failure: None,
        }
    }

    fn due_summary(&self, receive_cycle: u64) -> (u64, usize) {
        let tick = (receive_cycle - 1) % u64::from(self.schedule.hyperperiod_ticks());
        let due_mask = self.schedule.due_mask(tick as u32);
        let expected_frames = self
            .entries
            .iter()
            .flatten()
            .filter(|entry| due_mask & (1u64 << entry.id) != 0)
            .map(|entry| entry.plans.frame_count())
            .sum();
        (due_mask, expected_frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: [ControlledStopLimits; 1] = [ControlledStopLimits {
        max_velocity_step: 10,
        max_torque_step: 5,
        max_stationary_velocity: 1,
        max_zero_torque: 1,
    }];

    #[test]
    fn controlled_stop_fallback_is_latched_only_for_its_transition_sequence() {
        let mut controlled = ControlledStopCycleState::new(LIMITS);

        controlled.latch_fallback(7);
        controlled.prepare(7);
        assert_eq!(controlled.sequence(), Some(7));
        assert!(controlled.fallback_latched());

        controlled.prepare(8);
        assert_eq!(controlled.sequence(), Some(8));
        assert!(!controlled.fallback_latched());
        assert_eq!(controlled.phase(0), Some(ControlledStopPhase::Idle));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduledProductionState {
    PrimingRequired,
    ReceiveArmed(ScheduledProcessHandoff),
    OutputPending { cycle: u64, generation: u16 },
}

/// Fixed-capacity owner for the stable process-cycle handoff. Initial
/// priming records the only explicit pre-RX submission. Every later receive
/// consumes the prior lifecycle output and every lifecycle settlement records
/// the next in-flight generation, preventing duplicate index submission.
pub struct ScheduledProductionCycleOwner<'a, const DOMAINS: usize, const SCHEDULE_SLOTS: usize> {
    schedule: &'a ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
    state: ScheduledProductionState,
}

impl<'a, const DOMAINS: usize, const SCHEDULE_SLOTS: usize>
    ScheduledProductionCycleOwner<'a, DOMAINS, SCHEDULE_SLOTS>
{
    pub fn new<const FRAMES: usize, const DATAGRAMS: usize>(
        outputs: &ScheduledAuxiliaryOutputs<'a, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
    ) -> Self {
        Self {
            schedule: outputs.schedule,
            state: ScheduledProductionState::PrimingRequired,
        }
    }

    pub const fn phase(&self) -> ScheduledProductionPhase {
        match self.state {
            ScheduledProductionState::PrimingRequired => ScheduledProductionPhase::PrimingRequired,
            ScheduledProductionState::ReceiveArmed(_) => ScheduledProductionPhase::ReceiveArmed,
            ScheduledProductionState::OutputPending { .. } => {
                ScheduledProductionPhase::OutputPending
            }
        }
    }

    pub const fn in_flight(&self) -> Option<ScheduledProcessHandoff> {
        match self.state {
            ScheduledProductionState::ReceiveArmed(handoff) => Some(handoff),
            _ => None,
        }
    }

    pub fn arm_priming<E, const PROCESS_FRAMES: usize, const PROCESS_DATAGRAMS: usize>(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        inputs: &ScheduledProcessInputs<
            '_,
            DOMAINS,
            SCHEDULE_SLOTS,
            PROCESS_FRAMES,
            PROCESS_DATAGRAMS,
        >,
        report: &ScheduledProcessTxReport<E>,
        expected_generation: u16,
        expected_rx_deadline_ns: u64,
    ) -> Result<ScheduledProcessHandoff, ScheduledProductionCycleError> {
        if !matches!(self.state, ScheduledProductionState::PrimingRequired) {
            return Err(ScheduledProductionCycleError::InvalidPhase);
        }
        if !domain_bank.uses_schedule(self.schedule) {
            return Err(ScheduledProductionCycleError::InvalidBinding);
        }
        if report.generation != expected_generation
            || report.rx_deadline_ns != expected_rx_deadline_ns
            || !domain_bank.accepts_process_tx(inputs, report)
        {
            return Err(ScheduledProductionCycleError::ProcessMismatch);
        }
        let handoff = ScheduledProcessHandoff::from_process_tx(report);
        self.state = ScheduledProductionState::ReceiveArmed(handoff);
        Ok(handoff)
    }

    pub fn complete_receive<E>(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        received: &ScheduledReceiveReport<E, DOMAINS>,
    ) -> Result<(), ScheduledProductionCycleError> {
        let ScheduledProductionState::ReceiveArmed(handoff) = self.state else {
            return Err(ScheduledProductionCycleError::InvalidPhase);
        };
        if !domain_bank.uses_schedule(self.schedule) {
            return Err(ScheduledProductionCycleError::InvalidBinding);
        }
        if handoff.cycle != received.report.cycle
            || handoff.generation != received.generation
            || !domain_bank.confirms_receive(received)
        {
            return Err(ScheduledProductionCycleError::ReceiveMismatch);
        }
        self.state = ScheduledProductionState::OutputPending {
            cycle: handoff.cycle,
            generation: handoff.generation,
        };
        Ok(())
    }

    pub fn complete_mailbox_cycle<E>(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledMailboxCycleReport<E, DOMAINS>,
    ) -> Result<(), ScheduledProductionCycleError> {
        self.complete_receive(domain_bank, &cycle.receive.received)
    }

    pub fn complete_control_cycle<E>(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledControlCycleReport<E, DOMAINS>,
    ) -> Result<(), ScheduledProductionCycleError> {
        if !domain_bank.confirms_control_cycle(cycle) {
            return Err(ScheduledProductionCycleError::ReceiveMismatch);
        }
        self.complete_receive(domain_bank, cycle.received())
    }

    pub fn complete_service_cycle<E>(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledProductionServiceCycleReport<E, DOMAINS>,
    ) -> Result<(), ScheduledProductionCycleError> {
        if !domain_bank.confirms_production_service_cycle(cycle) {
            return Err(ScheduledProductionCycleError::ReceiveMismatch);
        }
        self.complete_receive(domain_bank, cycle.received())
    }

    pub fn settle_output<E, const FRAMES: usize, const DATAGRAMS: usize>(
        &mut self,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        outcome: &StopCycleOutcome<E>,
        expected_next_rx_deadline_ns: u64,
    ) -> Result<ScheduledProductionRelease, ScheduledProductionCycleError> {
        let ScheduledProductionState::OutputPending { cycle, generation } = self.state else {
            return Err(ScheduledProductionCycleError::InvalidPhase);
        };
        if !core::ptr::eq(self.schedule, outputs.schedule) {
            return Err(ScheduledProductionCycleError::InvalidBinding);
        }
        let Some(handoff) = outcome.process_handoff else {
            return Err(ScheduledProductionCycleError::OutputMismatch);
        };
        let Some(expected_cycle) = cycle.checked_add(1) else {
            return Err(ScheduledProductionCycleError::OutputMismatch);
        };
        let (due_mask, expected_auxiliary_frames) = outputs.due_summary(handoff.cycle);
        if handoff.cycle != expected_cycle
            || handoff.generation != generation.wrapping_add(1)
            || expected_next_rx_deadline_ns == 0
            || handoff.rx_deadline_ns != expected_next_rx_deadline_ns
            || handoff.due_mask != due_mask
            || handoff.expected_frames != expected_auxiliary_frames.saturating_add(1)
            || handoff.sent_frames > handoff.expected_frames
        {
            return Err(ScheduledProductionCycleError::OutputMismatch);
        }

        let release = ScheduledProductionRelease {
            cycle,
            next: handoff,
            process_complete: handoff.complete(),
            final_deadline_met: outcome.post_tx_deadline_met == Some(true)
                && outcome.post_publication_deadline_met == Some(true),
            state_published: outcome.state_publish.is_ok()
                && outcome.deadline_correction_publish.is_none(),
            events_published: matches!(outcome.event_publish, Some(Ok(_)))
                && outcome.deadline_correction_events.is_none(),
            controlled_stop_used: outcome.controlled_stop_used,
            controlled_stop_fallback: outcome.controlled_stop_fallback,
        };
        self.state = ScheduledProductionState::ReceiveArmed(handoff);
        Ok(release)
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
    pub axis_policies: &'a [Cia402AxisCommandPolicy; AXES],
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
    permit: Option<MotionPermit>,
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
        self.run_inner::<1>(None, None, None, None)
    }

    /// As `run`, with an absolute deadline on the same monotonic clock as the
    /// port. A miss before TX inhibits output; a miss during TX faults before
    /// State publication. A miss after publication attempts a correction, but
    /// cannot revoke a snapshot already observed by a concurrent reader.
    pub fn run_until(
        &mut self,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner::<1>(None, None, None, Some(cycle_deadline_ns))
    }

    /// Stop-only entry with product-configured Hold/RampToZero. A controlled
    /// failure latches this MLG transition sequence to the default stop path.
    pub fn run_with_controlled_stop(
        &mut self,
        controlled: &mut ControlledStopCycleState<AXES>,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner::<1>(None, None, Some(controlled), None)
    }

    /// Deadline-checked variant of `run_with_controlled_stop`.
    pub fn run_with_controlled_stop_until(
        &mut self,
        controlled: &mut ControlledStopCycleState<AXES>,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        self.run_inner::<1>(None, None, Some(controlled), Some(cycle_deadline_ns))
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
                permit: None,
            }),
            None,
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
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
        )
    }

    /// Prepare an admitted SI-valued command against the current lifecycle
    /// permit and execute it through the active CiA 402 path.
    pub fn run_with_procbuf_command(
        &mut self,
        command: &CommandPage<AXES, IO>,
        cycle_period_ns: u64,
        guards: &mut [CyclicSetpointGuard; AXES],
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let prepared = prepare_cia402_command(
            command,
            self.guard,
            self.modes,
            self.axis_policies,
            cycle_period_ns,
            self.port.now_ns().max(self.now_ns),
        )
        .map_err(StopCycleError::Command)?;
        self.run_inner::<1>(
            None,
            Some(MotionInputs {
                targets: prepared.targets(),
                guards,
                limits: prepared.limits(),
                permit: Some(prepared.permit()),
            }),
            None,
            None,
        )
    }

    /// Deadline-checked variant of [`Self::run_with_procbuf_command`].
    pub fn run_with_procbuf_command_until(
        &mut self,
        command: &CommandPage<AXES, IO>,
        cycle_period_ns: u64,
        guards: &mut [CyclicSetpointGuard; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let prepared = prepare_cia402_command(
            command,
            self.guard,
            self.modes,
            self.axis_policies,
            cycle_period_ns,
            self.port.now_ns().max(self.now_ns),
        )
        .map_err(StopCycleError::Command)?;
        self.run_inner::<1>(
            None,
            Some(MotionInputs {
                targets: prepared.targets(),
                guards,
                limits: prepared.limits(),
                permit: Some(prepared.permit()),
            }),
            None,
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
        self.run_inner(
            Some((schedule, domains, motion_domain_id)),
            None,
            None,
            None,
        )
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
                permit: None,
            }),
            None,
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
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
        )
    }

    /// Scheduled-Domain variant using an admitted ProcBuf command rather than
    /// caller-constructed raw targets and limits.
    #[allow(clippy::too_many_arguments)]
    pub fn run_scheduled_with_procbuf_command<const SCHEDULE_SLOTS: usize>(
        &mut self,
        schedule: &ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        domains: &[ScheduledDomainQuality; DOMAINS],
        motion_domain_id: u8,
        command: &CommandPage<AXES, IO>,
        cycle_period_ns: u64,
        guards: &mut [CyclicSetpointGuard; AXES],
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let prepared = prepare_cia402_command(
            command,
            self.guard,
            self.modes,
            self.axis_policies,
            cycle_period_ns,
            self.port.now_ns().max(self.now_ns),
        )
        .map_err(StopCycleError::Command)?;
        self.run_inner(
            Some((schedule, domains, motion_domain_id)),
            Some(MotionInputs {
                targets: prepared.targets(),
                guards,
                limits: prepared.limits(),
                permit: Some(prepared.permit()),
            }),
            None,
            None,
        )
    }

    /// Deadline-checked variant of
    /// [`Self::run_scheduled_with_procbuf_command`].
    #[allow(clippy::too_many_arguments)]
    pub fn run_scheduled_with_procbuf_command_until<const SCHEDULE_SLOTS: usize>(
        &mut self,
        schedule: &ScheduleTable<DOMAINS, SCHEDULE_SLOTS>,
        domains: &[ScheduledDomainQuality; DOMAINS],
        motion_domain_id: u8,
        command: &CommandPage<AXES, IO>,
        cycle_period_ns: u64,
        guards: &mut [CyclicSetpointGuard; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let prepared = prepare_cia402_command(
            command,
            self.guard,
            self.modes,
            self.axis_policies,
            cycle_period_ns,
            self.port.now_ns().max(self.now_ns),
        )
        .map_err(StopCycleError::Command)?;
        self.run_inner(
            Some((schedule, domains, motion_domain_id)),
            Some(MotionInputs {
                targets: prepared.targets(),
                guards,
                limits: prepared.limits(),
                permit: Some(prepared.permit()),
            }),
            None,
            Some(cycle_deadline_ns),
        )
    }

    /// Submit each safe auxiliary Domain due for the next shared RX before
    /// considering active motion output. An auxiliary failure stops remaining
    /// auxiliary TX, revokes motion authority, and still attempts the motion
    /// Domain's stop frame.
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
        let other = self.other;
        self.run_inner_with_outputs(
            Some((schedule, domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Bind the output decision to the same finalized shared RX report that
    /// supplied the motion Domain. The bank supplies the frozen Domain order;
    /// callers cannot substitute a stale or rearranged quality array. This
    /// still leaves DC/control TX and its service-result projection with the
    /// outer cycle owner. The supplied deadline is checked again after State
    /// and event publication.
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
        let domains = self.received_domains(domain_bank, received, motion_domain_id, outputs)?;
        let other = self.other;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Stable-cycle variant with a product-configured controlled stop runtime.
    /// Active cycles still use the normal setpoint guard. Once the MLG enters
    /// Stopping, Hold/RampToZero are attempted transactionally; any controlled
    /// error latches this transition sequence to the default fail-closed stop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_received_with_controlled_stop_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        received: &ScheduledReceiveReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        controlled: &mut ControlledStopCycleState<AXES>,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let domains = self.received_domains(domain_bank, received, motion_domain_id, outputs)?;
        let other = self.other;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            Some(controlled),
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Consume the complete bounded mailbox/DC service result, project its
    /// failures into lifecycle facts, and finish due auxiliary/motion output,
    /// State/event publication, and the final deadline observation. Process
    /// Domain submission and task release remain owned by the outer cycle.
    #[allow(clippy::too_many_arguments)]
    pub fn run_mailbox_cycle_with_outputs_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledMailboxCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let (domains, other) =
            self.mailbox_cycle_inputs(domain_bank, cycle, motion_domain_id, outputs)?;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Consume a complete non-mailbox control/DC cycle, bind the matching
    /// service FSM readiness to its declared gate, then run auxiliary/motion
    /// output, State/event publication, and the final deadline observation.
    #[allow(clippy::too_many_arguments)]
    pub fn run_control_cycle_with_outputs_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledControlCycleReport<P::Error, DOMAINS>,
        gate: ScheduledControlGate,
        service_ready: bool,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let (domains, other) = self.control_cycle_inputs(
            domain_bank,
            cycle,
            gate,
            service_ready,
            motion_domain_id,
            outputs,
        )?;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Control-service stable-cycle entry with controlled Hold/RampToZero and
    /// a sequence-latched fail-closed fallback.
    #[allow(clippy::too_many_arguments)]
    pub fn run_control_cycle_with_controlled_stop_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledControlCycleReport<P::Error, DOMAINS>,
        gate: ScheduledControlGate,
        service_ready: bool,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        controlled: &mut ControlledStopCycleState<AXES>,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let (domains, other) = self.control_cycle_inputs(
            domain_bank,
            cycle,
            gate,
            service_ready,
            motion_domain_id,
            outputs,
        )?;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            Some(controlled),
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Consume one scheduler-owned startup/configuration/mailbox service
    /// cycle. Service selection and readiness are part of the verified report;
    /// the caller supplies only motion/output ownership and the final deadline.
    #[allow(clippy::too_many_arguments)]
    pub fn run_service_cycle_with_outputs_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledProductionServiceCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let (domains, other) =
            self.service_cycle_inputs(domain_bank, cycle, motion_domain_id, outputs)?;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Unified production-service entry with product-configured controlled
    /// stopping. The returned outcome and `ScheduledProductionRelease` expose
    /// whether the controlled frame was used or the transition is latched to
    /// the default stop path.
    #[allow(clippy::too_many_arguments)]
    pub fn run_service_cycle_with_controlled_stop_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledProductionServiceCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        controlled: &mut ControlledStopCycleState<AXES>,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let (domains, other) =
            self.service_cycle_inputs(domain_bank, cycle, motion_domain_id, outputs)?;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            Some(controlled),
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Unified production-service entry using the current admitted ProcBuf
    /// command and the product-configured controlled-stop fallback.
    #[allow(clippy::too_many_arguments)]
    pub fn run_service_cycle_with_procbuf_command_and_controlled_stop_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledProductionServiceCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        command: &CommandPage<AXES, IO>,
        cycle_period_ns: u64,
        guards: &mut [CyclicSetpointGuard; AXES],
        controlled: &mut ControlledStopCycleState<AXES>,
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let prepared = prepare_cia402_command(
            command,
            self.guard,
            self.modes,
            self.axis_policies,
            cycle_period_ns,
            self.port.now_ns().max(self.now_ns),
        )
        .map_err(StopCycleError::Command)?;
        let (domains, other) =
            self.service_cycle_inputs(domain_bank, cycle, motion_domain_id, outputs)?;
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets: prepared.targets(),
                guards,
                limits: prepared.limits(),
                permit: Some(prepared.permit()),
            }),
            Some(controlled),
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Consume both bounded pre-RX stages as one causal input. The process
    /// submission report must match the frozen due traversal and the same
    /// finalized bank cycle as the mailbox/DC report. A rejected process frame
    /// or process-stage deadline miss clears budget qualification before any
    /// auxiliary or motion output is attempted.
    #[allow(clippy::too_many_arguments)]
    pub fn run_process_mailbox_cycle_with_outputs_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
        const PROCESS_FRAMES: usize,
        const PROCESS_DATAGRAMS: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        process_inputs: &ScheduledProcessInputs<
            '_,
            DOMAINS,
            SCHEDULE_SLOTS,
            PROCESS_FRAMES,
            PROCESS_DATAGRAMS,
        >,
        process: &ScheduledProcessTxReport<P::Error>,
        cycle: &ScheduledMailboxCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        if !domain_bank.confirms_process_tx(process_inputs, process) {
            return Err(StopCycleError::ProcessCycleMismatch);
        }
        let (domains, other) =
            self.mailbox_cycle_inputs(domain_bank, cycle, motion_domain_id, outputs)?;
        let other = other_cycle_facts_from_process_tx(process, other);
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Initial priming variant of `run_control_cycle_with_outputs_until`.
    /// The explicit process submission must describe the same finalized bank
    /// cycle before any output or lifecycle publication is attempted.
    #[allow(clippy::too_many_arguments)]
    pub fn run_process_control_cycle_with_outputs_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
        const PROCESS_FRAMES: usize,
        const PROCESS_DATAGRAMS: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        process_inputs: &ScheduledProcessInputs<
            '_,
            DOMAINS,
            SCHEDULE_SLOTS,
            PROCESS_FRAMES,
            PROCESS_DATAGRAMS,
        >,
        process: &ScheduledProcessTxReport<P::Error>,
        cycle: &ScheduledControlCycleReport<P::Error, DOMAINS>,
        gate: ScheduledControlGate,
        service_ready: bool,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        if !domain_bank.confirms_process_tx(process_inputs, process) {
            return Err(StopCycleError::ProcessCycleMismatch);
        }
        let (domains, other) = self.control_cycle_inputs(
            domain_bank,
            cycle,
            gate,
            service_ready,
            motion_domain_id,
            outputs,
        )?;
        let other = other_cycle_facts_from_process_tx(process, other);
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    /// Initial priming variant of `run_service_cycle_with_outputs_until`.
    /// Process submission and service transport must describe the same
    /// finalized bank cycle before lifecycle publication starts.
    #[allow(clippy::too_many_arguments)]
    pub fn run_process_service_cycle_with_outputs_until<
        const SCHEDULE_SLOTS: usize,
        const FRAMES: usize,
        const PROCESS_FRAMES: usize,
        const PROCESS_DATAGRAMS: usize,
    >(
        &mut self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        process_inputs: &ScheduledProcessInputs<
            '_,
            DOMAINS,
            SCHEDULE_SLOTS,
            PROCESS_FRAMES,
            PROCESS_DATAGRAMS,
        >,
        process: &ScheduledProcessTxReport<P::Error>,
        cycle: &ScheduledProductionServiceCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
        targets: &[Option<Cia402Target>; AXES],
        guards: &mut [CyclicSetpointGuard; AXES],
        limits: &[CyclicLimits; AXES],
        cycle_deadline_ns: u64,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        if !domain_bank.confirms_process_tx(process_inputs, process) {
            return Err(StopCycleError::ProcessCycleMismatch);
        }
        let (domains, other) =
            self.service_cycle_inputs(domain_bank, cycle, motion_domain_id, outputs)?;
        let other = other_cycle_facts_from_process_tx(process, other);
        self.run_inner_with_outputs(
            Some((outputs.schedule, &domains, motion_domain_id)),
            Some(MotionInputs {
                targets,
                guards,
                limits,
                permit: None,
            }),
            None,
            Some(cycle_deadline_ns),
            Some(outputs),
            other,
        )
    }

    fn control_cycle_inputs<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledControlCycleReport<P::Error, DOMAINS>,
        gate: ScheduledControlGate,
        service_ready: bool,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
    ) -> Result<([ScheduledDomainQuality; DOMAINS], OtherCycleFacts), StopCycleError> {
        if !domain_bank.confirms_control_cycle(cycle) {
            return Err(StopCycleError::ServiceCycleMismatch);
        }
        let domains =
            self.received_domains(domain_bank, cycle.received(), motion_domain_id, outputs)?;
        let other = other_cycle_facts_from_control_cycle(cycle, gate, service_ready, self.other);
        Ok((domains, other))
    }

    fn service_cycle_inputs<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledProductionServiceCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
    ) -> Result<([ScheduledDomainQuality; DOMAINS], OtherCycleFacts), StopCycleError> {
        if !domain_bank.confirms_production_service_cycle(cycle) {
            return Err(StopCycleError::ServiceCycleMismatch);
        }
        let domains =
            self.received_domains(domain_bank, cycle.received(), motion_domain_id, outputs)?;
        let other = other_cycle_facts_from_production_service_cycle(cycle, self.other);
        Ok((domains, other))
    }

    fn mailbox_cycle_inputs<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        cycle: &ScheduledMailboxCycleReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
    ) -> Result<([ScheduledDomainQuality; DOMAINS], OtherCycleFacts), StopCycleError> {
        let service = &cycle.tx.service;
        let service_shape_invalid = cycle.request != cycle.tx.request
            || (cycle.receive.mailbox_progress.is_some() && cycle.request.is_some())
            || (cycle.post_receive_deadline_met && !service.post_tx_deadline_met)
            || (service.failure.is_none() && !service.dc_sent)
            || (service.control_sent && !service.dc_sent)
            || matches!(
                service.failure,
                Some(ScheduledServiceTxFailure::Dc(_)) if service.dc_sent
            )
            || matches!(
                service.failure,
                Some(ScheduledServiceTxFailure::Control(_))
                    if !service.dc_sent || service.control_sent
            );
        if service_shape_invalid {
            return Err(StopCycleError::ServiceCycleMismatch);
        }
        let domains = self.received_domains(
            domain_bank,
            &cycle.receive.received,
            motion_domain_id,
            outputs,
        )?;
        let other = other_cycle_facts_from_mailbox_cycle(cycle, self.other);
        Ok((domains, other))
    }

    fn received_domains<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &self,
        domain_bank: &ScheduledDomainBank<'_, DOMAINS, SCHEDULE_SLOTS>,
        received: &ScheduledReceiveReport<P::Error, DOMAINS>,
        motion_domain_id: u8,
        outputs: &ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>,
    ) -> Result<[ScheduledDomainQuality; DOMAINS], StopCycleError> {
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
        Ok(core::array::from_fn(|slot| ScheduledDomainQuality {
            id: outputs.schedule.domains()[slot].id,
            quality: received.qualities[slot],
        }))
    }

    fn run_inner<const SCHEDULE_SLOTS: usize>(
        &mut self,
        scheduled: Option<ScheduledInputs<'_, DOMAINS, SCHEDULE_SLOTS>>,
        motion: Option<MotionInputs<'_, AXES>>,
        controlled: Option<&mut ControlledStopCycleState<AXES>>,
        cycle_deadline_ns: Option<u64>,
    ) -> Result<StopCycleOutcome<P::Error>, StopCycleError> {
        let other = self.other;
        self.run_inner_with_outputs::<SCHEDULE_SLOTS, 1>(
            scheduled,
            motion,
            controlled,
            cycle_deadline_ns,
            None,
            other,
        )
    }

    fn run_inner_with_outputs<const SCHEDULE_SLOTS: usize, const FRAMES: usize>(
        &mut self,
        scheduled: Option<ScheduledInputs<'_, DOMAINS, SCHEDULE_SLOTS>>,
        motion: Option<MotionInputs<'_, AXES>>,
        mut controlled: Option<&mut ControlledStopCycleState<AXES>>,
        cycle_deadline_ns: Option<u64>,
        outputs: Option<&ScheduledAuxiliaryOutputs<'_, DOMAINS, SCHEDULE_SLOTS, FRAMES, DATAGRAMS>>,
        other: OtherCycleFacts,
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
        let next_receive_cycle = if outputs.is_some() {
            Some(
                self.report
                    .cycle
                    .checked_add(1)
                    .ok_or(StopCycleError::CycleMismatch)?,
            )
        } else {
            None
        };
        if cycle_deadline_ns == Some(0) {
            return Err(StopCycleError::InvalidCycleDeadline);
        }

        let previous_transition_sequence = self.guard.transition_sequence;
        let mut decision_now_ns = cycle_deadline_ns
            .map(|_| self.port.now_ns().max(self.now_ns))
            .unwrap_or(self.now_ns);
        let mut other = other;
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
        let mut process_due_mask = 0;
        let mut expected_auxiliary_frames = 0;
        if let Some(outputs) = outputs {
            let Some(receive_cycle) = next_receive_cycle else {
                return Err(StopCycleError::InvalidAuxiliaryOutputs);
            };
            let (due_mask, expected_frames) = outputs.due_summary(receive_cycle);
            process_due_mask = due_mask;
            expected_auxiliary_frames = expected_frames;
            // A failed pre-TX deadline must not emit auxiliary outputs either.
            if quality.cycle_within_budget {
                let sent = outputs.submit_due(
                    receive_cycle,
                    self.master,
                    self.port,
                    self.next_generation,
                    self.deadline_ns,
                    cycle_deadline_ns,
                );
                debug_assert_eq!(sent.due_mask, process_due_mask);
                debug_assert_eq!(sent.expected_frames, expected_auxiliary_frames);
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
        let (
            mut action,
            mut transmission,
            feedback,
            mut stop_mask,
            mut controlled_stop_used,
            mut controlled_stop_fallback,
            controlled_stop_failure,
            mut accepted_outputs,
        ) = {
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
            let mut controlled_report: Option<ControlledStopFrameReport<AXES>> = None;
            let mut controlled_stop_used = false;
            let mut controlled_stop_fallback = false;
            let mut controlled_stop_failure = None;
            let transmission = match (action, motion) {
                (LifecycleAction::Stop(_), _) => {
                    if let Some(controlled) = controlled.as_deref_mut() {
                        let sequence = decision.transition_sequence();
                        controlled.prepare(sequence);
                        if controlled.fallback_latched() {
                            controlled_stop_fallback = true;
                            submit_stopping_frame(
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
                            )
                        } else {
                            let attempt = {
                                let (planners, limits) = controlled.parts();
                                submit_controlled_stopping_frame(
                                    &mut decision,
                                    self.report,
                                    planners,
                                    limits,
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
                            };
                            match attempt {
                                Ok(report) => {
                                    controlled_stop_used = true;
                                    let length = report.length;
                                    controlled_report = Some(report);
                                    Ok(length)
                                }
                                Err(error) => {
                                    controlled.latch_fallback(sequence);
                                    controlled_stop_fallback = true;
                                    controlled_stop_failure = Some(error);
                                    submit_stopping_frame(
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
                                    )
                                }
                            }
                        }
                    } else {
                        submit_stopping_frame(
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
                        )
                    }
                }
                (LifecycleAction::EnableAllowed, Some(motion)) => {
                    let mut next_guards = *motion.guards;
                    for guard in &mut next_guards {
                        guard.bind_activation(activation.0, activation.1);
                    }
                    let transmission = if let Some(permit) = motion.permit {
                        submit_prepared_active_frame(
                            &decision,
                            permit,
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
                        )
                    } else {
                        submit_active_frame(
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
                        )
                    };
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
                if let Some(report) = controlled_report.as_ref() {
                    controlled_axis_stops_to_procbuf(self.state, &decision, report, feedback)
                        .map_err(StopCycleError::Evidence)?;
                } else {
                    axis_stops_to_procbuf(self.state, &decision, &outputs, feedback)
                        .map_err(StopCycleError::Evidence)?;
                }
            }
            let accepted_outputs = transmission.as_ref().ok().map(|_| {
                controlled_report
                    .as_ref()
                    .map_or(outputs, |report| report.outputs)
            });
            (
                action,
                transmission,
                feedback,
                decision.stopping_axis_mask(),
                controlled_stop_used,
                controlled_stop_fallback,
                controlled_stop_failure,
                accepted_outputs,
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
            if let Some(controlled) = controlled {
                controlled.latch_fallback(stop_decision.transition_sequence());
                controlled_stop_used = false;
                controlled_stop_fallback = true;
            }
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
            accepted_outputs = stop_transmission.as_ref().ok().map(|_| stop_outputs);
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
        cia402_feedback_to_procbuf(
            self.state,
            self.domain.input(),
            self.maps,
            self.modes,
            self.axis_policies,
            quality.domain_valid && quality.wkc_valid,
            accepted_outputs.as_ref(),
        )
        .map_err(StopCycleError::Feedback)?;
        self.state.lifecycle = lifecycle_to_procbuf(
            self.guard.snapshot(self.report.cycle, publish_now_ns),
            if self.guard.transition_sequence != previous_transition_sequence {
                publish_now_ns
            } else {
                self.transition_time_ns
            },
        );
        self.state.monotonic_time_ns = publish_now_ns;
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
        let process_handoff = next_receive_cycle.map(|cycle| ScheduledProcessHandoff {
            cycle,
            generation: self.next_generation,
            rx_deadline_ns: self.deadline_ns,
            due_mask: process_due_mask,
            expected_frames: expected_auxiliary_frames.saturating_add(1),
            sent_frames: auxiliary_frames_sent + usize::from(transmission.is_ok()),
        });
        Ok(StopCycleOutcome {
            action,
            active_failure,
            controlled_stop_used,
            controlled_stop_fallback,
            controlled_stop_failure,
            auxiliary_frames_sent,
            auxiliary_failure,
            post_tx_deadline_met,
            post_publication_deadline_met,
            deadline_correction_publish,
            deadline_correction_events,
            active_tx_before_deadline_miss,
            process_handoff,
            quality,
            feedback,
            acknowledged,
            transmission,
            state_publish,
            event_publish,
        })
    }
}
