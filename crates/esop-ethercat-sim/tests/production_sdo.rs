//! Production scheduler integration over independent ESC wire responses.
//! Host test traces allocate; the protocol/scheduler path uses its fixed banks.
use esop_ethercat_core::wire::{Command, FrameBuilder, FrameView, MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_core::*;
use esop_ethercat_sim::{Fault, SimError, VirtualBus, fixtures};
use esop_lifecycle_guard::ethercat::{
    OtherCycleFacts, other_cycle_facts_from_production_service_cycle,
};

const PERIOD: u64 = 1_000_000;
const LEASES: [u8; 8] = [220, 221, 222, 223, 224, 225, 226, 227];
const CYCLIC: [u8; 6] = [0, 1, 2, 3, 240, 241];

mod product {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-dual-axis/expected/esop_product_config.rs"
    ));
}

#[derive(Debug)]
struct Trace {
    at: u64,
    command: Command,
    index: u8,
    address: u32,
    payload: Vec<u8>,
}

struct TestPort {
    bus: VirtualBus<3>,
    trace: Vec<Trace>,
    next_mailbox_read_fault: Option<Fault>,
    fault_at: Option<u64>,
    fault_station: u16,
}

impl EthercatPort for TestPort {
    type Error = SimError;
    fn now_ns(&self) -> u64 {
        self.bus.now_ns()
    }
    fn link_state(&self) -> LinkState {
        self.bus.link_state()
    }
    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, SimError> {
        self.bus.rx_poll(destination)
    }
    fn tx_submit(&mut self, bytes: &[u8]) -> Result<(), SimError> {
        let frame = FrameView::parse(bytes).map_err(|_| SimError::InvalidFrame)?;
        for datagram in frame.datagrams() {
            let datagram = datagram.map_err(|_| SimError::InvalidFrame)?;
            if LEASES.contains(&datagram.header.index)
                && datagram.header.command == Command::Fprd
                && datagram.header.address == fixed_address(self.fault_station, 0x1100)
                && let Some(fault) = self.next_mailbox_read_fault.take()
            {
                self.bus.inject(fault)?;
                self.fault_at = Some(self.now_ns());
            }
            self.trace.push(Trace {
                at: self.now_ns(),
                command: datagram.header.command,
                index: datagram.header.index,
                address: datagram.header.address,
                payload: datagram.payload.to_vec(),
            });
        }
        self.bus.tx_submit(bytes)
    }
}

fn wire(port: &mut TestPort, command: Command, address: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = [0; MAX_ETHERNET_FRAME_LEN];
    let mut builder = FrameBuilder::new(&mut bytes, [0xff; 6], [2, 0, 0, 0, 0, 1]).unwrap();
    builder.push(command, 250, address, payload).unwrap();
    let len = builder.finish().unwrap();
    port.tx_submit(&bytes[..len]).unwrap();
    let RxPoll::Frame(len) = port.rx_poll(&mut bytes).unwrap() else {
        panic!("missing configuration reply")
    };
    let frame = FrameView::parse(&bytes[..len]).unwrap();
    let reply = frame.datagrams().next().unwrap().unwrap();
    assert_eq!(reply.working_counter, 1);
    reply.payload.to_vec()
}

fn physical_bus() -> TestPort {
    let mut port = TestPort {
        bus: VirtualBus::new([
            fixtures::drive(1, 0),
            fixtures::drive(2, 0),
            fixtures::drive(3, 0),
        ])
        .unwrap(),
        trace: Vec::new(),
        next_mailbox_read_fault: None,
        fault_at: None,
        fault_station: 0x1001,
    };
    for position in 0..3 {
        let station = 0x1001 + position as u16;
        wire(
            &mut port,
            Command::Apwr,
            fixed_address(0u16.wrapping_sub(position as u16), 0x10),
            &station.to_le_bytes(),
        );
        // Configure a real input SyncManager and FMMU through register writes.
        let mut image = [0; 16];
        SyncManagerConfig {
            index: 3,
            physical_start: 0x1500,
            length: 2,
            control: 0x20,
            status: 0,
            enable: true,
        }
        .encode(&mut image[..8])
        .unwrap();
        wire(
            &mut port,
            Command::Fpwr,
            fixed_address(station, 0x0818),
            &image[..8],
        );
        FmmuConfig {
            index: 0,
            logical_start: if position == 2 {
                0x1020
            } else {
                0x1000 + position as u32 * 2
            },
            length: 2,
            logical_start_bit: 0,
            logical_end_bit: 7,
            physical_start: 0x1500,
            physical_start_bit: 0,
            fmmu_type: 1,
            enable: true,
        }
        .encode(&mut image)
        .unwrap();
        wire(
            &mut port,
            Command::Fpwr,
            fixed_address(station, 0x0600),
            &image,
        );
        wire(
            &mut port,
            Command::Fpwr,
            fixed_address(station, 0x0120),
            &[2, 0],
        );
        wire(
            &mut port,
            Command::Fpwr,
            fixed_address(station, 0x0120),
            &[4, 0],
        );
        assert_eq!(
            wire(
                &mut port,
                Command::Fprd,
                fixed_address(station, 0x0130),
                &[0; 6]
            )[0],
            4
        );
    }
    port.trace.clear();
    port
}

fn mailbox_config(timeout: u64) -> MailboxConfig {
    let mut config = MailboxConfig::new(0x1000, 64, 0x1100, 64);
    config.request_timeout_ns = timeout;
    config.timeout_ns = 5_000_000_000;
    config.poll_interval_ns = PERIOD;
    config
}

fn retained_startup(port: &mut TestPort) -> StartupController<3> {
    let expected = core::array::from_fn::<_, 3, _>(|position| ExpectedSlave {
        position: position as u16,
        station_address: 0x1001 + position as u16,
        identity: SlaveIdentity {
            vendor_id: 0xe500,
            product_code: 0x4020,
            revision: 1,
            serial: position as u32 + 1,
        },
    });
    let profiles = core::array::from_fn::<_, 3, _>(|position| {
        let mut profile = product::PRODUCT_CONFIG.startup_profiles().unwrap()[0];
        profile.position = position as u16;
        profile.dc_requirement = StartupDcRequirement::None;
        profile.expected_dc_mode = None;
        profile.expected_requesting_id = None;
        profile.expected_mailbox = Some(
            mailbox_config(10 * PERIOD)
                .with_status_bit(MailboxStatusBit::sync_manager_mailbox_full(1)),
        );
        profile
    });
    let mut startup = StartupController::new(0x1001);
    let mut config = StartupConfig::new(EthercatState::SafeOp);
    config.request_timeout_ns = 100 * PERIOD;
    startup
        .start_with_profiles(1, port.now_ns(), config, &expected, &profiles)
        .unwrap();
    for _ in 0..10_000 {
        if startup.phase() == StartupPhase::Ready {
            return startup;
        }
        port.bus.advance_by(10_000).unwrap();
        let Some(action) = startup.next_action(port.now_ns()).unwrap() else {
            continue;
        };
        let command = match action.operation() {
            RegisterOperation::Read => Command::Fprd,
            RegisterOperation::Write => Command::Fpwr,
            RegisterOperation::AutoIncrementRead => Command::Aprd,
            RegisterOperation::AutoIncrementWrite => Command::Apwr,
            RegisterOperation::BroadcastRead => Command::Brd,
        };
        let mut tx = [0; MAX_ETHERNET_FRAME_LEN];
        let mut data = [0; 256];
        data[..action.payload().len()].copy_from_slice(action.payload());
        let mut frame = FrameBuilder::new(&mut tx, [0xff; 6], [2, 0, 0, 0, 0, 1]).unwrap();
        frame
            .push(
                command,
                action.datagram_index(),
                action.address(),
                &data[..action.datagram_len()],
            )
            .unwrap();
        let len = frame.finish().unwrap();
        port.tx_submit(&tx[..len]).unwrap();
        let RxPoll::Frame(len) = port.rx_poll(&mut tx).unwrap() else {
            panic!("startup response missing")
        };
        let parsed = FrameView::parse(&tx[..len]).unwrap();
        let datagram = parsed.datagrams().next().unwrap().unwrap();
        startup
            .accept(
                action,
                action.generation(),
                &datagram.payload[..action.response_len()],
                datagram.working_counter,
                port.now_ns(),
            )
            .unwrap();
    }
    panic!("startup never produced retained evidence")
}

#[derive(Clone, Copy)]
enum Case {
    Eight,
    Abort,
    Delayed,
    Dropped,
    IdleEmergency,
    Mapped,
    MappedIdle,
    Handoff,
    Continuous,
    QueuedExpiry,
    LegacyMapped,
    LegacyTimeout,
}

fn ready_facts() -> OtherCycleFacts {
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

fn production(case: Case) {
    let mut port = physical_bus();
    let handoff = matches!(case, Case::Handoff);
    let mut startup = handoff.then(|| retained_startup(&mut port));
    let legacy = matches!(case, Case::LegacyMapped | Case::LegacyTimeout);
    let runtime_slaves = if legacy { 2 } else { 3 };
    let mapped = matches!(case, Case::Mapped | Case::MappedIdle | Case::LegacyMapped);
    let idle = matches!(case, Case::IdleEmergency | Case::MappedIdle);
    let status_fmmu = FmmuConfig {
        index: 1,
        logical_start: 0x1004,
        length: 1,
        logical_start_bit: 0,
        logical_end_bit: 0,
        physical_start: 0x080D,
        physical_start_bit: 3,
        fmmu_type: 1,
        enable: true,
    };
    if mapped {
        let mut bytes = [0; 16];
        status_fmmu.encode(&mut bytes).unwrap();
        wire(
            &mut port,
            Command::Fpwr,
            fixed_address(if legacy { 0x1003 } else { 0x1001 }, 0x0610),
            &bytes,
        );
    }
    let schedule = ScheduleTable::<1, 1>::build(
        PERIOD,
        &[ScheduleDomain {
            id: 0,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    // Station 2 remains cyclic while a different physical slave is serviced.
    let mut domain = Domain::<3, 1>::new(0x1002);
    domain
        .add_segment(DomainSegment {
            datagram_index: 0,
            input_offset: 0,
            len: if mapped { 3 } else { 2 },
            expected_wkc: if mapped { 2 } else { 1 },
        })
        .unwrap();
    let mut domains = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 0,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let mut plans = FramePlanSet::<1, 1>::new();
    plans
        .push(DatagramPlan {
            command: Command::Lrd,
            index: 0,
            address: 0x1002,
            payload_offset: 0,
            payload_len: if mapped { 3 } else { 2 },
            expected_wkc: if mapped { 2 } else { 1 },
        })
        .unwrap();
    let image = [0; 3];
    let inputs = ScheduledProcessInputs::new(
        &domains,
        &schedule,
        [ScheduledProcessInputEntry {
            id: 0,
            image: &image,
            plans: &plans,
        }],
    )
    .unwrap();
    let mut master = EthercatMaster::<8, MAX_ETHERNET_FRAME_LEN>::new_with_bus_transit_count(
        MasterConfig::new([0xff; 6], [2, 0, 0, 0, 0, 1]),
        3,
    );
    let mut config = DcCyclicConfig::new(0x1001, 240, 0);
    config.expected_wkc = 3;
    let mut dc =
        DcCyclicSync::new_with_bus_transit_count(config, DcMonitor::new(100, 100, 1, 2), 3)
            .with_sync_window(DcSyncWindowConfig::new(241, 8, 3, 100, 1, 2).unwrap())
            .unwrap();
    let mut dc_image = [0; 12];
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut controls =
        ControlRequestPool::<8>::new_with_datagram_indices_excluding(&LEASES, &CYCLIC).unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut runtime = SdoRequestBank::<3>::new();
    let timeout = if matches!(case, Case::Delayed | Case::Dropped) {
        10 * PERIOD
    } else {
        PERIOD
    };
    let config = if handoff {
        startup.as_ref().unwrap().verified_mailbox(0).unwrap()
    } else {
        mailbox_config(timeout)
    };
    for slot in 0..if handoff { 0 } else { runtime_slaves } {
        let config = if mapped && !legacy && slot == 0 {
            config.with_status_bit(MailboxStatusBit::sync_manager_mailbox_full(1))
        } else {
            config
        };
        runtime
            .bind_verified_slave(slot, 0x1001 + slot as u16, config)
            .unwrap();
    }
    if mapped && !legacy {
        runtime
            .bind_verified_mapped_status(0x1001, MailboxMappedStatusBit::new(0, 16, 2, status_fmmu))
            .unwrap();
    }
    let mut monitor = SlaveMonitorController::<3>::new(
        40,
        &(0..3)
            .map(|position| SlaveMonitorConfig {
                position,
                station_address: 0x1001 + position,
                required_al_state: EthercatState::SafeOp,
                required_dl_mask: 0,
            })
            .collect::<Vec<_>>(),
        PERIOD,
        100 * PERIOD,
        10 * PERIOD,
        0,
    )
    .unwrap();
    let mut registers = EscRegisterRequestController::<2>::new(41);
    let mut diagnostic = registers
        .submit_read(0x1002, 0, 2, port.now_ns(), 5_000 * PERIOD)
        .unwrap();
    let mut configuration = PdoConfigBatch::<3, 1>::new();
    if handoff {
        let mut plan = PdoConfigBatchPlan::new();
        for position in 0..3 {
            let mut writes = PdoConfigPlan::new();
            writes
                .push(PdoSdoWrite::new(0x2007, 0, &[7; 4]).unwrap())
                .unwrap();
            plan.push(PdoConfigJob::new(0x1001 + position, writes, config))
                .unwrap();
        }
        configuration
            .start(plan, 20, port.now_ns(), 1_000 * PERIOD, 100 * PERIOD)
            .unwrap();
        for generation in 1..500 {
            port.bus.advance_by(PERIOD).unwrap();
            let now = port.now_ns();
            domains
                .submit_due_process_inputs(
                    &inputs,
                    &mut master,
                    &mut port,
                    generation,
                    now + PERIOD - 1,
                    now + PERIOD - 1,
                )
                .unwrap();
            let report = scheduler
                .run_cycle(
                    &mut domains,
                    &mut master,
                    &mut port,
                    &mut scratch,
                    &mut dc,
                    &mut dc_image,
                    now,
                    &mut controls,
                    &mut ScheduledProductionServices::<3, 0, 0, 1, 3>::new(
                        startup.as_mut(),
                        None,
                        None,
                        None,
                    )
                    .with_pdo_configuration(ScheduledPdoConfiguration::batch(&mut configuration)),
                    generation,
                    now + PERIOD - 1,
                    now + PERIOD - 1,
                )
                .unwrap();
            assert_eq!(report.fault(), None);
            if configuration.phase() == PdoConfigBatchPhase::Complete {
                break;
            }
        }
        assert_eq!(configuration.phase(), PdoConfigBatchPhase::Complete);
        for slot in 0..3 {
            let station = 0x1001 + slot as u16;
            runtime
                .bind_verified_slave_with_transport(
                    slot,
                    station,
                    config,
                    configuration.take_transport_state(station).unwrap(),
                )
                .unwrap();
        }
        assert!(!configuration.mailbox().owns_transport());
    }
    port.trace.clear();
    let origin = port.now_ns();
    let mut legacy_mailbox = legacy.then(MailboxController::new);
    if let Some(mailbox) = legacy_mailbox.as_mut() {
        let mut config = mailbox_config(2 * PERIOD)
            .with_status_bit(MailboxStatusBit::sync_manager_mailbox_full(1));
        config.timeout_ns = 100 * PERIOD;
        let request = [0, 0x20, 0x40, 0, 0x20, 0, 0, 0, 0, 0];
        if mapped {
            mailbox
                .start_with_mapped_status(
                    config,
                    0x1003,
                    900,
                    origin,
                    MailboxProtocol::CoE,
                    &request,
                )
                .unwrap();
        } else {
            mailbox
                .start(config, 0x1003, 900, origin, MailboxProtocol::CoE, &request)
                .unwrap();
            port.fault_station = 0x1003;
        }
    }
    let mut handles = Vec::new();
    if !idle {
        for object in 0..8u16 {
            let bytes: Vec<u8> = (0..1024)
                .map(|index| (index as u8).wrapping_add(object as u8))
                .collect();
            port.bus
                .set_object(object as usize % runtime_slaves, 0x2000 + object, 0, &bytes)
                .unwrap();
            handles.push(
                runtime
                    .submit_upload(
                        0x1001 + object % runtime_slaves as u16,
                        if matches!(case, Case::Abort) && object == 0 {
                            0x7777
                        } else {
                            0x2000 + object
                        },
                        0,
                        SdoAccess::Single,
                        SdoAccessPolicy::new(false),
                        origin,
                        origin
                            + if matches!(case, Case::QueuedExpiry) {
                                PERIOD
                            } else {
                                5_000 * PERIOD
                            },
                    )
                    .unwrap(),
            );
        }
    } else {
        port.bus.emergency(0, 0x2310, 2, [1, 2, 3, 4, 5]).unwrap();
    }
    port.next_mailbox_read_fault = match case {
        Case::Delayed => Some(Fault::DelayNextResponse(2 * PERIOD)),
        Case::Dropped => Some(Fault::DropNextResponse),
        Case::LegacyTimeout => Some(Fault::DropNextResponse),
        _ => None,
    };
    let mut terminal_events = Vec::new();
    let mut register_completed_at = None;
    let mut all_completed_at = None;
    let mut emergency_consumed = false;
    let mut diagnostic_completions = 0;
    let mut other_cycles_after_legacy_fault = 0;
    for tick in 1..=5_100u16 {
        let now = origin + u64::from(tick) * PERIOD;
        port.bus.advance_to(now).unwrap();
        domains
            .submit_due_process_inputs(
                &inputs,
                &mut master,
                &mut port,
                tick,
                now + PERIOD - 1,
                now + PERIOD - 1,
            )
            .unwrap();
        let mut services =
            ScheduledProductionServices::<3, 0, 0>::new(None, None, None, legacy_mailbox.as_mut())
                .with_slave_monitor(&mut monitor)
                .with_runtime_sdo(&mut runtime)
                .with_register_requests(&mut registers);
        if legacy && mapped {
            services = services.with_mapped_mailbox_status(MailboxMappedStatusBit::new(
                0,
                16,
                2,
                status_fmmu,
            ));
        }
        let report = scheduler
            .run_cycle(
                &mut domains,
                &mut master,
                &mut port,
                &mut scratch,
                &mut dc,
                &mut dc_image,
                now,
                &mut controls,
                &mut services,
                tick,
                now + PERIOD - 1,
                now + PERIOD - 1,
            )
            .unwrap();
        assert!(
            report
                .received()
                .qualities
                .iter()
                .all(|quality| quality.valid)
        );
        assert!(report.received().dc_result.is_ok());
        if report.selected() == ScheduledProductionServiceKind::RuntimeSdo {
            assert!(
                report.service_ready(),
                "runtime failure may not close Configuration: {report:?}"
            );
        }
        if let Some(status) = report.runtime_sdo_terminal() {
            terminal_events.push(status);
        }
        assert_eq!(report.runtime_sdo_lost_events(), 0);
        if legacy_mailbox
            .as_ref()
            .is_some_and(|mailbox| mailbox.phase() == MailboxPhase::Faulted)
        {
            assert_eq!(report.legacy_mailbox_ready(), Some(false));
            assert!(
                !other_cycle_facts_from_production_service_cycle(&report, ready_facts()).coe_ready
            );
            if report.selected() != ScheduledProductionServiceKind::Mailbox {
                other_cycles_after_legacy_fault += 1;
            }
        }
        if registers.status(diagnostic).unwrap().state == EscRegisterRequestState::Success {
            if register_completed_at.is_none() || matches!(case, Case::Continuous) {
                diagnostic_completions += 1;
            }
            register_completed_at.get_or_insert(tick);
            if matches!(case, Case::Continuous) {
                registers.release(diagnostic).unwrap();
                diagnostic = registers
                    .submit_read(0x1002, 0, 2, now, 100 * PERIOD)
                    .unwrap();
            }
        }
        if idle {
            emergency_consumed |= runtime.mailbox(0x1001).unwrap().pop_emergency().is_some();
            if emergency_consumed && register_completed_at.is_some() {
                break;
            }
        } else if handles.iter().all(|handle| {
            matches!(
                runtime.status(*handle).unwrap().phase,
                SdoRequestPhase::Complete | SdoRequestPhase::Failed
            )
        }) {
            all_completed_at = Some(tick);
            if !matches!(case, Case::QueuedExpiry) || terminal_events.len() == 8 {
                break;
            }
        }
    }
    assert!(
        register_completed_at.is_some(),
        "lower-priority register diagnostics starved"
    );
    if idle {
        assert!(emergency_consumed);
    }
    if !idle {
        assert!(
            all_completed_at.is_some(),
            "runtime SDO requests made no bounded progress"
        );
        assert!(register_completed_at.unwrap() < all_completed_at.unwrap());
        for (object, handle) in handles.iter().copied().enumerate() {
            if matches!(case, Case::QueuedExpiry) {
                assert_eq!(
                    runtime.status(handle).unwrap().phase,
                    SdoRequestPhase::Failed
                );
                assert_eq!(runtime.result(handle), Err(SdoRequestError::Timeout));
            } else if object == 0 && matches!(case, Case::Abort | Case::Dropped) {
                assert_eq!(
                    runtime.status(handle).unwrap().phase,
                    SdoRequestPhase::Failed
                );
            } else {
                let data = runtime
                    .result(handle)
                    .unwrap_or_else(|error| panic!("object {object}: {error:?}"));
                assert_eq!(data.len(), 1024);
                assert!(
                    data.iter()
                        .enumerate()
                        .all(|(index, byte)| *byte == (index as u8).wrapping_add(object as u8))
                );
            }
        }
        assert_eq!(
            terminal_events.len(),
            8,
            "terminal reports are immutable and emitted once"
        );
        for handle in &handles {
            assert_eq!(
                terminal_events
                    .iter()
                    .filter(|event| event.handle == *handle)
                    .count(),
                1
            );
        }
    }
    if matches!(case, Case::Continuous) {
        assert!(
            diagnostic_completions >= 10,
            "continuous diagnostics made too little progress"
        );
        assert!(
            monitor
                .snapshots()
                .iter()
                .all(|snapshot| snapshot.known && snapshot.sample_sequence > 1)
        );
    }
    for at in 1..=u64::from(all_completed_at.unwrap_or(register_completed_at.unwrap())) {
        let traces: Vec<_> = port
            .trace
            .iter()
            .filter(|entry| entry.at == origin + at * PERIOD)
            .collect();
        assert_eq!((traces[0].command, traces[0].index), (Command::Lrd, 0));
        assert_eq!((traces[1].command, traces[1].index), (Command::Frmw, 240));
        assert_eq!((traces[2].command, traces[2].index), (Command::Brd, 241));
        assert!(traces.len() <= 4);
        if traces.len() == 4 {
            assert!(LEASES.contains(&traces[3].index));
        }
    }
    if matches!(case, Case::Delayed) {
        let at = port.fault_at.unwrap();
        assert_eq!(
            port.trace
                .iter()
                .filter(|entry| entry.at >= at
                    && entry.at < at + 2 * PERIOD
                    && LEASES.contains(&entry.index))
                .count(),
            1,
            "in-flight control work was retransmitted or preempted"
        );
    }
    if mapped {
        assert!(
            !port.trace.iter().any(|entry| LEASES.contains(&entry.index)
                && entry.command == Command::Fprd
                && entry.address == fixed_address(if legacy { 0x1003 } else { 0x1001 }, 0x080D)),
            "mapped status may not silently fall back to a direct status read"
        );
    }
    if matches!(case, Case::QueuedExpiry) {
        assert!(
            !port
                .trace
                .iter()
                .any(|entry| entry.command == Command::Fpwr && entry.address >> 16 == 0x1000)
        );
        assert_eq!(runtime.lost_terminal_statuses(), 0);
    }
    if matches!(case, Case::LegacyMapped) {
        assert_eq!(
            legacy_mailbox.as_ref().unwrap().phase(),
            MailboxPhase::Complete
        );
        assert_eq!(
            SdoResponse::parse(legacy_mailbox.as_ref().unwrap().response().unwrap().1)
                .unwrap()
                .data,
            &[0; 4]
        );
    }
    if matches!(case, Case::LegacyTimeout) {
        assert!(
            other_cycles_after_legacy_fault > 3,
            "a faulted required mailbox may not own the optional transport"
        );
        port.bus.advance_by(PERIOD).unwrap();
        let now = port.now_ns();
        domains
            .submit_due_process_inputs(
                &inputs,
                &mut master,
                &mut port,
                20_000,
                now + PERIOD - 1,
                now + PERIOD - 1,
            )
            .unwrap();
        let idle = scheduler
            .run_cycle(
                &mut domains,
                &mut master,
                &mut port,
                &mut scratch,
                &mut dc,
                &mut dc_image,
                now,
                &mut controls,
                &mut ScheduledProductionServices::<0, 0, 0>::new(
                    None,
                    None,
                    None,
                    legacy_mailbox.as_mut(),
                ),
                20_000,
                now + PERIOD - 1,
                now + PERIOD - 1,
            )
            .unwrap();
        assert_eq!(idle.selected(), ScheduledProductionServiceKind::Idle);
        assert_eq!(idle.legacy_mailbox_ready(), Some(false));
        assert!(!other_cycle_facts_from_production_service_cycle(&idle, ready_facts()).coe_ready);
    }
    assert!(
        port.trace
            .iter()
            .filter(|entry| entry.command == Command::Fpwr && entry.address >> 16 == 0x1000)
            .all(|entry| entry.payload.len() == 64)
    );
    if handoff {
        for station in 0x1001..=0x1003 {
            assert_eq!(
                port.trace
                    .iter()
                    .find(|entry| entry.command == Command::Fpwr
                        && entry.address == fixed_address(station, 0x1000))
                    .unwrap()
                    .payload[5]
                    >> 4,
                3
            );
        }
        let mut repair = ReconfigureSlaveController::<3, 4, 1, 1>::new();
        repair
            .adopt_mailbox_transport(0x1001, runtime.take_transport_state(0x1001).unwrap())
            .unwrap();
        let mut writes = PdoConfigPlan::new();
        writes
            .push(PdoSdoWrite::new(0x2007, 0, &[9, 8, 7, 6]).unwrap())
            .unwrap();
        let mut mapping = MappingTable::<4, 1>::new();
        for (index, physical_start, length, control) in [
            (0, 0x1000, 64, 0x26),
            (1, 0x1100, 64, 0x22),
            (2, 0x1400, 7, 0x24),
            (3, 0x1500, 2, 0x20),
        ] {
            mapping
                .add_sync_manager(SyncManagerConfig {
                    index,
                    physical_start,
                    length,
                    control,
                    status: 0,
                    enable: true,
                })
                .unwrap();
        }
        mapping
            .add_fmmu(FmmuConfig {
                index: 0,
                logical_start: 0x1000,
                length: 2,
                logical_start_bit: 0,
                logical_end_bit: 7,
                physical_start: 0x1500,
                physical_start_bit: 0,
                fmmu_type: 1,
                enable: true,
            })
            .unwrap();
        mapping.mark_op_only_output(2, 9).unwrap();
        let plan = ReconfigureSlavePlan::new(
            0,
            0x1001,
            startup.as_ref().unwrap().records()[0].identity,
            mapping,
        )
        .with_pdo(config, writes);
        let now = port.now_ns();
        let operation = startup
            .as_mut()
            .unwrap()
            .start_reconfigure_slave(&mut repair, plan, 90, now, now + 200 * PERIOD, now)
            .unwrap();
        for generation in 10_000..10_200 {
            port.bus.advance_by(PERIOD).unwrap();
            let now = port.now_ns();
            domains
                .submit_due_process_inputs(
                    &inputs,
                    &mut master,
                    &mut port,
                    generation,
                    now + PERIOD - 1,
                    now + PERIOD - 1,
                )
                .unwrap();
            let report = scheduler
                .run_cycle(
                    &mut domains,
                    &mut master,
                    &mut port,
                    &mut scratch,
                    &mut dc,
                    &mut dc_image,
                    now,
                    &mut controls,
                    &mut ScheduledProductionServices::<3, 4, 1, 1, 3>::new(
                        startup.as_mut(),
                        None,
                        None,
                        None,
                    )
                    .with_reconfigure_slave(&mut repair)
                    .with_runtime_sdo(&mut runtime),
                    generation,
                    now + PERIOD - 1,
                    now + PERIOD - 1,
                )
                .unwrap();
            assert_eq!(report.fault(), None);
            assert!(report.received().qualities[0].valid);
            if repair.phase() == ReconfigureSlavePhase::Complete {
                break;
            }
        }
        assert_eq!(repair.phase(), ReconfigureSlavePhase::Complete);
        assert_eq!(
            repair.result(operation).unwrap().observed_status.state,
            EthercatState::PreOp
        );
        assert!(startup.as_ref().unwrap().records()[0].configured);
        let mailbox_sm = wire(
            &mut port,
            Command::Fprd,
            fixed_address(0x1001, 0x0800),
            &[0; 16],
        );
        assert_eq!(
            mailbox_sm[6] & 1,
            1,
            "repair must retain send mailbox SM: {mailbox_sm:?}"
        );
        assert_eq!(
            mailbox_sm[14] & 1,
            1,
            "repair must retain receive mailbox SM: {mailbox_sm:?}"
        );
        runtime
            .return_transport_state(0x1001, repair.take_mailbox_transport().unwrap())
            .unwrap();
        assert!(!repair.mailbox().owns_transport());
        runtime.release(handles[0]).unwrap();
        let now = port.now_ns();
        let verify = runtime
            .submit_upload(
                0x1001,
                0x2007,
                0,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                now,
                now + 100 * PERIOD,
            )
            .unwrap();
        for generation in 11_000..11_100 {
            port.bus.advance_by(PERIOD).unwrap();
            let now = port.now_ns();
            domains
                .submit_due_process_inputs(
                    &inputs,
                    &mut master,
                    &mut port,
                    generation,
                    now + PERIOD - 1,
                    now + PERIOD - 1,
                )
                .unwrap();
            scheduler
                .run_cycle(
                    &mut domains,
                    &mut master,
                    &mut port,
                    &mut scratch,
                    &mut dc,
                    &mut dc_image,
                    now,
                    &mut controls,
                    &mut ScheduledProductionServices::<3, 0, 0>::new(
                        startup.as_mut(),
                        None,
                        None,
                        None,
                    )
                    .with_runtime_sdo(&mut runtime),
                    generation,
                    now + PERIOD - 1,
                    now + PERIOD - 1,
                )
                .unwrap();
            if runtime.status(verify).unwrap().phase == SdoRequestPhase::Complete {
                break;
            }
        }
        assert_eq!(runtime.result(verify).unwrap_or_else(|error| panic!("returned token request {error:?}; status {:?}; scheduler {:?}; mailbox phase {:?}; last trace {:?}",
            runtime.status(verify), scheduler.active(), runtime.mailbox(0x1001).map(|mailbox| mailbox.phase()),
            port.trace.iter().filter(|entry| entry.at >= now && LEASES.contains(&entry.index)).take(16).collect::<Vec<_>>())), &[9, 8, 7, 6]);
        let counters: Vec<_> = port
            .trace
            .iter()
            .filter(|entry| {
                entry.command == Command::Fpwr && entry.address == fixed_address(0x1001, 0x1000)
            })
            .map(|entry| entry.payload[5] >> 4)
            .collect();
        assert!(
            counters.windows(2).all(|pair| pair[1] == pair[0] % 7 + 1),
            "configuration/runtime/repair/runtime must share one counter authority"
        );
    }
}

#[test]
fn eight_requests_share_the_real_production_scheduler_without_aging_unsent_deadlines() {
    production(Case::Eight);
}
#[test]
fn object_abort_is_request_local_and_register_diagnostics_do_not_starve() {
    production(Case::Abort);
}
#[test]
fn delayed_inflight_sdo_keeps_ownership_without_retransmission() {
    production(Case::Delayed);
}
#[test]
fn lost_response_times_out_only_its_request_and_same_slave_fifo_continues() {
    production(Case::Dropped);
}
#[test]
fn idle_emergency_is_consumed_by_the_production_scheduler() {
    production(Case::IdleEmergency);
}
#[test]
fn generated_mapped_status_drives_runtime_sdo_without_direct_fallback() {
    production(Case::Mapped);
}
#[test]
fn mapped_idle_emergency_uses_verified_domain_status() {
    production(Case::MappedIdle);
}
#[test]
fn production_configuration_and_explicit_repair_handoff_one_physical_transport() {
    production(Case::Handoff);
}
#[test]
fn continuous_register_and_monitor_traffic_cannot_starve_runtime_sdo_or_each_other() {
    production(Case::Continuous);
}
#[test]
fn all_queued_expirations_are_reported_once_while_other_production_work_runs() {
    production(Case::QueuedExpiry);
}

#[test]
fn legacy_mapped_mailbox_participates_in_optional_wire_round_robin() {
    production(Case::LegacyMapped);
}

#[test]
fn legacy_timeout_does_not_starve_optional_work_or_hide_its_configuration_fault() {
    production(Case::LegacyTimeout);
}

fn idle_receive_after_mandatory_configuration_delay(queue_before_timeout: bool) {
    let mut port = physical_bus();
    port.bus.set_object(0, 0x2000, 0, &[1, 3, 5, 7]).unwrap();
    port.bus.emergency(0, 0x2310, 2, [1, 2, 3, 4, 5]).unwrap();
    let schedule = ScheduleTable::<1, 1>::build(
        PERIOD,
        &[ScheduleDomain {
            id: 0,
            period_ticks: 1,
            phase_ticks: 0,
        }],
    )
    .unwrap();
    let mut domain = Domain::<2, 1>::new(0x1002);
    domain
        .add_segment(DomainSegment {
            datagram_index: 0,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let mut domains = ScheduledDomainBank::new(
        &schedule,
        [ScheduledDomainEntry {
            id: 0,
            domain: &mut domain,
        }],
    )
    .unwrap();
    let mut plans = FramePlanSet::<1, 1>::new();
    plans
        .push(DatagramPlan {
            command: Command::Lrd,
            index: 0,
            address: 0x1002,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
    let image = [0; 2];
    let inputs = ScheduledProcessInputs::new(
        &domains,
        &schedule,
        [ScheduledProcessInputEntry {
            id: 0,
            image: &image,
            plans: &plans,
        }],
    )
    .unwrap();
    let mut master = EthercatMaster::<8, MAX_ETHERNET_FRAME_LEN>::new_with_bus_transit_count(
        MasterConfig::new([0xff; 6], [2, 0, 0, 0, 0, 1]),
        3,
    );
    let mut dc_config = DcCyclicConfig::new(0x1001, 240, 0);
    dc_config.expected_wkc = 3;
    let mut dc =
        DcCyclicSync::new_with_bus_transit_count(dc_config, DcMonitor::new(100, 100, 1, 2), 3);
    let mut controls =
        ControlRequestPool::<8>::new_with_datagram_indices_excluding(&LEASES, &CYCLIC).unwrap();
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut dc_image = [0; 8];
    let mut runtime = SdoRequestBank::<1, 1>::new();
    let mut config = mailbox_config(PERIOD);
    config.timeout_ns = 30 * PERIOD;
    runtime.bind_verified_slave(0, 0x1001, config).unwrap();
    let mut configuration = PdoConfigBatch::<1, 8>::new();
    let mut primed_at = None;
    let mut handle = None;
    let mut request_deadline = 0;
    let mut completed = false;
    for tick in 1..=250u16 {
        port.bus.advance_by(PERIOD).unwrap();
        let now = port.now_ns();
        domains
            .submit_due_process_inputs(
                &inputs,
                &mut master,
                &mut port,
                tick,
                now + PERIOD - 1,
                now + PERIOD - 1,
            )
            .unwrap();
        let report = scheduler
            .run_cycle(
                &mut domains,
                &mut master,
                &mut port,
                &mut scratch,
                &mut dc,
                &mut dc_image,
                now,
                &mut controls,
                &mut ScheduledProductionServices::<0, 0, 0, 8, 1>::new(None, None, None, None)
                    .with_pdo_configuration(ScheduledPdoConfiguration::batch(&mut configuration))
                    .with_runtime_sdo(&mut runtime),
                tick,
                now + PERIOD - 1,
                now + PERIOD - 1,
            )
            .unwrap();
        assert!(report.received().qualities[0].valid);
        assert!(report.received().dc_result.is_ok());
        assert_eq!(report.runtime_sdo_lost_events(), 0);
        if primed_at.is_none() && runtime.mailbox(0x1001).unwrap().phase() == MailboxPhase::Polling
        {
            // The real idle SM-full read has completed. Mandatory work on a
            // different station now delays its receive past the fixed timeout.
            primed_at = Some(now);
            let mut writes = PdoConfigPlan::new();
            for index in 0x2000..=0x2007 {
                writes
                    .push(PdoSdoWrite::new(index, 0, &[9; 4]).unwrap())
                    .unwrap();
            }
            let mut plan = PdoConfigBatchPlan::new();
            plan.push(PdoConfigJob::new(0x1003, writes, mailbox_config(PERIOD)))
                .unwrap();
            configuration
                .start(plan, 20, now, 500 * PERIOD, 100 * PERIOD)
                .unwrap();
            if queue_before_timeout {
                request_deadline = now + 250 * PERIOD;
                handle = Some(
                    runtime
                        .submit_upload(
                            0x1001,
                            0x2000,
                            0,
                            SdoAccess::Single,
                            SdoAccessPolicy::new(false),
                            now,
                            request_deadline,
                        )
                        .unwrap(),
                );
            }
        }
        if handle.is_none() && runtime.mailbox(0x1001).unwrap().phase() == MailboxPhase::Faulted {
            assert_eq!(configuration.phase(), PdoConfigBatchPhase::Complete);
            assert!(now >= primed_at.unwrap() + config.timeout_ns);
            assert_eq!(
                runtime.mailbox(0x1001).unwrap().last_error(),
                Some(MailboxError::Timeout)
            );
            request_deadline = now + 100 * PERIOD;
            handle = Some(
                runtime
                    .submit_upload(
                        0x1001,
                        0x2000,
                        0,
                        SdoAccess::Single,
                        SdoAccessPolicy::new(false),
                        now,
                        request_deadline,
                    )
                    .unwrap(),
            );
            assert!(
                runtime.is_due(now),
                "expired idle ownership must not hide a new SDO"
            );
        }
        if let Some(handle) = handle {
            assert_eq!(
                runtime.status(handle).unwrap().deadline_ns,
                request_deadline
            );
            if runtime.status(handle).unwrap().phase == SdoRequestPhase::Complete {
                assert!(now < request_deadline);
                assert_eq!(runtime.result(handle).unwrap(), &[1, 3, 5, 7]);
                assert_eq!(
                    runtime
                        .mailbox(0x1001)
                        .unwrap()
                        .pop_emergency()
                        .unwrap()
                        .error_code,
                    0x2310
                );
                assert_eq!(report.runtime_sdo_terminal().unwrap().handle, handle);
                completed = true;
                break;
            }
        }
    }
    assert!(
        completed,
        "queued same-station SDO must make bounded progress"
    );
}

#[test]
fn expired_idle_receive_releases_its_station_after_mandatory_configuration_delay() {
    idle_receive_after_mandatory_configuration_delay(false);
}

#[test]
fn queued_sdo_survives_idle_receive_expiry_during_mandatory_configuration() {
    idle_receive_after_mandatory_configuration_delay(true);
}
