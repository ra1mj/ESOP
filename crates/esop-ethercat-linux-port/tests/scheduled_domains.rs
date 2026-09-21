use esop_ethercat_core::wire::{Command, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    CycleError, DatagramPlan, DcCyclicConfig, DcCyclicError, DcCyclicSync, DcMonitor, Domain,
    DomainSegment, EthercatMaster, EthercatPort, FramePlan, LinkState, MasterConfig, PortError,
    RxPoll, ScheduleDomain, ScheduleTable, ScheduledDomainBank, ScheduledDomainEntry,
    ScheduledReceiveError,
};
use esop_ethercat_linux_port::SimulatedPort;
use esop_lifecycle_guard::ethercat::{
    OtherCycleFacts, ScheduledDomainQuality, cyclic_quality_from_schedule,
};
use esop_lifecycle_guard::{GateId, GuardPolicy, LifecycleAction, LifecycleGuard, MotionPermit};

#[test]
fn real_multi_domain_receive_quality_revokes_motion_on_a_missing_due_sample() {
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
    let mut motion = Domain::<2, 1>::new(0x1000);
    let mut auxiliary = Domain::<1, 1>::new(0x2000);
    motion
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    auxiliary
        .add_segment(DomainSegment {
            datagram_index: 13,
            input_offset: 0,
            len: 1,
            expected_wkc: 1,
        })
        .unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [
            ScheduledDomainEntry {
                id: 9,
                domain: &mut motion,
            },
            ScheduledDomainEntry {
                id: 10,
                domain: &mut auxiliary,
            },
        ],
    )
    .unwrap();
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut initial = FramePlan::<2>::new();
    initial
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 12,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut motion_only = FramePlan::<2>::new();
    motion_only.push(initial.datagrams()[0]).unwrap();
    initial
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 13,
            address: 0x2000,
            payload_offset: 2,
            payload_len: 1,
            expected_wkc: 1,
        })
        .unwrap();
    let mut port = SimulatedPort::new(1);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let image = [0x40, 0x00, 0xAA];
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
    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    for generation in 1..=3u16 {
        port.set_now_ns(u64::from(generation) * 100_000);
        bank.begin_due(u64::from(generation), generation).unwrap();
        let frame = master
            .acquire_frame(generation, port.now_ns_value() + 50_000)
            .unwrap();
        master
            .build_and_arm_frame_from_plan(
                frame,
                if generation == 1 {
                    &initial
                } else {
                    &motion_only
                },
                &image,
            )
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        let report = master
            .cycle_receive_with_consumer(&mut port, &mut scratch, generation, &mut bank)
            .unwrap();
        let qualities = bank.finish_due(report.cycle, generation).unwrap();
        let snapshots = [
            ScheduledDomainQuality {
                id: 9,
                quality: qualities[0],
            },
            ScheduledDomainQuality {
                id: 10,
                quality: qualities[1],
            },
        ];
        let facts = cyclic_quality_from_schedule(report, &schedule, &snapshots, &dc, other);
        guard.update_cyclic_quality(facts, report.cycle);
        if generation == 1 {
            guard
                .request_rearm(
                    MotionPermit {
                        boot_id: 7,
                        source_id: 1,
                        permit_epoch: 1,
                        sequence: 1,
                        expires_at_ns: 500_000,
                        axis_mask: 1,
                        authority: 1,
                        reserved: [0; 3],
                        policy_version: 1,
                    },
                    report.cycle,
                    port.now_ns_value(),
                )
                .unwrap();
        }
        let action = guard.cycle(report.cycle, port.now_ns_value());
        if generation == 3 {
            assert!(!facts.domain_valid && !facts.wkc_valid);
            assert!(matches!(action, LifecycleAction::Stop(_)));
            assert!(guard.permit().is_none());
            assert_eq!(qualities[1].last_valid_cycle, 1);
        } else {
            assert!(facts.domain_valid && facts.wkc_valid);
            assert_eq!(action, LifecycleAction::EnableAllowed);
        }
    }
}

#[test]
fn scheduled_rx_dispatches_dc_and_retires_missing_sync_without_reusing_old_lock() {
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
    let mut motion = Domain::<2, 1>::new(0x1000);
    motion
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut auxiliary = Domain::<1, 1>::new(0x2000);
    auxiliary
        .add_segment(DomainSegment {
            datagram_index: 13,
            input_offset: 0,
            len: 1,
            expected_wkc: 1,
        })
        .unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [
            ScheduledDomainEntry {
                id: 9,
                domain: &mut motion,
            },
            ScheduledDomainEntry {
                id: 10,
                domain: &mut auxiliary,
            },
        ],
    )
    .unwrap();
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 14, 3),
        DcMonitor::new(50, 10, 1, 2),
    );
    let motion_datagram = DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 2,
        expected_wkc: 1,
    };
    let mut combined = FramePlan::<3>::new();
    combined.push(motion_datagram).unwrap();
    combined
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 13,
            address: 0x2000,
            payload_offset: 2,
            payload_len: 1,
            expected_wkc: 1,
        })
        .unwrap();
    combined.push(dc.datagram_plan()).unwrap();
    let mut motion_only = FramePlan::<1>::new();
    motion_only.push(motion_datagram).unwrap();
    let mut dc_only = FramePlan::<1>::new();
    dc_only.push(dc.datagram_plan()).unwrap();
    let mut port = SimulatedPort::new(1);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut image = [0u8; 11];
    image[..3].copy_from_slice(&[0x40, 0, 0xAA]);
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
        GateId::Domain.bit() | GateId::Link.bit() | GateId::DistributedClock.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    for generation in 1..=3u16 {
        let now_ns = u64::from(generation) * 100_000;
        port.set_now_ns(now_ns);
        master.reap_expired_rx_before_tx(now_ns);
        dc.prepare(generation, now_ns, &mut image).unwrap();
        if generation == 2 {
            let frame = master.acquire_frame(generation, now_ns + 50_000).unwrap();
            master
                .build_and_arm_frame_from_plan(frame, &dc_only, &image)
                .unwrap();
            port.drop_next_response();
            master.submit_frame(&mut port, frame).unwrap();
            let frame = master.acquire_frame(generation, now_ns + 50_000).unwrap();
            master
                .build_and_arm_frame_from_plan(frame, &motion_only, &image)
                .unwrap();
            master.submit_frame(&mut port, frame).unwrap();
        } else {
            let frame = master.acquire_frame(generation, now_ns + 50_000).unwrap();
            master
                .build_and_arm_frame_from_plan(frame, &combined, &image)
                .unwrap();
            master.submit_frame(&mut port, frame).unwrap();
        }
        let received = bank
            .receive_with_dc(&mut master, &mut port, &mut scratch, generation, &mut dc)
            .unwrap();
        assert_eq!(received.report.cycle, u64::from(generation));
        assert!(received.transport_error.is_none());
        assert!(received.qualities[0].valid);
        let snapshots = [
            ScheduledDomainQuality {
                id: 9,
                quality: received.qualities[0],
            },
            ScheduledDomainQuality {
                id: 10,
                quality: received.qualities[1],
            },
        ];
        let facts =
            cyclic_quality_from_schedule(received.report, &schedule, &snapshots, &dc, other);
        guard.update_cyclic_quality(facts, received.report.cycle);
        if generation == 1 {
            guard
                .request_rearm(
                    MotionPermit {
                        boot_id: 7,
                        source_id: 1,
                        permit_epoch: 1,
                        sequence: 1,
                        expires_at_ns: 500_000,
                        axis_mask: 1,
                        authority: 1,
                        reserved: [0; 3],
                        policy_version: 1,
                    },
                    received.report.cycle,
                    now_ns,
                )
                .unwrap();
        }
        let action = guard.cycle(received.report.cycle, now_ns);
        if generation == 2 {
            assert_eq!(received.dc_result, Err(DcCyclicError::MissingResponse));
            assert_eq!(dc.last_sync_cycle(), 1);
            assert_eq!(dc.pending_generation(), None);
            assert_eq!(received.qualities[1].last_valid_cycle, 1);
            assert!(facts.domain_valid && facts.wkc_valid);
            assert!(!facts.distributed_clock_locked);
            assert!(matches!(action, LifecycleAction::Stop(_)));
            assert!(guard.permit().is_none());
        } else {
            assert_eq!(received.dc_result, Ok(()));
            assert_eq!(dc.last_sync_cycle(), u64::from(generation));
            assert!(received.qualities[1].valid);
            assert!(facts.distributed_clock_locked);
            if generation == 1 {
                assert_eq!(action, LifecycleAction::EnableAllowed);
            } else {
                assert!(matches!(action, LifecycleAction::Stop(_)));
            }
        }
    }
}

struct FailingRxPort<'a>(&'a mut SimulatedPort, usize);

impl EthercatPort for FailingRxPort<'_> {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.0.link_state()
    }
    fn now_ns(&self) -> u64 {
        self.0.now_ns()
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.0.tx_submit(frame)
    }
    fn rx_poll(
        &mut self,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        if self.1 == 0 {
            return Err(PortError::HardwareFault);
        }
        self.1 -= 1;
        self.0.rx_poll(scratch)
    }
}

#[test]
fn scheduled_rx_port_error_returns_a_fail_closed_report_and_releases_dc_pending() {
    let schedule = ScheduleTable::<1, 1>::build(
        100_000,
        &[ScheduleDomain {
            id: 9,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut domain = Domain::<2, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 9,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 14, 2),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut image = [0; 10];
    dc.prepare(1, 100_000, &mut image).unwrap();
    let mut port = SimulatedPort::new(1);
    port.set_now_ns(100_000);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let failed = bank
        .receive_with_dc(
            &mut master,
            &mut FailingRxPort(&mut port, 0),
            &mut scratch,
            1,
            &mut dc,
        )
        .unwrap();
    assert!(failed.report.budget_exhausted);
    assert_eq!(failed.report.cycle, master.cycle_number());
    assert!(matches!(
        failed.transport_error,
        Some(CycleError::Port(PortError::HardwareFault))
    ));
    assert!(!failed.qualities[0].valid);
    assert_eq!(failed.dc_result, Err(DcCyclicError::MissingResponse));
    assert_eq!(dc.pending_generation(), None);
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
    let snapshots = [ScheduledDomainQuality {
        id: 9,
        quality: failed.qualities[0],
    }];
    let facts = cyclic_quality_from_schedule(failed.report, &schedule, &snapshots, &dc, other);
    assert!(!facts.domain_valid && !facts.wkc_valid && !facts.distributed_clock_locked);
    assert!(!facts.cycle_within_budget);
    dc.prepare(2, 200_000, &mut image).unwrap();
    let mut conflicting = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 12, 2),
        DcMonitor::new(50, 10, 1, 2),
    );
    conflicting.prepare(2, 200_000, &mut image).unwrap();
    assert!(matches!(
        bank.receive_with_dc(&mut master, &mut port, &mut scratch, 2, &mut conflicting),
        Err(ScheduledReceiveError::DcIndexConflict(12))
    ));
    assert!(matches!(
        bank.receive_with_dc(&mut master, &mut port, &mut scratch, 3, &mut dc),
        Err(ScheduledReceiveError::DcGenerationMismatch)
    ));
    assert_eq!(dc.pending_generation(), Some(2));
    assert_eq!(master.cycle_number(), 1);
}

#[test]
fn port_error_after_valid_frame_still_blocks_this_cycle() {
    let schedule = ScheduleTable::<1, 1>::build(
        100_000,
        &[ScheduleDomain {
            id: 9,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut domain = Domain::<2, 1>::new(0x1000);
    domain
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut bank = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 9,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 14, 2),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut port = SimulatedPort::new(1);
    port.set_now_ns(100_000);
    let mut image = [0u8; 10];
    image[..2].copy_from_slice(&[0x40, 0]);
    dc.prepare(1, 100_000, &mut image).unwrap();
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
    plan.push(dc.datagram_plan()).unwrap();
    let frame = master.acquire_frame(1, 150_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &plan, &image)
        .unwrap();
    master.submit_frame(&mut port, frame).unwrap();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let result = bank
        .receive_with_dc(
            &mut master,
            &mut FailingRxPort(&mut port, 1),
            &mut scratch,
            1,
            &mut dc,
        )
        .unwrap();
    assert!(result.report.budget_exhausted);
    assert_eq!(result.report.cycle, 1);
    assert!(matches!(
        result.transport_error,
        Some(CycleError::Port(PortError::HardwareFault))
    ));
    assert!(result.qualities[0].valid);
    assert_eq!(result.dc_result, Ok(()));
    let snapshots = [ScheduledDomainQuality {
        id: 9,
        quality: result.qualities[0],
    }];
    let facts = cyclic_quality_from_schedule(
        result.report,
        &schedule,
        &snapshots,
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
    );
    assert!(facts.domain_valid);
    assert!(!facts.wkc_valid && !facts.cycle_within_budget);
}
