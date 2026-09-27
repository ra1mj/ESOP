use crate::error::{GeneratorError, Result};
use esop_ethercat_core::{
    ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1, MAX_ESC_SYNC_MANAGERS, MAX_SII_FMMU_USAGES,
    MailboxConfig, SYNC_MANAGER_ENABLE_FLAG, SYNC_MANAGER_OP_ONLY_FLAG,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EsiCatalog {
    pub vendor_id: u32,
    pub devices: Vec<EsiDevice>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EsiDevice {
    pub type_name: String,
    pub name: String,
    pub product_code: u32,
    pub revision: u32,
    pub transition_timeouts: EsiTransitionTimeouts,
    pub fmmu_usages: Vec<EsiFmmuUsage>,
    pub sync_managers: Vec<EsiSyncManager>,
    pub mailbox: Option<EsiMailbox>,
    pub coe_supported: bool,
    pub dc_modes: Vec<EsiDcMode>,
    pub rx_pdos: Vec<EsiPdo>,
    pub tx_pdos: Vec<EsiPdo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EsiFmmuUsage {
    Unused,
    Outputs,
    Inputs,
    SyncManagerStatus,
    Unspecified,
}

impl EsiFmmuUsage {
    pub const fn raw(self) -> u8 {
        match self {
            Self::Unused => 0,
            Self::Outputs => 1,
            Self::Inputs => 2,
            Self::SyncManagerStatus => 3,
            Self::Unspecified => 0xff,
        }
    }

    pub const fn rust_variant(self) -> &'static str {
        match self {
            Self::Unused => "Unused",
            Self::Outputs => "Outputs",
            Self::Inputs => "Inputs",
            Self::SyncManagerStatus => "SyncManagerStatus",
            Self::Unspecified => "Unspecified",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EsiDcMode {
    pub name: String,
    pub description: Option<String>,
    pub cycle_time0_ns: u32,
    pub shift_time0_ns: i32,
    pub shift_time1_ns: i32,
    pub sync1_cycle_factor: i16,
    pub assign_activate: u16,
    pub sync0_cycle_factor: i16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct EsiTransitionTimeouts {
    pub preop_ns: u64,
    pub safeop_to_op_ns: u64,
    pub back_to_init_ns: u64,
    pub back_to_safeop_ns: u64,
}

impl Default for EsiTransitionTimeouts {
    fn default() -> Self {
        let value = ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1;
        Self {
            preop_ns: value.preop_ns,
            safeop_to_op_ns: value.safeop_to_op_ns,
            back_to_init_ns: value.back_to_init_ns,
            back_to_safeop_ns: value.back_to_safeop_ns,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EsiSyncManager {
    pub index: u8,
    pub direction: String,
    pub start_address: Option<u16>,
    pub default_size: Option<u16>,
    pub control_byte: Option<u8>,
    pub activation: u8,
    pub op_only: bool,
}

impl EsiSyncManager {
    pub fn is_output(&self) -> bool {
        self.direction.eq_ignore_ascii_case("Outputs")
    }

    pub fn is_enabled(&self) -> bool {
        self.activation & SYNC_MANAGER_ENABLE_FLAG != 0
    }

    pub fn is_mailbox_out(&self) -> bool {
        self.direction.eq_ignore_ascii_case("MBoxOut")
    }

    pub fn is_mailbox_in(&self) -> bool {
        self.direction.eq_ignore_ascii_case("MBoxIn")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct EsiMailbox {
    pub send_address: u16,
    pub send_capacity: u16,
    pub send_control_byte: u8,
    pub receive_address: u16,
    pub receive_capacity: u16,
    pub receive_control_byte: u8,
}

impl EsiMailbox {
    pub fn mailbox_config(self) -> MailboxConfig {
        MailboxConfig::new(
            self.send_address,
            self.send_capacity,
            self.receive_address,
            self.receive_capacity,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EsiPdo {
    pub index: u16,
    pub sync_manager: Option<u8>,
    pub entries: Vec<EsiEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EsiEntry {
    pub index: u16,
    pub subindex: u8,
    pub bit_length: u8,
    pub signed: bool,
    pub name: String,
}

#[derive(Default)]
struct DeviceBuilder {
    type_name: Option<String>,
    name: Option<String>,
    product_code: Option<u32>,
    revision: Option<u32>,
    transition_timeouts: EsiTransitionTimeouts,
    fmmu_usages: Vec<EsiFmmuUsage>,
    sync_managers: Vec<EsiSyncManager>,
    coe_supported: bool,
    dc_modes: Vec<EsiDcMode>,
    rx_pdos: Vec<EsiPdo>,
    tx_pdos: Vec<EsiPdo>,
}

impl DeviceBuilder {
    fn finish(self) -> std::result::Result<EsiDevice, String> {
        if self.fmmu_usages.len() > MAX_SII_FMMU_USAGES {
            return Err(format!(
                "Device declares {} FMMUs, exceeding supported capacity {MAX_SII_FMMU_USAGES}",
                self.fmmu_usages.len()
            ));
        }
        let mailbox = mailbox_from_sync_managers(&self.sync_managers)?;
        for (index, mode) in self.dc_modes.iter().enumerate() {
            if self.dc_modes[..index]
                .iter()
                .any(|existing| existing.name == mode.name)
            {
                return Err(format!("duplicate DC OpMode name {:?}", mode.name));
            }
        }
        Ok(EsiDevice {
            type_name: self.type_name.ok_or("Device Type text is missing")?,
            name: self.name.ok_or("Device Name is missing")?,
            product_code: self
                .product_code
                .ok_or("Device Type ProductCode is missing")?,
            revision: self.revision.ok_or("Device Type RevisionNo is missing")?,
            transition_timeouts: self.transition_timeouts,
            fmmu_usages: self.fmmu_usages,
            sync_managers: self.sync_managers,
            mailbox,
            coe_supported: self.coe_supported,
            dc_modes: self.dc_modes,
            rx_pdos: self.rx_pdos,
            tx_pdos: self.tx_pdos,
        })
    }
}

#[derive(Default)]
struct DcModeBuilder {
    name: Option<String>,
    description: Option<String>,
    cycle_time0_ns: u32,
    shift_time0_ns: i32,
    shift_time1_ns: i32,
    sync1_cycle_factor: i16,
    assign_activate: Option<u16>,
    sync0_cycle_factor: i16,
}

impl DcModeBuilder {
    fn finish(self) -> std::result::Result<EsiDcMode, String> {
        let name = self.name.ok_or("DC OpMode Name is missing")?;
        if name.is_empty() {
            return Err("DC OpMode Name is empty".to_owned());
        }
        Ok(EsiDcMode {
            name,
            description: self.description.filter(|value| !value.is_empty()),
            cycle_time0_ns: self.cycle_time0_ns,
            shift_time0_ns: self.shift_time0_ns,
            shift_time1_ns: self.shift_time1_ns,
            sync1_cycle_factor: self.sync1_cycle_factor,
            assign_activate: self
                .assign_activate
                .ok_or("DC OpMode AssignActivate is missing")?,
            sync0_cycle_factor: self.sync0_cycle_factor,
        })
    }
}

struct SyncManagerBuilder {
    enabled: bool,
    op_only: bool,
    start_address: Option<u16>,
    default_size: Option<u16>,
    control_byte: Option<u8>,
}

impl SyncManagerBuilder {
    fn finish(self, index: usize, direction: &str) -> std::result::Result<EsiSyncManager, String> {
        if direction.is_empty() {
            return Err("Sm direction text is empty".to_owned());
        }
        if index >= MAX_ESC_SYNC_MANAGERS {
            return Err(format!(
                "Sm index {index} exceeds supported SyncManager capacity {MAX_ESC_SYNC_MANAGERS}"
            ));
        }
        if self.op_only && !direction.eq_ignore_ascii_case("Outputs") {
            return Err(format!(
                "Sm {index} declares OpOnly for non-output direction {direction:?}"
            ));
        }
        let activation = if self.enabled {
            SYNC_MANAGER_ENABLE_FLAG
        } else {
            0
        } | if self.op_only {
            SYNC_MANAGER_OP_ONLY_FLAG
        } else {
            0
        };
        Ok(EsiSyncManager {
            index: index as u8,
            direction: direction.to_owned(),
            start_address: self.start_address,
            default_size: self.default_size,
            control_byte: self.control_byte,
            activation,
            op_only: self.op_only,
        })
    }
}

fn mailbox_from_sync_managers(
    sync_managers: &[EsiSyncManager],
) -> std::result::Result<Option<EsiMailbox>, String> {
    let mut mailbox_out = None;
    let mut mailbox_in = None;
    for sync_manager in sync_managers {
        let slot = if sync_manager.is_mailbox_out() {
            &mut mailbox_out
        } else if sync_manager.is_mailbox_in() {
            &mut mailbox_in
        } else {
            continue;
        };
        if slot.replace(sync_manager).is_some() {
            return Err(format!(
                "duplicate {} SyncManager declarations",
                sync_manager.direction
            ));
        }
    }
    let (mailbox_out, mailbox_in) = match (mailbox_out, mailbox_in) {
        (None, None) => return Ok(None),
        (Some(_), None) => return Err("MBoxOut is present but MBoxIn is missing".to_owned()),
        (None, Some(_)) => return Err("MBoxIn is present but MBoxOut is missing".to_owned()),
        (Some(mailbox_out), Some(mailbox_in)) => (mailbox_out, mailbox_in),
    };
    if !mailbox_out.is_enabled() || !mailbox_in.is_enabled() {
        return Err("mailbox SyncManagers must be enabled".to_owned());
    }
    let mailbox = EsiMailbox {
        send_address: mailbox_attribute(mailbox_out, mailbox_out.start_address, "StartAddress")?,
        send_capacity: mailbox_attribute(mailbox_out, mailbox_out.default_size, "DefaultSize")?,
        send_control_byte: mailbox_attribute(mailbox_out, mailbox_out.control_byte, "ControlByte")?,
        receive_address: mailbox_attribute(mailbox_in, mailbox_in.start_address, "StartAddress")?,
        receive_capacity: mailbox_attribute(mailbox_in, mailbox_in.default_size, "DefaultSize")?,
        receive_control_byte: mailbox_attribute(
            mailbox_in,
            mailbox_in.control_byte,
            "ControlByte",
        )?,
    };
    mailbox
        .mailbox_config()
        .validate()
        .map_err(|error| format!("invalid ESI mailbox configuration: {error:?}"))?;
    Ok(Some(mailbox))
}

fn mailbox_attribute<T: Copy>(
    sync_manager: &EsiSyncManager,
    value: Option<T>,
    name: &str,
) -> std::result::Result<T, String> {
    value.ok_or_else(|| format!("{} SyncManager is missing {name}", sync_manager.direction))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdoDirection {
    Rx,
    Tx,
}

struct PdoBuilder {
    direction: PdoDirection,
    index: Option<u16>,
    sync_manager: Option<u8>,
    entries: Vec<EsiEntry>,
}

impl PdoBuilder {
    fn finish(self) -> std::result::Result<(PdoDirection, EsiPdo), String> {
        let index = self.index.ok_or("PDO Index is missing")?;
        if self.entries.is_empty() {
            return Err(format!("PDO 0x{index:04x} has no entries"));
        }
        let bits = self.entries.iter().try_fold(0usize, |total, entry| {
            total
                .checked_add(usize::from(entry.bit_length))
                .ok_or_else(|| format!("PDO 0x{index:04x} bit length overflows"))
        })?;
        if bits % 8 != 0 {
            return Err(format!(
                "PDO 0x{index:04x} is not byte-addressable ({bits} bits)"
            ));
        }
        Ok((
            self.direction,
            EsiPdo {
                index,
                sync_manager: self.sync_manager,
                entries: self.entries,
            },
        ))
    }
}

#[derive(Default)]
struct EntryBuilder {
    index: Option<u16>,
    subindex: Option<u8>,
    bit_length: Option<u8>,
    signed: Option<bool>,
    name: Option<String>,
}

impl EntryBuilder {
    fn finish(self) -> std::result::Result<EsiEntry, String> {
        Ok(EsiEntry {
            index: self.index.ok_or("Entry Index is missing")?,
            subindex: self.subindex.unwrap_or(0),
            bit_length: self.bit_length.ok_or("Entry BitLen is missing")?,
            signed: self
                .signed
                .ok_or("Entry DataType is missing or unsupported")?,
            name: self.name.unwrap_or_default(),
        })
    }
}

pub fn parse(path: &Path) -> Result<EsiCatalog> {
    let xml = fs::read_to_string(path).map_err(|error| GeneratorError::io("read", path, error))?;
    parse_text(path, &xml)
}

fn parse_text(path: &Path, xml: &str) -> Result<EsiCatalog> {
    let mut reader = Reader::from_reader(xml.as_bytes());
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut text = String::new();
    let mut vendor_id = None;
    let mut devices = Vec::new();
    let mut device = None::<DeviceBuilder>;
    let mut pdo = None::<PdoBuilder>;
    let mut entry = None::<EntryBuilder>;
    let mut sync_manager = None::<SyncManagerBuilder>;
    let mut dc_mode = None::<DcModeBuilder>;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| GeneratorError::Xml {
                path: path.to_owned(),
                detail: error.to_string(),
            })?;
        match event {
            Event::Start(start) => {
                let name = local_name(start.name().as_ref());
                stack.push(name.clone());
                text.clear();
                match name.as_str() {
                    "Device" => {
                        if device.is_some() {
                            return xml_error(path, "nested Device elements are unsupported");
                        }
                        device = Some(DeviceBuilder::default());
                    }
                    "Type" if device.is_some() && pdo.is_none() => {
                        let product = required_attribute(&start, "ProductCode", path)?;
                        let revision = required_attribute(&start, "RevisionNo", path)?;
                        let builder = device.as_mut().expect("device checked above");
                        builder.product_code =
                            Some(parse_u32(&product).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid Device ProductCode: {detail}"),
                            })?);
                        builder.revision =
                            Some(parse_u32(&revision).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid Device RevisionNo: {detail}"),
                            })?);
                    }
                    "RxPdo" | "TxPdo" if device.is_some() => {
                        if pdo.is_some() {
                            return xml_error(path, "nested PDO elements are unsupported");
                        }
                        let sync_manager = optional_attribute(&start, "Sm", path)?
                            .map(|value| parse_u8(&value))
                            .transpose()
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid PDO Sm: {detail}"),
                            })?;
                        pdo = Some(PdoBuilder {
                            direction: if name == "RxPdo" {
                                PdoDirection::Rx
                            } else {
                                PdoDirection::Tx
                            },
                            index: None,
                            sync_manager,
                            entries: Vec::new(),
                        });
                    }
                    "Sm" if device.is_some() && pdo.is_none() => {
                        if sync_manager.is_some() {
                            return xml_error(path, "nested Sm elements are unsupported");
                        }
                        let enabled = optional_attribute(&start, "Enable", path)?
                            .map(|value| parse_bool(&value))
                            .transpose()
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid Sm Enable: {detail}"),
                            })?
                            .unwrap_or(true);
                        let op_only = optional_attribute(&start, "OpOnly", path)?
                            .map(|value| parse_bool(&value))
                            .transpose()
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid Sm OpOnly: {detail}"),
                            })?
                            .unwrap_or(false);
                        let start_address = optional_u16_attribute(&start, "StartAddress", path)?;
                        let default_size = optional_u16_attribute(&start, "DefaultSize", path)?;
                        let control_byte = optional_u8_attribute(&start, "ControlByte", path)?;
                        sync_manager = Some(SyncManagerBuilder {
                            enabled,
                            op_only,
                            start_address,
                            default_size,
                            control_byte,
                        });
                    }
                    "OpMode" if dc_mode.is_some() => {
                        return xml_error(path, "nested DC OpMode elements are unsupported");
                    }
                    "OpMode"
                        if device.is_some()
                            && pdo.is_none()
                            && stack_ends_with(&stack, &["Device", "Dc", "OpMode"]) =>
                    {
                        dc_mode = Some(DcModeBuilder::default());
                    }
                    "OpMode" if device.is_some() && pdo.is_none() => {
                        return xml_error(
                            path,
                            "DC OpMode must be nested directly under Device/Dc",
                        );
                    }
                    "CycleTimeSync0"
                        if dc_mode.is_some()
                            && stack_ends_with(
                                &stack,
                                &["Device", "Dc", "OpMode", "CycleTimeSync0"],
                            ) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").sync0_cycle_factor =
                            optional_attribute(&start, "Factor", path)?
                                .map(|value| parse_i16(&value))
                                .transpose()
                                .map_err(|detail| GeneratorError::Xml {
                                    path: path.to_owned(),
                                    detail: format!("invalid DC CycleTimeSync0 Factor: {detail}"),
                                })?
                                .unwrap_or(0);
                    }
                    "CycleTimeSync0" if dc_mode.is_some() => {
                        return xml_error(path, "DC CycleTimeSync0 must be a direct OpMode child");
                    }
                    "CycleTimeSync1"
                        if dc_mode.is_some()
                            && stack_ends_with(
                                &stack,
                                &["Device", "Dc", "OpMode", "CycleTimeSync1"],
                            ) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").sync1_cycle_factor =
                            optional_attribute(&start, "Factor", path)?
                                .map(|value| parse_i16(&value))
                                .transpose()
                                .map_err(|detail| GeneratorError::Xml {
                                    path: path.to_owned(),
                                    detail: format!("invalid DC CycleTimeSync1 Factor: {detail}"),
                                })?
                                .unwrap_or(0);
                    }
                    "CycleTimeSync1" if dc_mode.is_some() => {
                        return xml_error(path, "DC CycleTimeSync1 must be a direct OpMode child");
                    }
                    "CoE"
                        if device.is_some()
                            && pdo.is_none()
                            && stack_ends_with(&stack, &["Mailbox", "CoE"]) =>
                    {
                        device.as_mut().expect("device exists").coe_supported = true;
                    }
                    "Entry" if pdo.is_some() => {
                        if entry.is_some() {
                            return xml_error(path, "nested Entry elements are unsupported");
                        }
                        entry = Some(EntryBuilder::default());
                    }
                    _ => {}
                }
            }
            Event::Empty(start) => {
                let name = local_name(start.name().as_ref());
                if name == "CoE"
                    && pdo.is_none()
                    && stack_ends_with(&stack, &["Device", "Mailbox"])
                    && let Some(device) = device.as_mut()
                {
                    device.coe_supported = true;
                }
                if matches!(
                    name.as_str(),
                    "Device" | "Fmmu" | "RxPdo" | "TxPdo" | "Entry" | "Sm" | "OpMode"
                ) {
                    return xml_error(path, format!("empty {name} elements are unsupported"));
                }
            }
            Event::Text(value) => {
                let decoded = quick_xml::escape::unescape(value.as_ref()).map_err(|error| {
                    GeneratorError::Xml {
                        path: path.to_owned(),
                        detail: error.to_string(),
                    }
                })?;
                text.push_str(&decoded);
            }
            Event::End(end) => {
                let name = local_name(end.name().as_ref());
                let value = text.trim();
                if dc_mode.is_some()
                    && matches!(
                        name.as_str(),
                        "Name"
                            | "Desc"
                            | "AssignActivate"
                            | "CycleTimeSync0"
                            | "ShiftTimeSync0"
                            | "CycleTimeSync1"
                            | "ShiftTimeSync1"
                    )
                    && !stack_ends_with(&stack, &["Dc", "OpMode", name.as_str()])
                {
                    return xml_error(path, format!("DC {name} must be a direct OpMode child"));
                }
                match name.as_str() {
                    "Id" if device.is_none() && stack_ends_with(&stack, &["Vendor", "Id"]) => {
                        vendor_id =
                            Some(parse_u32(value).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid Vendor Id: {detail}"),
                            })?);
                    }
                    "Type" if device.is_some() && pdo.is_none() => {
                        if value.is_empty() {
                            return xml_error(path, "Device Type text is empty");
                        }
                        device.as_mut().expect("device exists").type_name = Some(value.to_owned());
                    }
                    "Fmmu"
                        if device.is_some()
                            && pdo.is_none()
                            && stack_ends_with(&stack, &["Device", "Fmmu"]) =>
                    {
                        let usage = parse_fmmu_usage(value).ok_or_else(|| GeneratorError::Xml {
                            path: path.to_owned(),
                            detail: format!("unsupported Device Fmmu usage {value:?}"),
                        })?;
                        device
                            .as_mut()
                            .expect("device exists")
                            .fmmu_usages
                            .push(usage);
                    }
                    "Fmmu" if device.is_some() && pdo.is_none() => {
                        return xml_error(path, "Fmmu must be a direct Device child");
                    }
                    "Name"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Dc", "OpMode", "Name"]) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").name = Some(value.to_owned());
                    }
                    "Desc"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Dc", "OpMode", "Desc"]) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").description =
                            Some(value.to_owned());
                    }
                    "AssignActivate"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Dc", "OpMode", "AssignActivate"]) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").assign_activate =
                            Some(parse_u16(value).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid DC AssignActivate: {detail}"),
                            })?);
                    }
                    "CycleTimeSync0"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Dc", "OpMode", "CycleTimeSync0"]) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").cycle_time0_ns = parse_u32(value)
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid DC CycleTimeSync0: {detail}"),
                            })?;
                    }
                    "ShiftTimeSync0"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Dc", "OpMode", "ShiftTimeSync0"]) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").shift_time0_ns = parse_i32(value)
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid DC ShiftTimeSync0: {detail}"),
                            })?;
                    }
                    "CycleTimeSync1"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Dc", "OpMode", "CycleTimeSync1"]) =>
                    {
                        let cycle_time =
                            parse_u32(value).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid DC CycleTimeSync1: {detail}"),
                            })?;
                        if cycle_time != 0 {
                            return xml_error(
                                path,
                                "DC CycleTimeSync1 must be zero; SII stores only its factor",
                            );
                        }
                    }
                    "ShiftTimeSync1"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Dc", "OpMode", "ShiftTimeSync1"]) =>
                    {
                        dc_mode.as_mut().expect("DC mode exists").shift_time1_ns = parse_i32(value)
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid DC ShiftTimeSync1: {detail}"),
                            })?;
                    }
                    "Name" if entry.is_some() => {
                        entry.as_mut().expect("entry exists").name = Some(value.to_owned());
                    }
                    "Name" if device.is_some() && pdo.is_none() => {
                        let builder = device.as_mut().expect("device exists");
                        if builder.name.is_none() && !value.is_empty() {
                            builder.name = Some(value.to_owned());
                        }
                    }
                    "Index" if entry.is_some() => {
                        entry.as_mut().expect("entry exists").index =
                            Some(parse_u16(value).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid Entry Index: {detail}"),
                            })?);
                    }
                    "Index" if pdo.is_some() => {
                        pdo.as_mut().expect("pdo exists").index =
                            Some(parse_u16(value).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid PDO Index: {detail}"),
                            })?);
                    }
                    "SubIndex" if entry.is_some() => {
                        entry.as_mut().expect("entry exists").subindex =
                            Some(parse_u8(value).map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("invalid Entry SubIndex: {detail}"),
                            })?);
                    }
                    "BitLen" if entry.is_some() => {
                        let bits = parse_u8(value).map_err(|detail| GeneratorError::Xml {
                            path: path.to_owned(),
                            detail: format!("invalid Entry BitLen: {detail}"),
                        })?;
                        if !(1..=64).contains(&bits) {
                            return xml_error(
                                path,
                                format!("Entry BitLen {bits} is outside 1..=64"),
                            );
                        }
                        entry.as_mut().expect("entry exists").bit_length = Some(bits);
                    }
                    "DataType" if entry.is_some() => {
                        entry.as_mut().expect("entry exists").signed =
                            Some(data_type_signed(value).ok_or_else(|| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: format!("unsupported Entry DataType {value:?}"),
                            })?);
                    }
                    "PreopTimeout"
                        if device.is_some()
                            && stack_ends_with(
                                &stack,
                                &["StateMachine", "Timeout", "PreopTimeout"],
                            ) =>
                    {
                        device
                            .as_mut()
                            .expect("device exists")
                            .transition_timeouts
                            .preop_ns = parse_timeout_ns(value, "PreopTimeout", path)?;
                    }
                    "SafeopOpTimeout"
                        if device.is_some()
                            && stack_ends_with(
                                &stack,
                                &["StateMachine", "Timeout", "SafeopOpTimeout"],
                            ) =>
                    {
                        device
                            .as_mut()
                            .expect("device exists")
                            .transition_timeouts
                            .safeop_to_op_ns = parse_timeout_ns(value, "SafeopOpTimeout", path)?;
                    }
                    "BackToInitTimeout"
                        if device.is_some()
                            && stack_ends_with(
                                &stack,
                                &["StateMachine", "Timeout", "BackToInitTimeout"],
                            ) =>
                    {
                        device
                            .as_mut()
                            .expect("device exists")
                            .transition_timeouts
                            .back_to_init_ns = parse_timeout_ns(value, "BackToInitTimeout", path)?;
                    }
                    "BackToSafeopTimeout"
                        if device.is_some()
                            && stack_ends_with(
                                &stack,
                                &["StateMachine", "Timeout", "BackToSafeopTimeout"],
                            ) =>
                    {
                        device
                            .as_mut()
                            .expect("device exists")
                            .transition_timeouts
                            .back_to_safeop_ns =
                            parse_timeout_ns(value, "BackToSafeopTimeout", path)?;
                    }
                    "Sm" if sync_manager.is_some() => {
                        let builder = sync_manager.take().ok_or_else(|| GeneratorError::Xml {
                            path: path.to_owned(),
                            detail: "Sm end without start".to_owned(),
                        })?;
                        let device = device.as_mut().ok_or_else(|| GeneratorError::Xml {
                            path: path.to_owned(),
                            detail: "Sm outside Device".to_owned(),
                        })?;
                        let finished = builder.finish(device.sync_managers.len(), value).map_err(
                            |detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail,
                            },
                        )?;
                        device.sync_managers.push(finished);
                    }
                    "Entry" => {
                        let finished = entry
                            .take()
                            .ok_or_else(|| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: "Entry end without start".to_owned(),
                            })?
                            .finish()
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail,
                            })?;
                        pdo.as_mut()
                            .ok_or_else(|| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: "Entry outside PDO".to_owned(),
                            })?
                            .entries
                            .push(finished);
                    }
                    "RxPdo" | "TxPdo" => {
                        let (direction, finished) = pdo
                            .take()
                            .ok_or_else(|| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: "PDO end without start".to_owned(),
                            })?
                            .finish()
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail,
                            })?;
                        let builder = device.as_mut().ok_or_else(|| GeneratorError::Xml {
                            path: path.to_owned(),
                            detail: "PDO outside Device".to_owned(),
                        })?;
                        match direction {
                            PdoDirection::Rx => builder.rx_pdos.push(finished),
                            PdoDirection::Tx => builder.tx_pdos.push(finished),
                        }
                    }
                    "OpMode"
                        if dc_mode.is_some()
                            && stack_ends_with(&stack, &["Device", "Dc", "OpMode"]) =>
                    {
                        let finished =
                            dc_mode
                                .take()
                                .expect("DC mode exists")
                                .finish()
                                .map_err(|detail| GeneratorError::Xml {
                                    path: path.to_owned(),
                                    detail,
                                })?;
                        device
                            .as_mut()
                            .ok_or_else(|| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: "DC OpMode outside Device".to_owned(),
                            })?
                            .dc_modes
                            .push(finished);
                    }
                    "Device" => {
                        let finished = device
                            .take()
                            .ok_or_else(|| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail: "Device end without start".to_owned(),
                            })?
                            .finish()
                            .map_err(|detail| GeneratorError::Xml {
                                path: path.to_owned(),
                                detail,
                            })?;
                        devices.push(finished);
                    }
                    _ => {}
                }
                let popped = stack.pop();
                if popped.as_deref() != Some(name.as_str()) {
                    return xml_error(path, "XML element stack mismatch");
                }
                text.clear();
            }
            Event::Eof => break,
            Event::Decl(_)
            | Event::Comment(_)
            | Event::CData(_)
            | Event::PI(_)
            | Event::DocType(_) => {}
            Event::GeneralRef(_) => {}
        }
        buffer.clear();
    }

    if device.is_some()
        || pdo.is_some()
        || entry.is_some()
        || sync_manager.is_some()
        || dc_mode.is_some()
        || !stack.is_empty()
    {
        return xml_error(path, "unterminated ESI element");
    }
    if devices.is_empty() {
        return xml_error(path, "ESI contains no Device elements");
    }
    Ok(EsiCatalog {
        vendor_id: vendor_id.ok_or_else(|| GeneratorError::Xml {
            path: path.to_owned(),
            detail: "Vendor Id is missing".to_owned(),
        })?,
        devices,
    })
}

fn local_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

fn stack_ends_with(stack: &[String], suffix: &[&str]) -> bool {
    stack.len() >= suffix.len()
        && stack[stack.len() - suffix.len()..]
            .iter()
            .map(String::as_str)
            .eq(suffix.iter().copied())
}

fn parse_fmmu_usage(value: &str) -> Option<EsiFmmuUsage> {
    if value.eq_ignore_ascii_case("unused") {
        Some(EsiFmmuUsage::Unused)
    } else if value.eq_ignore_ascii_case("outputs") {
        Some(EsiFmmuUsage::Outputs)
    } else if value.eq_ignore_ascii_case("inputs") {
        Some(EsiFmmuUsage::Inputs)
    } else if value.eq_ignore_ascii_case("mboxstate") || value.eq_ignore_ascii_case("smstatus") {
        Some(EsiFmmuUsage::SyncManagerStatus)
    } else if value.eq_ignore_ascii_case("unspecified") {
        Some(EsiFmmuUsage::Unspecified)
    } else {
        None
    }
}

fn required_attribute(start: &BytesStart<'_>, name: &str, path: &Path) -> Result<String> {
    optional_attribute(start, name, path)?.ok_or_else(|| GeneratorError::Xml {
        path: path.to_owned(),
        detail: format!(
            "{} attribute {name} is missing",
            local_name(start.name().as_ref())
        ),
    })
}

fn optional_attribute(start: &BytesStart<'_>, name: &str, path: &Path) -> Result<Option<String>> {
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| GeneratorError::Xml {
            path: path.to_owned(),
            detail: error.to_string(),
        })?;
        if local_name(attribute.key.as_ref()) == name {
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| GeneratorError::Xml {
                    path: path.to_owned(),
                    detail: error.to_string(),
                })?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn optional_u16_attribute(start: &BytesStart<'_>, name: &str, path: &Path) -> Result<Option<u16>> {
    optional_attribute(start, name, path)?
        .map(|value| parse_u16(&value))
        .transpose()
        .map_err(|detail| GeneratorError::Xml {
            path: path.to_owned(),
            detail: format!("invalid Sm {name}: {detail}"),
        })
}

fn optional_u8_attribute(start: &BytesStart<'_>, name: &str, path: &Path) -> Result<Option<u8>> {
    optional_attribute(start, name, path)?
        .map(|value| parse_u8(&value))
        .transpose()
        .map_err(|detail| GeneratorError::Xml {
            path: path.to_owned(),
            detail: format!("invalid Sm {name}: {detail}"),
        })
}

fn parse_number(value: &str) -> std::result::Result<u64, String> {
    let value = value.trim();
    if let Some(digits) = value
        .strip_prefix("#x")
        .or_else(|| value.strip_prefix("#X"))
        .or_else(|| value.strip_prefix("0x"))
        .or_else(|| value.strip_prefix("0X"))
    {
        return u64::from_str_radix(digits, 16).map_err(|error| error.to_string());
    }
    value.parse::<u64>().map_err(|error| error.to_string())
}

fn parse_u32(value: &str) -> std::result::Result<u32, String> {
    u32::try_from(parse_number(value)?).map_err(|_| "value exceeds u32".to_owned())
}

fn parse_u16(value: &str) -> std::result::Result<u16, String> {
    u16::try_from(parse_number(value)?).map_err(|_| "value exceeds u16".to_owned())
}

fn parse_u8(value: &str) -> std::result::Result<u8, String> {
    u8::try_from(parse_number(value)?).map_err(|_| "value exceeds u8".to_owned())
}

fn parse_i32(value: &str) -> std::result::Result<i32, String> {
    let value = value.trim();
    if value.starts_with("#x")
        || value.starts_with("#X")
        || value.starts_with("0x")
        || value.starts_with("0X")
    {
        return u32::try_from(parse_number(value)?)
            .map(|bits| bits as i32)
            .map_err(|_| "value exceeds i32 storage width".to_owned());
    }
    value.parse::<i32>().map_err(|error| error.to_string())
}

fn parse_i16(value: &str) -> std::result::Result<i16, String> {
    let value = value.trim();
    if value.starts_with("#x")
        || value.starts_with("#X")
        || value.starts_with("0x")
        || value.starts_with("0X")
    {
        return u16::try_from(parse_number(value)?)
            .map(|bits| bits as i16)
            .map_err(|_| "value exceeds i16 storage width".to_owned());
    }
    value.parse::<i16>().map_err(|error| error.to_string())
}

fn parse_bool(value: &str) -> std::result::Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(format!("unsupported boolean {value:?}")),
    }
}

fn parse_timeout_ns(value: &str, field: &str, path: &Path) -> Result<u64> {
    let milliseconds = parse_number(value).map_err(|detail| GeneratorError::Xml {
        path: path.to_owned(),
        detail: format!("invalid {field}: {detail}"),
    })?;
    if milliseconds == 0 {
        return xml_error(path, format!("{field} must be greater than zero"));
    }
    milliseconds
        .checked_mul(1_000_000)
        .ok_or_else(|| GeneratorError::Xml {
            path: path.to_owned(),
            detail: format!("{field} overflows nanoseconds"),
        })
}

fn data_type_signed(value: &str) -> Option<bool> {
    let normalized = value.trim().to_ascii_uppercase();
    if matches!(
        normalized.as_str(),
        "SINT" | "INT" | "DINT" | "LINT" | "INTEGER8" | "INTEGER16" | "INTEGER32" | "INTEGER64"
    ) {
        return Some(true);
    }
    if matches!(
        normalized.as_str(),
        "BOOL"
            | "BIT"
            | "BYTE"
            | "WORD"
            | "DWORD"
            | "LWORD"
            | "USINT"
            | "UINT"
            | "UDINT"
            | "ULINT"
            | "UNSIGNED8"
            | "UNSIGNED16"
            | "UNSIGNED32"
            | "UNSIGNED64"
    ) {
        return Some(false);
    }
    None
}

fn xml_error<T>(path: &Path, detail: impl Into<String>) -> Result<T> {
    Err(GeneratorError::Xml {
        path: path.to_owned(),
        detail: detail.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_qualified_esi_subset_is_parsed() {
        let xml = r##"<?xml version="1.0"?>
<e:EtherCATInfo xmlns:e="urn:ethercat">
  <e:Vendor><e:Id>#x00000002</e:Id></e:Vendor>
  <e:Descriptions><e:Devices><e:Device>
    <e:Type ProductCode="#x00001001" RevisionNo="#x00000003">ESOP Drive</e:Type>
    <e:Name>Drive A</e:Name>
    <e:RxPdo Sm="2"><e:Index>#x1600</e:Index><e:Entry>
      <e:Index>#x6040</e:Index><e:SubIndex>0</e:SubIndex><e:BitLen>16</e:BitLen>
      <e:Name>Controlword</e:Name><e:DataType>UINT</e:DataType>
    </e:Entry></e:RxPdo>
    <e:TxPdo Sm="3"><e:Index>#x1A00</e:Index><e:Entry>
      <e:Index>#x6041</e:Index><e:SubIndex>0</e:SubIndex><e:BitLen>16</e:BitLen>
      <e:Name>Statusword</e:Name><e:DataType>UINT</e:DataType>
    </e:Entry></e:TxPdo>
  </e:Device></e:Devices></e:Descriptions>
</e:EtherCATInfo>"##;
        let catalog = parse_text(Path::new("fixture.xml"), xml).unwrap();
        assert_eq!(catalog.vendor_id, 2);
        assert_eq!(catalog.devices.len(), 1);
        assert_eq!(catalog.devices[0].product_code, 0x1001);
        assert_eq!(catalog.devices[0].rx_pdos[0].sync_manager, Some(2));
        assert!(!catalog.devices[0].rx_pdos[0].entries[0].signed);
        assert_eq!(
            catalog.devices[0].transition_timeouts,
            EsiTransitionTimeouts::default()
        );
        assert!(catalog.devices[0].sync_managers.is_empty());
    }

    #[test]
    fn state_machine_timeouts_and_op_only_outputs_are_parsed() {
        let xml = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">Drive</Type><Name>Drive</Name>
<Sm StartAddress="#x1000" DefaultSize="64" ControlByte="#x26" Enable="1">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22" Enable="1">MBoxIn</Sm>
<Sm Enable="true" OpOnly="1">Outputs</Sm><Sm Enable="1">Inputs</Sm>
<Mailbox><CoE/></Mailbox>
<StateMachine><Timeout><PreopTimeout>11</PreopTimeout><SafeopOpTimeout>22</SafeopOpTimeout>
<BackToInitTimeout>33</BackToInitTimeout><BackToSafeopTimeout>44</BackToSafeopTimeout>
</Timeout></StateMachine>
<RxPdo Sm="2"><Index>#x1600</Index><Entry><Index>#x6040</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></RxPdo>
<TxPdo Sm="3"><Index>#x1A00</Index><Entry><Index>#x6041</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></TxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;
        let catalog = parse_text(Path::new("fixture.xml"), xml).unwrap();
        let device = &catalog.devices[0];
        assert_eq!(
            device.transition_timeouts,
            EsiTransitionTimeouts {
                preop_ns: 11_000_000,
                safeop_to_op_ns: 22_000_000,
                back_to_init_ns: 33_000_000,
                back_to_safeop_ns: 44_000_000,
            }
        );
        assert_eq!(device.sync_managers.len(), 4);
        assert_eq!(device.sync_managers[2].index, 2);
        assert!(device.sync_managers[2].is_output());
        assert!(device.sync_managers[2].op_only);
        assert_eq!(device.sync_managers[2].activation, 0x09);
        assert!(device.coe_supported);
        assert_eq!(
            device.mailbox,
            Some(EsiMailbox {
                send_address: 0x1000,
                send_capacity: 64,
                send_control_byte: 0x26,
                receive_address: 0x1100,
                receive_capacity: 32,
                receive_control_byte: 0x22,
            })
        );
        assert_eq!(
            device.mailbox.unwrap().mailbox_config(),
            MailboxConfig::new(0x1000, 64, 0x1100, 32)
        );
    }

    #[test]
    fn direct_fmmu_usage_descriptors_preserve_standard_order() {
        let xml = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">Drive</Type><Name>Drive</Name>
<Fmmu>Unused</Fmmu><Fmmu>Outputs</Fmmu><Fmmu>Inputs</Fmmu>
<Fmmu>MBoxState</Fmmu><Fmmu>SMStatus</Fmmu><Fmmu>Unspecified</Fmmu>
<RxPdo Sm="2"><Index>#x1600</Index><Entry><Index>#x6040</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></RxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;
        let catalog = parse_text(Path::new("fixture.xml"), xml).unwrap();
        assert_eq!(
            catalog.devices[0].fmmu_usages,
            vec![
                EsiFmmuUsage::Unused,
                EsiFmmuUsage::Outputs,
                EsiFmmuUsage::Inputs,
                EsiFmmuUsage::SyncManagerStatus,
                EsiFmmuUsage::SyncManagerStatus,
                EsiFmmuUsage::Unspecified,
            ]
        );
    }

    #[test]
    fn dc_op_modes_preserve_the_sii_descriptor_fields() {
        let xml = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">Drive</Type><Name>Drive</Name>
<Dc><OpMode><Name>DcSync</Name><Desc>Primary sync mode</Desc><AssignActivate>#x0300</AssignActivate>
<CycleTimeSync0 Factor="1">1000000</CycleTimeSync0><ShiftTimeSync0>-125</ShiftTimeSync0>
<CycleTimeSync1 Factor="#xFFFE">0</CycleTimeSync1><ShiftTimeSync1>#x000000fa</ShiftTimeSync1>
</OpMode><OpMode><Name>FreeRun</Name><AssignActivate>0</AssignActivate></OpMode></Dc>
<RxPdo Sm="2"><Index>#x1600</Index><Entry><Index>#x6040</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></RxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;
        let catalog = parse_text(Path::new("fixture.xml"), xml).unwrap();
        assert_eq!(
            catalog.devices[0].dc_modes,
            vec![
                EsiDcMode {
                    name: "DcSync".to_owned(),
                    description: Some("Primary sync mode".to_owned()),
                    cycle_time0_ns: 1_000_000,
                    shift_time0_ns: -125,
                    shift_time1_ns: 250,
                    sync1_cycle_factor: -2,
                    assign_activate: 0x0300,
                    sync0_cycle_factor: 1,
                },
                EsiDcMode {
                    name: "FreeRun".to_owned(),
                    description: None,
                    cycle_time0_ns: 0,
                    shift_time0_ns: 0,
                    shift_time1_ns: 0,
                    sync1_cycle_factor: 0,
                    assign_activate: 0,
                    sync0_cycle_factor: 0,
                },
            ]
        );
    }

    #[test]
    fn invalid_or_duplicate_dc_op_modes_are_rejected() {
        let base = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">Drive</Type><Name>Drive</Name><Dc>{modes}</Dc>
<RxPdo Sm="2"><Index>#x1600</Index><Entry><Index>#x6040</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></RxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;
        for (modes, expected) in [
            (
                "<OpMode><Name>dc</Name><CycleTimeSync0>1</CycleTimeSync0></OpMode>",
                "AssignActivate is missing",
            ),
            (
                "<OpMode><Name>dc</Name><AssignActivate>1</AssignActivate><CycleTimeSync1>2</CycleTimeSync1></OpMode>",
                "must be zero",
            ),
            (
                "<OpMode><Name>dc</Name><AssignActivate>1</AssignActivate></OpMode><OpMode><Name>dc</Name><AssignActivate>2</AssignActivate></OpMode>",
                "duplicate DC OpMode",
            ),
            (
                "<OpMode><Name>dc</Name><AssignActivate>1</AssignActivate><ShiftTimeSync0>2147483648</ShiftTimeSync0></OpMode>",
                "number too large",
            ),
            (
                "<OpMode><Name>dc</Name><AssignActivate>1</AssignActivate><CycleTimeSync0 Factor=\"32768\">1</CycleTimeSync0></OpMode>",
                "number too large",
            ),
            (
                "<OpMode><Name>dc</Name><AssignActivate>1</AssignActivate><OpMode><Name>nested</Name><AssignActivate>2</AssignActivate></OpMode></OpMode>",
                "nested DC OpMode",
            ),
            (
                "<OpMode><Name>dc</Name><AssignActivate>1</AssignActivate><Wrapper><CycleTimeSync0 Factor=\"1\">1</CycleTimeSync0></Wrapper></OpMode>",
                "direct OpMode child",
            ),
            (
                "<OpMode><Wrapper><Name>dc</Name></Wrapper><AssignActivate>1</AssignActivate></OpMode>",
                "DC Name must be a direct OpMode child",
            ),
        ] {
            let error = parse_text(Path::new("fixture.xml"), &base.replace("{modes}", modes))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(expected),
                "expected {expected:?} in {error:?}"
            );
        }
    }

    #[test]
    fn incomplete_duplicate_disabled_or_overlapping_mailboxes_are_rejected() {
        let base = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">Drive</Type><Name>Drive</Name>
{mailboxes}<Mailbox><CoE/></Mailbox>
<RxPdo Sm="2"><Index>#x1600</Index><Entry><Index>#x6040</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></RxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;

        for (mailboxes, expected) in [
            (
                r##"<Sm StartAddress="#x1000" DefaultSize="32" ControlByte="#x26">MBoxOut</Sm>"##,
                "MBoxIn is missing",
            ),
            (
                r##"<Sm StartAddress="#x1000" DefaultSize="32" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1200" DefaultSize="32" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "duplicate MBoxOut",
            ),
            (
                r##"<Sm StartAddress="#x1000" DefaultSize="32" ControlByte="#x26" Enable="0">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "must be enabled",
            ),
            (
                r##"<Sm DefaultSize="32" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "missing StartAddress",
            ),
            (
                r##"<Sm StartAddress="#x1000" DefaultSize="32" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1010" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "AddressRangeOverlap",
            ),
            (
                r##"<Sm StartAddress="0" DefaultSize="32" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "AddressZero(Send)",
            ),
            (
                r##"<Sm StartAddress="#x1000" DefaultSize="5" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "CapacityTooSmall(Send)",
            ),
            (
                r##"<Sm StartAddress="#x1000" DefaultSize="129" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "CapacityExceeded(Send)",
            ),
            (
                r##"<Sm StartAddress="#xfff0" DefaultSize="32" ControlByte="#x26">MBoxOut</Sm>
<Sm StartAddress="#x1100" DefaultSize="32" ControlByte="#x22">MBoxIn</Sm>"##,
                "AddressRangeOverflow(Send)",
            ),
        ] {
            let xml = base.replace("{mailboxes}", mailboxes);
            assert!(
                parse_text(Path::new("fixture.xml"), &xml)
                    .unwrap_err()
                    .to_string()
                    .contains(expected),
                "expected {expected}"
            );
        }
    }

    #[test]
    fn invalid_timeout_or_non_output_op_only_is_rejected() {
        let zero_timeout = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">IO</Type><Name>IO</Name>
<StateMachine><Timeout><PreopTimeout>0</PreopTimeout></Timeout></StateMachine>
<RxPdo Sm="2"><Index>#x1600</Index><Entry><Index>#x7000</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></RxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;
        assert!(
            parse_text(Path::new("fixture.xml"), zero_timeout)
                .unwrap_err()
                .to_string()
                .contains("must be greater than zero")
        );

        let input_op_only = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">IO</Type><Name>IO</Name>
<Sm Enable="1" OpOnly="true">Inputs</Sm>
<TxPdo Sm="0"><Index>#x1A00</Index><Entry><Index>#x6000</Index><BitLen>16</BitLen>
<DataType>UINT</DataType></Entry></TxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;
        assert!(
            parse_text(Path::new("fixture.xml"), input_op_only)
                .unwrap_err()
                .to_string()
                .contains("non-output")
        );
    }

    #[test]
    fn bit_packed_pdo_is_rejected() {
        let xml = r##"<EtherCATInfo><Vendor><Id>1</Id></Vendor><Descriptions><Devices><Device>
<Type ProductCode="1" RevisionNo="1">IO</Type><Name>IO</Name>
<RxPdo><Index>#x1600</Index><Entry><Index>#x7000</Index><SubIndex>1</SubIndex>
<BitLen>1</BitLen><DataType>BOOL</DataType></Entry></RxPdo>
</Device></Devices></Descriptions></EtherCATInfo>"##;
        let error = parse_text(Path::new("fixture.xml"), xml).unwrap_err();
        assert!(error.to_string().contains("not byte-addressable"));
    }
}
