#![cfg(all(feature = "ethercat", feature = "cia402", feature = "procbuf"))]
use esop_ethercat_core::wire::{Command, DatagramHeader, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::*;
use esop_lifecycle_guard::ethercat::OtherCycleFacts;
use esop_lifecycle_guard::maintenance::{
    FaultResetController, FaultResetError, FaultResetPhase, FaultResetRequest,
};
use esop_lifecycle_guard::procbuf::{
    Cia402AxisCommandPolicy, Cia402FeedbackError, LifecycleEventCursor,
};
use esop_lifecycle_guard::stop_cycle::{
    ScheduledAuxiliaryOutputs, ScheduledProductionCycleOwner, StopCycleContext, StopCycleError,
};
use esop_lifecycle_guard::{GateId, GuardPolicy, LifecycleError, LifecycleGuard, LifecycleState};
use esop_procbuf::{ProcBuf, StatePage};
use esop_profile_cia402::{
    Cia402AxisBank, Cia402PdoField, Cia402PdoMap, CyclicLimits, CyclicSetpointGuard, OperatingMode,
};

struct Port {
    now: u64,
    frames: [[u8; MAX_ETHERNET_FRAME_LEN]; 3],
    lengths: [usize; 3],
    head: usize,
    count: usize,
    fault: bool,
    pulses: usize,
    tx: usize,
    fail_pulse: bool,
    controls: Vec<u16>,
    raw_position: i32,
}
impl EthercatPort for Port {
    type Error = PortError;
    fn now_ns(&self) -> u64 {
        self.now
    }
    fn link_state(&self) -> LinkState {
        LinkState::Up
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), PortError> {
        self.tx += 1;
        let header = DatagramHeader::decode(&frame[16..26]).unwrap();
        if header.command == Command::Lrw {
            let control = u16::from_le_bytes([frame[42], frame[43]]);
            assert!(
                control == 0 || control == 0x80,
                "maintenance can never enable operation"
            );
            if control == 0x80 && self.fail_pulse {
                self.fail_pulse = false;
                return Err(PortError::HardwareFault);
            }
            self.controls.push(control);
            if control == 0x80 {
                self.pulses += 1;
                self.fault = false;
            }
        }
        let slot = self.count;
        self.count += 1;
        self.frames[slot][..frame.len()].copy_from_slice(frame);
        self.lengths[slot] = frame.len();
        if header.command == Command::Lrw {
            self.frames[slot][26..28]
                .copy_from_slice(&(if self.fault { 8u16 } else { 0x40u16 }).to_le_bytes());
            self.frames[slot][28] = 8;
            self.frames[slot][29..31]
                .copy_from_slice(&(if self.fault { 0x2310u16 } else { 0 }).to_le_bytes());
            self.frames[slot][31..35].copy_from_slice(&self.raw_position.to_le_bytes());
        }
        let end = 26 + header.length as usize;
        self.frames[slot][end..end + 2].copy_from_slice(&1u16.to_le_bytes());
        Ok(())
    }
    fn rx_poll(&mut self, scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN]) -> Result<RxPoll, PortError> {
        if self.head == self.count {
            self.head = 0;
            self.count = 0;
            return Ok(RxPoll::Empty);
        }
        let len = self.lengths[self.head];
        scratch[..len].copy_from_slice(&self.frames[self.head][..len]);
        self.head += 1;
        Ok(RxPoll::Frame(len))
    }
}
fn map() -> Cia402PdoMap {
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
    map
}
const POLICY: Cia402AxisCommandPolicy = Cia402AxisCommandPolicy {
    position_units_per_radian: 1.,
    velocity_units_per_radian_per_second: 1.,
    torque_units_per_newton_metre: 1.,
    position_offset: 0,
    min_position_radians: -100.,
    max_position_radians: 100.,
    max_velocity_radians_per_second: 100.,
    max_torque_newton_metres: 100.,
    max_position_step_radians: 1.,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum InputCase {
    Complete,
    MissingTarget,
    NonFiniteFeedback,
}

fn run_flow(fail_pulse: bool, input_case: InputCase) {
    let schedule = ScheduleTable::<1, 1>::build(
        1_000_000,
        &[ScheduleDomain {
            id: 0,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut domain = Domain::<32, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 1,
            input_offset: 0,
            len: 32,
            expected_wkc: 1,
        })
        .unwrap();
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 1,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 32,
        expected_wkc: 1,
    })
    .unwrap();
    let mut plans = FramePlanSet::<1, 1>::new();
    plans.push(plan.datagrams()[0]).unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 0,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let outputs =
        ScheduledAuxiliaryOutputs::<1, 1, 1, 1>::new(&bank, &schedule, 0, &plan, [None]).unwrap();
    let mut owner = ScheduledProductionCycleOwner::new(&outputs);
    let wrong_owner = ScheduledProductionCycleOwner::new(&outputs);
    let safe = [0; 32];
    let inputs = ScheduledProcessInputs::new(
        &bank,
        &schedule,
        [ScheduledProcessInputEntry {
            id: 0,
            image: &safe,
            plans: &plans,
        }],
    )
    .unwrap();
    let mut master =
        EthercatMaster::<3, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new([0xff; 6], [0; 6]));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x1000, 240, 0),
        DcMonitor::new(50, 10, 1, 1),
    );
    let mut controls = ControlRequestPool::<1>::new();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut port = Port {
        now: 0,
        frames: [[0; MAX_ETHERNET_FRAME_LEN]; 3],
        lengths: [0; 3],
        head: 0,
        count: 0,
        fault: true,
        pulses: 0,
        tx: 0,
        fail_pulse,
        controls: Vec::new(),
        raw_position: if input_case == InputCase::NonFiniteFeedback {
            1
        } else {
            0
        },
    };
    let prime = bank
        .submit_due_process_inputs(&inputs, &mut master, &mut port, 1, 500_000, 800_000)
        .unwrap();
    owner
        .arm_priming(&bank, &inputs, &prime, 1, 500_000)
        .unwrap();
    let mut guard = LifecycleGuard::new(
        GateId::Drive.bit(),
        7,
        GuardPolicy {
            allowed_axis_mask: 1,
            enter_good_cycles: 2,
            ..GuardPolicy::conservative()
        },
    );
    guard.latch_fault(123, 0);
    let mut reset = FaultResetController::new(7, 3).unwrap();
    reset
        .submit(
            &guard,
            FaultResetRequest {
                boot_id: 7,
                sequence: 1,
                transition_sequence: guard.snapshot(0, 0).transition_sequence,
                axis_mask: 1,
                deadline_ns: 5_000_000,
            },
            0,
        )
        .unwrap();
    let buffer = ProcBuf::<1, 0, 1, 16>::new(1, 7);
    let mut state = StatePage::new(7);
    let mut cursor = LifecycleEventCursor::new(&guard);
    let mut axis_bank = Cia402AxisBank::<1>::new();
    let mut maps = [map()];
    if input_case == InputCase::MissingTarget {
        maps[0].clear_entry(Cia402PdoField::TargetPosition);
    }
    let mut policies = [POLICY];
    if input_case == InputCase::NonFiniteFeedback {
        policies[0].position_units_per_radian = 1e-320;
        policies[0].validate_for_product().unwrap();
    }
    let mut target_guards = [CyclicSetpointGuard::new()];
    let limits = [CyclicLimits {
        max_position_step: 1.,
        max_velocity: 1.,
        max_torque: 1.,
    }];
    for tick in 0..if fail_pulse { 1 } else { 3 } {
        let now = tick * 1_000_000;
        port.now = now;
        let generation = tick as u16 + 1;
        let service = scheduler
            .run_cycle(
                &mut bank,
                &mut master,
                &mut port,
                &mut [0; MAX_ETHERNET_FRAME_LEN],
                &mut dc,
                &mut [0; 8],
                now,
                &mut controls,
                &mut ScheduledProductionServices::<0, 0, 0>::new(None, None, None, None),
                generation,
                now + 500_000,
                now + 800_000,
            )
            .unwrap();
        owner.complete_service_cycle(&bank, &service).unwrap();
        state.sequence = service.received().report.cycle;
        let tx_before = port.tx;
        let other = OtherCycleFacts {
            platform_ready: true,
            coe_ready: true,
            topology_valid: true,
            drive_ready: !port.fault,
            command_current: true,
            supervisor_healthy: true,
            external_safety_clear: true,
            deadline_met: true,
        };
        let mut context = StopCycleContext {
            guard: &mut guard,
            bank: &mut axis_bank,
            master: &mut master,
            port: &mut port,
            domain: bank.domain::<32, 1>(0).unwrap(),
            dc: &dc,
            buffer: &buffer,
            event_cursor: &mut cursor,
            state: &mut state,
            report: service.received().report,
            other,
            maps: &maps,
            modes: &[OperatingMode::Csp],
            axis_policies: &policies,
            max_stationary_velocities: &[0],
            safe_process_image: &safe,
            shared_process_image: None,
            plan: &plan,
            next_generation: generation + 1,
            deadline_ns: now + 1_500_000,
            now_ns: now,
            transition_time_ns: 0,
        };
        if tick == 0 {
            assert!(matches!(
                context.run_service_cycle_with_fault_reset_until(
                    &wrong_owner,
                    &bank,
                    &service,
                    0,
                    &outputs,
                    &mut reset,
                    now + 800_000
                ),
                Err(StopCycleError::ServiceCycleMismatch)
            ));
            assert_eq!(context.port.tx, tx_before);
        }
        if input_case != InputCase::Complete {
            let reset_before = reset.status();
            let error = match context.run_service_cycle_with_fault_reset_until(
                &owner,
                &bank,
                &service,
                0,
                &outputs,
                &mut reset,
                now + 800_000,
            ) {
                Err(error) => error,
                Ok(_) => panic!("invalid feedback accepted a maintenance output"),
            };
            if input_case == InputCase::MissingTarget {
                assert!(matches!(
                    error,
                    StopCycleError::Feedback(Cia402FeedbackError::Pdo(0, _))
                ));
            } else {
                assert!(matches!(
                    error,
                    StopCycleError::Feedback(Cia402FeedbackError::NonFiniteValue(
                        0,
                        Cia402PdoField::ActualPosition
                    ))
                ));
            }
            assert_eq!(context.port.tx, tx_before);
            assert_eq!(reset.status(), reset_before);
            assert_eq!(context.state.axes[0].controlword, 0);
            return;
        }
        let outcome = if tick < 2 {
            context
                .run_service_cycle_with_fault_reset_until(
                    &owner,
                    &bank,
                    &service,
                    0,
                    &outputs,
                    &mut reset,
                    now + 800_000,
                )
                .unwrap()
        } else {
            context
                .run_service_cycle_with_outputs_until(
                    &bank,
                    &service,
                    0,
                    &outputs,
                    &[None],
                    &mut target_guards,
                    &limits,
                    now + 800_000,
                )
                .unwrap()
        };
        if fail_pulse {
            assert!(outcome.transmission.is_err());
            assert!(outcome.state_publish.is_ok());
            assert_eq!(reset.status().pulse_accepted_cycle, None);
            assert_eq!(port.pulses, 0);
            assert_eq!(state.axes[0].controlword, 0);
        } else {
            assert!(outcome.transmission.is_ok());
            assert!(outcome.state_publish.is_ok());
            assert_eq!(state.axes[0].controlword, if tick == 0 { 0x80 } else { 0 });
            assert_eq!(state.axis_stops[0].issued_action, 0);
            if tick == 0 {
                assert_eq!(reset.status().phase, FaultResetPhase::ZeroRequired);
                assert_eq!(
                    reset.submit(
                        &guard,
                        FaultResetRequest {
                            boot_id: 7,
                            sequence: 2,
                            transition_sequence: guard.snapshot(1, now).transition_sequence,
                            axis_mask: 1,
                            deadline_ns: 5_000_000
                        },
                        now
                    ),
                    Err(FaultResetError::ZeroRequired)
                );
            }
            if tick == 1 {
                assert_eq!(reset.status().phase, FaultResetPhase::Complete);
                assert_eq!(guard.clear_fault(2), Err(LifecycleError::NotReady));
            }
            assert_eq!(guard.state(), LifecycleState::FaultLatched);
            assert!(!guard.snapshot(tick + 1, now).motion_permit_current);
        }
        owner
            .settle_output(&outputs, &outcome, now + 1_500_000)
            .unwrap();
    }
    if !fail_pulse {
        assert_eq!(port.pulses, 1);
        assert_eq!(port.controls, &[0, 0x80, 0, 0]);
        guard.clear_fault(3).unwrap();
        assert_eq!(guard.state(), LifecycleState::Qualifying);
        assert!(!guard.snapshot(3, 3_000_000).motion_permit_current);
    }
}
#[test]
fn owner_pipeline_pulses_once_deasserts_and_preserves_latched_fault_until_explicit_stable_clear() {
    run_flow(false, InputCase::Complete);
}
#[test]
fn failed_reset_tx_publishes_unissued_truth_and_never_advances_pulse_state() {
    run_flow(true, InputCase::Complete);
}

#[test]
fn missing_mode_target_mapping_is_rejected_before_any_maintenance_output_tx() {
    run_flow(false, InputCase::MissingTarget);
}

#[test]
fn valid_small_scale_nonfinite_feedback_is_rejected_before_any_maintenance_output_tx() {
    run_flow(false, InputCase::NonFiniteFeedback);
}
