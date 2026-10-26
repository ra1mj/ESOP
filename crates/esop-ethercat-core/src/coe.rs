//! Allocation-free CoE SDO codec and transfer state machine.
//!
//! The controller only deals with CoE payloads. Mailbox headers, register
//! access, retry policy and cycle budgeting remain owned by `mailbox.rs` and
//! the caller, respectively.

use crate::mailbox::{MAILBOX_HEADER_LEN, MAX_MAILBOX_BYTES};

pub const COE_HEADER_LEN: usize = 2;
pub const SDO_DATA_OFFSET: usize = 6;
pub const MAX_SDO_DATA: usize = MAX_MAILBOX_BYTES - SDO_DATA_OFFSET;
/// Minimum CANopen segment data area, padded when the final data is shorter.
/// EtherCAT segments may carry more than this when the mailbox has room.
pub const MAX_SDO_SEGMENT_BYTES: usize = 7;
pub const COE_EMERGENCY_LEN: usize = 10;
pub const SDO_INITIATE_LEN: usize = COE_HEADER_LEN + 8;
pub const MAX_COE_PAYLOAD_BYTES: usize = MAX_MAILBOX_BYTES - MAILBOX_HEADER_LEN;

const SDO_UPLOAD_REQUEST: u8 = 0x40;
const SDO_DOWNLOAD_EXPEDITED: u8 = 0x23;
const SDO_DOWNLOAD_NORMAL: u8 = 0x21;
const SDO_ABORT: u8 = 0x80;
const SDO_DOWNLOAD_RESPONSE: u8 = 0x60;
const SDO_UPLOAD_SEGMENT_REQUEST: u8 = 0x60;
const SDO_DOWNLOAD_SEGMENT_RESPONSE: u8 = 0x20;
const SDO_COMPLETE_ACCESS: u8 = 0x10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CoeService {
    Emergency = 0x01,
    SdoRequest = 0x02,
    SdoResponse = 0x03,
    TxPdo = 0x04,
    RxPdo = 0x05,
    TxPdoRemote = 0x06,
    RxPdoRemote = 0x07,
    SdoInformation = 0x08,
}

impl CoeService {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::Emergency,
            0x02 => Self::SdoRequest,
            0x03 => Self::SdoResponse,
            0x04 => Self::TxPdo,
            0x05 => Self::RxPdo,
            0x06 => Self::TxPdoRemote,
            0x07 => Self::RxPdoRemote,
            0x08 => Self::SdoInformation,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoeHeader {
    pub number: u8,
    pub service: CoeService,
}

impl CoeHeader {
    pub fn encode(&self, dst: &mut [u8]) -> Result<(), SdoError> {
        if dst.len() < COE_HEADER_LEN {
            return Err(SdoError::BufferTooSmall);
        }
        if self.number > 0x0F {
            return Err(SdoError::HeaderOutOfBounds);
        }
        dst[0] = self.number;
        dst[1] = (self.service as u8) << 4;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> Result<Self, SdoError> {
        if src.len() < COE_HEADER_LEN {
            return Err(SdoError::Truncated);
        }
        let service = CoeService::from_u8(src[1] >> 4).ok_or(SdoError::UnknownService)?;
        Ok(Self {
            number: src[0] & 0x0F,
            service,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoeEmergency {
    pub error_code: u16,
    pub error_register: u8,
    pub manufacturer_data: [u8; 5],
}

impl CoeEmergency {
    pub fn parse(payload: &[u8]) -> Result<Self, SdoError> {
        let header = CoeHeader::decode(payload)?;
        if header.service != CoeService::Emergency {
            return Err(SdoError::WrongService);
        }
        if payload.len() < COE_EMERGENCY_LEN {
            return Err(SdoError::Truncated);
        }
        let mut manufacturer_data = [0; 5];
        manufacturer_data.copy_from_slice(&payload[5..COE_EMERGENCY_LEN]);
        Ok(Self {
            error_code: u16::from_le_bytes([payload[2], payload[3]]),
            error_register: payload[4],
            manufacturer_data,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoDirection {
    Upload,
    Download,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SdoAccess {
    #[default]
    Single,
    Complete,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdoAccessPolicy {
    complete_access_enabled: bool,
}

impl SdoAccessPolicy {
    pub const fn new(complete_access_enabled: bool) -> Self {
        Self {
            complete_access_enabled,
        }
    }

    pub const fn complete_access_enabled(self) -> bool {
        self.complete_access_enabled
    }

    pub const fn permits(self, access: SdoAccess) -> bool {
        matches!(access, SdoAccess::Single) || self.complete_access_enabled
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoPhase {
    Idle,
    AwaitInitiate,
    AwaitSegment,
    Complete,
    Aborted,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoProgress {
    Advanced,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdoError {
    Busy,
    InvalidState,
    BufferTooSmall,
    PayloadTooLarge,
    Truncated,
    HeaderOutOfBounds,
    UnknownService,
    WrongService,
    UnexpectedCommand,
    IndexMismatch,
    SubindexMismatch,
    ToggleMismatch,
    SizeMismatch,
    SegmentMalformed,
    CompleteAccessDisabled,
    InvalidCompleteAccessSubindex,
    CompleteAccessMismatch,
    Timeout,
    Abort(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdoResponse<'a> {
    pub index: Option<u16>,
    pub subindex: Option<u8>,
    pub data: &'a [u8],
    pub total_size: Option<usize>,
    pub toggle: bool,
    pub last: bool,
    pub abort_code: Option<u32>,
    pub command: u8,
    pub complete_access: Option<bool>,
}

impl<'a> SdoResponse<'a> {
    pub fn parse(payload: &'a [u8]) -> Result<Self, SdoError> {
        let header = CoeHeader::decode(payload)?;
        if header.service != CoeService::SdoResponse {
            return Err(SdoError::WrongService);
        }
        if payload.len() < 3 {
            return Err(SdoError::Truncated);
        }
        let command = payload[2];

        if command == SDO_ABORT {
            if payload.len() < SDO_DATA_OFFSET + 4 {
                return Err(SdoError::Truncated);
            }
            let index = u16::from_le_bytes([payload[3], payload[4]]);
            let subindex = payload[5];
            return Ok(Self {
                index: Some(index),
                subindex: Some(subindex),
                data: &[],
                total_size: None,
                toggle: false,
                last: true,
                abort_code: Some(u32::from_le_bytes([
                    payload[6], payload[7], payload[8], payload[9],
                ])),
                command,
                complete_access: None,
            });
        }

        if command & 0xE0 == 0x40 {
            if payload.len() < SDO_INITIATE_LEN {
                return Err(SdoError::Truncated);
            }
            let index = u16::from_le_bytes([payload[3], payload[4]]);
            let subindex = payload[5];
            if command & 0x02 != 0 {
                let data_len = if command & 0x01 != 0 {
                    4 - ((command >> 2) & 0x03) as usize
                } else {
                    4
                };
                return Ok(Self {
                    index: Some(index),
                    subindex: Some(subindex),
                    data: &payload[SDO_DATA_OFFSET..SDO_DATA_OFFSET + data_len],
                    total_size: None,
                    toggle: false,
                    last: true,
                    abort_code: None,
                    command,
                    complete_access: Some(command & SDO_COMPLETE_ACCESS != 0),
                });
            }

            let total_size = if command & 0x01 != 0 {
                if payload.len() < SDO_DATA_OFFSET + 4 {
                    return Err(SdoError::Truncated);
                }
                Some(u32::from_le_bytes([payload[6], payload[7], payload[8], payload[9]]) as usize)
            } else {
                None
            };
            return Ok(Self {
                index: Some(index),
                subindex: Some(subindex),
                data: &payload[SDO_INITIATE_LEN..],
                total_size,
                toggle: false,
                last: false,
                abort_code: None,
                command,
                complete_access: Some(command & SDO_COMPLETE_ACCESS != 0),
            });
        }

        if command == SDO_DOWNLOAD_RESPONSE {
            if payload.len() < SDO_DATA_OFFSET {
                return Err(SdoError::Truncated);
            }
            let index = u16::from_le_bytes([payload[3], payload[4]]);
            let subindex = payload[5];
            return Ok(Self {
                index: Some(index),
                subindex: Some(subindex),
                data: &[],
                total_size: None,
                toggle: false,
                last: true,
                abort_code: None,
                command,
                complete_access: None,
            });
        }

        if command & 0xE0 == 0 {
            let unused = ((command >> 1) & 0x07) as usize;
            let segment_len = payload.len().saturating_sub(3);
            if segment_len < MAX_SDO_SEGMENT_BYTES
                || (unused != 0 && (command & 0x01 == 0 || segment_len != MAX_SDO_SEGMENT_BYTES))
            {
                return Err(SdoError::SegmentMalformed);
            }
            let data_len = if command & 0x01 != 0 {
                segment_len - unused
            } else {
                segment_len
            };
            return Ok(Self {
                index: None,
                subindex: None,
                data: &payload[3..3 + data_len],
                total_size: None,
                toggle: command & 0x10 != 0,
                last: command & 0x01 != 0,
                abort_code: None,
                command,
                complete_access: None,
            });
        }

        if command & 0xE0 == SDO_DOWNLOAD_SEGMENT_RESPONSE {
            if command & 0x0F != 0 {
                return Err(SdoError::SegmentMalformed);
            }
            return Ok(Self {
                index: None,
                subindex: None,
                data: &[],
                total_size: None,
                toggle: command & 0x10 != 0,
                last: true,
                abort_code: None,
                command,
                complete_access: None,
            });
        }

        Err(SdoError::UnexpectedCommand)
    }
}

/// The legacy object capacity is independent of the physical mailbox size.
pub type SdoTransfer = BoundedSdoTransfer<MAX_SDO_DATA>;

/// A caller-owned object buffer and bounded physical CoE request buffer.
pub struct BoundedSdoTransfer<const OBJECT_BYTES: usize> {
    phase: SdoPhase,
    direction: SdoDirection,
    index: u16,
    subindex: u8,
    access: SdoAccess,
    access_policy: SdoAccessPolicy,
    data: [u8; OBJECT_BYTES],
    data_len: usize,
    expected_size: Option<usize>,
    offset: usize,
    toggle: bool,
    pending: [u8; MAX_MAILBOX_BYTES],
    pending_len: usize,
    abort_code: Option<u32>,
    last_error: Option<SdoError>,
    mailbox_payload_capacity: usize,
}

impl<const OBJECT_BYTES: usize> BoundedSdoTransfer<OBJECT_BYTES> {
    pub const fn new() -> Self {
        Self::with_policy(SdoAccessPolicy::new(false))
    }

    pub const fn with_policy(access_policy: SdoAccessPolicy) -> Self {
        Self {
            phase: SdoPhase::Idle,
            direction: SdoDirection::Upload,
            index: 0,
            subindex: 0,
            access: SdoAccess::Single,
            access_policy,
            data: [0; OBJECT_BYTES],
            data_len: 0,
            expected_size: None,
            offset: 0,
            toggle: false,
            pending: [0; MAX_MAILBOX_BYTES],
            pending_len: 0,
            abort_code: None,
            last_error: None,
            mailbox_payload_capacity: MAX_COE_PAYLOAD_BYTES,
        }
    }

    /// Freeze the CoE capacity, excluding the six-byte mailbox header.
    /// The caller derives this from the slave's send mailbox before starting.
    pub const fn with_mailbox_payload_capacity(
        access_policy: SdoAccessPolicy,
        mailbox_payload_capacity: usize,
    ) -> Result<Self, SdoError> {
        if mailbox_payload_capacity < SDO_INITIATE_LEN
            || mailbox_payload_capacity > MAX_COE_PAYLOAD_BYTES
        {
            return Err(SdoError::BufferTooSmall);
        }
        let mut transfer = Self::with_policy(access_policy);
        transfer.mailbox_payload_capacity = mailbox_payload_capacity;
        Ok(transfer)
    }

    pub const fn mailbox_payload_capacity(&self) -> usize {
        self.mailbox_payload_capacity
    }

    pub const fn object_capacity(&self) -> usize {
        OBJECT_BYTES
    }

    pub const fn phase(&self) -> SdoPhase {
        self.phase
    }

    pub const fn direction(&self) -> SdoDirection {
        self.direction
    }

    pub const fn index(&self) -> u16 {
        self.index
    }

    pub const fn subindex(&self) -> u8 {
        self.subindex
    }

    pub const fn access(&self) -> SdoAccess {
        self.access
    }

    pub const fn access_policy(&self) -> SdoAccessPolicy {
        self.access_policy
    }

    pub const fn data_len(&self) -> usize {
        if matches!(self.phase, SdoPhase::Complete) {
            self.data_len
        } else {
            0
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.data_len()]
    }

    pub fn request(&self) -> Option<&[u8]> {
        if self.pending_len == 0 {
            None
        } else {
            Some(&self.pending[..self.pending_len])
        }
    }

    pub const fn expected_size(&self) -> Option<usize> {
        self.expected_size
    }

    pub const fn abort_code(&self) -> Option<u32> {
        self.abort_code
    }

    pub const fn last_error(&self) -> Option<SdoError> {
        self.last_error
    }

    pub fn start_upload(
        &mut self,
        index: u16,
        subindex: u8,
        access: SdoAccess,
    ) -> Result<(), SdoError> {
        self.begin(SdoDirection::Upload, index, subindex, access, &[])?;
        self.build_upload_initiate();
        Ok(())
    }

    pub fn start_download(
        &mut self,
        index: u16,
        subindex: u8,
        data: &[u8],
        access: SdoAccess,
    ) -> Result<(), SdoError> {
        self.begin(SdoDirection::Download, index, subindex, access, data)?;
        if !data.is_empty() && data.len() <= 4 {
            self.build_download_expedited();
        } else {
            self.build_download_initiate();
        }
        Ok(())
    }

    pub fn accept_response(&mut self, payload: &[u8]) -> Result<SdoProgress, SdoError> {
        if !matches!(self.phase, SdoPhase::AwaitInitiate | SdoPhase::AwaitSegment) {
            return Err(SdoError::InvalidState);
        }
        if payload.len() > MAX_COE_PAYLOAD_BYTES {
            return self.fail(SdoError::PayloadTooLarge);
        }
        let response = match SdoResponse::parse(payload) {
            Ok(response) => response,
            Err(error) => return self.fail(error),
        };
        if let Some(code) = response.abort_code {
            if let Err(error) = self.check_object(response.index, response.subindex) {
                return self.fail(error);
            }
            self.phase = SdoPhase::Aborted;
            self.pending_len = 0;
            self.abort_code = Some(code);
            self.last_error = Some(SdoError::Abort(code));
            return Err(SdoError::Abort(code));
        }

        let result = match (self.direction, self.phase) {
            (SdoDirection::Upload, SdoPhase::AwaitInitiate) => {
                if response.command & 0xE0 != 0x40 {
                    return self.fail(SdoError::UnexpectedCommand);
                }
                if let Err(error) = self.check_object(response.index, response.subindex) {
                    return self.fail(error);
                }
                if response.complete_access != Some(matches!(self.access, SdoAccess::Complete)) {
                    return self.fail(SdoError::CompleteAccessMismatch);
                }
                if response.command & 0x02 != 0 {
                    if let Err(error) = self.store_upload_data(response.data) {
                        return self.fail(error);
                    }
                    self.expected_size = Some(response.data.len());
                    self.complete();
                    Ok(SdoProgress::Complete)
                } else {
                    if let Some(expected_size) = response.total_size {
                        if expected_size > OBJECT_BYTES {
                            return self.fail(SdoError::PayloadTooLarge);
                        }
                        self.expected_size = Some(expected_size);
                    } else {
                        self.expected_size = None;
                    }
                    if let Err(error) = self.store_upload_data(response.data) {
                        return self.fail(error);
                    }
                    if self.expected_size == Some(self.data_len) {
                        self.complete();
                        return Ok(SdoProgress::Complete);
                    }
                    self.toggle = false;
                    self.build_upload_segment_request();
                    Ok(SdoProgress::Advanced)
                }
            }
            (SdoDirection::Download, SdoPhase::AwaitInitiate) => {
                if let Err(error) = self.check_object(response.index, response.subindex) {
                    return self.fail(error);
                }
                if response.command != SDO_DOWNLOAD_RESPONSE {
                    return self.fail(SdoError::UnexpectedCommand);
                }
                if self.offset >= self.data_len {
                    self.complete();
                    Ok(SdoProgress::Complete)
                } else {
                    self.toggle = false;
                    self.build_download_segment();
                    Ok(SdoProgress::Advanced)
                }
            }
            (SdoDirection::Upload, SdoPhase::AwaitSegment) => {
                if response.index.is_some()
                    || response.subindex.is_some()
                    || response.command & 0xE0 != 0
                {
                    return self.fail(SdoError::UnexpectedCommand);
                }
                if response.toggle != self.toggle {
                    return self.fail(SdoError::ToggleMismatch);
                }
                if let Err(error) = self.store_upload_data(response.data) {
                    return self.fail(error);
                }
                if response.last {
                    if self
                        .expected_size
                        .is_some_and(|expected_size| expected_size != self.data_len)
                    {
                        return self.fail(SdoError::SizeMismatch);
                    }
                    self.complete();
                    Ok(SdoProgress::Complete)
                } else {
                    self.toggle = !self.toggle;
                    self.build_upload_segment_request();
                    Ok(SdoProgress::Advanced)
                }
            }
            (SdoDirection::Download, SdoPhase::AwaitSegment) => {
                if response.index.is_some() || response.subindex.is_some() {
                    return self.fail(SdoError::UnexpectedCommand);
                }
                if response.command & 0xE0 != SDO_DOWNLOAD_SEGMENT_RESPONSE {
                    return self.fail(SdoError::UnexpectedCommand);
                }
                if response.toggle != self.toggle {
                    return self.fail(SdoError::ToggleMismatch);
                }
                if self.offset >= self.data_len {
                    self.complete();
                    Ok(SdoProgress::Complete)
                } else {
                    self.toggle = !self.toggle;
                    self.build_download_segment();
                    Ok(SdoProgress::Advanced)
                }
            }
            _ => self.fail(SdoError::InvalidState),
        };

        match result {
            Ok(progress) => Ok(progress),
            Err(error) => self.fail(error),
        }
    }

    /// End active work when its owning transport deadline or request fails.
    /// Completed results stay immutable until another explicit start.
    pub fn terminate(&mut self, error: SdoError) -> Result<(), SdoError> {
        if !matches!(self.phase, SdoPhase::AwaitInitiate | SdoPhase::AwaitSegment) {
            return Err(SdoError::InvalidState);
        }
        self.phase = SdoPhase::Faulted;
        self.pending_len = 0;
        self.last_error = Some(error);
        Ok(())
    }

    fn begin(
        &mut self,
        direction: SdoDirection,
        index: u16,
        subindex: u8,
        access: SdoAccess,
        data: &[u8],
    ) -> Result<(), SdoError> {
        if !matches!(
            self.phase,
            SdoPhase::Idle | SdoPhase::Complete | SdoPhase::Aborted | SdoPhase::Faulted
        ) {
            return Err(SdoError::Busy);
        }
        if data.len() > OBJECT_BYTES || data.len() > u32::MAX as usize {
            return Err(SdoError::PayloadTooLarge);
        }
        if !self.access_policy.permits(access) {
            return Err(SdoError::CompleteAccessDisabled);
        }
        if matches!(access, SdoAccess::Complete) && subindex > 1 {
            return Err(SdoError::InvalidCompleteAccessSubindex);
        }
        self.phase = SdoPhase::AwaitInitiate;
        self.direction = direction;
        self.index = index;
        self.subindex = subindex;
        self.access = access;
        self.data.fill(0);
        self.data[..data.len()].copy_from_slice(data);
        self.data_len = data.len();
        self.expected_size = None;
        self.offset = 0;
        self.toggle = false;
        self.pending.fill(0);
        self.pending_len = 0;
        self.abort_code = None;
        self.last_error = None;
        Ok(())
    }

    fn check_object(&self, index: Option<u16>, subindex: Option<u8>) -> Result<(), SdoError> {
        if index != Some(self.index) {
            return Err(SdoError::IndexMismatch);
        }
        if subindex != Some(self.subindex) {
            return Err(SdoError::SubindexMismatch);
        }
        Ok(())
    }

    fn write_common_header(&mut self, service: CoeService, command: u8) {
        let _ = (CoeHeader { number: 0, service }).encode(&mut self.pending[..COE_HEADER_LEN]);
        self.pending[2] = command;
        self.pending[3..5].copy_from_slice(&self.index.to_le_bytes());
        self.pending[5] = self.subindex;
    }

    fn build_upload_initiate(&mut self) {
        let command = SDO_UPLOAD_REQUEST
            | if matches!(self.access, SdoAccess::Complete) {
                SDO_COMPLETE_ACCESS
            } else {
                0
            };
        self.write_common_header(CoeService::SdoRequest, command);
        self.pending[SDO_DATA_OFFSET..SDO_INITIATE_LEN].fill(0);
        self.pending_len = SDO_INITIATE_LEN;
    }

    fn build_download_expedited(&mut self) {
        let unused = 4 - self.data_len;
        let command = SDO_DOWNLOAD_EXPEDITED
            | ((unused as u8) << 2)
            | if matches!(self.access, SdoAccess::Complete) {
                SDO_COMPLETE_ACCESS
            } else {
                0
            };
        self.write_common_header(CoeService::SdoRequest, command);
        self.pending[SDO_DATA_OFFSET..SDO_DATA_OFFSET + 4].fill(0);
        self.pending[SDO_DATA_OFFSET..SDO_DATA_OFFSET + self.data_len]
            .copy_from_slice(&self.data[..self.data_len]);
        self.pending_len = SDO_DATA_OFFSET + 4;
        self.offset = self.data_len;
    }

    fn build_download_initiate(&mut self) {
        let command = SDO_DOWNLOAD_NORMAL
            | if matches!(self.access, SdoAccess::Complete) {
                SDO_COMPLETE_ACCESS
            } else {
                0
            };
        self.write_common_header(CoeService::SdoRequest, command);
        self.pending[SDO_DATA_OFFSET..SDO_DATA_OFFSET + 4]
            .copy_from_slice(&(self.data_len as u32).to_le_bytes());
        let initial_len = self
            .data_len
            .min(self.mailbox_payload_capacity - SDO_INITIATE_LEN);
        self.pending[SDO_INITIATE_LEN..SDO_INITIATE_LEN + initial_len]
            .copy_from_slice(&self.data[..initial_len]);
        self.pending_len = SDO_INITIATE_LEN + initial_len;
        self.offset = initial_len;
    }

    fn build_upload_segment_request(&mut self) {
        let command = SDO_UPLOAD_SEGMENT_REQUEST | if self.toggle { 0x10 } else { 0 };
        let _ = (CoeHeader {
            number: 0,
            service: CoeService::SdoRequest,
        })
        .encode(&mut self.pending[..COE_HEADER_LEN]);
        self.pending[2] = command;
        self.pending[3..SDO_INITIATE_LEN].fill(0);
        self.pending_len = SDO_INITIATE_LEN;
        self.phase = SdoPhase::AwaitSegment;
    }

    fn build_download_segment(&mut self) {
        let remaining = self.data_len - self.offset;
        let segment_len = remaining.min(self.mailbox_payload_capacity - 3);
        let last = segment_len == remaining;
        let unused = if last && segment_len < MAX_SDO_SEGMENT_BYTES {
            MAX_SDO_SEGMENT_BYTES - segment_len
        } else {
            0
        };
        let command =
            (if self.toggle { 0x10 } else { 0 }) | ((unused as u8) << 1) | if last { 1 } else { 0 };
        let _ = (CoeHeader {
            number: 0,
            service: CoeService::SdoRequest,
        })
        .encode(&mut self.pending[..COE_HEADER_LEN]);
        self.pending[2] = command;
        let body_len = segment_len.max(MAX_SDO_SEGMENT_BYTES);
        self.pending[3..3 + body_len].fill(0);
        self.pending[3..3 + segment_len]
            .copy_from_slice(&self.data[self.offset..self.offset + segment_len]);
        self.pending_len = 3 + body_len;
        self.offset += segment_len;
        self.phase = SdoPhase::AwaitSegment;
    }

    fn store_upload_data(&mut self, data: &[u8]) -> Result<(), SdoError> {
        if data.len() > OBJECT_BYTES.saturating_sub(self.data_len) {
            return Err(SdoError::PayloadTooLarge);
        }
        if self
            .expected_size
            .is_some_and(|size| data.len() > size.saturating_sub(self.data_len))
        {
            return Err(SdoError::SizeMismatch);
        }
        self.data[self.data_len..self.data_len + data.len()].copy_from_slice(data);
        self.data_len += data.len();
        Ok(())
    }

    fn complete(&mut self) {
        self.phase = SdoPhase::Complete;
        self.pending_len = 0;
    }

    fn fail<T>(&mut self, error: SdoError) -> Result<T, SdoError> {
        self.phase = SdoPhase::Faulted;
        self.pending_len = 0;
        self.last_error = Some(error);
        Err(error)
    }
}

impl<const OBJECT_BYTES: usize> Default for BoundedSdoTransfer<OBJECT_BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_header(dst: &mut [u8]) {
        CoeHeader {
            number: 0,
            service: CoeService::SdoResponse,
        }
        .encode(dst)
        .unwrap();
    }

    #[test]
    fn expedited_download_encodes_command_and_fixed_data_area() {
        let mut transfer = SdoTransfer::new();
        transfer
            .start_download(0x6040, 0, &[0x06, 0x00], SdoAccess::Single)
            .unwrap();
        let request = transfer.request().unwrap();
        assert_eq!(request.len(), 10);
        assert_eq!(request[0..2], [0, 0x20]);
        assert_eq!(request[2], 0x2B);
        assert_eq!(&request[3..6], &[0x40, 0x60, 0]);
        assert_eq!(&request[6..10], &[0x06, 0x00, 0, 0]);
    }

    #[test]
    fn complete_access_requests_use_bit_four_only_on_initiate() {
        let policy = SdoAccessPolicy::new(true);

        let mut upload = SdoTransfer::with_policy(policy);
        upload.start_upload(0x2000, 1, SdoAccess::Complete).unwrap();
        assert_eq!(upload.request().unwrap()[2], 0x50);

        let mut expedited = SdoTransfer::with_policy(policy);
        expedited
            .start_download(0x2000, 1, &[1, 2], SdoAccess::Complete)
            .unwrap();
        assert_eq!(expedited.request().unwrap()[2], 0x3B);

        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let mut segmented = SdoTransfer::with_mailbox_payload_capacity(policy, 10).unwrap();
        segmented
            .start_download(0x2000, 1, &data, SdoAccess::Complete)
            .unwrap();
        assert_eq!(segmented.request().unwrap()[2], 0x31);

        let mut initiate_response = [0; 6];
        response_header(&mut initiate_response);
        initiate_response[2] = SDO_DOWNLOAD_RESPONSE;
        initiate_response[3..5].copy_from_slice(&0x2000u16.to_le_bytes());
        initiate_response[5] = 1;
        assert_eq!(
            segmented.accept_response(&initiate_response),
            Ok(SdoProgress::Advanced)
        );
        assert_eq!(segmented.request().unwrap()[2], 0);

        let mut ack = [0; 3];
        response_header(&mut ack);
        ack[2] = SDO_DOWNLOAD_SEGMENT_RESPONSE;
        assert_eq!(segmented.accept_response(&ack), Ok(SdoProgress::Advanced));
        assert_eq!(segmented.request().unwrap()[2], 0x1B);
    }

    #[test]
    fn complete_access_admission_is_fail_closed_before_state_mutation() {
        let mut disabled = SdoTransfer::new();
        assert_eq!(
            disabled.start_upload(0x2000, 0, SdoAccess::Complete),
            Err(SdoError::CompleteAccessDisabled)
        );
        assert_eq!(disabled.phase(), SdoPhase::Idle);
        assert_eq!(disabled.index(), 0);
        assert_eq!(disabled.request(), None);

        let mut invalid_subindex = SdoTransfer::with_policy(SdoAccessPolicy::new(true));
        assert_eq!(
            invalid_subindex.start_download(0x2000, 2, &[1], SdoAccess::Complete),
            Err(SdoError::InvalidCompleteAccessSubindex)
        );
        assert_eq!(invalid_subindex.phase(), SdoPhase::Idle);
        assert_eq!(invalid_subindex.request(), None);
    }

    #[test]
    fn complete_access_upload_requires_matching_response_mode() {
        let mut transfer = SdoTransfer::with_policy(SdoAccessPolicy::new(true));
        transfer
            .start_upload(0x2000, 1, SdoAccess::Complete)
            .unwrap();

        let mut response = [0; 10];
        response_header(&mut response);
        response[2] = 0x4B;
        response[3..5].copy_from_slice(&0x2000u16.to_le_bytes());
        response[5] = 1;
        response[6..8].copy_from_slice(&[0x34, 0x12]);
        assert_eq!(
            transfer.accept_response(&response),
            Err(SdoError::CompleteAccessMismatch)
        );
        assert_eq!(transfer.phase(), SdoPhase::Faulted);

        let mut matching = SdoTransfer::with_policy(SdoAccessPolicy::new(true));
        matching
            .start_upload(0x2000, 1, SdoAccess::Complete)
            .unwrap();
        response[2] = 0x5B;
        assert_eq!(
            matching.accept_response(&response),
            Ok(SdoProgress::Complete)
        );
        assert_eq!(matching.data(), &[0x34, 0x12]);
    }

    #[test]
    fn expedited_upload_completes_and_validates_object() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x6064, 0, SdoAccess::Single).unwrap();
        assert_eq!(transfer.request().unwrap()[2], SDO_UPLOAD_REQUEST);

        let mut response = [0; 10];
        response_header(&mut response);
        response[2] = 0x4B;
        response[3..5].copy_from_slice(&0x6064u16.to_le_bytes());
        response[5] = 0;
        response[6..8].copy_from_slice(&[0x34, 0x12]);
        assert_eq!(
            transfer.accept_response(&response),
            Ok(SdoProgress::Complete)
        );
        assert_eq!(transfer.data(), &[0x34, 0x12]);
        assert_eq!(transfer.phase(), SdoPhase::Complete);
    }

    #[test]
    fn segmented_download_tracks_toggle_and_last_segment() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let mut transfer =
            SdoTransfer::with_mailbox_payload_capacity(SdoAccessPolicy::new(false), 10).unwrap();
        transfer
            .start_download(0x2000, 1, &data, SdoAccess::Single)
            .unwrap();

        let mut initiate_response = [0; 6];
        response_header(&mut initiate_response);
        initiate_response[2] = SDO_DOWNLOAD_RESPONSE;
        initiate_response[3..5].copy_from_slice(&0x2000u16.to_le_bytes());
        initiate_response[5] = 1;
        assert_eq!(
            transfer.accept_response(&initiate_response),
            Ok(SdoProgress::Advanced)
        );
        assert_eq!(&transfer.request().unwrap()[2..], &[0, 1, 2, 3, 4, 5, 6, 7]);

        let mut ack = [0; 3];
        response_header(&mut ack);
        ack[2] = SDO_DOWNLOAD_SEGMENT_RESPONSE;
        assert_eq!(transfer.accept_response(&ack), Ok(SdoProgress::Advanced));
        assert_eq!(
            &transfer.request().unwrap()[2..],
            &[0x1B, 8, 9, 0, 0, 0, 0, 0]
        );

        ack[2] = SDO_DOWNLOAD_SEGMENT_RESPONSE | 0x10;
        assert_eq!(transfer.accept_response(&ack), Ok(SdoProgress::Complete));
        assert_eq!(transfer.phase(), SdoPhase::Complete);
    }

    #[test]
    fn segmented_upload_rejects_wrong_toggle_without_publishing_partial_data() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();

        let mut initiate_response = [0; 10];
        response_header(&mut initiate_response);
        initiate_response[2] = 0x41;
        initiate_response[3..5].copy_from_slice(&0x2000u16.to_le_bytes());
        initiate_response[5] = 1;
        initiate_response[6..10].copy_from_slice(&8u32.to_le_bytes());
        transfer.accept_response(&initiate_response).unwrap();

        let mut segment = [0; 10];
        response_header(&mut segment);
        segment[2] = 0x11;
        segment[3..10].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(
            transfer.accept_response(&segment),
            Err(SdoError::ToggleMismatch)
        );
        assert_eq!(transfer.data_len(), 0);
        assert_eq!(transfer.phase(), SdoPhase::Faulted);
    }

    #[test]
    fn normal_upload_without_size_indication_completes_on_last_segment() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();

        let mut initiate_response = [0; 10];
        response_header(&mut initiate_response);
        initiate_response[2] = 0x40;
        initiate_response[3..5].copy_from_slice(&0x2000u16.to_le_bytes());
        initiate_response[5] = 1;
        assert_eq!(
            transfer.accept_response(&initiate_response),
            Ok(SdoProgress::Advanced)
        );

        let mut segment = [0; 10];
        response_header(&mut segment);
        segment[2] = 0x05;
        segment[3..8].copy_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(
            transfer.accept_response(&segment),
            Ok(SdoProgress::Complete)
        );
        assert_eq!(transfer.data(), &[1, 2, 3, 4, 5]);
        assert_eq!(transfer.expected_size(), None);
    }

    #[test]
    fn abort_response_is_preserved_as_a_terminal_protocol_result() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x6041, 0, SdoAccess::Single).unwrap();

        let mut abort = [0; 10];
        response_header(&mut abort);
        abort[2] = SDO_ABORT;
        abort[3..5].copy_from_slice(&0x6041u16.to_le_bytes());
        abort[5] = 0;
        abort[6..10].copy_from_slice(&0x0602_0000u32.to_le_bytes());
        assert_eq!(
            transfer.accept_response(&abort),
            Err(SdoError::Abort(0x0602_0000))
        );
        assert_eq!(transfer.phase(), SdoPhase::Aborted);
        assert_eq!(transfer.abort_code(), Some(0x0602_0000));
        assert!(transfer.request().is_none());
    }

    // Independent wire vectors follow the CoE SDO layouts in SANMOTION R 3E
    // EtherCAT M0011697C, section 2 (figures 5/6, pages 2-21/2-22):
    // https://www.sanyodenki.com/global/america/file/SANMOTION_R_3E_EtherCAT_Servo_M0011697C.pdf
    // Header bytes and unused-byte fields are literal, not codec-generated.
    #[test]
    fn upload_initiate_is_the_full_eight_byte_sdo_request() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        assert_eq!(
            transfer.request(),
            Some(&[0, 0x20, 0x40, 0, 0x20, 1, 0, 0, 0, 0][..])
        );
    }

    #[test]
    fn normal_initiate_upload_carries_all_initial_data_including_empty_objects() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let full = [0, 0x30, 0x41, 0, 0x20, 1, 5, 0, 0, 0, 1, 2, 3, 4, 5];
        assert_eq!(transfer.accept_response(&full), Ok(SdoProgress::Complete));
        assert_eq!(transfer.data(), &[1, 2, 3, 4, 5]);
        assert_eq!(transfer.expected_size(), Some(5));
        assert_eq!(transfer.request(), None);

        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let empty = [0, 0x30, 0x41, 0, 0x20, 1, 0, 0, 0, 0];
        assert_eq!(transfer.accept_response(&empty), Ok(SdoProgress::Complete));
        assert_eq!(transfer.data(), &[]);
        assert_eq!(transfer.expected_size(), Some(0));
    }

    #[test]
    fn normal_initiate_upload_stages_partial_data_before_segments() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let partial = [0, 0x30, 0x41, 0, 0x20, 1, 10, 0, 0, 0, 1, 2, 3];
        assert_eq!(
            transfer.accept_response(&partial),
            Ok(SdoProgress::Advanced)
        );
        assert_eq!(transfer.data(), &[]);
        assert_eq!(transfer.data_len(), 0);
        assert_eq!(
            transfer.request(),
            Some(&[0, 0x20, 0x60, 0, 0, 0, 0, 0, 0, 0][..])
        );
        let final_segment = [0, 0x30, 1, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(
            transfer.accept_response(&final_segment),
            Ok(SdoProgress::Complete)
        );
        assert_eq!(transfer.data(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn normal_download_initiate_carries_data_and_zero_length_uses_normal() {
        let mut transfer = SdoTransfer::new();
        transfer
            .start_download(0x2000, 1, &[1, 2, 3, 4, 5], SdoAccess::Single)
            .unwrap();
        assert_eq!(
            transfer.request(),
            Some(&[0, 0x20, 0x21, 0, 0x20, 1, 5, 0, 0, 0, 1, 2, 3, 4, 5][..])
        );
        assert_eq!(transfer.data(), &[]);
        let ack = [0, 0x30, 0x60, 0, 0x20, 1, 0, 0, 0, 0];
        assert_eq!(transfer.accept_response(&ack), Ok(SdoProgress::Complete));

        transfer
            .start_download(0x2000, 1, &[], SdoAccess::Single)
            .unwrap();
        assert_eq!(
            transfer.request(),
            Some(&[0, 0x20, 0x21, 0, 0x20, 1, 0, 0, 0, 0][..])
        );
        assert_eq!(transfer.accept_response(&ack), Ok(SdoProgress::Complete));
    }

    #[test]
    fn extended_download_segments_and_short_final_padding_match_wire_vectors() {
        let mut transfer =
            SdoTransfer::with_mailbox_payload_capacity(SdoAccessPolicy::new(false), 18).unwrap();
        transfer
            .start_download(
                0x2000,
                1,
                &[
                    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                    23, 24, 25, 26, 27, 28,
                ],
                SdoAccess::Single,
            )
            .unwrap();
        assert_eq!(
            transfer.request(),
            Some(
                &[
                    0, 0x20, 0x21, 0, 0x20, 1, 28, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8
                ][..]
            )
        );
        assert_eq!(
            transfer.accept_response(&[0, 0x30, 0x60, 0, 0x20, 1, 0, 0, 0, 0]),
            Ok(SdoProgress::Advanced)
        );
        assert_eq!(
            transfer.request(),
            Some(
                &[
                    0, 0x20, 0, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23
                ][..]
            )
        );
        assert_eq!(
            transfer.accept_response(&[0, 0x30, 0x20]),
            Ok(SdoProgress::Advanced)
        );
        assert_eq!(
            transfer.request(),
            Some(&[0, 0x20, 0x15, 24, 25, 26, 27, 28, 0, 0][..])
        );
        assert_eq!(
            transfer.accept_response(&[0, 0x30, 0x30]),
            Ok(SdoProgress::Complete)
        );
    }

    #[test]
    fn segment_decoder_distinguishes_extended_data_from_padded_final_data() {
        let extended = [0, 0x30, 1, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let response = SdoResponse::parse(&extended).unwrap();
        assert_eq!(response.data, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(response.last);
        let padded = [0, 0x30, 0x0B, 1, 2, 0, 0, 0, 0, 0];
        assert_eq!(SdoResponse::parse(&padded).unwrap().data, &[1, 2]);
        let empty = [0, 0x30, 0x0F, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(SdoResponse::parse(&empty).unwrap().data, &[]);
        for malformed in [
            &[0, 0x30, 0x0B, 1, 2][..],
            &[0, 0x30, 0x03, 1, 2, 3, 4, 5, 6, 7, 8][..],
            &[0, 0x30, 0x02, 1, 2, 3, 4, 5, 6, 7][..],
            &[0, 0x30, 0x21][..],
        ] {
            assert_eq!(
                SdoResponse::parse(malformed),
                Err(SdoError::SegmentMalformed)
            );
        }
    }

    #[test]
    fn one_kib_object_upload_is_independent_of_the_128_byte_mailbox() {
        let source = core::array::from_fn::<_, 1024, _>(|index| index as u8);
        let mut transfer = BoundedSdoTransfer::<1024>::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let mut initiate = [0; 122];
        initiate[..10].copy_from_slice(&[0, 0x30, 0x41, 0, 0x20, 1, 0, 4, 0, 0]);
        initiate[10..].copy_from_slice(&source[..112]);
        assert_eq!(
            transfer.accept_response(&initiate),
            Ok(SdoProgress::Advanced)
        );
        assert_eq!(transfer.expected_size(), Some(1024));
        assert_eq!(transfer.data(), &[]);
        let mut offset = 112;
        for ordinal in 0..8 {
            let count = if ordinal == 7 { 79 } else { 119 };
            let mut segment = [0; 122];
            segment[..3].copy_from_slice(&[
                0,
                0x30,
                if ordinal % 2 != 0 { 0x10 } else { 0 } | if ordinal == 7 { 1 } else { 0 },
            ]);
            segment[3..3 + count].copy_from_slice(&source[offset..offset + count]);
            let progress = transfer.accept_response(&segment[..3 + count]);
            if ordinal == 7 {
                assert_eq!(progress, Ok(SdoProgress::Complete));
            } else {
                assert_eq!(progress, Ok(SdoProgress::Advanced));
                assert_eq!(transfer.data(), &[]);
            }
            offset += count;
        }
        assert_eq!(offset, 1024);
        assert_eq!(transfer.data(), source);
    }

    #[test]
    fn one_kib_download_has_initial_data_and_eight_bounded_segments() {
        let source = core::array::from_fn::<_, 1024, _>(|index| index as u8);
        let mut transfer = BoundedSdoTransfer::<1024>::new();
        transfer
            .start_download(0x2000, 1, &source, SdoAccess::Single)
            .unwrap();
        let initiate = transfer.request().unwrap();
        assert_eq!(initiate.len(), 122);
        assert_eq!(&initiate[..10], &[0, 0x20, 0x21, 0, 0x20, 1, 0, 4, 0, 0]);
        assert_eq!(&initiate[10..], &source[..112]);
        assert_eq!(
            transfer.accept_response(&[0, 0x30, 0x60, 0, 0x20, 1]),
            Ok(SdoProgress::Advanced)
        );
        let mut offset = 112;
        for ordinal in 0..8 {
            let count = if ordinal == 7 { 79 } else { 119 };
            let segment = transfer.request().unwrap();
            assert_eq!(segment.len(), count + 3);
            assert_eq!(segment[2] & 1 != 0, ordinal == 7);
            assert_eq!(segment[2] & 0x10 != 0, ordinal % 2 != 0);
            assert_eq!(&segment[3..], &source[offset..offset + count]);
            let ack = [0, 0x30, if ordinal % 2 != 0 { 0x30 } else { 0x20 }];
            let progress = transfer.accept_response(&ack);
            assert_eq!(
                progress,
                Ok(if ordinal == 7 {
                    SdoProgress::Complete
                } else {
                    SdoProgress::Advanced
                })
            );
            offset += count;
        }
        assert_eq!(transfer.data(), source);
        assert_eq!(offset, 1024);
    }

    #[test]
    fn known_and_unknown_upload_sizes_reject_overflow_before_publication() {
        let mut transfer = BoundedSdoTransfer::<8>::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let oversized = [0, 0x30, 0x41, 0, 0x20, 1, 9, 0, 0, 0, 1];
        assert_eq!(
            transfer.accept_response(&oversized),
            Err(SdoError::PayloadTooLarge)
        );
        assert_eq!(transfer.data(), &[]);

        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let unknown = [0, 0x30, 0x40, 0, 0x20, 1, 0, 0, 0, 0, 1, 2];
        assert_eq!(
            transfer.accept_response(&unknown),
            Ok(SdoProgress::Advanced)
        );
        assert_eq!(
            transfer.accept_response(&[0, 0x30, 1, 3, 4, 5, 6, 7, 8, 9]),
            Err(SdoError::PayloadTooLarge)
        );
        assert_eq!(transfer.data(), &[]);

        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let exact = [0, 0x30, 0x41, 0, 0x20, 1, 8, 0, 0, 0, 1];
        transfer.accept_response(&exact).unwrap();
        assert_eq!(
            transfer.accept_response(&[0, 0x30, 1, 2, 3, 4, 5, 6, 7, 8]),
            Ok(SdoProgress::Complete)
        );
        assert_eq!(transfer.data(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn announced_size_and_object_identity_are_checked_before_staging_or_abort() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let bad_size = [0, 0x30, 0x41, 0, 0x20, 1, 1, 0, 0, 0, 1, 2];
        assert_eq!(
            transfer.accept_response(&bad_size),
            Err(SdoError::SizeMismatch)
        );
        assert_eq!(transfer.data(), &[]);

        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let wrong_abort = [0, 0x30, 0x80, 1, 0x20, 1, 0, 0, 2, 6];
        assert_eq!(
            transfer.accept_response(&wrong_abort),
            Err(SdoError::IndexMismatch)
        );
        assert_eq!(transfer.abort_code(), None);
    }

    #[test]
    fn transport_timeout_discards_staging_and_late_responses_preserve_terminal_result() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        transfer
            .accept_response(&[0, 0x30, 0x41, 0, 0x20, 1, 8, 0, 0, 0, 1])
            .unwrap();
        assert_eq!(transfer.terminate(SdoError::Timeout), Ok(()));
        assert_eq!(transfer.phase(), SdoPhase::Faulted);
        assert_eq!(transfer.last_error(), Some(SdoError::Timeout));
        assert_eq!(transfer.data(), &[]);
        assert_eq!(transfer.request(), None);
        assert_eq!(
            transfer.accept_response(&[0, 0x30, 1, 2, 3, 4, 5, 6, 7, 8]),
            Err(SdoError::InvalidState)
        );
        assert_eq!(transfer.last_error(), Some(SdoError::Timeout));

        transfer.start_upload(0x2000, 1, SdoAccess::Single).unwrap();
        let full = [0, 0x30, 0x41, 0, 0x20, 1, 1, 0, 0, 0, 42];
        transfer.accept_response(&full).unwrap();
        assert_eq!(
            transfer.terminate(SdoError::Timeout),
            Err(SdoError::InvalidState)
        );
        assert_eq!(transfer.accept_response(&full), Err(SdoError::InvalidState));
        assert_eq!(transfer.phase(), SdoPhase::Complete);
        assert_eq!(transfer.data(), &[42]);
    }

    #[test]
    fn capacity_and_download_overflow_validation_preserve_the_previous_result() {
        assert_eq!(SdoTransfer::new().object_capacity(), 122);
        assert_eq!(SdoTransfer::new().mailbox_payload_capacity(), 122);
        for capacity in [0, 9, 123, 128, usize::MAX] {
            assert!(matches!(
                SdoTransfer::with_mailbox_payload_capacity(SdoAccessPolicy::new(false), capacity),
                Err(SdoError::BufferTooSmall)
            ));
        }
        let mut transfer = BoundedSdoTransfer::<8>::new();
        transfer
            .start_download(0x2000, 1, &[1; 8], SdoAccess::Single)
            .unwrap();
        transfer
            .accept_response(&[0, 0x30, 0x60, 0, 0x20, 1])
            .unwrap();
        assert_eq!(
            transfer.start_download(0x2001, 2, &[2; 9], SdoAccess::Single),
            Err(SdoError::PayloadTooLarge)
        );
        assert_eq!(transfer.phase(), SdoPhase::Complete);
        assert_eq!(transfer.index(), 0x2000);
        assert_eq!(transfer.data(), &[1; 8]);
    }

    #[test]
    fn emergency_payload_is_decoded_without_allocation() {
        let mut payload = [0; COE_EMERGENCY_LEN];
        CoeHeader {
            number: 0,
            service: CoeService::Emergency,
        }
        .encode(&mut payload)
        .unwrap();
        payload[2..4].copy_from_slice(&0x2310u16.to_le_bytes());
        payload[4] = 0x81;
        payload[5..10].copy_from_slice(&[9, 8, 7, 6, 5]);
        assert_eq!(
            CoeEmergency::parse(&payload),
            Ok(CoeEmergency {
                error_code: 0x2310,
                error_register: 0x81,
                manufacturer_data: [9, 8, 7, 6, 5],
            })
        );
    }
}
