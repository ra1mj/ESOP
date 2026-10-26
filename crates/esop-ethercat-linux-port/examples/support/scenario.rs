//! Caller-owned R0-R2 composition. Device simulation remains behind raw frames.
use super::evidence::{CycleTiming, RunEvidenceCollector};
use super::virtual_i2c::{I2cOutcome, VirtualI2c};
use esop_ethercat_core::wire::MAX_ETHERNET_FRAME_LEN;
use esop_ethercat_core::*;
use esop_ethercat_sim::Fault;
use esop_lifecycle_guard::ethercat::OtherCycleFacts;
use esop_lifecycle_guard::maintenance::{FaultResetController, FaultResetPhase, FaultResetRequest};
use esop_lifecycle_guard::procbuf::LifecycleEventCursor;
use esop_lifecycle_guard::stop_cycle::{
    ScheduledAuxiliaryOutputs, ScheduledProductionCycleOwner, SharedAuxiliaryOutputEntry,
    StopCycleContext,
};
use esop_lifecycle_guard::*;
use esop_procbuf::{ProcBuf, StatePage};
use esop_profile_cia402::{Cia402AxisBank, Cia402Target, CyclicLimits, CyclicSetpointGuard};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub mod generated {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-r0-r2/expected/csp/esop_product_config.rs"
    ));
}
mod generated_csv {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-r0-r2/expected/csv/esop_product_config.rs"
    ));
}
mod generated_cst {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-r0-r2/expected/cst/esop_product_config.rs"
    ));
}
pub fn product(
    mode: &str,
) -> Result<&'static esop_product_config::StaticProductConfig<'static, 3, 2, 2>, String> {
    let _ = [
        generated::PRODUCT_RUNTIME_SDO_REQUESTS,
        generated::PRODUCT_MAX_SDO_OBJECT_BYTES,
        generated_csv::PRODUCT_RUNTIME_SDO_REQUESTS,
        generated_csv::PRODUCT_MAX_SDO_OBJECT_BYTES,
        generated_cst::PRODUCT_RUNTIME_SDO_REQUESTS,
        generated_cst::PRODUCT_MAX_SDO_OBJECT_BYTES,
    ];
    match mode {
        "csp" => Ok(&generated::PRODUCT_CONFIG),
        "csv" => Ok(&generated_csv::PRODUCT_CONFIG),
        "cst" => Ok(&generated_cst::PRODUCT_CONFIG),
        _ => Err("mode must be csp, csv or cst".into()),
    }
}
const BOOT: u64 = 0xe502_2026;

mod load_q1 {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-r0-r2-load/q1/expected/esop_product_config.rs"
    ));
}
mod load_q2 {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-r0-r2-load/q2/expected/esop_product_config.rs"
    ));
}
mod load_q4 {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/examples/sim-r0-r2-load/q4/expected/esop_product_config.rs"
    ));
}

pub struct ScenarioShape {
    pub metadata: esop_product_config::ProductMetadata,
    pub topology: super::evidence::TopologyMeasurements,
    pub procbuf_bytes: u64,
    pub config_bytes: &'static [u8],
}
fn shape_for<const SLAVES: usize, const AXES: usize, const DOMAINS: usize>(
    p: &esop_product_config::StaticProductConfig<SLAVES, DOMAINS, AXES>,
    config_bytes: &'static [u8],
) -> ScenarioShape {
    ScenarioShape {
        metadata: p.metadata,
        topology: super::evidence::TopologyMeasurements {
            slave_count: SLAVES as u64,
            axis_count: AXES as u64,
            io_channels: 16,
            pdo_bytes: p
                .domains
                .iter()
                .map(|d| d.config.process_image_len as u64)
                .sum(),
            frame_count: DOMAINS as u64,
            dc_enabled: p.slaves.iter().any(|s| s.dc_required),
        },
        procbuf_bytes: core::mem::size_of::<ProcBuf<AXES, 16, DOMAINS, 64>>() as u64,
        config_bytes,
    }
}
pub fn canonical_scenario(name: &str) -> Result<&'static str, String> {
    match name {
        "full" => Ok("full"),
        "Q1" | "q1" => Ok("Q1"),
        "Q2" | "q2" => Ok("Q2"),
        "Q4" | "q4" => Ok("Q4"),
        "Q4-baseline" | "q4-baseline" => Ok("Q4-baseline"),
        "command_expiry" => Ok("command_expiry"),
        "dc_fault" => Ok("dc_fault"),
        "wkc_fault" => Ok("wkc_fault"),
        "link_fault" => Ok("link_fault"),
        "drive_fault" => Ok("drive_fault"),
        "supervisor_restart" => Ok("supervisor_restart"),
        _ => Err(format!(
            "unsupported scenario {name}; use --help for supported scenarios"
        )),
    }
}

pub fn shape(scenario: &str, mode: &str) -> Result<ScenarioShape, String> {
    let scenario = canonical_scenario(scenario)?;
    let _ = [
        load_q1::PRODUCT_RUNTIME_SDO_REQUESTS,
        load_q1::PRODUCT_MAX_SDO_OBJECT_BYTES,
        load_q2::PRODUCT_RUNTIME_SDO_REQUESTS,
        load_q2::PRODUCT_MAX_SDO_OBJECT_BYTES,
        load_q4::PRODUCT_RUNTIME_SDO_REQUESTS,
        load_q4::PRODUCT_MAX_SDO_OBJECT_BYTES,
    ];
    if matches!(
        scenario,
        "Q1" | "q1" | "Q2" | "q2" | "Q4" | "q4" | "Q4-baseline" | "q4-baseline"
    ) && mode != "csp"
    {
        return Err("Q workload fixtures use their frozen CSP product".into());
    }
    Ok(match scenario {
        "Q1" | "q1" => shape_for(
            &load_q1::PRODUCT_CONFIG,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../config/examples/sim-r0-r2-load/q1/expected/product_config.json"
            )),
        ),
        "Q2" | "q2" => shape_for(
            &load_q2::PRODUCT_CONFIG,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../config/examples/sim-r0-r2-load/q2/expected/product_config.json"
            )),
        ),
        "Q4" | "q4" | "Q4-baseline" | "q4-baseline" => shape_for(
            &load_q4::PRODUCT_CONFIG,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../config/examples/sim-r0-r2-load/q4/expected/product_config.json"
            )),
        ),
        _ => shape_for(
            product(mode)?,
            match mode {
                "csp" => include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../config/examples/sim-r0-r2/expected/csp/product_config.json"
                )),
                "csv" => include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../config/examples/sim-r0-r2/expected/csv/product_config.json"
                )),
                _ => include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../config/examples/sim-r0-r2/expected/cst/product_config.json"
                )),
            },
        ),
    })
}

pub trait ScenarioPort: EthercatPort {
    fn advance_to(&mut self, now: u64) -> Result<(), String>;
    fn fault(&mut self, fault: Fault) -> Result<(), String>;
    fn motion_allowed(&self) -> bool {
        true
    }
    fn emergency(&mut self) -> Result<(), String>;
    fn begin_cycle(&mut self) {}
    fn observed_round_trip_ns(&self) -> Option<u64> {
        None
    }
}

#[derive(Debug, Serialize)]
pub struct ScenarioResult {
    pub schema_version: &'static str,
    pub scenario: String,
    pub slave_count: usize,
    pub axis_count: usize,
    pub domain_count: usize,
    pub cycles: u64,
    pub period_ns: u64,
    pub startup_wire_actions: u64,
    pub startup_ready: bool,
    pub confirmed_stops: u64,
    pub explicit_rearm: bool,
    pub injected_fault_observed: bool,
    pub implicit_rearm: bool,
    pub fault_reset_complete: bool,
    pub domain_complete_cycles: u64,
    pub dc_good_cycles: u64,
    pub active_cycles: u64,
    pub stopping_cycles: u64,
    pub sdo_completed: u64,
    pub sdo_outstanding: u64,
    pub sdo_verified: u64,
    pub emergencies_consumed: u64,
    pub transitions: u64,
    pub final_state: u8,
    pub state_sha256: String,
    pub event_sha256: String,
    pub software_scenario_passed: bool,
    pub hardware_qualified: bool,
}

fn check<T, E: core::fmt::Debug>(result: Result<T, E>) -> Result<T, String> {
    result.map_err(|e| format!("{e:?}"))
}
fn fill_load<const SLAVES: usize>(
    bank: &mut SdoRequestBank<SLAVES, 8, 1024>,
    stations: &[u16; 8],
    handles: &mut [Option<SdoRequestHandle>; 8],
    upload: bool,
    now: u64,
    deadline: u64,
) -> Result<(), String> {
    for (i, handle) in handles.iter_mut().enumerate() {
        *handle = Some(if upload {
            check(bank.submit_upload(
                stations[i],
                0x2000 + i as u16,
                0,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                now,
                deadline,
            ))?
        } else {
            let bytes = core::array::from_fn::<_, 1024, _>(|n| (n as u8).wrapping_add(i as u8));
            check(bank.submit_download(
                stations[i],
                0x2000 + i as u16,
                0,
                &bytes,
                SdoAccess::Single,
                SdoAccessPolicy::new(false),
                now,
                deadline,
            ))?
        });
    }
    Ok(())
}

fn mapping<const SLAVES: usize, const DOMAINS: usize, const AXES: usize>(
    p: &esop_product_config::StaticProductConfig<SLAVES, DOMAINS, AXES>,
    position: usize,
) -> Result<MappingTable<4, 3>, String> {
    let slave = p.slaves[position];
    let mut table = MappingTable::new();
    let domain = p
        .domains
        .iter()
        .find(|d| d.config.id == slave.domain_id)
        .ok_or("missing slave Domain")?;
    let mut spans = [(usize::MAX, 0usize); 2];
    for pdo in p
        .pdos
        .iter()
        .filter(|pdo| pdo.request.slave_position == slave.position)
    {
        let direction = usize::from(pdo.request.direction == PdoDirection::Tx);
        spans[direction].0 = spans[direction].0.min(pdo.bit_offset);
        spans[direction].1 = spans[direction]
            .1
            .max(pdo.bit_offset + pdo.request.bit_length as usize);
    }
    if spans
        .iter()
        .any(|(start, end)| *start == usize::MAX || start % 8 != 0 || end <= start)
    {
        return Err("invalid PDO span".into());
    }
    let lengths = spans.map(|(start, end)| (end - start).div_ceil(8) as u16);
    for (index, start, length, control) in [
        (
            0,
            slave.mailbox_config.send_address,
            slave.mailbox_config.send_capacity,
            0x26,
        ),
        (
            1,
            slave.mailbox_config.receive_address,
            slave.mailbox_config.receive_capacity,
            0x22,
        ),
        (2, 0x1400, lengths[0], 0x24),
        (3, 0x1500, lengths[1], 0x20),
    ] {
        check(table.add_sync_manager(SyncManagerConfig {
            index,
            physical_start: start,
            length,
            control,
            status: 0,
            enable: true,
        }))?;
    }
    for direction in 0..2 {
        check(table.add_fmmu(FmmuConfig {
            index: direction as u8,
            logical_start: domain.config.logical_address + (spans[direction].0 / 8) as u32,
            length: lengths[direction],
            logical_start_bit: 0,
            logical_end_bit: 7,
            physical_start: if direction == 0 { 0x1400 } else { 0x1500 },
            physical_start_bit: 0,
            fmmu_type: if direction == 0 { 2 } else { 1 },
            enable: true,
        }))?;
    }
    if let Some(status) = slave.mapped_mailbox_status {
        check(table.add_fmmu(status.fmmu))?;
    }
    for index in 0..4 {
        if let Some(activation) = slave.op_only_outputs.activation_template(index) {
            check(table.mark_op_only_output(index, activation))?;
        }
    }
    Ok(table)
}

fn startup<P: ScenarioPort, const SLAVES: usize, const DOMAINS: usize, const AXES: usize>(
    port: &mut P,
    product: &esop_product_config::StaticProductConfig<SLAVES, DOMAINS, AXES>,
) -> Result<
    (
        StartupController<SLAVES>,
        u64,
        SdoRequestBank<SLAVES, 8, 1024>,
    ),
    String,
>
where
    P::Error: core::fmt::Debug,
{
    let mut startup = StartupController::<SLAVES>::new(0x1001);
    let services = StartupConfigurationServices::new()
        .with_pdo_configuration()
        .with_mapping()
        .with_watchdog_configuration()
        .with_dc_clock_configuration()
        .with_dc_sync_configuration();
    check(product.start_startup(
        &mut startup,
        1,
        port.now_ns(),
        StartupConfig::new(EthercatState::Op).with_configuration_services(services),
    ))?;
    let mut master = EthercatMaster::<8, MAX_ETHERNET_FRAME_LEN>::new_with_bus_transit_count(
        MasterConfig::new([0xff; 6], [2, 0, 0, 0, 0, 1]),
        SLAVES as u16,
    );
    let mut dc = cyclic_dc(
        SLAVES as u16,
        product.slaves.iter().filter(|s| s.dc_required).count() as u16,
    );
    let mut dc_image = [0; 8];
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut controls = check(
        ControlRequestPool::<8>::new_with_datagram_indices_excluding(
            &[220, 221, 222, 223, 224, 225, 226, 227],
            &[0, 1, 2, 3, 250, 253],
        ),
    )?;
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut actions = 0;
    for _ in 1..20000u16 {
        if startup.phase() == StartupPhase::AwaitingConfiguration {
            break;
        }
        port.advance_to(port.now_ns() + 10_000)?;
        let now = port.now_ns();
        let Some(action) = check(startup.next_action(now))? else {
            continue;
        };
        let request = check(startup.enqueue_pending(&mut controls))?;
        let frame = check(master.acquire_frame(action.generation(), action.deadline_ns()))?;
        check(master.build_control_request(&mut controls, request, frame))?;
        check(master.submit_frame(port, frame))?;
        for _ in 0..128 {
            let report = check(master.cycle_receive_with_consumer(
                port,
                &mut scratch,
                action.generation(),
                &mut ControlRxConsumer::new(&mut controls),
            ))?;
            if report.corrupt_frames != 0
                || report.wkc_mismatches != 0
                || report.consumer_rejections != 0
            {
                return Err(format!("startup RX validation {report:?}"));
            }
            if matches!(
                controls.get(request).map(|r| r.state),
                Some(RequestState::Complete | RequestState::Failed)
            ) {
                break;
            }
            port.advance_to(port.now_ns() + 1000)?;
            controls.expire_in_flight(port.now_ns());
        }
        check(startup.accept_completed(&mut controls, request, port.now_ns()))?;
        actions += 1;
    }
    if startup.phase() != StartupPhase::AwaitingConfiguration {
        return Err("startup did not reach configuration barrier".into());
    }
    if controls.in_use() != 0 {
        return Err("startup left control requests in flight".into());
    }
    master = EthercatMaster::<8, MAX_ETHERNET_FRAME_LEN>::new_with_bus_transit_count(
        MasterConfig::new([0xff; 6], [2, 0, 0, 0, 0, 1]),
        SLAVES as u16,
    );
    // Stage the frozen product plan before activation. SAFEOP/transition input
    // remains unqualified, while inhibited output frames feed watchdogs on
    // slaves that reach OP before the rest of the topology.
    let configured_schedule = product.domains.map(|d| ScheduleDomain {
        id: d.config.id,
        period_ticks: d.config.period_ticks,
        phase_ticks: d.config.phase_ticks,
    });
    let schedule = check(ScheduleTable::<DOMAINS, 4>::build(
        product.metadata.base_period_ns,
        &configured_schedule,
    ))?;
    let mut storage: [Domain<2048, 2>; DOMAINS] =
        core::array::from_fn(|i| Domain::new(product.domains[i].config.logical_address));
    let mut plans: [FramePlanSet<2, 16>; DOMAINS] = core::array::from_fn(|_| FramePlanSet::new());
    for entry in product.datagrams {
        let slot = product
            .domains
            .iter()
            .position(|d| d.config.id == entry.domain_id)
            .ok_or("missing configuration Domain")?;
        let spec = entry.spec;
        check(storage[slot].add_segment(DomainSegment {
            datagram_index: spec.index,
            input_offset: spec.payload_offset,
            len: spec.payload_len,
            expected_wkc: spec.expected_wkc,
        }))?;
        check(plans[slot].push(DatagramPlan {
            command: spec.command,
            index: spec.index,
            address: spec.address,
            payload_offset: product.domains[slot].config.process_image_offset + spec.payload_offset,
            payload_len: spec.payload_len,
            expected_wkc: spec.expected_wkc,
        }))?;
    }
    let mut index = 0;
    let entries = storage.each_mut().map(|domain| {
        let d = product.domains[index].config;
        index += 1;
        ScheduledProcessImageDomainEntry {
            id: d.id,
            process_image_offset: d.process_image_offset,
            domain,
        }
    });
    let mut domains = check(ScheduledDomainBank::new_with_process_image_offsets(
        &schedule, entries,
    ))?;
    let inhibited = [0; 2048];
    let mut pdo = PdoConfigBatch::<SLAVES, 40>::new();
    check(pdo.start(
        check(product.build_generated_pdo_configuration_batch())?,
        10,
        port.now_ns(),
        10_000_000_000,
        1_000_000,
    ))?;
    let mut watchdog = WatchdogController::<SLAVES>::new();
    check(watchdog.start(
        WatchdogControllerConfig::new(),
        &check(product.watchdog_plan())?,
        20,
        port.now_ns(),
    ))?;
    let topology = *startup.dc_topology().ok_or("no discovered DC topology")?;
    let mut clocks = DcClockController::<SLAVES>::new();
    check(clocks.start(
        DcClockConfig::new(),
        &topology,
        30,
        port.now_ns(),
        port.now_ns(),
    ))?;
    let mut sync = DcSyncController::<SLAVES>::new();
    check(sync.start(
        DcSyncConfig::new(),
        &check(product.dc_sync_plan())?,
        &topology,
        40,
        port.now_ns(),
    ))?;
    let mut mapping_controller = MappingConfigController::<4, 3>::new();
    let mut mapping_position = 0;
    check(startup.start_mapping_for_position(
        0,
        &mut mapping_controller,
        50,
        port.now_ns(),
        10_000_000_000,
        1_000_000,
        &mapping(product, 0)?,
    ))?;
    for generation in 1..20000u16 {
        if startup.phase() == StartupPhase::Ready {
            break;
        }
        if mapping_controller.phase() == MappingConfigPhase::Complete
            && mapping_position + 1 < SLAVES
        {
            mapping_position += 1;
            check(startup.start_mapping_for_position(
                mapping_position as u16,
                &mut mapping_controller,
                50 + mapping_position as u16,
                port.now_ns(),
                10_000_000_000,
                1_000_000,
                &mapping(product, mapping_position)?,
            ))?;
        }
        port.advance_to(port.now_ns() + 100_000)?;
        let now = port.now_ns();
        if mapping_position + 1 == SLAVES
            && mapping_controller.phase() == MappingConfigPhase::Complete
        {
            let inputs = check(ScheduledProcessInputs::new(
                &domains,
                &schedule,
                core::array::from_fn(|i| ScheduledProcessInputEntry {
                    id: product.domains[i].config.id,
                    image: &inhibited,
                    plans: &plans[i],
                }),
            ))?;
            let process = check(domains.submit_due_process_inputs(
                &inputs,
                &mut master,
                port,
                generation,
                now + 900_000,
                now + 900_000,
            ))?;
            if process.failure.is_some() {
                return Err(format!(
                    "configuration inhibited PDO TX {:?}",
                    process.failure
                ));
            }
        }
        let report = check(
            scheduler.run_cycle(
                &mut domains,
                &mut master,
                port,
                &mut scratch,
                &mut dc,
                &mut dc_image,
                now,
                &mut controls,
                &mut ScheduledProductionServices::<SLAVES, 4, 3, 40, SLAVES>::new(
                    Some(&mut startup),
                    Some(&mut mapping_controller),
                    None,
                    None,
                )
                .with_pdo_configuration(ScheduledPdoConfiguration::batch(&mut pdo))
                .with_watchdog_configuration(&mut watchdog)
                .with_dc_clock_configuration(&mut clocks)
                .with_dc_sync_configuration(&mut sync),
                generation,
                now + 900_000,
                now + 900_000,
            ),
        )?;
        if let Some(fault) = report.fault() {
            return Err(format!("configuration fault {fault:?}"));
        }
        actions += 1;
    }
    if startup.phase() != StartupPhase::Ready {
        return Err(format!(
            "configuration stalled {:?} {:?}",
            startup.phase(),
            mapping_controller.phase()
        ));
    }
    let mut runtime = SdoRequestBank::<SLAVES, 8, 1024>::new();
    for (slot, slave) in product.slaves.iter().enumerate() {
        check(runtime.bind_verified_slave_with_transport(
            slot,
            slave.station_address,
            slave.mailbox_config,
            check(pdo.take_transport_state(slave.station_address))?,
        ))?;
    }
    Ok((startup, actions, runtime))
}

fn cyclic_dc(transit: u16, dc_count: u16) -> DcCyclicSync {
    let mut config = DcCyclicConfig::new(0x1001, 250, 0);
    config.expected_wkc = dc_count;
    DcCyclicSync::new_with_bus_transit_count(config, DcMonitor::new(5_000, 1_000, 3, 2), transit)
}

// Canonical little-endian State encoding excludes Rust padding and preserves
// every public observation, including chronological lifecycle history.
fn state_record<const AXES: usize, const DOMAINS: usize>(
    state: &StatePage<AXES, 16, DOMAINS>,
) -> ([u8; 4096], usize) {
    let mut bytes = [0; 4096];
    let mut cursor = 0;
    macro_rules! add {
        ($value:expr) => {{
            let v = $value.to_le_bytes();
            bytes[cursor..cursor + v.len()].copy_from_slice(&v);
            cursor += v.len();
        }};
    }
    add!(state.boot_id);
    add!(state.sequence);
    add!(state.ecat_time_ns);
    add!(state.monotonic_time_ns);
    for a in state.axes {
        add!(a.position.to_bits());
        add!(a.velocity.to_bits());
        add!(a.torque.to_bits());
        add!(a.following_error.to_bits());
        add!(a.statusword);
        add!(a.controlword);
        add!(a.error_code);
        add!(a.drive_state);
        add!(a.actual_mode);
        add!(a.quality);
    }
    for io in state.io {
        add!(io.input_bits);
        add!(io.quality);
    }
    let q = state.quality;
    add!(q.sequence);
    add!(q.link_up);
    add!(q.al_state);
    add!(q.dc_locked);
    add!(q.fault_bitmap);
    add!(q.command_age_cycles);
    add!(q.deadline_misses);
    add!(q.dc_offset_ns);
    add!(q.cyclic.known_mask);
    add!(q.cyclic.good_mask);
    for d in q.domains {
        add!(d.expected_wkc);
        add!(d.actual_wkc);
        add!(d.valid);
        add!(d.complete);
        add!(d.consecutive_wkc_mismatches);
        add!(d.last_valid_cycle);
        add!(d.input_age_cycles);
    }
    let l = state.lifecycle;
    add!(l.state);
    add!(l.stop_action);
    add!(l.gates_ready);
    add!(l.motion_permit);
    add!(l.required_gate_mask);
    add!(l.valid_gate_mask);
    add!(l.qualified_gate_mask);
    add!(l.ready_gate_mask);
    add!(l.first_blocking_code);
    add!(l.latched_fault_code);
    add!(l.permit_epoch);
    add!(l.permit_expires_at_ns);
    add!(l.transition_sequence);
    add!(l.transition_cycle);
    add!(l.transition_time_ns);
    add!(l.recovery_count);
    add!(l.permit_audit_sequence);
    for stop in state.axis_stops {
        add!(stop.request_cycle);
        add!(stop.feedback_cycle);
        add!(stop.requested_action);
        add!(stop.issued_action);
        add!(stop.feedback_valid);
        add!(stop.stationary);
        add!(stop.non_enabled);
    }
    add!(state.lifecycle_history.len() as u64);
    for index in 0..state.lifecycle_history.len() {
        if let Some(h) = state.lifecycle_history.get(index) {
            add!(h.sequence);
            add!(h.timestamp_ns);
            add!(h.from_state);
            add!(h.to_state);
            add!(h.fault_code);
        }
    }
    let r = state.runtime_observation;
    add!(r.latest_incident_id);
    add!(r.agent_epoch);
    add!(r.observed_at_ns);
    add!(r.observation_window_ns);
    add!(r.lost_events);
    add!(r.incident_count);
    add!(r.health);
    (bytes, cursor)
}
fn event_record(event: esop_procbuf::ProcBufEvent) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&event.sequence.to_le_bytes());
    bytes[8..16].copy_from_slice(&event.timestamp_ns.to_le_bytes());
    bytes[16..18].copy_from_slice(&event.source.to_le_bytes());
    bytes[18] = event.severity as u8;
    bytes[19..21].copy_from_slice(&event.code.to_le_bytes());
    bytes[21..23].copy_from_slice(&event.axis_or_device.to_le_bytes());
    bytes[23..27].copy_from_slice(&event.value.to_le_bytes());
    bytes[27..31].copy_from_slice(&event.aux.to_le_bytes());
    bytes
}

pub fn run<P: ScenarioPort>(
    port: &mut P,
    scenario: &str,
    cycles: u64,
    mode: &str,
    evidence: &mut RunEvidenceCollector,
) -> Result<ScenarioResult, String>
where
    P::Error: core::fmt::Debug,
{
    let scenario = canonical_scenario(scenario)?;
    match scenario {
        "Q1" | "q1" => run_product::<P, 32, 32, 1, 1024>(
            port,
            &load_q1::PRODUCT_CONFIG,
            scenario,
            cycles,
            evidence,
        ),
        "Q2" | "q2" => run_product::<P, 17, 16, 2, 2048>(
            port,
            &load_q2::PRODUCT_CONFIG,
            scenario,
            cycles,
            evidence,
        ),
        "Q4" | "q4" | "Q4-baseline" | "q4-baseline" => run_product::<P, 17, 16, 2, 2048>(
            port,
            &load_q4::PRODUCT_CONFIG,
            scenario,
            cycles,
            evidence,
        ),
        _ => run_product::<P, 3, 2, 2, 128>(port, product(mode)?, scenario, cycles, evidence),
    }
}
pub fn run_product<
    P: ScenarioPort,
    const SLAVES: usize,
    const AXES: usize,
    const DOMAINS: usize,
    const IMAGE: usize,
>(
    port: &mut P,
    product: &esop_product_config::StaticProductConfig<SLAVES, DOMAINS, AXES>,
    scenario: &str,
    cycles: u64,
    evidence: &mut RunEvidenceCollector,
) -> Result<ScenarioResult, String>
where
    P::Error: core::fmt::Debug,
{
    if SLAVES == 0 || AXES == 0 || AXES > 32 {
        return Err("invalid scenario shape".into());
    }
    let period = product.metadata.base_period_ns;
    let budget = product.metadata.deadline_ns;
    let axis_mask = if AXES == 32 {
        u32::MAX
    } else {
        (1u32 << AXES) - 1
    };
    let (startup, startup_wire_actions, mut runtime) =
        startup(port, product).map_err(|e| format!("startup: {e}"))?;
    let buffer = ProcBuf::<AXES, 16, DOMAINS, 64>::new(product.metadata.robot_id, BOOT);
    let active = check(product.activate::<16, 64, 512, 2, 4, 2, 16>(
        product.metadata.config_sha256,
        startup.records(),
        &buffer,
        BOOT,
    ))?;
    for slave in product.slaves {
        if let Some(mapping) = slave.mapped_mailbox_status {
            check(runtime.bind_verified_mapped_status(slave.station_address, mapping))?;
        }
    }
    let registry = active.registry();
    let schedule = active.schedule();
    let mut domain_storage: [Domain<IMAGE, 2>; DOMAINS] =
        core::array::from_fn(|i| Domain::new(product.domains[i].config.logical_address));
    for entry in product.datagrams {
        let spec = entry.spec;
        let slot = product
            .domains
            .iter()
            .position(|d| d.config.id == entry.domain_id)
            .ok_or("missing Domain")?;
        check(domain_storage[slot].add_segment(DomainSegment {
            datagram_index: spec.index,
            input_offset: spec.payload_offset,
            len: spec.payload_len,
            expected_wkc: spec.expected_wkc,
        }))?;
    }
    let mut index = 0;
    let entries = domain_storage.each_mut().map(|domain| {
        let config = product.domains[index].config;
        index += 1;
        ScheduledProcessImageDomainEntry {
            id: config.id,
            process_image_offset: config.process_image_offset,
            domain,
        }
    });
    let mut domains = check(ScheduledDomainBank::new_with_process_image_offsets(
        schedule, entries,
    ))?;
    let plans = active.frame_plans();
    let mut motion_plan = FramePlan::<16>::new();
    check(registry.append_frame_plan(0, &mut motion_plan))?;
    let safe = [0; IMAGE];
    let mut copy_image = SlaveCopyProcessImage::new(safe);
    let outputs = check(ScheduledAuxiliaryOutputs::new_with_shared_process_image(
        &domains,
        schedule,
        0,
        &motion_plan,
        &copy_image,
        core::array::from_fn(|i| {
            if product.domains[i].config.id == 0 {
                None
            } else {
                Some(SharedAuxiliaryOutputEntry {
                    id: product.domains[i].config.id,
                    plans: &plans[i],
                })
            }
        }),
    ))?;
    let mut owner = check(ScheduledProductionCycleOwner::with_slave_copies(
        &outputs,
        active.slave_copy_plans(),
        &copy_image,
    ))?;
    let mut master = EthercatMaster::<8, MAX_ETHERNET_FRAME_LEN>::new_with_bus_transit_count(
        MasterConfig::new([0xff; 6], [2, 0, 0, 0, 0, 1]),
        SLAVES as u16,
    );
    let mut dc = cyclic_dc(
        SLAVES as u16,
        product.slaves.iter().filter(|s| s.dc_required).count() as u16,
    );
    let mut dc_image = [0; 8];
    let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
    let mut controls = check(
        ControlRequestPool::<8>::new_with_datagram_indices_excluding(
            &[220, 221, 222, 223, 224, 225, 226, 227],
            &[0, 1, 2, 3, 250, 253],
        ),
    )?;
    let mut scheduler = ScheduledProductionServiceScheduler::new();
    let mut guard = LifecycleGuard::new(
        GateId::Domain.bit()
            | GateId::Link.bit()
            | GateId::Configuration.bit()
            | GateId::Topology.bit()
            | GateId::Drive.bit()
            | GateId::Platform.bit()
            | GateId::ExternalSafety.bit()
            | GateId::Budget.bit()
            | GateId::DistributedClock.bit()
            | GateId::Command.bit()
            | GateId::Supervisor.bit(),
        BOOT,
        GuardPolicy {
            enter_good_cycles: 3,
            allowed_axis_mask: axis_mask,
            max_age_cycles: 4,
            ..GuardPolicy::conservative()
        },
    );
    let mut cursor = LifecycleEventCursor::new(&guard);
    let mut axes = Cia402AxisBank::<AXES>::new();
    let mut state = StatePage::<AXES, 16, DOMAINS>::new(BOOT);
    let mut setpoint_guards = [CyclicSetpointGuard::new(); AXES];
    let limits = [CyclicLimits {
        max_position_step: 1000.0,
        max_velocity: 1000.0,
        max_torque: 1000.0,
    }; AXES];
    let priming = check(ScheduledProcessInputs::new(
        &domains,
        schedule,
        core::array::from_fn(|i| ScheduledProcessInputEntry {
            id: product.domains[i].config.id,
            image: &safe,
            plans: &plans[i],
        }),
    ))?;
    let origin = port.now_ns() + period;
    port.advance_to(origin)?;
    let first = check(domains.submit_due_process_inputs(
        &priming,
        &mut master,
        port,
        1,
        origin + 2 * period,
        origin + budget,
    ))?;
    owner
        .arm_priming(&domains, &priming, &first, 1, origin + 2 * period)
        .map_err(|e| format!("priming {e:?}"))?;
    let mut result = ScenarioResult {
        schema_version: "esop.software-scenario.v1",
        scenario: scenario.into(),
        slave_count: SLAVES,
        axis_count: AXES,
        domain_count: DOMAINS,
        cycles,
        period_ns: period,
        startup_wire_actions,
        startup_ready: true,
        confirmed_stops: 0,
        explicit_rearm: false,
        injected_fault_observed: false,
        implicit_rearm: false,
        fault_reset_complete: false,
        domain_complete_cycles: 0,
        dc_good_cycles: 0,
        active_cycles: 0,
        stopping_cycles: 0,
        sdo_completed: 0,
        sdo_outstanding: 0,
        sdo_verified: 0,
        emergencies_consumed: 0,
        transitions: 0,
        final_state: 0,
        state_sha256: String::new(),
        event_sha256: String::new(),
        software_scenario_passed: false,
        hardware_qualified: false,
    };
    let load = matches!(scenario, "full" | "Q4" | "q4");
    let q4_baseline = matches!(scenario, "Q4-baseline" | "q4-baseline");
    let mut stations = [0; 8];
    for (i, station) in stations.iter_mut().enumerate() {
        *station = product
            .slaves
            .iter()
            .find(|s| s.position == product.axes[i % AXES].slave_position)
            .ok_or("axis station missing")?
            .station_address;
    }
    let mut handles = [None; 8];
    let mut load_phase = 0;
    let mut load_started = false;
    let mut accounted = [false; 8];

    let configs = core::array::from_fn::<_, SLAVES, _>(|i| SlaveMonitorConfig {
        position: startup.records()[i].position,
        station_address: startup.records()[i].station_address,
        required_al_state: EthercatState::Op,
        required_dl_mask: startup.scan_records()[i].dl_status & 0xf0,
    });
    let mut monitor = check(SlaveMonitorController::<SLAVES>::new(
        230,
        &configs,
        4 * period,
        ((SLAVES as u64 * 8 + 20).max(50)) * period,
        period,
        origin,
    ))?;
    let mut i2c = check(VirtualI2c::new(0x200, 0x40, 10 * period))?;
    check(i2c.initialize(0))?;
    let mut i2c_fault = 0;
    let mut i2c_fault_cycle = 0;
    let mut nack_count = 0;
    let mut timeout_count = 0;
    let mut emergency_injected = false;
    let mut confirmed_stops = 0;
    let expiry_case = matches!(
        scenario,
        "full"
            | "Q1"
            | "q1"
            | "Q2"
            | "q2"
            | "Q4"
            | "q4"
            | "Q4-baseline"
            | "q4-baseline"
            | "command_expiry"
    );
    let mut reset = check(FaultResetController::new(BOOT, 3))?;
    let mut armed = false;
    let mut rearmed = false;
    let mut state_digest = Sha256::new();
    let mut event_digest = Sha256::new();
    for tick in 0..cycles {
        if !emergency_injected
            && port.motion_allowed()
            && ((!load && !q4_baseline && tick >= cycles / 2)
                || ((load || q4_baseline) && !load_started))
        {
            port.emergency()?;
            emergency_injected = true;
        }
        if tick == cycles / 3 && port.motion_allowed() {
            match scenario {
                "dc_fault" => port.fault(Fault::DcOffset {
                    slave: 0,
                    nanoseconds: 100_000,
                })?,
                "link_fault" => port.fault(Fault::Link(false))?,
                "drive_fault" => port.fault(Fault::Drive {
                    slave: 0,
                    code: 0x2310,
                })?,
                _ => {}
            }
        }
        if tick == cycles / 3 + 3 && port.motion_allowed() {
            match scenario {
                "dc_fault" => port.fault(Fault::DcOffset {
                    slave: 0,
                    nanoseconds: 0,
                })?,
                "link_fault" => port.fault(Fault::Link(true))?,
                "drive_fault" => port.fault(Fault::Drive { slave: 0, code: 0 })?,
                _ => {}
            }
        }
        let now = origin + (tick + 1) * period;
        port.advance_to(now)?;
        let cycle_start = std::time::Instant::now();
        let jitter = port.now_ns().abs_diff(now);
        let _activated = crate::activation::enter();
        port.begin_cycle();
        if load && !load_started && tick >= 20 && result.emergencies_consumed != 0 {
            fill_load(
                &mut runtime,
                &stations,
                &mut handles,
                false,
                now,
                origin + (cycles + 10) * period,
            )?;
            load_started = true;
            evidence.record_sdo(8, 0, 0, 8);
        }
        let generation = (tick as u16).wrapping_add(1);
        let report = check(
            scheduler.run_cycle(
                &mut domains,
                &mut master,
                port,
                &mut scratch,
                &mut dc,
                &mut dc_image,
                now,
                &mut controls,
                &mut ScheduledProductionServices::<SLAVES, 4, 3, 40, SLAVES>::new(
                    None, None, None, None,
                )
                .with_runtime_sdo(&mut runtime)
                .with_slave_monitor(&mut monitor),
                generation,
                now + budget,
                now + budget,
            ),
        )?;
        owner
            .complete_service_cycle_with_slave_copies(&domains, &report, &mut copy_image)
            .map_err(|e| format!("receive tick{tick} {e:?}"))?;
        if report.received().qualities.iter().all(|q| q.valid) {
            result.domain_complete_cycles += 1;
        }
        if report.received().dc_result.is_ok() {
            result.dc_good_cycles += 1;
        }
        let rx_elapsed = port.observed_round_trip_ns();
        let cycle = report.received().report.cycle;
        state.sequence = cycle;
        let other = OtherCycleFacts {
            platform_ready: true,
            coe_ready: report.service_ready(),
            topology_valid: startup.phase() == StartupPhase::Ready,
            drive_ready: domains.domain::<IMAGE, 2>(0).is_some_and(|domain| {
                domain.quality().valid
                    && domain.quality().complete
                    && active
                        .axis_pdo_maps()
                        .iter()
                        .enumerate()
                        .all(|(axis, map)| {
                            map.read_inputs_for(domain.input(), active.axis_modes()[axis])
                                .is_ok_and(|input| {
                                    !matches!(
                                        esop_profile_cia402::DriveState::from_statusword(
                                            input.statusword
                                        ),
                                        esop_profile_cia402::DriveState::Fault
                                            | esop_profile_cia402::DriveState::FaultReactionActive
                                            | esop_profile_cia402::DriveState::Unknown
                                    ) && input.error_code == 0
                                        && input.actual_mode == active.axis_modes()[axis]
                                })
                        })
            }),
            command_current: !(expiry_case && tick >= cycles / 3 && tick < cycles / 3 + 3)
                && tick < cycles.saturating_sub(5),
            supervisor_healthy: !(scenario == "supervisor_restart"
                && tick >= cycles / 3
                && tick < cycles / 3 + 3),
            external_safety_clear: true,
            deadline_met: true,
        };
        if !armed
            && tick >= 6
            && port.motion_allowed()
            && guard.ready_gate_mask(cycle) & guard.required_mask() == guard.required_mask()
        {
            check(guard.request_rearm(
                MotionPermit {
                    boot_id: BOOT,
                    source_id: 1,
                    permit_epoch: 1,
                    sequence: 1,
                    expires_at_ns: origin
                        + (if expiry_case {
                            cycles / 3 + 1
                        } else {
                            cycles + 10
                        }) * period,
                    axis_mask,
                    authority: 1,
                    reserved: [0; 3],
                    policy_version: 1,
                },
                cycle,
                now,
            ))?;
            armed = true;
        }
        let motion_input = domains
            .domain::<IMAGE, 2>(0)
            .ok_or("motion binding")?
            .input();
        if tick >= cycles * 2 / 3
            && guard.state() == LifecycleState::FaultLatched
            && guard.ready_gate_mask(cycle) & guard.required_mask() == guard.required_mask()
        {
            check(guard.clear_fault(cycle))?;
        }
        if armed
            && !rearmed
            && tick >= cycles * 2 / 3
            && tick < cycles.saturating_sub(10)
            && guard.ready_gate_mask(cycle) & guard.required_mask() == guard.required_mask()
            && matches!(
                guard.state(),
                LifecycleState::Ready | LifecycleState::Qualifying
            )
        {
            check(guard.request_rearm(
                MotionPermit {
                    boot_id: BOOT,
                    source_id: 1,
                    permit_epoch: 100,
                    sequence: 1,
                    expires_at_ns: origin + (cycles + 10) * period,
                    axis_mask,
                    authority: 1,
                    reserved: [0; 3],
                    policy_version: 1,
                },
                cycle,
                now,
            ))?;
            rearmed = true;
        }
        let targets = core::array::from_fn(|axis| {
            let input = active.axis_pdo_maps()[axis]
                .read_inputs_for(motion_input, active.axis_modes()[axis])
                .ok()?;
            match (input.statusword & 0x6f, active.axis_modes()[axis]) {
                (0x23, esop_profile_cia402::OperatingMode::Csp) => {
                    Some(Cia402Target::Position(input.actual_position?))
                }
                (0x27, esop_profile_cia402::OperatingMode::Csp) => {
                    Some(Cia402Target::Position((tick % 100) as i32))
                }
                (0x23, esop_profile_cia402::OperatingMode::Csv) => {
                    Some(Cia402Target::Velocity(input.actual_velocity?))
                }
                (0x27, esop_profile_cia402::OperatingMode::Csv) => {
                    Some(Cia402Target::Velocity(1000))
                }
                (0x23, esop_profile_cia402::OperatingMode::Cst) => {
                    Some(Cia402Target::Torque(input.actual_torque?))
                }
                (0x27, esop_profile_cia402::OperatingMode::Cst) => Some(Cia402Target::Torque(10)),
                _ => None,
            }
        });
        if scenario == "wkc_fault" && tick == cycles / 3 {
            port.fault(Fault::WorkingCounterNextResponse {
                datagram: 0,
                value: 0,
            })?;
        }
        if scenario == "drive_fault"
            && tick >= cycles / 3 + 10
            && reset.status().phase == FaultResetPhase::Idle
            && guard.state() == LifecycleState::FaultLatched
        {
            check(reset.submit(
                &guard,
                FaultResetRequest {
                    boot_id: BOOT,
                    sequence: 1,
                    transition_sequence: guard.transition_sequence(),
                    axis_mask: 1,
                    deadline_ns: now + 5 * period,
                },
                now,
            ))?;
        }
        let mut context = StopCycleContext {
            guard: &mut guard,
            bank: &mut axes,
            master: &mut master,
            port,
            domain: domains.domain::<IMAGE, 2>(0).ok_or("motion binding")?,
            dc: &dc,
            buffer: &buffer,
            event_cursor: &mut cursor,
            state: &mut state,
            report: report.received().report,
            other,
            maps: active.axis_pdo_maps(),
            modes: active.axis_modes(),
            axis_policies: active.axis_policies(),
            max_stationary_velocities: &[1; AXES],
            safe_process_image: copy_image.published(),
            shared_process_image: Some(copy_image.published_image()),
            plan: &motion_plan,
            next_generation: generation.wrapping_add(1),
            deadline_ns: now + 2 * period,
            now_ns: now,
            transition_time_ns: now,
        };
        let outcome = if matches!(
            reset.status().phase,
            FaultResetPhase::Pending | FaultResetPhase::ZeroRequired
        ) {
            check(context.run_service_cycle_with_fault_reset_until(
                &owner,
                &domains,
                &report,
                0,
                &outputs,
                &mut reset,
                now + budget,
            ))?
        } else {
            check(context.run_service_cycle_with_outputs_until(
                &domains,
                &report,
                0,
                &outputs,
                &targets,
                &mut setpoint_guards,
                &limits,
                now + budget,
            ))?
        };
        { owner.settle_output(&outputs, &outcome, now + 2 * period) }
            .map_err(|e| format!("settle tick{tick} {e:?}"))?;
        if load && load_started {
            for (i, handle) in handles.iter().enumerate() {
                if let Some(h) = handle {
                    let status = check(runtime.status(*h))?;
                    if !accounted[i]
                        && matches!(
                            status.phase,
                            SdoRequestPhase::Complete | SdoRequestPhase::Failed
                        )
                    {
                        if status.phase != SdoRequestPhase::Complete {
                            return Err(format!("runtime SDO {status:?}"));
                        }
                        if load_phase % 2 == 1 {
                            let bytes = check(runtime.result(*h))?;
                            if bytes.len() != 1024
                                || bytes
                                    .iter()
                                    .enumerate()
                                    .any(|(n, v)| *v != (n as u8).wrapping_add(i as u8))
                            {
                                return Err("SDO object verification mismatch".into());
                            }
                            result.sdo_verified += 1;
                        }
                        accounted[i] = true;
                        result.sdo_completed += 1;
                        evidence.record_sdo(0, 1, 0, 0);
                    }
                }
            }
            if handles.iter().all(|h| h.is_some()) && accounted.iter().all(|done| *done) {
                for handle in &mut handles {
                    check(runtime.release(handle.take().ok_or("missing SDO")?))?;
                }
                load_phase += 1;
                if tick < cycles.saturating_sub(100) {
                    fill_load(
                        &mut runtime,
                        &stations,
                        &mut handles,
                        load_phase % 2 == 1,
                        now,
                        origin + (cycles + 10) * period,
                    )?;
                    accounted.fill(false);
                    evidence.record_sdo(8, 0, 0, 8);
                }
            }
        }
        if tick >= cycles / 3
            && tick < cycles / 3 + 12
            && matches!(
                guard.state(),
                LifecycleState::Stopping | LifecycleState::FaultLatched
            )
        {
            result.injected_fault_observed = true;
        }
        if tick > cycles / 3 + 5 && tick < cycles * 2 / 3 && guard.state() == LifecycleState::Active
        {
            result.implicit_rearm = true;
        }
        match guard.state() {
            LifecycleState::Active => result.active_cycles += 1,
            LifecycleState::Stopping => result.stopping_cycles += 1,
            _ => {}
        }
        if outcome.acknowledged {
            confirmed_stops += 1;
        }
        let published = buffer.read_state().ok_or("published State unavailable")?;
        let (record, len) = state_record(&published.state);
        state_digest.update((len as u64).to_le_bytes());
        state_digest.update(&record[..len]);
        evidence.record_state(&record[..len]);
        while let Some(event) = buffer.pop_event() {
            let record = event_record(event);
            event_digest.update((record.len() as u64).to_le_bytes());
            event_digest.update(record);
            evidence.record_event(&record);
        }
        for slave in product.slaves {
            if let Some(mailbox) = runtime.mailbox(slave.station_address) {
                while let Some(event) = mailbox.pop_emergency() {
                    let _ = event;
                    result.emergencies_consumed += 1;
                }
            }
        }
        while let Some(event) = monitor.pop_event() {
            let _ = event;
        }
        if matches!(scenario, "full" | "Q2" | "q2") {
            let snapshot = check(i2c.snapshot(now))?;
            if i2c_fault == 0 && tick >= cycles / 4 && !snapshot.pending {
                check(i2c.submit(now, period, 1, I2cOutcome::Nack))?;
                i2c_fault = 1;
                i2c_fault_cycle = tick;
            } else if i2c_fault == 1 && snapshot.nacks != 0 && tick >= i2c_fault_cycle + 3 {
                check(i2c.request_recovery(cycle))?;
                check(i2c.reconfigure(cycle))?;
                i2c_fault = 2;
            } else if i2c_fault == 2 && tick >= cycles / 2 && !snapshot.pending {
                check(i2c.submit(now, period, 0, I2cOutcome::NeverCompletes))?;
                i2c_fault = 3;
                i2c_fault_cycle = tick;
            } else if i2c_fault == 3 && snapshot.timeouts != 0 && tick >= i2c_fault_cycle + 3 {
                check(i2c.request_recovery(cycle))?;
                check(i2c.reconfigure(cycle))?;
                i2c_fault = 4;
            }
        }
        check(i2c.service_cycle(now, cycle))?;
        let snapshot = check(i2c.snapshot(now))?;
        evidence.record_auxiliary_errors(
            snapshot.nacks - nack_count,
            snapshot.timeouts - timeout_count,
            0,
            u64::from(report.received().dc_result.is_err()),
            0,
        );
        nack_count = snapshot.nacks;
        timeout_count = snapshot.timeouts;
        let fast = cycle_start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        evidence.record_cycle(
            &report.received().report,
            CycleTiming {
                release_jitter_abs_ns: jitter,
                fast_path_ns: fast,
                rx_round_trip_ns: rx_elapsed,
            },
            outcome.transmission.as_ref().copied().unwrap_or(0) as u64,
            report.received().report.received_bytes as u64,
            report.received().report.parsed_datagrams as u64,
        );
        if outcome.post_tx_deadline_met != Some(true)
            || outcome.post_publication_deadline_met != Some(true)
        {
            evidence.record_deadline_miss();
        }
        if outcome.transmission.is_err() {
            evidence.record_tx_starvation();
        }
    }
    result.fault_reset_complete = reset.status().phase == FaultResetPhase::Complete;
    result.sdo_outstanding = handles
        .iter()
        .flatten()
        .filter(|h| {
            runtime
                .status(**h)
                .is_ok_and(|s| matches!(s.phase, SdoRequestPhase::Queued | SdoRequestPhase::Active))
        })
        .count() as u64;
    result.confirmed_stops = confirmed_stops;
    result.explicit_rearm = rearmed;
    result.transitions = guard.transition_sequence();
    result.final_state = guard.state() as u8;
    result.state_sha256 = state_digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    result.event_sha256 = event_digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    result.software_scenario_passed = result.domain_complete_cycles >= cycles.saturating_sub(10)
        && result.dc_good_cycles >= cycles.saturating_sub(10)
        && !result.implicit_rearm
        && result.injected_fault_observed
        && (!port.motion_allowed()
            || (result.active_cycles > 0
                && confirmed_stops >= 2
                && rearmed
                && guard.state() != LifecycleState::Active))
        && (!load || (result.sdo_completed >= 16 && result.sdo_verified >= 8))
        && (q4_baseline || !port.motion_allowed() || result.emergencies_consumed > 0)
        && (scenario != "drive_fault" || result.fault_reset_complete);
    Ok(result)
}
