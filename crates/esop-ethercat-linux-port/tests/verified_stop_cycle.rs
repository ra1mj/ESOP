use esop_ethercat_core::wire::{Command, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    CycleError, CycleReport, DatagramPlan, DcCyclicConfig, DcCyclicSync, DcMonitor, Domain,
    DomainSegment, EthercatMaster, EthercatPort, FramePlan, LinkState, MasterConfig, PdoDirection,
    PdoEntry, PortError, RxConsumerMux, RxPoll, ScheduleDomain, ScheduleTable,
};
use esop_ethercat_linux_port::SimulatedPort;
use esop_lifecycle_guard::cia402::step_axis_bank;
use esop_lifecycle_guard::ethercat::{
    OtherCycleFacts, ScheduledDomainQuality, StopFrameError, cyclic_quality_from_ethercat,
    cyclic_quality_from_schedule, submit_active_frame, submit_inhibited_frame,
    submit_stopping_frame, verified_ethercat_stop_feedback,
};
use esop_lifecycle_guard::procbuf::{
    LifecycleEventCursor, axis_stops_to_procbuf, lifecycle_events_to_procbuf, lifecycle_to_procbuf,
};
use esop_lifecycle_guard::stop_cycle::{StopCycleContext, StopCycleError};
use esop_lifecycle_guard::{
    GateId, GuardPolicy, LifecycleAction, LifecycleError, LifecycleGuard, LifecycleState,
    MotionPermit,
};
use esop_procbuf::{ProcBuf, QualityFact, StatePage};
use esop_profile_cia402::{
    CONTROLWORD_DISABLE_VOLTAGE, CONTROLWORD_QUICK_STOP, Cia402AxisBank, Cia402PdoField,
    Cia402PdoMap, Cia402Target, CyclicLimits, CyclicSetpoint, CyclicSetpointError,
    CyclicSetpointGuard, DriveRequest, OperatingMode,
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

struct AdvancingTxPort<'a> {
    inner: &'a mut SimulatedPort,
    tx_duration_ns: u64,
}

impl EthercatPort for AdvancingTxPort<'_> {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.inner.link_state()
    }

    fn now_ns(&self) -> u64 {
        self.inner.now_ns()
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        let result = self.inner.tx_submit(frame);
        self.inner.advance_ns(self.tx_duration_ns);
        result
    }

    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        self.inner.rx_poll(destination)
    }
}

#[test]
fn checked_cycle_records_deadline_after_tx_without_claiming_a_stop_was_sent() {
    for (final_deadline_ns, tx_duration_ns, failed_active_tx, expected_active_tx) in [
        (100_000, 0, false, false),
        (105_000, 10_000, false, true),
        (115_000, 10_000, true, false),
        (120_000, 0, false, false),
    ] {
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
        let mut port = SimulatedPort::new(1);
        let image = input_image(0x0040, 0);
        submit(&mut master, &mut port, &mut domain, &plan, 1, &image, false);
        let report = receive(&mut master, &mut port, &mut domain, 1);
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
        let mut guard = LifecycleGuard::new(
            GateId::Domain.bit() | GateId::Link.bit() | GateId::Budget.bit(),
            7,
            GuardPolicy {
                enter_good_cycles: 1,
                allowed_axis_mask: 1,
                ..GuardPolicy::conservative()
            },
        );
        guard.update_cyclic_quality(
            cyclic_quality_from_ethercat(report, &[domain.quality()], &dc, other),
            report.cycle,
        );
        guard
            .request_rearm(
                MotionPermit {
                    boot_id: 7,
                    source_id: 1,
                    permit_epoch: 1,
                    sequence: 1,
                    expires_at_ns: 300_000,
                    axis_mask: 1,
                    authority: 1,
                    reserved: [0; 3],
                    policy_version: 1,
                },
                report.cycle,
                100_000,
            )
            .unwrap();
        let buffer = ProcBuf::<1, 0, 1, 8>::new(1, 7);
        let mut cursor = LifecycleEventCursor::new(&guard);
        let mut state = StatePage::<1, 0, 1>::new(7);
        state.sequence = report.cycle;
        let mut bank = Cia402AxisBank::<1>::new();
        let maps = [map()];
        let modes = [OperatingMode::Csp];
        let limits = [CyclicLimits {
            max_position_step: 2.0,
            max_velocity: 2.0,
            max_torque: 2.0,
        }];
        let mut guards = [CyclicSetpointGuard::new()];
        if failed_active_tx {
            port.fail_next_tx();
        }
        let mut timed_port = AdvancingTxPort {
            inner: &mut port,
            tx_duration_ns,
        };
        {
            let mut context = StopCycleContext {
                guard: &mut guard,
                bank: &mut bank,
                master: &mut master,
                port: &mut timed_port,
                domain: &domain,
                dc: &dc,
                buffer: &buffer,
                event_cursor: &mut cursor,
                state: &mut state,
                report,
                other,
                maps: &maps,
                modes: &modes,
                max_stationary_velocities: &[1],
                safe_process_image: &image,
                plan: &plan,
                next_generation: 2,
                deadline_ns: 150_000,
                now_ns: 100_000,
                transition_time_ns: 100_000,
            };
            assert!(matches!(
                context.run_with_motion_until(&[None], &mut guards, &limits, 0),
                Err(StopCycleError::InvalidCycleDeadline)
            ));
            assert_eq!(context.port.inner.tx_frames(), 1);
            let result = context
                .run_with_motion_until(&[None], &mut guards, &limits, final_deadline_ns)
                .unwrap();
            assert!(result.transmission.is_ok());
            assert_eq!(result.active_failure.is_some(), failed_active_tx);
            assert_eq!(result.active_tx_before_deadline_miss, expected_active_tx);
            assert_eq!(
                result.post_tx_deadline_met,
                Some(final_deadline_ns == 120_000)
            );
            assert_eq!(result.state_publish, Ok(1));
            assert_eq!(context.port.inner.tx_frames(), 2);
            if final_deadline_ns == 120_000 {
                assert_eq!(result.action, LifecycleAction::EnableAllowed);
                assert!(context.guard.permit().is_some());
            } else {
                assert!(matches!(result.action, LifecycleAction::Stop(_)));
                assert!(context.guard.permit().is_none());
                assert_eq!(context.guard.state(), LifecycleState::Stopping);
                assert_eq!(
                    context.state.lifecycle.first_blocking_code,
                    if failed_active_tx {
                        0x5458_0001
                    } else {
                        0x4255_0001
                    }
                );
                assert!(!result.quality.cycle_within_budget);
                assert_eq!(context.state.quality.sequence, report.cycle);
                assert!(context.state.quality.cyclic.complete());
                assert!(!context.state.quality.cyclic.good(QualityFact::CycleBudget));
                assert_ne!(context.state.axis_stops[0].requested_action, 0);
                assert_eq!(
                    context.state.axis_stops[0].issued_action != 0,
                    !expected_active_tx
                );
            }
        }
        if expected_active_tx {
            domain.begin_receive(2).unwrap();
            let second = receive(&mut master, &mut port, &mut domain, 2);
            assert_eq!(second.cycle, 2);
            let mut next_state = StatePage::<1, 0, 1>::new(7);
            next_state.sequence = second.cycle;
            let now_ns = port.now_ns_value();
            let mut timed_stop_port = AdvancingTxPort {
                inner: &mut port,
                tx_duration_ns: 40_000,
            };
            let sent = StopCycleContext {
                guard: &mut guard,
                bank: &mut bank,
                master: &mut master,
                port: &mut timed_stop_port,
                domain: &domain,
                dc: &dc,
                buffer: &buffer,
                event_cursor: &mut cursor,
                state: &mut next_state,
                report: second,
                other,
                maps: &maps,
                modes: &modes,
                max_stationary_velocities: &[1],
                safe_process_image: &image,
                plan: &plan,
                next_generation: 3,
                deadline_ns: 180_000,
                now_ns,
                transition_time_ns: 110_000,
            }
            .run_until(140_000)
            .unwrap();
            assert!(matches!(sent.action, LifecycleAction::Stop(_)));
            assert_eq!(sent.post_tx_deadline_met, Some(false));
            assert!(!sent.active_tx_before_deadline_miss);
            assert!(sent.transmission.is_ok());
            let published = buffer.read_state().unwrap().state;
            assert!(!published.quality.cyclic.good(QualityFact::CycleBudget));
            assert_ne!(published.axis_stops[0].issued_action, 0);
        }
    }
}

#[test]
fn active_frame_requires_verified_feedback_and_bounded_target_committed_only_after_tx() {
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
    let mut port = SimulatedPort::new(1);
    submit(
        &mut master,
        &mut port,
        &mut domain,
        &plan,
        1,
        &input_image(0x0027, 0),
        false,
    );
    let report = receive(&mut master, &mut port, &mut domain, 1);
    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    guard.update_cyclic_quality(
        cyclic_quality_from_ethercat(
            report,
            &[domain.quality()],
            &dc,
            OtherCycleFacts {
                platform_ready: true,
                coe_ready: true,
                topology_valid: true,
                drive_ready: true,
                command_current: true,
                supervisor_healthy: true,
                external_safety_clear: true,
                deadline_met: true,
            },
        ),
        report.cycle,
    );
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
            report.cycle,
            100,
        )
        .unwrap();
    let maps = [map()];
    let modes = [OperatingMode::Csp];
    let image = input_image(0x0040, 0);
    let mut bank = Cia402AxisBank::<1>::new();
    let mut guards = [CyclicSetpointGuard::new()];
    let limits = [CyclicLimits {
        max_position_step: 2.0,
        max_velocity: 2.0,
        max_torque: 2.0,
    }];
    let second = {
        let decision = guard.cycle_axes(report.cycle, 101);
        assert_eq!(decision.action(), LifecycleAction::EnableAllowed);
        let outputs = step_axis_bank(&mut bank, &decision, [0x0027], [DriveRequest::Enable]);
        let tx_before = port.tx_frames();
        let mut bad_report = report;
        bad_report.budget_exhausted = true;
        assert!(matches!(
            submit_active_frame(
                &decision,
                bad_report,
                &outputs,
                &[Some(Cia402Target::Position(1))],
                &mut guards,
                &limits,
                &maps,
                &modes,
                &image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                150_000,
            ),
            Err(StopFrameError::UnverifiedInput)
        ));
        assert!(matches!(
            submit_active_frame(
                &decision,
                report,
                &outputs,
                &[Some(Cia402Target::Position(1))],
                &mut guards,
                &limits,
                &maps,
                &modes,
                &image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                150_000,
            ),
            Err(StopFrameError::InvalidTarget(
                0,
                CyclicSetpointError::NotSeeded
            ))
        ));
        guards[0].seed_from_actual(CyclicSetpoint::ZERO).unwrap();
        assert!(matches!(
            submit_active_frame(
                &decision,
                report,
                &outputs,
                &[Some(Cia402Target::Position(3))],
                &mut guards,
                &limits,
                &maps,
                &modes,
                &image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                150_000,
            ),
            Err(StopFrameError::InvalidTarget(
                0,
                CyclicSetpointError::PositionStepExceeded
            ))
        ));
        let mut unsafe_outputs = outputs;
        unsafe_outputs[0].controlword = CONTROLWORD_DISABLE_VOLTAGE;
        assert!(matches!(
            submit_active_frame(
                &decision,
                report,
                &unsafe_outputs,
                &[Some(Cia402Target::Position(1))],
                &mut guards,
                &limits,
                &maps,
                &modes,
                &image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                150_000,
            ),
            Err(StopFrameError::UnsafeOutput(0))
        ));
        assert_eq!(port.tx_frames(), tx_before);
        assert_eq!(guards[0].last(), CyclicSetpoint::ZERO);

        port.fail_next_tx();
        assert!(matches!(
            submit_active_frame(
                &decision,
                report,
                &outputs,
                &[Some(Cia402Target::Position(1))],
                &mut guards,
                &limits,
                &maps,
                &modes,
                &image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                150_000,
            ),
            Err(StopFrameError::Transmit(CycleError::Port(_)))
        ));
        assert_eq!(guards[0].last(), CyclicSetpoint::ZERO);
        assert_eq!(port.tx_frames(), tx_before);
        assert!(
            submit_active_frame(
                &decision,
                report,
                &outputs,
                &[Some(Cia402Target::Position(1))],
                &mut guards,
                &limits,
                &maps,
                &modes,
                &image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                150_000,
            )
            .is_ok()
        );
        assert_eq!(guards[0].last().position, 1.0);
        assert_eq!(port.tx_frames(), tx_before + 1);
        domain.begin_receive(2).unwrap();
        let second = receive(&mut master, &mut port, &mut domain, 2);
        assert_eq!(
            u16::from_le_bytes(domain.input()[16..18].try_into().unwrap()) & 0x000F,
            0x000F
        );
        assert_eq!(
            i32::from_le_bytes(domain.input()[19..23].try_into().unwrap()),
            1
        );

        second
    };
    let buffer = ProcBuf::<1, 0, 1, 8>::new(1, 7);
    let mut cursor = LifecycleEventCursor::new(&guard);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = second.cycle;
    port.set_now_ns(200_000);
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
    let mut enable_image = input_image(0x0023, 0);
    enable_image[5..9].copy_from_slice(&25i32.to_le_bytes());
    enable_image[19..23].copy_from_slice(&999i32.to_le_bytes());
    let mut running_image = input_image(0x0027, 0);
    running_image[5..9].copy_from_slice(&25i32.to_le_bytes());
    running_image[19..23].copy_from_slice(&999i32.to_le_bytes());
    let handshake = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: second,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &[1],
        safe_process_image: &enable_image,
        plan: &plan,
        next_generation: 3,
        deadline_ns: 250_000,
        now_ns: 201,
        transition_time_ns: 100,
    }
    .run_with_motion(&[None], &mut guards, &limits)
    .unwrap();
    assert_eq!(handshake.action, LifecycleAction::EnableAllowed);
    assert!(handshake.active_failure.is_none());
    assert!(handshake.transmission.is_ok());
    assert_eq!(handshake.state_publish, Ok(1));
    assert!(!guards[0].seeded());
    assert_eq!(guards[0].last().position, 0.0);
    assert_eq!(
        buffer.read_state().unwrap().state.axis_stops[0].request_cycle,
        0
    );

    domain.begin_receive(3).unwrap();
    let third = receive(&mut master, &mut port, &mut domain, 3);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = third.cycle;
    port.set_now_ns(300_000);
    {
        let decision = guard.cycle_axes(third.cycle, 301);
        let outputs = step_axis_bank(&mut bank, &decision, [0x0023], [DriveRequest::Enable]);
        port.fail_next_tx();
        assert!(matches!(
            submit_active_frame(
                &decision,
                third,
                &outputs,
                &[Some(Cia402Target::Position(25))],
                &mut guards,
                &limits,
                &maps,
                &modes,
                &enable_image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                4,
                350_000,
            ),
            Err(StopFrameError::Transmit(CycleError::Port(_)))
        ));
        assert!(!guards[0].seeded());
    }
    let enabled = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: third,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        plan: &plan,
        next_generation: 4,
        deadline_ns: 350_000,
        now_ns: 301,
        transition_time_ns: 100,
    }
    .run_with_motion(&[Some(Cia402Target::Position(25))], &mut guards, &limits)
    .unwrap();
    assert_eq!(enabled.action, LifecycleAction::EnableAllowed);
    assert!(enabled.transmission.is_ok());
    assert!(enabled.active_failure.is_none());
    assert_eq!(enabled.state_publish, Ok(2));
    assert!(guards[0].seeded());
    assert_eq!(guards[0].last().position, 25.0);
    domain.begin_receive(4).unwrap();
    let fourth = receive(&mut master, &mut port, &mut domain, 4);
    assert_eq!(
        u16::from_le_bytes(domain.input()[16..18].try_into().unwrap()) & 0x000F,
        0x000F
    );
    assert_eq!(domain.input()[19..23], 25i32.to_le_bytes());

    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = fourth.cycle;
    port.set_now_ns(400_000);
    guard
        .accept_permit(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 2,
                sequence: 2,
                expires_at_ns: 10_000,
                axis_mask: 1,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            401,
        )
        .unwrap();
    let running = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: fourth,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        plan: &plan,
        next_generation: 5,
        deadline_ns: 450_000,
        now_ns: 401,
        transition_time_ns: 100,
    }
    .run_with_motion(&[Some(Cia402Target::Position(26))], &mut guards, &limits)
    .unwrap();
    assert_eq!(running.action, LifecycleAction::EnableAllowed);
    assert!(running.transmission.is_ok());
    assert_eq!(guards[0].last().position, 26.0);

    domain.begin_receive(5).unwrap();
    let fifth = receive(&mut master, &mut port, &mut domain, 5);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = fifth.cycle;
    port.set_now_ns(500_000);
    let failed_motion = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: fifth,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        plan: &plan,
        next_generation: 6,
        deadline_ns: 550_000,
        now_ns: 501,
        transition_time_ns: 100,
    }
    .run_with_motion(&[Some(Cia402Target::Position(29))], &mut guards, &limits)
    .unwrap();
    assert!(matches!(failed_motion.action, LifecycleAction::Stop(_)));
    assert!(matches!(
        failed_motion.active_failure,
        Some(StopFrameError::InvalidTarget(
            0,
            CyclicSetpointError::PositionStepExceeded
        ))
    ));
    assert!(failed_motion.transmission.is_ok());
    assert_eq!(failed_motion.state_publish, Ok(4));
    assert_eq!(guard.state(), LifecycleState::Stopping);
    assert_eq!(guard.first_fault_code(), 0x5458_0001);
    assert!(guard.permit().is_none());
    assert_eq!(guards[0].last().position, 26.0);
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.sequence, fifth.cycle);
    assert_eq!(published.axis_stops[0].request_cycle, fifth.cycle);
    assert_ne!(published.axis_stops[0].issued_action, 0);
    assert!(matches!(failed_motion.event_publish, Some(Ok(_))));
}

#[test]
fn active_target_cannot_alias_an_unpermitted_axis_target() {
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
    let mut port = SimulatedPort::new(1);
    let mut image = [0; 64];
    image[..IMAGE_BYTES].copy_from_slice(&input_image(0x0027, 0));
    image[IMAGE_BYTES..].copy_from_slice(&input_image(0x0040, 0));
    submit(&mut master, &mut port, &mut domain, &plan, 1, &image, false);
    let report = receive(&mut master, &mut port, &mut domain, 1);
    let mut second_map = map_at(256);
    let prior = second_map.entry(Cia402PdoField::TargetPosition).unwrap();
    second_map.set_entry(
        Cia402PdoField::TargetPosition,
        PdoEntry {
            bit_offset: 152,
            ..prior
        },
    );
    second_map.validate_for(OperatingMode::Csp).unwrap();
    let maps = [map(), second_map];
    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 0b01,
            ..GuardPolicy::conservative()
        },
    );
    let dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x2000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    guard.update_cyclic_quality(
        cyclic_quality_from_ethercat(
            report,
            &[domain.quality()],
            &dc,
            OtherCycleFacts {
                platform_ready: true,
                coe_ready: true,
                topology_valid: true,
                drive_ready: true,
                command_current: true,
                supervisor_healthy: true,
                external_safety_clear: true,
                deadline_met: true,
            },
        ),
        report.cycle,
    );
    guard
        .request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                expires_at_ns: 10_000,
                axis_mask: 0b01,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            report.cycle,
            100,
        )
        .unwrap();
    let decision = guard.cycle_axes(report.cycle, 101);
    let mut bank = Cia402AxisBank::<2>::new();
    let outputs = step_axis_bank(
        &mut bank,
        &decision,
        [0x0027, 0x0040],
        [DriveRequest::Enable; 2],
    );
    let mut guards = [CyclicSetpointGuard::new(); 2];
    let limits = [CyclicLimits {
        max_position_step: 2.0,
        max_velocity: 2.0,
        max_torque: 2.0,
    }; 2];
    let tx_before = port.tx_frames();
    assert!(matches!(
        submit_active_frame(
            &decision,
            report,
            &outputs,
            &[Some(Cia402Target::Position(1)), None],
            &mut guards,
            &limits,
            &maps,
            &[OperatingMode::Csp; 2],
            &image,
            &domain,
            &plan,
            &mut master,
            &mut port,
            2,
            150_000,
        ),
        Err(StopFrameError::OverlappingOutput(0, 1))
    ));
    assert_eq!(port.tx_frames(), tx_before);
    assert!(!guards[0].seeded());
}

#[test]
fn stop_cycle_owner_publishes_failed_tx_then_qualifies_a_later_response() {
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
    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    let mut cursor = LifecycleEventCursor::new(&guard);
    let buffer = ProcBuf::<1, 0, 1, 8>::new(1, 7);
    let mut bank = Cia402AxisBank::<1>::new();
    let maps = [map()];
    let modes = [OperatingMode::Csp];
    let max_stationary_velocities = [1];
    let safe_image = input_image(0x0040, 0);
    let mut port = SimulatedPort::new(1);

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
    guard.update_cyclic_quality(
        cyclic_quality_from_ethercat(first, &[domain.quality()], &dc, other),
        first.cycle,
    );
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

    let tx_before = port.tx_frames();
    {
        let decision = guard.cycle_axes(first.cycle, 101);
        let outputs = step_axis_bank(&mut bank, &decision, [0x0027], [DriveRequest::Disable]);
        assert!(matches!(
            submit_inhibited_frame(
                &decision,
                &outputs,
                &maps,
                &modes,
                &safe_image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                250_000,
            ),
            Err(StopFrameError::InvalidDecision)
        ));
    }
    let mut active_state = StatePage::<1, 0, 1>::new(7);
    active_state.sequence = first.cycle;
    let active = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut active_state,
        report: first,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 2,
        deadline_ns: 250_000,
        now_ns: 101,
        transition_time_ns: 100,
    }
    .run();
    assert!(matches!(
        active,
        Err(StopCycleError::NotStopping(LifecycleAction::EnableAllowed))
    ));
    assert_eq!(port.tx_frames(), tx_before);
    assert_eq!(buffer.pending_events(), 0);

    port.set_response_wkc(0);
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
    port.set_response_wkc(1);
    port.set_now_ns(300_000);
    let mut mismatched_guard = LifecycleGuard::new(
        GateId::Domain.bit(),
        7,
        GuardPolicy {
            allowed_axis_mask: 0b10,
            ..GuardPolicy::conservative()
        },
    );
    let mut mismatched_cursor = LifecycleEventCursor::new(&mismatched_guard);
    let mut rejected_page = StatePage::<1, 0, 1>::new(7);
    rejected_page.sequence = second.cycle;
    assert!(matches!(
        (StopCycleContext {
            guard: &mut mismatched_guard,
            bank: &mut bank,
            master: &mut master,
            port: &mut port,
            domain: &domain,
            dc: &dc,
            buffer: &buffer,
            event_cursor: &mut mismatched_cursor,
            state: &mut rejected_page,
            report: second,
            other,
            maps: &maps,
            modes: &modes,
            max_stationary_velocities: &max_stationary_velocities,
            safe_process_image: &safe_image,
            plan: &plan,
            next_generation: 3,
            deadline_ns: 350_000,
            now_ns: 200,
            transition_time_ns: 200,
        })
        .run(),
        Err(StopCycleError::AxisCapacityExceeded)
    ));
    assert_eq!(rejected_page.lifecycle.state, 0);
    assert_eq!(buffer.pending_events(), 0);

    port.fail_next_tx();
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = second.cycle;
    let failed = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: second,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 3,
        deadline_ns: 350_000,
        now_ns: 200,
        transition_time_ns: 200,
    }
    .run()
    .unwrap();
    assert!(!failed.quality.domain_valid);
    assert!(matches!(
        failed.transmission,
        Err(StopFrameError::Transmit(CycleError::Port(_)))
    ));
    assert_eq!(failed.state_publish, Ok(1));
    assert_eq!(failed.event_publish, Some(Ok(2)));
    assert!(!failed.acknowledged);
    assert_eq!(guard.state(), LifecycleState::Stopping);
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.axis_stops[0].issued_action, 0);
    assert_eq!(published.lifecycle.transition_time_ns, 200);
    assert_eq!(buffer.pop_event().unwrap().sequence, 1);
    assert_eq!(buffer.pop_event().unwrap().sequence, 2);

    domain.begin_receive(3).unwrap();
    let third = receive(&mut master, &mut port, &mut domain, 3);
    port.set_now_ns(400_000);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = third.cycle;
    let sent = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: third,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 4,
        deadline_ns: 450_000,
        now_ns: 300,
        transition_time_ns: 200,
    }
    .run()
    .unwrap();
    assert!(sent.transmission.is_ok());
    assert_eq!(sent.feedback, None);
    assert!(!sent.acknowledged);
    assert_eq!(sent.state_publish, Ok(2));
    assert_eq!(sent.event_publish, Some(Ok(0)));
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.sequence, 3);
    assert_eq!(published.axis_stops[0].issued_action, 3);
    assert_eq!(published.axis_stops[0].feedback_valid, 0);
    assert_eq!(published.quality.domains[0].valid, 0);
    assert_eq!(published.lifecycle.transition_time_ns, 200);

    domain.begin_receive(4).unwrap();
    let fourth = receive(&mut master, &mut port, &mut domain, 4);
    port.set_now_ns(500_000);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = fourth.cycle;
    let complete = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: fourth,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 5,
        deadline_ns: 550_000,
        now_ns: 400,
        transition_time_ns: 400,
    }
    .run()
    .unwrap();
    assert!(complete.transmission.is_ok());
    assert!(complete.acknowledged);
    assert_eq!(complete.state_publish, Ok(3));
    assert_eq!(complete.event_publish, Some(Ok(1)));
    assert_eq!(guard.state(), LifecycleState::Ready);
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.sequence, fourth.cycle);
    assert_eq!(published.lifecycle.state, LifecycleState::Ready as u8);
    assert_eq!(published.axis_stops[0].feedback_valid, 1);
    assert_eq!(published.axis_stops[0].stationary, 1);
    assert_eq!(published.axis_stops[0].non_enabled, 1);
    assert_eq!(published.lifecycle.transition_time_ns, 400);
    assert_eq!(
        buffer.pop_event().unwrap().sequence,
        published.lifecycle.transition_sequence
    );
    assert_eq!(buffer.pop_event(), None);

    // A fabricated next-cycle report must not advance the guard or publish.
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = fourth.cycle + 1;
    let forged = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: CycleReport {
            cycle: fourth.cycle + 1,
            ..fourth
        },
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 6,
        deadline_ns: 650_000,
        now_ns: 500,
        transition_time_ns: 400,
    }
    .run();
    assert!(matches!(forged, Err(StopCycleError::CycleMismatch)));
    assert_eq!(guard.state(), LifecycleState::Ready);
    assert_eq!(state.sequence, fourth.cycle + 1);
    assert_eq!(state.lifecycle.state, 0);

    // A real ready cycle emits a Disable-only frame and clears stop evidence.
    domain.begin_receive(5).unwrap();
    let fifth = receive(&mut master, &mut port, &mut domain, 5);
    port.set_now_ns(600_000);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = fifth.cycle;
    let inactive = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: fifth,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 6,
        deadline_ns: 650_000,
        now_ns: 500,
        transition_time_ns: 400,
    }
    .run()
    .unwrap();
    assert_eq!(inactive.action, LifecycleAction::Hold);
    assert!(inactive.transmission.is_ok());
    assert_eq!(inactive.state_publish, Ok(4));
    assert_eq!(inactive.event_publish, Some(Ok(1)));
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.lifecycle.state, LifecycleState::Qualifying as u8);
    assert_eq!(published.axis_stops[0].issued_action, 0);
    assert_eq!(published.axis_stops[0].feedback_valid, 0);
    assert_eq!(
        buffer.pop_event().unwrap().sequence,
        published.lifecycle.transition_sequence
    );
    domain.begin_receive(6).unwrap();
    receive(&mut master, &mut port, &mut domain, 6);
    assert_eq!(
        maps[0]
            .entry(Cia402PdoField::Controlword)
            .unwrap()
            .read_unsigned(domain.input()),
        Ok(u64::from(CONTROLWORD_DISABLE_VOLTAGE))
    );
    assert_eq!(domain.input()[19..23], [0; 4]);
}

#[test]
fn stop_timeout_latches_and_still_sends_disable_with_ordered_events() {
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
    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            stop_timeout_cycles: 2,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    let mut cursor = LifecycleEventCursor::new(&guard);
    let buffer = ProcBuf::<1, 0, 1, 8>::new(1, 7);
    let mut bank = Cia402AxisBank::<1>::new();
    let maps = [map()];
    let modes = [OperatingMode::Csp];
    let thresholds = [1];
    let safe_image = input_image(0x0040, 0);
    let mut port = SimulatedPort::new(1);

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
    guard.update_cyclic_quality(
        cyclic_quality_from_ethercat(first, &[domain.quality()], &dc, other),
        first.cycle,
    );
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

    port.set_response_wkc(0);
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
    port.set_response_wkc(1);
    port.set_now_ns(300_000);
    port.fail_next_tx();
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = second.cycle;
    let failed = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: second,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 3,
        deadline_ns: 350_000,
        now_ns: 200,
        transition_time_ns: 200,
    }
    .run()
    .unwrap();
    assert!(matches!(
        failed.transmission,
        Err(StopFrameError::Transmit(_))
    ));
    assert_eq!(failed.state_publish, Ok(1));
    assert_eq!(
        buffer.read_state().unwrap().state.axis_stops[0].issued_action,
        0
    );
    while buffer.pop_event().is_some() {}

    domain.begin_receive(3).unwrap();
    let third = receive(&mut master, &mut port, &mut domain, 3);
    port.set_now_ns(400_000);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = third.cycle;
    let issued = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: third,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 4,
        deadline_ns: 450_000,
        now_ns: 300,
        transition_time_ns: 200,
    }
    .run()
    .unwrap();
    assert!(issued.transmission.is_ok());
    assert_eq!(
        buffer.read_state().unwrap().state.axis_stops[0].issued_action,
        3
    );
    assert_eq!(buffer.pop_event(), None);

    domain.begin_receive(4).unwrap();
    let fourth = receive(&mut master, &mut port, &mut domain, 4);
    port.set_now_ns(500_000);
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = fourth.cycle;
    let latched = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: fourth,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 5,
        deadline_ns: 550_000,
        now_ns: 400,
        transition_time_ns: 200,
    }
    .run()
    .unwrap();
    assert_eq!(latched.action, LifecycleAction::FaultLatched);
    assert!(latched.transmission.is_ok());
    assert_eq!(latched.feedback, None);
    assert!(!latched.acknowledged);
    assert_eq!(latched.state_publish, Ok(3));
    assert_eq!(latched.event_publish, Some(Ok(2)));
    let published = buffer.read_state().unwrap().state;
    assert_eq!(
        published.lifecycle.state,
        LifecycleState::FaultLatched as u8
    );
    assert_eq!(published.lifecycle.transition_time_ns, 400);
    assert_eq!(published.axis_stops[0].requested_action, 0);
    assert_eq!(published.axis_stops[0].issued_action, 0);
    let transition = buffer.pop_event().unwrap();
    let timeout = buffer.pop_event().unwrap();
    assert_eq!(transition.code, 2);
    assert_eq!(timeout.code, 1);
    assert_eq!(timeout.axis_or_device, 0);
    assert_eq!(timeout.sequence, transition.sequence);
    assert_ne!(timeout.aux & (1 << 16), 0);
    assert_eq!(buffer.pop_event(), None);

    domain.begin_receive(5).unwrap();
    let fifth = receive(&mut master, &mut port, &mut domain, 5);
    assert_eq!(
        maps[0]
            .entry(Cia402PdoField::Controlword)
            .unwrap()
            .read_unsigned(domain.input()),
        Ok(u64::from(CONTROLWORD_DISABLE_VOLTAGE))
    );

    port.set_now_ns(600_000);
    port.fail_next_tx();
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = fifth.cycle;
    let failed_inhibit = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: fifth,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        plan: &plan,
        next_generation: 6,
        deadline_ns: 650_000,
        now_ns: 500,
        transition_time_ns: 400,
    }
    .run()
    .unwrap();
    assert_eq!(failed_inhibit.action, LifecycleAction::FaultLatched);
    assert!(matches!(
        failed_inhibit.transmission,
        Err(StopFrameError::Transmit(_))
    ));
    assert_eq!(failed_inhibit.state_publish, Ok(4));
    assert_eq!(failed_inhibit.event_publish, Some(Ok(0)));
    assert_eq!(
        buffer.read_state().unwrap().state.axis_stops[0].issued_action,
        0
    );
    assert_eq!(buffer.pop_event(), None);
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
fn scheduled_cycle_stops_when_another_due_domain_misses_its_receive() {
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut motion = Domain::<IMAGE_BYTES, 1>::new(0x1000);
    motion
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: IMAGE_BYTES,
            expected_wkc: 1,
        })
        .unwrap();
    let mut auxiliary = Domain::<2, 1>::new(0x2000);
    auxiliary
        .add_segment(DomainSegment {
            datagram_index: 13,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let motion_datagram = DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: IMAGE_BYTES,
        expected_wkc: 1,
    };
    let mut initial_plan = FramePlan::<2>::new();
    initial_plan.push(motion_datagram).unwrap();
    initial_plan
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 13,
            address: 0x2000,
            payload_offset: IMAGE_BYTES,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut motion_plan = FramePlan::<1>::new();
    motion_plan.push(motion_datagram).unwrap();
    let schedule = ScheduleTable::<2, 2>::build(
        100_000,
        &[
            ScheduleDomain {
                id: 9,
                period_ticks: 1,
                phase_ticks: 0,
            },
            ScheduleDomain {
                id: 10,
                period_ticks: 2,
                phase_ticks: 0,
            },
        ],
    )
    .unwrap();
    let dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 14, 0),
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
    let mut mux = RxConsumerMux::new(motion, auxiliary);
    let mut port = SimulatedPort::new(1);
    let safe_image = input_image(0x0040, 0);
    let mut initial_image = [0u8; IMAGE_BYTES + 2];
    initial_image[..IMAGE_BYTES].copy_from_slice(&safe_image);
    initial_image[IMAGE_BYTES..].copy_from_slice(&[0xAB, 0xCD]);
    port.set_now_ns(100_000);
    mux.first_mut().begin_receive(1).unwrap();
    mux.second_mut().begin_receive(1).unwrap();
    let frame = master.acquire_frame(1, 150_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &initial_plan, &initial_image)
        .unwrap();
    master.submit_frame(&mut port, frame).unwrap();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let first = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 1, &mut mux)
        .unwrap();
    assert!(mux.first_mut().finish_receive(1, first.cycle).unwrap());
    assert!(mux.second_mut().finish_receive(1, first.cycle).unwrap());
    assert_eq!(mux.second().input(), &[0xAB, 0xCD]);
    let mut snapshots = [
        ScheduledDomainQuality {
            id: 9,
            quality: mux.first().quality(),
        },
        ScheduledDomainQuality {
            id: 10,
            quality: mux.second().quality(),
        },
    ];

    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    guard.update_cyclic_quality(
        cyclic_quality_from_schedule(first, &schedule, &snapshots, &dc, other),
        first.cycle,
    );
    guard
        .request_rearm(
            MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                expires_at_ns: 400_000,
                axis_mask: 1,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            },
            first.cycle,
            100_000,
        )
        .unwrap();
    let mut cursor = LifecycleEventCursor::new(&guard);
    let buffer = ProcBuf::<1, 0, 2, 8>::new(1, 7);
    let mut bank = Cia402AxisBank::<1>::new();
    let maps = [map()];
    let modes = [OperatingMode::Csp];
    let limits = [CyclicLimits {
        max_position_step: 2.0,
        max_velocity: 2.0,
        max_torque: 2.0,
    }];
    let mut guards = [CyclicSetpointGuard::new()];
    let mut state = StatePage::<1, 0, 2>::new(7);
    state.sequence = first.cycle;
    {
        let mut context = StopCycleContext {
            guard: &mut guard,
            bank: &mut bank,
            master: &mut master,
            port: &mut port,
            domain: mux.first(),
            dc: &dc,
            buffer: &buffer,
            event_cursor: &mut cursor,
            state: &mut state,
            report: first,
            other,
            maps: &maps,
            modes: &modes,
            max_stationary_velocities: &[1],
            safe_process_image: &safe_image,
            plan: &motion_plan,
            next_generation: 2,
            deadline_ns: 250_000,
            now_ns: 100_000,
            transition_time_ns: 100_000,
        };
        assert!(matches!(
            context.run_with_motion(&[None], &mut guards, &limits),
            Err(StopCycleError::ScheduleRequired)
        ));
        let reversed = [snapshots[1], snapshots[0]];
        assert!(matches!(
            context.run_scheduled_with_motion(
                &schedule,
                &reversed,
                9,
                &[None],
                &mut guards,
                &limits,
            ),
            Err(StopCycleError::InvalidSchedule)
        ));
        let mut forged = snapshots;
        forged[0].quality.actual_wkc = 0;
        assert!(matches!(
            context
                .run_scheduled_with_motion(&schedule, &forged, 9, &[None], &mut guards, &limits,),
            Err(StopCycleError::MotionDomainMismatch)
        ));
        let sparse = ScheduleTable::<2, 2>::build(
            100_000,
            &[ScheduleDomain {
                id: 9,
                period_ticks: 1,
                phase_ticks: 0,
            }],
        )
        .unwrap();
        assert!(matches!(
            context.run_scheduled_with_motion(
                &sparse,
                &snapshots,
                9,
                &[None],
                &mut guards,
                &limits,
            ),
            Err(StopCycleError::InvalidSchedule)
        ));
        let slow_motion = ScheduleTable::<2, 2>::build(
            100_000,
            &[
                ScheduleDomain {
                    id: 9,
                    period_ticks: 2,
                    phase_ticks: 0,
                },
                ScheduleDomain {
                    id: 10,
                    period_ticks: 1,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        assert!(matches!(
            context.run_scheduled_with_motion(
                &slow_motion,
                &snapshots,
                9,
                &[None],
                &mut guards,
                &limits,
            ),
            Err(StopCycleError::MotionDomainMismatch)
        ));
        assert_eq!(context.state.quality.sequence, 0);
        assert_eq!(context.port.tx_frames(), 1);

        let sent = context
            .run_scheduled_with_motion_until(
                &schedule,
                &snapshots,
                9,
                &[None],
                &mut guards,
                &limits,
                150_000,
            )
            .unwrap();
        assert_eq!(sent.action, LifecycleAction::EnableAllowed);
        assert!(sent.transmission.is_ok());
        assert!(sent.quality.domain_valid);
        assert_eq!(sent.post_tx_deadline_met, Some(true));
        assert_eq!(sent.state_publish, Ok(1));
    }

    port.set_now_ns(200_000);
    mux.first_mut().begin_receive(2).unwrap();
    let second = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 2, &mut mux)
        .unwrap();
    assert!(mux.first_mut().finish_receive(2, second.cycle).unwrap());
    snapshots[0].quality = mux.first().quality();
    snapshots[1].quality = mux.second().quality();
    let mut state = StatePage::<1, 0, 2>::new(7);
    state.sequence = second.cycle;
    let idle = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: mux.first(),
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: second,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &[1],
        safe_process_image: &safe_image,
        plan: &motion_plan,
        next_generation: 3,
        deadline_ns: 350_000,
        now_ns: 200_000,
        transition_time_ns: 100_000,
    }
    .run_scheduled_with_motion_until(
        &schedule,
        &snapshots,
        9,
        &[None],
        &mut guards,
        &limits,
        250_000,
    )
    .unwrap();
    assert_eq!(idle.action, LifecycleAction::EnableAllowed);
    assert!(idle.transmission.is_ok());
    assert!(idle.quality.domain_valid);
    assert_eq!(idle.post_tx_deadline_met, Some(true));
    assert_eq!(
        buffer.read_state().unwrap().state.quality.domains[1].input_age_cycles,
        1
    );

    port.set_now_ns(300_000);
    mux.first_mut().begin_receive(3).unwrap();
    let third = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 3, &mut mux)
        .unwrap();
    assert!(mux.first_mut().finish_receive(3, third.cycle).unwrap());
    snapshots[0].quality = mux.first().quality();
    snapshots[1].quality = mux.second().quality();
    let mut state = StatePage::<1, 0, 2>::new(7);
    state.sequence = third.cycle;
    let failed = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: mux.first(),
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: third,
        other,
        maps: &maps,
        modes: &modes,
        max_stationary_velocities: &[1],
        safe_process_image: &safe_image,
        plan: &motion_plan,
        next_generation: 4,
        deadline_ns: 450_000,
        now_ns: 300_000,
        transition_time_ns: 100_000,
    }
    .run_scheduled_with_motion_until(
        &schedule,
        &snapshots,
        9,
        &[None],
        &mut guards,
        &limits,
        350_000,
    )
    .unwrap();
    assert!(matches!(failed.action, LifecycleAction::Stop(_)));
    assert!(failed.transmission.is_ok());
    assert!(!failed.quality.domain_valid);
    assert_eq!(failed.post_tx_deadline_met, Some(true));
    assert!(guard.permit().is_none());
    assert_eq!(guard.state(), LifecycleState::Stopping);
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.quality.domains[1].input_age_cycles, 2);
    assert_ne!(published.axis_stops[0].issued_action, 0);
    assert_eq!(published.lifecycle.first_blocking_code, 0x444F_0001);
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
