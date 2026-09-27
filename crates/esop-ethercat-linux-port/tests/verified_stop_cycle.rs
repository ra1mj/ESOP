use esop_ethercat_core::wire::{Command, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::{
    ControlRequestPool, CycleError, CycleReport, DatagramPlan, DcCyclicConfig, DcCyclicError,
    DcCyclicSync, DcMonitor, Domain, DomainConfig, DomainDatagramSpec, DomainRegistry,
    DomainSegment, EthercatMaster, EthercatPort, FramePlan, FramePlanSet, LinkState, MailboxConfig,
    MailboxController, MailboxError, MailboxProgress, MailboxProtocol, MasterConfig, PdoDirection,
    PdoEntry, PdoRegistrationRequest, PortError, RegisterOperation, RequestHandle, RequestState,
    RxPoll, ScheduleDomain, ScheduleTable, ScheduledDomainBank, ScheduledDomainEntry,
    ScheduledProcessImageDomainEntry, ScheduledProcessInputEntry, ScheduledProcessInputs,
    ScheduledServiceTxFailure, ScheduledSlaveCopyError, SlaveCopyPlan, SlaveCopyPlanSet,
    SlaveCopyProcessImage, SlaveCopyStatus,
};
use esop_ethercat_linux_port::SimulatedPort;
use esop_lifecycle_guard::cia402::{
    ControlledStopLimits, ControlledStopPhase, ControlledStopPlanner, step_axis_bank,
};
use esop_lifecycle_guard::ethercat::{
    OtherCycleFacts, ScheduledControlGate, ScheduledDomainQuality, StopFrameError,
    cyclic_quality_from_ethercat, cyclic_quality_from_schedule,
    other_cycle_facts_from_mailbox_cycle, submit_active_frame, submit_controlled_stopping_frame,
    submit_inhibited_frame, submit_prepared_active_frame, submit_stopping_frame,
    verified_ethercat_stop_feedback,
};
use esop_lifecycle_guard::procbuf::{
    Cia402AxisCommandPolicy, LifecycleEventCursor, axis_stops_to_procbuf,
    controlled_axis_stops_to_procbuf, lifecycle_events_to_procbuf, lifecycle_to_procbuf,
    motion_permit_from_command,
};
use esop_lifecycle_guard::stop_cycle::{
    AuxiliaryOutputEntry, AuxiliaryOutputPlanError, ControlledStopCycleState,
    ScheduledAuxiliaryOutputs, ScheduledProductionCycleError, ScheduledProductionCycleOwner,
    ScheduledProductionPhase, SharedAuxiliaryOutputEntry, StopCycleContext, StopCycleError,
};
use esop_lifecycle_guard::{
    AxisStopPolicy, GateId, GuardPolicy, LifecycleAction, LifecycleError, LifecycleGuard,
    LifecycleState, MotionPermit, StopAction,
};
use esop_procbuf::{CommandPage, ControlMode, JointCommand, ProcBuf, QualityFact, StatePage};
use esop_profile_cia402::{
    CONTROLWORD_DISABLE_VOLTAGE, CONTROLWORD_QUICK_STOP, Cia402AxisBank, Cia402PdoField,
    Cia402PdoMap, Cia402Target, CyclicLimits, CyclicSetpoint, CyclicSetpointError,
    CyclicSetpointGuard, DriveRequest, OperatingMode,
};

mod generated_product {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-dual-axis/expected/esop_product_config.rs"
    ));
}

const IMAGE_BYTES: usize = 32;
const FEEDBACK_POLICY: Cia402AxisCommandPolicy = Cia402AxisCommandPolicy {
    position_units_per_radian: 1.0,
    velocity_units_per_radian_per_second: 1.0,
    torque_units_per_newton_metre: 1.0,
    position_offset: 0,
    min_position_radians: f64::MIN,
    max_position_radians: f64::MAX,
    max_velocity_radians_per_second: f64::MAX,
    max_torque_newton_metres: f64::MAX,
    max_position_step_radians: f64::MAX,
};

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

fn csp_input_image(statusword: u16, position: i32, velocity: i32) -> [u8; IMAGE_BYTES] {
    let mut image = input_image(statusword, velocity);
    image[5..9].copy_from_slice(&position.to_le_bytes());
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

struct QueuedRxPort {
    inner: SimulatedPort,
    pending: [Option<(usize, [u8; MAX_ETHERNET_FRAME_LEN])>; 4],
    read: usize,
    written: usize,
}

impl EthercatPort for QueuedRxPort {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.inner.link_state()
    }

    fn now_ns(&self) -> u64 {
        self.inner.now_ns()
    }

    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.inner.tx_submit(frame)?;
        let mut response = [0; MAX_ETHERNET_FRAME_LEN];
        if let RxPoll::Frame(length) = self.inner.rx_poll(&mut response)? {
            assert!(self.written - self.read < self.pending.len());
            self.pending[self.written % self.pending.len()] = Some((length, response));
            self.written += 1;
        }
        Ok(())
    }

    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        if self.read != self.written {
            let (length, response) = self.pending[self.read % self.pending.len()].take().unwrap();
            destination[..length].copy_from_slice(&response[..length]);
            self.read += 1;
            Ok(RxPoll::Frame(length))
        } else {
            self.inner.rx_poll(destination)
        }
    }
}

impl QueuedRxPort {
    fn tx_frames(&self) -> usize {
        self.inner.tx_frames()
    }
}

#[test]
fn rejected_process_submission_is_bound_to_the_invalidated_receive_cycle() {
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
    let mut plans = FramePlanSet::<1, 1>::new();
    plans
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 12,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let image = [0u8; 2];
    let inputs = ScheduledProcessInputs::new(
        &bank,
        &schedule,
        [ScheduledProcessInputEntry {
            id: 9,
            image: &image,
            plans: &plans,
        }],
    )
    .unwrap();
    let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut port = SimulatedPort::new(1);
    port.set_now_ns(100_000);
    port.fail_next_tx();
    let mut process = bank
        .submit_due_process_inputs(&inputs, &mut master, &mut port, 1, 150_000, 150_000)
        .unwrap();
    assert_eq!(process.expected_frames, 1);
    assert_eq!(process.sent_frames, 0);
    assert!(process.failure.is_some());
    assert!(process.post_tx_deadline_met);
    assert!(bank.accepts_process_tx(&inputs, &process));
    process.rx_deadline_ns = 0;
    assert!(!bank.accepts_process_tx(&inputs, &process));
    process.rx_deadline_ns = 150_000;

    bank.begin_due(1, 1).unwrap();
    assert!(!bank.accepts_process_tx(&inputs, &process));
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let received = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 1, &mut bank)
        .unwrap();
    let qualities = bank.finish_due(received.cycle, 1).unwrap();
    assert!(!qualities[0].valid);
    assert!(bank.confirms_process_tx(&inputs, &process));
}

#[test]
fn shared_rx_report_qualifies_scheduled_outputs_only_for_its_bound_cycle() {
    const PRODUCT_IMAGE_BYTES: usize = IMAGE_BYTES + 5;
    let mut registry = DomainRegistry::<2, 5, 2>::new();
    registry
        .register_domain(DomainConfig::new(9, 0x1000, 0, IMAGE_BYTES, 1, 0))
        .unwrap();
    registry
        .register_domain(DomainConfig::new(10, 0x2000, IMAGE_BYTES, 5, 1, 0))
        .unwrap();
    let source = registry
        .register_pdo_at(
            9,
            40,
            PdoRegistrationRequest::new(0, 0x6064, 0, PdoDirection::Tx, 32, true),
        )
        .unwrap();
    let target = registry
        .register_pdo_at(
            10,
            0,
            PdoRegistrationRequest::new(1, 0x7010, 1, PdoDirection::Rx, 32, true),
        )
        .unwrap();
    let quality = registry
        .register_pdo_at(
            10,
            32,
            PdoRegistrationRequest::new(1, 0x7011, 1, PdoDirection::Rx, 8, false),
        )
        .unwrap();
    let motion_target = registry
        .register_pdo_at(
            9,
            192,
            PdoRegistrationRequest::new(1, 0x607A, 0, PdoDirection::Rx, 32, true),
        )
        .unwrap();
    let motion_quality = registry
        .register_pdo_at(
            9,
            224,
            PdoRegistrationRequest::new(1, 0x7012, 1, PdoDirection::Rx, 8, false),
        )
        .unwrap();
    registry
        .register_datagram(
            9,
            DomainDatagramSpec::input(Command::Lrw, 12, 0x1000, 0, IMAGE_BYTES, 1),
        )
        .unwrap();
    registry
        .register_datagram(
            10,
            DomainDatagramSpec::input(Command::Lrw, 13, 0x2000, 0, 5, 1),
        )
        .unwrap();
    let mut frame_plans = [FramePlanSet::<1, 3>::new(); 2];
    let schedule = registry
        .activate_with_frame_plans::<1, 1, 3>(100_000, &mut frame_plans)
        .unwrap();
    let mut copy_plans = SlaveCopyPlanSet::<1>::new();
    copy_plans
        .push(SlaveCopyPlan::build(&registry, source, target, quality, 0xCC).unwrap())
        .unwrap();
    let mut motion_copy_plans = SlaveCopyPlanSet::<1>::new();
    motion_copy_plans
        .push(SlaveCopyPlan::build(&registry, source, motion_target, motion_quality, 0xCC).unwrap())
        .unwrap();
    let motion_plan = *frame_plans[0].plans().first().unwrap();
    let mut motion = Domain::<PRODUCT_IMAGE_BYTES, 1>::new(0x1000);
    motion
        .add_segment(DomainSegment {
            datagram_index: 12,
            input_offset: 0,
            len: IMAGE_BYTES,
            expected_wkc: 1,
        })
        .unwrap();
    let mut auxiliary = Domain::<5, 1>::new(0x2000);
    auxiliary
        .add_segment(DomainSegment {
            datagram_index: 13,
            input_offset: 0,
            len: 5,
            expected_wkc: 1,
        })
        .unwrap();
    let mut domain_bank = ScheduledDomainBank::new_with_process_image_offsets(
        &schedule,
        [
            ScheduledProcessImageDomainEntry {
                id: 9,
                process_image_offset: 0,
                domain: &mut motion,
            },
            ScheduledProcessImageDomainEntry {
                id: 10,
                process_image_offset: IMAGE_BYTES,
                domain: &mut auxiliary,
            },
        ],
    )
    .unwrap();
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 14, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut initial_image = [0u8; PRODUCT_IMAGE_BYTES];
    initial_image[..IMAGE_BYTES].copy_from_slice(&csp_input_image(0x0027, 120, 10));
    initial_image[IMAGE_BYTES..].copy_from_slice(&[0xAA, 0xBB, 0, 0, 0]);
    let priming_image = initial_image;
    let mut copy_image = SlaveCopyProcessImage::new(initial_image);
    let mut fault_image = SlaveCopyProcessImage::new(initial_image);
    let unpublished_image = SlaveCopyProcessImage::new(initial_image);
    let auxiliary_outputs = ScheduledAuxiliaryOutputs::new_with_shared_process_image(
        &domain_bank,
        &schedule,
        9,
        &motion_plan,
        &copy_image,
        [
            None,
            Some(SharedAuxiliaryOutputEntry {
                id: 10,
                plans: &frame_plans[1],
            }),
        ],
    )
    .unwrap();
    assert_eq!(
        ScheduledProductionCycleOwner::with_slave_copies(
            &auxiliary_outputs,
            &motion_copy_plans,
            &copy_image,
        )
        .map(|_| ()),
        Err(ScheduledProductionCycleError::InvalidCopyTarget {
            plan_index: 0,
            domain_id: 9,
        })
    );
    let mut master = EthercatMaster::<2, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut port = QueuedRxPort {
        inner: SimulatedPort::new(1),
        pending: [None; 4],
        read: 0,
        written: 0,
    };
    port.inner.set_now_ns(100_000);
    let mut controls = ControlRequestPool::<1>::new();
    let mut mailbox = MailboxController::new();
    mailbox
        .start(
            MailboxConfig::new(0x4000, 32, 0x4100, 32),
            0x5000,
            1,
            100_000,
            MailboxProtocol::CoE,
            &[1],
        )
        .unwrap();
    let process_inputs = ScheduledProcessInputs::new(
        &domain_bank,
        &schedule,
        [
            ScheduledProcessInputEntry {
                id: 9,
                image: &priming_image,
                plans: &frame_plans[0],
            },
            ScheduledProcessInputEntry {
                id: 10,
                image: &priming_image,
                plans: &frame_plans[1],
            },
        ],
    )
    .unwrap();
    let mut process = domain_bank
        .submit_due_process_inputs(&process_inputs, &mut master, &mut port, 1, 150_000, 150_000)
        .unwrap();
    assert_eq!(process.cycle, 1);
    assert_eq!(process.expected_frames, 2);
    assert_eq!(process.sent_frames, 2);
    assert!(process.failure.is_none() && process.post_tx_deadline_met);
    let mut production = ScheduledProductionCycleOwner::with_slave_copies(
        &auxiliary_outputs,
        &copy_plans,
        &copy_image,
    )
    .unwrap();
    let mut faulted_production = ScheduledProductionCycleOwner::with_slave_copies(
        &auxiliary_outputs,
        &copy_plans,
        &fault_image,
    )
    .unwrap();
    assert_eq!(
        production.phase(),
        ScheduledProductionPhase::PrimingRequired
    );
    process.generation = 2;
    assert_eq!(
        production.arm_priming(&domain_bank, &process_inputs, &process, 1, 150_000),
        Err(ScheduledProductionCycleError::ProcessMismatch)
    );
    process.generation = 1;
    let primed = production
        .arm_priming(&domain_bank, &process_inputs, &process, 1, 150_000)
        .unwrap();
    faulted_production
        .arm_priming(&domain_bank, &process_inputs, &process, 1, 150_000)
        .unwrap();
    assert_eq!(primed.cycle(), 1);
    assert_eq!(primed.generation(), 1);
    assert!(primed.complete());
    assert_eq!(production.phase(), ScheduledProductionPhase::ReceiveArmed);
    assert_eq!(
        production.arm_priming(&domain_bank, &process_inputs, &process, 1, 150_000),
        Err(ScheduledProductionCycleError::InvalidPhase)
    );
    let mut dc_image = [0u8; 8];
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut service_cycle = domain_bank
        .run_dc_and_mailbox_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            100_000,
            &mut controls,
            &mut mailbox,
            None,
            1,
            150_000,
            150_000,
        )
        .unwrap();
    assert!(service_cycle.tx.service.dc_sent && service_cycle.tx.service.control_sent);
    assert!(
        service_cycle.tx.service.failure.is_none() && service_cycle.tx.service.post_tx_deadline_met
    );
    let received = &service_cycle.receive.received;
    assert_eq!(received.dc_result, Ok(()));
    assert!(received.transport_error.is_none());
    assert!(received.qualities.iter().all(|quality| quality.valid));
    assert!(domain_bank.confirms_receive(received));
    assert_eq!(
        service_cycle.receive.mailbox_progress,
        Some(Ok(MailboxProgress::Advanced))
    );
    assert_eq!(service_cycle.request, None);
    assert_eq!(controls.in_use(), 0);
    service_cycle.receive.received.generation = 2;
    assert!(!domain_bank.confirms_receive(&service_cycle.receive.received));
    service_cycle.receive.received.generation = 1;
    domain_bank
        .publish_slave_copies(&copy_plans, &mut fault_image, 2)
        .unwrap();
    let fault_page = *fault_image.published();
    assert_eq!(
        faulted_production.complete_mailbox_cycle_with_slave_copies(
            &domain_bank,
            &service_cycle,
            &mut fault_image,
        ),
        Err(ScheduledProductionCycleError::SlaveCopy(
            ScheduledSlaveCopyError::CycleOrder,
        ))
    );
    assert_eq!(
        faulted_production.phase(),
        ScheduledProductionPhase::Faulted
    );
    assert_eq!(fault_image.published(), &fault_page);
    assert_eq!(
        faulted_production.complete_mailbox_cycle_with_slave_copies(
            &domain_bank,
            &service_cycle,
            &mut fault_image,
        ),
        Err(ScheduledProductionCycleError::InvalidPhase)
    );
    assert_eq!(fault_image.published(), &fault_page);
    let publication = production
        .complete_mailbox_cycle_with_slave_copies(&domain_bank, &service_cycle, &mut copy_image)
        .unwrap();
    assert_eq!(publication.target_cycle(), 2);
    assert_eq!(publication.len(), 1);
    assert_eq!(
        publication.applications()[0].outcome.status,
        SlaveCopyStatus::Valid
    );
    assert_eq!(copy_image.published_cycle(), 2);
    assert_eq!(&copy_image.published()[IMAGE_BYTES..], &[120, 0, 0, 0, 1]);
    assert_eq!(production.phase(), ScheduledProductionPhase::OutputPending);

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
    assert_eq!(
        other_cycle_facts_from_mailbox_cycle(&service_cycle, other),
        other
    );
    service_cycle.tx.service.failure = Some(ScheduledServiceTxFailure::Deadline);
    assert!(!other_cycle_facts_from_mailbox_cycle(&service_cycle, other).coe_ready);
    service_cycle.tx.service.failure = None;
    service_cycle.receive.mailbox_progress = Some(Ok(MailboxProgress::RetryScheduled));
    assert!(!other_cycle_facts_from_mailbox_cycle(&service_cycle, other).coe_ready);
    service_cycle.receive.mailbox_progress = Some(Err(MailboxError::Timeout));
    assert!(!other_cycle_facts_from_mailbox_cycle(&service_cycle, other).coe_ready);
    service_cycle.receive.mailbox_progress = Some(Ok(MailboxProgress::Advanced));
    service_cycle.post_receive_deadline_met = false;
    assert!(!other_cycle_facts_from_mailbox_cycle(&service_cycle, other).deadline_met);
    service_cycle.post_receive_deadline_met = true;
    let snapshots = [
        ScheduledDomainQuality {
            id: 9,
            quality: service_cycle.receive.received.qualities[0],
        },
        ScheduledDomainQuality {
            id: 10,
            quality: service_cycle.receive.received.qualities[1],
        },
    ];
    let mut guard = LifecycleGuard::new_with_axis_stop_policy(
        GateId::Domain.bit()
            | GateId::Link.bit()
            | GateId::Configuration.bit()
            | GateId::Budget.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
        AxisStopPolicy::uniform(StopAction::Hold),
    );
    guard.update_cyclic_quality(
        cyclic_quality_from_schedule(
            service_cycle.receive.received.report,
            &schedule,
            &snapshots,
            &dc,
            other,
        ),
        service_cycle.receive.received.report.cycle,
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
            service_cycle.receive.received.report.cycle,
            100_000,
        )
        .unwrap();
    let mut cursor = LifecycleEventCursor::new(&guard);
    let buffer = ProcBuf::<1, 0, 2, 8>::new(1, 7);
    let mut axis_bank = Cia402AxisBank::<1>::new();
    let maps = [map()];
    let modes = [OperatingMode::Csp];
    let limits = [CyclicLimits {
        max_position_step: 2.0,
        max_velocity: 2.0,
        max_torque: 2.0,
    }];
    let mut guards = [CyclicSetpointGuard::new()];
    guards[0].bind_activation(7, guard.transition_sequence());
    guards[0]
        .seed_from_actual(CyclicSetpoint {
            position: 120.0,
            ..CyclicSetpoint::ZERO
        })
        .unwrap();
    let targets = [Some(Cia402Target::Position(120))];
    let mut state = StatePage::<1, 0, 2>::new(7);
    state.sequence = service_cycle.receive.received.report.cycle;
    {
        let mut context = StopCycleContext {
            guard: &mut guard,
            bank: &mut axis_bank,
            master: &mut master,
            port: &mut port,
            domain: domain_bank.domain::<PRODUCT_IMAGE_BYTES, 1>(9).unwrap(),
            dc: &dc,
            buffer: &buffer,
            event_cursor: &mut cursor,
            state: &mut state,
            report: service_cycle.receive.received.report,
            other,
            maps: &maps,
            modes: &modes,
            axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
            max_stationary_velocities: &[1],
            safe_process_image: copy_image.published(),
            shared_process_image: None,
            plan: &motion_plan,
            next_generation: 2,
            deadline_ns: 250_000,
            now_ns: 100_000,
            transition_time_ns: 100_000,
        };
        let tx_before_publication_proof = context.port.tx_frames();
        assert!(matches!(
            context.run_process_mailbox_cycle_with_outputs_until(
                &domain_bank,
                &process_inputs,
                &process,
                &service_cycle,
                9,
                &auxiliary_outputs,
                &targets,
                &mut guards,
                &limits,
                150_000,
            ),
            Err(StopCycleError::InvalidAuxiliaryOutputs)
        ));
        assert_eq!(context.port.tx_frames(), tx_before_publication_proof);
        assert_eq!(context.state.quality.sequence, 0);
        context.shared_process_image = Some(unpublished_image.published_image());
        assert!(matches!(
            context.run_process_mailbox_cycle_with_outputs_until(
                &domain_bank,
                &process_inputs,
                &process,
                &service_cycle,
                9,
                &auxiliary_outputs,
                &targets,
                &mut guards,
                &limits,
                150_000,
            ),
            Err(StopCycleError::InvalidAuxiliaryOutputs)
        ));
        assert_eq!(context.port.tx_frames(), tx_before_publication_proof);
        assert_eq!(context.state.quality.sequence, 0);
        context.shared_process_image = Some(copy_image.published_image());
        service_cycle.receive.received.qualities[1].actual_wkc = 0;
        assert!(!domain_bank.confirms_receive(&service_cycle.receive.received));
        assert!(matches!(
            context.run_process_mailbox_cycle_with_outputs_until(
                &domain_bank,
                &process_inputs,
                &process,
                &service_cycle,
                9,
                &auxiliary_outputs,
                &targets,
                &mut guards,
                &limits,
                150_000,
            ),
            Err(StopCycleError::ReceiveMismatch)
        ));
        service_cycle.receive.received.qualities[1].actual_wkc = 1;
        service_cycle.receive.received.report.cycle = 0;
        assert!(matches!(
            context.run_process_mailbox_cycle_with_outputs_until(
                &domain_bank,
                &process_inputs,
                &process,
                &service_cycle,
                9,
                &auxiliary_outputs,
                &targets,
                &mut guards,
                &limits,
                150_000,
            ),
            Err(StopCycleError::ReceiveMismatch)
        ));
        service_cycle.receive.received.report.cycle = 1;
        service_cycle.request = RequestHandle::from_index(0);
        assert!(matches!(
            context.run_process_mailbox_cycle_with_outputs_until(
                &domain_bank,
                &process_inputs,
                &process,
                &service_cycle,
                9,
                &auxiliary_outputs,
                &targets,
                &mut guards,
                &limits,
                150_000,
            ),
            Err(StopCycleError::ServiceCycleMismatch)
        ));
        service_cycle.request = None;
        process.cycle = 0;
        assert!(matches!(
            context.run_process_mailbox_cycle_with_outputs_until(
                &domain_bank,
                &process_inputs,
                &process,
                &service_cycle,
                9,
                &auxiliary_outputs,
                &targets,
                &mut guards,
                &limits,
                150_000,
            ),
            Err(StopCycleError::ProcessCycleMismatch)
        ));
        process.cycle = 1;
        assert_eq!(context.port.tx_frames(), 4);
        assert_eq!(context.state.quality.sequence, 0);
        let outcome = context
            .run_process_mailbox_cycle_with_outputs_until(
                &domain_bank,
                &process_inputs,
                &process,
                &service_cycle,
                9,
                &auxiliary_outputs,
                &targets,
                &mut guards,
                &limits,
                150_000,
            )
            .unwrap();
        assert_eq!(outcome.action, LifecycleAction::EnableAllowed);
        assert!(outcome.transmission.is_ok());
        assert_eq!(outcome.auxiliary_frames_sent, 1);
        assert!(outcome.quality.distributed_clock_locked);
        assert_eq!(outcome.post_tx_deadline_met, Some(true));
        assert_eq!(outcome.state_publish, Ok(1));
        assert_eq!(buffer.read_state().unwrap().state.sequence, 1);
        assert_eq!(context.port.tx_frames(), 6);
        let auxiliary_slot = context.port.read % context.port.pending.len();
        let (length, frame) = context.port.pending[auxiliary_slot].as_ref().unwrap();
        let frame = FrameView::parse(&frame[..*length]).unwrap();
        let datagram = frame.datagrams().next().unwrap().unwrap();
        assert_eq!(datagram.header.index, 13);
        assert_eq!(datagram.payload, &[120, 0, 0, 0, 1]);
        context.port.pending[auxiliary_slot].take();
        context.port.read += 1;
        assert_eq!(
            production.settle_output(&auxiliary_outputs, &outcome, 249_999),
            Err(ScheduledProductionCycleError::OutputMismatch)
        );
        assert_eq!(production.phase(), ScheduledProductionPhase::OutputPending);
        let release = production
            .settle_output(&auxiliary_outputs, &outcome, 250_000)
            .unwrap();
        assert!(release.task_released());
        assert_eq!(release.cycle, 1);
        assert_eq!(release.next.cycle(), 2);
        assert_eq!(release.next.generation(), 2);
        assert_eq!(release.next.sent_frames(), 2);
        assert_eq!(release.slave_copies_applied, 1);
        assert_eq!(production.phase(), ScheduledProductionPhase::ReceiveArmed);
    }
    port.inner.set_now_ns(200_000);
    let control = controls
        .acquire(15, 2, 0x5000, RegisterOperation::Read, &[0; 4], 250_000)
        .unwrap();
    // Drop only the newly submitted DC response. The motion response from the
    // prior output handoff and this control response still share the same RX.
    port.inner.drop_next_response();
    let control_cycle = domain_bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            200_000,
            &mut controls,
            Some(control),
            2,
            250_000,
            250_000,
        )
        .unwrap();
    assert!(control_cycle.service().dc_sent && control_cycle.service().control_sent);
    assert_eq!(
        control_cycle.request_state_after_rx(),
        Some(RequestState::Complete)
    );
    assert!(domain_bank.confirms_control_cycle(&control_cycle));
    let publication = production
        .complete_control_cycle_with_slave_copies(&domain_bank, &control_cycle, &mut copy_image)
        .unwrap();
    assert_eq!(publication.target_cycle(), 3);
    assert_eq!(publication.len(), 1);
    assert_eq!(
        publication.applications()[0].outcome.status,
        SlaveCopyStatus::Valid
    );
    assert_eq!(production.phase(), ScheduledProductionPhase::OutputPending);
    controls.release(control).unwrap();
    let received = control_cycle.received();
    assert!(received.qualities[0].valid);
    assert!(!received.qualities[1].valid);
    assert_eq!(received.dc_result, Err(DcCyclicError::MissingResponse));
    assert!(!domain_bank.confirms_receive(&service_cycle.receive.received));
    let mut state = StatePage::<1, 0, 2>::new(7);
    state.sequence = received.report.cycle;
    let mut controlled = ControlledStopCycleState::new([ControlledStopLimits {
        max_velocity_step: 1,
        max_torque_step: 1,
        max_stationary_velocity: 1,
        max_zero_torque: 1,
    }]);
    let stopped = StopCycleContext {
        guard: &mut guard,
        bank: &mut axis_bank,
        master: &mut master,
        port: &mut port,
        domain: domain_bank.domain::<PRODUCT_IMAGE_BYTES, 1>(9).unwrap(),
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: received.report,
        other,
        maps: &maps,
        modes: &modes,
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &[1],
        safe_process_image: copy_image.published(),
        shared_process_image: Some(copy_image.published_image()),
        plan: &motion_plan,
        next_generation: 3,
        deadline_ns: 350_000,
        now_ns: 200_000,
        transition_time_ns: 100_000,
    }
    .run_control_cycle_with_controlled_stop_until(
        &domain_bank,
        &control_cycle,
        ScheduledControlGate::Configuration,
        true,
        9,
        &auxiliary_outputs,
        &[None],
        &mut guards,
        &limits,
        &mut controlled,
        250_000,
    )
    .unwrap();
    assert!(matches!(stopped.action, LifecycleAction::Stop(_)));
    assert!(stopped.controlled_stop_used);
    assert!(!stopped.controlled_stop_fallback);
    assert!(stopped.controlled_stop_failure.is_none());
    assert_eq!(controlled.phase(0), Some(ControlledStopPhase::Applying));
    assert!(!stopped.quality.domain_valid);
    assert!(!stopped.quality.distributed_clock_locked);
    assert_eq!(guard.state(), LifecycleState::Stopping);
    assert!(guard.permit().is_none());
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.sequence, 2);
    assert_eq!(
        published.axis_stops[0].requested_action,
        StopAction::Hold as u8 + 1
    );
    assert_eq!(
        published.axis_stops[0].issued_action,
        StopAction::Hold as u8 + 1
    );
    let release = production
        .settle_output(&auxiliary_outputs, &stopped, 350_000)
        .unwrap();
    assert!(!release.task_released());
    assert!(!release.process_complete);
    assert_eq!(release.next.sent_frames(), 1);
    assert_eq!(release.next.expected_frames(), 2);
    assert_eq!(release.slave_copies_applied, 1);
    assert!(release.controlled_stop_used);
    assert!(!release.controlled_stop_fallback);
}

#[test]
fn generated_product_copy_plan_publishes_the_next_due_io_frame_through_the_owner() {
    const BOOT_ID: u64 = 0x2026_0927;
    const PRODUCT_IMAGE_BYTES: usize = 73;

    let procbuf =
        ProcBuf::<2, 16, 2, 64>::new(generated_product::PRODUCT_CONFIG.metadata.robot_id, BOOT_ID);
    let observed =
        generated_product::PRODUCT_CONFIG
            .slaves
            .map(|slave| esop_product_config::SlaveRecord {
                position: slave.position,
                station_address: slave.station_address,
                identity: slave.identity,
                online: true,
                configured: true,
                last_seen_cycle: 1,
                ..esop_product_config::SlaveRecord::EMPTY
            });
    let active = generated_product::PRODUCT_CONFIG
        .activate::<16, 64, 14, 2, 4, 2, 16>(
            generated_product::PRODUCT_CONFIG.metadata.config_sha256,
            &observed,
            &procbuf,
            BOOT_ID,
        )
        .unwrap();
    assert_eq!(active.slave_copy_plans().len(), 1);

    let motion_info = active.registry().domain(0).unwrap();
    let io_info = active.registry().domain(1).unwrap();
    let mut motion_segments = [DomainSegment::EMPTY; 1];
    let mut io_segments = [DomainSegment::EMPTY; 1];
    assert_eq!(
        active
            .registry()
            .copy_domain_segments(0, &mut motion_segments)
            .unwrap(),
        1
    );
    assert_eq!(
        active
            .registry()
            .copy_domain_segments(1, &mut io_segments)
            .unwrap(),
        1
    );
    let mut motion = Domain::<32, 1>::new(motion_info.config.logical_address);
    motion.add_segment(motion_segments[0]).unwrap();
    let mut io = Domain::<9, 1>::new(io_info.config.logical_address);
    io.add_segment(io_segments[0]).unwrap();
    let mut bank = ScheduledDomainBank::new_with_process_image_offsets(
        active.schedule(),
        [
            ScheduledProcessImageDomainEntry {
                id: 0,
                process_image_offset: motion_info.config.process_image_offset,
                domain: &mut motion,
            },
            ScheduledProcessImageDomainEntry {
                id: 1,
                process_image_offset: io_info.config.process_image_offset,
                domain: &mut io,
            },
        ],
    )
    .unwrap();

    let mut process_image = [0u8; PRODUCT_IMAGE_BYTES];
    process_image[19..23].copy_from_slice(&0x1234_5678i32.to_le_bytes());
    let mut published = SlaveCopyProcessImage::new(process_image);
    let motion_plan = *active.frame_plans()[0].plans().first().unwrap();
    let process_inputs = ScheduledProcessInputs::new(
        &bank,
        active.schedule(),
        [
            ScheduledProcessInputEntry {
                id: 0,
                image: &process_image,
                plans: &active.frame_plans()[0],
            },
            ScheduledProcessInputEntry {
                id: 1,
                image: &process_image,
                plans: &active.frame_plans()[1],
            },
        ],
    )
    .unwrap();
    let outputs = ScheduledAuxiliaryOutputs::new_with_shared_process_image(
        &bank,
        active.schedule(),
        0,
        &motion_plan,
        &published,
        [
            None,
            Some(SharedAuxiliaryOutputEntry {
                id: 1,
                plans: &active.frame_plans()[1],
            }),
        ],
    )
    .unwrap();
    let mut non_due_owner = ScheduledProductionCycleOwner::with_slave_copies(
        &outputs,
        active.slave_copy_plans(),
        &published,
    )
    .unwrap();

    let mut master = EthercatMaster::<4, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
        [0xFF; 6],
        [1, 2, 3, 4, 5, 6],
    ));
    let mut port = QueuedRxPort {
        inner: SimulatedPort::new(2),
        pending: [None; 4],
        read: 0,
        written: 0,
    };
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc = DcCyclicSync::new(
        DcCyclicConfig::new(0x3000, 20, 0),
        DcMonitor::new(50, 10, 1, 2),
    );
    let mut dc_image = [0u8; 8];
    let mut controls = ControlRequestPool::<1>::new();

    port.inner.set_now_ns(1_000_000);
    let first_process = bank
        .submit_due_process_inputs(
            &process_inputs,
            &mut master,
            &mut port,
            1,
            1_800_000,
            1_800_000,
        )
        .unwrap();
    assert_eq!(first_process.expected_frames, 2);
    non_due_owner
        .arm_priming(&bank, &process_inputs, &first_process, 1, 1_800_000)
        .unwrap();
    port.inner.set_response_wkc(1);
    let first_cycle = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            1_000_000,
            &mut controls,
            None,
            1,
            1_800_000,
            1_800_000,
        )
        .unwrap();
    let initial_published = *published.published();
    let non_due = non_due_owner
        .complete_control_cycle_with_slave_copies(&bank, &first_cycle, &mut published)
        .unwrap();
    assert_eq!(non_due.target_cycle(), 2);
    assert!(non_due.is_empty());
    assert_eq!(published.published_cycle(), 2);
    assert_eq!(published.published(), &initial_published);

    port.inner.set_response_wkc(2);
    for cycle in 2..=3u64 {
        let generation = cycle as u16;
        port.inner.set_now_ns(cycle * 1_000_000);
        bank.begin_due(cycle, generation).unwrap();
        let frame = master
            .acquire_frame(generation, cycle * 1_000_000 + 800_000)
            .unwrap();
        master
            .build_and_arm_frame_from_plan(frame, &motion_plan, &process_image)
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        let report = master
            .cycle_receive_with_consumer(&mut port, &mut scratch, generation, &mut bank)
            .unwrap();
        let qualities = bank.finish_due(report.cycle, generation).unwrap();
        assert!(qualities[0].valid);
    }

    let mut owner = ScheduledProductionCycleOwner::with_slave_copies(
        &outputs,
        active.slave_copy_plans(),
        &published,
    )
    .unwrap();
    port.inner.set_now_ns(4_000_000);
    let process = bank
        .submit_due_process_inputs(
            &process_inputs,
            &mut master,
            &mut port,
            4,
            4_800_000,
            4_800_000,
        )
        .unwrap();
    assert_eq!(process.expected_frames, 1);
    owner
        .arm_priming(&bank, &process_inputs, &process, 4, 4_800_000)
        .unwrap();

    port.inner.set_response_wkc(1);
    let control_cycle = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            4_000_000,
            &mut controls,
            None,
            4,
            4_800_000,
            4_800_000,
        )
        .unwrap();
    let publication = owner
        .complete_control_cycle_with_slave_copies(&bank, &control_cycle, &mut published)
        .unwrap();
    assert_eq!(publication.target_cycle(), 5);
    assert_eq!(publication.len(), 1);
    assert_eq!(
        publication.applications()[0].outcome.status,
        SlaveCopyStatus::Valid
    );
    assert_eq!(published.published_cycle(), 5);
    assert_eq!(&published.published()[66..71], &[0x78, 0x56, 0x34, 0x12, 1]);

    let mut frame = [0u8; MAX_ETHERNET_FRAME_LEN];
    let frame_len = active.frame_plans()[1].plans()[0]
        .build(
            &mut frame,
            [0xFF; 6],
            [1, 2, 3, 4, 5, 6],
            published.published(),
        )
        .unwrap();
    let parsed = FrameView::parse(&frame[..frame_len]).unwrap();
    let io_output = parsed.datagrams().next().unwrap().unwrap();
    assert_eq!(io_output.header.index, 2);
    assert_eq!(io_output.payload, &[0, 0, 0x78, 0x56, 0x34, 0x12, 1]);
    assert_eq!(owner.phase(), ScheduledProductionPhase::OutputPending);

    port.inner.set_response_wkc(2);
    for cycle in 5..=7u64 {
        let generation = cycle as u16;
        port.inner.set_now_ns(cycle * 1_000_000);
        bank.begin_due(cycle, generation).unwrap();
        let frame = master
            .acquire_frame(generation, cycle * 1_000_000 + 800_000)
            .unwrap();
        master
            .build_and_arm_frame_from_plan(frame, &motion_plan, &process_image)
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        let report = master
            .cycle_receive_with_consumer(&mut port, &mut scratch, generation, &mut bank)
            .unwrap();
        bank.finish_due(report.cycle, generation).unwrap();
    }

    let mut invalid_owner = ScheduledProductionCycleOwner::with_slave_copies(
        &outputs,
        active.slave_copy_plans(),
        &published,
    )
    .unwrap();
    port.inner.set_now_ns(8_000_000);
    port.inner.set_response_wkc(0);
    let invalid_process = bank
        .submit_due_process_inputs(
            &process_inputs,
            &mut master,
            &mut port,
            8,
            8_800_000,
            8_800_000,
        )
        .unwrap();
    invalid_owner
        .arm_priming(&bank, &process_inputs, &invalid_process, 8, 8_800_000)
        .unwrap();
    port.inner.set_response_wkc(1);
    let invalid_cycle = bank
        .run_dc_and_control_cycle(
            &mut master,
            &mut port,
            &mut scratch,
            &mut dc,
            &mut dc_image,
            8_000_000,
            &mut controls,
            None,
            8,
            8_800_000,
            8_800_000,
        )
        .unwrap();
    let invalid = invalid_owner
        .complete_control_cycle_with_slave_copies(&bank, &invalid_cycle, &mut published)
        .unwrap();
    assert_eq!(invalid.target_cycle(), 9);
    assert_eq!(invalid.len(), 1);
    assert_eq!(
        invalid.applications()[0].outcome.status,
        SlaveCopyStatus::InvalidSource
    );
    assert_eq!(&published.published()[66..71], &[0; 5]);
}

struct AdvancingTxPort<'a> {
    inner: &'a mut SimulatedPort,
    tx_duration_ns: u64,
    late_after_events: Option<(&'a ProcBuf<1, 0, 1, 8>, u64)>,
}

impl EthercatPort for AdvancingTxPort<'_> {
    type Error = PortError;

    fn link_state(&self) -> LinkState {
        self.inner.link_state()
    }

    fn now_ns(&self) -> u64 {
        self.late_after_events
            .filter(|(buffer, _)| buffer.pending_events() != 0)
            .map_or_else(|| self.inner.now_ns(), |(_, late)| late)
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
    for (final_deadline_ns, tx_duration_ns, failed_active_tx, expected_active_tx, late_events) in [
        (100_000, 0, false, false, false),
        (105_000, 10_000, false, true, false),
        (115_000, 10_000, true, false, false),
        (120_000, 0, false, false, false),
        (120_000, 0, false, true, true),
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
            late_after_events: late_events.then_some((&buffer, final_deadline_ns)),
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
                axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
                max_stationary_velocities: &[1],
                safe_process_image: &image,
                shared_process_image: None,
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
            assert_eq!(
                result.post_publication_deadline_met,
                Some(final_deadline_ns == 120_000 && !late_events)
            );
            assert_eq!(
                result.deadline_correction_publish,
                late_events.then_some(Ok(2))
            );
            assert_eq!(
                result.deadline_correction_events,
                late_events.then_some(Ok(1))
            );
            assert_eq!(context.port.inner.tx_frames(), 2);
            if final_deadline_ns == 120_000 && !late_events {
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
                if late_events {
                    let corrected = buffer.read_state().unwrap();
                    assert_eq!(corrected.publish_sequence, 2);
                    assert_eq!(
                        corrected.state.lifecycle.state,
                        LifecycleState::Stopping as u8
                    );
                    assert!(
                        !corrected
                            .state
                            .quality
                            .cyclic
                            .good(QualityFact::CycleBudget)
                    );
                    assert_eq!(corrected.state.axis_stops[0].issued_action, 0);
                    let mut last_event = None;
                    while let Some(event) = buffer.pop_event() {
                        last_event = Some(event);
                    }
                    let correction_event = last_event.expect("late stop transition must be sent");
                    assert_eq!(
                        correction_event.sequence,
                        context.guard.transition_sequence()
                    );
                    assert_eq!(correction_event.timestamp_ns, final_deadline_ns);
                    assert_eq!(correction_event.code, 2);
                }
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
                late_after_events: None,
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
                axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
                max_stationary_velocities: &[1],
                safe_process_image: &image,
                shared_process_image: None,
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &[1],
        safe_process_image: &enable_image,
        shared_process_image: None,
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
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.axis_stops[0].request_cycle, 0);
    assert_eq!(published.axes[0].statusword, 0x0040);
    assert_eq!(published.axes[0].controlword, 0x0006);
    assert_eq!(published.axes[0].position, 0.0);

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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        shared_process_image: None,
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        shared_process_image: None,
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
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.axes[0].statusword, 0x0027);
    assert_eq!(published.axes[0].controlword, 0x000F);
    assert_ne!(
        published.axes[0].quality & esop_procbuf::JointStateQuality::OPERATION_ENABLED,
        0
    );

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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        shared_process_image: None,
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
    assert_eq!(published.axes[0].controlword, CONTROLWORD_QUICK_STOP);
    assert!(matches!(failed_motion.event_publish, Some(Ok(_))));
}

#[test]
fn procbuf_command_executes_actual_hold_then_scaled_csp_target() {
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
        &csp_input_image(0x0023, 25, 0),
        false,
    );
    let first = receive(&mut master, &mut port, &mut domain, 1);

    let buffer = ProcBuf::<1, 0, 1, 8>::new(1, 7);
    let command = CommandPage {
        boot_id: 7,
        sequence: 1,
        deadline_ns: 1_000_000,
        source_id: 1,
        permit_epoch: 1,
        permit_expires_at_ns: 1_000_000,
        axis_mask: 1,
        requested_mode: ControlMode::Csp,
        motion_enable_request: 1,
        authority: 1,
        reserved: [0; 3],
        policy_version: 1,
        axes: [JointCommand {
            position: 0.5,
            velocity: 0.0,
            torque: 0.0,
            max_velocity: 10.0,
            max_torque: 1.0,
        }],
        io: [],
    };
    buffer.publish_command(command).unwrap();
    let mut command_sequence = 0;
    let command = buffer
        .read_command(100_000, &mut command_sequence)
        .unwrap()
        .command;
    assert_eq!(command_sequence, 1);

    let mut guard = LifecycleGuard::new(
        GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
    );
    guard.update_gate(GateId::Link, true, first.cycle, 0);
    let permit = motion_permit_from_command(&command).unwrap();
    guard.request_rearm(permit, first.cycle, 100_000).unwrap();

    let maps = [map()];
    let modes = [OperatingMode::Csp];
    let policies = [Cia402AxisCommandPolicy {
        position_units_per_radian: 100.0,
        velocity_units_per_radian_per_second: 10.0,
        torque_units_per_newton_metre: 10.0,
        position_offset: 0,
        min_position_radians: -2.0,
        max_position_radians: 2.0,
        max_velocity_radians_per_second: 10.0,
        max_torque_newton_metres: 5.0,
        max_position_step_radians: 0.5,
    }];
    let mut running_image = csp_input_image(0x0027, 25, 0);
    running_image[19..23].copy_from_slice(&999i32.to_le_bytes());
    let tx_before_recheck = port.tx_frames();
    {
        let decision = guard.cycle_axes(first.cycle, 150_000);
        let mut recheck_bank = Cia402AxisBank::<1>::new();
        let outputs = step_axis_bank(
            &mut recheck_bank,
            &decision,
            [0x0023],
            [DriveRequest::Enable],
        );
        let mut wrong_permit = permit;
        wrong_permit.sequence += 1;
        let mut recheck_guards = [CyclicSetpointGuard::new()];
        let limits = [CyclicLimits {
            max_position_step: 50.0,
            max_velocity: 100.0,
            max_torque: 10.0,
        }];
        assert!(matches!(
            submit_prepared_active_frame(
                &decision,
                wrong_permit,
                first,
                &outputs,
                &[Some(Cia402Target::Position(50))],
                &mut recheck_guards,
                &limits,
                &maps,
                &modes,
                &running_image,
                &domain,
                &plan,
                &mut master,
                &mut port,
                2,
                250_000,
            ),
            Err(StopFrameError::CommandPermitMismatch)
        ));
        assert!(!recheck_guards[0].seeded());
    }
    assert_eq!(port.tx_frames(), tx_before_recheck);
    assert_eq!(guard.permit(), Some(permit));

    let mut bank = Cia402AxisBank::<1>::new();
    let mut guards = [CyclicSetpointGuard::new()];
    let mut cursor = LifecycleEventCursor::new(&guard);
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
    port.set_now_ns(200_000);
    let mut first_state = StatePage::<1, 0, 1>::new(7);
    first_state.sequence = first.cycle;
    let held = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut first_state,
        report: first,
        other,
        maps: &maps,
        modes: &modes,
        axis_policies: &policies,
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        shared_process_image: None,
        plan: &plan,
        next_generation: 2,
        deadline_ns: 250_000,
        now_ns: 200_000,
        transition_time_ns: 100_000,
    }
    .run_with_procbuf_command(&command, 100_000_000, &mut guards)
    .unwrap();
    assert_eq!(held.action, LifecycleAction::EnableAllowed);
    assert!(held.transmission.is_ok());
    assert_eq!(guards[0].last().position, 25.0);
    let published = buffer.read_state().unwrap().state;
    assert!((published.axes[0].position - 0.25).abs() < f64::EPSILON);
    assert_eq!(published.axes[0].statusword, 0x0023);
    assert_eq!(published.axes[0].controlword, 0x000F);
    assert_eq!(published.axes[0].error_code, 0);
    assert_ne!(
        published.axes[0].quality & esop_procbuf::JointStateQuality::CURRENT_INPUT,
        0
    );
    assert_eq!(
        published.axes[0].quality & esop_procbuf::JointStateQuality::OPERATION_ENABLED,
        0
    );

    domain.begin_receive(2).unwrap();
    let second = receive(&mut master, &mut port, &mut domain, 2);
    assert_eq!(domain.input()[19..23], 25i32.to_le_bytes());

    port.set_now_ns(300_000);
    let mut second_state = StatePage::<1, 0, 1>::new(7);
    second_state.sequence = second.cycle;
    let running = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut second_state,
        report: second,
        other,
        maps: &maps,
        modes: &modes,
        axis_policies: &policies,
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        shared_process_image: None,
        plan: &plan,
        next_generation: 3,
        deadline_ns: 350_000,
        now_ns: 300_000,
        transition_time_ns: 100_000,
    }
    .run_with_procbuf_command(&command, 100_000_000, &mut guards)
    .unwrap();
    assert_eq!(running.action, LifecycleAction::EnableAllowed);
    assert!(running.transmission.is_ok());
    assert_eq!(guards[0].last().position, 50.0);
    let published = buffer.read_state().unwrap().state;
    assert!((published.axes[0].position - 0.25).abs() < f64::EPSILON);
    assert_eq!(published.axes[0].statusword, 0x0027);
    assert_eq!(published.axes[0].controlword, 0x000F);
    assert_ne!(
        published.axes[0].quality & esop_procbuf::JointStateQuality::OPERATION_ENABLED,
        0
    );

    domain.begin_receive(3).unwrap();
    let third = receive(&mut master, &mut port, &mut domain, 3);
    assert_eq!(domain.input()[19..23], 50i32.to_le_bytes());

    let tx_before = port.tx_frames();
    let guard_before = guards[0].last();
    let permit_before = guard.permit();
    let mut wrong_identity = command;
    wrong_identity.sequence = 2;
    let mut rejected_state = StatePage::<1, 0, 1>::new(7);
    rejected_state.sequence = third.cycle;
    let rejected = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: &domain,
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut rejected_state,
        report: third,
        other,
        maps: &maps,
        modes: &modes,
        axis_policies: &policies,
        max_stationary_velocities: &[1],
        safe_process_image: &running_image,
        shared_process_image: None,
        plan: &plan,
        next_generation: 4,
        deadline_ns: 450_000,
        now_ns: 400_000,
        transition_time_ns: 100_000,
    }
    .run_with_procbuf_command(&wrong_identity, 100_000_000, &mut guards);
    assert!(matches!(
        rejected,
        Err(StopCycleError::Command(
            esop_lifecycle_guard::procbuf::ProcBufCia402CommandError::PermitMismatch
        ))
    ));
    assert_eq!(port.tx_frames(), tx_before);
    assert_eq!(guards[0].last(), guard_before);
    assert_eq!(guard.permit(), permit_before);
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
    let mut guard = LifecycleGuard::new_with_axis_stop_policy(
        GateId::Domain.bit() | GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
        AxisStopPolicy::uniform(StopAction::Hold),
    );
    let mut cursor = LifecycleEventCursor::new(&guard);
    let buffer = ProcBuf::<1, 0, 1, 8>::new(1, 7);
    let mut bank = Cia402AxisBank::<1>::new();
    let maps = [map()];
    let modes = [OperatingMode::Csp];
    let max_stationary_velocities = [1];
    let safe_image = input_image(0x0040, 0);
    let mut port = SimulatedPort::new(1);
    let mut controlled = ControlledStopCycleState::new([ControlledStopLimits {
        max_velocity_step: 1,
        max_torque_step: 1,
        max_stationary_velocity: 1,
        max_zero_torque: 1,
    }]);

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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        shared_process_image: None,
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
            axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
            max_stationary_velocities: &max_stationary_velocities,
            safe_process_image: &safe_image,
            shared_process_image: None,
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        shared_process_image: None,
        plan: &plan,
        next_generation: 3,
        deadline_ns: 350_000,
        now_ns: 200,
        transition_time_ns: 200,
    }
    .run_with_controlled_stop(&mut controlled)
    .unwrap();
    assert!(!failed.quality.domain_valid);
    assert!(!failed.controlled_stop_used);
    assert!(failed.controlled_stop_fallback);
    assert!(
        matches!(
            failed.controlled_stop_failure,
            Some(StopFrameError::UnverifiedInput)
        ),
        "unexpected controlled failure: {:?}",
        failed.controlled_stop_failure
    );
    assert!(controlled.fallback_latched());
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        shared_process_image: None,
        plan: &plan,
        next_generation: 4,
        deadline_ns: 450_000,
        now_ns: 300,
        transition_time_ns: 200,
    }
    .run_with_controlled_stop(&mut controlled)
    .unwrap();
    assert!(sent.transmission.is_ok());
    assert!(!sent.controlled_stop_used);
    assert!(sent.controlled_stop_fallback);
    assert!(sent.controlled_stop_failure.is_none());
    assert_eq!(sent.feedback, None);
    assert!(!sent.acknowledged);
    assert_eq!(sent.state_publish, Ok(2));
    assert_eq!(sent.event_publish, Some(Ok(0)));
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.sequence, 3);
    assert_eq!(
        published.axis_stops[0].requested_action,
        StopAction::Hold as u8 + 1
    );
    assert_eq!(
        published.axis_stops[0].issued_action,
        StopAction::Disable as u8 + 1
    );
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        shared_process_image: None,
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        shared_process_image: None,
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &max_stationary_velocities,
        safe_process_image: &safe_image,
        shared_process_image: None,
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
    state.axes[0].position = 42.0;
    state.axes[0].statusword = 0x0027;
    state.axes[0].controlword = 0x1234;
    state.axes[0].error_code = 0x2310;
    state.axes[0].quality = u8::MAX;
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        shared_process_image: None,
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
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.axis_stops[0].issued_action, 0);
    assert_eq!(published.axes[0].position, 42.0);
    assert_eq!(published.axes[0].statusword, 0x0027);
    assert_eq!(published.axes[0].controlword, 0x1234);
    assert_eq!(published.axes[0].error_code, 0x2310);
    assert_eq!(published.axes[0].quality, 0);
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        shared_process_image: None,
        plan: &plan,
        next_generation: 4,
        deadline_ns: 450_000,
        now_ns: 300,
        transition_time_ns: 200,
    }
    .run()
    .unwrap();
    assert!(issued.transmission.is_ok());
    let published = buffer.read_state().unwrap().state;
    assert_eq!(published.axis_stops[0].issued_action, 3);
    assert_eq!(published.axes[0].controlword, CONTROLWORD_QUICK_STOP);
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        shared_process_image: None,
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
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &thresholds,
        safe_process_image: &safe_image,
        shared_process_image: None,
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
fn controlled_hold_commits_only_after_tx_and_publishes_the_actual_action() {
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
    let mut guard = LifecycleGuard::new_with_axis_stop_policy(
        GateId::Link.bit(),
        7,
        GuardPolicy {
            enter_good_cycles: 1,
            allowed_axis_mask: 1,
            ..GuardPolicy::conservative()
        },
        AxisStopPolicy::uniform(StopAction::Hold),
    );
    let permit = MotionPermit {
        boot_id: 7,
        source_id: 1,
        permit_epoch: 1,
        sequence: 1,
        expires_at_ns: 10_000,
        axis_mask: 1,
        authority: 1,
        reserved: [0; 3],
        policy_version: 1,
    };

    submit(
        &mut master,
        &mut port,
        &mut domain,
        &plan,
        1,
        &csp_input_image(0x0027, 120, 10),
        false,
    );
    let first = receive(&mut master, &mut port, &mut domain, 1);
    guard.update_gate(GateId::Link, true, first.cycle, 0);
    guard.request_rearm(permit, first.cycle, 100).unwrap();

    submit(
        &mut master,
        &mut port,
        &mut domain,
        &plan,
        2,
        &csp_input_image(0x0027, 120, 10),
        false,
    );
    let second = receive(&mut master, &mut port, &mut domain, 2);
    guard.update_gate(GateId::Link, false, second.cycle, 0xCAFE);
    let mut decision = guard.cycle_axes(second.cycle, 200);
    assert_eq!(
        decision.axis(0),
        esop_lifecycle_guard::AxisDirective::Stop(StopAction::Hold)
    );

    let limits = [ControlledStopLimits {
        max_velocity_step: 1,
        max_torque_step: 1,
        max_stationary_velocity: 1,
        max_zero_torque: 1,
    }];
    let mut planners = [ControlledStopPlanner::new()];
    let safe_image = csp_input_image(0x0027, 120, 0);
    port.set_now_ns(300_000);
    port.fail_next_tx();
    let failed = submit_controlled_stopping_frame(
        &mut decision,
        second,
        &mut planners,
        &limits,
        &[map],
        &[OperatingMode::Csp],
        &safe_image,
        &domain,
        &plan,
        &mut master,
        &mut port,
        3,
        350_000,
    );
    assert!(
        matches!(failed, Err(StopFrameError::Transmit(CycleError::Port(_)))),
        "unexpected controlled-stop failure: {failed:?}"
    );
    assert_eq!(planners[0].phase(), ControlledStopPhase::Idle);
    assert_eq!(decision.stop_issued_cycle(), None);

    let submitted = submit_controlled_stopping_frame(
        &mut decision,
        second,
        &mut planners,
        &limits,
        &[map],
        &[OperatingMode::Csp],
        &safe_image,
        &domain,
        &plan,
        &mut master,
        &mut port,
        3,
        350_000,
    )
    .unwrap();
    domain.begin_receive(3).unwrap();
    assert_eq!(planners[0].phase(), ControlledStopPhase::Applying);
    assert_eq!(decision.stop_issued_cycle(), Some(second.cycle));
    assert_eq!(
        submitted.controlled_commands[0].unwrap().target,
        Some(Cia402Target::Position(120))
    );
    let mut state = StatePage::<1, 0, 1>::new(7);
    state.sequence = second.cycle;
    controlled_axis_stops_to_procbuf(&mut state, &decision, &submitted, None).unwrap();
    assert_eq!(
        state.axis_stops[0].requested_action,
        StopAction::Hold as u8 + 1
    );
    assert_eq!(
        state.axis_stops[0].issued_action,
        StopAction::Hold as u8 + 1
    );

    let third = receive(&mut master, &mut port, &mut domain, 3);
    assert_eq!(&domain.input()[16..18], &0x000Fu16.to_le_bytes());
    assert_eq!(domain.input()[18], OperatingMode::Csp.raw() as u8);
    assert_eq!(&domain.input()[19..23], &120i32.to_le_bytes());

    let mut terminal = guard.cycle_axes(third.cycle, 300);
    port.set_now_ns(400_000);
    let disabled = submit_controlled_stopping_frame(
        &mut terminal,
        third,
        &mut planners,
        &limits,
        &[map],
        &[OperatingMode::Csp],
        &safe_image,
        &domain,
        &plan,
        &mut master,
        &mut port,
        4,
        450_000,
    )
    .unwrap();
    domain.begin_receive(4).unwrap();
    assert_eq!(planners[0].phase(), ControlledStopPhase::DisableRequested);
    assert_eq!(disabled.controlled_commands[0].unwrap().target, None);
    assert_eq!(disabled.outputs[0].controlword, CONTROLWORD_DISABLE_VOLTAGE);
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
    let mut auxiliary_plans = FramePlanSet::<1, 1>::new();
    auxiliary_plans
        .push(DatagramPlan {
            command: Command::Lrw,
            index: 13,
            address: 0x2000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
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
    let mut domain_bank = ScheduledDomainBank::new(
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
    let mut port = QueuedRxPort {
        inner: SimulatedPort::new(1),
        pending: [None; 4],
        read: 0,
        written: 0,
    };
    let safe_image = input_image(0x0040, 0);
    let mut initial_image = [0u8; IMAGE_BYTES + 2];
    initial_image[..IMAGE_BYTES].copy_from_slice(&safe_image);
    initial_image[IMAGE_BYTES..].copy_from_slice(&[0xAB, 0xCD]);
    port.inner.set_now_ns(100_000);
    domain_bank.begin_due(1, 1).unwrap();
    let frame = master.acquire_frame(1, 150_000).unwrap();
    master
        .build_and_arm_frame_from_plan(frame, &initial_plan, &initial_image)
        .unwrap();
    master.submit_frame(&mut port, frame).unwrap();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let first = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 1, &mut domain_bank)
        .unwrap();
    let received = domain_bank.finish_due(first.cycle, 1).unwrap();
    assert_eq!(
        domain_bank.domain::<2, 1>(10).unwrap().input(),
        &[0xAB, 0xCD]
    );
    assert!(domain_bank.domain::<2, 1>(9).is_none());
    let mut snapshots = [
        ScheduledDomainQuality {
            id: 9,
            quality: received[0],
        },
        ScheduledDomainQuality {
            id: 10,
            quality: received[1],
        },
    ];
    let auxiliary_image = [0xAB, 0xCD];
    let auxiliary_outputs = ScheduledAuxiliaryOutputs::new(
        &domain_bank,
        &schedule,
        9,
        &motion_plan,
        [
            None,
            Some(AuxiliaryOutputEntry {
                id: 10,
                image: &auxiliary_image,
                plans: &auxiliary_plans,
            }),
        ],
    )
    .unwrap();

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
            domain: domain_bank.domain::<IMAGE_BYTES, 1>(9).unwrap(),
            dc: &dc,
            buffer: &buffer,
            event_cursor: &mut cursor,
            state: &mut state,
            report: first,
            other,
            maps: &maps,
            modes: &modes,
            axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
            max_stationary_velocities: &[1],
            safe_process_image: &safe_image,
            shared_process_image: None,
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
            .run_scheduled_with_outputs_until(
                &schedule,
                &snapshots,
                9,
                &auxiliary_outputs,
                &[None],
                &mut guards,
                &limits,
                150_000,
            )
            .unwrap();
        assert_eq!(sent.action, LifecycleAction::EnableAllowed);
        assert!(sent.transmission.is_ok());
        assert_eq!(sent.auxiliary_frames_sent, 0);
        assert!(sent.auxiliary_failure.is_none());
        assert!(sent.quality.domain_valid);
        assert_eq!(sent.post_tx_deadline_met, Some(true));
        assert_eq!(sent.state_publish, Ok(1));
    }

    port.inner.set_now_ns(200_000);
    domain_bank.begin_due(2, 2).unwrap();
    let second = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 2, &mut domain_bank)
        .unwrap();
    let received = domain_bank.finish_due(second.cycle, 2).unwrap();
    snapshots[0].quality = received[0];
    snapshots[1].quality = received[1];
    let mut state = StatePage::<1, 0, 2>::new(7);
    state.sequence = second.cycle;
    // Cycle 2 must arm the period-2 auxiliary frame for cycle 3. Drop that
    // response while retaining the following motion response in the queue.
    port.inner.drop_next_response();
    let idle = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: domain_bank.domain::<IMAGE_BYTES, 1>(9).unwrap(),
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: second,
        other,
        maps: &maps,
        modes: &modes,
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &[1],
        safe_process_image: &safe_image,
        shared_process_image: None,
        plan: &motion_plan,
        next_generation: 3,
        deadline_ns: 350_000,
        now_ns: 200_000,
        transition_time_ns: 100_000,
    }
    .run_scheduled_with_outputs_until(
        &schedule,
        &snapshots,
        9,
        &auxiliary_outputs,
        &[None],
        &mut guards,
        &limits,
        250_000,
    )
    .unwrap();
    assert_eq!(idle.action, LifecycleAction::EnableAllowed);
    assert!(idle.transmission.is_ok());
    assert_eq!(idle.auxiliary_frames_sent, 1);
    assert!(idle.quality.domain_valid);
    assert_eq!(idle.post_tx_deadline_met, Some(true));
    assert_eq!(
        buffer.read_state().unwrap().state.quality.domains[1].input_age_cycles,
        1
    );

    port.inner.set_now_ns(300_000);
    domain_bank.begin_due(3, 3).unwrap();
    let third = master
        .cycle_receive_with_consumer(&mut port, &mut scratch, 3, &mut domain_bank)
        .unwrap();
    let received = domain_bank.finish_due(third.cycle, 3).unwrap();
    snapshots[0].quality = received[0];
    snapshots[1].quality = received[1];
    let mut state = StatePage::<1, 0, 2>::new(7);
    state.sequence = third.cycle;
    let failed = StopCycleContext {
        guard: &mut guard,
        bank: &mut bank,
        master: &mut master,
        port: &mut port,
        domain: domain_bank.domain::<IMAGE_BYTES, 1>(9).unwrap(),
        dc: &dc,
        buffer: &buffer,
        event_cursor: &mut cursor,
        state: &mut state,
        report: third,
        other,
        maps: &maps,
        modes: &modes,
        axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
        max_stationary_velocities: &[1],
        safe_process_image: &safe_image,
        shared_process_image: None,
        plan: &motion_plan,
        next_generation: 4,
        deadline_ns: 450_000,
        now_ns: 300_000,
        transition_time_ns: 100_000,
    }
    .run_scheduled_with_outputs_until(
        &schedule,
        &snapshots,
        9,
        &auxiliary_outputs,
        &[None],
        &mut guards,
        &limits,
        350_000,
    )
    .unwrap();
    assert!(matches!(failed.action, LifecycleAction::Stop(_)));
    assert!(failed.transmission.is_ok());
    assert_eq!(failed.auxiliary_frames_sent, 0);
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
fn due_auxiliary_output_failure_or_overrun_blocks_active_motion() {
    for (fail_tx, tx_duration_ns, cycle_deadline_ns) in
        [(true, 0, 150_000), (false, 10_000, 105_000)]
    {
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
        let mut auxiliary = Domain::<2, 2>::new(0x2000);
        for (index, offset) in [(13, 0), (14, 1)] {
            auxiliary
                .add_segment(DomainSegment {
                    datagram_index: index,
                    input_offset: offset,
                    len: 1,
                    expected_wkc: 1,
                })
                .unwrap();
        }
        let motion_datagram = DatagramPlan {
            command: Command::Lrw,
            index: 12,
            address: 0x1000,
            payload_offset: 0,
            payload_len: IMAGE_BYTES,
            expected_wkc: 1,
        };
        let mut motion_plan = FramePlan::<1>::new();
        motion_plan.push(motion_datagram).unwrap();
        let mut initial_plan = FramePlan::<3>::new();
        initial_plan.push(motion_datagram).unwrap();
        let mut auxiliary_plans = FramePlanSet::<2, 1>::new();
        for (index, offset) in [(13, 0), (14, 1)] {
            initial_plan
                .push(DatagramPlan {
                    command: Command::Lrw,
                    index,
                    address: 0x2000 + offset as u32,
                    payload_offset: IMAGE_BYTES + offset,
                    payload_len: 1,
                    expected_wkc: 1,
                })
                .unwrap();
            auxiliary_plans
                .push(DatagramPlan {
                    command: Command::Lrw,
                    index,
                    address: 0x2000 + offset as u32,
                    payload_offset: offset,
                    payload_len: 1,
                    expected_wkc: 1,
                })
                .unwrap();
        }
        assert_eq!(auxiliary_plans.frame_count(), 2);
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
                    period_ticks: 1,
                    phase_ticks: 0,
                },
            ],
        )
        .unwrap();
        let mut domain_bank = ScheduledDomainBank::new(
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
        let safe_image = input_image(0x0040, 0);
        let auxiliary_image = [0xAB, 0xCD];
        assert!(matches!(
            ScheduledAuxiliaryOutputs::new(
                &domain_bank,
                &schedule,
                9,
                &motion_plan,
                [
                    None,
                    Some(AuxiliaryOutputEntry {
                        id: 10,
                        image: &[0],
                        plans: &auxiliary_plans
                    })
                ],
            ),
            Err(AuxiliaryOutputPlanError::InvalidPlan(10))
        ));
        let outputs = ScheduledAuxiliaryOutputs::new(
            &domain_bank,
            &schedule,
            9,
            &motion_plan,
            [
                None,
                Some(AuxiliaryOutputEntry {
                    id: 10,
                    image: &auxiliary_image,
                    plans: &auxiliary_plans,
                }),
            ],
        )
        .unwrap();

        let mut port = SimulatedPort::new(1);
        port.set_now_ns(100_000);
        domain_bank.begin_due(1, 1).unwrap();
        let mut initial_image = [0; IMAGE_BYTES + 2];
        initial_image[..IMAGE_BYTES].copy_from_slice(&safe_image);
        initial_image[IMAGE_BYTES..].copy_from_slice(&auxiliary_image);
        let frame = master.acquire_frame(1, 150_000).unwrap();
        master
            .build_and_arm_frame_from_plan(frame, &initial_plan, &initial_image)
            .unwrap();
        master.submit_frame(&mut port, frame).unwrap();
        let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
        let report = master
            .cycle_receive_with_consumer(&mut port, &mut scratch, 1, &mut domain_bank)
            .unwrap();
        let received = domain_bank.finish_due(report.cycle, 1).unwrap();
        let snapshots = [
            ScheduledDomainQuality {
                id: 9,
                quality: received[0],
            },
            ScheduledDomainQuality {
                id: 10,
                quality: received[1],
            },
        ];
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
        guard.update_cyclic_quality(
            cyclic_quality_from_schedule(report, &schedule, &snapshots, &dc, other),
            report.cycle,
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
                report.cycle,
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
        state.sequence = report.cycle;
        if fail_tx {
            port.fail_next_tx();
        }
        let mut timed_port = AdvancingTxPort {
            inner: &mut port,
            tx_duration_ns,
            late_after_events: None,
        };
        let outcome = StopCycleContext {
            guard: &mut guard,
            bank: &mut bank,
            master: &mut master,
            port: &mut timed_port,
            domain: domain_bank.domain::<IMAGE_BYTES, 1>(9).unwrap(),
            dc: &dc,
            buffer: &buffer,
            event_cursor: &mut cursor,
            state: &mut state,
            report,
            other,
            maps: &maps,
            modes: &modes,
            axis_policies: &core::array::from_fn(|_| FEEDBACK_POLICY),
            max_stationary_velocities: &[1],
            safe_process_image: &safe_image,
            shared_process_image: None,
            plan: &motion_plan,
            next_generation: 2,
            deadline_ns: 250_000,
            now_ns: 100_000,
            transition_time_ns: 100_000,
        }
        .run_scheduled_with_outputs_until(
            &schedule,
            &snapshots,
            9,
            &outputs,
            &[None],
            &mut guards,
            &limits,
            cycle_deadline_ns,
        )
        .unwrap();
        assert!(matches!(outcome.action, LifecycleAction::Stop(_)));
        assert!(outcome.transmission.is_ok());
        assert!(guard.permit().is_none());
        assert_eq!(guard.state(), LifecycleState::Stopping);
        assert_eq!(outcome.state_publish, Ok(1));
        let published = buffer.read_state().unwrap().state;
        assert_ne!(published.axis_stops[0].issued_action, 0);
        assert!(!outcome.process_handoff.unwrap().complete());
        if fail_tx {
            let failure = outcome.auxiliary_failure.unwrap();
            assert_eq!(failure.domain_id, 10);
            assert_eq!(failure.frame_index, 0);
            assert!(matches!(
                failure.error,
                StopFrameError::Transmit(CycleError::Port(PortError::HardwareFault))
            ));
            assert_eq!(outcome.auxiliary_frames_sent, 0);
            assert_eq!(outcome.post_tx_deadline_met, Some(true));
            assert_eq!(published.lifecycle.first_blocking_code, 0x5458_0002);
            assert_eq!(port.tx_frames(), 2);
        } else {
            let failure = outcome.auxiliary_failure.unwrap();
            assert_eq!(failure.domain_id, 10);
            assert_eq!(failure.frame_index, 1);
            assert!(matches!(failure.error, StopFrameError::InvalidDeadline));
            assert_eq!(outcome.auxiliary_frames_sent, 1);
            assert_eq!(outcome.post_tx_deadline_met, Some(false));
            assert!(!outcome.quality.cycle_within_budget);
            assert!(!guard.gate(GateId::Budget).valid);
            assert_eq!(port.tx_frames(), 3);
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
