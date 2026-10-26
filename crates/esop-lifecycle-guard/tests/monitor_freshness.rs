#![cfg(all(feature = "ethercat", feature = "cia402", feature = "procbuf"))]
use core::cell::Cell;
use esop_ethercat_core::wire::{Command, DatagramHeader, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::*;
use esop_lifecycle_guard::ethercat::OtherCycleFacts;
use esop_lifecycle_guard::procbuf::{Cia402AxisCommandPolicy, LifecycleEventCursor};
use esop_lifecycle_guard::stop_cycle::{
    AuxiliaryOutputEntry, ScheduledAuxiliaryOutputs, ScheduledProductionCycleOwner,
    StopCycleContext,
};
use esop_lifecycle_guard::{
    GateId, GuardPolicy, LifecycleAction, LifecycleGuard, LifecycleState, MotionPermit,
};
use esop_procbuf::{ProcBuf, StatePage};
use esop_profile_cia402::{
    Cia402AxisBank, Cia402PdoField, Cia402PdoMap, Cia402Target, CyclicLimits, CyclicSetpointGuard,
    OperatingMode,
};

#[derive(Clone, Copy, PartialEq)]
enum Expiry {
    Never,
    BeforeOutput,
    DuringAuxiliary,
    DuringMotion,
    DuringPublication,
}

struct Port {
    now: u64,
    frames: [[u8; MAX_ETHERNET_FRAME_LEN]; 4],
    lengths: [usize; 4],
    head: usize,
    count: usize,
    expire_on: Option<u32>,
    controls: Vec<u16>,
    expire_after_motion: bool,
    post_motion_reads: Cell<u32>,
}
impl EthercatPort for Port {
    type Error = core::convert::Infallible;
    fn now_ns(&self) -> u64 {
        let reads = self.post_motion_reads.get();
        if reads != 0 {
            self.post_motion_reads.set(reads + 1);
            if reads >= 2 {
                return 22;
            }
        }
        self.now
    }
    fn link_state(&self) -> LinkState {
        LinkState::Up
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        let header = DatagramHeader::decode(&frame[16..26]).unwrap();
        let slot = self.count;
        assert!(slot < self.frames.len());
        self.frames[slot][..frame.len()].copy_from_slice(frame);
        self.lengths[slot] = frame.len();
        self.count += 1;
        match (header.command, header.address) {
            (Command::Lrw, 0x1000) => {
                self.controls
                    .push(u16::from_le_bytes([frame[42], frame[43]]));
                self.frames[slot][26..28].copy_from_slice(&0x23u16.to_le_bytes());
                self.frames[slot][28] = 8;
                if self.expire_after_motion {
                    self.post_motion_reads.set(1);
                }
            }
            (Command::Fprd, 0x0130_1000) => {
                self.frames[slot][26..32].fill(0);
                self.frames[slot][26] = 8;
            }
            (Command::Fprd, 0x0110_1000) => {
                self.frames[slot][26..28].copy_from_slice(&0x10u16.to_le_bytes());
            }
            _ => {}
        }
        let end = 26 + usize::from(header.length);
        self.frames[slot][end..end + 2].copy_from_slice(&1u16.to_le_bytes());
        if self.expire_on == Some(header.address) {
            self.expire_on = None;
            self.now = 22;
        }
        Ok(())
    }
    fn rx_poll(
        &mut self,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
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

fn run(expiry: Expiry) {
    let schedule = ScheduleTable::<2, 1>::build(
        10,
        &[
            ScheduleDomain {
                id: 0,
                period_ticks: 1,
                phase_ticks: 0,
            },
            ScheduleDomain {
                id: 1,
                period_ticks: 1,
                phase_ticks: 0,
            },
        ],
    )
    .unwrap();
    let mut motion = Domain::<32, 1>::new(0x1000);
    motion
        .add_segment(DomainSegment {
            datagram_index: 1,
            input_offset: 0,
            len: 32,
            expected_wkc: 1,
        })
        .unwrap();
    let mut auxiliary = Domain::<1, 1>::new(0x2000);
    auxiliary
        .add_segment(DomainSegment {
            datagram_index: 2,
            input_offset: 0,
            len: 1,
            expected_wkc: 1,
        })
        .unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [
            ScheduledDomainEntry {
                id: 0,
                domain: &mut motion,
            },
            ScheduledDomainEntry {
                id: 1,
                domain: &mut auxiliary,
            },
        ],
    )
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
    let mut motion_plans = FramePlanSet::<1, 1>::new();
    motion_plans.push(plan.datagrams()[0]).unwrap();
    let mut auxiliary_plans = FramePlanSet::<1, 1>::new();
    auxiliary_plans
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 2,
            address: 0x2000,
            payload_offset: 0,
            payload_len: 1,
            expected_wkc: 1,
        })
        .unwrap();
    let safe = [0; 32];
    let aux_safe = [0; 1];
    let outputs = ScheduledAuxiliaryOutputs::<2, 1, 1, 1>::new(
        &bank,
        &schedule,
        0,
        &plan,
        [
            None,
            Some(AuxiliaryOutputEntry {
                id: 1,
                image: &aux_safe,
                plans: &auxiliary_plans,
            }),
        ],
    )
    .unwrap();
    let mut owner = ScheduledProductionCycleOwner::new(&outputs);
    let inputs = ScheduledProcessInputs::new(
        &bank,
        &schedule,
        [
            ScheduledProcessInputEntry {
                id: 0,
                image: &safe,
                plans: &motion_plans,
            },
            ScheduledProcessInputEntry {
                id: 1,
                image: &aux_safe,
                plans: &auxiliary_plans,
            },
        ],
    )
    .unwrap();
    let mut master =
        EthercatMaster::<4, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new([0xff; 6], [0; 6]));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x1000, 240, 0),
        DcMonitor::new(50, 10, 1, 1),
    );
    let mut monitor = SlaveMonitorController::<1>::new(
        42,
        &[SlaveMonitorConfig {
            position: 0,
            station_address: 0x1000,
            required_al_state: EthercatState::Op,
            required_dl_mask: 0x10,
        }],
        15,
        21,
        100,
        0,
    )
    .unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = Port {
        now: 0,
        frames: [[0; MAX_ETHERNET_FRAME_LEN]; 4],
        lengths: [0; 4],
        head: 0,
        count: 0,
        expire_on: None,
        controls: Vec::new(),
        expire_after_motion: false,
        post_motion_reads: Cell::new(0),
    };
    for tick in 0..3u16 {
        let now = u64::from(tick) * 10;
        port.now = now;
        let prime = bank
            .submit_due_process_inputs(&inputs, &mut master, &mut port, tick + 1, now + 5, now + 5)
            .unwrap();
        if tick == 2 {
            owner
                .arm_priming(&bank, &inputs, &prime, tick + 1, now + 5)
                .unwrap();
        }
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
                &mut ScheduledProductionServices::<0, 0, 0>::new(None, None, None, None)
                    .with_slave_monitor(&mut monitor),
                tick + 1,
                now + 5,
                now + 5,
            )
            .unwrap();
        if tick != 2 {
            continue;
        }
        let quality = service.slave_monitor_quality().unwrap();
        assert!(quality.is_good_at(20));
        assert_eq!(quality.fresh_until_ns, Some(21));
        owner.complete_service_cycle(&bank, &service).unwrap();
        let mut guard = LifecycleGuard::new(
            GateId::Topology.bit(),
            7,
            GuardPolicy {
                allowed_axis_mask: 1,
                enter_good_cycles: 1,
                ..GuardPolicy::conservative()
            },
        );
        guard.update_gate(GateId::Topology, true, service.received().report.cycle, 0);
        guard
            .request_rearm(
                MotionPermit {
                    boot_id: 7,
                    source_id: 1,
                    permit_epoch: 1,
                    sequence: 1,
                    expires_at_ns: 1000,
                    axis_mask: 1,
                    authority: 1,
                    reserved: [0; 3],
                    policy_version: 1,
                },
                service.received().report.cycle,
                now,
            )
            .unwrap();
        assert_eq!(guard.state(), LifecycleState::Active);
        let buffer = ProcBuf::<1, 0, 2, 16>::new(1, 7);
        let mut state = StatePage::new(7);
        state.sequence = service.received().report.cycle;
        let mut cursor = LifecycleEventCursor::new(&guard);
        let mut axis_bank = Cia402AxisBank::<1>::new();
        let mut target_guards = [CyclicSetpointGuard::new()];
        if expiry == Expiry::BeforeOutput {
            port.now = 22;
        }
        port.expire_on = match expiry {
            Expiry::DuringAuxiliary => Some(0x2000),
            Expiry::DuringMotion => Some(0x1000),
            _ => None,
        };
        port.expire_after_motion = expiry == Expiry::DuringPublication;
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
        let outcome = StopCycleContext {
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
            maps: &[map()],
            modes: &[OperatingMode::Csp],
            axis_policies: &[POLICY],
            max_stationary_velocities: &[0],
            safe_process_image: &safe,
            shared_process_image: None,
            plan: &plan,
            next_generation: tick + 2,
            deadline_ns: 35,
            now_ns: now,
            transition_time_ns: 0,
        }
        .run_service_cycle_with_outputs_until(
            &bank,
            &service,
            0,
            &outputs,
            &[Some(Cia402Target::Position(0))],
            &mut target_guards,
            &[CyclicLimits {
                max_position_step: 1.,
                max_velocity: 1.,
                max_torque: 1.,
            }],
            25,
        )
        .unwrap();
        assert!(outcome.transmission.is_ok());
        assert!(outcome.state_publish.is_ok());
        assert!(outcome.quality.cycle_within_budget);
        assert_eq!(outcome.post_tx_deadline_met, Some(true));
        assert!(!outcome.active_tx_before_deadline_miss);
        assert_eq!(state.axes[0].controlword, *port.controls.last().unwrap());
        if expiry == Expiry::Never {
            assert_eq!(outcome.action, LifecycleAction::EnableAllowed);
            assert_eq!(state.axes[0].controlword, 0xf);
            assert!(outcome.quality.topology_valid);
        } else {
            assert!(matches!(outcome.action, LifecycleAction::Stop(_)));
            assert_eq!(guard.state(), LifecycleState::Stopping);
            assert!(!outcome.quality.topology_valid);
            assert!(
                !guard
                    .snapshot(service.received().report.cycle, 22)
                    .motion_permit_current
            );
            if matches!(expiry, Expiry::DuringMotion | Expiry::DuringPublication) {
                assert_eq!(state.axes[0].controlword, 0xf); // Accepted TX is immutable truth.
                assert_eq!(state.axis_stops[0].issued_action, 0); // No stop frame was sent yet.
            } else {
                assert_ne!(state.axes[0].controlword, 0xf);
                assert_ne!(state.axis_stops[0].issued_action, 0);
            }
        }
        if expiry == Expiry::DuringPublication {
            assert!(matches!(outcome.deadline_correction_publish, Some(Ok(_))));
            assert!(matches!(outcome.deadline_correction_events, Some(Ok(_))));
            assert_eq!(outcome.post_publication_deadline_met, Some(true));
        }
        owner.settle_output(&outputs, &outcome, 35).unwrap();
    }
}
#[test]
fn current_monitor_allows_the_real_enable_path() {
    run(Expiry::Never);
}
#[test]
fn monitor_expiry_between_receive_and_output_stops_before_enable() {
    run(Expiry::BeforeOutput);
}
#[test]
fn monitor_expiry_during_auxiliary_tx_stops_before_enable() {
    run(Expiry::DuringAuxiliary);
}
#[test]
fn monitor_expiry_during_accepted_motion_tx_preserves_wire_truth_and_requests_stop() {
    run(Expiry::DuringMotion);
}

#[test]
fn monitor_expiry_after_publication_corrects_state_and_preserves_accepted_tx() {
    run(Expiry::DuringPublication);
}
