use esop_ethercat_core::wire::{
    Command, DATAGRAM_HEADER_LEN, DatagramHeader, ETHERCAT_FRAME_HEADER_LEN, ETHERNET_HEADER_LEN,
    MAX_ETHERNET_FRAME_LEN, WORKING_COUNTER_LEN,
};
use esop_ethercat_core::{
    CoeHeader, CoeService, ControlError, ControlRequestPool, CycleError, DatagramPlan,
    DcCyclicConfig, DcCyclicError, DcCyclicSync, DcMonitor, Domain, DomainSegment, ESC_AL_STATUS,
    ESC_CONFIGURATION, EthercatMaster, EthercatPort, EthercatState, ExpectedSlave, FramePlan,
    FramePlanSet, LinkState, MAX_MAILBOX_BYTES, MailboxConfig, MailboxController, MailboxError,
    MailboxHeader, MailboxPhase, MailboxProgress, MailboxProtocol, MailboxRetryPolicy,
    MappingConfigController, MappingConfigPhase, MappingConfigProgress, MappingTable, MasterConfig,
    PdoConfigAction, PdoConfigBatch, PdoConfigBatchPhase, PdoConfigBatchPlan, PdoConfigController,
    PdoConfigError, PdoConfigJob, PdoConfigPhase, PdoConfigPlan, PdoConfigProgress, PdoConfigStep,
    PdoSdoWrite, PortError, RegisterOperation, RequestHandle, RequestState, RxPoll, RxSlotState,
    ScheduleDomain, ScheduleTable, ScheduledControlCycleError, ScheduledDomainBank,
    ScheduledDomainEntry, ScheduledPdoConfiguration, ScheduledPdoConfigurationProgress,
    ScheduledProcessInputEntry, ScheduledProcessInputs, ScheduledProductionServiceCycleError,
    ScheduledProductionServiceFault, ScheduledProductionServiceKind,
    ScheduledProductionServiceProgress, ScheduledProductionServiceRecovery,
    ScheduledProductionServiceScheduler, ScheduledProductionServices, ScheduledReceiveError,
    ScheduledServiceFrameError, ScheduledServiceTxError, ScheduledServiceTxFailure, SlaveIdentity,
    StartupAction, StartupConfig, StartupConfigurationServices, StartupController, StartupPhase,
    StartupProgress, SyncManagerConfig, fixed_address,
};
use esop_ethercat_linux_port::SimulatedPort;
use esop_lifecycle_guard::ethercat::{
    OtherCycleFacts, ScheduledControlGate, ScheduledDomainQuality, cyclic_quality_from_schedule,
    other_cycle_facts_from_control_cycle, other_cycle_facts_from_production_service_cycle,
};
use esop_lifecycle_guard::stop_cycle::{
    ScheduledAuxiliaryOutputs, ScheduledProductionCycleOwner, ScheduledProductionPhase,
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
fn production_service_scheduler_prioritizes_mapping_and_accepts_its_own_generation() {
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
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut table = MappingTable::<1, 0>::new();
    table
        .add_sync_manager(SyncManagerConfig {
            index: 2,
            physical_start: 0x1000,
            length: 2,
            control: 0x24,
            status: 0,
            enable: true,
        })
        .unwrap();
    let mut mapping = MappingConfigController::<1, 0>::new();
    mapping
        .start(1, 41, 100_000, 500_000, 100_000, &table)
        .unwrap();
    let mailbox_config = MailboxConfig::new(0x1000, 16, 0x1100, 16);
    let mut mailbox = MailboxController::new();
    mailbox
        .start(mailbox_config, 1, 52, 100_000, MailboxProtocol::CoE, &[1])
        .unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<2>::new();
    let mut port = TwoFrameSimPort::new();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc_image = [0; 8];
    let process_image = [0x40, 0];
    let mut process_plan = FramePlan::<1>::new();
    process_plan
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 12,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut process_plans = FramePlanSet::<1, 1>::new();
    process_plans.push(process_plan.datagrams()[0]).unwrap();
    let process_inputs = ScheduledProcessInputs::new(
        &bank,
        &schedule,
        [ScheduledProcessInputEntry {
            id: 9,
            image: &process_image,
            plans: &process_plans,
        }],
    )
    .unwrap();
    let outputs =
        ScheduledAuxiliaryOutputs::<1, 1, 1, 1>::new(&bank, &schedule, 9, &process_plan, [None])
            .unwrap();
    let mut production = ScheduledProductionCycleOwner::new(&outputs);

    port.set_now_ns(100_000);
    let process = bank
        .submit_due_process_inputs(&process_inputs, &mut master, &mut port, 1, 150_000, 150_000)
        .unwrap();
    production
        .arm_priming(&bank, &process_inputs, &process, 1, 150_000)
        .unwrap();
    let first = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut mailbox),
            ),
            1,
            150_000,
            150_000,
        )
        .unwrap();
    assert_eq!(first.selected(), ScheduledProductionServiceKind::Mapping);
    assert_eq!(
        first.progress(),
        ScheduledProductionServiceProgress::Mapping(MappingConfigProgress::Advanced)
    );
    assert_eq!(first.request(), None);
    assert_eq!(first.recovery(), ScheduledProductionServiceRecovery::None);
    assert!(!first.service_ready());
    assert!(bank.confirms_production_service_cycle(&first));
    production.complete_service_cycle(&bank, &first).unwrap();
    assert_eq!(production.phase(), ScheduledProductionPhase::OutputPending);
    assert!(
        !other_cycle_facts_from_production_service_cycle(&first, ready_other_cycle_facts(),)
            .coe_ready
    );
    assert_eq!(controls.in_use(), 0);

    port.set_now_ns(200_000);
    let second = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            200_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut mailbox),
            ),
            2,
            250_000,
            250_000,
        )
        .unwrap();
    assert_eq!(second.selected(), ScheduledProductionServiceKind::Mapping);
    assert_eq!(
        second.fault(),
        Some(ScheduledProductionServiceFault::Mapping(
            esop_ethercat_core::MappingConfigError::ReadbackMismatch
        ))
    );
    assert_eq!(
        second.progress(),
        ScheduledProductionServiceProgress::Waiting
    );
    assert!(!second.service_ready());
    assert_eq!(mapping.phase(), MappingConfigPhase::Faulted);

    let empty = MappingTable::<1, 0>::new();
    mapping
        .start(1, 42, 250_000, 500_000, 100_000, &empty)
        .unwrap();
    assert_eq!(mapping.phase(), MappingConfigPhase::Complete);

    port.set_now_ns(300_000);
    let third = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            300_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut mailbox),
            ),
            3,
            350_000,
            350_000,
        )
        .unwrap();
    assert_eq!(third.selected(), ScheduledProductionServiceKind::Mailbox);
    assert_eq!(
        third.progress(),
        ScheduledProductionServiceProgress::Mailbox(MailboxProgress::Advanced)
    );
    assert!(third.service_ready());
    assert_eq!(controls.in_use(), 0);
}

#[test]
fn production_service_scheduler_rebuilds_prepared_and_never_retransmits_inflight() {
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
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut table = MappingTable::<1, 0>::new();
    table
        .add_sync_manager(SyncManagerConfig {
            index: 2,
            physical_start: 0x1000,
            length: 2,
            control: 0x24,
            status: 0,
            enable: true,
        })
        .unwrap();
    let mut mapping = MappingConfigController::<1, 0>::new();
    mapping
        .start(1, 41, 100_000, 500_000, 50_000, &table)
        .unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = TwoFrameSimPort::new();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc_image = [0; 8];

    port.set_now_ns(100_000);
    let invalid_deadline = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(None, Some(&mut mapping), None, None),
            1,
            100_000,
            150_000,
        )
        .unwrap_err();
    assert!(matches!(
        invalid_deadline,
        ScheduledProductionServiceCycleError::ControlCycle(ScheduledControlCycleError::Submit(
            ScheduledServiceTxError::InvalidDeadline
        ))
    ));
    assert_eq!(scheduler.request(), None);
    assert_eq!(controls.in_use(), 0);

    port.fail_nth_next_transmission(1);
    let rebuild = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(None, Some(&mut mapping), None, None),
            1,
            150_000,
            150_000,
        )
        .unwrap();
    assert_eq!(
        rebuild.recovery(),
        ScheduledProductionServiceRecovery::RebuildRequest
    );
    assert_eq!(rebuild.request(), None);
    assert_eq!(controls.in_use(), 0);
    assert!(mapping.pending().is_some());

    port.set_now_ns(150_001);
    let expired_rebuild = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            150_001,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(None, Some(&mut mapping), None, None),
            2,
            200_000,
            200_000,
        )
        .unwrap();
    assert_eq!(expired_rebuild.request(), None);
    assert_eq!(
        expired_rebuild.fault(),
        Some(ScheduledProductionServiceFault::Mapping(
            esop_ethercat_core::MappingConfigError::Timeout
        ))
    );
    assert_eq!(controls.in_use(), 0);
    assert_eq!(mapping.phase(), MappingConfigPhase::Faulted);

    mapping
        .start(1, 42, 160_000, 500_000, 50_000, &table)
        .unwrap();
    port.set_now_ns(160_000);
    port.drop_nth_next_response(2);
    let waiting = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            160_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(None, Some(&mut mapping), None, None),
            3,
            210_000,
            210_000,
        )
        .unwrap();
    let request = waiting.request().unwrap();
    assert_eq!(
        waiting.recovery(),
        ScheduledProductionServiceRecovery::AwaitingResponse
    );
    assert_eq!(controls.get(request).unwrap().state, RequestState::InFlight);
    let attempts_after_first_send = port.tx_attempts;

    port.set_now_ns(170_000);
    let retained = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            170_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(None, Some(&mut mapping), None, None),
            4,
            220_000,
            220_000,
        )
        .unwrap();
    assert_eq!(retained.request(), Some(request));
    assert_eq!(port.tx_attempts, attempts_after_first_send + 1);

    port.set_now_ns(210_001);
    let timed_out = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            210_001,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0>::new(None, Some(&mut mapping), None, None),
            5,
            260_000,
            260_000,
        )
        .unwrap();
    assert_eq!(timed_out.request(), None);
    assert_eq!(
        timed_out.fault(),
        Some(ScheduledProductionServiceFault::Mapping(
            esop_ethercat_core::MappingConfigError::Timeout
        ))
    );
    assert_eq!(
        timed_out.recovery(),
        ScheduledProductionServiceRecovery::Faulted
    );
    assert_eq!(mapping.phase(), MappingConfigPhase::Faulted);
    assert_eq!(controls.in_use(), 0);
}

fn pdo_download_response(action: PdoConfigAction) -> [u8; 6] {
    let mut response = [0; 6];
    CoeHeader {
        number: 0,
        service: CoeService::SdoResponse,
    }
    .encode(&mut response)
    .unwrap();
    response[2] = 0x60;
    response[3..5].copy_from_slice(&action.sdo_index.to_le_bytes());
    response[5] = action.sdo_subindex;
    response
}

fn pdo_upload_response(action: PdoConfigAction, data: &[u8]) -> [u8; 10] {
    assert!((1..=4).contains(&data.len()));
    let mut response = [0; 10];
    CoeHeader {
        number: 0,
        service: CoeService::SdoResponse,
    }
    .encode(&mut response)
    .unwrap();
    response[2] = 0x43 | (((4 - data.len()) as u8) << 2);
    response[3..5].copy_from_slice(&action.sdo_index.to_le_bytes());
    response[5] = action.sdo_subindex;
    response[6..6 + data.len()].copy_from_slice(data);
    response
}

#[test]
fn production_scheduler_runs_pdo_mailbox_readback_before_lower_priority_services() {
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
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut mapping_table = MappingTable::<1, 0>::new();
    mapping_table
        .add_sync_manager(SyncManagerConfig {
            index: 2,
            physical_start: 0x1000,
            length: 2,
            control: 0x24,
            status: 0,
            enable: true,
        })
        .unwrap();
    let mut mapping = MappingConfigController::<1, 0>::new();
    mapping
        .start(1, 51, 100_000, 1_000_000, 100_000, &mapping_table)
        .unwrap();
    let mut plan = PdoConfigPlan::<1>::new();
    plan.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
        .unwrap();
    let mut pdo = PdoConfigController::<1>::new();
    pdo.start(plan, 1, 41, 100_000, 1_000_000, 100_000).unwrap();
    let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32)
        .with_retry_policy(MailboxRetryPolicy::new(1, 10_000));
    let mut pdo_mailbox = MailboxController::new();
    let mut normal_mailbox = MailboxController::new();
    normal_mailbox
        .start(mailbox_config, 1, 61, 100_000, MailboxProtocol::CoE, &[1])
        .unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<2>::new();
    let mut port = TwoFrameSimPort::new();
    port.configure_mailbox(1, mailbox_config);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc_image = [0; 8];

    port.set_now_ns(100_000);
    let rejected = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            1,
            100_000,
            150_000,
        )
        .unwrap_err();
    assert!(matches!(
        rejected,
        ScheduledProductionServiceCycleError::MailboxCycle(
            esop_ethercat_core::ScheduledMailboxCycleError::Submit(
                esop_ethercat_core::ScheduledMailboxTxError::Service(
                    ScheduledServiceTxError::InvalidDeadline
                )
            )
        )
    ));
    let bound_action = pdo.pending().unwrap();
    let bound_mailbox_action = pdo_mailbox.pending().unwrap();
    assert_eq!(scheduler.request(), None);
    assert_eq!(controls.in_use(), 0);

    let first = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            1,
            150_000,
            150_000,
        )
        .unwrap();
    assert_eq!(
        first.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    assert_eq!(pdo.pending(), Some(bound_action));
    assert_eq!(pdo_mailbox.phase(), MailboxPhase::Polling);
    assert_eq!(bound_mailbox_action.token, 1);
    assert_eq!(
        first.progress(),
        ScheduledProductionServiceProgress::PdoConfiguration(
            ScheduledPdoConfigurationProgress::Mailbox(MailboxProgress::Advanced)
        )
    );
    assert!(!first.service_ready());
    assert!(first.mailbox_cycle().is_some());
    assert!(bank.confirms_production_service_cycle(&first));
    assert!(
        !other_cycle_facts_from_production_service_cycle(&first, ready_other_cycle_facts())
            .coe_ready
    );
    assert_eq!(controls.in_use(), 0);
    assert_eq!(normal_mailbox.phase(), MailboxPhase::Sending);

    port.set_now_ns(110_000);
    let wrong_counter = port.mailbox_counter.wrapping_add(1);
    port.set_next_mailbox_response_with_counter(
        &pdo_download_response(bound_action),
        wrong_counter,
    );
    let retry = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            110_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            2,
            160_000,
            160_000,
        )
        .unwrap();
    assert_eq!(
        retry.progress(),
        ScheduledProductionServiceProgress::PdoConfiguration(
            ScheduledPdoConfigurationProgress::Mailbox(MailboxProgress::RetryScheduled)
        )
    );
    assert_eq!(retry.request(), None);
    assert_eq!(pdo.pending(), Some(bound_action));
    assert_eq!(pdo.operation_index(), 0);
    assert_eq!(pdo_mailbox.retry_count(), 1);
    assert_eq!(
        pdo_mailbox.last_retry_error(),
        Some(MailboxError::CounterMismatch)
    );
    assert_eq!(controls.in_use(), 0);

    port.set_now_ns(115_000);
    let retry_wait = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            115_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            3,
            165_000,
            165_000,
        )
        .unwrap();
    assert_eq!(
        retry_wait.progress(),
        ScheduledProductionServiceProgress::Waiting
    );
    assert_eq!(retry_wait.request(), None);
    assert_eq!(pdo.pending(), Some(bound_action));
    assert_eq!(controls.in_use(), 0);

    port.set_now_ns(120_000);
    port.set_next_mailbox_response(&pdo_download_response(bound_action));
    let second = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            120_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            4,
            170_000,
            170_000,
        )
        .unwrap();
    assert_eq!(
        second.progress(),
        ScheduledProductionServiceProgress::PdoConfiguration(
            ScheduledPdoConfigurationProgress::Configuration(PdoConfigProgress::Advanced)
        )
    );
    assert_eq!(pdo.phase(), PdoConfigPhase::Sending);
    assert_eq!(pdo.step(), PdoConfigStep::VerifyUpload);

    port.set_now_ns(130_000);
    let third = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            130_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            5,
            180_000,
            180_000,
        )
        .unwrap();
    let verify_action = pdo.pending().unwrap();
    assert_eq!(verify_action.step, PdoConfigStep::VerifyUpload);
    assert_eq!(
        third.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    assert_eq!(normal_mailbox.phase(), MailboxPhase::Sending);

    port.set_now_ns(140_000);
    port.set_next_mailbox_response(&pdo_upload_response(verify_action, &[0]));
    let fourth = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            140_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            6,
            190_000,
            190_000,
        )
        .unwrap();
    assert_eq!(
        fourth.progress(),
        ScheduledProductionServiceProgress::PdoConfiguration(
            ScheduledPdoConfigurationProgress::Configuration(PdoConfigProgress::Complete)
        )
    );
    assert_eq!(pdo.phase(), PdoConfigPhase::Complete);
    assert_eq!(pdo.operation_index(), 1);
    assert!(fourth.service_ready());
    assert!(bank.confirms_production_service_cycle(&fourth));
    assert!(
        other_cycle_facts_from_production_service_cycle(&fourth, ready_other_cycle_facts())
            .coe_ready
    );
    assert_eq!(controls.in_use(), 0);

    port.set_now_ns(150_000);
    let lower_priority = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            150_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                Some(&mut normal_mailbox),
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            7,
            200_000,
            200_000,
        )
        .unwrap();
    assert_eq!(
        lower_priority.selected(),
        ScheduledProductionServiceKind::Mapping
    );
}

#[test]
fn pdo_transport_fault_blocks_lower_priority_until_explicit_restart() {
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
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut mapping_table = MappingTable::<1, 0>::new();
    mapping_table
        .add_sync_manager(SyncManagerConfig {
            index: 2,
            physical_start: 0x1000,
            length: 2,
            control: 0x24,
            status: 0,
            enable: true,
        })
        .unwrap();
    let mut mapping = MappingConfigController::<1, 0>::new();
    mapping
        .start(1, 51, 100_000, 1_000_000, 100_000, &mapping_table)
        .unwrap();
    let mut plan = PdoConfigPlan::<1>::new();
    plan.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
        .unwrap();
    let mut pdo = PdoConfigController::<1>::new();
    pdo.start(plan, 1, 41, 100_000, 1_000_000, 100_000).unwrap();
    let invalid_mailbox_config = MailboxConfig::new(0x1000, 6, 0x1100, 32);
    let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
    let mut pdo_mailbox = MailboxController::new();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = TwoFrameSimPort::new();
    port.configure_mailbox(1, mailbox_config);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc_image = [0; 8];

    port.set_now_ns(100_000);
    let faulted = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                invalid_mailbox_config,
            )),
            1,
            150_000,
            150_000,
        )
        .unwrap();
    assert_eq!(
        faulted.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    assert_eq!(
        faulted.fault(),
        Some(ScheduledProductionServiceFault::PdoConfiguration(
            PdoConfigError::Mailbox(MailboxError::InvalidConfiguration)
        ))
    );
    assert_eq!(
        faulted.recovery(),
        ScheduledProductionServiceRecovery::Faulted
    );
    assert_eq!(pdo.phase(), PdoConfigPhase::Faulted);
    assert_eq!(pdo.operation_index(), 0);
    assert_eq!(controls.in_use(), 0);
    assert!(
        !other_cycle_facts_from_production_service_cycle(&faulted, ready_other_cycle_facts())
            .coe_ready
    );

    port.set_now_ns(110_000);
    let blocked = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            110_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                invalid_mailbox_config,
            )),
            2,
            160_000,
            160_000,
        )
        .unwrap();
    assert_eq!(
        blocked.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    assert!(blocked.fault().is_some());

    let mut mismatch_plan = PdoConfigPlan::<1>::new();
    mismatch_plan
        .push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
        .unwrap();
    pdo.start(mismatch_plan, 1, 42, 120_000, 1_000_000, 100_000)
        .unwrap();
    port.set_now_ns(120_000);
    let download_sent = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            120_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            3,
            170_000,
            170_000,
        )
        .unwrap();
    assert_eq!(
        download_sent.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    let download_action = pdo.pending().unwrap();

    port.set_now_ns(130_000);
    port.set_next_mailbox_response(&pdo_download_response(download_action));
    let download_complete = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            130_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            4,
            180_000,
            180_000,
        )
        .unwrap();
    assert_eq!(
        download_complete.progress(),
        ScheduledProductionServiceProgress::PdoConfiguration(
            ScheduledPdoConfigurationProgress::Configuration(PdoConfigProgress::Advanced)
        )
    );

    port.set_now_ns(140_000);
    let upload_sent = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            140_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            5,
            190_000,
            190_000,
        )
        .unwrap();
    assert_eq!(
        upload_sent.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    let upload_action = pdo.pending().unwrap();
    assert_eq!(upload_action.step, PdoConfigStep::VerifyUpload);

    port.set_now_ns(150_000);
    port.set_next_mailbox_response(&pdo_upload_response(upload_action, &[1]));
    let mismatch = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            150_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            6,
            200_000,
            200_000,
        )
        .unwrap();
    assert_eq!(
        mismatch.fault(),
        Some(ScheduledProductionServiceFault::PdoConfiguration(
            PdoConfigError::ReadbackValueMismatch {
                byte_index: 0,
                expected: 0,
                actual: 1,
            }
        ))
    );
    assert_eq!(pdo.phase(), PdoConfigPhase::Faulted);
    assert_eq!(pdo.operation_index(), 0);
    assert_eq!(controls.in_use(), 0);
    assert!(
        !other_cycle_facts_from_production_service_cycle(&mismatch, ready_other_cycle_facts())
            .coe_ready
    );

    port.set_now_ns(160_000);
    let mismatch_blocked = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            160_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            7,
            210_000,
            210_000,
        )
        .unwrap();
    assert_eq!(
        mismatch_blocked.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );

    pdo.start(PdoConfigPlan::new(), 1, 43, 170_000, 1_000_000, 100_000)
        .unwrap();
    port.set_now_ns(170_000);
    let resumed = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            170_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 1, 0, 1>::new(
                None,
                Some(&mut mapping),
                None,
                None,
            )
            .with_pdo_configuration(ScheduledPdoConfiguration::new(
                &mut pdo,
                &mut pdo_mailbox,
                mailbox_config,
            )),
            8,
            220_000,
            220_000,
        )
        .unwrap();
    assert_eq!(resumed.selected(), ScheduledProductionServiceKind::Mapping);
}

#[test]
fn pdo_inflight_mailbox_request_crosses_production_generation_without_retransmit() {
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
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut plan = PdoConfigPlan::<1>::new();
    plan.push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
        .unwrap();
    let mut pdo = PdoConfigController::<1>::new();
    pdo.start(plan, 1, 41, 100_000, 500_000, 50_000).unwrap();
    let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
    let mut pdo_mailbox = MailboxController::new();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = TwoFrameSimPort::new();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc_image = [0; 8];

    port.set_now_ns(100_000);
    port.drop_nth_next_response(2);
    let waiting = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 0, 0, 1>::new(None, None, None, None)
                .with_pdo_configuration(ScheduledPdoConfiguration::new(
                    &mut pdo,
                    &mut pdo_mailbox,
                    mailbox_config,
                )),
            1,
            150_000,
            150_000,
        )
        .unwrap();
    let request = waiting.request().unwrap();
    let pdo_action = pdo.pending().unwrap();
    assert_eq!(controls.get(request).unwrap().generation, 41);
    assert_eq!(controls.get(request).unwrap().state, RequestState::InFlight);
    assert_eq!(
        waiting.recovery(),
        ScheduledProductionServiceRecovery::AwaitingResponse
    );
    let attempts_after_first_send = port.tx_attempts;

    port.set_now_ns(110_000);
    let retained = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            110_000,
            &mut controls,
            &mut ScheduledProductionServices::<0, 0, 0, 1>::new(None, None, None, None)
                .with_pdo_configuration(ScheduledPdoConfiguration::new(
                    &mut pdo,
                    &mut pdo_mailbox,
                    mailbox_config,
                )),
            2,
            160_000,
            160_000,
        )
        .unwrap();
    assert_eq!(retained.request(), Some(request));
    assert_eq!(pdo.pending(), Some(pdo_action));
    assert_eq!(port.tx_attempts, attempts_after_first_send + 1);
    assert_eq!(controls.get(request).unwrap().state, RequestState::InFlight);

    port.set_now_ns(150_001);
    let timed_out = scheduler
        .run_cycle(
            &mut bank,
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            150_001,
            &mut controls,
            &mut ScheduledProductionServices::<0, 0, 0, 1>::new(None, None, None, None)
                .with_pdo_configuration(ScheduledPdoConfiguration::new(
                    &mut pdo,
                    &mut pdo_mailbox,
                    mailbox_config,
                )),
            3,
            200_000,
            200_000,
        )
        .unwrap();
    assert_eq!(timed_out.request(), None);
    assert_eq!(
        timed_out.fault(),
        Some(ScheduledProductionServiceFault::PdoConfiguration(
            PdoConfigError::Mailbox(MailboxError::Timeout)
        ))
    );
    assert_eq!(pdo.phase(), PdoConfigPhase::Faulted);
    assert_eq!(pdo.operation_index(), 0);
    assert_eq!(controls.in_use(), 0);
}

fn ready_other_cycle_facts() -> OtherCycleFacts {
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

fn startup_status(state: EthercatState) -> [u8; 6] {
    let mut payload = [0; 6];
    payload[..2].copy_from_slice(&(state as u16).to_le_bytes());
    payload
}

fn accept_startup_action(
    startup: &mut StartupController<2>,
    action: StartupAction,
    payload: &[u8],
    now_ns: u64,
) -> StartupProgress {
    startup
        .accept(action, action.generation(), payload, 1, now_ns)
        .unwrap()
}

fn drive_startup_to_pdo_barrier(startup: &mut StartupController<2>, expected: &[ExpectedSlave; 1]) {
    startup
        .start(
            7,
            0,
            StartupConfig::new(EthercatState::Op).with_configuration_services(
                StartupConfigurationServices::new().with_pdo_configuration(),
            ),
            expected,
        )
        .unwrap();
    let probe = startup.next_action(1).unwrap().unwrap();
    accept_startup_action(startup, probe, &[0x88, 0x02], 2);
    let basic = startup.next_action(3).unwrap().unwrap();
    accept_startup_action(startup, basic, &[0x88, 0x02, 3, 4, 1, 2, 0x00, 0x20, 1], 4);
    let assign = startup.next_action(5).unwrap().unwrap();
    accept_startup_action(startup, assign, &[], 6);
    let configuration = startup.next_action(7).unwrap().unwrap();
    assert_eq!(
        configuration.address(),
        fixed_address(0x1000, ESC_CONFIGURATION)
    );
    accept_startup_action(startup, configuration, &[0], 8);
    let status = startup.next_action(9).unwrap().unwrap();
    accept_startup_action(startup, status, &startup_status(EthercatState::Init), 10);
    let end_probe = startup.next_action(11).unwrap().unwrap();
    assert!(matches!(end_probe, StartupAction::Scan(_)));
    startup.timeout(end_probe, end_probe.deadline_ns()).unwrap();
    assert_eq!(startup.phase(), StartupPhase::ReadingIdentity);

    let mut now_ns = 12;
    for word in [
        0x3344u16, 0x1122, 0x7788, 0x5566, 0xBBCC, 0x99AA, 0xFF00, 0xDDEE,
    ] {
        let address = startup.next_action(now_ns).unwrap().unwrap();
        accept_startup_action(startup, address, &[], now_ns + 1);
        let issue = startup.next_action(now_ns + 2).unwrap().unwrap();
        accept_startup_action(startup, issue, &[], now_ns + 3);
        let poll = startup.next_action(now_ns + 4).unwrap().unwrap();
        accept_startup_action(startup, poll, &[0, 0], now_ns + 5);
        let data = startup.next_action(now_ns + 6).unwrap().unwrap();
        accept_startup_action(startup, data, &word.to_le_bytes(), now_ns + 7);
        now_ns += 8;
    }

    let write = startup.next_action(now_ns).unwrap().unwrap();
    assert!(matches!(write, StartupAction::Al(_)));
    accept_startup_action(startup, write, &[], now_ns + 1);
    let read = startup.next_action(now_ns + 2).unwrap().unwrap();
    assert_eq!(
        accept_startup_action(
            startup,
            read,
            &startup_status(EthercatState::PreOp),
            now_ns + 3,
        ),
        StartupProgress::AwaitingConfiguration
    );
    assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
}

#[test]
fn production_scheduler_runs_pdo_batch_then_releases_startup_through_safeop_to_op() {
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
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 13, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let identity = SlaveIdentity {
        vendor_id: 0x1122_3344,
        product_code: 0x5566_7788,
        revision: 0x99AA_BBCC,
        serial: 0xDDEE_FF00,
    };
    let expected = [ExpectedSlave {
        position: 0,
        station_address: 0x1000,
        identity,
    }];
    let mut startup = StartupController::<2>::new(0x1000);
    drive_startup_to_pdo_barrier(&mut startup, &expected);
    let mailbox_config = MailboxConfig::new(0x1000, 32, 0x1100, 32);
    let mut first_plan = PdoConfigPlan::<1>::new();
    first_plan
        .push(PdoSdoWrite::new(0x1C12, 0, &[0]).unwrap())
        .unwrap();
    let mut second_plan = PdoConfigPlan::<1>::new();
    second_plan
        .push(PdoSdoWrite::new(0x1C13, 0, &[1]).unwrap())
        .unwrap();
    let mut batch_plan = PdoConfigBatchPlan::<2, 1>::new();
    batch_plan
        .push(PdoConfigJob::new(0x1000, first_plan, mailbox_config))
        .unwrap();
    batch_plan
        .push(PdoConfigJob::new(0x1001, second_plan, mailbox_config))
        .unwrap();
    let mut pdo_batch = PdoConfigBatch::new();
    pdo_batch
        .start(batch_plan, 41, 90_000, 1_000_000, 100_000)
        .unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut controls = ControlRequestPool::<2>::new();
    let mut port = TwoFrameSimPort::new();
    port.configure_mailbox(0x1000, mailbox_config);
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc_image = [0; 8];

    macro_rules! run_cycle {
        ($now_ns:expr, $generation:expr) => {{
            port.set_now_ns($now_ns);
            scheduler
                .run_cycle(
                    &mut bank,
                    &mut master,
                    &mut port,
                    &mut scratch,
                    &mut dc,
                    &mut dc_image,
                    $now_ns,
                    &mut controls,
                    &mut ScheduledProductionServices::<2, 0, 0, 1, 2>::new(
                        Some(&mut startup),
                        None,
                        None,
                        None,
                    )
                    .with_pdo_configuration(ScheduledPdoConfiguration::batch(&mut pdo_batch)),
                    $generation,
                    $now_ns + 50_000,
                    $now_ns + 50_000,
                )
                .unwrap()
        }};
    }

    let first = run_cycle!(100_000, 1);
    assert_eq!(
        first.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    assert_eq!(
        first.startup_phase(),
        Some(StartupPhase::AwaitingConfiguration)
    );
    let first_status = first.pdo_batch_status().unwrap();
    assert_eq!(first_status.phase, PdoConfigBatchPhase::Configuring);
    assert_eq!(first_status.current_index, 0);
    assert_eq!(first_status.job_count, 2);
    assert_eq!(first_status.station_address, Some(0x1000));
    assert_eq!(first_status.generation, Some(41));
    let first_facts =
        other_cycle_facts_from_production_service_cycle(&first, ready_other_cycle_facts());
    assert!(!first_facts.coe_ready);
    assert!(!first_facts.topology_valid);
    let download = pdo_batch.controller().pending().unwrap();

    port.set_next_mailbox_response(&pdo_download_response(download));
    let second = run_cycle!(110_000, 2);
    assert_eq!(
        second.progress(),
        ScheduledProductionServiceProgress::PdoConfiguration(
            ScheduledPdoConfigurationProgress::Configuration(PdoConfigProgress::Advanced)
        )
    );
    let third = run_cycle!(120_000, 3);
    assert_eq!(
        third.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    let upload = pdo_batch.controller().pending().unwrap();
    assert_eq!(upload.step, PdoConfigStep::VerifyUpload);

    port.set_next_mailbox_response(&pdo_upload_response(upload, &[0]));
    let first_configured = run_cycle!(130_000, 4);
    assert_eq!(pdo_batch.controller().phase(), PdoConfigPhase::Sending);
    assert!(!first_configured.service_ready());
    assert_eq!(
        first_configured.startup_phase(),
        Some(StartupPhase::AwaitingConfiguration)
    );
    let second_status = first_configured.pdo_batch_status().unwrap();
    assert_eq!(second_status.phase, PdoConfigBatchPhase::Configuring);
    assert_eq!(second_status.current_index, 1);
    assert_eq!(second_status.job_count, 2);
    assert_eq!(second_status.station_address, Some(0x1001));
    assert_eq!(second_status.generation, Some(42));
    let first_configured_facts = other_cycle_facts_from_production_service_cycle(
        &first_configured,
        ready_other_cycle_facts(),
    );
    assert!(!first_configured_facts.coe_ready);
    assert!(!first_configured_facts.topology_valid);

    port.configure_mailbox(0x1001, mailbox_config);
    let second_download_send = run_cycle!(140_000, 5);
    assert_eq!(
        second_download_send.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    assert_eq!(startup.phase(), StartupPhase::AwaitingConfiguration);
    let second_download = pdo_batch.controller().pending().unwrap();
    assert_eq!(second_download.generation, 42);
    assert_eq!(second_download.station_address, 0x1001);

    port.set_next_mailbox_response(&pdo_download_response(second_download));
    let second_download_complete = run_cycle!(150_000, 6);
    assert_eq!(
        second_download_complete.progress(),
        ScheduledProductionServiceProgress::PdoConfiguration(
            ScheduledPdoConfigurationProgress::Configuration(PdoConfigProgress::Advanced)
        )
    );
    let second_upload_send = run_cycle!(160_000, 7);
    assert_eq!(
        second_upload_send.selected(),
        ScheduledProductionServiceKind::PdoConfiguration
    );
    let second_upload = pdo_batch.controller().pending().unwrap();
    assert_eq!(second_upload.step, PdoConfigStep::VerifyUpload);
    assert_eq!(second_upload.generation, 42);

    port.set_next_mailbox_response(&pdo_upload_response(second_upload, &[1]));
    let configured = run_cycle!(170_000, 8);
    assert_eq!(pdo_batch.phase(), PdoConfigBatchPhase::Complete);
    assert!(configured.service_ready());
    let complete_status = configured.pdo_batch_status().unwrap();
    assert_eq!(complete_status.phase, PdoConfigBatchPhase::Complete);
    assert_eq!(complete_status.current_index, 2);
    assert_eq!(complete_status.job_count, 2);
    assert_eq!(complete_status.station_address, None);
    assert_eq!(complete_status.generation, None);
    assert_eq!(
        configured.startup_phase(),
        Some(StartupPhase::AwaitingConfiguration)
    );
    let configured_facts =
        other_cycle_facts_from_production_service_cycle(&configured, ready_other_cycle_facts());
    assert!(configured_facts.coe_ready);
    assert!(!configured_facts.topology_valid);

    let safeop_write = run_cycle!(180_000, 9);
    assert_eq!(
        safeop_write.selected(),
        ScheduledProductionServiceKind::Startup
    );
    assert_eq!(
        safeop_write.progress(),
        ScheduledProductionServiceProgress::Startup(StartupProgress::Advanced)
    );
    assert_eq!(startup.phase(), StartupPhase::TransitioningAl);
    assert_eq!(startup.records()[0].identity, identity);

    port.set_next_control_response(
        fixed_address(0x1000, ESC_AL_STATUS),
        &startup_status(EthercatState::SafeOp),
    );
    let safeop_read = run_cycle!(190_000, 10);
    assert_eq!(
        safeop_read.startup_phase(),
        Some(StartupPhase::TransitioningAl)
    );
    assert_eq!(startup.records()[0].al_status.state, EthercatState::SafeOp);

    let op_write = run_cycle!(200_000, 11);
    assert_eq!(op_write.selected(), ScheduledProductionServiceKind::Startup);
    port.set_next_control_response(
        fixed_address(0x1000, ESC_AL_STATUS),
        &startup_status(EthercatState::Op),
    );
    let ready = run_cycle!(210_000, 12);
    assert_eq!(
        ready.progress(),
        ScheduledProductionServiceProgress::Startup(StartupProgress::Ready)
    );
    assert!(ready.service_ready());
    assert_eq!(ready.startup_phase(), Some(StartupPhase::Ready));
    let ready_facts =
        other_cycle_facts_from_production_service_cycle(&ready, ready_other_cycle_facts());
    assert!(ready_facts.coe_ready);
    assert!(ready_facts.topology_valid);
    assert_eq!(startup.phase(), StartupPhase::Ready);
    assert_eq!(startup.records().len(), 1);
    assert_eq!(startup.records()[0].identity, identity);
    assert_eq!(startup.records()[0].al_status.state, EthercatState::Op);
    assert_eq!(controls.in_use(), 0);
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

#[test]
fn mailbox_cycle_owner_carries_only_live_requests_and_observes_rx_deadline() {
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
    let mut master_config = MasterConfig::new([0xFF; 6], [1, 2, 3, 4, 5, 6]);
    master_config.rx_budget_ns = 100_000;
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(master_config);
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
    let domain_image = [0x40, 0];
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
    let mut request = None;

    for tick in 1..=3u64 {
        let now_ns = tick * 100_000;
        let cycle_deadline_ns = now_ns + 50_000;
        port.set_now_ns(now_ns);
        if tick == 2 {
            // Domain, DC, then mailbox control.
            port.drop_nth_next_response(3);
        }
        let frame = master.acquire_frame(7, cycle_deadline_ns).unwrap();
        master
            .build_and_arm_frame_from_plan(frame, &plan, &domain_image)
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        if tick == 3 {
            // Domain and DC are both delivered, then the service-stage
            // deadline observation sees the exact deadline boundary.
            port.report_now_after_next_rx_polls(2, cycle_deadline_ns);
        }

        let cycle = bank
            .run_dc_and_mailbox_cycle(
                &mut master,
                &mut port,
                &mut scratch,
                &mut dc,
                &mut dc_image,
                now_ns,
                &mut controls,
                &mut mailbox,
                request,
                7,
                now_ns + 80_000,
                cycle_deadline_ns,
            )
            .unwrap();
        assert!(cycle.tx.service.dc_sent);
        assert_eq!(cycle.tx.service.control_sent, tick <= 2);
        assert_eq!(cycle.receive.received.report.cycle, tick);
        assert!(cycle.receive.received.qualities[0].valid);
        assert_eq!(cycle.receive.received.dc_result, Ok(()));
        assert_eq!(dc.pending_generation(), None);

        if tick == 1 {
            assert_eq!(
                cycle.receive.mailbox_progress,
                Some(Ok(MailboxProgress::Advanced))
            );
            assert_eq!(cycle.tx.request, None);
            assert_eq!(cycle.request, None);
            assert!(cycle.post_receive_deadline_met);
            assert_eq!(controls.in_use(), 0);
        } else if tick == 2 {
            assert_eq!(cycle.receive.mailbox_progress, None);
            assert_eq!(cycle.tx.request, cycle.request);
            assert!(cycle.request.is_some());
            assert!(cycle.post_receive_deadline_met);
            assert_eq!(
                controls.get(cycle.request.unwrap()).unwrap().state,
                RequestState::InFlight
            );
        } else {
            assert_eq!(
                cycle.receive.mailbox_progress,
                Some(Ok(MailboxProgress::RetryScheduled))
            );
            assert_eq!(cycle.tx.request, None);
            assert_eq!(cycle.request, None);
            assert!(!cycle.post_receive_deadline_met);
            assert_eq!(controls.in_use(), 0);
            assert_eq!(
                mailbox.last_retry_error(),
                Some(esop_ethercat_core::MailboxError::Timeout)
            );
        }
        request = cycle.request;
    }
}

#[test]
fn non_mailbox_control_cycle_advances_mapping_without_retransmitting_in_flight_requests() {
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
    let mut table = MappingTable::<1, 0>::new();
    table
        .add_sync_manager(SyncManagerConfig {
            index: 2,
            physical_start: 0x1000,
            length: 8,
            control: 0x26,
            status: 0,
            enable: true,
        })
        .unwrap();
    let mut mapping = MappingConfigController::<1, 0>::new();
    mapping
        .start(0x1000, 7, 100_000, 500_000, 50_000, &table)
        .unwrap();
    let mut controls = ControlRequestPool::<1>::new();
    let mut port = TwoFrameSimPort::new();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let domain_image = [0x40, 0];
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

    port.set_now_ns(100_000);
    mapping.next_action(port.now_ns()).unwrap().unwrap();
    let write = mapping.enqueue_pending(&mut controls).unwrap();
    let frame = master.acquire_frame(7, 150_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &plan, &domain_image)
        .unwrap();
    master.submit_frame(&mut port, frame).unwrap();
    let write_cycle = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            Some(write),
            7,
            150_000,
            150_000,
        )
        .unwrap();
    assert!(bank.confirms_control_cycle(&write_cycle));
    assert!(write_cycle.service().dc_sent && write_cycle.service().control_sent);
    assert_eq!(
        write_cycle.request_state_before_tx(),
        Some(RequestState::Prepared)
    );
    assert_eq!(
        write_cycle.request_state_after_rx(),
        Some(RequestState::Complete)
    );
    assert_eq!(
        mapping
            .accept_completed(&mut controls, write, port.now_ns())
            .unwrap(),
        MappingConfigProgress::Advanced
    );
    assert_eq!(mapping.phase(), MappingConfigPhase::VerifyingSyncManager);
    assert_eq!(controls.in_use(), 0);
    assert_eq!(
        other_cycle_facts_from_control_cycle(
            &write_cycle,
            ScheduledControlGate::Configuration,
            true,
            other,
        ),
        other
    );
    assert!(
        !other_cycle_facts_from_control_cycle(
            &write_cycle,
            ScheduledControlGate::Configuration,
            false,
            other,
        )
        .coe_ready
    );

    port.set_now_ns(200_000);
    mapping.next_action(port.now_ns()).unwrap().unwrap();
    let verify = mapping.enqueue_pending(&mut controls).unwrap();
    let frame = master.acquire_frame(7, 250_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &plan, &domain_image)
        .unwrap();
    master.submit_frame(&mut port, frame).unwrap();
    port.drop_nth_next_response(2);
    let missing = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            200_000,
            &mut controls,
            Some(verify),
            7,
            250_000,
            250_000,
        )
        .unwrap();
    assert!(missing.service().control_sent);
    assert_eq!(
        missing.request_state_after_rx(),
        Some(RequestState::InFlight)
    );

    port.set_now_ns(240_000);
    let frame = master.acquire_frame(7, 290_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &plan, &domain_image)
        .unwrap();
    master.submit_frame(&mut port, frame).unwrap();
    let waiting = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            240_000,
            &mut controls,
            Some(verify),
            7,
            290_000,
            290_000,
        )
        .unwrap();
    assert!(!waiting.service().control_sent);
    assert_eq!(
        waiting.request_state_before_tx(),
        Some(RequestState::InFlight)
    );
    assert_eq!(
        waiting.request_state_after_rx(),
        Some(RequestState::InFlight)
    );

    port.set_now_ns(260_000);
    let frame = master.acquire_frame(7, 310_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &plan, &domain_image)
        .unwrap();
    master.submit_frame(&mut port, frame).unwrap();
    let expired = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            260_000,
            &mut controls,
            Some(verify),
            7,
            310_000,
            310_000,
        )
        .unwrap();
    assert!(!expired.service().control_sent);
    assert_eq!(expired.request_state_after_rx(), Some(RequestState::Failed));
    assert!(expired.received().control_expiry.contains(verify));
    assert!(
        !other_cycle_facts_from_control_cycle(
            &expired,
            ScheduledControlGate::Configuration,
            true,
            other,
        )
        .coe_ready
    );
    assert!(
        mapping
            .accept_completed(&mut controls, verify, port.now_ns())
            .is_err()
    );
    assert_eq!(mapping.phase(), MappingConfigPhase::Faulted);
    assert_eq!(controls.in_use(), 0);
}

struct TwoFrameSimPort {
    inner: SimulatedPort,
    frames: [[u8; MAX_ETHERNET_FRAME_LEN]; 4],
    lengths: [usize; 4],
    count: usize,
    tx_attempts: usize,
    drop_on_attempt: Option<usize>,
    fail_on_attempt: Option<usize>,
    rx_polls: usize,
    reported_now_after_rx_polls: Option<(usize, u64)>,
    mailbox_send_address: Option<u32>,
    mailbox_receive_address: Option<u32>,
    mailbox_counter: u8,
    mailbox_response: [u8; MAX_MAILBOX_BYTES],
    mailbox_response_len: usize,
    control_response_address: Option<u32>,
    control_response: [u8; MAX_MAILBOX_BYTES],
    control_response_len: usize,
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
            fail_on_attempt: None,
            rx_polls: 0,
            reported_now_after_rx_polls: None,
            mailbox_send_address: None,
            mailbox_receive_address: None,
            mailbox_counter: 0,
            mailbox_response: [0; MAX_MAILBOX_BYTES],
            mailbox_response_len: 0,
            control_response_address: None,
            control_response: [0; MAX_MAILBOX_BYTES],
            control_response_len: 0,
        }
    }

    fn configure_mailbox(&mut self, station_address: u16, config: MailboxConfig) {
        self.mailbox_send_address = Some(fixed_address(station_address, config.send_address));
        self.mailbox_receive_address = Some(fixed_address(station_address, config.receive_address));
    }

    fn set_next_mailbox_response(&mut self, payload: &[u8]) {
        self.set_next_mailbox_response_with_counter(payload, self.mailbox_counter);
    }

    fn set_next_mailbox_response_with_counter(&mut self, payload: &[u8], counter: u8) {
        assert!(payload.len() + 6 <= self.mailbox_response.len());
        self.mailbox_response.fill(0);
        MailboxHeader {
            length: payload.len() as u16,
            address: 0,
            priority: 0,
            protocol: MailboxProtocol::CoE,
            counter,
        }
        .encode(&mut self.mailbox_response)
        .unwrap();
        self.mailbox_response[6..6 + payload.len()].copy_from_slice(payload);
        self.mailbox_response_len = 6 + payload.len();
    }

    fn set_next_control_response(&mut self, address: u32, payload: &[u8]) {
        assert!(payload.len() <= self.control_response.len());
        self.control_response.fill(0);
        self.control_response[..payload.len()].copy_from_slice(payload);
        self.control_response_address = Some(address);
        self.control_response_len = payload.len();
    }

    fn set_now_ns(&mut self, now_ns: u64) {
        self.inner.set_now_ns(now_ns);
        self.reported_now_after_rx_polls = None;
    }

    fn drop_next_response(&mut self) {
        self.drop_nth_next_response(1);
    }

    fn drop_nth_next_response(&mut self, n: usize) {
        assert!(n > 0);
        self.drop_on_attempt = Some(self.tx_attempts + n);
    }

    fn fail_nth_next_transmission(&mut self, n: usize) {
        assert!(n > 0);
        self.fail_on_attempt = Some(self.tx_attempts + n);
    }

    fn report_now_after_next_rx_polls(&mut self, polls: usize, now_ns: u64) {
        assert!(polls > 0);
        self.reported_now_after_rx_polls = Some((self.rx_polls + polls, now_ns));
    }
}

impl EthercatPort for TwoFrameSimPort {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.inner.link_state()
    }

    fn now_ns(&self) -> u64 {
        self.reported_now_after_rx_polls
            .filter(|(polls, _)| self.rx_polls >= *polls)
            .map_or_else(|| self.inner.now_ns(), |(_, now_ns)| now_ns)
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        if self.count == self.frames.len() {
            return Err(PortError::HardwareFault);
        }
        self.tx_attempts += 1;
        if self.fail_on_attempt == Some(self.tx_attempts) {
            self.fail_on_attempt = None;
            return Err(PortError::HardwareFault);
        }
        if self.drop_on_attempt == Some(self.tx_attempts) {
            self.inner.drop_next_response();
            self.drop_on_attempt = None;
        }
        self.inner.tx_submit(frame)?;
        if let RxPoll::Frame(len) = self.inner.rx_poll(&mut self.frames[self.count])? {
            let mut offset = ETHERNET_HEADER_LEN + ETHERCAT_FRAME_HEADER_LEN;
            loop {
                let header_end = offset + DATAGRAM_HEADER_LEN;
                let header =
                    DatagramHeader::decode(&self.frames[self.count][offset..header_end]).unwrap();
                let payload_end = header_end + header.length as usize;
                if Some(header.address) == self.mailbox_send_address
                    && header.command == Command::Fpwr
                {
                    self.mailbox_counter =
                        MailboxHeader::decode(&self.frames[self.count][header_end..payload_end])
                            .unwrap()
                            .counter;
                } else if Some(header.address) == self.mailbox_receive_address
                    && header.command == Command::Fprd
                    && self.mailbox_response_len != 0
                {
                    let response_len = self.mailbox_response_len.min(header.length as usize);
                    self.frames[self.count][header_end..payload_end].fill(0);
                    self.frames[self.count][header_end..header_end + response_len]
                        .copy_from_slice(&self.mailbox_response[..response_len]);
                    self.mailbox_response_len = 0;
                } else if self.control_response_address == Some(header.address)
                    && header.command == Command::Fprd
                    && self.control_response_len != 0
                {
                    let response_len = self.control_response_len.min(header.length as usize);
                    self.frames[self.count][header_end..payload_end].fill(0);
                    self.frames[self.count][header_end..header_end + response_len]
                        .copy_from_slice(&self.control_response[..response_len]);
                    self.control_response_address = None;
                    self.control_response_len = 0;
                }
                offset = payload_end + WORKING_COUNTER_LEN;
                if header.last {
                    break;
                }
            }
            self.lengths[self.count] = len;
            self.count += 1;
        }
        Ok(())
    }

    fn rx_poll(
        &mut self,
        scratch: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        self.rx_polls += 1;
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
    let sent = bank
        .submit_dc_and_control(
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
            2,
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
    assert_eq!(
        controls.get(retained).unwrap().state,
        RequestState::Prepared
    );
    controls.release(retained).unwrap();
    port.fail_on = 6;
    let failed_dc = bank
        .run_dc_and_mailbox_cycle(
            &mut master,
            &mut port,
            &mut scratch,
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
    assert!(!failed_dc.tx.service.dc_sent && !failed_dc.tx.service.control_sent);
    assert!(matches!(
        failed_dc.tx.service.failure,
        Some(ScheduledServiceTxFailure::Dc(
            ScheduledServiceFrameError::Transmit(CycleError::Port(PortError::HardwareFault))
        ))
    ));
    assert_eq!(failed_dc.request, None);
    assert_eq!(controls.in_use(), 0);
    assert!(second_mailbox.pending().is_some());
    assert_eq!(
        failed_dc.receive.received.dc_result,
        Err(DcCyclicError::MissingResponse)
    );
    assert_eq!(failed_dc.receive.mailbox_progress, None);
    assert_eq!(dc.pending_generation(), None);
    assert!(failed_dc.post_receive_deadline_met);
    assert!(second_mailbox.next_action(400_000).unwrap().is_some());
}
