//! Allocation-free CoE SDO codec and transfer state machine.
//!
//! The controller only deals with CoE payloads. Mailbox headers, register
//! access, retry policy and cycle budgeting remain owned by `mailbox.rs` and
//! the caller, respectively.

use crate::mailbox::MAX_MAILBOX_BYTES;

pub const COE_HEADER_LEN: usize = 2;
pub const SDO_DATA_OFFSET: usize = 6;
pub const MAX_SDO_DATA: usize = MAX_MAILBOX_BYTES - SDO_DATA_OFFSET;
pub const MAX_SDO_SEGMENT_BYTES: usize = 7;
pub const COE_EMERGENCY_LEN: usize = 10;

const SDO_UPLOAD_REQUEST: u8 = 0x40;
const SDO_DOWNLOAD_EXPEDITED: u8 = 0x23;
const SDO_DOWNLOAD_NORMAL: u8 = 0x21;
const SDO_ABORT: u8 = 0x80;
const SDO_DOWNLOAD_RESPONSE: u8 = 0x60;
const SDO_UPLOAD_SEGMENT_REQUEST: u8 = 0x60;
const SDO_DOWNLOAD_SEGMENT_RESPONSE: u8 = 0x20;

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
            });
        }

        if command & 0xE0 == 0x40 {
            if payload.len() < SDO_DATA_OFFSET {
                return Err(SdoError::Truncated);
            }
            let index = u16::from_le_bytes([payload[3], payload[4]]);
            let subindex = payload[5];
            if command & 0x02 != 0 {
                let data_len = 4 - ((command >> 2) & 0x03) as usize;
                if payload.len() < SDO_DATA_OFFSET + data_len {
                    return Err(SdoError::Truncated);
                }
                return Ok(Self {
                    index: Some(index),
                    subindex: Some(subindex),
                    data: &payload[SDO_DATA_OFFSET..SDO_DATA_OFFSET + data_len],
                    total_size: None,
                    toggle: false,
                    last: true,
                    abort_code: None,
                    command,
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
                data: &[],
                total_size,
                toggle: false,
                last: false,
                abort_code: None,
                command,
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
            });
        }

        if command & 0xE0 == 0 {
            let unused = ((command >> 1) & 0x07) as usize;
            let segment_len = payload.len().saturating_sub(3);
            if unused > segment_len || (command & 0x01 == 0 && unused != 0) {
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
            });
        }

        if command & 0xE0 == SDO_DOWNLOAD_SEGMENT_RESPONSE {
            return Ok(Self {
                index: None,
                subindex: None,
                data: &[],
                total_size: None,
                toggle: command & 0x10 != 0,
                last: true,
                abort_code: None,
                command,
            });
        }

        Err(SdoError::UnexpectedCommand)
    }
}

pub struct SdoTransfer {
    phase: SdoPhase,
    direction: SdoDirection,
    index: u16,
    subindex: u8,
    complete_access: bool,
    data: [u8; MAX_SDO_DATA],
    data_len: usize,
    expected_size: Option<usize>,
    offset: usize,
    toggle: bool,
    pending: [u8; MAX_MAILBOX_BYTES],
    pending_len: usize,
    abort_code: Option<u32>,
    last_error: Option<SdoError>,
}

impl SdoTransfer {
    pub const fn new() -> Self {
        Self {
            phase: SdoPhase::Idle,
            direction: SdoDirection::Upload,
            index: 0,
            subindex: 0,
            complete_access: false,
            data: [0; MAX_SDO_DATA],
            data_len: 0,
            expected_size: None,
            offset: 0,
            toggle: false,
            pending: [0; MAX_MAILBOX_BYTES],
            pending_len: 0,
            abort_code: None,
            last_error: None,
        }
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

    pub const fn data_len(&self) -> usize {
        self.data_len
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.data_len]
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
        complete_access: bool,
    ) -> Result<(), SdoError> {
        self.begin(SdoDirection::Upload, index, subindex, complete_access, &[])?;
        self.build_upload_initiate();
        Ok(())
    }

    pub fn start_download(
        &mut self,
        index: u16,
        subindex: u8,
        data: &[u8],
        complete_access: bool,
    ) -> Result<(), SdoError> {
        self.begin(
            SdoDirection::Download,
            index,
            subindex,
            complete_access,
            data,
        )?;
        if data.len() <= 4 {
            self.build_download_expedited();
        } else {
            self.build_download_initiate();
        }
        Ok(())
    }

    pub fn accept_response(&mut self, payload: &[u8]) -> Result<SdoProgress, SdoError> {
        if !matches!(self.phase, SdoPhase::AwaitInitiate | SdoPhase::AwaitSegment) {
            return self.fail(SdoError::InvalidState);
        }
        let response = match SdoResponse::parse(payload) {
            Ok(response) => response,
            Err(error) => return self.fail(error),
        };
        if let Some(code) = response.abort_code {
            self.phase = SdoPhase::Aborted;
            self.pending_len = 0;
            self.abort_code = Some(code);
            self.last_error = Some(SdoError::Abort(code));
            return Err(SdoError::Abort(code));
        }

        let result = match (self.direction, self.phase) {
            (SdoDirection::Upload, SdoPhase::AwaitInitiate) => {
                if let Err(error) = self.check_object(response.index, response.subindex) {
                    return self.fail(error);
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
                        if expected_size > MAX_SDO_DATA {
                            return self.fail(SdoError::PayloadTooLarge);
                        }
                        self.expected_size = Some(expected_size);
                    } else {
                        self.expected_size = None;
                    }
                    self.offset = 0;
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
                if self.data_len <= 4 {
                    self.complete();
                    Ok(SdoProgress::Complete)
                } else {
                    self.offset = 0;
                    self.toggle = false;
                    self.build_download_segment();
                    Ok(SdoProgress::Advanced)
                }
            }
            (SdoDirection::Upload, SdoPhase::AwaitSegment) => {
                if response.index.is_some() || response.subindex.is_some() {
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
                if response.command & 0xE0 != SDO_DOWNLOAD_SEGMENT_RESPONSE
                    || response.toggle != self.toggle
                {
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

    fn begin(
        &mut self,
        direction: SdoDirection,
        index: u16,
        subindex: u8,
        complete_access: bool,
        data: &[u8],
    ) -> Result<(), SdoError> {
        if !matches!(
            self.phase,
            SdoPhase::Idle | SdoPhase::Complete | SdoPhase::Aborted | SdoPhase::Faulted
        ) {
            return Err(SdoError::Busy);
        }
        if data.len() > MAX_SDO_DATA {
            return Err(SdoError::PayloadTooLarge);
        }
        self.phase = SdoPhase::AwaitInitiate;
        self.direction = direction;
        self.index = index;
        self.subindex = subindex;
        self.complete_access = complete_access;
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
        let command = SDO_UPLOAD_REQUEST | if self.complete_access { 0x80 } else { 0 };
        self.write_common_header(CoeService::SdoRequest, command);
        self.pending_len = SDO_DATA_OFFSET;
    }

    fn build_download_expedited(&mut self) {
        let unused = 4 - self.data_len;
        let command = SDO_DOWNLOAD_EXPEDITED
            | ((unused as u8) << 2)
            | if self.complete_access { 0x80 } else { 0 };
        self.write_common_header(CoeService::SdoRequest, command);
        self.pending[SDO_DATA_OFFSET..SDO_DATA_OFFSET + 4].fill(0);
        self.pending[SDO_DATA_OFFSET..SDO_DATA_OFFSET + self.data_len]
            .copy_from_slice(&self.data[..self.data_len]);
        self.pending_len = SDO_DATA_OFFSET + 4;
    }

    fn build_download_initiate(&mut self) {
        let command = SDO_DOWNLOAD_NORMAL | if self.complete_access { 0x80 } else { 0 };
        self.write_common_header(CoeService::SdoRequest, command);
        self.pending[SDO_DATA_OFFSET..SDO_DATA_OFFSET + 4]
            .copy_from_slice(&(self.data_len as u32).to_le_bytes());
        self.pending_len = SDO_DATA_OFFSET + 4;
    }

    fn build_upload_segment_request(&mut self) {
        let command = SDO_UPLOAD_SEGMENT_REQUEST | if self.toggle { 0x10 } else { 0 };
        let _ = (CoeHeader {
            number: 0,
            service: CoeService::SdoRequest,
        })
        .encode(&mut self.pending[..COE_HEADER_LEN]);
        self.pending[2] = command;
        self.pending_len = 3;
        self.phase = SdoPhase::AwaitSegment;
    }

    fn build_download_segment(&mut self) {
        let remaining = self.data_len - self.offset;
        let segment_len = remaining.min(MAX_SDO_SEGMENT_BYTES);
        let last = segment_len == remaining;
        let unused = if last {
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
        self.pending[3..3 + segment_len]
            .copy_from_slice(&self.data[self.offset..self.offset + segment_len]);
        self.pending_len = 3 + segment_len;
        self.offset += segment_len;
        self.phase = SdoPhase::AwaitSegment;
    }

    fn store_upload_data(&mut self, data: &[u8]) -> Result<(), SdoError> {
        if self.data_len + data.len() > MAX_SDO_DATA {
            return Err(SdoError::BufferTooSmall);
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

impl Default for SdoTransfer {
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
            .start_download(0x6040, 0, &[0x06, 0x00], false)
            .unwrap();
        let request = transfer.request().unwrap();
        assert_eq!(request.len(), 10);
        assert_eq!(request[0..2], [0, 0x20]);
        assert_eq!(request[2], 0x2B);
        assert_eq!(&request[3..6], &[0x40, 0x60, 0]);
        assert_eq!(&request[6..10], &[0x06, 0x00, 0, 0]);
    }

    #[test]
    fn expedited_upload_completes_and_validates_object() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x6064, 0, false).unwrap();
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
        let mut transfer = SdoTransfer::new();
        transfer.start_download(0x2000, 1, &data, false).unwrap();

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
        assert_eq!(&transfer.request().unwrap()[2..], &[0x1B, 8, 9]);

        ack[2] = SDO_DOWNLOAD_SEGMENT_RESPONSE | 0x10;
        assert_eq!(transfer.accept_response(&ack), Ok(SdoProgress::Complete));
        assert_eq!(transfer.phase(), SdoPhase::Complete);
    }

    #[test]
    fn segmented_upload_rejects_wrong_toggle_without_publishing_partial_data() {
        let mut transfer = SdoTransfer::new();
        transfer.start_upload(0x2000, 1, false).unwrap();

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
        transfer.start_upload(0x2000, 1, false).unwrap();

        let mut initiate_response = [0; 6];
        response_header(&mut initiate_response);
        initiate_response[2] = 0x40;
        initiate_response[3..5].copy_from_slice(&0x2000u16.to_le_bytes());
        initiate_response[5] = 1;
        assert_eq!(
            transfer.accept_response(&initiate_response),
            Ok(SdoProgress::Advanced)
        );

        let mut segment = [0; 8];
        response_header(&mut segment);
        segment[2] = 0x01;
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
        transfer.start_upload(0x6041, 0, false).unwrap();

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
