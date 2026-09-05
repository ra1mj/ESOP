//! Typed CiA 402 process-data binding for the EtherCAT core PDO contract.
//!
//! The profile state machine remains transport-independent. This adapter is
//! enabled with the `ethercat` feature and only translates frozen
//! [`esop_ethercat_core::PdoEntry`] values to and from caller-owned images.

use esop_ethercat_core::{PdoDirection, PdoEntry, PdoError, PdoLayout};

use crate::{CONTROLWORD_ENABLE_OPERATION, OperatingMode};

pub const CONTROLWORD_INDEX: u16 = 0x6040;
pub const MODE_OF_OPERATION_INDEX: u16 = 0x6060;
pub const TARGET_POSITION_INDEX: u16 = 0x607A;
pub const TARGET_VELOCITY_INDEX: u16 = 0x60FF;
pub const TARGET_TORQUE_INDEX: u16 = 0x6071;
pub const STATUSWORD_INDEX: u16 = 0x6041;
pub const MODE_DISPLAY_INDEX: u16 = 0x6061;
pub const ERROR_CODE_INDEX: u16 = 0x603F;
pub const ACTUAL_POSITION_INDEX: u16 = 0x6064;
pub const ACTUAL_VELOCITY_INDEX: u16 = 0x606C;
pub const ACTUAL_TORQUE_INDEX: u16 = 0x6077;
pub const FOLLOWING_ERROR_INDEX: u16 = 0x60F4;

const FIELD_COUNT: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Cia402PdoField {
    Controlword = 0,
    ModeOfOperation = 1,
    TargetPosition = 2,
    TargetVelocity = 3,
    TargetTorque = 4,
    Statusword = 5,
    ModeDisplay = 6,
    ErrorCode = 7,
    ActualPosition = 8,
    ActualVelocity = 9,
    ActualTorque = 10,
    FollowingError = 11,
}

impl Cia402PdoField {
    pub const fn object_index(self) -> u16 {
        match self {
            Self::Controlword => CONTROLWORD_INDEX,
            Self::ModeOfOperation => MODE_OF_OPERATION_INDEX,
            Self::TargetPosition => TARGET_POSITION_INDEX,
            Self::TargetVelocity => TARGET_VELOCITY_INDEX,
            Self::TargetTorque => TARGET_TORQUE_INDEX,
            Self::Statusword => STATUSWORD_INDEX,
            Self::ModeDisplay => MODE_DISPLAY_INDEX,
            Self::ErrorCode => ERROR_CODE_INDEX,
            Self::ActualPosition => ACTUAL_POSITION_INDEX,
            Self::ActualVelocity => ACTUAL_VELOCITY_INDEX,
            Self::ActualTorque => ACTUAL_TORQUE_INDEX,
            Self::FollowingError => FOLLOWING_ERROR_INDEX,
        }
    }

    pub const fn direction(self) -> PdoDirection {
        match self {
            Self::Controlword
            | Self::ModeOfOperation
            | Self::TargetPosition
            | Self::TargetVelocity
            | Self::TargetTorque => PdoDirection::Rx,
            Self::Statusword
            | Self::ModeDisplay
            | Self::ErrorCode
            | Self::ActualPosition
            | Self::ActualVelocity
            | Self::ActualTorque
            | Self::FollowingError => PdoDirection::Tx,
        }
    }
}

const COMMON_FIELDS: [Cia402PdoField; 5] = [
    Cia402PdoField::Controlword,
    Cia402PdoField::ModeOfOperation,
    Cia402PdoField::Statusword,
    Cia402PdoField::ModeDisplay,
    Cia402PdoField::ErrorCode,
];

const ALL_FIELDS: [Cia402PdoField; FIELD_COUNT] = [
    Cia402PdoField::Controlword,
    Cia402PdoField::ModeOfOperation,
    Cia402PdoField::TargetPosition,
    Cia402PdoField::TargetVelocity,
    Cia402PdoField::TargetTorque,
    Cia402PdoField::Statusword,
    Cia402PdoField::ModeDisplay,
    Cia402PdoField::ErrorCode,
    Cia402PdoField::ActualPosition,
    Cia402PdoField::ActualVelocity,
    Cia402PdoField::ActualTorque,
    Cia402PdoField::FollowingError,
];

#[derive(Clone, Copy)]
struct EntrySpec {
    index: u16,
    direction: PdoDirection,
    bit_length: u8,
    signed: bool,
}

const fn entry_spec(field: Cia402PdoField) -> EntrySpec {
    match field {
        Cia402PdoField::Controlword => EntrySpec {
            index: CONTROLWORD_INDEX,
            direction: PdoDirection::Rx,
            bit_length: 16,
            signed: false,
        },
        Cia402PdoField::ModeOfOperation => EntrySpec {
            index: MODE_OF_OPERATION_INDEX,
            direction: PdoDirection::Rx,
            bit_length: 8,
            signed: true,
        },
        Cia402PdoField::TargetPosition => EntrySpec {
            index: TARGET_POSITION_INDEX,
            direction: PdoDirection::Rx,
            bit_length: 32,
            signed: true,
        },
        Cia402PdoField::TargetVelocity => EntrySpec {
            index: TARGET_VELOCITY_INDEX,
            direction: PdoDirection::Rx,
            bit_length: 32,
            signed: true,
        },
        Cia402PdoField::TargetTorque => EntrySpec {
            index: TARGET_TORQUE_INDEX,
            direction: PdoDirection::Rx,
            bit_length: 16,
            signed: true,
        },
        Cia402PdoField::Statusword => EntrySpec {
            index: STATUSWORD_INDEX,
            direction: PdoDirection::Tx,
            bit_length: 16,
            signed: false,
        },
        Cia402PdoField::ModeDisplay => EntrySpec {
            index: MODE_DISPLAY_INDEX,
            direction: PdoDirection::Tx,
            bit_length: 8,
            signed: true,
        },
        Cia402PdoField::ErrorCode => EntrySpec {
            index: ERROR_CODE_INDEX,
            direction: PdoDirection::Tx,
            bit_length: 16,
            signed: false,
        },
        Cia402PdoField::ActualPosition => EntrySpec {
            index: ACTUAL_POSITION_INDEX,
            direction: PdoDirection::Tx,
            bit_length: 32,
            signed: true,
        },
        Cia402PdoField::ActualVelocity => EntrySpec {
            index: ACTUAL_VELOCITY_INDEX,
            direction: PdoDirection::Tx,
            bit_length: 32,
            signed: true,
        },
        Cia402PdoField::ActualTorque => EntrySpec {
            index: ACTUAL_TORQUE_INDEX,
            direction: PdoDirection::Tx,
            bit_length: 16,
            signed: true,
        },
        Cia402PdoField::FollowingError => EntrySpec {
            index: FOLLOWING_ERROR_INDEX,
            direction: PdoDirection::Tx,
            bit_length: 32,
            signed: true,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cia402PdoError {
    InvalidMode,
    MissingField(Cia402PdoField),
    DuplicateField(Cia402PdoField),
    InvalidEntry(Cia402PdoField),
    Pdo(PdoError),
    MotionNotAllowed,
    ControlwordNotOperationEnabled,
    TargetModeMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cia402MotionGate {
    pub lifecycle_permit: bool,
    pub mode_confirmed: bool,
    pub operation_enabled: bool,
    pub setpoint_valid: bool,
}

impl Cia402MotionGate {
    pub const fn allows_motion(self) -> bool {
        self.lifecycle_permit
            && self.mode_confirmed
            && self.operation_enabled
            && self.setpoint_valid
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cia402Target {
    Position(i32),
    Velocity(i32),
    Torque(i16),
}

impl Cia402Target {
    pub const fn mode(self) -> OperatingMode {
        match self {
            Self::Position(_) => OperatingMode::Csp,
            Self::Velocity(_) => OperatingMode::Csv,
            Self::Torque(_) => OperatingMode::Cst,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cia402PdoInputs {
    pub statusword: u16,
    pub actual_mode: OperatingMode,
    pub error_code: u16,
    pub actual_position: Option<i32>,
    pub actual_velocity: Option<i32>,
    pub actual_torque: Option<i16>,
    pub following_error: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cia402PdoCommand {
    pub controlword: u16,
    pub mode: OperatingMode,
    pub target: Cia402Target,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cia402PdoMap {
    entries: [Option<PdoEntry>; FIELD_COUNT],
}

impl Cia402PdoMap {
    pub const fn new() -> Self {
        Self {
            entries: [None; FIELD_COUNT],
        }
    }

    /// Build a profile map from the frozen entries emitted by SII/ESI
    /// configuration. Unknown vendor or diagnostic objects are left for
    /// their owning profile; standard CiA 402 objects are copied by value.
    /// Entries must already use the unified Domain image offsets.
    pub fn from_pdo_entries(entries: &[PdoEntry]) -> Result<Self, Cia402PdoError> {
        let mut map = Self::new();
        for entry in entries {
            let Some(field) = ALL_FIELDS.iter().copied().find(|field| {
                entry.index == field.object_index()
                    && entry.subindex == 0
                    && entry.direction == field.direction()
            }) else {
                continue;
            };
            if map.entry(field).is_some() {
                return Err(Cia402PdoError::DuplicateField(field));
            }
            map.set_entry(field, *entry);
        }
        map.validate_present_entries()?;
        Ok(map)
    }

    pub const fn with_entry(mut self, field: Cia402PdoField, entry: PdoEntry) -> Self {
        self.entries[field as usize] = Some(entry);
        self
    }

    pub fn set_entry(&mut self, field: Cia402PdoField, entry: PdoEntry) {
        self.entries[field as usize] = Some(entry);
    }

    pub fn clear_entry(&mut self, field: Cia402PdoField) {
        self.entries[field as usize] = None;
    }

    pub const fn entry(&self, field: Cia402PdoField) -> Option<PdoEntry> {
        self.entries[field as usize]
    }

    pub fn validate_for(&self, mode: OperatingMode) -> Result<(), Cia402PdoError> {
        if !mode.is_cyclic() {
            return Err(Cia402PdoError::InvalidMode);
        }
        self.validate_common()?;
        self.require(target_field(mode))?;
        self.require(actual_field(mode))?;
        Ok(())
    }

    pub fn validate_all_modes(&self) -> Result<(), Cia402PdoError> {
        self.validate_for(OperatingMode::Csp)?;
        self.validate_for(OperatingMode::Csv)?;
        self.validate_for(OperatingMode::Cst)
    }

    pub fn read_inputs(&self, image: &[u8]) -> Result<Cia402PdoInputs, Cia402PdoError> {
        self.validate_common()?;
        self.preflight_input(image)?;
        Ok(Cia402PdoInputs {
            statusword: self.read_unsigned(Cia402PdoField::Statusword, image)? as u16,
            actual_mode: OperatingMode::from_raw(
                self.read_signed(Cia402PdoField::ModeDisplay, image)? as i8,
            ),
            error_code: self.read_unsigned(Cia402PdoField::ErrorCode, image)? as u16,
            actual_position: self.read_optional_i32(Cia402PdoField::ActualPosition, image)?,
            actual_velocity: self.read_optional_i32(Cia402PdoField::ActualVelocity, image)?,
            actual_torque: self.read_optional_i16(Cia402PdoField::ActualTorque, image)?,
            following_error: self.read_optional_i32(Cia402PdoField::FollowingError, image)?,
        })
    }

    pub fn read_inputs_for(
        &self,
        image: &[u8],
        mode: OperatingMode,
    ) -> Result<Cia402PdoInputs, Cia402PdoError> {
        self.validate_for(mode)?;
        self.read_inputs(image)
    }

    /// Write only the mode and Controlword fields used during PDS/mode
    /// handshaking. It deliberately does not write a cyclic target.
    pub fn write_control(
        &self,
        image: &mut [u8],
        mode: OperatingMode,
        controlword: u16,
    ) -> Result<(), Cia402PdoError> {
        if !mode.is_cyclic() {
            return Err(Cia402PdoError::InvalidMode);
        }
        self.validate_common()?;
        self.preflight_output(
            image,
            &[Cia402PdoField::Controlword, Cia402PdoField::ModeOfOperation],
        )?;
        self.write_unsigned(Cia402PdoField::Controlword, image, controlword as u64)?;
        self.write_signed(Cia402PdoField::ModeOfOperation, image, mode.raw() as i64)
    }

    /// Write an active cyclic command only after every motion gate and the
    /// Operation Enabled Controlword condition have been established.
    pub fn write_cyclic(
        &self,
        image: &mut [u8],
        command: Cia402PdoCommand,
        gate: Cia402MotionGate,
    ) -> Result<(), Cia402PdoError> {
        if !command.mode.is_cyclic() {
            return Err(Cia402PdoError::InvalidMode);
        }
        if !gate.allows_motion() {
            return Err(Cia402PdoError::MotionNotAllowed);
        }
        if command.target.mode() != command.mode {
            return Err(Cia402PdoError::TargetModeMismatch);
        }
        if command.controlword & 0x000F != CONTROLWORD_ENABLE_OPERATION {
            return Err(Cia402PdoError::ControlwordNotOperationEnabled);
        }
        self.validate_for(command.mode)?;
        let target_field = target_field(command.mode);
        self.preflight_output(
            image,
            &[
                Cia402PdoField::Controlword,
                Cia402PdoField::ModeOfOperation,
                target_field,
            ],
        )?;
        self.write_unsigned(
            Cia402PdoField::Controlword,
            image,
            command.controlword as u64,
        )?;
        self.write_signed(
            Cia402PdoField::ModeOfOperation,
            image,
            command.mode.raw() as i64,
        )?;
        match command.target {
            Cia402Target::Position(value) | Cia402Target::Velocity(value) => {
                self.write_signed(target_field, image, value as i64)?;
            }
            Cia402Target::Torque(value) => {
                self.write_signed(target_field, image, value as i64)?;
            }
        }
        Ok(())
    }

    fn validate_common(&self) -> Result<(), Cia402PdoError> {
        self.validate_present_entries()?;
        for field in COMMON_FIELDS {
            self.require(field)?;
        }
        self.validate_layout()
    }

    fn validate_present_entries(&self) -> Result<(), Cia402PdoError> {
        for field in ALL_FIELDS {
            if let Some(entry) = self.entry(field) {
                let spec = entry_spec(field);
                if entry.index != spec.index
                    || entry.subindex != 0
                    || entry.direction != spec.direction
                    || entry.bit_length != spec.bit_length
                    || entry.signed != spec.signed
                {
                    return Err(Cia402PdoError::InvalidEntry(field));
                }
            }
        }
        Ok(())
    }

    fn validate_layout(&self) -> Result<(), Cia402PdoError> {
        let mut layout = PdoLayout::<FIELD_COUNT>::new();
        for field in ALL_FIELDS {
            if let Some(entry) = self.entry(field) {
                layout.add(entry).map_err(Cia402PdoError::Pdo)?;
            }
        }
        Ok(())
    }

    fn require(&self, field: Cia402PdoField) -> Result<PdoEntry, Cia402PdoError> {
        self.entry(field).ok_or(Cia402PdoError::MissingField(field))
    }

    fn preflight_input(&self, image: &[u8]) -> Result<(), Cia402PdoError> {
        for field in ALL_FIELDS {
            if let Some(entry) = self.entry(field) {
                if entry.direction == PdoDirection::Tx {
                    entry.read_unsigned(image).map_err(Cia402PdoError::Pdo)?;
                }
            }
        }
        Ok(())
    }

    fn preflight_output(
        &self,
        image: &[u8],
        fields: &[Cia402PdoField],
    ) -> Result<(), Cia402PdoError> {
        for field in fields {
            let entry = self.require(*field)?;
            entry.read_unsigned(image).map_err(Cia402PdoError::Pdo)?;
        }
        Ok(())
    }

    fn read_unsigned(&self, field: Cia402PdoField, image: &[u8]) -> Result<u64, Cia402PdoError> {
        self.require(field)?
            .read_unsigned(image)
            .map_err(Cia402PdoError::Pdo)
    }

    fn read_signed(&self, field: Cia402PdoField, image: &[u8]) -> Result<i64, Cia402PdoError> {
        self.require(field)?
            .read_signed(image)
            .map_err(Cia402PdoError::Pdo)
    }

    fn read_optional_i32(
        &self,
        field: Cia402PdoField,
        image: &[u8],
    ) -> Result<Option<i32>, Cia402PdoError> {
        self.entry(field)
            .map(|_| self.read_signed(field, image).map(|value| value as i32))
            .transpose()
    }

    fn read_optional_i16(
        &self,
        field: Cia402PdoField,
        image: &[u8],
    ) -> Result<Option<i16>, Cia402PdoError> {
        self.entry(field)
            .map(|_| self.read_signed(field, image).map(|value| value as i16))
            .transpose()
    }

    fn write_unsigned(
        &self,
        field: Cia402PdoField,
        image: &mut [u8],
        value: u64,
    ) -> Result<(), Cia402PdoError> {
        self.require(field)?
            .write_unsigned(image, value)
            .map_err(Cia402PdoError::Pdo)
    }

    fn write_signed(
        &self,
        field: Cia402PdoField,
        image: &mut [u8],
        value: i64,
    ) -> Result<(), Cia402PdoError> {
        self.require(field)?
            .write_signed(image, value)
            .map_err(Cia402PdoError::Pdo)
    }
}

impl Default for Cia402PdoMap {
    fn default() -> Self {
        Self::new()
    }
}

const fn target_field(mode: OperatingMode) -> Cia402PdoField {
    match mode {
        OperatingMode::Csp => Cia402PdoField::TargetPosition,
        OperatingMode::Csv => Cia402PdoField::TargetVelocity,
        OperatingMode::Cst => Cia402PdoField::TargetTorque,
        OperatingMode::Unknown => Cia402PdoField::TargetPosition,
    }
}

const fn actual_field(mode: OperatingMode) -> Cia402PdoField {
    match mode {
        OperatingMode::Csp => Cia402PdoField::ActualPosition,
        OperatingMode::Csv => Cia402PdoField::ActualVelocity,
        OperatingMode::Cst => Cia402PdoField::ActualTorque,
        OperatingMode::Unknown => Cia402PdoField::ActualPosition,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(field: Cia402PdoField, bit_offset: usize) -> PdoEntry {
        let spec = entry_spec(field);
        PdoEntry {
            index: spec.index,
            subindex: 0,
            bit_offset,
            bit_length: spec.bit_length,
            signed: spec.signed,
            direction: spec.direction,
        }
    }

    fn complete_map() -> Cia402PdoMap {
        let mut map = Cia402PdoMap::new();
        for (field, bit_offset) in [
            (Cia402PdoField::Controlword, 0),
            (Cia402PdoField::ModeOfOperation, 16),
            (Cia402PdoField::TargetPosition, 24),
            (Cia402PdoField::TargetVelocity, 56),
            (Cia402PdoField::TargetTorque, 88),
            (Cia402PdoField::Statusword, 0),
            (Cia402PdoField::ModeDisplay, 16),
            (Cia402PdoField::ErrorCode, 24),
            (Cia402PdoField::ActualPosition, 40),
            (Cia402PdoField::ActualVelocity, 72),
            (Cia402PdoField::ActualTorque, 104),
            (Cia402PdoField::FollowingError, 120),
        ] {
            map.set_entry(field, entry(field, bit_offset));
        }
        map
    }

    #[test]
    fn complete_map_validates_for_all_cyclic_modes() {
        assert_eq!(complete_map().validate_all_modes(), Ok(()));
    }

    #[test]
    fn missing_or_malformed_mode_fields_are_rejected() {
        let mut map = complete_map();
        map.clear_entry(Cia402PdoField::TargetVelocity);
        assert_eq!(
            map.validate_for(OperatingMode::Csv),
            Err(Cia402PdoError::MissingField(Cia402PdoField::TargetVelocity))
        );

        let mut malformed = complete_map();
        let mut wrong = malformed.entry(Cia402PdoField::Statusword).unwrap();
        wrong.direction = PdoDirection::Rx;
        malformed.set_entry(Cia402PdoField::Statusword, wrong);
        assert_eq!(
            malformed.validate_for(OperatingMode::Csp),
            Err(Cia402PdoError::InvalidEntry(Cia402PdoField::Statusword))
        );

        let mut wrong_width = complete_map();
        let mut wrong_statusword = wrong_width.entry(Cia402PdoField::Statusword).unwrap();
        wrong_statusword.bit_length = 8;
        wrong_width.set_entry(Cia402PdoField::Statusword, wrong_statusword);
        assert_eq!(
            wrong_width.validate_for(OperatingMode::Csp),
            Err(Cia402PdoError::InvalidEntry(Cia402PdoField::Statusword))
        );

        let mut wrong_index = complete_map();
        let mut wrong_statusword = wrong_index.entry(Cia402PdoField::Statusword).unwrap();
        wrong_statusword.index = STATUSWORD_INDEX.wrapping_add(1);
        wrong_index.set_entry(Cia402PdoField::Statusword, wrong_statusword);
        assert_eq!(
            wrong_index.validate_for(OperatingMode::Csp),
            Err(Cia402PdoError::InvalidEntry(Cia402PdoField::Statusword))
        );

        let mut wrong_signedness = complete_map();
        let mut wrong_statusword = wrong_signedness.entry(Cia402PdoField::Statusword).unwrap();
        wrong_statusword.signed = true;
        wrong_signedness.set_entry(Cia402PdoField::Statusword, wrong_statusword);
        assert_eq!(
            wrong_signedness.validate_for(OperatingMode::Csp),
            Err(Cia402PdoError::InvalidEntry(Cia402PdoField::Statusword))
        );

        let mut overlap = complete_map();
        let mut overlapping = overlap.entry(Cia402PdoField::TargetVelocity).unwrap();
        overlapping.bit_offset = 24;
        overlap.set_entry(Cia402PdoField::TargetVelocity, overlapping);
        assert_eq!(
            overlap.validate_for(OperatingMode::Csp),
            Err(Cia402PdoError::Pdo(PdoError::BitOverlap))
        );
    }

    #[test]
    fn input_decoder_reads_signed_values_and_unknown_mode_fail_closed() {
        let map = complete_map();
        let mut image = [0_u8; 20];
        map.entry(Cia402PdoField::Statusword)
            .unwrap()
            .write_unsigned(&mut image, 0x1234)
            .unwrap();
        map.entry(Cia402PdoField::ModeDisplay)
            .unwrap()
            .write_signed(&mut image, 8)
            .unwrap();
        map.entry(Cia402PdoField::ErrorCode)
            .unwrap()
            .write_unsigned(&mut image, 0xBEEF)
            .unwrap();
        map.entry(Cia402PdoField::ActualPosition)
            .unwrap()
            .write_signed(&mut image, -1234)
            .unwrap();
        map.entry(Cia402PdoField::ActualVelocity)
            .unwrap()
            .write_signed(&mut image, 5678)
            .unwrap();
        map.entry(Cia402PdoField::ActualTorque)
            .unwrap()
            .write_signed(&mut image, -42)
            .unwrap();
        map.entry(Cia402PdoField::FollowingError)
            .unwrap()
            .write_signed(&mut image, -7)
            .unwrap();

        let inputs = map.read_inputs(&image).unwrap();
        assert_eq!(inputs.statusword, 0x1234);
        assert_eq!(inputs.actual_mode, OperatingMode::Csp);
        assert_eq!(inputs.error_code, 0xBEEF);
        assert_eq!(inputs.actual_position, Some(-1234));
        assert_eq!(inputs.actual_velocity, Some(5678));
        assert_eq!(inputs.actual_torque, Some(-42));
        assert_eq!(inputs.following_error, Some(-7));

        map.entry(Cia402PdoField::ModeDisplay)
            .unwrap()
            .write_signed(&mut image, 127)
            .unwrap();
        assert_eq!(
            map.read_inputs(&image).unwrap().actual_mode,
            OperatingMode::Unknown
        );
    }

    #[test]
    fn cyclic_writer_selects_target_by_mode_and_preserves_neighbours() {
        let map = complete_map();
        let mut image = [0xA5_u8; 20];
        let before = image;
        map.write_control(&mut image, OperatingMode::Csp, 0x0007)
            .unwrap();
        let gate = Cia402MotionGate {
            lifecycle_permit: true,
            mode_confirmed: true,
            operation_enabled: true,
            setpoint_valid: true,
        };
        map.write_cyclic(
            &mut image,
            Cia402PdoCommand {
                controlword: CONTROLWORD_ENABLE_OPERATION,
                mode: OperatingMode::Csp,
                target: Cia402Target::Position(-100),
            },
            gate,
        )
        .unwrap();
        assert_eq!(
            map.entry(Cia402PdoField::TargetPosition)
                .unwrap()
                .read_signed(&image),
            Ok(-100)
        );
        assert_eq!(
            map.entry(Cia402PdoField::TargetVelocity)
                .unwrap()
                .read_signed(&image),
            Ok(0xA5A5_A5A5u32 as i32 as i64)
        );
        assert_eq!(image[17..20], before[17..20]);
    }

    #[test]
    fn denied_gate_and_short_image_leave_output_untouched() {
        let map = complete_map();
        let mut image = [0x5A_u8; 20];
        let before = image;
        let denied = Cia402MotionGate {
            lifecycle_permit: false,
            mode_confirmed: true,
            operation_enabled: true,
            setpoint_valid: true,
        };
        assert_eq!(
            map.write_cyclic(
                &mut image,
                Cia402PdoCommand {
                    controlword: CONTROLWORD_ENABLE_OPERATION,
                    mode: OperatingMode::Csv,
                    target: Cia402Target::Velocity(1),
                },
                denied,
            ),
            Err(Cia402PdoError::MotionNotAllowed)
        );
        assert_eq!(image, before);

        let mut short = [0_u8; 2];
        let before_short = short;
        assert_eq!(
            map.write_cyclic(
                &mut short,
                Cia402PdoCommand {
                    controlword: CONTROLWORD_ENABLE_OPERATION,
                    mode: OperatingMode::Cst,
                    target: Cia402Target::Torque(1),
                },
                Cia402MotionGate {
                    lifecycle_permit: true,
                    mode_confirmed: true,
                    operation_enabled: true,
                    setpoint_valid: true,
                },
            ),
            Err(Cia402PdoError::Pdo(PdoError::ImageBounds))
        );
        assert_eq!(short, before_short);
    }

    #[test]
    fn control_writer_is_permit_independent_and_atomic_on_short_image() {
        let map = complete_map();
        let mut image = [0xA5_u8; 20];
        map.write_control(&mut image, OperatingMode::Cst, 0x0006)
            .unwrap();
        assert_eq!(
            map.entry(Cia402PdoField::Controlword)
                .unwrap()
                .read_unsigned(&image),
            Ok(0x0006)
        );
        assert_eq!(
            map.entry(Cia402PdoField::ModeOfOperation)
                .unwrap()
                .read_signed(&image),
            Ok(OperatingMode::Cst.raw() as i64)
        );

        let mut short = [0x5A_u8; 2];
        let before = short;
        assert_eq!(
            map.write_control(&mut short, OperatingMode::Csv, 0x0007),
            Err(Cia402PdoError::Pdo(PdoError::ImageBounds))
        );
        assert_eq!(short, before);
    }

    #[test]
    fn cyclic_writer_requires_matching_mode_and_operation_enabled_controlword() {
        let map = complete_map();
        let mut image = [0_u8; 20];
        let gate = Cia402MotionGate {
            lifecycle_permit: true,
            mode_confirmed: true,
            operation_enabled: true,
            setpoint_valid: true,
        };
        assert_eq!(
            map.write_cyclic(
                &mut image,
                Cia402PdoCommand {
                    controlword: CONTROLWORD_ENABLE_OPERATION,
                    mode: OperatingMode::Csv,
                    target: Cia402Target::Position(1),
                },
                gate,
            ),
            Err(Cia402PdoError::TargetModeMismatch)
        );
        assert_eq!(
            map.write_cyclic(
                &mut image,
                Cia402PdoCommand {
                    controlword: 0x0007,
                    mode: OperatingMode::Csv,
                    target: Cia402Target::Velocity(1),
                },
                gate,
            ),
            Err(Cia402PdoError::ControlwordNotOperationEnabled)
        );
        assert_eq!(
            map.write_cyclic(
                &mut image,
                Cia402PdoCommand {
                    controlword: CONTROLWORD_ENABLE_OPERATION,
                    mode: OperatingMode::Unknown,
                    target: Cia402Target::Position(1),
                },
                gate,
            ),
            Err(Cia402PdoError::InvalidMode)
        );
    }
}
