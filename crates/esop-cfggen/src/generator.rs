use crate::error::{GeneratorError, Result};
use crate::esi::{self, EsiCatalog, EsiDevice, EsiEntry, EsiPdo};
use crate::model::{
    AxisMode, AxisPolicyManifest, DomainManifest, HexU16, HexU32, ProductManifest, SlaveKind,
    SlaveManifest,
};
use esop_ethercat_core::wire::{
    Command, ETHERCAT_FRAME_HEADER_LEN, ETHERNET_HEADER_LEN, MIN_ETHERNET_FRAME_LEN,
};
use esop_ethercat_core::{
    DomainConfig, DomainDatagramSpec, DomainRegistry, FramePlanSet, PdoDirection,
    PdoRegistrationRequest,
};
use esop_lifecycle_guard::procbuf::Cia402AxisCommandPolicy;
use esop_procbuf::{ABI_VERSION, ProcBufDimensions, ProcBufLayoutDescriptor, describe_layout};
use esop_product_config::{
    MAX_PRODUCT_AXIS_PDOS, MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN, PRODUCT_RUNTIME_SCHEMA,
};
use esop_profile_cia402::{Cia402PdoMap, OperatingMode};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

const PRODUCT_SCHEMA: &str = "esop.product.v1";
const MAX_DOMAINS: usize = 16;
const MAX_DATAGRAMS_PER_DOMAIN: usize = 2;
const MAX_FRAMES_PER_DOMAIN: usize = 8;
const MAX_DATAGRAMS_PER_FRAME: usize = 16;
const MAX_SCHEDULE_SLOTS: usize = 1024;
const MAX_GENERATED_TEXT_BYTES: usize = 128;
const ETHERNET_FCS_BYTES: usize = 4;
const ETHERNET_PREAMBLE_SFD_BYTES: usize = 8;
const ETHERNET_INTER_PACKET_GAP_BYTES: usize = 12;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum GeneratedDirection {
    Rx,
    Tx,
}

impl GeneratedDirection {
    const fn core(self) -> PdoDirection {
        match self {
            Self::Rx => PdoDirection::Rx,
            Self::Tx => PdoDirection::Tx,
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Rx => 0,
            Self::Tx => 1,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct GeneratedPdo {
    domain_id: u8,
    slave_position: u16,
    assignment_index: HexU16,
    sync_manager: u8,
    object_index: HexU16,
    subindex: u8,
    direction: GeneratedDirection,
    bit_offset: usize,
    bit_length: u8,
    signed: bool,
    name: String,
}

#[derive(Clone, Debug, Serialize)]
struct GeneratedDatagram {
    domain_id: u8,
    command: &'static str,
    command_code: u8,
    index: u8,
    logical_address: HexU32,
    domain_offset: usize,
    process_image_offset: usize,
    payload_len: usize,
    expected_wkc: u16,
    input: bool,
}

#[derive(Clone, Debug, Serialize)]
struct GeneratedFrame {
    index: usize,
    datagram_indices: Vec<u8>,
    wire_bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
struct GeneratedDomain {
    id: u8,
    name: String,
    logical_address: HexU32,
    process_image_offset: usize,
    process_image_bytes: usize,
    output_bytes: usize,
    input_bytes: usize,
    period_ticks: u32,
    phase_ticks: u32,
    expected_wkc: u16,
    input_expected_wkc: u16,
    pdo_count: usize,
    datagram_count: usize,
    frames: Vec<GeneratedFrame>,
}

#[derive(Clone, Debug, Serialize)]
struct GeneratedSlave {
    name: String,
    kind: SlaveKind,
    position: u16,
    station_address: HexU16,
    domain_id: u8,
    vendor_id: HexU32,
    product_code: HexU32,
    revision: HexU32,
    serial: Option<HexU32>,
    esi_label: String,
    esi_semantic_sha256: String,
    esi_type_name: String,
    esi_device_name: String,
    rx_pdos: Vec<HexU16>,
    tx_pdos: Vec<HexU16>,
}

#[derive(Clone, Debug, Serialize)]
struct GeneratedAxis {
    index: u8,
    name: String,
    slave: String,
    slave_position: u16,
    mode: AxisMode,
    mode_raw: i8,
    policy: AxisPolicyManifest,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct CycleMetrics {
    pdo_bytes_per_cycle: usize,
    frame_count: usize,
    expected_wkc: u16,
    copy_bytes_per_cycle: usize,
    wire_bytes_per_cycle: usize,
}

#[derive(Clone, Debug)]
struct ResolvedSlave {
    manifest: SlaveManifest,
    device: EsiDevice,
    rx_pdos: Vec<EsiPdo>,
    tx_pdos: Vec<EsiPdo>,
    semantic_sha256: String,
}

#[derive(Clone, Debug)]
struct DomainWork {
    manifest: DomainManifest,
    pdos: Vec<GeneratedPdo>,
    output_bytes: usize,
    input_bytes: usize,
    output_wkc: u16,
    input_wkc: u16,
}

#[derive(Clone, Debug)]
pub struct GenerationSummary {
    pub config_sha256: String,
    pub artifact_count: usize,
}

struct GeneratedArtifacts {
    files: BTreeMap<&'static str, Vec<u8>>,
    summary: GenerationSummary,
}

pub fn generate(input: &Path, output: &Path) -> Result<GenerationSummary> {
    let artifacts = build_artifacts(input)?;
    publish(output, &artifacts.files)?;
    Ok(artifacts.summary)
}

fn build_artifacts(input: &Path) -> Result<GeneratedArtifacts> {
    let source = fs::read(input).map_err(|error| GeneratorError::io("read", input, error))?;
    let mut manifest: ProductManifest =
        serde_json::from_slice(&source).map_err(|source| GeneratorError::Json {
            path: input.to_owned(),
            source,
        })?;
    normalize_and_validate_manifest(&mut manifest)?;
    let base = input.parent().unwrap_or_else(|| Path::new("."));
    let resolved = resolve_slaves(base, &manifest.slaves)?;
    let BuiltDomainPlan {
        mut registry,
        mut domains,
        pdos,
        datagrams,
    } = build_domain_plan(&manifest, &resolved)?;
    let axes = validate_axes(&manifest, &resolved, &pdos)?;
    let procbuf = describe_layout(ProcBufDimensions {
        axes: manifest.procbuf.axes,
        io_channels: manifest.procbuf.io_channels,
        domains: manifest.procbuf.domains,
        event_capacity: manifest.procbuf.event_capacity,
    })
    .map_err(|error| GeneratorError::ProcBuf(format!("{error:?}")))?;

    let mut frame_plans =
        [FramePlanSet::<MAX_FRAMES_PER_DOMAIN, MAX_DATAGRAMS_PER_FRAME>::new(); MAX_DOMAINS];
    let schedule = registry
        .activate_with_frame_plans::<
            MAX_SCHEDULE_SLOTS,
            MAX_FRAMES_PER_DOMAIN,
            MAX_DATAGRAMS_PER_FRAME,
        >(manifest.cycle.base_period_ns, &mut frame_plans)
        .map_err(|error| GeneratorError::Registry(format!("{error:?}")))?;
    if schedule.hyperperiod_ticks() as usize > manifest.capacities.max_schedule_slots {
        return Err(GeneratorError::Invalid(format!(
            "schedule hyperperiod {} exceeds declared max_schedule_slots {}",
            schedule.hyperperiod_ticks(),
            manifest.capacities.max_schedule_slots
        )));
    }
    for (index, domain) in domains.iter_mut().enumerate() {
        let plans = &frame_plans[index];
        let declared = manifest
            .domains
            .iter()
            .find(|candidate| candidate.id == domain.id)
            .expect("normalized domain exists");
        if plans.frame_count() > declared.max_frames
            || plans.frame_count() > manifest.capacities.max_frames_per_domain
        {
            return Err(GeneratorError::Invalid(format!(
                "domain {} needs {} frames, exceeding its declared limit",
                domain.name,
                plans.frame_count()
            )));
        }
        domain.frames = plans
            .plans()
            .iter()
            .enumerate()
            .map(|(frame_index, plan)| GeneratedFrame {
                index: frame_index,
                datagram_indices: plan
                    .datagrams()
                    .iter()
                    .map(|datagram| datagram.index)
                    .collect(),
                wire_bytes: frame_wire_bytes(plan.payload_len()),
            })
            .collect();
    }
    let metrics = calculate_cycle_metrics(&domains, &schedule)?;
    let generated_slaves = resolved
        .iter()
        .map(|slave| GeneratedSlave {
            name: slave.manifest.name.clone(),
            kind: slave.manifest.kind,
            position: slave.manifest.position,
            station_address: slave.manifest.station_address,
            domain_id: slave.manifest.domain_id,
            vendor_id: slave.manifest.vendor_id,
            product_code: slave.manifest.product_code,
            revision: slave.manifest.revision,
            serial: slave.manifest.serial,
            esi_label: slave.manifest.esi.label.clone(),
            esi_semantic_sha256: slave.semantic_sha256.clone(),
            esi_type_name: slave.device.type_name.clone(),
            esi_device_name: slave.device.name.clone(),
            rx_pdos: slave.manifest.rx_pdos.clone(),
            tx_pdos: slave.manifest.tx_pdos.clone(),
        })
        .collect::<Vec<_>>();

    let semantic = semantic_identity(
        &manifest,
        &generated_slaves,
        &domains,
        &pdos,
        &datagrams,
        &axes,
    )?;
    let config_sha256 = sha256_json(&semantic)?;

    let product_config = json!({
        "schema_version": "esop.product-config.v1",
        "config_sha256": config_sha256,
        "product": manifest.product,
        "cycle": manifest.cycle,
        "capacities": manifest.capacities,
        "slaves": generated_slaves,
        "domains": domains,
        "pdo_entries": pdos,
        "datagrams": datagrams,
        "axes": axes,
        "schedule": {
            "base_tick_ns": schedule.base_tick_ns(),
            "hyperperiod_ticks": schedule.hyperperiod_ticks(),
            "due_masks": (0..schedule.hyperperiod_ticks())
                .map(|tick| format!("0x{:016x}", schedule.due_mask(tick)))
                .collect::<Vec<_>>(),
        },
        "cycle_metrics": metrics,
    });
    let inventory = json!({
        "schema_version": "esop.device-inventory.v1",
        "config_sha256": config_sha256,
        "devices": generated_slaves,
    });
    let procbuf_layout = procbuf_json(procbuf, &config_sha256);
    let build_input = robot_build_input(&manifest, &config_sha256, metrics, procbuf);
    let header = render_header(
        &manifest,
        &config_sha256,
        &generated_slaves,
        &domains,
        &pdos,
        &datagrams,
        &axes,
        procbuf,
    );
    let rust_module = render_rust_module(
        &manifest,
        &config_sha256,
        &generated_slaves,
        &domains,
        &pdos,
        &datagrams,
        &axes,
        procbuf,
    )?;

    let mut files = BTreeMap::new();
    files.insert("esop_product_config.h", header.into_bytes());
    files.insert("esop_product_config.rs", rust_module.into_bytes());
    files.insert("product_config.json", pretty_json(&product_config)?);
    files.insert("device_inventory.json", pretty_json(&inventory)?);
    files.insert("procbuf_layout.json", pretty_json(&procbuf_layout)?);
    files.insert("robot_build_input.json", pretty_json(&build_input)?);
    Ok(GeneratedArtifacts {
        summary: GenerationSummary {
            config_sha256,
            artifact_count: files.len(),
        },
        files,
    })
}

fn normalize_and_validate_manifest(manifest: &mut ProductManifest) -> Result<()> {
    if manifest.schema_version != PRODUCT_SCHEMA {
        return Err(GeneratorError::Invalid(format!(
            "unsupported schema_version {:?}; expected {PRODUCT_SCHEMA}",
            manifest.schema_version
        )));
    }
    validate_text("product.name", &manifest.product.name)?;
    if manifest.product.robot_id.0 == 0 || manifest.product.policy_version == 0 {
        return Err(GeneratorError::Invalid(
            "product robot_id and policy_version must be nonzero".to_owned(),
        ));
    }
    if manifest.cycle.base_period_ns == 0
        || manifest.cycle.deadline_ns == 0
        || manifest.cycle.deadline_ns > manifest.cycle.base_period_ns
    {
        return Err(GeneratorError::Invalid(
            "cycle period/deadline must be nonzero and deadline must not exceed period".to_owned(),
        ));
    }
    let capacities = manifest.capacities;
    if capacities.max_process_image_bytes == 0
        || capacities.max_pdo_entries_per_domain == 0
        || capacities.max_datagrams_per_domain == 0
        || capacities.max_frames_per_domain == 0
        || capacities.max_schedule_slots == 0
        || capacities.max_pdo_entries_per_domain > MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN
        || capacities.max_datagrams_per_domain > MAX_DATAGRAMS_PER_DOMAIN
        || capacities.max_frames_per_domain > MAX_FRAMES_PER_DOMAIN
        || capacities.max_schedule_slots > MAX_SCHEDULE_SLOTS
    {
        return Err(GeneratorError::Invalid(
            "declared generator capacities are zero or exceed compiled cfggen limits".to_owned(),
        ));
    }
    if manifest.domains.is_empty() || manifest.slaves.is_empty() {
        return Err(GeneratorError::Invalid(
            "at least one Domain and one slave are required".to_owned(),
        ));
    }
    if manifest.domains.len() > MAX_DOMAINS
        || usize::from(manifest.procbuf.domains) != manifest.domains.len()
        || usize::from(manifest.procbuf.axes) != manifest.axes.len()
        || manifest.procbuf.axes > 32
    {
        return Err(GeneratorError::Invalid(
            "ProcBuf dimensions do not match product counts or exceed supported limits".to_owned(),
        ));
    }

    manifest.domains.sort_by_key(|domain| domain.id);
    manifest.slaves.sort_by_key(|slave| slave.position);
    manifest.axes.sort_by_key(|axis| axis.index);

    let mut domain_ids = BTreeSet::new();
    let mut domain_names = BTreeSet::new();
    let mut image_ranges = Vec::new();
    for domain in &manifest.domains {
        validate_text("domain.name", &domain.name)?;
        if !domain_ids.insert(domain.id) || !domain_names.insert(domain.name.clone()) {
            return Err(GeneratorError::Invalid(format!(
                "duplicate Domain id or name for {}",
                domain.name
            )));
        }
        if domain.process_image_capacity_bytes == 0
            || domain.max_pdo_entries == 0
            || domain.max_frames == 0
            || domain.max_pdo_entries > capacities.max_pdo_entries_per_domain
            || domain.max_frames > capacities.max_frames_per_domain
        {
            return Err(GeneratorError::Invalid(format!(
                "Domain {} has invalid capacity limits",
                domain.name
            )));
        }
        let end = domain
            .process_image_offset
            .checked_add(domain.process_image_capacity_bytes)
            .ok_or_else(|| GeneratorError::Invalid("process image range overflows".to_owned()))?;
        if end > capacities.max_process_image_bytes {
            return Err(GeneratorError::Invalid(format!(
                "Domain {} process image range exceeds product capacity",
                domain.name
            )));
        }
        if image_ranges
            .iter()
            .any(|(start, other_end): &(usize, usize)| {
                domain.process_image_offset < *other_end && *start < end
            })
        {
            return Err(GeneratorError::Invalid(format!(
                "Domain {} process image capacity overlaps another Domain",
                domain.name
            )));
        }
        image_ranges.push((domain.process_image_offset, end));
    }

    let mut names = BTreeSet::new();
    let mut positions = BTreeSet::new();
    let mut stations = BTreeSet::new();
    for slave in &manifest.slaves {
        validate_text("slave.name", &slave.name)?;
        validate_text("slave.esi.label", &slave.esi.label)?;
        if !is_confined_relative_path(Path::new(&slave.esi.path)) {
            return Err(GeneratorError::Invalid(format!(
                "slave {} ESI path must be a confined relative path",
                slave.name
            )));
        }
        if !names.insert(slave.name.clone())
            || !positions.insert(slave.position)
            || slave.station_address.0 == 0
            || !stations.insert(slave.station_address.0)
        {
            return Err(GeneratorError::Invalid(format!(
                "slave {} has duplicate name, position, or station address",
                slave.name
            )));
        }
        if !domain_ids.contains(&slave.domain_id) {
            return Err(GeneratorError::Invalid(format!(
                "slave {} references unknown Domain {}",
                slave.name, slave.domain_id
            )));
        }
        reject_duplicates(&slave.rx_pdos, &format!("slave {} rx_pdos", slave.name))?;
        reject_duplicates(&slave.tx_pdos, &format!("slave {} tx_pdos", slave.name))?;
        if slave.rx_pdos.is_empty() && slave.tx_pdos.is_empty() {
            return Err(GeneratorError::Invalid(format!(
                "slave {} selects no PDOs",
                slave.name
            )));
        }
    }

    let mut axis_names = BTreeSet::new();
    for (expected, axis) in manifest.axes.iter().enumerate() {
        validate_text("axis.name", &axis.name)?;
        if usize::from(axis.index) != expected || !axis_names.insert(axis.name.clone()) {
            return Err(GeneratorError::Invalid(
                "axis indices must be unique and contiguous from zero, and names must be unique"
                    .to_owned(),
            ));
        }
        if !names.contains(&axis.slave) {
            return Err(GeneratorError::Invalid(format!(
                "axis {} references unknown slave {}",
                axis.name, axis.slave
            )));
        }
    }
    Ok(())
}

fn resolve_slaves(base: &Path, slaves: &[SlaveManifest]) -> Result<Vec<ResolvedSlave>> {
    let base =
        fs::canonicalize(base).map_err(|error| GeneratorError::io("canonicalize", base, error))?;
    let mut catalogs = HashMap::<PathBuf, EsiCatalog>::new();
    let mut resolved = Vec::with_capacity(slaves.len());
    let mut source_labels = BTreeMap::<String, String>::new();
    for manifest in slaves {
        let requested_path = base.join(&manifest.esi.path);
        let path = fs::canonicalize(&requested_path)
            .map_err(|error| GeneratorError::io("canonicalize", &requested_path, error))?;
        if !path.starts_with(&base) {
            return Err(GeneratorError::Invalid(format!(
                "slave {} ESI path resolves outside the product directory",
                manifest.name
            )));
        }
        let catalog = if let Some(catalog) = catalogs.get(&path) {
            catalog.clone()
        } else {
            let catalog = esi::parse(&path)?;
            catalogs.insert(path.clone(), catalog.clone());
            catalog
        };
        if catalog.vendor_id != manifest.vendor_id.0 {
            return Err(GeneratorError::Invalid(format!(
                "slave {} expects vendor {}, ESI contains 0x{:08x}",
                manifest.name, manifest.vendor_id, catalog.vendor_id
            )));
        }
        let matches = catalog
            .devices
            .iter()
            .filter(|device| {
                device.product_code == manifest.product_code.0
                    && device.revision == manifest.revision.0
            })
            .cloned()
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(GeneratorError::Invalid(format!(
                "slave {} identity {} / {} matched {} ESI devices",
                manifest.name,
                manifest.product_code,
                manifest.revision,
                matches.len()
            )));
        }
        let device = matches.into_iter().next().expect("one device matched");
        let rx_pdos = select_pdos(&device.rx_pdos, &manifest.rx_pdos, &manifest.name, "Rx")?;
        let tx_pdos = select_pdos(&device.tx_pdos, &manifest.tx_pdos, &manifest.name, "Tx")?;
        validate_selected_entries(&manifest.name, GeneratedDirection::Rx, &rx_pdos)?;
        validate_selected_entries(&manifest.name, GeneratedDirection::Tx, &tx_pdos)?;
        let semantic_sha256 = sha256_json(&json!({
            "vendor_id": catalog.vendor_id,
            "device": device,
        }))?;
        if let Some(previous) =
            source_labels.insert(manifest.esi.label.clone(), semantic_sha256.clone())
        {
            if previous != semantic_sha256 {
                return Err(GeneratorError::Invalid(format!(
                    "ESI label {:?} refers to different semantic content",
                    manifest.esi.label
                )));
            }
        }
        resolved.push(ResolvedSlave {
            manifest: manifest.clone(),
            device,
            rx_pdos,
            tx_pdos,
            semantic_sha256,
        });
    }
    Ok(resolved)
}

fn select_pdos(
    available: &[EsiPdo],
    selected: &[HexU16],
    slave: &str,
    direction: &str,
) -> Result<Vec<EsiPdo>> {
    let mut result = Vec::with_capacity(selected.len());
    for assignment in selected {
        let matches = available
            .iter()
            .filter(|pdo| pdo.index == assignment.0)
            .cloned()
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(GeneratorError::Invalid(format!(
                "slave {slave} selected {direction}PDO {assignment}, matched {} entries",
                matches.len()
            )));
        }
        let pdo = matches.into_iter().next().expect("one PDO matched");
        if pdo.sync_manager.is_none() {
            return Err(GeneratorError::Invalid(format!(
                "slave {slave} selected {direction}PDO {assignment} without an Sm number"
            )));
        }
        result.push(pdo);
    }
    Ok(result)
}

fn validate_selected_entries(
    slave: &str,
    direction: GeneratedDirection,
    pdos: &[EsiPdo],
) -> Result<()> {
    let mut objects = BTreeSet::new();
    for pdo in pdos {
        for entry in &pdo.entries {
            if !objects.insert((entry.index, entry.subindex)) {
                return Err(GeneratorError::Invalid(format!(
                    "slave {slave} has duplicate {:?} object 0x{:04x}:{} in selected PDOs",
                    direction, entry.index, entry.subindex
                )));
            }
        }
    }
    Ok(())
}

type Registry =
    DomainRegistry<MAX_DOMAINS, MAX_PRODUCT_PDO_ENTRIES_PER_DOMAIN, MAX_DATAGRAMS_PER_DOMAIN>;

struct BuiltDomainPlan {
    registry: Registry,
    domains: Vec<GeneratedDomain>,
    pdos: Vec<GeneratedPdo>,
    datagrams: Vec<GeneratedDatagram>,
}

fn build_domain_plan(
    manifest: &ProductManifest,
    slaves: &[ResolvedSlave],
) -> Result<BuiltDomainPlan> {
    let mut work = Vec::with_capacity(manifest.domains.len());
    for domain in &manifest.domains {
        let domain_slaves = slaves
            .iter()
            .filter(|slave| slave.manifest.domain_id == domain.id)
            .collect::<Vec<_>>();
        if domain_slaves.is_empty() {
            return Err(GeneratorError::Invalid(format!(
                "Domain {} contains no slaves",
                domain.name
            )));
        }
        let mut pdos = Vec::new();
        let mut bit_offset = 0usize;
        let mut output_slaves = BTreeSet::new();
        let mut input_slaves = BTreeSet::new();
        for slave in &domain_slaves {
            append_slave_pdos(
                &mut pdos,
                &mut bit_offset,
                slave,
                GeneratedDirection::Rx,
                &slave.rx_pdos,
            )?;
            if !slave.rx_pdos.is_empty() {
                output_slaves.insert(slave.manifest.position);
            }
        }
        let output_bytes = bit_offset / 8;
        for slave in &domain_slaves {
            append_slave_pdos(
                &mut pdos,
                &mut bit_offset,
                slave,
                GeneratedDirection::Tx,
                &slave.tx_pdos,
            )?;
            if !slave.tx_pdos.is_empty() {
                input_slaves.insert(slave.manifest.position);
            }
        }
        let image_bytes = bit_offset / 8;
        let input_bytes = image_bytes.saturating_sub(output_bytes);
        if image_bytes == 0
            || image_bytes > domain.process_image_capacity_bytes
            || pdos.len() > domain.max_pdo_entries
            || pdos.len() > manifest.capacities.max_pdo_entries_per_domain
        {
            return Err(GeneratorError::Invalid(format!(
                "Domain {} generated image/PDO count exceeds declared capacity",
                domain.name
            )));
        }
        work.push(DomainWork {
            manifest: domain.clone(),
            pdos,
            output_bytes,
            input_bytes,
            output_wkc: u16::try_from(output_slaves.len()).map_err(|_| {
                GeneratorError::Invalid("output working counter exceeds u16".to_owned())
            })?,
            input_wkc: u16::try_from(input_slaves.len()).map_err(|_| {
                GeneratorError::Invalid("input working counter exceeds u16".to_owned())
            })?,
        });
    }

    let mut registry = Registry::new();
    let mut generated_domains = Vec::with_capacity(work.len());
    let mut generated_pdos = Vec::new();
    let mut generated_datagrams = Vec::new();
    let mut datagram_index = 0u16;
    for domain in work {
        let image_bytes = domain.output_bytes + domain.input_bytes;
        registry
            .register_domain(DomainConfig::new(
                domain.manifest.id,
                domain.manifest.logical_address.0,
                domain.manifest.process_image_offset,
                image_bytes,
                domain.manifest.period_ticks,
                domain.manifest.phase_ticks,
            ))
            .map_err(|error| GeneratorError::Registry(format!("{error:?}")))?;
        for pdo in &domain.pdos {
            registry
                .register_pdo_at(
                    domain.manifest.id,
                    pdo.bit_offset,
                    PdoRegistrationRequest::new(
                        pdo.slave_position,
                        pdo.object_index.0,
                        pdo.subindex,
                        pdo.direction.clone().core(),
                        pdo.bit_length,
                        pdo.signed,
                    ),
                )
                .map_err(|error| GeneratorError::Registry(format!("{error:?}")))?;
        }
        if domain.output_bytes != 0 {
            let index = next_datagram_index(&mut datagram_index)?;
            let generated = register_datagram(
                &mut registry,
                &domain,
                Command::Lwr,
                index,
                0,
                domain.output_bytes,
                domain.output_wkc,
                false,
            )?;
            generated_datagrams.push(generated);
        }
        if domain.input_bytes != 0 {
            let index = next_datagram_index(&mut datagram_index)?;
            let generated = register_datagram(
                &mut registry,
                &domain,
                Command::Lrd,
                index,
                domain.output_bytes,
                domain.input_bytes,
                domain.input_wkc,
                true,
            )?;
            generated_datagrams.push(generated);
        }
        let info = registry
            .domain(domain.manifest.id)
            .map_err(|error| GeneratorError::Registry(format!("{error:?}")))?;
        generated_domains.push(GeneratedDomain {
            id: domain.manifest.id,
            name: domain.manifest.name.clone(),
            logical_address: domain.manifest.logical_address,
            process_image_offset: domain.manifest.process_image_offset,
            process_image_bytes: image_bytes,
            output_bytes: domain.output_bytes,
            input_bytes: domain.input_bytes,
            period_ticks: domain.manifest.period_ticks,
            phase_ticks: domain.manifest.phase_ticks,
            expected_wkc: info.expected_wkc,
            input_expected_wkc: info.input_expected_wkc,
            pdo_count: info.pdo_count,
            datagram_count: info.datagram_count,
            frames: Vec::new(),
        });
        generated_pdos.extend(domain.pdos);
    }
    Ok(BuiltDomainPlan {
        registry,
        domains: generated_domains,
        pdos: generated_pdos,
        datagrams: generated_datagrams,
    })
}

fn append_slave_pdos(
    output: &mut Vec<GeneratedPdo>,
    bit_offset: &mut usize,
    slave: &ResolvedSlave,
    direction: GeneratedDirection,
    pdos: &[EsiPdo],
) -> Result<()> {
    *bit_offset = bit_offset.div_ceil(8) * 8;
    for pdo in pdos {
        let sync_manager = pdo.sync_manager.ok_or_else(|| {
            GeneratorError::Invalid(format!(
                "slave {} PDO 0x{:04x} has no SyncManager",
                slave.manifest.name, pdo.index
            ))
        })?;
        for entry in &pdo.entries {
            output.push(generated_pdo(
                slave,
                direction.clone(),
                pdo,
                entry,
                *bit_offset,
                sync_manager,
            ));
            *bit_offset = bit_offset
                .checked_add(usize::from(entry.bit_length))
                .ok_or_else(|| GeneratorError::Invalid("PDO bit offset overflows".to_owned()))?;
        }
    }
    if *bit_offset % 8 != 0 {
        return Err(GeneratorError::Invalid(format!(
            "slave {} {:?} PDO selection is not byte-addressable",
            slave.manifest.name, direction
        )));
    }
    Ok(())
}

fn generated_pdo(
    slave: &ResolvedSlave,
    direction: GeneratedDirection,
    pdo: &EsiPdo,
    entry: &EsiEntry,
    bit_offset: usize,
    sync_manager: u8,
) -> GeneratedPdo {
    GeneratedPdo {
        domain_id: slave.manifest.domain_id,
        slave_position: slave.manifest.position,
        assignment_index: HexU16(pdo.index),
        sync_manager,
        object_index: HexU16(entry.index),
        subindex: entry.subindex,
        direction,
        bit_offset,
        bit_length: entry.bit_length,
        signed: entry.signed,
        name: entry.name.clone(),
    }
}

fn next_datagram_index(next: &mut u16) -> Result<u8> {
    let value = u8::try_from(*next)
        .map_err(|_| GeneratorError::Invalid("datagram index capacity exceeded".to_owned()))?;
    *next = next.saturating_add(1);
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn register_datagram(
    registry: &mut Registry,
    domain: &DomainWork,
    command: Command,
    index: u8,
    domain_offset: usize,
    payload_len: usize,
    expected_wkc: u16,
    input: bool,
) -> Result<GeneratedDatagram> {
    if expected_wkc == 0 {
        return Err(GeneratorError::Invalid(format!(
            "Domain {} generated a zero-WKC datagram",
            domain.manifest.name
        )));
    }
    let logical_address = domain
        .manifest
        .logical_address
        .0
        .checked_add(
            u32::try_from(domain_offset).map_err(|_| {
                GeneratorError::Invalid("Domain logical offset exceeds u32".to_owned())
            })?,
        )
        .ok_or_else(|| GeneratorError::Invalid("Domain logical address overflows".to_owned()))?;
    registry
        .register_datagram(
            domain.manifest.id,
            DomainDatagramSpec::new(
                command,
                index,
                logical_address,
                domain_offset,
                payload_len,
                expected_wkc,
                input,
            ),
        )
        .map_err(|error| GeneratorError::Registry(format!("{error:?}")))?;
    Ok(GeneratedDatagram {
        domain_id: domain.manifest.id,
        command: match command {
            Command::Lrd => "lrd",
            Command::Lwr => "lwr",
            _ => "unsupported",
        },
        command_code: command as u8,
        index,
        logical_address: HexU32(logical_address),
        domain_offset,
        process_image_offset: domain.manifest.process_image_offset + domain_offset,
        payload_len,
        expected_wkc,
        input,
    })
}

fn validate_axes(
    manifest: &ProductManifest,
    slaves: &[ResolvedSlave],
    pdos: &[GeneratedPdo],
) -> Result<Vec<GeneratedAxis>> {
    let mut used_drives = BTreeSet::new();
    let mut axes = Vec::with_capacity(manifest.axes.len());
    for axis in &manifest.axes {
        let slave = slaves
            .iter()
            .find(|slave| slave.manifest.name == axis.slave)
            .expect("manifest references were validated");
        if slave.manifest.kind != SlaveKind::Cia402Drive {
            return Err(GeneratorError::Cia402(format!(
                "axis {} references non-drive slave {}",
                axis.name, axis.slave
            )));
        }
        if !used_drives.insert(slave.manifest.position) {
            return Err(GeneratorError::Cia402(format!(
                "multiple axes reference drive {}; multi-axis ESI devices are not supported yet",
                axis.slave
            )));
        }
        let mode = operating_mode(axis.mode);
        let policy = runtime_policy(axis.policy);
        policy.validate_for_product().map_err(|error| {
            GeneratorError::Cia402(format!("axis {} policy: {error:?}", axis.name))
        })?;
        let entries = pdos
            .iter()
            .filter(|pdo| pdo.slave_position == slave.manifest.position)
            .map(|pdo| esop_ethercat_core::PdoEntry {
                index: pdo.object_index.0,
                subindex: pdo.subindex,
                bit_offset: pdo.bit_offset,
                bit_length: pdo.bit_length,
                signed: pdo.signed,
                direction: pdo.direction.clone().core(),
            })
            .collect::<Vec<_>>();
        if entries.len() > MAX_PRODUCT_AXIS_PDOS {
            return Err(GeneratorError::Cia402(format!(
                "axis {} owns {} PDO entries, exceeding runtime limit {}",
                axis.name,
                entries.len(),
                MAX_PRODUCT_AXIS_PDOS
            )));
        }
        Cia402PdoMap::from_pdo_entries(&entries)
            .and_then(|map| map.validate_for(mode))
            .map_err(|error| {
                GeneratorError::Cia402(format!("axis {} PDO map: {error:?}", axis.name))
            })?;
        axes.push(GeneratedAxis {
            index: axis.index,
            name: axis.name.clone(),
            slave: axis.slave.clone(),
            slave_position: slave.manifest.position,
            mode: axis.mode,
            mode_raw: mode.raw(),
            policy: axis.policy,
        });
    }
    Ok(axes)
}

const fn operating_mode(mode: AxisMode) -> OperatingMode {
    match mode {
        AxisMode::Csp => OperatingMode::Csp,
        AxisMode::Csv => OperatingMode::Csv,
        AxisMode::Cst => OperatingMode::Cst,
    }
}

const fn runtime_policy(policy: AxisPolicyManifest) -> Cia402AxisCommandPolicy {
    Cia402AxisCommandPolicy {
        position_units_per_radian: policy.position_units_per_radian,
        velocity_units_per_radian_per_second: policy.velocity_units_per_radian_per_second,
        torque_units_per_newton_metre: policy.torque_units_per_newton_metre,
        position_offset: policy.position_offset,
        min_position_radians: policy.min_position_radians,
        max_position_radians: policy.max_position_radians,
        max_velocity_radians_per_second: policy.max_velocity_radians_per_second,
        max_torque_newton_metres: policy.max_torque_newton_metres,
        max_position_step_radians: policy.max_position_step_radians,
    }
}

fn calculate_cycle_metrics(
    domains: &[GeneratedDomain],
    schedule: &esop_ethercat_core::ScheduleTable<MAX_DOMAINS, MAX_SCHEDULE_SLOTS>,
) -> Result<CycleMetrics> {
    let mut maximum = CycleMetrics {
        pdo_bytes_per_cycle: 0,
        frame_count: 0,
        expected_wkc: 0,
        copy_bytes_per_cycle: 0,
        wire_bytes_per_cycle: 0,
    };
    for tick in 0..schedule.hyperperiod_ticks() {
        let due = schedule.due_mask(tick);
        let mut current = CycleMetrics {
            pdo_bytes_per_cycle: 0,
            frame_count: 0,
            expected_wkc: 0,
            copy_bytes_per_cycle: 0,
            wire_bytes_per_cycle: 0,
        };
        for domain in domains {
            if due & (1u64 << domain.id) == 0 {
                continue;
            }
            current.pdo_bytes_per_cycle = current
                .pdo_bytes_per_cycle
                .checked_add(domain.process_image_bytes)
                .ok_or_else(|| GeneratorError::Invalid("cycle PDO bytes overflow".to_owned()))?;
            current.frame_count = current
                .frame_count
                .checked_add(domain.frames.len())
                .ok_or_else(|| GeneratorError::Invalid("cycle frame count overflow".to_owned()))?;
            current.expected_wkc = current
                .expected_wkc
                .checked_add(domain.expected_wkc)
                .ok_or_else(|| GeneratorError::Invalid("cycle WKC overflows".to_owned()))?;
            current.copy_bytes_per_cycle = current
                .copy_bytes_per_cycle
                .checked_add(domain.input_bytes)
                .ok_or_else(|| GeneratorError::Invalid("cycle copy bytes overflow".to_owned()))?;
            current.wire_bytes_per_cycle = current
                .wire_bytes_per_cycle
                .checked_add(
                    domain
                        .frames
                        .iter()
                        .map(|frame| frame.wire_bytes)
                        .sum::<usize>(),
                )
                .ok_or_else(|| GeneratorError::Invalid("cycle wire bytes overflow".to_owned()))?;
        }
        maximum.pdo_bytes_per_cycle = maximum.pdo_bytes_per_cycle.max(current.pdo_bytes_per_cycle);
        maximum.frame_count = maximum.frame_count.max(current.frame_count);
        maximum.expected_wkc = maximum.expected_wkc.max(current.expected_wkc);
        maximum.copy_bytes_per_cycle = maximum
            .copy_bytes_per_cycle
            .max(current.copy_bytes_per_cycle);
        maximum.wire_bytes_per_cycle = maximum
            .wire_bytes_per_cycle
            .max(current.wire_bytes_per_cycle);
    }
    Ok(maximum)
}

fn frame_wire_bytes(ethercat_datagram_bytes: usize) -> usize {
    let mac_frame_bytes = MIN_ETHERNET_FRAME_LEN.max(
        ETHERNET_HEADER_LEN
            + ETHERCAT_FRAME_HEADER_LEN
            + ethercat_datagram_bytes
            + ETHERNET_FCS_BYTES,
    );
    ETHERNET_PREAMBLE_SFD_BYTES + mac_frame_bytes + ETHERNET_INTER_PACKET_GAP_BYTES
}

fn semantic_identity(
    manifest: &ProductManifest,
    slaves: &[GeneratedSlave],
    domains: &[GeneratedDomain],
    pdos: &[GeneratedPdo],
    datagrams: &[GeneratedDatagram],
    axes: &[GeneratedAxis],
) -> Result<Value> {
    let mut normalized_manifest = serde_json::to_value(manifest).map_err(|error| {
        GeneratorError::Invalid(format!("cannot serialize normalized manifest: {error}"))
    })?;
    if let Some(slaves) = normalized_manifest
        .get_mut("slaves")
        .and_then(Value::as_array_mut)
    {
        for slave in slaves {
            if let Some(esi) = slave.get_mut("esi").and_then(Value::as_object_mut) {
                esi.remove("path");
            }
        }
    }
    let esi_sources = slaves
        .iter()
        .map(|slave| (slave.esi_label.clone(), slave.esi_semantic_sha256.clone()))
        .collect::<BTreeMap<_, _>>();
    Ok(json!({
        "manifest": normalized_manifest,
        "esi_sources": esi_sources,
        "domains": domains,
        "pdo_entries": pdos,
        "datagrams": datagrams,
        "axes": axes,
    }))
}

fn procbuf_json(descriptor: ProcBufLayoutDescriptor, config_sha256: &str) -> Value {
    json!({
        "schema_version": "esop.procbuf-layout.v1",
        "config_sha256": config_sha256,
        "abi_version": ABI_VERSION,
        "dimensions": {
            "axes": descriptor.dimensions.axes,
            "io_channels": descriptor.dimensions.io_channels,
            "domains": descriptor.dimensions.domains,
            "event_capacity": descriptor.dimensions.event_capacity,
        },
        "header_bytes": descriptor.header_bytes,
        "command_page_bytes": descriptor.command_page_bytes,
        "state_page_bytes": descriptor.state_page_bytes,
        "event_record_bytes": descriptor.event_record_bytes,
        "event_ring_bytes": descriptor.event_ring_bytes,
        "region_bytes": descriptor.region_bytes,
        "layout_hash": format!("0x{:016x}", descriptor.layout_hash),
    })
}

fn robot_build_input(
    manifest: &ProductManifest,
    config_sha256: &str,
    metrics: CycleMetrics,
    procbuf: ProcBufLayoutDescriptor,
) -> Value {
    json!({
        "schema_version": "esop.product-build-input.v1",
        "config_sha256": config_sha256,
        "platform": {
            "board": manifest.platform.board,
            "soc": manifest.platform.soc,
            "port": manifest.platform.port,
            "rtos_or_kernel": manifest.platform.rtos_or_kernel,
            "cache_policy": manifest.platform.cache_policy,
        },
        "devices": {
            "declared_slaves": manifest.slaves.len(),
            "declared_axes": manifest.axes.len(),
            "declared_io_channels": manifest.procbuf.io_channels,
            "source": "esop-cfggen validated product manifest and ESI subset",
        },
        "process_data": metrics,
        "cycle_budget": {
            "period_ns": manifest.cycle.base_period_ns,
            "deadline_ns": manifest.cycle.deadline_ns,
            "qualification": "generated_not_measured",
        },
        "resources": {
            "procbuf_bytes": procbuf.region_bytes,
            "dma_bytes": manifest.platform.dma_bytes,
            "rt_stack_peak_bytes": null,
            "text_rodata_bytes": null,
        },
        "qualification": {
            "scenario": "generated-simulator-product",
            "passed": false,
            "failures": [
                "generated topology is not physical HIL evidence",
                "cycle deadline and target resource use are not measured",
            ],
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn render_header(
    manifest: &ProductManifest,
    config_sha256: &str,
    slaves: &[GeneratedSlave],
    domains: &[GeneratedDomain],
    pdos: &[GeneratedPdo],
    datagrams: &[GeneratedDatagram],
    axes: &[GeneratedAxis],
    procbuf: ProcBufLayoutDescriptor,
) -> String {
    let mut header = String::from(
        "#ifndef ESOP_PRODUCT_CONFIG_H\n#define ESOP_PRODUCT_CONFIG_H\n\n#include <stdint.h>\n\n",
    );
    header.push_str("typedef struct { const char *name; uint16_t position; uint16_t station_address; uint8_t domain_id; uint8_t kind; uint32_t vendor_id; uint32_t product_code; uint32_t revision; uint32_t serial; uint8_t has_serial; } esop_slave_config_t;\n");
    header.push_str("typedef struct { const char *name; uint8_t id; uint32_t logical_address; uint32_t image_offset; uint32_t image_bytes; uint32_t output_bytes; uint32_t input_bytes; uint32_t period_ticks; uint32_t phase_ticks; uint16_t expected_wkc; } esop_domain_config_t;\n");
    header.push_str("typedef struct { uint8_t domain_id; uint16_t slave_position; uint16_t assignment_index; uint8_t sync_manager; uint16_t object_index; uint8_t subindex; uint8_t direction; uint32_t bit_offset; uint8_t bit_length; uint8_t is_signed; } esop_pdo_config_t;\n");
    header.push_str("typedef struct { uint8_t domain_id; uint8_t command; uint8_t index; uint32_t logical_address; uint32_t image_offset; uint16_t payload_len; uint16_t expected_wkc; uint8_t input; } esop_datagram_config_t;\n");
    header.push_str("typedef struct { const char *name; uint8_t index; uint16_t slave_position; int8_t mode; double position_scale; double velocity_scale; double torque_scale; int32_t position_offset; double min_position; double max_position; double max_velocity; double max_torque; double max_position_step; } esop_axis_config_t;\n\n");
    header.push_str(&format!(
        "#define ESOP_PRODUCT_NAME {}\n#define ESOP_CONFIG_SHA256 {}\n#define ESOP_ROBOT_ID UINT64_C(0x{:016x})\n#define ESOP_POLICY_VERSION UINT32_C({})\n",
        c_string(&manifest.product.name),
        c_string(config_sha256),
        manifest.product.robot_id.0,
        manifest.product.policy_version,
    ));
    header.push_str(&format!(
        "#define ESOP_SLAVE_COUNT {}u\n#define ESOP_DOMAIN_COUNT {}u\n#define ESOP_PDO_COUNT {}u\n#define ESOP_DATAGRAM_COUNT {}u\n#define ESOP_AXIS_COUNT {}u\n",
        slaves.len(),
        domains.len(),
        pdos.len(),
        datagrams.len(),
        axes.len(),
    ));
    header.push_str("#define ESOP_SLAVE_STORAGE_COUNT (ESOP_SLAVE_COUNT ? ESOP_SLAVE_COUNT : 1u)\n#define ESOP_DOMAIN_STORAGE_COUNT (ESOP_DOMAIN_COUNT ? ESOP_DOMAIN_COUNT : 1u)\n#define ESOP_PDO_STORAGE_COUNT (ESOP_PDO_COUNT ? ESOP_PDO_COUNT : 1u)\n#define ESOP_DATAGRAM_STORAGE_COUNT (ESOP_DATAGRAM_COUNT ? ESOP_DATAGRAM_COUNT : 1u)\n#define ESOP_AXIS_STORAGE_COUNT (ESOP_AXIS_COUNT ? ESOP_AXIS_COUNT : 1u)\n\n");
    header.push_str(&format!(
        "static const uint16_t esop_procbuf_abi_version = {}u;\nstatic const uint32_t esop_procbuf_region_bytes = {}u;\nstatic const uint64_t esop_procbuf_layout_hash = UINT64_C(0x{:016x});\n\n",
        ABI_VERSION, procbuf.region_bytes, procbuf.layout_hash
    ));

    header.push_str("static const esop_slave_config_t esop_slaves[ESOP_SLAVE_STORAGE_COUNT] = {\n");
    if slaves.is_empty() {
        header.push_str("  {0},\n");
    } else {
        for slave in slaves {
            header.push_str(&format!(
                "  {{{}, {}u, UINT16_C(0x{:04x}), {}u, {}u, UINT32_C(0x{:08x}), UINT32_C(0x{:08x}), UINT32_C(0x{:08x}), UINT32_C(0x{:08x}), {}u}},\n",
                c_string(&slave.name),
                slave.position,
                slave.station_address.0,
                slave.domain_id,
                match slave.kind { SlaveKind::Cia402Drive => 1, SlaveKind::EthercatIo => 2 },
                slave.vendor_id.0,
                slave.product_code.0,
                slave.revision.0,
                slave.serial.map_or(0, |value| value.0),
                u8::from(slave.serial.is_some()),
            ));
        }
    }
    header.push_str(
        "};\n\nstatic const esop_domain_config_t esop_domains[ESOP_DOMAIN_STORAGE_COUNT] = {\n",
    );
    for domain in domains {
        header.push_str(&format!(
            "  {{{}, {}u, UINT32_C(0x{:08x}), {}u, {}u, {}u, {}u, {}u, {}u, {}u}},\n",
            c_string(&domain.name),
            domain.id,
            domain.logical_address.0,
            domain.process_image_offset,
            domain.process_image_bytes,
            domain.output_bytes,
            domain.input_bytes,
            domain.period_ticks,
            domain.phase_ticks,
            domain.expected_wkc,
        ));
    }
    header.push_str("};\n\nstatic const esop_pdo_config_t esop_pdos[ESOP_PDO_STORAGE_COUNT] = {\n");
    for pdo in pdos {
        header.push_str(&format!(
            "  {{{}u, {}u, UINT16_C(0x{:04x}), {}u, UINT16_C(0x{:04x}), {}u, {}u, {}u, {}u, {}u}},\n",
            pdo.domain_id, pdo.slave_position, pdo.assignment_index.0, pdo.sync_manager,
            pdo.object_index.0, pdo.subindex, pdo.direction.clone().code(), pdo.bit_offset,
            pdo.bit_length, u8::from(pdo.signed),
        ));
    }
    header.push_str("};\n\nstatic const esop_datagram_config_t esop_datagrams[ESOP_DATAGRAM_STORAGE_COUNT] = {\n");
    for datagram in datagrams {
        header.push_str(&format!(
            "  {{{}u, UINT8_C(0x{:02x}), {}u, UINT32_C(0x{:08x}), {}u, {}u, {}u, {}u}},\n",
            datagram.domain_id,
            datagram.command_code,
            datagram.index,
            datagram.logical_address.0,
            datagram.process_image_offset,
            datagram.payload_len,
            datagram.expected_wkc,
            u8::from(datagram.input),
        ));
    }
    header
        .push_str("};\n\nstatic const esop_axis_config_t esop_axes[ESOP_AXIS_STORAGE_COUNT] = {\n");
    if axes.is_empty() {
        header.push_str("  {0},\n");
    } else {
        for axis in axes {
            let policy = axis.policy;
            header.push_str(&format!(
                "  {{{}, {}u, {}u, {}, {:.17}, {:.17}, {:.17}, {}, {:.17}, {:.17}, {:.17}, {:.17}, {:.17}}},\n",
                c_string(&axis.name), axis.index, axis.slave_position, axis.mode_raw,
                policy.position_units_per_radian,
                policy.velocity_units_per_radian_per_second,
                policy.torque_units_per_newton_metre,
                policy.position_offset,
                policy.min_position_radians,
                policy.max_position_radians,
                policy.max_velocity_radians_per_second,
                policy.max_torque_newton_metres,
                policy.max_position_step_radians,
            ));
        }
    }
    header.push_str("};\n\n#endif\n");
    header
}

#[allow(clippy::too_many_arguments)]
fn render_rust_module(
    manifest: &ProductManifest,
    config_sha256: &str,
    slaves: &[GeneratedSlave],
    domains: &[GeneratedDomain],
    pdos: &[GeneratedPdo],
    datagrams: &[GeneratedDatagram],
    axes: &[GeneratedAxis],
    procbuf: ProcBufLayoutDescriptor,
) -> Result<String> {
    let config_hash = decode_sha256(config_sha256)?;
    let mut output = String::from(
        "// @generated by esop-cfggen; do not edit.\n\
use esop_product_config::{\n\
    Cia402AxisCommandPolicy, Command, DomainConfig, DomainDatagramSpec, OperatingMode,\n\
    PdoDirection, PdoRegistrationRequest, ProcBufDimensions, ProcBufLayoutDescriptor,\n\
    ProductAxisConfig, ProductDatagramConfig, ProductDomainConfig, ProductMetadata,\n\
    ProductPdoConfig, ProductSlaveConfig, ProductSlaveKind, SlaveIdentity,\n\
    StaticProductConfig,\n\
};\n\n",
    );

    output.push_str(&format!(
        "static PRODUCT_PDOS: [ProductPdoConfig; {}] = [\n",
        pdos.len()
    ));
    for pdo in pdos {
        output.push_str(&format!(
            "    ProductPdoConfig {{ domain_id: {}, assignment_index: 0x{:04x}, sync_manager: {}, bit_offset: {}, request: PdoRegistrationRequest::new({}, 0x{:04x}, {}, PdoDirection::{}, {}, {}) }},\n",
            pdo.domain_id,
            pdo.assignment_index.0,
            pdo.sync_manager,
            pdo.bit_offset,
            pdo.slave_position,
            pdo.object_index.0,
            pdo.subindex,
            rust_direction(&pdo.direction),
            pdo.bit_length,
            pdo.signed,
        ));
    }
    output.push_str("];\n\n");

    output.push_str(&format!(
        "static PRODUCT_DATAGRAMS: [ProductDatagramConfig; {}] = [\n",
        datagrams.len()
    ));
    for datagram in datagrams {
        output.push_str(&format!(
            "    ProductDatagramConfig {{ domain_id: {}, spec: DomainDatagramSpec::new(Command::{}, {}, 0x{:08x}, {}, {}, {}, {}) }},\n",
            datagram.domain_id,
            rust_command(datagram.command)?,
            datagram.index,
            datagram.logical_address.0,
            datagram.domain_offset,
            datagram.payload_len,
            datagram.expected_wkc,
            datagram.input,
        ));
    }
    output.push_str("];\n\n");

    output.push_str(&format!(
        "#[allow(clippy::approx_constant)]\npub static PRODUCT_CONFIG: StaticProductConfig<'static, {}, {}, {}> = StaticProductConfig {{\n",
        slaves.len(),
        domains.len(),
        axes.len()
    ));
    output.push_str("    metadata: ProductMetadata {\n");
    output.push_str(&format!(
        "        schema_version: {},\n        product_name: {},\n",
        rust_string(PRODUCT_RUNTIME_SCHEMA),
        rust_string(&manifest.product.name)
    ));
    output.push_str("        config_sha256: [");
    for (index, byte) in config_hash.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        output.push_str(&format!("0x{byte:02x}"));
    }
    output.push_str("],\n");
    output.push_str(&format!(
        "        robot_id: 0x{:016x},\n        policy_version: {},\n        base_period_ns: {},\n        deadline_ns: {},\n    }},\n",
        manifest.product.robot_id.0,
        manifest.product.policy_version,
        manifest.cycle.base_period_ns,
        manifest.cycle.deadline_ns,
    ));
    output.push_str(&format!(
        "    procbuf_layout: ProcBufLayoutDescriptor {{ dimensions: ProcBufDimensions {{ axes: {}, io_channels: {}, domains: {}, event_capacity: {} }}, header_bytes: {}, command_page_bytes: {}, state_page_bytes: {}, event_record_bytes: {}, event_ring_bytes: {}, region_bytes: {}, layout_hash: 0x{:016x} }},\n",
        procbuf.dimensions.axes,
        procbuf.dimensions.io_channels,
        procbuf.dimensions.domains,
        procbuf.dimensions.event_capacity,
        procbuf.header_bytes,
        procbuf.command_page_bytes,
        procbuf.state_page_bytes,
        procbuf.event_record_bytes,
        procbuf.event_ring_bytes,
        procbuf.region_bytes,
        procbuf.layout_hash,
    ));

    output.push_str("    slaves: [\n");
    for slave in slaves {
        output.push_str(&format!(
            "        ProductSlaveConfig {{ name: {}, position: {}, station_address: 0x{:04x}, domain_id: {}, kind: ProductSlaveKind::{}, identity: SlaveIdentity {{ vendor_id: 0x{:08x}, product_code: 0x{:08x}, revision: 0x{:08x}, serial: 0x{:08x} }} }},\n",
            rust_string(&slave.name),
            slave.position,
            slave.station_address.0,
            slave.domain_id,
            rust_slave_kind(slave.kind),
            slave.vendor_id.0,
            slave.product_code.0,
            slave.revision.0,
            slave.serial.map_or(0, |value| value.0),
        ));
    }
    output.push_str("    ],\n");

    output.push_str("    domains: [\n");
    for domain in domains {
        output.push_str(&format!(
            "        ProductDomainConfig {{ name: {}, config: DomainConfig::new({}, 0x{:08x}, {}, {}, {}, {}), expected_pdo_count: {}, expected_datagram_count: {}, expected_wkc: {}, input_expected_wkc: {} }},\n",
            rust_string(&domain.name),
            domain.id,
            domain.logical_address.0,
            domain.process_image_offset,
            domain.process_image_bytes,
            domain.period_ticks,
            domain.phase_ticks,
            domain.pdo_count,
            domain.datagram_count,
            domain.expected_wkc,
            domain.input_expected_wkc,
        ));
    }
    output.push_str("    ],\n    pdos: &PRODUCT_PDOS,\n    datagrams: &PRODUCT_DATAGRAMS,\n");

    output.push_str("    axes: [\n");
    for axis in axes {
        let policy = axis.policy;
        output.push_str(&format!(
            "        ProductAxisConfig {{ name: {}, index: {}, slave_position: {}, mode: OperatingMode::{}, policy: Cia402AxisCommandPolicy {{ position_units_per_radian: {}, velocity_units_per_radian_per_second: {}, torque_units_per_newton_metre: {}, position_offset: {}, min_position_radians: {}, max_position_radians: {}, max_velocity_radians_per_second: {}, max_torque_newton_metres: {}, max_position_step_radians: {} }} }},\n",
            rust_string(&axis.name),
            axis.index,
            axis.slave_position,
            rust_axis_mode(axis.mode),
            rust_float(policy.position_units_per_radian),
            rust_float(policy.velocity_units_per_radian_per_second),
            rust_float(policy.torque_units_per_newton_metre),
            policy.position_offset,
            rust_float(policy.min_position_radians),
            rust_float(policy.max_position_radians),
            rust_float(policy.max_velocity_radians_per_second),
            rust_float(policy.max_torque_newton_metres),
            rust_float(policy.max_position_step_radians),
        ));
    }
    output.push_str("    ],\n};\n");
    Ok(output)
}

fn rust_direction(direction: &GeneratedDirection) -> &'static str {
    match direction {
        GeneratedDirection::Rx => "Rx",
        GeneratedDirection::Tx => "Tx",
    }
}

fn rust_command(command: &str) -> Result<&'static str> {
    match command {
        "lrd" => Ok("Lrd"),
        "lwr" => Ok("Lwr"),
        other => Err(GeneratorError::Invalid(format!(
            "cannot render unsupported datagram command {other:?}"
        ))),
    }
}

const fn rust_slave_kind(kind: SlaveKind) -> &'static str {
    match kind {
        SlaveKind::Cia402Drive => "Cia402Drive",
        SlaveKind::EthercatIo => "EthercatIo",
    }
}

const fn rust_axis_mode(mode: AxisMode) -> &'static str {
    match mode {
        AxisMode::Csp => "Csp",
        AxisMode::Csv => "Csv",
        AxisMode::Cst => "Cst",
    }
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn rust_float(value: f64) -> String {
    format!("{value:?}")
}

fn decode_sha256(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GeneratorError::Invalid(
            "configuration SHA-256 is not 64 hexadecimal characters".to_owned(),
        ));
    }
    let mut output = [0u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| GeneratorError::Invalid("configuration SHA-256 is invalid".to_owned()))?;
    }
    Ok(output)
}

fn c_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for byte in value.as_bytes() {
        match *byte {
            b'"' => escaped.push_str("\\\""),
            b'\\' => escaped.push_str("\\\\"),
            0x20..=0x7e => escaped.push(char::from(*byte)),
            byte => escaped.push_str(&format!("\\{byte:03o}")),
        }
    }
    escaped.push('"');
    escaped
}

fn pretty_json(value: &Value) -> Result<Vec<u8>> {
    let mut output = serde_json::to_vec_pretty(value)
        .map_err(|error| GeneratorError::Invalid(format!("cannot render JSON: {error}")))?;
    output.push(b'\n');
    Ok(output)
}

fn sha256_json(value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        GeneratorError::Invalid(format!("cannot canonicalize JSON for hashing: {error}"))
    })?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn reject_duplicates<T: Ord + Clone>(values: &[T], label: &str) -> Result<()> {
    let mut unique = BTreeSet::new();
    if values.iter().any(|value| !unique.insert(value.clone())) {
        return Err(GeneratorError::Invalid(format!(
            "{label} contains duplicate values"
        )));
    }
    Ok(())
}

fn is_confined_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn validate_text(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.as_bytes().contains(&0)
        || value.len() > MAX_GENERATED_TEXT_BYTES
    {
        return Err(GeneratorError::Invalid(format!(
            "{label} must be nonempty, contain no NUL bytes, and fit within {MAX_GENERATED_TEXT_BYTES} UTF-8 bytes"
        )));
    }
    Ok(())
}

fn publish(output: &Path, artifacts: &BTreeMap<&'static str, Vec<u8>>) -> Result<()> {
    if output.exists() && !output.is_dir() {
        return Err(GeneratorError::Publication(format!(
            "{} exists and is not a directory",
            output.display()
        )));
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| GeneratorError::io("create", parent, error))?;
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            GeneratorError::Publication("output path has no UTF-8 file name".to_owned())
        })?;
    let staging = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let backup = parent.join(format!(".{name}.bak-{}", std::process::id()));
    remove_if_exists(&staging)?;
    remove_if_exists(&backup)?;
    fs::create_dir(&staging).map_err(|error| GeneratorError::io("create", &staging, error))?;
    let write_result = (|| {
        for (file_name, contents) in artifacts {
            let path = staging.join(file_name);
            let mut file =
                File::create(&path).map_err(|error| GeneratorError::io("create", &path, error))?;
            file.write_all(contents)
                .map_err(|error| GeneratorError::io("write", &path, error))?;
            file.sync_all()
                .map_err(|error| GeneratorError::io("sync", &path, error))?;
        }
        File::open(&staging)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| GeneratorError::io("sync", &staging, error))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let had_output = output.exists();
    if had_output {
        fs::rename(output, &backup).map_err(|error| GeneratorError::io("rename", output, error))?;
    }
    if let Err(error) = fs::rename(&staging, output) {
        if had_output {
            let _ = fs::rename(&backup, output);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(GeneratorError::io("rename", &staging, error));
    }
    if had_output {
        fs::remove_dir_all(&backup)
            .map_err(|error| GeneratorError::io("remove", &backup, error))?;
    }
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| GeneratorError::io("sync", parent, error))?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(GeneratorError::io("remove", path, error)),
    }
}
