//! Allocation-free CoE SDO Information codec and PDO metadata verifier.
//!
//! Mailbox transport, retries, working-counter checks and cycle budgets remain
//! owned by the caller. This module owns only service `0x08` payloads and the
//! deterministic comparison against a generated expectation plan.

use crate::coe::{CoeHeader, CoeService, SdoError};
use crate::mailbox::MAX_MAILBOX_BYTES;

pub const SDO_INFORMATION_HEADER_LEN: usize = 6;
pub const MAX_SDO_INFORMATION_FRAGMENTS: u8 = 64;

const SDO_INFO_INCOMPLETE: u8 = 0x80;
const SDO_INFO_OPCODE_MASK: u8 = 0x7f;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum CanopenDataType {
    Boolean = 0x0001,
    Integer8 = 0x0002,
    Integer16 = 0x0003,
    Integer32 = 0x0004,
    Unsigned8 = 0x0005,
    Unsigned16 = 0x0006,
    Unsigned32 = 0x0007,
    Real32 = 0x0008,
    VisibleString = 0x0009,
    OctetString = 0x000a,
    UnicodeString = 0x000b,
    TimeOfDay = 0x000c,
    TimeDifference = 0x000d,
    Domain = 0x000f,
    Integer24 = 0x0010,
    Real64 = 0x0011,
    Integer40 = 0x0012,
    Integer48 = 0x0013,
    Integer56 = 0x0014,
    Integer64 = 0x0015,
    Unsigned24 = 0x0016,
    Unsigned40 = 0x0018,
    Unsigned48 = 0x0019,
    Unsigned56 = 0x001a,
    Unsigned64 = 0x001b,
    Bit1 = 0x0030,
    Bit2 = 0x0031,
    Bit3 = 0x0032,
    Bit4 = 0x0033,
    Bit5 = 0x0034,
    Bit6 = 0x0035,
    Bit7 = 0x0036,
    Bit8 = 0x0037,
}

impl CanopenDataType {
    pub const fn from_raw(value: u16) -> Option<Self> {
        Some(match value {
            0x0001 => Self::Boolean,
            0x0002 => Self::Integer8,
            0x0003 => Self::Integer16,
            0x0004 => Self::Integer32,
            0x0005 => Self::Unsigned8,
            0x0006 => Self::Unsigned16,
            0x0007 => Self::Unsigned32,
            0x0008 => Self::Real32,
            0x0009 => Self::VisibleString,
            0x000a => Self::OctetString,
            0x000b => Self::UnicodeString,
            0x000c => Self::TimeOfDay,
            0x000d => Self::TimeDifference,
            0x000f => Self::Domain,
            0x0010 => Self::Integer24,
            0x0011 => Self::Real64,
            0x0012 => Self::Integer40,
            0x0013 => Self::Integer48,
            0x0014 => Self::Integer56,
            0x0015 => Self::Integer64,
            0x0016 => Self::Unsigned24,
            0x0018 => Self::Unsigned40,
            0x0019 => Self::Unsigned48,
            0x001a => Self::Unsigned56,
            0x001b => Self::Unsigned64,
            0x0030 => Self::Bit1,
            0x0031 => Self::Bit2,
            0x0032 => Self::Bit3,
            0x0033 => Self::Bit4,
            0x0034 => Self::Bit5,
            0x0035 => Self::Bit6,
            0x0036 => Self::Bit7,
            0x0037 => Self::Bit8,
            _ => return None,
        })
    }

    pub const fn raw(self) -> u16 {
        self as u16
    }

    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::Integer8
                | Self::Integer16
                | Self::Integer24
                | Self::Integer32
                | Self::Integer40
                | Self::Integer48
                | Self::Integer56
                | Self::Integer64
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SdoInfoOpcode {
    GetOdListRequest = 0x01,
    GetOdListResponse = 0x02,
    GetObjectDescriptionRequest = 0x03,
    GetObjectDescriptionResponse = 0x04,
    GetEntryDescriptionRequest = 0x05,
    GetEntryDescriptionResponse = 0x06,
    ErrorResponse = 0x07,
}

impl SdoInfoOpcode {
    const fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::GetOdListRequest,
            0x02 => Self::GetOdListResponse,
            0x03 => Self::GetObjectDescriptionRequest,
            0x04 => Self::GetObjectDescriptionResponse,
            0x05 => Self::GetEntryDescriptionRequest,
            0x06 => Self::GetEntryDescriptionResponse,
            0x07 => Self::ErrorResponse,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum SdoInfoOdListType {
    Lengths = 0x0000,
    All = 0x0001,
    RxPdoMappable = 0x0002,
    TxPdoMappable = 0x0003,
    Backup = 0x0004,
    Settings = 0x0005,
}

impl SdoInfoOdListType {
    const fn from_raw(value: u16) -> Option<Self> {
        Some(match value {
            0x0000 => Self::Lengths,
            0x0001 => Self::All,
            0x0002 => Self::RxPdoMappable,
            0x0003 => Self::TxPdoMappable,
            0x0004 => Self::Backup,
            0x0005 => Self::Settings,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SdoInfoObjectCode {
    Domain = 0x02,
    DefType = 0x05,
    DefStruct = 0x06,
    Variable = 0x07,
    Array = 0x08,
    Record = 0x09,
}

impl SdoInfoObjectCode {
    pub const fn from_raw(value: u8) -> Option<Self> {
        Some(match value {
            0x02 => Self::Domain,
            0x05 => Self::DefType,
            0x06 => Self::DefStruct,
            0x07 => Self::Variable,
            0x08 => Self::Array,
            0x09 => Self::Record,
            _ => return None,
        })
    }

    pub const fn raw(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdoInfoValueInfo(u8);

impl SdoInfoValueInfo {
    pub const ACCESS: Self = Self(0x01);

    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdoInfoObjectAccess(u16);

impl SdoInfoObjectAccess {
    pub const READ_PREOP: u16 = 0x0001;
    pub const READ_SAFEOP: u16 = 0x0002;
    pub const READ_OP: u16 = 0x0004;
    pub const WRITE_PREOP: u16 = 0x0008;
    pub const WRITE_SAFEOP: u16 = 0x0010;
    pub const WRITE_OP: u16 = 0x0020;
    pub const RX_PDO_MAPPABLE: u16 = 0x0040;
    pub const TX_PDO_MAPPABLE: u16 = 0x0080;
    pub const BACKUP: u16 = 0x0100;
    pub const SETTINGS: u16 = 0x0200;

    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }

    pub const fn readable(self) -> bool {
        self.0 & (Self::READ_PREOP | Self::READ_SAFEOP | Self::READ_OP) != 0
    }

    pub const fn writable(self) -> bool {
        self.0 & (Self::WRITE_PREOP | Self::WRITE_SAFEOP | Self::WRITE_OP) != 0
    }

    pub const fn rx_pdo_mappable(self) -> bool {
        self.0 & Self::RX_PDO_MAPPABLE != 0
    }

    pub const fn tx_pdo_mappable(self) -> bool {
        self.0 & Self::TX_PDO_MAPPABLE != 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdoInformationPolicy {
    enabled: bool,
}

impl SdoInformationPolicy {
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub const fn enabled(self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdoInfoObjectDescription {
    pub index: u16,
    pub data_type_raw: u16,
    pub data_type: Option<CanopenDataType>,
    pub max_subindex: u8,
    pub object_code_raw: u8,
    pub object_code: Option<SdoInfoObjectCode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdoInfoEntryDescription {
    pub index: u16,
    pub subindex: u8,
    pub value_info: SdoInfoValueInfo,
    pub data_type_raw: u16,
    pub data_type: Option<CanopenDataType>,
    pub bit_length: u16,
    pub access: SdoInfoObjectAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoInformationPhase {
    Idle,
    AwaitResponse,
    AwaitFragment,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoInformationProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoInformationResult {
    ObjectList,
    ObjectDescription,
    EntryDescription,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoInformationError {
    Busy,
    Disabled,
    InvalidState,
    InvalidObjectIndex,
    EmptyValueInfo,
    BufferTooSmall,
    PayloadTooLarge,
    Truncated,
    ReservedByte(u8),
    Coe(SdoError),
    WrongService,
    UnknownOpcode(u8),
    UnexpectedOpcode {
        expected: SdoInfoOpcode,
        actual: SdoInfoOpcode,
    },
    ListTypeMismatch {
        expected: SdoInfoOdListType,
        actual: u16,
    },
    IndexMismatch {
        expected: u16,
        actual: u16,
    },
    SubindexMismatch {
        expected: u8,
        actual: u8,
    },
    ValueInfoMismatch {
        expected: SdoInfoValueInfo,
        actual: SdoInfoValueInfo,
    },
    MalformedList,
    DuplicateObjectIndex(u16),
    ObjectCapacity,
    FragmentSequence {
        expected: u16,
        actual: u16,
    },
    FragmentState,
    TooManyFragments,
    Abort(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SdoInformationRequest {
    None,
    ObjectList(SdoInfoOdListType),
    ObjectDescription(u16),
    EntryDescription {
        index: u16,
        subindex: u8,
        value_info: SdoInfoValueInfo,
    },
}

pub struct SdoInformationTransfer<const MAX_OBJECTS: usize> {
    policy: SdoInformationPolicy,
    phase: SdoInformationPhase,
    request: SdoInformationRequest,
    pending: [u8; MAX_MAILBOX_BYTES],
    pending_len: usize,
    fragments_seen: u8,
    next_fragments_left: Option<u16>,
    object_indices: [u16; MAX_OBJECTS],
    object_count: usize,
    staged_object: Option<SdoInfoObjectDescription>,
    staged_entry: Option<SdoInfoEntryDescription>,
    result: Option<SdoInformationResult>,
    last_error: Option<SdoInformationError>,
}

impl<const MAX_OBJECTS: usize> SdoInformationTransfer<MAX_OBJECTS> {
    pub const fn new() -> Self {
        Self::with_policy(SdoInformationPolicy::new(false))
    }

    pub const fn with_policy(policy: SdoInformationPolicy) -> Self {
        Self {
            policy,
            phase: SdoInformationPhase::Idle,
            request: SdoInformationRequest::None,
            pending: [0; MAX_MAILBOX_BYTES],
            pending_len: 0,
            fragments_seen: 0,
            next_fragments_left: None,
            object_indices: [0; MAX_OBJECTS],
            object_count: 0,
            staged_object: None,
            staged_entry: None,
            result: None,
            last_error: None,
        }
    }

    pub const fn policy(&self) -> SdoInformationPolicy {
        self.policy
    }

    pub const fn phase(&self) -> SdoInformationPhase {
        self.phase
    }

    pub const fn result(&self) -> Option<SdoInformationResult> {
        if matches!(self.phase, SdoInformationPhase::Complete) {
            self.result
        } else {
            None
        }
    }

    pub const fn last_error(&self) -> Option<SdoInformationError> {
        self.last_error
    }

    pub fn pending_payload(&self) -> &[u8] {
        &self.pending[..self.pending_len]
    }

    pub fn object_indices(&self) -> Option<&[u16]> {
        if self.result() == Some(SdoInformationResult::ObjectList) {
            Some(&self.object_indices[..self.object_count])
        } else {
            None
        }
    }

    pub fn object_description(&self) -> Option<SdoInfoObjectDescription> {
        if self.result() == Some(SdoInformationResult::ObjectDescription) {
            self.staged_object
        } else {
            None
        }
    }

    pub fn entry_description(&self) -> Option<SdoInfoEntryDescription> {
        if self.result() == Some(SdoInformationResult::EntryDescription) {
            self.staged_entry
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.phase = SdoInformationPhase::Idle;
        self.request = SdoInformationRequest::None;
        self.clear_staging();
    }

    pub fn start_object_list(
        &mut self,
        list_type: SdoInfoOdListType,
    ) -> Result<(), SdoInformationError> {
        self.begin(SdoInformationRequest::ObjectList(list_type), 8)?;
        self.pending[6..8].copy_from_slice(&(list_type as u16).to_le_bytes());
        Ok(())
    }

    pub fn start_object_description(&mut self, index: u16) -> Result<(), SdoInformationError> {
        if index == 0 {
            return Err(SdoInformationError::InvalidObjectIndex);
        }
        self.begin(SdoInformationRequest::ObjectDescription(index), 8)?;
        self.pending[6..8].copy_from_slice(&index.to_le_bytes());
        Ok(())
    }

    pub fn start_entry_description(
        &mut self,
        index: u16,
        subindex: u8,
        value_info: SdoInfoValueInfo,
    ) -> Result<(), SdoInformationError> {
        if index == 0 {
            return Err(SdoInformationError::InvalidObjectIndex);
        }
        if value_info.raw() == 0 {
            return Err(SdoInformationError::EmptyValueInfo);
        }
        self.begin(
            SdoInformationRequest::EntryDescription {
                index,
                subindex,
                value_info,
            },
            10,
        )?;
        self.pending[6..8].copy_from_slice(&index.to_le_bytes());
        self.pending[8] = subindex;
        self.pending[9] = value_info.raw();
        Ok(())
    }

    pub fn accept_response(
        &mut self,
        payload: &[u8],
    ) -> Result<SdoInformationProgress, SdoInformationError> {
        if !matches!(
            self.phase,
            SdoInformationPhase::AwaitResponse | SdoInformationPhase::AwaitFragment
        ) {
            return Err(SdoInformationError::InvalidState);
        }
        if payload.len() > MAX_MAILBOX_BYTES {
            return self.fail(SdoInformationError::PayloadTooLarge);
        }
        if payload.len() < SDO_INFORMATION_HEADER_LEN {
            return self.fail(SdoInformationError::Truncated);
        }
        let header = match CoeHeader::decode(payload) {
            Ok(header) => header,
            Err(error) => return self.fail(SdoInformationError::Coe(error)),
        };
        if header.service != CoeService::SdoInformation {
            return self.fail(SdoInformationError::WrongService);
        }
        if payload[3] != 0 {
            return self.fail(SdoInformationError::ReservedByte(payload[3]));
        }

        let incomplete = payload[2] & SDO_INFO_INCOMPLETE != 0;
        let raw_opcode = payload[2] & SDO_INFO_OPCODE_MASK;
        let opcode = match SdoInfoOpcode::from_raw(raw_opcode) {
            Some(opcode) => opcode,
            None => return self.fail(SdoInformationError::UnknownOpcode(raw_opcode)),
        };
        if opcode == SdoInfoOpcode::ErrorResponse {
            if payload.len() < 10 {
                return self.fail(SdoInformationError::Truncated);
            }
            let code = u32::from_le_bytes([payload[6], payload[7], payload[8], payload[9]]);
            return self.fail(SdoInformationError::Abort(code));
        }

        let expected_opcode = self.expected_response_opcode();
        if opcode != expected_opcode {
            return self.fail(SdoInformationError::UnexpectedOpcode {
                expected: expected_opcode,
                actual: opcode,
            });
        }

        let fragments_left = u16::from_le_bytes([payload[4], payload[5]]);
        let first = self.fragments_seen == 0;
        self.validate_fragment_sequence(first, incomplete, fragments_left)?;
        self.fragments_seen = self.fragments_seen.saturating_add(1);
        self.pending_len = 0;

        if first {
            self.parse_first_fragment(payload)?;
        } else if matches!(self.request, SdoInformationRequest::ObjectList(_)) {
            self.append_object_indices(&payload[SDO_INFORMATION_HEADER_LEN..])?;
        }

        let more = incomplete || fragments_left != 0;
        if more {
            self.phase = SdoInformationPhase::AwaitFragment;
            self.next_fragments_left = if fragments_left == 0 {
                None
            } else {
                Some(fragments_left - 1)
            };
            return Ok(SdoInformationProgress::Advanced);
        }

        self.next_fragments_left = None;
        self.result = Some(match self.request {
            SdoInformationRequest::ObjectList(_) => SdoInformationResult::ObjectList,
            SdoInformationRequest::ObjectDescription(_) => {
                if self.staged_object.is_none() {
                    return self.fail(SdoInformationError::Truncated);
                }
                SdoInformationResult::ObjectDescription
            }
            SdoInformationRequest::EntryDescription { .. } => {
                if self.staged_entry.is_none() {
                    return self.fail(SdoInformationError::Truncated);
                }
                SdoInformationResult::EntryDescription
            }
            SdoInformationRequest::None => {
                return self.fail(SdoInformationError::InvalidState);
            }
        });
        self.phase = SdoInformationPhase::Complete;
        Ok(SdoInformationProgress::Complete)
    }

    fn begin(
        &mut self,
        request: SdoInformationRequest,
        request_len: usize,
    ) -> Result<(), SdoInformationError> {
        if !self.policy.enabled() {
            return Err(SdoInformationError::Disabled);
        }
        if matches!(
            self.phase,
            SdoInformationPhase::AwaitResponse | SdoInformationPhase::AwaitFragment
        ) {
            return Err(SdoInformationError::Busy);
        }
        if request_len > self.pending.len() {
            return Err(SdoInformationError::BufferTooSmall);
        }

        self.clear_staging();
        self.request = request;
        self.pending_len = request_len;
        CoeHeader {
            number: 0,
            service: CoeService::SdoInformation,
        }
        .encode(&mut self.pending[..request_len])
        .map_err(SdoInformationError::Coe)?;
        self.pending[2] = match request {
            SdoInformationRequest::ObjectList(_) => SdoInfoOpcode::GetOdListRequest as u8,
            SdoInformationRequest::ObjectDescription(_) => {
                SdoInfoOpcode::GetObjectDescriptionRequest as u8
            }
            SdoInformationRequest::EntryDescription { .. } => {
                SdoInfoOpcode::GetEntryDescriptionRequest as u8
            }
            SdoInformationRequest::None => return Err(SdoInformationError::InvalidState),
        };
        self.pending[3] = 0;
        self.pending[4] = 0;
        self.pending[5] = 0;
        self.phase = SdoInformationPhase::AwaitResponse;
        Ok(())
    }

    fn clear_staging(&mut self) {
        self.pending_len = 0;
        self.fragments_seen = 0;
        self.next_fragments_left = None;
        self.object_count = 0;
        self.staged_object = None;
        self.staged_entry = None;
        self.result = None;
        self.last_error = None;
    }

    fn expected_response_opcode(&self) -> SdoInfoOpcode {
        match self.request {
            SdoInformationRequest::ObjectList(_) => SdoInfoOpcode::GetOdListResponse,
            SdoInformationRequest::ObjectDescription(_) => {
                SdoInfoOpcode::GetObjectDescriptionResponse
            }
            SdoInformationRequest::EntryDescription { .. } => {
                SdoInfoOpcode::GetEntryDescriptionResponse
            }
            SdoInformationRequest::None => SdoInfoOpcode::ErrorResponse,
        }
    }

    fn validate_fragment_sequence(
        &mut self,
        first: bool,
        incomplete: bool,
        fragments_left: u16,
    ) -> Result<(), SdoInformationError> {
        if self.fragments_seen >= MAX_SDO_INFORMATION_FRAGMENTS {
            return self.fail(SdoInformationError::TooManyFragments);
        }
        if fragments_left != 0 && !incomplete {
            return self.fail(SdoInformationError::FragmentState);
        }
        if !first {
            if let Some(expected) = self.next_fragments_left {
                if expected != fragments_left {
                    return self.fail(SdoInformationError::FragmentSequence {
                        expected,
                        actual: fragments_left,
                    });
                }
                if expected == 0 && incomplete {
                    return self.fail(SdoInformationError::FragmentState);
                }
            }
        }
        Ok(())
    }

    fn parse_first_fragment(&mut self, payload: &[u8]) -> Result<(), SdoInformationError> {
        match self.request {
            SdoInformationRequest::ObjectList(expected_list_type) => {
                if payload.len() < 8 {
                    return self.fail(SdoInformationError::Truncated);
                }
                let actual = u16::from_le_bytes([payload[6], payload[7]]);
                if SdoInfoOdListType::from_raw(actual) != Some(expected_list_type) {
                    return self.fail(SdoInformationError::ListTypeMismatch {
                        expected: expected_list_type,
                        actual,
                    });
                }
                self.append_object_indices(&payload[8..])
            }
            SdoInformationRequest::ObjectDescription(expected_index) => {
                if payload.len() < 12 {
                    return self.fail(SdoInformationError::Truncated);
                }
                let actual_index = u16::from_le_bytes([payload[6], payload[7]]);
                if actual_index != expected_index {
                    return self.fail(SdoInformationError::IndexMismatch {
                        expected: expected_index,
                        actual: actual_index,
                    });
                }
                let data_type_raw = u16::from_le_bytes([payload[8], payload[9]]);
                let object_code_raw = payload[11];
                self.staged_object = Some(SdoInfoObjectDescription {
                    index: actual_index,
                    data_type_raw,
                    data_type: CanopenDataType::from_raw(data_type_raw),
                    max_subindex: payload[10],
                    object_code_raw,
                    object_code: SdoInfoObjectCode::from_raw(object_code_raw),
                });
                Ok(())
            }
            SdoInformationRequest::EntryDescription {
                index: expected_index,
                subindex: expected_subindex,
                value_info: expected_value_info,
            } => {
                if payload.len() < 16 {
                    return self.fail(SdoInformationError::Truncated);
                }
                let actual_index = u16::from_le_bytes([payload[6], payload[7]]);
                if actual_index != expected_index {
                    return self.fail(SdoInformationError::IndexMismatch {
                        expected: expected_index,
                        actual: actual_index,
                    });
                }
                let actual_subindex = payload[8];
                if actual_subindex != expected_subindex {
                    return self.fail(SdoInformationError::SubindexMismatch {
                        expected: expected_subindex,
                        actual: actual_subindex,
                    });
                }
                let value_info = SdoInfoValueInfo::from_raw(payload[9]);
                if !value_info.contains(expected_value_info) {
                    return self.fail(SdoInformationError::ValueInfoMismatch {
                        expected: expected_value_info,
                        actual: value_info,
                    });
                }
                let data_type_raw = u16::from_le_bytes([payload[10], payload[11]]);
                self.staged_entry = Some(SdoInfoEntryDescription {
                    index: actual_index,
                    subindex: actual_subindex,
                    value_info,
                    data_type_raw,
                    data_type: CanopenDataType::from_raw(data_type_raw),
                    bit_length: u16::from_le_bytes([payload[12], payload[13]]),
                    access: SdoInfoObjectAccess::from_raw(u16::from_le_bytes([
                        payload[14],
                        payload[15],
                    ])),
                });
                Ok(())
            }
            SdoInformationRequest::None => self.fail(SdoInformationError::InvalidState),
        }
    }

    fn append_object_indices(&mut self, bytes: &[u8]) -> Result<(), SdoInformationError> {
        if bytes.len() % 2 != 0 {
            return self.fail(SdoInformationError::MalformedList);
        }
        for chunk in bytes.chunks_exact(2) {
            let index = u16::from_le_bytes([chunk[0], chunk[1]]);
            if index == 0 {
                return self.fail(SdoInformationError::InvalidObjectIndex);
            }
            if self.object_indices[..self.object_count].contains(&index) {
                return self.fail(SdoInformationError::DuplicateObjectIndex(index));
            }
            if self.object_count == MAX_OBJECTS {
                return self.fail(SdoInformationError::ObjectCapacity);
            }
            self.object_indices[self.object_count] = index;
            self.object_count += 1;
        }
        Ok(())
    }

    fn fail<T>(&mut self, error: SdoInformationError) -> Result<T, SdoInformationError> {
        self.phase = SdoInformationPhase::Faulted;
        self.pending_len = 0;
        self.result = None;
        self.last_error = Some(error);
        Err(error)
    }
}

impl<const MAX_OBJECTS: usize> Default for SdoInformationTransfer<MAX_OBJECTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdoInformationRequiredAccess(u8);

impl SdoInformationRequiredAccess {
    pub const READ: u8 = 0x01;
    pub const WRITE: u8 = 0x02;
    pub const RX_PDO: u8 = 0x04;
    pub const TX_PDO: u8 = 0x08;
    const KNOWN: u8 = Self::READ | Self::WRITE | Self::RX_PDO | Self::TX_PDO;

    pub const RX_PDO_ENTRY: Self = Self(Self::WRITE | Self::RX_PDO);
    pub const TX_PDO_ENTRY: Self = Self(Self::READ | Self::TX_PDO);

    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn valid(self) -> bool {
        self.0 != 0 && self.0 & !Self::KNOWN == 0
    }

    const fn requires(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdoInformationExpectation {
    pub slave_position: u16,
    pub index: u16,
    pub subindex: u8,
    pub data_type: CanopenDataType,
    pub bit_length: u16,
    pub required_access: SdoInformationRequiredAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoInformationVerifierPhase {
    Idle,
    AwaitObject,
    AwaitEntry,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoInformationVerifierProgress {
    AwaitingFragment,
    RequestReady,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoInformationVerifierError {
    Protocol(SdoInformationError),
    EmptyPlan,
    InvalidExpectation(usize),
    WrongSlavePosition {
        expectation: usize,
        expected: u16,
        actual: u16,
    },
    DuplicateOrUnorderedExpectation(usize),
    ObjectMissingSubindex {
        index: u16,
        expected_subindex: u8,
        max_subindex: u8,
    },
    DataTypeMismatch {
        index: u16,
        subindex: u8,
        expected: CanopenDataType,
        actual: u16,
    },
    BitLengthMismatch {
        index: u16,
        subindex: u8,
        expected: u16,
        actual: u16,
    },
    ReadAccessMissing {
        index: u16,
        subindex: u8,
    },
    WriteAccessMissing {
        index: u16,
        subindex: u8,
    },
    RxPdoMappingMissing {
        index: u16,
        subindex: u8,
    },
    TxPdoMappingMissing {
        index: u16,
        subindex: u8,
    },
}

pub struct SdoInformationVerifier<'a, const MAX_OBJECTS: usize> {
    transfer: SdoInformationTransfer<MAX_OBJECTS>,
    expectations: &'a [SdoInformationExpectation],
    phase: SdoInformationVerifierPhase,
    current: usize,
    verified_count: usize,
    last_error: Option<SdoInformationVerifierError>,
}

impl<'a, const MAX_OBJECTS: usize> SdoInformationVerifier<'a, MAX_OBJECTS> {
    pub const fn new(policy: SdoInformationPolicy) -> Self {
        Self {
            transfer: SdoInformationTransfer::with_policy(policy),
            expectations: &[],
            phase: SdoInformationVerifierPhase::Idle,
            current: 0,
            verified_count: 0,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> SdoInformationVerifierPhase {
        self.phase
    }

    pub const fn verified_count(&self) -> usize {
        if matches!(self.phase, SdoInformationVerifierPhase::Complete) {
            self.verified_count
        } else {
            0
        }
    }

    pub const fn last_error(&self) -> Option<SdoInformationVerifierError> {
        self.last_error
    }

    pub fn pending_payload(&self) -> &[u8] {
        self.transfer.pending_payload()
    }

    pub fn start(
        &mut self,
        slave_position: u16,
        expectations: &'a [SdoInformationExpectation],
    ) -> Result<(), SdoInformationVerifierError> {
        self.transfer.reset();
        self.expectations = &[];
        self.phase = SdoInformationVerifierPhase::Idle;
        self.current = 0;
        self.verified_count = 0;
        self.last_error = None;

        if expectations.is_empty() {
            return Err(SdoInformationVerifierError::EmptyPlan);
        }
        for (position, expectation) in expectations.iter().enumerate() {
            if expectation.slave_position != slave_position {
                return Err(SdoInformationVerifierError::WrongSlavePosition {
                    expectation: position,
                    expected: slave_position,
                    actual: expectation.slave_position,
                });
            }
            if expectation.index == 0
                || expectation.bit_length == 0
                || !expectation.required_access.valid()
            {
                return Err(SdoInformationVerifierError::InvalidExpectation(position));
            }
            if position > 0 {
                let previous = expectations[position - 1];
                if (expectation.index, expectation.subindex) <= (previous.index, previous.subindex)
                {
                    return Err(
                        SdoInformationVerifierError::DuplicateOrUnorderedExpectation(position),
                    );
                }
            }
        }

        if let Err(error) = self
            .transfer
            .start_object_description(expectations[0].index)
        {
            return self.fail(SdoInformationVerifierError::Protocol(error));
        }
        self.expectations = expectations;
        self.phase = SdoInformationVerifierPhase::AwaitObject;
        Ok(())
    }

    pub fn accept_response(
        &mut self,
        payload: &[u8],
    ) -> Result<SdoInformationVerifierProgress, SdoInformationVerifierError> {
        if !matches!(
            self.phase,
            SdoInformationVerifierPhase::AwaitObject | SdoInformationVerifierPhase::AwaitEntry
        ) {
            return Err(SdoInformationVerifierError::Protocol(
                SdoInformationError::InvalidState,
            ));
        }
        let progress = match self.transfer.accept_response(payload) {
            Ok(progress) => progress,
            Err(error) => return self.fail(SdoInformationVerifierError::Protocol(error)),
        };
        if progress == SdoInformationProgress::Advanced {
            return Ok(SdoInformationVerifierProgress::AwaitingFragment);
        }

        match self.phase {
            SdoInformationVerifierPhase::AwaitObject => self.finish_object(),
            SdoInformationVerifierPhase::AwaitEntry => self.finish_entry(),
            _ => self.fail(SdoInformationVerifierError::Protocol(
                SdoInformationError::InvalidState,
            )),
        }
    }

    fn finish_object(
        &mut self,
    ) -> Result<SdoInformationVerifierProgress, SdoInformationVerifierError> {
        let expectation = self.expectations[self.current];
        let description = match self.transfer.object_description() {
            Some(description) => description,
            None => {
                return self.fail(SdoInformationVerifierError::Protocol(
                    SdoInformationError::Truncated,
                ));
            }
        };
        if description.max_subindex < expectation.subindex {
            return self.fail(SdoInformationVerifierError::ObjectMissingSubindex {
                index: expectation.index,
                expected_subindex: expectation.subindex,
                max_subindex: description.max_subindex,
            });
        }
        if let Err(error) = self.transfer.start_entry_description(
            expectation.index,
            expectation.subindex,
            SdoInfoValueInfo::ACCESS,
        ) {
            return self.fail(SdoInformationVerifierError::Protocol(error));
        }
        self.phase = SdoInformationVerifierPhase::AwaitEntry;
        Ok(SdoInformationVerifierProgress::RequestReady)
    }

    fn finish_entry(
        &mut self,
    ) -> Result<SdoInformationVerifierProgress, SdoInformationVerifierError> {
        let expectation = self.expectations[self.current];
        let description = match self.transfer.entry_description() {
            Some(description) => description,
            None => {
                return self.fail(SdoInformationVerifierError::Protocol(
                    SdoInformationError::Truncated,
                ));
            }
        };
        if description.data_type_raw != expectation.data_type.raw() {
            return self.fail(SdoInformationVerifierError::DataTypeMismatch {
                index: expectation.index,
                subindex: expectation.subindex,
                expected: expectation.data_type,
                actual: description.data_type_raw,
            });
        }
        if description.bit_length != expectation.bit_length {
            return self.fail(SdoInformationVerifierError::BitLengthMismatch {
                index: expectation.index,
                subindex: expectation.subindex,
                expected: expectation.bit_length,
                actual: description.bit_length,
            });
        }
        let required = expectation.required_access;
        let access = description.access;
        if required.requires(SdoInformationRequiredAccess::READ) && !access.readable() {
            return self.fail(SdoInformationVerifierError::ReadAccessMissing {
                index: expectation.index,
                subindex: expectation.subindex,
            });
        }
        if required.requires(SdoInformationRequiredAccess::WRITE) && !access.writable() {
            return self.fail(SdoInformationVerifierError::WriteAccessMissing {
                index: expectation.index,
                subindex: expectation.subindex,
            });
        }
        if required.requires(SdoInformationRequiredAccess::RX_PDO) && !access.rx_pdo_mappable() {
            return self.fail(SdoInformationVerifierError::RxPdoMappingMissing {
                index: expectation.index,
                subindex: expectation.subindex,
            });
        }
        if required.requires(SdoInformationRequiredAccess::TX_PDO) && !access.tx_pdo_mappable() {
            return self.fail(SdoInformationVerifierError::TxPdoMappingMissing {
                index: expectation.index,
                subindex: expectation.subindex,
            });
        }

        self.current += 1;
        if self.current == self.expectations.len() {
            self.verified_count = self.expectations.len();
            self.phase = SdoInformationVerifierPhase::Complete;
            return Ok(SdoInformationVerifierProgress::Complete);
        }

        let next = self.expectations[self.current];
        if next.index == expectation.index {
            if let Err(error) = self.transfer.start_entry_description(
                next.index,
                next.subindex,
                SdoInfoValueInfo::ACCESS,
            ) {
                return self.fail(SdoInformationVerifierError::Protocol(error));
            }
            self.phase = SdoInformationVerifierPhase::AwaitEntry;
        } else {
            if let Err(error) = self.transfer.start_object_description(next.index) {
                return self.fail(SdoInformationVerifierError::Protocol(error));
            }
            self.phase = SdoInformationVerifierPhase::AwaitObject;
        }
        Ok(SdoInformationVerifierProgress::RequestReady)
    }

    fn fail<T>(
        &mut self,
        error: SdoInformationVerifierError,
    ) -> Result<T, SdoInformationVerifierError> {
        self.phase = SdoInformationVerifierPhase::Faulted;
        self.verified_count = 0;
        self.last_error = Some(error);
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENABLED: SdoInformationPolicy = SdoInformationPolicy::new(true);

    fn object_response(index: u16, max_subindex: u8, incomplete: bool, left: u16) -> [u8; 12] {
        let mut response = [0u8; 12];
        response[1] = 0x80;
        response[2] = SdoInfoOpcode::GetObjectDescriptionResponse as u8
            | if incomplete { SDO_INFO_INCOMPLETE } else { 0 };
        response[4..6].copy_from_slice(&left.to_le_bytes());
        response[6..8].copy_from_slice(&index.to_le_bytes());
        response[8..10].copy_from_slice(&0u16.to_le_bytes());
        response[10] = max_subindex;
        response[11] = SdoInfoObjectCode::Record as u8;
        response
    }

    fn entry_response(expectation: SdoInformationExpectation, access: u16) -> [u8; 16] {
        let mut response = [0u8; 16];
        response[1] = 0x80;
        response[2] = SdoInfoOpcode::GetEntryDescriptionResponse as u8;
        response[6..8].copy_from_slice(&expectation.index.to_le_bytes());
        response[8] = expectation.subindex;
        response[9] = SdoInfoValueInfo::ACCESS.raw();
        response[10..12].copy_from_slice(&expectation.data_type.raw().to_le_bytes());
        response[12..14].copy_from_slice(&expectation.bit_length.to_le_bytes());
        response[14..16].copy_from_slice(&access.to_le_bytes());
        response
    }

    #[test]
    fn requests_use_exact_sdo_information_wire_format() {
        let mut transfer = SdoInformationTransfer::<4>::with_policy(ENABLED);
        transfer.start_object_list(SdoInfoOdListType::All).unwrap();
        assert_eq!(transfer.pending_payload(), &[0, 0x80, 0x01, 0, 0, 0, 1, 0]);

        transfer.reset();
        transfer.start_object_description(0x6040).unwrap();
        assert_eq!(
            transfer.pending_payload(),
            &[0, 0x80, 0x03, 0, 0, 0, 0x40, 0x60]
        );

        transfer.reset();
        transfer
            .start_entry_description(0x6040, 2, SdoInfoValueInfo::ACCESS)
            .unwrap();
        assert_eq!(
            transfer.pending_payload(),
            &[0, 0x80, 0x05, 0, 0, 0, 0x40, 0x60, 2, 1]
        );
    }

    #[test]
    fn disabled_policy_fails_before_state_mutation() {
        let mut transfer = SdoInformationTransfer::<1>::new();
        assert_eq!(
            transfer.start_object_description(0x6040),
            Err(SdoInformationError::Disabled)
        );
        assert_eq!(transfer.phase(), SdoInformationPhase::Idle);
        assert!(transfer.pending_payload().is_empty());
    }

    #[test]
    fn fragmented_object_list_is_bounded_and_ordered() {
        let mut transfer = SdoInformationTransfer::<3>::with_policy(ENABLED);
        transfer.start_object_list(SdoInfoOdListType::All).unwrap();
        let first = [0, 0x80, 0x82, 0, 1, 0, 1, 0, 0x40, 0x60, 0x41, 0x60];
        assert_eq!(
            transfer.accept_response(&first),
            Ok(SdoInformationProgress::Advanced)
        );
        assert_eq!(transfer.phase(), SdoInformationPhase::AwaitFragment);
        assert!(transfer.object_indices().is_none());

        let second = [0, 0x80, 0x02, 0, 0, 0, 0x60, 0x60];
        assert_eq!(
            transfer.accept_response(&second),
            Ok(SdoInformationProgress::Complete)
        );
        assert_eq!(
            transfer.object_indices(),
            Some(&[0x6040, 0x6041, 0x6060][..])
        );
    }

    #[test]
    fn fragment_sequence_and_capacity_fail_closed() {
        let mut transfer = SdoInformationTransfer::<2>::with_policy(ENABLED);
        transfer.start_object_list(SdoInfoOdListType::All).unwrap();
        let first = [0, 0x80, 0x82, 0, 2, 0, 1, 0, 0x40, 0x60];
        transfer.accept_response(&first).unwrap();
        let wrong = [0, 0x80, 0x02, 0, 0, 0, 0x41, 0x60];
        assert_eq!(
            transfer.accept_response(&wrong),
            Err(SdoInformationError::FragmentSequence {
                expected: 1,
                actual: 0,
            })
        );
        assert!(transfer.object_indices().is_none());

        transfer.reset();
        transfer.start_object_list(SdoInfoOdListType::All).unwrap();
        let contradictory = [0, 0x80, 0x02, 0, 1, 0, 1, 0, 0x40, 0x60];
        assert_eq!(
            transfer.accept_response(&contradictory),
            Err(SdoInformationError::FragmentState)
        );
        assert_eq!(transfer.phase(), SdoInformationPhase::Faulted);

        transfer.reset();
        transfer.start_object_list(SdoInfoOdListType::All).unwrap();
        let too_many = [
            0, 0x80, 0x02, 0, 0, 0, 1, 0, 0x40, 0x60, 0x41, 0x60, 0x60, 0x60,
        ];
        assert_eq!(
            transfer.accept_response(&too_many),
            Err(SdoInformationError::ObjectCapacity)
        );
        assert!(transfer.object_indices().is_none());
    }

    #[test]
    fn descriptions_decode_typed_metadata_and_abort() {
        let mut transfer = SdoInformationTransfer::<1>::with_policy(ENABLED);
        transfer.start_object_description(0x6040).unwrap();
        let response = object_response(0x6040, 3, false, 0);
        transfer.accept_response(&response).unwrap();
        assert_eq!(
            transfer.object_description(),
            Some(SdoInfoObjectDescription {
                index: 0x6040,
                data_type_raw: 0,
                data_type: None,
                max_subindex: 3,
                object_code_raw: SdoInfoObjectCode::Record as u8,
                object_code: Some(SdoInfoObjectCode::Record),
            })
        );

        transfer.reset();
        transfer
            .start_entry_description(0x6040, 0, SdoInfoValueInfo::ACCESS)
            .unwrap();
        let expectation = SdoInformationExpectation {
            slave_position: 0,
            index: 0x6040,
            subindex: 0,
            data_type: CanopenDataType::Unsigned16,
            bit_length: 16,
            required_access: SdoInformationRequiredAccess::RX_PDO_ENTRY,
        };
        let response = entry_response(expectation, 0x007f);
        transfer.accept_response(&response).unwrap();
        assert_eq!(
            transfer.entry_description().unwrap().data_type,
            Some(CanopenDataType::Unsigned16)
        );
        assert!(transfer.entry_description().unwrap().access.writable());
        assert!(
            transfer
                .entry_description()
                .unwrap()
                .access
                .rx_pdo_mappable()
        );

        transfer.reset();
        transfer.start_object_description(0x6040).unwrap();
        let abort = [0, 0x80, 0x07, 0, 0, 0, 0, 0, 2, 6];
        assert_eq!(
            transfer.accept_response(&abort),
            Err(SdoInformationError::Abort(0x0602_0000))
        );
        assert_eq!(transfer.phase(), SdoInformationPhase::Faulted);
        assert_eq!(
            transfer.last_error(),
            Some(SdoInformationError::Abort(0x0602_0000))
        );
        assert!(transfer.pending_payload().is_empty());
    }

    #[test]
    fn malformed_and_mismatched_responses_fail_transactionally() {
        let mut transfer = SdoInformationTransfer::<1>::with_policy(ENABLED);
        transfer.start_object_description(0x6040).unwrap();
        let mut response = object_response(0x6040, 0, false, 0);
        response[1] = (CoeService::SdoResponse as u8) << 4;
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoInformationError::WrongService)
        );
        assert_eq!(transfer.phase(), SdoInformationPhase::Faulted);
        assert!(transfer.object_description().is_none());

        transfer.reset();
        transfer.start_object_description(0x6040).unwrap();
        let mut response = object_response(0x6040, 0, false, 0);
        response[3] = 1;
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoInformationError::ReservedByte(1))
        );
        assert!(transfer.object_description().is_none());

        transfer.reset();
        transfer.start_object_description(0x6040).unwrap();
        let response = object_response(0x6041, 0, false, 0);
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoInformationError::IndexMismatch {
                expected: 0x6040,
                actual: 0x6041,
            })
        );
        assert!(transfer.object_description().is_none());

        transfer.reset();
        transfer
            .start_entry_description(0x6040, 1, SdoInfoValueInfo::ACCESS)
            .unwrap();
        let expectation = SdoInformationExpectation {
            slave_position: 0,
            index: 0x6040,
            subindex: 0,
            data_type: CanopenDataType::Unsigned16,
            bit_length: 16,
            required_access: SdoInformationRequiredAccess::RX_PDO_ENTRY,
        };
        let response = entry_response(expectation, SdoInfoObjectAccess::WRITE_PREOP);
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoInformationError::SubindexMismatch {
                expected: 1,
                actual: 0,
            })
        );
        assert!(transfer.entry_description().is_none());

        transfer.reset();
        transfer
            .start_entry_description(0x6040, 0, SdoInfoValueInfo::ACCESS)
            .unwrap();
        let mut response = entry_response(expectation, SdoInfoObjectAccess::WRITE_PREOP);
        response[9] = 0;
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoInformationError::ValueInfoMismatch {
                expected: SdoInfoValueInfo::ACCESS,
                actual: SdoInfoValueInfo::from_raw(0),
            })
        );
        assert!(transfer.entry_description().is_none());
    }

    #[test]
    fn unknown_opcode_and_coe_service_fail_into_terminal_state() {
        let mut transfer = SdoInformationTransfer::<1>::with_policy(ENABLED);
        transfer.start_object_description(0x6040).unwrap();
        let mut response = object_response(0x6040, 0, false, 0);
        response[2] = 0x7e;
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoInformationError::UnknownOpcode(0x7e))
        );
        assert_eq!(transfer.phase(), SdoInformationPhase::Faulted);

        transfer.reset();
        transfer.start_object_description(0x6040).unwrap();
        let mut response = object_response(0x6040, 0, false, 0);
        response[1] = 0xf0;
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoInformationError::Coe(SdoError::UnknownService))
        );
        assert_eq!(transfer.phase(), SdoInformationPhase::Faulted);
        assert_eq!(
            transfer.last_error(),
            Some(SdoInformationError::Coe(SdoError::UnknownService))
        );
    }

    #[test]
    fn verifier_completes_multi_object_plan_transactionally() {
        let expectations = [
            SdoInformationExpectation {
                slave_position: 0,
                index: 0x6040,
                subindex: 0,
                data_type: CanopenDataType::Unsigned16,
                bit_length: 16,
                required_access: SdoInformationRequiredAccess::RX_PDO_ENTRY,
            },
            SdoInformationExpectation {
                slave_position: 0,
                index: 0x6060,
                subindex: 0,
                data_type: CanopenDataType::Integer8,
                bit_length: 8,
                required_access: SdoInformationRequiredAccess::RX_PDO_ENTRY,
            },
            SdoInformationExpectation {
                slave_position: 0,
                index: 0x6061,
                subindex: 0,
                data_type: CanopenDataType::Integer8,
                bit_length: 8,
                required_access: SdoInformationRequiredAccess::TX_PDO_ENTRY,
            },
        ];
        let mut verifier = SdoInformationVerifier::<0>::new(ENABLED);
        verifier.start(0, &expectations).unwrap();
        assert_eq!(verifier.verified_count(), 0);

        for expectation in expectations {
            assert_eq!(
                verifier.accept_response(&object_response(
                    expectation.index,
                    expectation.subindex,
                    false,
                    0,
                )),
                Ok(SdoInformationVerifierProgress::RequestReady)
            );
            let access = if expectation
                .required_access
                .requires(SdoInformationRequiredAccess::RX_PDO)
            {
                SdoInfoObjectAccess::WRITE_PREOP | SdoInfoObjectAccess::RX_PDO_MAPPABLE
            } else {
                SdoInfoObjectAccess::READ_PREOP | SdoInfoObjectAccess::TX_PDO_MAPPABLE
            };
            let expected_progress = if expectation.index == 0x6061 {
                SdoInformationVerifierProgress::Complete
            } else {
                SdoInformationVerifierProgress::RequestReady
            };
            assert_eq!(
                verifier.accept_response(&entry_response(expectation, access)),
                Ok(expected_progress)
            );
        }
        assert_eq!(verifier.verified_count(), expectations.len());
    }

    #[test]
    fn verifier_rejects_metadata_and_access_mismatches_without_publication() {
        let expectation = SdoInformationExpectation {
            slave_position: 0,
            index: 0x6040,
            subindex: 0,
            data_type: CanopenDataType::Unsigned16,
            bit_length: 16,
            required_access: SdoInformationRequiredAccess::RX_PDO_ENTRY,
        };
        let expectations = [expectation];
        let mut verifier = SdoInformationVerifier::<0>::new(ENABLED);
        verifier.start(0, &expectations).unwrap();
        verifier
            .accept_response(&object_response(0x6040, 0, false, 0))
            .unwrap();
        let response = entry_response(expectation, SdoInfoObjectAccess::WRITE_PREOP);
        assert_eq!(
            verifier.accept_response(&response),
            Err(SdoInformationVerifierError::RxPdoMappingMissing {
                index: 0x6040,
                subindex: 0,
            })
        );
        assert_eq!(verifier.verified_count(), 0);
        assert_eq!(verifier.phase(), SdoInformationVerifierPhase::Faulted);

        let duplicate = [expectation, expectation];
        let mut verifier = SdoInformationVerifier::<0>::new(ENABLED);
        assert_eq!(
            verifier.start(0, &duplicate),
            Err(SdoInformationVerifierError::DuplicateOrUnorderedExpectation(1))
        );
        assert_eq!(verifier.phase(), SdoInformationVerifierPhase::Idle);
    }

    #[test]
    fn verifier_preflight_rejects_wrong_owner_and_invalid_expectations() {
        let wrong_owner = [SdoInformationExpectation {
            slave_position: 2,
            index: 0x6040,
            subindex: 0,
            data_type: CanopenDataType::Unsigned16,
            bit_length: 16,
            required_access: SdoInformationRequiredAccess::RX_PDO_ENTRY,
        }];
        let mut verifier = SdoInformationVerifier::<0>::new(ENABLED);
        assert_eq!(
            verifier.start(1, &wrong_owner),
            Err(SdoInformationVerifierError::WrongSlavePosition {
                expectation: 0,
                expected: 1,
                actual: 2,
            })
        );
        assert_eq!(verifier.phase(), SdoInformationVerifierPhase::Idle);

        let invalid = [SdoInformationExpectation {
            slave_position: 1,
            index: 0,
            subindex: 0,
            data_type: CanopenDataType::Unsigned16,
            bit_length: 0,
            required_access: SdoInformationRequiredAccess::from_raw(0),
        }];
        assert_eq!(
            verifier.start(1, &invalid),
            Err(SdoInformationVerifierError::InvalidExpectation(0))
        );
        assert_eq!(verifier.phase(), SdoInformationVerifierPhase::Idle);
    }

    #[test]
    fn verifier_restart_clears_previous_success_before_preflight() {
        let expectation = SdoInformationExpectation {
            slave_position: 0,
            index: 0x6040,
            subindex: 0,
            data_type: CanopenDataType::Unsigned16,
            bit_length: 16,
            required_access: SdoInformationRequiredAccess::RX_PDO_ENTRY,
        };
        let expectations = [expectation];
        let mut verifier = SdoInformationVerifier::<0>::new(ENABLED);
        verifier.start(0, &expectations).unwrap();
        verifier
            .accept_response(&object_response(0x6040, 0, false, 0))
            .unwrap();
        verifier
            .accept_response(&entry_response(
                expectation,
                SdoInfoObjectAccess::WRITE_PREOP | SdoInfoObjectAccess::RX_PDO_MAPPABLE,
            ))
            .unwrap();
        assert_eq!(verifier.verified_count(), 1);

        assert_eq!(
            verifier.start(0, &[]),
            Err(SdoInformationVerifierError::EmptyPlan)
        );
        assert_eq!(verifier.phase(), SdoInformationVerifierPhase::Idle);
        assert_eq!(verifier.verified_count(), 0);
        assert!(verifier.pending_payload().is_empty());
    }
}
