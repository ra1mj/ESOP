//! Allocation-free projection of verified EtherCAT cycle evidence into MLG facts.

use crate::CyclicQuality;
use esop_ethercat_core::{
    CycleReport, DcCyclicSync, DomainQuality, MailboxProgress, ScheduleTable,
    ScheduledMailboxCycleReport, ScheduledProcessTxReport,
};

#[cfg(feature = "cia402")]
use crate::{
    AxisCycleDecision, AxisDirective, LifecycleAction, MAX_MOTION_AXES, StopAction, StopFeedback,
    cia402::stop_feedback_from_cia402,
};
#[cfg(feature = "cia402")]
use esop_ethercat_core::{
    CycleError, Domain, EthercatMaster, EthercatPort, FramePlan, FramePoolError, wire::Command,
};
#[cfg(feature = "cia402")]
use esop_profile_cia402::{
    CONTROLWORD_DISABLE_VOLTAGE, CONTROLWORD_QUICK_STOP, Cia402Controller, Cia402MotionGate,
    Cia402Output, Cia402PdoCommand, Cia402PdoError, Cia402PdoField, Cia402PdoMap, Cia402Target,
    CyclicLimits, CyclicSetpoint, CyclicSetpointError, CyclicSetpointGuard, DriveRequest,
    DriveState, OperatingMode,
};

/// A stopped-axis frame is prepared but does not count as issued until the
/// platform accepts it. Callers must supply a frozen safe image for all other
/// outputs; this function only overwrites the CiA 402 control and mode fields.
#[cfg(feature = "cia402")]
#[derive(Debug)]
pub enum StopFrameError<E> {
    InvalidDecision,
    UnverifiedInput,
    AxisCapacityExceeded,
    InvalidDeadline,
    UnsafeOutput(usize),
    MissingTarget(usize),
    UnexpectedTarget(usize),
    InvalidTarget(usize, CyclicSetpointError),
    UncoveredOutput(usize, Cia402PdoField),
    OverlappingOutput(usize, usize),
    Pdo(usize, Cia402PdoError),
    FramePool(FramePoolError),
    Build(CycleError<core::convert::Infallible>),
    Transmit(CycleError<E>),
}

/// Submit an active, single-Domain cycle only after the completed receive and
/// lifecycle decision agree. A permitted axis may handshake without a target
/// until Switched On; the enable-operation edge seeds an unseeded guard from
/// verified actual feedback and must carry that actual-feedback hold target.
/// Operation Enabled requires a seeded, bounded target in the confirmed mode.
/// The caller must reset guards on a new activation (the cycle context binds
/// them automatically) and provide a safe image for all other outputs.
/// Failed validation/build/TX cannot advance targets.
#[cfg(feature = "cia402")]
#[allow(clippy::too_many_arguments)]
pub fn submit_active_frame<
    P: EthercatPort,
    const AXES: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
>(
    decision: &AxisCycleDecision<'_>,
    report: CycleReport,
    outputs: &[Cia402Output; AXES],
    targets: &[Option<Cia402Target>; AXES],
    guards: &mut [CyclicSetpointGuard; AXES],
    limits: &[CyclicLimits; AXES],
    maps: &[Cia402PdoMap; AXES],
    modes: &[OperatingMode; AXES],
    safe_process_image: &[u8; BYTES],
    domain: &Domain<BYTES, SEGMENTS>,
    plan: &FramePlan<DATAGRAMS>,
    master: &mut EthercatMaster<SLOTS, MTU>,
    port: &mut P,
    generation: u16,
    deadline_ns: u64,
) -> Result<usize, StopFrameError<P::Error>> {
    let permitted = decision.permitted_axis_mask();
    if !matches!(decision.action(), LifecycleAction::EnableAllowed)
        || decision.stopping_axis_mask() != 0
        || permitted == 0
    {
        return Err(StopFrameError::InvalidDecision);
    }
    if AXES > MAX_MOTION_AXES || (AXES < MAX_MOTION_AXES && permitted >> AXES != 0) {
        return Err(StopFrameError::AxisCapacityExceeded);
    }
    if decision.cycle() != report.cycle
        || report.budget_exhausted
        || !rx_current(report)
        || !domain_current(report, domain.quality())
    {
        return Err(StopFrameError::UnverifiedInput);
    }
    if deadline_ns <= port.now_ns() {
        return Err(StopFrameError::InvalidDeadline);
    }

    // Even an inactive target PDO cannot alias a different axis's active
    // target: the frame would otherwise move an unpermitted axis.
    const OUTPUT_FIELDS: [Cia402PdoField; 5] = [
        Cia402PdoField::Controlword,
        Cia402PdoField::ModeOfOperation,
        Cia402PdoField::TargetPosition,
        Cia402PdoField::TargetVelocity,
        Cia402PdoField::TargetTorque,
    ];
    for (axis, map) in maps.iter().enumerate() {
        for (prior_axis, prior) in maps.iter().enumerate().take(axis) {
            for field in OUTPUT_FIELDS {
                let Some(entry) = map.entry(field) else {
                    continue;
                };
                for prior_field in OUTPUT_FIELDS {
                    if prior.entry(prior_field).is_some_and(|earlier| {
                        entry.bit_offset
                            < earlier
                                .bit_offset
                                .saturating_add(earlier.bit_length as usize)
                            && earlier.bit_offset
                                < entry.bit_offset.saturating_add(entry.bit_length as usize)
                    }) {
                        return Err(StopFrameError::OverlappingOutput(prior_axis, axis));
                    }
                }
            }
        }
    }

    let mut image = *safe_process_image;
    let mut accepted = *guards;
    for axis in 0..AXES {
        let inputs = maps[axis]
            .read_inputs_for(domain.input(), modes[axis])
            .map_err(|error| StopFrameError::Pdo(axis, error))?;
        let authorized = permitted & (1u32 << axis) != 0;
        let request = if authorized {
            DriveRequest::Enable
        } else {
            DriveRequest::Disable
        };
        let output = outputs[axis];
        if output != Cia402Controller::new().step(inputs.statusword, request, authorized) {
            return Err(StopFrameError::UnsafeOutput(axis));
        }
        let mut fields = [
            Cia402PdoField::Controlword,
            Cia402PdoField::ModeOfOperation,
            Cia402PdoField::Controlword,
        ];
        let mut field_count = 2;
        match (authorized, output.state, targets[axis]) {
            (false, _, None)
            | (true, DriveState::SwitchOnDisabled | DriveState::ReadyToSwitchOn, None) => {
                maps[axis]
                    .write_control(&mut image, modes[axis], output.controlword)
                    .map_err(|error| StopFrameError::Pdo(axis, error))?;
            }
            (false, _, Some(_))
            | (true, DriveState::SwitchOnDisabled | DriveState::ReadyToSwitchOn, Some(_)) => {
                return Err(StopFrameError::UnexpectedTarget(axis));
            }
            (true, DriveState::SwitchedOn | DriveState::OperationEnabled, None) => {
                return Err(StopFrameError::MissingTarget(axis));
            }
            (true, DriveState::SwitchedOn | DriveState::OperationEnabled, Some(target)) => {
                if output.state == DriveState::SwitchedOn {
                    maps[axis]
                        .write_enable_with_actual_target(
                            &mut image,
                            Cia402PdoCommand {
                                controlword: output.controlword,
                                mode: modes[axis],
                                target,
                            },
                            inputs,
                        )
                        .map_err(|error| StopFrameError::Pdo(axis, error))?;
                    if !accepted[axis].seeded() {
                        let actual = match target {
                            Cia402Target::Position(value) => CyclicSetpoint {
                                position: f64::from(value),
                                ..CyclicSetpoint::ZERO
                            },
                            Cia402Target::Velocity(value) => CyclicSetpoint {
                                velocity: f64::from(value),
                                ..CyclicSetpoint::ZERO
                            },
                            Cia402Target::Torque(value) => CyclicSetpoint {
                                torque: f64::from(value),
                                ..CyclicSetpoint::ZERO
                            },
                        };
                        accepted[axis]
                            .seed_from_actual(actual)
                            .map_err(|error| StopFrameError::InvalidTarget(axis, error))?;
                    }
                }
                let current = accepted[axis].last();
                let setpoint = match target {
                    Cia402Target::Position(value) => CyclicSetpoint {
                        position: value as f64,
                        ..current
                    },
                    Cia402Target::Velocity(value) => CyclicSetpoint {
                        velocity: value as f64,
                        ..current
                    },
                    Cia402Target::Torque(value) => CyclicSetpoint {
                        torque: value as f64,
                        ..current
                    },
                };
                let mode_ready = inputs.actual_mode == modes[axis];
                accepted[axis]
                    .validate_and_accept(modes[axis], mode_ready, setpoint, limits[axis])
                    .map_err(|error| StopFrameError::InvalidTarget(axis, error))?;
                let command = Cia402PdoCommand {
                    controlword: output.controlword,
                    mode: modes[axis],
                    target,
                };
                if output.state == DriveState::OperationEnabled {
                    maps[axis]
                        .write_cyclic(
                            &mut image,
                            command,
                            Cia402MotionGate {
                                lifecycle_permit: authorized,
                                mode_confirmed: mode_ready,
                                operation_enabled: output.operation_enabled,
                                setpoint_valid: true,
                            },
                        )
                        .map_err(|error| StopFrameError::Pdo(axis, error))?;
                }
                let field = match target {
                    Cia402Target::Position(_) => Cia402PdoField::TargetPosition,
                    Cia402Target::Velocity(_) => Cia402PdoField::TargetVelocity,
                    Cia402Target::Torque(_) => Cia402PdoField::TargetTorque,
                };
                fields[2] = field;
                field_count = 3;
            }
            (true, _, _) => return Err(StopFrameError::UnsafeOutput(axis)),
        }
        for field in fields.iter().take(field_count) {
            if !writable_domain_field(&maps[axis], *field, domain, plan) {
                return Err(StopFrameError::UncoveredOutput(axis, *field));
            }
        }
    }
    let length = transmit_image(&image, plan, master, port, generation, deadline_ns)?;
    *guards = accepted;
    Ok(length)
}

/// Submit one frozen, writable Domain plan for the current stop decision.
/// The same decision must be used to derive `outputs` and later ProcBuf proof.
/// A failed build or TX never marks a stop as issued; the caller must still
/// publish the failed cycle and retry or escalate according to its policy.
#[cfg(feature = "cia402")]
#[allow(clippy::too_many_arguments)]
pub fn submit_stopping_frame<
    P: EthercatPort,
    const AXES: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
>(
    decision: &mut AxisCycleDecision<'_>,
    outputs: &[Cia402Output; AXES],
    maps: &[Cia402PdoMap; AXES],
    modes: &[OperatingMode; AXES],
    safe_process_image: &[u8; BYTES],
    domain: &Domain<BYTES, SEGMENTS>,
    plan: &FramePlan<DATAGRAMS>,
    master: &mut EthercatMaster<SLOTS, MTU>,
    port: &mut P,
    generation: u16,
    deadline_ns: u64,
) -> Result<usize, StopFrameError<P::Error>> {
    let stopping = decision.stopping_axis_mask();
    if !matches!(decision.action(), LifecycleAction::Stop(_)) || stopping == 0 {
        return Err(StopFrameError::InvalidDecision);
    }
    let length = submit_safe_frame(
        decision,
        outputs,
        maps,
        modes,
        safe_process_image,
        domain,
        plan,
        master,
        port,
        generation,
        deadline_ns,
    )?;
    decision
        .mark_stop_transmitted()
        .map_err(|_| StopFrameError::InvalidDecision)?;
    Ok(length)
}

/// Submit a Disable-only frame while the guard holds or has latched a fault.
/// This shares the stop frame's writable-Domain and cross-axis alias checks,
/// but never records stop issuance or treats a fault-latched state as motion.
#[cfg(feature = "cia402")]
#[allow(clippy::too_many_arguments)]
pub fn submit_inhibited_frame<
    P: EthercatPort,
    const AXES: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
>(
    decision: &AxisCycleDecision<'_>,
    outputs: &[Cia402Output; AXES],
    maps: &[Cia402PdoMap; AXES],
    modes: &[OperatingMode; AXES],
    safe_process_image: &[u8; BYTES],
    domain: &Domain<BYTES, SEGMENTS>,
    plan: &FramePlan<DATAGRAMS>,
    master: &mut EthercatMaster<SLOTS, MTU>,
    port: &mut P,
    generation: u16,
    deadline_ns: u64,
) -> Result<usize, StopFrameError<P::Error>> {
    if !matches!(
        decision.action(),
        LifecycleAction::Hold | LifecycleAction::FaultLatched
    ) || decision.stopping_axis_mask() != 0
        || decision.permitted_axis_mask() != 0
    {
        return Err(StopFrameError::InvalidDecision);
    }
    submit_safe_frame(
        decision,
        outputs,
        maps,
        modes,
        safe_process_image,
        domain,
        plan,
        master,
        port,
        generation,
        deadline_ns,
    )
}

#[cfg(feature = "cia402")]
#[allow(clippy::too_many_arguments)]
fn submit_safe_frame<
    P: EthercatPort,
    const AXES: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
>(
    decision: &AxisCycleDecision<'_>,
    outputs: &[Cia402Output; AXES],
    maps: &[Cia402PdoMap; AXES],
    modes: &[OperatingMode; AXES],
    safe_process_image: &[u8; BYTES],
    domain: &Domain<BYTES, SEGMENTS>,
    plan: &FramePlan<DATAGRAMS>,
    master: &mut EthercatMaster<SLOTS, MTU>,
    port: &mut P,
    generation: u16,
    deadline_ns: u64,
) -> Result<usize, StopFrameError<P::Error>> {
    let stopping = decision.stopping_axis_mask();
    if AXES > MAX_MOTION_AXES || (AXES < MAX_MOTION_AXES && stopping >> AXES != 0) {
        return Err(StopFrameError::AxisCapacityExceeded);
    }
    if deadline_ns <= port.now_ns() {
        return Err(StopFrameError::InvalidDeadline);
    }

    let mut image = *safe_process_image;
    for axis in 0..AXES {
        let expected = match decision.axis(axis) {
            AxisDirective::Stop(StopAction::QuickStop) => CONTROLWORD_QUICK_STOP,
            AxisDirective::Stop(_) | AxisDirective::Inhibit => CONTROLWORD_DISABLE_VOLTAGE,
            AxisDirective::EnableAllowed => return Err(StopFrameError::InvalidDecision),
        };
        let output = outputs[axis];
        if output.controlword != expected || output.motion_allowed || output.fault_reset_pulse {
            return Err(StopFrameError::UnsafeOutput(axis));
        }
        for field in [Cia402PdoField::Controlword, Cia402PdoField::ModeOfOperation] {
            if !writable_domain_field(&maps[axis], field, domain, plan) {
                return Err(StopFrameError::UncoveredOutput(axis, field));
            }
            let Some(current) = maps[axis].entry(field) else {
                return Err(StopFrameError::UncoveredOutput(axis, field));
            };
            for (earlier, prior_map) in maps.iter().enumerate().take(axis) {
                for prior_field in [Cia402PdoField::Controlword, Cia402PdoField::ModeOfOperation] {
                    if prior_map.entry(prior_field).is_some_and(|prior| {
                        current.bit_offset
                            < prior.bit_offset.saturating_add(prior.bit_length as usize)
                            && prior.bit_offset
                                < current
                                    .bit_offset
                                    .saturating_add(current.bit_length as usize)
                    }) {
                        return Err(StopFrameError::OverlappingOutput(earlier, axis));
                    }
                }
            }
        }
        maps[axis]
            .write_control(&mut image, modes[axis], output.controlword)
            .map_err(|error| StopFrameError::Pdo(axis, error))?;
    }

    transmit_image(&image, plan, master, port, generation, deadline_ns)
}

#[cfg(feature = "cia402")]
fn transmit_image<
    P: EthercatPort,
    const BYTES: usize,
    const DATAGRAMS: usize,
    const SLOTS: usize,
    const MTU: usize,
>(
    image: &[u8; BYTES],
    plan: &FramePlan<DATAGRAMS>,
    master: &mut EthercatMaster<SLOTS, MTU>,
    port: &mut P,
    generation: u16,
    deadline_ns: u64,
) -> Result<usize, StopFrameError<P::Error>> {
    let now_ns = port.now_ns();
    if deadline_ns <= now_ns {
        return Err(StopFrameError::InvalidDeadline);
    }
    master.reap_expired_rx_before_tx(now_ns);
    let frame = master
        .acquire_frame(generation, deadline_ns)
        .map_err(StopFrameError::FramePool)?;
    let length = match master.build_and_arm_frame_from_plan(frame, plan, image) {
        Ok(length) => length,
        Err(error) => {
            // The nonzero deadline and pre-arm index check make a failed build
            // leave no new RX expectations; never cancel an older slot's RX.
            master
                .release_unarmed_frame(frame)
                .map_err(StopFrameError::FramePool)?;
            return Err(StopFrameError::Build(error));
        }
    };
    master
        .submit_frame(port, frame)
        .map_err(StopFrameError::Transmit)?;
    Ok(length)
}

#[cfg(feature = "cia402")]
fn writable_domain_field<const BYTES: usize, const SEGMENTS: usize, const DATAGRAMS: usize>(
    map: &Cia402PdoMap,
    field: Cia402PdoField,
    domain: &Domain<BYTES, SEGMENTS>,
    plan: &FramePlan<DATAGRAMS>,
) -> bool {
    let Some(entry) = map.entry(field) else {
        return false;
    };
    let Some(end_bit) = entry.bit_offset.checked_add(entry.bit_length as usize) else {
        return false;
    };
    let first_byte = entry.bit_offset / 8;
    let end_byte = end_bit.div_ceil(8);
    let Some(field_start) = u32::try_from(first_byte)
        .ok()
        .and_then(|offset| domain.logical_address().checked_add(offset))
    else {
        return false;
    };
    let Some(field_end) = u32::try_from(end_byte)
        .ok()
        .and_then(|offset| domain.logical_address().checked_add(offset))
    else {
        return false;
    };
    // A later writable datagram must not rewrite this logical PDO field from
    // unrelated process-image bytes, even if an earlier datagram covers it.
    if plan.datagrams().iter().any(|datagram| {
        matches!(datagram.command, Command::Lrw | Command::Lwr)
            && datagram
                .address
                .checked_add(datagram.payload_len as u32)
                .is_none_or(|end| {
                    datagram.address < field_end
                        && field_start < end
                        && u32::try_from(datagram.payload_offset)
                            .ok()
                            .and_then(|offset| domain.logical_address().checked_add(offset))
                            != Some(datagram.address)
                })
    }) {
        return false;
    }
    plan.datagrams().iter().any(|datagram| {
        matches!(datagram.command, Command::Lrw | Command::Lwr)
            && datagram.payload_offset <= first_byte
            && datagram
                .payload_offset
                .checked_add(datagram.payload_len)
                .is_some_and(|end| end >= end_byte && end <= BYTES)
            && u32::try_from(datagram.payload_offset)
                .ok()
                .and_then(|offset| domain.logical_address().checked_add(offset))
                == Some(datagram.address)
            && domain.segments().iter().any(|segment| {
                segment.datagram_index == datagram.index
                    && segment.input_offset == datagram.payload_offset
                    && segment.len == datagram.payload_len
                    && segment.expected_wkc != 0
                    && segment.expected_wkc == datagram.expected_wkc
            })
    })
}

/// One quality snapshot bound to a frozen schedule Domain ID, in schedule order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledDomainQuality {
    pub id: u8,
    pub quality: DomainQuality,
}

/// Facts owned by the cycle caller, not inferable from EtherCAT RX or DC.
/// Callers must sample each from its actual owner for the same cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OtherCycleFacts {
    pub platform_ready: bool,
    pub coe_ready: bool,
    pub topology_valid: bool,
    pub drive_ready: bool,
    pub command_current: bool,
    pub supervisor_healthy: bool,
    pub external_safety_clear: bool,
    pub deadline_met: bool,
}

/// Conservatively bind the bounded mailbox/DC service stage to lifecycle
/// facts. A service TX failure, a terminal mailbox error, or a scheduled retry
/// blocks configuration readiness for this cycle. The post-RX observation is
/// an intermediate budget fact and can only clear, never restore, the caller's
/// deadline qualification.
pub fn other_cycle_facts_from_mailbox_cycle<E, const DOMAINS: usize>(
    cycle: &ScheduledMailboxCycleReport<E, DOMAINS>,
    mut other: OtherCycleFacts,
) -> OtherCycleFacts {
    let service_ready = cycle.tx.service.failure.is_none();
    let mailbox_ready = !matches!(
        cycle.receive.mailbox_progress,
        Some(Ok(MailboxProgress::RetryScheduled)) | Some(Err(_))
    );
    other.coe_ready &= service_ready && mailbox_ready;
    other.deadline_met &= cycle.post_receive_deadline_met;
    other
}

/// Bind process-Domain submission evidence to the cycle budget gate. Any
/// rejected due frame means the planned cyclic body was not completed, even
/// when the port clock has not yet crossed the absolute deadline. A late
/// post-TX observation can only clear an already-qualified budget fact.
pub fn other_cycle_facts_from_process_tx<E>(
    process: &ScheduledProcessTxReport<E>,
    mut other: OtherCycleFacts,
) -> OtherCycleFacts {
    other.deadline_met &= process.failure.is_none() && process.post_tx_deadline_met;
    other
}

/// Collect after `Domain::finish_receive` for every required, scheduled
/// Domain. `due_domains` must contain *all* Domains required in this cycle;
/// omitted or unscheduled Domains cannot be inferred from a receive report.
/// DC-enabled configurations must pass their actual cyclic sync instance.
/// A no-DC configuration must explicitly waive the DC gate in its policy.
pub fn cyclic_quality_from_ethercat(
    report: CycleReport,
    due_domains: &[DomainQuality],
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    cyclic_quality_from_domains(report, due_domains.iter(), dc, other)
}

pub(crate) fn cyclic_quality_from_domains<'a>(
    report: CycleReport,
    mut due_domains: impl Iterator<Item = &'a DomainQuality>,
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    let mut has_due_domain = false;
    let domain_valid = due_domains.all(|domain| {
        has_due_domain = true;
        domain_current(report, *domain)
    }) && has_due_domain;
    quality_from_domain_health(report, domain_valid, dc, other)
}

fn domain_current(report: CycleReport, domain: DomainQuality) -> bool {
    domain.valid
        && domain.complete
        && domain.expected_wkc != 0
        && domain.actual_wkc == domain.expected_wkc
        && domain.last_valid_cycle == report.cycle
        && domain.input_age_cycles == 0
}

fn rx_current(report: CycleReport) -> bool {
    !report.link_down
        && report.received_frames != 0
        && report.parsed_datagrams != 0
        && report.unmatched_datagrams == 0
        && report.corrupt_frames == 0
        && report.wkc_mismatches == 0
        && report.timed_out_datagrams == 0
        && report.consumer_rejections == 0
}

/// Build complete stop proof from the committed Domain image, never from a
/// retained input page after a missed receive. The Domain must have finished
/// this cycle's receive and the guard must already have issued a stop in an
/// earlier cycle. Missing velocity leaves the axis unconfirmed, and any
/// malformed PDO or incomplete axis set rejects the entire proof.
#[cfg(feature = "cia402")]
pub fn verified_ethercat_stop_feedback<
    const AXES: usize,
    const BYTES: usize,
    const SEGMENTS: usize,
>(
    decision: &AxisCycleDecision<'_>,
    report: CycleReport,
    domain: &Domain<BYTES, SEGMENTS>,
    maps: &[Cia402PdoMap; AXES],
    modes: &[OperatingMode; AXES],
    max_stationary_velocities: &[u32; AXES],
) -> Option<StopFeedback> {
    if AXES > crate::MAX_MOTION_AXES
        || decision.cycle() != report.cycle
        || !decision
            .stop_issued_cycle()
            .is_some_and(|issued| report.cycle > issued)
        || report.budget_exhausted
        || !rx_current(report)
        || !domain_current(report, domain.quality())
    {
        return None;
    }

    let mask = decision.stopping_axis_mask();
    if mask == 0 || (AXES < crate::MAX_MOTION_AXES && mask >> AXES != 0) {
        return None;
    }
    let mut feedback = StopFeedback::empty(report.cycle);
    for axis in 0..AXES {
        if mask & (1u32 << axis) == 0 {
            continue;
        }
        let inputs = maps[axis]
            .read_inputs_for(domain.input(), modes[axis])
            .ok()?;
        feedback = feedback.merge(stop_feedback_from_cia402(
            report.cycle,
            axis as u8,
            inputs,
            max_stationary_velocities[axis],
        )?)?;
    }
    Some(feedback)
}

/// Evaluate every configured Domain, including those not due on this tick.
/// The schedule's tick zero is master cycle one. A non-due Domain is healthy only
/// after a successful receive at its most recent scheduled tick. No Domain
/// may be omitted or substituted: snapshots must match schedule order and ID.
pub fn cyclic_quality_from_schedule<const DOMAINS: usize, const SLOTS: usize>(
    report: CycleReport,
    schedule: &ScheduleTable<DOMAINS, SLOTS>,
    domains: &[ScheduledDomainQuality],
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    let tick = report.cycle.saturating_sub(1);
    let domain_valid = report.cycle != 0
        && !domains.is_empty()
        && domains.len() == schedule.domain_count()
        && schedule
            .domains()
            .iter()
            .zip(domains)
            .all(|(configured, observed)| {
                if configured.id != observed.id {
                    return false;
                }
                let quality = observed.quality;
                if !quality.valid
                    || !quality.complete
                    || quality.expected_wkc == 0
                    || quality.actual_wkc != quality.expected_wkc
                    || quality.input_age_cycles != 0
                    || quality.last_valid_cycle == 0
                    || quality.last_valid_cycle > report.cycle
                {
                    return false;
                }
                let age = report.cycle - quality.last_valid_cycle;
                let is_due = schedule
                    .slot((tick % u64::from(schedule.hyperperiod_ticks())) as u32)
                    .is_due(configured.id);
                if is_due {
                    return age == 0;
                }
                age < u64::from(configured.period_ticks)
                    && tick.checked_sub(age).is_some_and(|last_tick| {
                        schedule
                            .slot((last_tick % u64::from(schedule.hyperperiod_ticks())) as u32)
                            .is_due(configured.id)
                    })
            });
    quality_from_domain_health(report, domain_valid, dc, other)
}

fn quality_from_domain_health(
    report: CycleReport,
    domain_valid: bool,
    dc: &DcCyclicSync,
    other: OtherCycleFacts,
) -> CyclicQuality {
    let rx_valid = rx_current(report);

    CyclicQuality {
        platform_ready: other.platform_ready,
        coe_ready: other.coe_ready,
        topology_valid: other.topology_valid,
        distributed_clock_locked: !report.link_down
            && dc.sync_count() != 0
            && dc.last_sync_cycle() == report.cycle
            && dc.last_error().is_none()
            && dc.pending_generation().is_none()
            && dc.monitor().is_locked(),
        drive_ready: other.drive_ready,
        domain_valid,
        wkc_valid: rx_valid && domain_valid,
        command_current: other.command_current,
        supervisor_healthy: other.supervisor_healthy,
        external_safety_clear: other.external_safety_clear,
        cycle_within_budget: other.deadline_met && !report.budget_exhausted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esop_ethercat_core::wire::{Command, DatagramHeader};
    use esop_ethercat_core::{
        DcCyclicConfig, DcMonitor, RxMatch, ScheduleDomain, ScheduledProcessFrameError,
        ScheduledProcessTxFailure,
    };

    fn report() -> CycleReport {
        CycleReport {
            cycle: 7,
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
        }
    }

    fn domain() -> DomainQuality {
        DomainQuality {
            expected_wkc: 1,
            actual_wkc: 1,
            valid: true,
            complete: true,
            last_valid_cycle: 7,
            input_age_cycles: 0,
        }
    }

    fn other() -> OtherCycleFacts {
        OtherCycleFacts {
            platform_ready: true,
            coe_ready: true,
            topology_valid: true,
            drive_ready: true,
            command_current: true,
            supervisor_healthy: true,
            external_safety_clear: true,
            deadline_met: true,
        }
    }

    #[test]
    fn process_submission_failure_or_late_stage_clears_budget_fact() {
        let mut process = ScheduledProcessTxReport::<core::convert::Infallible> {
            cycle: 1,
            generation: 1,
            rx_deadline_ns: 100,
            due_mask: 1,
            expected_frames: 1,
            sent_frames: 1,
            failure: None,
            post_tx_deadline_met: true,
        };
        assert_eq!(
            other_cycle_facts_from_process_tx(&process, other()),
            other()
        );

        process.post_tx_deadline_met = false;
        assert!(!other_cycle_facts_from_process_tx(&process, other()).deadline_met);

        process.post_tx_deadline_met = true;
        process.sent_frames = 0;
        process.failure = Some(ScheduledProcessTxFailure {
            domain_id: 9,
            frame_index: 0,
            error: ScheduledProcessFrameError::InvalidDeadline,
        });
        assert!(!other_cycle_facts_from_process_tx(&process, other()).deadline_met);
    }

    fn locked_dc() -> DcCyclicSync {
        let mut sync = DcCyclicSync::new(
            DcCyclicConfig::new(0x1000, 13, 0),
            DcMonitor::new(50, 10, 1, 2),
        );
        let mut image = [0; 8];
        sync.prepare(3, 100, &mut image).unwrap();
        let plan = sync.datagram_plan();
        sync.complete(
            7,
            100,
            RxMatch {
                slot_id: 0,
                generation: 3,
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
        sync
    }

    #[test]
    fn fresh_verified_cycle_produces_complete_good_facts() {
        let facts =
            cyclic_quality_from_ethercat(report(), &[domain(), domain()], &locked_dc(), other());
        assert!(facts.domain_valid && facts.wkc_valid && facts.distributed_clock_locked);
        assert!(facts.cycle_within_budget && facts.command_current);
    }

    #[test]
    fn missing_or_stale_domain_fails_closed() {
        let dc = locked_dc();
        let empty = cyclic_quality_from_ethercat(report(), &[], &dc, other());
        assert!(!empty.domain_valid && !empty.wkc_valid);
        let stale = DomainQuality {
            last_valid_cycle: 6,
            ..domain()
        };
        let facts = cyclic_quality_from_ethercat(report(), &[domain(), stale], &dc, other());
        assert!(!facts.domain_valid && !facts.wkc_valid);
        for faulty in [
            DomainQuality {
                expected_wkc: 0,
                actual_wkc: 0,
                ..domain()
            },
            DomainQuality {
                actual_wkc: 0,
                ..domain()
            },
            DomainQuality {
                valid: false,
                ..domain()
            },
            DomainQuality {
                input_age_cycles: 1,
                ..domain()
            },
            DomainQuality {
                complete: false,
                ..domain()
            },
        ] {
            assert!(!cyclic_quality_from_ethercat(report(), &[faulty], &dc, other()).domain_valid);
        }
    }

    #[test]
    fn receive_errors_and_dc_freshness_cannot_pass() {
        let dc = locked_dc();
        let bad_reports = [
            CycleReport {
                link_down: true,
                ..report()
            },
            CycleReport {
                received_frames: 0,
                ..report()
            },
            CycleReport {
                wkc_mismatches: 1,
                ..report()
            },
            CycleReport {
                timed_out_datagrams: 1,
                ..report()
            },
            CycleReport {
                corrupt_frames: 1,
                ..report()
            },
            CycleReport {
                unmatched_datagrams: 1,
                ..report()
            },
            CycleReport {
                consumer_rejections: 1,
                ..report()
            },
        ];
        for bad in bad_reports {
            assert!(!cyclic_quality_from_ethercat(bad, &[domain()], &dc, other()).wkc_valid);
            if bad.link_down {
                assert!(
                    !cyclic_quality_from_ethercat(bad, &[domain()], &dc, other())
                        .distributed_clock_locked
                );
            }
        }
        let next = CycleReport {
            cycle: 8,
            ..report()
        };
        assert!(
            !cyclic_quality_from_ethercat(next, &[domain()], &dc, other()).distributed_clock_locked
        );
        let mut image = [0; 8];
        let mut pending = locked_dc();
        pending.prepare(4, 101, &mut image).unwrap();
        assert!(
            !cyclic_quality_from_ethercat(report(), &[domain()], &pending, other())
                .distributed_clock_locked
        );
        let plan = pending.datagram_plan();
        assert!(
            pending
                .complete(
                    7,
                    100,
                    RxMatch {
                        slot_id: 0,
                        generation: 4,
                        working_counter: 0,
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
                .is_err()
        );
        // The monitor still remembers the previous lock; last_error must win.
        assert!(pending.monitor().is_locked());
        assert!(
            !cyclic_quality_from_ethercat(report(), &[domain()], &pending, other())
                .distributed_clock_locked
        );
    }

    #[test]
    fn deadline_and_budget_are_independent_of_rx_health() {
        let dc = locked_dc();
        for (report, other) in [
            (
                CycleReport {
                    budget_exhausted: true,
                    ..report()
                },
                other(),
            ),
            (
                report(),
                OtherCycleFacts {
                    deadline_met: false,
                    ..other()
                },
            ),
        ] {
            let facts = cyclic_quality_from_ethercat(report, &[domain()], &dc, other);
            assert!(!facts.cycle_within_budget);
            assert!(facts.wkc_valid);
        }
    }

    #[test]
    fn multi_rate_schedule_uses_last_due_sample_without_masking_missed_or_wrong_phase() {
        let schedule = ScheduleTable::<2, 8>::build(
            250_000,
            &[
                ScheduleDomain {
                    id: 3,
                    period_ticks: 4,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 17,
                    period_ticks: 4,
                    phase_ticks: 2,
                },
            ],
        )
        .unwrap();
        let dc = locked_dc();
        let samples = [
            ScheduledDomainQuality {
                id: 3,
                quality: DomainQuality {
                    last_valid_cycle: 5,
                    ..domain()
                },
            },
            ScheduledDomainQuality {
                id: 17,
                quality: domain(),
            },
        ];
        let good = cyclic_quality_from_schedule(report(), &schedule, &samples, &dc, other());
        assert!(good.domain_valid && good.wkc_valid);

        let first = CycleReport {
            cycle: 1,
            ..report()
        };
        let unobserved = [
            ScheduledDomainQuality {
                id: 3,
                quality: DomainQuality {
                    last_valid_cycle: 1,
                    ..domain()
                },
            },
            ScheduledDomainQuality {
                id: 17,
                quality: DomainQuality::EMPTY,
            },
        ];
        assert!(
            !cyclic_quality_from_schedule(first, &schedule, &unobserved, &dc, other()).domain_valid
        );

        let idle = CycleReport {
            cycle: 8,
            parsed_datagrams: 1,
            ..report()
        };
        let idle_quality = cyclic_quality_from_schedule(idle, &schedule, &samples, &dc, other());
        assert!(idle_quality.domain_valid && idle_quality.wkc_valid);
        assert!(!idle_quality.distributed_clock_locked);
        let link_down = CycleReport {
            link_down: true,
            ..idle
        };
        let failed_link =
            cyclic_quality_from_schedule(link_down, &schedule, &samples, &dc, other());
        assert!(!failed_link.wkc_valid && !failed_link.distributed_clock_locked);

        let due = CycleReport {
            cycle: 9,
            ..report()
        };
        assert!(!cyclic_quality_from_schedule(due, &schedule, &samples, &dc, other()).domain_valid);
        assert!(
            !cyclic_quality_from_schedule(report(), &schedule, &samples[..1], &dc, other())
                .domain_valid
        );
        let mut swapped = samples;
        swapped.swap(0, 1);
        assert!(
            !cyclic_quality_from_schedule(report(), &schedule, &swapped, &dc, other()).domain_valid
        );
        for invalid in [
            DomainQuality {
                last_valid_cycle: 4,
                ..domain()
            },
            DomainQuality {
                last_valid_cycle: 8,
                ..domain()
            },
            DomainQuality {
                last_valid_cycle: 6,
                ..domain()
            },
            DomainQuality {
                valid: false,
                ..domain()
            },
            DomainQuality {
                input_age_cycles: 1,
                ..domain()
            },
        ] {
            let mut faulty = samples;
            faulty[1].quality = invalid;
            assert!(
                !cyclic_quality_from_schedule(report(), &schedule, &faulty, &dc, other())
                    .domain_valid
            );
        }
        let rx_failure = CycleReport {
            wkc_mismatches: 1,
            ..idle
        };
        assert!(
            !cyclic_quality_from_schedule(rx_failure, &schedule, &samples, &dc, other()).wkc_valid
        );
    }
}
