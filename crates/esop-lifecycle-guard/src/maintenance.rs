//! Explicit bounded CiA 402 maintenance Fault Reset authority.
//!
//! This authority can only emit 0x0080 followed by Disable. It never changes a
//! lifecycle fault, qualifies a gate, grants a permit or writes a setpoint.
use crate::{LifecycleGuard, LifecycleState, MAX_MOTION_AXES};
use esop_ethercat_core::{CycleReport, Domain, FramePlan};
use esop_profile_cia402::{
    CONTROLWORD_DISABLE_VOLTAGE, CONTROLWORD_FAULT_RESET, Cia402Output, Cia402PdoField,
    Cia402PdoMap, DriveState, OperatingMode,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultResetRequest {
    pub boot_id: u64,
    pub sequence: u64,
    pub transition_sequence: u64,
    pub axis_mask: u32,
    pub deadline_ns: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultResetPhase {
    Idle,
    Pending,
    ZeroRequired,
    Complete,
    Failed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultResetError {
    InvalidPolicy,
    BootMismatch,
    SequenceReplayed,
    TransitionMismatch,
    InvalidAxisMask,
    InvalidLifecycle,
    PermitPresent,
    StopPending,
    Deadline,
    Busy,
    ZeroRequired,
    NoRequest,
    CycleReplayed,
    UnverifiedInput,
    NotInFault(usize),
    InvalidMapping(usize),
    OverlappingOutputs,
    AttemptsExhausted,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultResetStatus {
    pub phase: FaultResetPhase,
    pub request: Option<FaultResetRequest>,
    pub attempts: u8,
    pub pulse_accepted_cycle: Option<u64>,
    pub zero_accepted_cycle: Option<u64>,
    pub last_error: Option<FaultResetError>,
}

pub struct FaultResetController {
    boot_id: u64,
    max_pulse_attempts: u8,
    last_sequence: u64,
    request: Option<FaultResetRequest>,
    phase: FaultResetPhase,
    attempts: u8,
    last_attempt_cycle: Option<u64>,
    pulse_accepted_cycle: Option<u64>,
    zero_accepted_cycle: Option<u64>,
    last_error: Option<FaultResetError>,
}
#[derive(Clone, Copy)]
pub(crate) struct FaultResetPreview<const AXES: usize> {
    pub(crate) outputs: [Cia402Output; AXES],
    pub(crate) pulse_mask: u32,
    cycle: u64,
    sequence: u64,
    pub(crate) deadline_ns: u64,
}
impl FaultResetController {
    pub fn new(boot_id: u64, max_pulse_attempts: u8) -> Result<Self, FaultResetError> {
        if max_pulse_attempts == 0 || max_pulse_attempts > 8 {
            return Err(FaultResetError::InvalidPolicy);
        }
        Ok(Self {
            boot_id,
            max_pulse_attempts,
            last_sequence: 0,
            request: None,
            phase: FaultResetPhase::Idle,
            attempts: 0,
            last_attempt_cycle: None,
            pulse_accepted_cycle: None,
            zero_accepted_cycle: None,
            last_error: None,
        })
    }
    pub const fn status(&self) -> FaultResetStatus {
        FaultResetStatus {
            phase: self.phase,
            request: self.request,
            attempts: self.attempts,
            pulse_accepted_cycle: self.pulse_accepted_cycle,
            zero_accepted_cycle: self.zero_accepted_cycle,
            last_error: self.last_error,
        }
    }
    pub fn submit(
        &mut self,
        guard: &LifecycleGuard,
        request: FaultResetRequest,
        now_ns: u64,
    ) -> Result<(), FaultResetError> {
        self.expire_pending(now_ns);
        self.validate_guard(guard)?;
        if request.boot_id != self.boot_id {
            return Err(FaultResetError::BootMismatch);
        }
        if request.sequence == 0 || request.sequence <= self.last_sequence {
            return Err(FaultResetError::SequenceReplayed);
        }
        if self.phase == FaultResetPhase::ZeroRequired {
            return Err(FaultResetError::ZeroRequired);
        }
        if self.phase == FaultResetPhase::Pending {
            return Err(FaultResetError::Busy);
        }
        if request.transition_sequence != guard.transition_sequence {
            return Err(FaultResetError::TransitionMismatch);
        }
        if request.axis_mask == 0 || request.axis_mask & !guard.policy.allowed_axis_mask != 0 {
            return Err(FaultResetError::InvalidAxisMask);
        }
        if request.deadline_ns <= now_ns {
            return Err(FaultResetError::Deadline);
        }
        self.last_sequence = request.sequence;
        self.request = Some(request);
        self.phase = FaultResetPhase::Pending;
        self.attempts = 0;
        self.last_attempt_cycle = None;
        self.pulse_accepted_cycle = None;
        self.zero_accepted_cycle = None;
        self.last_error = None;
        Ok(())
    }
    fn expire_pending(&mut self, now_ns: u64) {
        if self.phase == FaultResetPhase::Pending
            && self
                .request
                .is_some_and(|request| now_ns >= request.deadline_ns)
        {
            self.phase = FaultResetPhase::Failed;
            self.last_error = Some(FaultResetError::Deadline);
        }
    }
    fn validate_guard(&self, guard: &LifecycleGuard) -> Result<(), FaultResetError> {
        if guard.boot_id != self.boot_id {
            return Err(FaultResetError::BootMismatch);
        }
        if !matches!(
            guard.state,
            LifecycleState::Maintenance | LifecycleState::FaultLatched
        ) {
            return Err(FaultResetError::InvalidLifecycle);
        }
        if guard.permit.is_some() {
            return Err(FaultResetError::PermitPresent);
        }
        if guard.motion_axes_mask != 0 || guard.maintenance_stop_pending {
            return Err(FaultResetError::StopPending);
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare<
        const AXES: usize,
        const BYTES: usize,
        const SEGMENTS: usize,
        const DATAGRAMS: usize,
    >(
        &mut self,
        guard: &LifecycleGuard,
        report: CycleReport,
        domain: &Domain<BYTES, SEGMENTS>,
        maps: &[Cia402PdoMap; AXES],
        modes: &[OperatingMode; AXES],
        safe: &[u8; BYTES],
        plan: &FramePlan<DATAGRAMS>,
        now_ns: u64,
    ) -> Result<FaultResetPreview<AXES>, FaultResetError> {
        self.expire_pending(now_ns);
        self.validate_guard(guard)?;
        let request = self.request.ok_or(FaultResetError::NoRequest)?;
        if AXES > MAX_MOTION_AXES || (AXES < MAX_MOTION_AXES && request.axis_mask >> AXES != 0) {
            return Err(FaultResetError::InvalidAxisMask);
        }
        let pulse = match self.phase {
            FaultResetPhase::Pending => {
                if request.transition_sequence != guard.transition_sequence {
                    return Err(FaultResetError::TransitionMismatch);
                }
                if now_ns >= request.deadline_ns {
                    self.phase = FaultResetPhase::Failed;
                    self.last_error = Some(FaultResetError::Deadline);
                    return Err(FaultResetError::Deadline);
                }
                if self.attempts >= self.max_pulse_attempts {
                    return Err(FaultResetError::AttemptsExhausted);
                }
                request.axis_mask
            }
            FaultResetPhase::ZeroRequired => 0,
            FaultResetPhase::Failed => {
                return Err(self.last_error.unwrap_or(FaultResetError::NoRequest));
            }
            _ => return Err(FaultResetError::NoRequest),
        };
        if self
            .last_attempt_cycle
            .is_some_and(|cycle| report.cycle <= cycle)
        {
            return Err(FaultResetError::CycleReplayed);
        }
        if report.cycle == 0
            || report.budget_exhausted
            || !crate::ethercat::rx_current(report)
            || !crate::ethercat::domain_current(report, domain.quality())
        {
            return Err(FaultResetError::UnverifiedInput);
        }
        let mut outputs = [Cia402Output {
            state: DriveState::Unknown,
            statusword: 0,
            controlword: 0,
            operation_enabled: false,
            motion_allowed: false,
            fault_reset_pulse: false,
        }; AXES];
        let mut image = *safe;
        for axis in 0..AXES {
            let inputs = maps[axis]
                .read_inputs_for(domain.input(), modes[axis])
                .map_err(|_| FaultResetError::InvalidMapping(axis))?;
            let selected = pulse & (1u32 << axis) != 0;
            if selected && DriveState::from_statusword(inputs.statusword) != DriveState::Fault {
                return Err(FaultResetError::NotInFault(axis));
            }
            for field in [Cia402PdoField::Controlword, Cia402PdoField::ModeOfOperation] {
                if !crate::ethercat::writable_domain_field(&maps[axis], field, domain, plan) {
                    return Err(FaultResetError::InvalidMapping(axis));
                }
            }
            outputs[axis] = Cia402Output {
                state: DriveState::from_statusword(inputs.statusword),
                statusword: inputs.statusword,
                controlword: if selected {
                    CONTROLWORD_FAULT_RESET
                } else {
                    CONTROLWORD_DISABLE_VOLTAGE
                },
                operation_enabled: false,
                motion_allowed: false,
                fault_reset_pulse: selected,
            };
            maps[axis]
                .write_control(&mut image, modes[axis], outputs[axis].controlword)
                .map_err(|_| FaultResetError::InvalidMapping(axis))?;
        }
        const FIELDS: [Cia402PdoField; 5] = [
            Cia402PdoField::Controlword,
            Cia402PdoField::ModeOfOperation,
            Cia402PdoField::TargetPosition,
            Cia402PdoField::TargetVelocity,
            Cia402PdoField::TargetTorque,
        ];
        for (axis, map) in maps.iter().enumerate() {
            for field in [Cia402PdoField::Controlword, Cia402PdoField::ModeOfOperation] {
                let entry = map
                    .entry(field)
                    .ok_or(FaultResetError::InvalidMapping(axis))?;
                for (other_axis, other) in maps.iter().enumerate() {
                    for other_field in FIELDS {
                        if axis == other_axis && field == other_field {
                            continue;
                        }
                        if other.entry(other_field).is_some_and(|other_entry| {
                            entry.bit_offset
                                < other_entry
                                    .bit_offset
                                    .saturating_add(other_entry.bit_length as usize)
                                && other_entry.bit_offset
                                    < entry.bit_offset.saturating_add(entry.bit_length as usize)
                        }) {
                            return Err(FaultResetError::OverlappingOutputs);
                        }
                    }
                }
            }
        }
        Ok(FaultResetPreview {
            outputs,
            pulse_mask: pulse,
            cycle: report.cycle,
            sequence: request.sequence,
            deadline_ns: if pulse != 0 {
                request.deadline_ns
            } else {
                u64::MAX
            },
        })
    }
    pub(crate) fn attempted<const AXES: usize>(
        &mut self,
        preview: FaultResetPreview<AXES>,
        accepted: bool,
    ) {
        if self
            .request
            .is_none_or(|request| request.sequence != preview.sequence)
        {
            return;
        }
        self.last_attempt_cycle = Some(preview.cycle);
        if preview.pulse_mask != 0 {
            self.attempts = self.attempts.saturating_add(1);
            if accepted {
                self.phase = FaultResetPhase::ZeroRequired;
                self.pulse_accepted_cycle = Some(preview.cycle);
                self.last_error = None;
            } else if self.attempts >= self.max_pulse_attempts {
                self.phase = FaultResetPhase::Failed;
                self.last_error = Some(FaultResetError::AttemptsExhausted);
            }
        } else if accepted {
            self.phase = FaultResetPhase::Complete;
            self.zero_accepted_cycle = Some(preview.cycle);
            self.last_error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GuardPolicy;
    use esop_ethercat_core::wire::{Command, DatagramHeader};
    use esop_ethercat_core::{
        DatagramPlan, DomainSegment, PdoDirection, PdoEntry, RxDatagramConsumer, RxMatch,
    };

    fn faulted_guard() -> LifecycleGuard {
        let mut guard = LifecycleGuard::new(
            0,
            7,
            GuardPolicy {
                allowed_axis_mask: 1,
                ..GuardPolicy::conservative()
            },
        );
        guard.latch_fault(123, 0);
        guard
    }
    fn request(guard: &LifecycleGuard) -> FaultResetRequest {
        FaultResetRequest {
            boot_id: 7,
            sequence: 1,
            transition_sequence: guard.transition_sequence,
            axis_mask: 1,
            deadline_ns: 1000,
        }
    }
    fn sample(
        cycle: u64,
        status: u16,
    ) -> (Domain<32, 1>, [Cia402PdoMap; 1], FramePlan<1>, CycleReport) {
        let mut map = Cia402PdoMap::new();
        for (field, offset, bits, signed, direction) in [
            (Cia402PdoField::Statusword, 0, 16, false, PdoDirection::Tx),
            (Cia402PdoField::ModeDisplay, 16, 8, true, PdoDirection::Tx),
            (Cia402PdoField::ErrorCode, 24, 16, false, PdoDirection::Tx),
            (
                Cia402PdoField::ActualPosition,
                40,
                32,
                true,
                PdoDirection::Tx,
            ),
            (
                Cia402PdoField::Controlword,
                128,
                16,
                false,
                PdoDirection::Rx,
            ),
            (
                Cia402PdoField::ModeOfOperation,
                144,
                8,
                true,
                PdoDirection::Rx,
            ),
            (
                Cia402PdoField::TargetPosition,
                152,
                32,
                true,
                PdoDirection::Rx,
            ),
        ] {
            map.set_entry(
                field,
                PdoEntry {
                    index: field.object_index(),
                    subindex: 0,
                    bit_offset: offset,
                    bit_length: bits,
                    signed,
                    direction,
                },
            );
        }
        let mut domain = Domain::new(0x1000);
        domain
            .add_segment(DomainSegment {
                datagram_index: 1,
                input_offset: 0,
                len: 32,
                expected_wkc: 1,
            })
            .unwrap();
        domain.begin_receive(cycle as u16).unwrap();
        let mut image = [0; 32];
        image[..2].copy_from_slice(&status.to_le_bytes());
        image[2] = 8;
        assert!(domain.accept(
            cycle,
            cycle,
            RxMatch {
                slot_id: 0,
                generation: cycle as u16,
                working_counter: 1
            },
            DatagramHeader::new(Command::Lrw, 1, 0x1000, 32),
            &image
        ));
        assert!(domain.finish_receive(cycle as u16, cycle).unwrap());
        let mut plan = FramePlan::new();
        plan.push(DatagramPlan {
            command: Command::Lrw,
            index: 1,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 32,
            expected_wkc: 1,
        })
        .unwrap();
        let report = CycleReport {
            cycle,
            received_frames: 1,
            received_bytes: 64,
            parsed_datagrams: 1,
            unmatched_datagrams: 0,
            corrupt_frames: 0,
            wkc_mismatches: 0,
            timed_out_datagrams: 0,
            consumer_rejections: 0,
            budget_exhausted: false,
            link_down: false,
        };
        (domain, [map], plan, report)
    }
    #[test]
    fn requests_are_explicit_bound_and_never_relax_lifecycle() {
        let guard = faulted_guard();
        for (bad, error) in [
            (
                FaultResetRequest {
                    boot_id: 8,
                    ..request(&guard)
                },
                FaultResetError::BootMismatch,
            ),
            (
                FaultResetRequest {
                    sequence: 0,
                    ..request(&guard)
                },
                FaultResetError::SequenceReplayed,
            ),
            (
                FaultResetRequest {
                    transition_sequence: 0,
                    ..request(&guard)
                },
                FaultResetError::TransitionMismatch,
            ),
            (
                FaultResetRequest {
                    axis_mask: 2,
                    ..request(&guard)
                },
                FaultResetError::InvalidAxisMask,
            ),
            (
                FaultResetRequest {
                    deadline_ns: 0,
                    ..request(&guard)
                },
                FaultResetError::Deadline,
            ),
        ] {
            let mut reset = FaultResetController::new(7, 3).unwrap();
            assert_eq!(reset.submit(&guard, bad, 0), Err(error));
            assert_eq!(reset.status().phase, FaultResetPhase::Idle);
        }
        let mut active = faulted_guard();
        active.state = LifecycleState::Active;
        let mut reset = FaultResetController::new(7, 3).unwrap();
        assert_eq!(
            reset.submit(&active, request(&active), 0),
            Err(FaultResetError::InvalidLifecycle)
        );
        assert_eq!(guard.state(), LifecycleState::FaultLatched);
    }
    #[test]
    fn preview_is_transactional_one_pulse_then_an_accepted_zero_is_mandatory() {
        let guard = faulted_guard();
        let mut reset = FaultResetController::new(7, 3).unwrap();
        reset.submit(&guard, request(&guard), 0).unwrap();
        let (domain, maps, plan, report) = sample(1, 8);
        let preview = reset
            .prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                1,
            )
            .unwrap();
        assert_eq!(preview.outputs[0].controlword, 0x80);
        assert!(!preview.outputs[0].motion_allowed);
        assert_eq!(reset.status().pulse_accepted_cycle, None);
        reset.attempted(preview, true);
        assert_eq!(reset.status().phase, FaultResetPhase::ZeroRequired);
        assert_eq!(
            reset.submit(
                &guard,
                FaultResetRequest {
                    sequence: 2,
                    ..request(&guard)
                },
                2
            ),
            Err(FaultResetError::ZeroRequired)
        );
        let (domain, maps, plan, report) = sample(2, 0x40);
        let zero = reset
            .prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                2,
            )
            .unwrap();
        assert_eq!(zero.outputs[0].controlword, 0);
        assert!(!zero.outputs[0].fault_reset_pulse);
        reset.attempted(zero, false);
        assert_eq!(reset.status().phase, FaultResetPhase::ZeroRequired);
        let (domain, maps, plan, report) = sample(3, 0x40);
        let zero = reset
            .prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                3,
            )
            .unwrap();
        reset.attempted(zero, true);
        assert_eq!(reset.status().phase, FaultResetPhase::Complete);
        assert_eq!(reset.status().pulse_accepted_cycle, Some(1));
        assert_eq!(reset.status().zero_accepted_cycle, Some(3));
        assert_eq!(guard.state(), LifecycleState::FaultLatched);
        assert!(guard.permit.is_none());
        assert_eq!(
            reset.submit(&guard, request(&guard), 4),
            Err(FaultResetError::SequenceReplayed)
        );
    }
    #[test]
    fn invalid_feedback_mapping_and_expiry_prevent_authority() {
        let guard = faulted_guard();
        let mut reset = FaultResetController::new(7, 3).unwrap();
        reset.submit(&guard, request(&guard), 0).unwrap();
        let (domain, maps, plan, report) = sample(1, 0x40);
        assert!(matches!(
            reset.prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                1
            ),
            Err(FaultResetError::NotInFault(0))
        ));
        let (domain, maps, plan, report) = sample(1, 8);
        assert!(matches!(
            reset.prepare(
                &guard,
                CycleReport { cycle: 2, ..report },
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                1
            ),
            Err(FaultResetError::UnverifiedInput)
        ));
        let mut wrong = maps;
        let mut control = wrong[0].entry(Cia402PdoField::Controlword).unwrap();
        control.bit_offset = 400;
        wrong[0].set_entry(Cia402PdoField::Controlword, control);
        assert!(matches!(
            reset.prepare(
                &guard,
                report,
                &domain,
                &wrong,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                1
            ),
            Err(FaultResetError::InvalidMapping(0))
        ));
        assert!(matches!(
            reset.prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                1000
            ),
            Err(FaultResetError::Deadline)
        ));
        assert_eq!(reset.status().pulse_accepted_cycle, None);
    }
    #[test]
    fn failed_pulse_transmits_are_bounded_and_require_a_new_request() {
        let guard = faulted_guard();
        let mut reset = FaultResetController::new(7, 2).unwrap();
        reset.submit(&guard, request(&guard), 0).unwrap();
        for cycle in 1..=2 {
            let (domain, maps, plan, report) = sample(cycle, 8);
            let preview = reset
                .prepare(
                    &guard,
                    report,
                    &domain,
                    &maps,
                    &[OperatingMode::Csp],
                    &[0; 32],
                    &plan,
                    cycle,
                )
                .unwrap();
            reset.attempted(preview, false);
            assert_eq!(reset.status().pulse_accepted_cycle, None);
        }
        assert_eq!(reset.status().phase, FaultResetPhase::Failed);
        let (domain, maps, plan, report) = sample(3, 8);
        assert!(matches!(
            reset.prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                3
            ),
            Err(FaultResetError::AttemptsExhausted)
        ));
        reset
            .submit(
                &guard,
                FaultResetRequest {
                    sequence: 2,
                    ..request(&guard)
                },
                3,
            )
            .unwrap();
        assert_eq!(reset.status().phase, FaultResetPhase::Pending);
    }
    #[test]
    fn expired_pending_request_cannot_block_a_new_explicit_sequence() {
        let guard = faulted_guard();
        let mut reset = FaultResetController::new(7, 3).unwrap();
        reset.submit(&guard, request(&guard), 0).unwrap();
        let next = FaultResetRequest {
            sequence: 2,
            deadline_ns: 2000,
            ..request(&guard)
        };
        reset.submit(&guard, next, 1000).unwrap();
        assert_eq!(reset.status().request, Some(next));
        let (domain, maps, plan, report) = sample(1, 8);
        let preview = reset
            .prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                1001,
            )
            .unwrap();
        reset.attempted(preview, false);
        assert!(matches!(
            reset.prepare(
                &guard,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[0; 32],
                &plan,
                1002
            ),
            Err(FaultResetError::CycleReplayed)
        ));
    }
}
