use crate::error::{GeneratorError, Result};
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
    pub rx_pdos: Vec<EsiPdo>,
    pub tx_pdos: Vec<EsiPdo>,
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
            rx_pdos: self.rx_pdos,
            tx_pdos: self.tx_pdos,
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
                if matches!(name.as_str(), "Device" | "RxPdo" | "TxPdo" | "Entry") {
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

    if device.is_some() || pdo.is_some() || entry.is_some() || !stack.is_empty() {
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
