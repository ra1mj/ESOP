use esop_ethercat_core::wire::{Command, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    CycleError, CycleReport, DatagramPlan, DcCyclicConfig, DcCyclicSync, DcMonitor, Domain,
    DomainSegment, EthercatMaster, FramePlan, MasterConfig, PdoDirection, PdoEntry,
};
use esop_ethercat_linux_port::SimulatedPort;
use esop_lifecycle_guard::cia402::step_axis_bank;
use esop_lifecycle_guard::ethercat::{
    OtherCycleFacts, StopFrameError, cyclic_quality_from_ethercat, submit_stopping_frame,
    verified_ethercat_stop_feedback,
};
use esop_lifecycle_guard::procbuf::{
    LifecycleEventCursor, axis_stops_to_procbuf, lifecycle_events_to_procbuf, lifecycle_to_procbuf,
};
use esop_lifecycle_guard::{
    GateId, GuardPolicy, LifecycleAction, LifecycleError, LifecycleGuard, LifecycleState,
    MotionPermit,
};
use esop_procbuf::{ProcBuf, StatePage};
use esop_profile_cia402::{
    CONTROLWORD_QUICK_STOP, Cia402AxisBank, Cia402PdoField, Cia402PdoMap, DriveRequest,
    OperatingMode,
};

const IMAGE_BYTES: usize = 32;

fn map_at(base: usize) -> Cia402PdoMap {
    let mut map = Cia402PdoMap::new();
    for (field, bit_offset, bit_length, signed, direction) in [
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
            Cia402PdoField::ActualVelocity,
            72,
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
                bit_offset: base + bit_offset,
                bit_length,
                signed,
                direction,
            },
        );
    }
    map.validate_for(OperatingMode::Csp).unwrap();
    map
}

fn map() -> Cia402PdoMap {
    map_at(0)
}

fn input_image(statusword: u16, velocity: i32) -> [u8; IMAGE_BYTES] {
    let mut image = [0; IMAGE_BYTES];
    image[..2].copy_from_slice(&statusword.to_le_bytes());
    image[2] = OperatingMode::Csp.raw() as u8;
    image[9..13].copy_from_slice(&velocity.to_le_bytes());
    image
}

fn submit<const BYTES: usize>(
    master: &mut EthercatMaster<1, MAX_ETHERNET_FRAME_LEN>,
    port: &mut SimulatedPort,
    domain: &mut Domain<BYTES, 1>,
    plan: &FramePlan<1>,
    generation: u16,
    image: &[u8; BYTES],
    drop_response: bool,
) {
    port.set_now_ns(u64::from(generation) * 100_000);
    assert_eq!(
        master.reap_expired_rx_before_tx(port.now_ns_value()),
        usize::from(generation == 5)
    );
    domain.begin_receive(generation).unwrap();
    let frame = master
        .acquire_frame(generation, port.now_ns_value() + 50_000)
        .unwrap();
    master
        .build_and_arm_frame_from_plan(frame, plan, image)
        .unwrap();
    if drop_response {
        port.drop_next_response();
    }
    master.submit_frame(port, frame).unwrap();
}

fn receive<const BYTES: usize>(
    master: &mut EthercatMaster<1, MAX_ETHERNET_FRAME_LEN>,
    port: &mut SimulatedPort,
    domain: &mut Domain<BYTES, 1>,
    generation: u16,
) -> CycleReport {
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let report = master
        .cycle_receive_with_consumer(port, &mut scratch, generation, domain)
        .unwrap();
    domain.finish_receive(generation, report.cycle).unwrap();
    report
}

#[test]
fn failed_stop_tx_never_becomes_stop_proof_and_retry_requires_a_new_response() {
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut domain = Domain::<IMAGE_BYTES, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: IMAGE_BYTES,
            expected_wkc: 1,
        })
        .unwrap();
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: IMAGE_BYTES,
        expected_wkc: 1,
    })
    .unwrap();
    let map = map();
    let mut port = SimulatedPort::new(1);
    let mut guard = LifecycleGuard::new(
        GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    submit(
        &mut master,
        &mut port,
        &mut domain,
        &plan,
        1,
        &input_image(0x0027, 10),
        false,
    );
    let first = receive(&mut master, &mut port, &mut domain, 1);
    guard.update_gate(GateId::Link, true, first.cycle, 0);
    guard
        .request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                expires_at_ns: 10_000,
                axis_mask: 1,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            first.cycle,
            100,
        )
        .unwrap();
    let mut allowed = guard.cycle_axes(first.cycle, 100);
    assert_eq!(
        allowed.mark_stop_transmitted(),
        Err(LifecycleError::InvalidState)
    );

    submit(
        &mut master,
        &mut port,
        &mut domain,
        &plan,
        2,
        &input_image(0x0027, 10),
        false,
    );
    let second = receive(&mut master, &mut port, &mut domain, 2);
    guard.update_gate(GateId::Link, false, second.cycle, 0xCAFE);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = second.cycle;
    let stop_image = input_image(0x0040, 0);
    port.set_now_ns(300_000);
    domain.begin_receive(3).unwrap();

    let outputs = {
        let mut decision = guard.cycle_axes(second.cycle, 200);
        let mut bank = Cia402AxisBank::<1>::new();
        let outputs = step_axis_bank(&mut bank, &decision, [0x0027], [DriveRequest::Enable]);
        assert_eq!(outputs[0].controlword, CONTROLWORD_QUICK_STOP);

        let mut unsafe_outputs = outputs;
        unsafe_outputs[0].controlword = 0x000F;
        assert!(matches!(
            submit_stopping_frame(
                &mut decision,
                &unsafe_outputs,
                &[map],
                &[OperatingMode::Csp],
                &stop_image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                3,
                350_000,
            ),
            Err(StopFrameError::UnsafeOutput(0))
        ));

        for (command, address) in [(Command::Lrd, 0x1000), (Command::Lrw, 0x1001)] {
            let mut invalid_plan = FramePlan::<1>::new();
            invalid_plan
                .push(DatagramPlan {
                    command,
                    index: 12,
                    address,
                    payload_offset: 0,
                    payload_len: IMAGE_BYTES,
                    expected_wkc: 1,
                })
                .unwrap();
            assert!(matches!(
                submit_stopping_frame(
                    &mut decision,
                    &outputs,
                    &[map],
                    &[OperatingMode::Csp],
                    &stop_image,
                    &domain,
                    &invalid_plan,
                    &mut master,
                    &mut port,
                    3,
                    350_000,
                ),
                Err(StopFrameError::UncoveredOutput(
                    0,
                    Cia402PdoField::Controlword
                ))
            ));
        }

        let duplicated_outputs = step_axis_bank(
            &mut Cia402AxisBank::<2>::new(),
            &decision,
            [0x0027, 0x0040],
            [DriveRequest::Enable, DriveRequest::Enable],
        );
        assert!(matches!(
            submit_stopping_frame(
                &mut decision,
                &duplicated_outputs,
                &[map, map],
                &[OperatingMode::Csp; 2],
                &stop_image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                3,
                350_000,
            ),
            Err(StopFrameError::OverlappingOutput(0, 1))
        ));

        let mut overwriting_plan = FramePlan::<2>::new();
        overwriting_plan.push(plan.datagrams()[0]).unwrap();
        overwriting_plan
            .push(DatagramPlan {
                command: Command::Lwr,
                index: 13,
                address: 0x1010,
                payload_offset: 20,
                payload_len: 2,
                expected_wkc: 1,
            })
            .unwrap();
        assert!(matches!(
            submit_stopping_frame(
                &mut decision,
                &outputs,
                &[map],
                &[OperatingMode::Csp],
                &stop_image,
                &domain,
                &overwriting_plan,
                &mut master,
                &mut port,
                3,
                350_000,
            ),
            Err(StopFrameError::UncoveredOutput(
                0,
                Cia402PdoField::Controlword
            ))
        ));

        let mut unbuildable_plan = FramePlan::<2>::new();
        unbuildable_plan.push(plan.datagrams()[0]).unwrap();
        unbuildable_plan
            .push(DatagramPlan {
                command: Command::Lrw,
                index: 13,
                address: 0x2000,
                payload_offset: IMAGE_BYTES,
                payload_len: 1,
                expected_wkc: 1,
            })
            .unwrap();
        assert!(matches!(
            submit_stopping_frame(
                &mut decision,
                &outputs,
                &[map],
                &[OperatingMode::Csp],
                &stop_image,
                &domain,
                &unbuildable_plan,
                &mut master,
                &mut port,
                3,
                350_000,
            ),
            Err(StopFrameError::Build(CycleError::Plan(_)))
        ));
        assert_eq!(decision.stop_issued_cycle(), None);

        port.fail_next_tx();
        assert!(matches!(
            submit_stopping_frame(
                &mut decision,
                &outputs,
                &[map],
                &[OperatingMode::Csp],
                &stop_image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                3,
                350_000,
            ),
            Err(StopFrameError::Transmit(CycleError::Port(_)))
        ));
        assert_eq!(decision.stop_issued_cycle(), None);
        axis_stops_to_procbuf(&mut state, &decision, &outputs, None).unwrap();
        assert_eq!(
            state.axis_stops[0].requested_action,
            esop_lifecycle_guard::StopAction::QuickStop as u8 + 1
        );
        assert_eq!(state.axis_stops[0].issued_action, 0);
        outputs
    };
    assert_eq!(
        guard.acknowledge_stopped(
            3,
            esop_lifecycle_guard::StopFeedback {
                cycle: 3,
                observed_axis_mask: 1,
                stationary_axis_mask: 1,
                non_enabled_axis_mask: 1,
            }
        ),
        Err(LifecycleError::InvalidStopFeedback)
    );

    let mut retry = guard.cycle_axes(second.cycle, 201);
    submit_stopping_frame(
        &mut retry,
        &outputs,
        &[map],
        &[OperatingMode::Csp],
        &stop_image,
        &domain,
        &plan,
        &mut master,
        &mut port,
        3,
        350_000,
    )
    .unwrap();
    assert_eq!(retry.stop_issued_cycle(), Some(second.cycle));
    axis_stops_to_procbuf(&mut state, &retry, &outputs, None).unwrap();
    assert_eq!(
        state.axis_stops[0].issued_action,
        esop_lifecycle_guard::StopAction::QuickStop as u8 + 1
    );
    let third = receive(&mut master, &mut port, &mut domain, 3);
    assert_eq!(
        &domain.input()[16..18],
        &CONTROLWORD_QUICK_STOP.to_le_bytes()
    );
    let decision = guard.cycle_axes(third.cycle, 300);
    let feedback = verified_ethercat_stop_feedback(
        &decision,
        third,
        &domain,
        &[map],
        &[OperatingMode::Csp],
        &[1],
    )
    .unwrap();
    guard.acknowledge_stopped(third.cycle, feedback).unwrap();
    assert_eq!(guard.state(), LifecycleState::Ready);
}

#[test]
fn verified_stop_requires_complete_feedback_for_every_armed_axis() {
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut domain = Domain::<64, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 64,
            expected_wkc: 1,
        })
        .unwrap();
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 64,
        expected_wkc: 1,
    })
    .unwrap();
    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            stop_timeout_cycles: 10,
            allowed_axis_mask: 3,
            ..GuardPolicy::conservative()
        },
    );
    let dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x2000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
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
    let mut port = SimulatedPort::new(1);
    let maps = [map_at(0), map_at(256)];
    let mut bank = Cia402AxisBank::<2>::new();
    let scenarios = [
        (1, 1, 0x0027, 10),
        (2, 0, 0x0040, 0),
        (3, 1, 0x0027, 0),
        (4, 1, 0x0040, 0),
    ];
    let mut first_image = [0; 64];
    first_image[..32].copy_from_slice(&input_image(0x0027, 0));
    first_image[32..].copy_from_slice(&input_image(0x0027, 10));
    submit(
        &mut master,
        &mut port,
        &mut domain,
        &plan,
        1,
        &first_image,
        false,
    );
    for (index, &(generation, _, _, _)) in scenarios.iter().enumerate() {
        let report = receive(&mut master, &mut port, &mut domain, generation);
        let facts = cyclic_quality_from_ethercat(report, &[domain.quality()], &dc, other);
        guard.update_cyclic_quality(facts, report.cycle);
        if generation == 1 {
            guard
                .request_rearm(
                    MotionPermit {
                        boot_id: 7,
                        source_id: 1,
                        permit_epoch: 1,
                        sequence: 1,
                        expires_at_ns: 10_000,
                        axis_mask: 3,
                        authority: 1,
                        reserved: [0; 3],
                        policy_version: 1,
                    },
                    report.cycle,
                    100,
                )
                .unwrap();
        }
        let feedback = {
            let mut decision = guard.cycle_axes(report.cycle, 100 + report.cycle);
            let feedback = verified_ethercat_stop_feedback(
                &decision,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp; 2],
                &[1; 2],
            );
            if generation >= 3 {
                let mut malformed = maps;
                malformed[1].clear_entry(Cia402PdoField::Statusword);
                assert_eq!(
                    verified_ethercat_stop_feedback(
                        &decision,
                        report,
                        &domain,
                        &malformed,
                        &[OperatingMode::Csp; 2],
                        &[1; 2],
                    ),
                    None
                );
            }
            if let Some(&(next_generation, next_wkc, next_status, next_velocity)) =
                scenarios.get(index + 1)
            {
                let statuswords = [
                    if generation == 1 { 0x0027 } else { 0x0040 },
                    if generation == 1 || generation == 3 {
                        0x0027
                    } else {
                        0x0040
                    },
                ];
                let outputs =
                    step_axis_bank(&mut bank, &decision, statuswords, [DriveRequest::Enable; 2]);
                let mut next_image = [0; 64];
                next_image[..32].copy_from_slice(&input_image(0x0040, 0));
                next_image[32..].copy_from_slice(&input_image(next_status, next_velocity));
                for axis in 0..2 {
                    maps[axis]
                        .write_control(
                            &mut next_image,
                            OperatingMode::Csp,
                            outputs[axis].controlword,
                        )
                        .unwrap();
                }
                port.set_response_wkc(next_wkc);
                submit(
                    &mut master,
                    &mut port,
                    &mut domain,
                    &plan,
                    next_generation,
                    &next_image,
                    false,
                );
                if decision.stopping_axis_mask() != 0 {
                    decision.mark_stop_transmitted().unwrap();
                }
            }
            feedback
        };
        if generation <= 2 {
            assert_eq!(feedback, None);
        }
        if generation == 3 {
            let feedback = feedback.unwrap();
            assert_eq!(feedback.observed_axis_mask, 3);
            assert_eq!(feedback.stationary_axis_mask, 3);
            assert_eq!(feedback.non_enabled_axis_mask, 1);
            assert_eq!(
                guard.acknowledge_stopped(report.cycle, feedback),
                Err(LifecycleError::InvalidStopFeedback)
            );
        }
        if generation == 4 {
            let feedback = feedback.unwrap();
            assert_eq!(feedback.non_enabled_axis_mask, 3);
            guard.acknowledge_stopped(report.cycle, feedback).unwrap();
            assert_eq!(guard.state(), LifecycleState::Ready);
        }
    }
}

#[test]
fn stop_confirmation_requires_fresh_complete_drive_input_after_stop_output() {
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut domain = Domain::<IMAGE_BYTES, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: IMAGE_BYTES,
            expected_wkc: 1,
        })
        .unwrap();
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: IMAGE_BYTES,
        expected_wkc: 1,
    })
    .unwrap();
    let dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x2000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let required = GateId::Domain.bit() | GateId::Link.bit() | GateId::Budget.bit();
    let mut guard = LifecycleGuard::new(
        required,
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            stop_timeout_cycles: 10,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    let buffer = ProcBuf::<1, 0, 1, 8>::new(1, 7);
    let mut events = LifecycleEventCursor::new(&guard);
    let mut bank = Cia402AxisBank::<1>::new();
    let maps = [map()];
    let mut port = SimulatedPort::new(1);
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

    let scenarios = [
        (1, 1, false, input_image(0x0027, 10)),
        (2, 0, false, input_image(0x0040, 0)),
        (3, 1, false, input_image(0x0040, 10)),
        (4, 1, true, input_image(0x0040, 0)),
        (5, 1, false, input_image(0x0040, 0)),
    ];
    submit(
        &mut master,
        &mut port,
        &mut domain,
        &plan,
        1,
        &scenarios[0].3,
        false,
    );
    for (index, &(generation, _, _, _)) in scenarios.iter().enumerate() {
        let report = receive(&mut master, &mut port, &mut domain, generation);
        let facts = cyclic_quality_from_ethercat(report, &[domain.quality()], &dc, other);
        guard.update_cyclic_quality(facts, report.cycle);
        if generation == 1 {
            assert_eq!(
                guard.request_rearm(
                    MotionPermit {
                        boot_id: 7,
                        source_id: 1,
                        permit_epoch: 1,
                        sequence: 1,
                        expires_at_ns: 10_000,
                        axis_mask: 1,
                        authority: 1,
                        reserved: [0; 3],
                        policy_version: 1,
                    },
                    report.cycle,
                    100,
                ),
                Ok(LifecycleAction::EnableAllowed)
            );
        }
        let mut state = StatePage::<1, 0, 1>::new(7);
        state.sequence = report.cycle;
        let statusword = if facts.domain_valid && facts.wkc_valid {
            maps[0]
                .read_inputs_for(domain.input(), OperatingMode::Csp)
                .unwrap()
                .statusword
        } else {
            u16::MAX
        };
        let feedback = {
            let mut decision = guard.cycle_axes(report.cycle, 100 + report.cycle);
            let outputs =
                step_axis_bank(&mut bank, &decision, [statusword], [DriveRequest::Enable]);
            let feedback = verified_ethercat_stop_feedback(
                &decision,
                report,
                &domain,
                &maps,
                &[OperatingMode::Csp],
                &[1],
            );
            if generation == 1 {
                assert_eq!(decision.action(), LifecycleAction::EnableAllowed);
                assert_eq!(feedback, None);
            } else {
                assert_eq!(outputs[0].controlword, CONTROLWORD_QUICK_STOP);
                assert!(!outputs[0].motion_allowed);
                assert_eq!(feedback.is_some(), generation == 3 || generation == 5);
            }
            if generation == 3 {
                assert_eq!(
                    verified_ethercat_stop_feedback(
                        &decision,
                        CycleReport {
                            budget_exhausted: true,
                            ..report
                        },
                        &domain,
                        &maps,
                        &[OperatingMode::Csp],
                        &[1],
                    ),
                    None
                );
                assert_eq!(
                    verified_ethercat_stop_feedback(
                        &decision,
                        CycleReport {
                            cycle: report.cycle + 1,
                            ..report
                        },
                        &domain,
                        &maps,
                        &[OperatingMode::Csp],
                        &[1],
                    ),
                    None
                );
                let mut invalid = maps[0];
                invalid.clear_entry(Cia402PdoField::Statusword);
                assert_eq!(
                    verified_ethercat_stop_feedback(
                        &decision,
                        report,
                        &domain,
                        &[invalid],
                        &[OperatingMode::Csp],
                        &[1],
                    ),
                    None
                );
                let mut without_velocity = maps[0];
                without_velocity.clear_entry(Cia402PdoField::ActualVelocity);
                let missing_velocity = verified_ethercat_stop_feedback(
                    &decision,
                    report,
                    &domain,
                    &[without_velocity],
                    &[OperatingMode::Csp],
                    &[1],
                )
                .unwrap();
                assert_eq!(missing_velocity.stationary_axis_mask, 0);
            }
            if let Some(&(next_generation, next_wkc, next_drop, mut next_image)) =
                scenarios.get(index + 1)
            {
                maps[0]
                    .write_control(&mut next_image, OperatingMode::Csp, outputs[0].controlword)
                    .unwrap();
                port.set_response_wkc(next_wkc);
                submit(
                    &mut master,
                    &mut port,
                    &mut domain,
                    &plan,
                    next_generation,
                    &next_image,
                    next_drop,
                );
                if decision.stopping_axis_mask() != 0 {
                    decision.mark_stop_transmitted().unwrap();
                }
            }
            if generation != 1 {
                axis_stops_to_procbuf(&mut state, &decision, &outputs, feedback).unwrap();
                assert_eq!(
                    state.axis_stops[0].feedback_valid,
                    u8::from(feedback.is_some())
                );
                assert_eq!(state.axis_stops[0].issued_action != 0, generation < 5);
            }
            feedback
        };

        if generation == 3 {
            let feedback = feedback.unwrap();
            assert_eq!(feedback.stationary_axis_mask, 0);
            assert_eq!(
                guard.acknowledge_stopped(report.cycle, feedback),
                Err(LifecycleError::InvalidStopFeedback)
            );
        }
        if generation == 4 {
            let mut retained = input_image(0x0040, 10);
            maps[0]
                .write_control(&mut retained, OperatingMode::Csp, CONTROLWORD_QUICK_STOP)
                .unwrap();
            assert_eq!(domain.input(), &retained);
            assert_eq!(guard.state(), LifecycleState::Stopping);
        }
        if generation == 5 {
            guard
                .acknowledge_stopped(report.cycle, feedback.unwrap())
                .unwrap();
            assert_eq!(guard.state(), LifecycleState::Ready);
        }
        state.lifecycle = lifecycle_to_procbuf(guard.snapshot(report.cycle, 100), 100);
        buffer.publish_state(state).unwrap();
        lifecycle_events_to_procbuf(&mut guard, &buffer, &mut events, 100 + report.cycle).unwrap();
        assert_eq!(
            buffer.read_state().unwrap().state.lifecycle.state,
            guard.state() as u8
        );
        while buffer.pop_event().is_some() {}
    }
}
