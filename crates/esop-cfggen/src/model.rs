use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

macro_rules! hex_type {
    ($name:ident, $inner:ty, $width:expr) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub $inner);

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&format!("0x{:0width$x}", self.0, width = $width))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                parse_hex(&value)
                    .and_then(|value| <$inner>::try_from(value).map_err(|_| "value out of range"))
                    .map(Self)
                    .map_err(serde::de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "0x{:0width$x}", self.0, width = $width)
            }
        }
    };
}

fn parse_hex(value: &str) -> std::result::Result<u64, &'static str> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or("expected a 0x-prefixed hexadecimal string")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid hexadecimal string");
    }
    u64::from_str_radix(digits, 16).map_err(|_| "hexadecimal value out of range")
}

hex_type!(HexU16, u16, 4);
hex_type!(HexU32, u32, 8);
hex_type!(HexU64, u64, 16);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductManifest {
    pub schema_version: String,
    pub product: ProductIdentity,
    pub procbuf: ProcBufManifest,
    pub cycle: CycleManifest,
    pub capacities: GeneratorCapacities,
    pub platform: PlatformManifest,
    pub domains: Vec<DomainManifest>,
    pub slaves: Vec<SlaveManifest>,
    pub axes: Vec<AxisManifest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductIdentity {
    pub name: String,
    pub robot_id: HexU64,
    pub policy_version: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcBufManifest {
    pub axes: u16,
    pub io_channels: u16,
    pub domains: u16,
    pub event_capacity: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CycleManifest {
    pub base_period_ns: u64,
    pub deadline_ns: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorCapacities {
    pub max_process_image_bytes: usize,
    pub max_pdo_entries_per_domain: usize,
    pub max_datagrams_per_domain: usize,
    pub max_frames_per_domain: usize,
    pub max_schedule_slots: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformManifest {
    pub board: String,
    pub soc: String,
    pub port: String,
    pub rtos_or_kernel: String,
    pub cache_policy: String,
    #[serde(default)]
    pub dma_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainManifest {
    pub id: u8,
    pub name: String,
    pub logical_address: HexU32,
    pub process_image_offset: usize,
    pub process_image_capacity_bytes: usize,
    pub period_ticks: u32,
    pub phase_ticks: u32,
    pub max_pdo_entries: usize,
    pub max_frames: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EsiSource {
    pub label: String,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlaveKind {
    Cia402Drive,
    EthercatIo,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlaveManifest {
    pub name: String,
    pub kind: SlaveKind,
    pub position: u16,
    pub station_address: HexU16,
    pub esi: EsiSource,
    pub vendor_id: HexU32,
    pub product_code: HexU32,
    pub revision: HexU32,
    #[serde(default)]
    pub serial: Option<HexU32>,
    pub domain_id: u8,
    pub rx_pdos: Vec<HexU16>,
    pub tx_pdos: Vec<HexU16>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AxisMode {
    Csp,
    Csv,
    Cst,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AxisManifest {
    pub index: u8,
    pub name: String,
    pub slave: String,
    pub mode: AxisMode,
    pub policy: AxisPolicyManifest,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AxisPolicyManifest {
    pub position_units_per_radian: f64,
    pub velocity_units_per_radian_per_second: f64,
    pub torque_units_per_newton_metre: f64,
    pub position_offset: i32,
    pub min_position_radians: f64,
    pub max_position_radians: f64,
    pub max_velocity_radians_per_second: f64,
    pub max_torque_newton_metres: f64,
    pub max_position_step_radians: f64,
}
