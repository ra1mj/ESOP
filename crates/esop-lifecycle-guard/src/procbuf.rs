//! Fixed-size projection for the optional ProcBuf ABI boundary.

use crate::{CyclicQuality, LifecycleSnapshot};
use esop_procbuf::{CyclicQualityMask, LifecycleSummary, QualityFact, StatePage};

use crate::{LifecycleGuard, LifecycleState, MAX_MOTION_AXES, STOP_TIMEOUT_FAULT_CODE, StopAction};
use esop_procbuf::{EventPushError, EventSeverity, HeaderError, ProcBuf, ProcBufEvent};

#[cfg(feature = "cia402")]
use crate::{AxisCycleDecision, AxisDirective, StopFeedback};
#[cfg(feature = "cia402")]
use esop_procbuf::AxisStopEvidence;
#[cfg(feature = "cia402")]
use esop_profile_cia402::{CONTROLWORD_DISABLE_VOLTAGE, CONTROLWORD_QUICK_STOP, Cia402Output};

#[cfg(feature = "ethercat")]
use crate::ethercat::{
    OtherCycleFacts, ScheduledDomainQuality, cyclic_quality_from_domains,
    cyclic_quality_from_schedule,
};
#[cfg(feature = "ethercat")]
use esop_ethercat_core::{
    CycleReport, DcCyclicSync, DomainQuality as EthercatDomainQuality, ScheduleTable,
};
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
    }
}
