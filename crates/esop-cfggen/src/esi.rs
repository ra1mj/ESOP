use crate::error::{GeneratorError, Result};
use esop_ethercat_core::{
    ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1, MAX_ESC_SYNC_MANAGERS, SYNC_MANAGER_ENABLE_FLAG,
    SYNC_MANAGER_OP_ONLY_FLAG,
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
    pub sync_managers: Vec<EsiSyncManager>,
    pub rx_pdos: Vec<EsiPdo>,
    pub tx_pdos: Vec<EsiPdo>,
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
    pub activation: u8,
    pub op_only: bool,
}

impl EsiSyncManager {
    pub fn is_output(&self) -> bool {
        self.direction.eq_ignore_ascii_case("Outputs")
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
    sync_managers: Vec<EsiSyncManager>,
    rx_pdos: Vec<EsiPdo>,
    tx_pdos: Vec<EsiPdo>,
}

impl DeviceBuilder {
    fn finish(self) -> std::result::Result<EsiDevice, String> {
        Ok(EsiDevice {
            type_name: self.type_name.ok_or("Device Type text is missing")?,
            name: self.name.ok_or("Device Name is missing")?,
            product_code: self
                .product_code
                .ok_or("Device Type ProductCode is missing")?,
            revision: self.revision.ok_or("Device Type RevisionNo is missing")?,
            transition_timeouts: self.transition_timeouts,
            sync_managers: self.sync_managers,
            rx_pdos: self.rx_pdos,
            tx_pdos: self.tx_pdos,
        })
    }
}

struct SyncManagerBuilder {
    enabled: bool,
    op_only: bool,
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
            activation,
            op_only: self.op_only,
        })
    }
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
                        sync_manager = Some(SyncManagerBuilder { enabled, op_only });
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
                if matches!(name.as_str(), "Device" | "RxPdo" | "TxPdo" | "Entry" | "Sm") {
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
<Sm Enable="1">MBoxOut</Sm><Sm Enable="1">MBoxIn</Sm>
<Sm Enable="true" OpOnly="1">Outputs</Sm><Sm Enable="1">Inputs</Sm>
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
