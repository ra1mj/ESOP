use esop_ethercat_core::wire::{Command, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    DatagramPlan, DcCyclicConfig, DcCyclicSync, DcMonitor, Domain, DomainSegment, EthercatMaster,
    FramePlan, MasterConfig, RxConsumerMux, ScheduleDomain, ScheduleTable,
};
use esop_ethercat_linux_port::SimulatedPort;
use esop_lifecycle_guard::ethercat::{OtherCycleFacts, ScheduledDomainQuality};
use esop_lifecycle_guard::procbuf::{
    ethercat_cycle_to_procbuf, lifecycle_to_procbuf, scheduled_ethercat_cycle_to_procbuf,
};
use esop_lifecycle_guard::{
    GateId, GuardPolicy, LifecycleAction, LifecycleGuard, MotionPermit, StopAction,
};
use esop_procbuf::{ProcBuf, QualityFact, StatePage};

#[test]
fn received_domain_and_dc_drive_guard_and_published_quality() {
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut domain = Domain::<2, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let sync = DcCyclicSync::new(
        DcCyclicConfig::new(0x1000, 13, 2),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut plan = FramePlan::<2>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();
    plan.push(sync.datagram_plan()).unwrap();
    let mut mux = RxConsumerMux::new(domain, sync);
    let mut port = SimulatedPort::new(1);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut image = [0; 10];

    let required = GateId::Domain.bit()
        | GateId::Link.bit()
        | GateId::DistributedClock.bit()
        | GateId::Budget.bit();
    let policy = GuardPolicy {
        enter_good_cycles: 1,
        exit_bad_cycles: 2,
        max_age_cycles: 1,
        stop_action: StopAction::QuickStop,
        ..GuardPolicy::conservative()
    };
    let mut guard = LifecycleGuard::new(required, 7, policy);
    let buffer = ProcBuf::<1, 0, 1, 2>::new(1, 7);
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

    for (generation, application_time) in [(7, 100), (8, 101)] {
        if generation == 8 {
            port.set_response_wkc(0);
        }
        mux.first_mut().begin_receive(generation).unwrap();
        mux.second_mut()
            .prepare(generation, application_time, &mut image)
            .unwrap();
        let frame = master.acquire_frame(generation, 100_000).unwrap();
        master
            .build_and_arm_frame_from_plan(frame, &plan, &image)
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        let report = master
            .cycle_receive_with_consumer(&mut port, &mut scratch, generation, &mut mux)
            .unwrap();
        let completed = mux
            .first_mut()
            .finish_receive(generation, report.cycle)
            .unwrap();
        assert_eq!(completed, generation == 7);
        let mut state = StatePage::<1, 0, 1>::new(7);
        state.sequence = report.cycle;
        let facts = ethercat_cycle_to_procbuf(
            &mut state,
            report,
            &[mux.first().quality()],
            &[true],
            mux.second(),
            other,
        );
        guard.update_cyclic_quality(facts, report.cycle);

        let action = if generation == 7 {
            let permit = MotionPermit {
                boot_id: 7,
                source_id: 1,
                permit_epoch: 1,
                sequence: 1,
                expires_at_ns: 1_000,
                axis_mask: 1,
                authority: 1,
                reserved: [0; 3],
                policy_version: 1,
            };
            assert_eq!(
                guard.request_rearm(permit, report.cycle, 10),
                Ok(LifecycleAction::EnableAllowed)
            );
            guard.cycle(report.cycle, 10)
        } else {
            guard.cycle(report.cycle, 10)
        };

        state.lifecycle = lifecycle_to_procbuf(guard.snapshot(report.cycle, 10), report.cycle);
        buffer.publish_state(state).unwrap();
        let published = buffer.read_state().unwrap().state;
        assert_eq!(published.sequence, published.quality.sequence);
        assert!(published.quality.cyclic.complete());
        assert_eq!(published.quality.link_up, 1);
        assert_eq!(
            published.quality.dc_locked,
            facts.distributed_clock_locked as u8
        );
        assert_eq!(published.quality.domains[0].expected_wkc, 1);
        assert_eq!(
            published.quality.domains[0].actual_wkc,
            u16::from(generation == 7)
        );
        assert_eq!(published.quality.domains[0].valid, facts.domain_valid as u8);
        if generation == 7 {
            assert_eq!(action, LifecycleAction::EnableAllowed);
            assert!(published.quality.cyclic.good(QualityFact::Domain));
            assert!(published.quality.cyclic.good(QualityFact::Wkc));
            assert!(published.quality.cyclic.good(QualityFact::DistributedClock));
            assert_eq!(published.lifecycle.gates_ready, 1);
            assert_eq!(published.quality.domains[0].input_age_cycles, 0);
        } else {
            assert_eq!(action, LifecycleAction::Stop(StopAction::QuickStop));
            assert!(!published.quality.cyclic.good(QualityFact::Domain));
            assert!(!published.quality.cyclic.good(QualityFact::Wkc));
            assert!(!published.quality.cyclic.good(QualityFact::DistributedClock));
            assert_eq!(published.lifecycle.gates_ready, 0);
            assert_eq!(published.lifecycle.first_blocking_code, 0x4443_0001);
            assert_eq!(published.quality.domains[0].input_age_cycles, 1);
        }
    }
}

#[test]
fn multi_rate_idle_tick_keeps_motion_but_missing_due_domain_stops_it() {
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut domain = Domain::<2, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x1000, 13, 2),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut with_domain = FramePlan::<2>::new();
    with_domain
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 12,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    with_domain.push(dc.datagram_plan()).unwrap();
    let mut dc_only = FramePlan::<2>::new();
    dc_only.push(dc.datagram_plan()).unwrap();
    let schedule = ScheduleTable::<1, 2>::build(
        250_000,
        &[ScheduleDomain {
            id: 9,
            period_ticks: 2,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut mux = RxConsumerMux::new(domain, dc);
    let mut port = SimulatedPort::new(1);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut image = [0; 10];
    let required = GateId::Domain.bit() | GateId::Link.bit() | GateId::DistributedClock.bit();
    let mut guard = LifecycleGuard::new(
        required,
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            exit_bad_cycles: 2,
            max_age_cycles: 1,
            stop_action: StopAction::QuickStop,
            ..GuardPolicy::conservative()
        },
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
    for tick in 0..3u64 {
        let generation = (tick + 1) as u16;
        // Simulate a missing Domain on tick 2: only the DC datagram arrives.
        if tick != 1 {
            mux.first_mut().begin_receive(generation).unwrap();
        }
        mux.second_mut()
            .prepare(generation, 100 + tick, &mut image)
            .unwrap();
        let frame = master.acquire_frame(generation, 100_000).unwrap();
        master
            .build_and_arm_frame_from_plan(
                frame,
                if tick == 0 { &with_domain } else { &dc_only },
                &image,
            )
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        let report = master
            .cycle_receive_with_consumer(&mut port, &mut scratch, generation, &mut mux)
            .unwrap();
        if tick != 1 {
            assert_eq!(
                mux.first_mut()
                    .finish_receive(generation, report.cycle)
                    .unwrap(),
                tick == 0
            );
        }
        let mut state = StatePage::<1, 0, 1>::new(7);
        state.sequence = report.cycle;
        let facts = scheduled_ethercat_cycle_to_procbuf(
            &mut state,
            report,
            &schedule,
            &[ScheduledDomainQuality {
                id: 9,
                quality: mux.first().quality(),
            }],
            mux.second(),
            other,
        );
        guard.update_cyclic_quality(facts, report.cycle);
        let action = if tick == 0 {
            assert_eq!(
                guard.request_rearm(
                    MotionPermit {
                        boot_id: 7,
                        source_id: 1,
                        permit_epoch: 1,
                        sequence: 1,
                        expires_at_ns: 1_000,
                        axis_mask: 1,
                        authority: 1,
                        reserved: [0; 3],
                        policy_version: 1,
                    },
                    report.cycle,
                    10
                ),
                Ok(LifecycleAction::EnableAllowed)
            );
            guard.cycle(report.cycle, 10)
        } else {
            guard.cycle(report.cycle, 10)
        };
        assert_eq!(state.quality.sequence, report.cycle);
        assert_eq!(state.quality.domains[0].input_age_cycles, tick);
        if tick == 2 {
            assert!(!facts.domain_valid && !facts.wkc_valid);
            assert_eq!(action, LifecycleAction::Stop(StopAction::QuickStop));
            assert!(!state.quality.cyclic.good(QualityFact::Domain));
        } else {
            assert!(facts.domain_valid && facts.wkc_valid && facts.distributed_clock_locked);
            assert_eq!(action, LifecycleAction::EnableAllowed);
            assert!(state.quality.cyclic.good(QualityFact::Domain));
        }
    }
}
