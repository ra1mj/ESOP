use esop_ethercat_core::wire::{Command, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    ControlError, ControlRequestPool, CycleError, DatagramPlan, DcCyclicConfig, DcCyclicError,
    DcCyclicSync, DcMonitor, Domain, DomainSegment, EthercatMaster, EthercatPort, FramePlan,
    LinkState, MailboxConfig, MailboxController, MailboxPhase, MailboxProgress, MailboxProtocol,
    MailboxRetryPolicy, MasterConfig, PortError, RegisterOperation, RequestHandle, RequestState,
    RxPoll, RxSlotState, ScheduleDomain, ScheduleTable, ScheduledDomainBank, ScheduledDomainEntry,
    ScheduledMailboxTxError, ScheduledReceiveError, ScheduledServiceFrameError,
    ScheduledServiceTxError, ScheduledServiceTxFailure,
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
    assert!(failed.control_expiry.is_empty());
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
    let mut controls = ControlRequestPool::<2>::new();
    let handle = controls
        .acquire(12, 2, 0x5000, RegisterOperation::Read, &[0; 4], 250_000)
        .unwrap();
    let mut control_frame = [0; MAX_ETHERNET_FRAME_LEN];
    controls
        .get_mut(handle)
        .unwrap()
        .build_frame(&mut control_frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
        .unwrap();
    assert!(matches!(
        bank.receive_with_dc_and_control(
            &mut master,
            &mut port,
            &mut scratch,
            2,
            &mut dc,
            &mut controls
        ),
        Err(ScheduledReceiveError::ControlIndexConflict(12))
    ));
    assert_eq!(controls.get(handle).unwrap().state, RequestState::InFlight);
    controls.release(handle).unwrap();
    let handle = controls
        .acquire(14, 2, 0x5000, RegisterOperation::Read, &[0; 4], 250_000)
        .unwrap();
    controls
        .get_mut(handle)
        .unwrap()
        .build_frame(&mut control_frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
        .unwrap();
    assert!(matches!(
        bank.receive_with_dc_and_control(
            &mut master,
            &mut port,
            &mut scratch,
            2,
            &mut dc,
            &mut controls
        ),
        Err(ScheduledReceiveError::ControlIndexConflict(14))
    ));
    controls.release(handle).unwrap();
    for _ in 0..2 {
        let handle = controls
            .acquire(15, 2, 0x5000, RegisterOperation::Read, &[0; 4], 250_000)
            .unwrap();
        controls
            .get_mut(handle)
            .unwrap()
            .build_frame(&mut control_frame, [0xFF; 6], [1, 2, 3, 4, 5, 6])
            .unwrap();
    }
    assert!(matches!(
        bank.receive_with_dc_and_control(
            &mut master,
            &mut port,
            &mut scratch,
            2,
            &mut dc,
            &mut controls
        ),
        Err(ScheduledReceiveError::ControlIndexConflict(15))
    ));
    port.set_now_ns(250_001);
    assert!(matches!(
        bank.receive_with_dc_and_control(
            &mut master,
            &mut port,
            &mut scratch,
            2,
            &mut dc,
            &mut controls
        ),
        Err(ScheduledReceiveError::ControlIndexConflict(15))
    ));
    assert_eq!(
        controls
            .get(RequestHandle::from_index(0).unwrap())
            .unwrap()
            .state,
        RequestState::InFlight
    );
    assert_eq!(dc.pending_generation(), Some(2));
    assert_eq!(master.cycle_number(), 1);
}

#[test]
fn scheduled_rx_retains_in_flight_control_then_expires_it_without_losing_domain_dc() {
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
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = TwoFrameSimPort::new();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut image = [0u8; 10];
    image[..2].copy_from_slice(&[0x40, 0]);
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
    let mut missing = None;
    for generation in 1..=3u16 {
        let now_ns = u64::from(generation) * 100_000;
        port.set_now_ns(now_ns);
        if generation != 3 {
            master.reap_expired_rx_before_tx(now_ns);
        }
        dc.prepare(generation, now_ns, &mut image).unwrap();
        let request = if generation <= 2 {
            let request = controls
                .acquire(
                    15,
                    generation,
                    0x5000,
                    RegisterOperation::Read,
                    &[0; 4],
                    now_ns + 50_000,
                )
                .unwrap();
            let control_frame = master.acquire_frame(generation, now_ns + 50_000).unwrap();
            master
                .build_control_request(&mut controls, request, control_frame)
                .unwrap();
            if generation == 2 {
                port.drop_next_response();
            }
            master.submit_frame(&mut port, control_frame).unwrap();
            Some(request)
        } else {
            None
        };
        if generation == 2 {
            missing = request;
        }
        let data_frame = master.acquire_frame(generation, now_ns + 50_000).unwrap();
        master
            .build_and_arm_frame_from_plan(data_frame, &plan, &image)
            .unwrap();
        master.submit_frame(&mut port, data_frame).unwrap();
        if generation == 2 {
            port.set_now_ns(now_ns + 50_000);
        }
        let received = bank
            .receive_with_dc_and_control(
                &mut master,
                &mut port,
                &mut scratch,
                generation,
                &mut dc,
                &mut controls,
            )
            .unwrap();
        assert_eq!(received.report.cycle, u64::from(generation));
        assert!(received.transport_error.is_none());
        assert_eq!(received.report.consumer_rejections, 0);
        assert!(received.qualities[0].valid);
        assert_eq!(
            received.qualities[0].last_valid_cycle,
            u64::from(generation)
        );
        assert_eq!(received.dc_result, Ok(()));
        assert_eq!(dc.last_sync_cycle(), u64::from(generation));
        if generation == 1 {
            assert_eq!(received.report.parsed_datagrams, 3);
            assert!(received.control_expiry.is_empty());
            let request = request.unwrap();
            assert_eq!(controls.get(request).unwrap().state, RequestState::Complete);
            controls.release(request).unwrap();
        } else if generation == 2 {
            assert_eq!(received.report.parsed_datagrams, 2);
            assert!(received.control_expiry.is_empty());
            assert_eq!(
                controls.get(request.unwrap()).unwrap().state,
                RequestState::InFlight
            );
            assert_eq!(master.rx_entry(15).state, RxSlotState::Armed);
        } else {
            assert_eq!(received.report.parsed_datagrams, 2);
            assert_eq!(received.report.timed_out_datagrams, 1);
            let missing = missing.unwrap();
            assert_eq!(received.control_expiry.handles().next(), Some(missing));
            assert_eq!(controls.get(missing).unwrap().state, RequestState::Failed);
            assert_eq!(
                controls.get(missing).unwrap().last_error(),
                Some(ControlError::Timeout)
            );
            assert!(controls.expire_in_flight(port.now_ns()).is_empty());
            assert_eq!(master.rx_entry(15).state, RxSlotState::Empty);
            controls.release(missing).unwrap();
        }
    }
    assert_eq!(controls.in_use(), 0);
}

#[test]
fn scheduled_control_expiry_finalizes_early_link_and_port_failures() {
    for link_down in [false, true] {
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
        let mut controls = ControlRequestPool::<1>::new();
        let request = controls
            .acquire(15, 1, 0x5000, RegisterOperation::Read, &[0; 4], 105_000)
            .unwrap();
        let frame = master.acquire_frame(1, 105_000).unwrap();
        master
            .build_control_request(&mut controls, request, frame)
            .unwrap();
        let mut port = SimulatedPort::new(1);
        port.set_now_ns(100_000);
        master.submit_frame(&mut port, frame).unwrap();
        port.set_now_ns(105_001);
        if link_down {
            port.set_link_state(LinkState::Down);
        }
        let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
        let result = bank
            .receive_with_dc_and_control(
                &mut master,
                &mut FailingRxPort(&mut port, 0),
                &mut scratch,
                1,
                &mut dc,
                &mut controls,
            )
            .unwrap();
        assert_eq!(result.report.link_down, link_down);
        assert_eq!(result.transport_error.is_some(), !link_down);
        assert!(!result.qualities[0].valid);
        assert_eq!(result.dc_result, Err(DcCyclicError::MissingResponse));
        assert_eq!(result.control_expiry.handles().next(), Some(request));
        assert_eq!(
            controls.get(request).unwrap().last_error(),
            Some(ControlError::Timeout)
        );
        assert_eq!(master.rx_entry(15).state, RxSlotState::Empty);
        assert_eq!(dc.pending_generation(), None);
        controls.release(request).unwrap();
    }
}

#[test]
fn mailbox_service_uses_shared_rx_expiry_to_retry_after_a_missing_poll() {
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
    let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
        .with_retry_policy(MailboxRetryPolicy::new(1, 10));
    config.request_timeout_ns = 50_000;
    config.timeout_ns = 500_000;
    let mut mailbox = MailboxController::new();
    mailbox
        .start(config, 0x1000, 7, 100_000, MailboxProtocol::CoE, &[1])
        .unwrap();
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = TwoFrameSimPort::new();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut domain_image = [0x40, 0];
    let mut dc_image = [0u8; 10];
    let mut plan = FramePlan::<1>::new();
    plan.push(DatagramPlan {
        command: Command::Lrw,
        index: 12,
        address: 0x1000,
        payload_offset: 0,
        payload_len: 2,
        expected_wkc: 1,
    })
    .unwrap();
    let mut mailbox_request = None;
    for tick in 1..=3u64 {
        let now_ns = tick * 100_000;
        port.set_now_ns(now_ns);
        if tick == 2 {
            port.drop_nth_next_response(2);
        }
        let submitted = bank
            .submit_dc_and_mailbox(
                &mut master,
                &mut port,
                &mut dc,
                &mut dc_image,
                now_ns,
                &mut controls,
                &mut mailbox,
                mailbox_request,
                7,
                now_ns + 50_000,
                now_ns + 50_000,
            )
            .unwrap();
        assert!(submitted.service.dc_sent);
        assert_eq!(submitted.service.control_sent, tick <= 2);
        mailbox_request = submitted.request;
        let frame = master.acquire_frame(7, now_ns + 50_000).unwrap();
        master
            .build_and_arm_frame_from_plan(frame, &plan, &domain_image)
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        if tick == 1 {
            let request = mailbox_request.unwrap();
            let datagram_index = controls.get(request).unwrap().datagram_index;
            controls.get_mut(request).unwrap().datagram_index = 99;
            assert!(matches!(
                bank.receive_with_dc_and_mailbox(
                    &mut master,
                    &mut port,
                    &mut scratch,
                    7,
                    &mut dc,
                    &mut controls,
                    &mut mailbox,
                    Some(request),
                ),
                Err(ScheduledReceiveError::MailboxRequestMismatch)
            ));
            assert_eq!(master.cycle_number(), 0);
            assert_eq!(dc.pending_generation(), Some(7));
            controls.get_mut(request).unwrap().datagram_index = datagram_index;
            controls.get_mut(request).unwrap().payload_mut()[0] ^= 1;
            assert!(matches!(
                bank.receive_with_dc_and_mailbox(
                    &mut master,
                    &mut port,
                    &mut scratch,
                    7,
                    &mut dc,
                    &mut controls,
                    &mut mailbox,
                    Some(request),
                ),
                Err(ScheduledReceiveError::MailboxRequestMismatch)
            ));
            assert_eq!(master.cycle_number(), 0);
            assert_eq!(dc.pending_generation(), Some(7));
            controls.get_mut(request).unwrap().payload_mut()[0] ^= 1;
        } else if tick == 2 {
            let request = mailbox_request.unwrap();
            controls.get_mut(request).unwrap().payload_mut()[0] = 1;
            assert!(matches!(
                bank.receive_with_dc_and_mailbox(
                    &mut master,
                    &mut port,
                    &mut scratch,
                    7,
                    &mut dc,
                    &mut controls,
                    &mut mailbox,
                    Some(request),
                ),
                Err(ScheduledReceiveError::MailboxRequestMismatch)
            ));
            assert_eq!(master.cycle_number(), 1);
            assert_eq!(dc.pending_generation(), Some(7));
            controls.get_mut(request).unwrap().payload_mut()[0] = 0;
        }
        let mailbox_received = bank
            .receive_with_dc_and_mailbox(
                &mut master,
                &mut port,
                &mut scratch,
                7,
                &mut dc,
                &mut controls,
                &mut mailbox,
                mailbox_request,
            )
            .unwrap();
        let received = &mailbox_received.received;
        assert_eq!(received.report.cycle, tick);
        assert!(received.qualities[0].valid);
        assert_eq!(received.dc_result, Ok(()));
        assert!(received.transport_error.is_none());
        if tick == 1 {
            assert_eq!(
                mailbox_received.mailbox_progress,
                Some(Ok(MailboxProgress::Advanced))
            );
            assert_eq!(controls.in_use(), 0);
            assert_eq!(mailbox.phase(), MailboxPhase::Polling);
            assert!(received.control_expiry.is_empty());
            mailbox_request = None;
        } else if tick == 2 {
            assert_eq!(mailbox_received.mailbox_progress, None);
            assert!(received.control_expiry.is_empty());
            assert_eq!(
                controls.get(mailbox_request.unwrap()).unwrap().state,
                RequestState::InFlight
            );
        } else {
            let missing = mailbox_request.unwrap();
            assert_eq!(received.control_expiry.handles().next(), Some(missing));
            assert_eq!(
                mailbox_received.mailbox_progress,
                Some(Ok(MailboxProgress::RetryScheduled))
            );
            assert_eq!(
                mailbox.last_retry_error(),
                Some(esop_ethercat_core::MailboxError::Timeout)
            );
            assert_eq!(controls.in_use(), 0);
            assert!(mailbox.next_action(now_ns + 9).unwrap().is_none());
            assert!(mailbox.next_action(now_ns + 10).unwrap().is_some());
            mailbox_request = None;
        }
        domain_image[0] = domain_image[0].wrapping_add(1);
    }
}

struct TwoFrameSimPort {
    inner: SimulatedPort,
    frames: [[u8; MAX_ETHERNET_FRAME_LEN]; 4],
    lengths: [usize; 4],
    count: usize,
    tx_attempts: usize,
    drop_on_attempt: Option<usize>,
}

impl TwoFrameSimPort {
    fn new() -> Self {
        Self {
            inner: SimulatedPort::new(1),
            frames: [[0; MAX_ETHERNET_FRAME_LEN]; 4],
            lengths: [0; 4],
            count: 0,
            tx_attempts: 0,
            drop_on_attempt: None,
        }
    }

    fn set_now_ns(&mut self, now_ns: u64) {
        self.inner.set_now_ns(now_ns);
    }

    fn drop_next_response(&mut self) {
        self.drop_nth_next_response(1);
    }

    fn drop_nth_next_response(&mut self, n: usize) {
        assert!(n > 0);
        self.drop_on_attempt = Some(self.tx_attempts + n);
    }
}

impl EthercatPort for TwoFrameSimPort {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.inner.link_state()
    }

    fn now_ns(&self) -> u64 {
        self.inner.now_ns()
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        if self.count == self.frames.len() {
            return Err(PortError::HardwareFault);
        }
        self.tx_attempts += 1;
        if self.drop_on_attempt == Some(self.tx_attempts) {
            self.inner.drop_next_response();
            self.drop_on_attempt = None;
        }
        self.inner.tx_submit(frame)?;
        if let RxPoll::Frame(len) = self.inner.rx_poll(&mut self.frames[self.count])? {
            self.lengths[self.count] = len;
            self.count += 1;
        }
        Ok(())
    }

    fn rx_poll(
        &mut self,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        if self.count == 0 {
            return Ok(RxPoll::Empty);
        }
        let len = self.lengths[0];
        scratch[..len].copy_from_slice(&self.frames[0][..len]);
        self.frames.rotate_left(1);
        self.lengths.rotate_left(1);
        self.count -= 1;
        Ok(RxPoll::Frame(len))
    }
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

struct FailOnNthTxPort {
    inner: SimulatedPort,
    attempts: usize,
    fail_on: usize,
}

impl EthercatPort for FailOnNthTxPort {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.inner.link_state()
    }

    fn now_ns(&self) -> u64 {
        self.inner.now_ns()
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.attempts += 1;
        if self.attempts == self.fail_on {
            Err(PortError::HardwareFault)
        } else {
            self.inner.tx_submit(frame)
        }
    }

    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        self.inner.rx_poll(destination)
    }
}

#[test]
fn service_tx_preflights_indices_and_retires_failed_transmissions_in_shared_rx() {
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
        DcCyclicConfig::new(0x3000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut port = FailOnNthTxPort {
        inner: SimulatedPort::new(1),
        attempts: 0,
        fail_on: 2,
    };
    port.inner.set_now_ns(100_000);
    let mut controls = ControlRequestPool::<1>::new();
    let mut dc_image = [0u8; 8];
    for (index, expected) in [(12, 12), (13, 13)] {
        let handle = controls
            .acquire(index, 1, 0x5000, RegisterOperation::Read, &[0; 4], 150_000)
            .unwrap();
        assert!(matches!(
            bank.submit_dc_and_control(
                &mut master,
                &mut port,
                &mut dc,
                &mut dc_image,
                100_000,
                &mut controls,
                Some(handle),
                1,
                150_000,
                150_000,
            ),
            Err(ScheduledServiceTxError::ControlIndexConflict(index)) if index == expected
        ));
        assert_eq!(controls.get(handle).unwrap().state, RequestState::Prepared);
        assert_eq!(dc.pending_generation(), None);
        assert_eq!(port.attempts, 0);
        controls.release(handle).unwrap();
    }

    let handle = controls
        .acquire(14, 1, 0x5000, RegisterOperation::Read, &[0; 4], 150_000)
        .unwrap();
    assert!(matches!(
        bank.submit_dc_and_control(
            &mut master,
            &mut port,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            Some(handle),
            2,
            150_000,
            150_000,
        ),
        Err(ScheduledServiceTxError::ControlGenerationMismatch)
    ));
    assert_eq!(port.attempts, 0);

    let sent = bank
        .submit_dc_and_control(
            &mut master,
            &mut port,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            Some(handle),
            1,
            150_000,
            150_000,
        )
        .unwrap();
    assert!(sent.dc_sent && !sent.control_sent);
    assert!(matches!(
        sent.failure,
        Some(ScheduledServiceTxFailure::Control(
            ScheduledServiceFrameError::Transmit(CycleError::Port(PortError::HardwareFault))
        ))
    ));
    assert!(sent.post_tx_deadline_met);
    let request = controls.get(handle).unwrap();
    assert_eq!(request.state, RequestState::Failed);
    assert_eq!(request.last_error(), Some(ControlError::TransmitFailed));
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let received = bank
        .receive_with_dc_and_control(
            &mut master,
            &mut port,
            &mut scratch,
            1,
            &mut dc,
            &mut controls,
        )
        .unwrap();
    assert_eq!(received.dc_result, Ok(()));
    assert!(!received.qualities[0].valid);
    assert!(received.control_expiry.is_empty());
    controls.release(handle).unwrap();

    port.inner.set_now_ns(200_000);
    let next = controls
        .acquire(14, 2, 0x5000, RegisterOperation::Read, &[0; 4], 250_000)
        .unwrap();
    port.fail_on = 3;
    let missing_dc = bank
        .submit_dc_and_control(
            &mut master,
            &mut port,
            &mut dc,
            &mut dc_image,
            200_000,
            &mut controls,
            Some(next),
            2,
            250_000,
            250_000,
        )
        .unwrap();
    assert!(!missing_dc.dc_sent && !missing_dc.control_sent);
    assert!(matches!(
        missing_dc.failure,
        Some(ScheduledServiceTxFailure::Dc(
            ScheduledServiceFrameError::Transmit(CycleError::Port(PortError::HardwareFault))
        ))
    ));
    assert_eq!(controls.get(next).unwrap().state, RequestState::Prepared);
    assert_eq!(dc.pending_generation(), Some(2));
    let retired = bank
        .receive_with_dc_and_control(
            &mut master,
            &mut port,
            &mut scratch,
            2,
            &mut dc,
            &mut controls,
        )
        .unwrap();
    assert_eq!(retired.dc_result, Err(DcCyclicError::MissingResponse));
    assert_eq!(dc.pending_generation(), None);
    controls.release(next).unwrap();

    port.inner.set_now_ns(300_000);
    let mut config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
        .with_retry_policy(MailboxRetryPolicy::new(1, 10));
    config.timeout_ns = 500_000;
    let mut mailbox = MailboxController::new();
    mailbox
        .start(config, 0x1000, 3, 300_000, MailboxProtocol::CoE, &[1])
        .unwrap();
    port.fail_on = 5;
    let failed_control = bank
        .submit_dc_and_mailbox(
            &mut master,
            &mut port,
            &mut dc,
            &mut dc_image,
            300_000,
            &mut controls,
            &mut mailbox,
            None,
            3,
            350_000,
            350_000,
        )
        .unwrap();
    assert!(failed_control.service.dc_sent && !failed_control.service.control_sent);
    assert!(matches!(
        failed_control.service.failure,
        Some(ScheduledServiceTxFailure::Control(
            ScheduledServiceFrameError::Transmit(CycleError::Port(PortError::HardwareFault))
        ))
    ));
    let mailbox_request = failed_control.request.unwrap();
    let consumed = bank
        .receive_with_dc_and_mailbox(
            &mut master,
            &mut port,
            &mut scratch,
            3,
            &mut dc,
            &mut controls,
            &mut mailbox,
            Some(mailbox_request),
        )
        .unwrap();
    assert_eq!(consumed.received.dc_result, Ok(()));
    assert_eq!(
        consumed.mailbox_progress,
        Some(Ok(MailboxProgress::RetryScheduled))
    );
    assert_eq!(
        mailbox.last_retry_error(),
        Some(esop_ethercat_core::MailboxError::Control(
            ControlError::TransmitFailed
        ))
    );
    assert_eq!(controls.in_use(), 0);
    assert!(mailbox.next_action(300_009).unwrap().is_none());
    assert!(mailbox.next_action(300_010).unwrap().is_some());

    port.inner.set_now_ns(400_000);
    let mut second_mailbox = MailboxController::new();
    second_mailbox
        .start(config, 0x1000, 4, 400_000, MailboxProtocol::CoE, &[1])
        .unwrap();
    second_mailbox.next_action(400_000).unwrap().unwrap();
    let retained = second_mailbox.enqueue_pending(&mut controls).unwrap();
    assert!(matches!(
        bank.submit_dc_and_mailbox(
            &mut master,
            &mut port,
            &mut dc,
            &mut dc_image,
            400_000,
            &mut controls,
            &mut second_mailbox,
            Some(retained),
            5,
            450_000,
            450_000,
        ),
        Err(ScheduledMailboxTxError::Service(
            ScheduledServiceTxError::ControlGenerationMismatch
        ))
    ));
    assert_eq!(
        controls.get(retained).unwrap().state,
        RequestState::Prepared
    );
    controls.release(retained).unwrap();
    port.fail_on = 6;
    let failed_dc = bank
        .submit_dc_and_mailbox(
            &mut master,
            &mut port,
            &mut dc,
            &mut dc_image,
            400_000,
            &mut controls,
            &mut second_mailbox,
            None,
            4,
            450_000,
            450_000,
        )
        .unwrap();
    assert!(!failed_dc.service.dc_sent && !failed_dc.service.control_sent);
    assert!(matches!(
        failed_dc.service.failure,
        Some(ScheduledServiceTxFailure::Dc(
            ScheduledServiceFrameError::Transmit(CycleError::Port(PortError::HardwareFault))
        ))
    ));
    assert_eq!(failed_dc.request, None);
    assert_eq!(controls.in_use(), 0);
    assert!(second_mailbox.pending().is_some());
    let retired = bank
        .receive_with_dc_and_mailbox(
            &mut master,
            &mut port,
            &mut scratch,
            4,
            &mut dc,
            &mut controls,
            &mut second_mailbox,
            None,
        )
        .unwrap();
    assert_eq!(
        retired.received.dc_result,
        Err(DcCyclicError::MissingResponse)
    );
    assert_eq!(retired.mailbox_progress, None);
    assert!(second_mailbox.next_action(400_000).unwrap().is_some());
}
