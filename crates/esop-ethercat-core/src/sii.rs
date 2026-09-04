//! Bounded SII/EEPROM identity reader.
//!
//! EEPROM access is a control-plane transaction. The state machine emits one
//! fixed-size ESC register request at a time and never waits or allocates in
//! the PDO cycle.

use crate::control::{ControlError, ControlRequestPool, RegisterOperation, RequestHandle};
use crate::registers::{ESC_EEPROM_ADDRESS, ESC_EEPROM_CONTROL, ESC_EEPROM_DATA, fixed_address};
use crate::slave::SlaveIdentity;

pub const EEPROM_BUSY: u16 = 1 << 15;
pub const EEPROM_ERROR_MASK: u16 = 0x7800;
pub const EEPROM_READ_COMMAND: u16 = 0x0100;
pub const SII_VENDOR_ID_WORD: u16 = 0x0008;
pub const SII_PRODUCT_CODE_WORD: u16 = 0x000A;
pub const SII_REVISION_WORD: u16 = 0x000C;
pub const SII_SERIAL_WORD: u16 = 0x000E;
pub const SII_CATEGORY_STRINGS: u16 = 0x000A;
pub const SII_CATEGORY_GENERAL: u16 = 0x001E;
pub const SII_CATEGORY_FMMU: u16 = 0x0028;
pub const SII_CATEGORY_SYNC_MANAGER: u16 = 0x0029;
pub const SII_CATEGORY_TX_PDO: u16 = 0x0032;
pub const SII_CATEGORY_RX_PDO: u16 = 0x0033;
pub const SII_CATEGORY_DC: u16 = 0x003C;
pub const SII_CATEGORY_END: u16 = 0xFFFF;

const ACTION_PAYLOAD_LEN: usize = 4;
const IDENTITY_WORDS: [u16; 8] = [
    SII_VENDOR_ID_WORD,
    SII_VENDOR_ID_WORD + 1,
    SII_PRODUCT_CODE_WORD,
    SII_PRODUCT_CODE_WORD + 1,
    SII_REVISION_WORD,
    SII_REVISION_WORD + 1,
    SII_SERIAL_WORD,
    SII_SERIAL_WORD + 1,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiPhase {
    Idle,
    WritingAddress,
    IssuingRead,
    Polling,
    ReadingData,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiError {
    Busy,
    NotStarted,
    NoPendingAction,
    TokenMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    EepromError(u16),
    InvalidResponse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiCategoryError {
    TruncatedHeader,
    TruncatedPayload,
    LengthOverflow,
    UnexpectedCategory,
    InvalidSyncManagerLength,
    SyncManagerCountOutOfBounds,
    InvalidPdoHeader,
    InvalidPdoEntry,
    EntryOutOfBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiCategory<'a> {
    pub kind: u16,
    pub offset_words: u16,
    data: &'a [u8],
}

impl<'a> SiiCategory<'a> {
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub const fn word_len(&self) -> usize {
        self.data.len() / 2
    }

    pub fn sync_managers(&self) -> Result<SiiSyncManagerCategory<'a>, SiiCategoryError> {
        if self.kind != SII_CATEGORY_SYNC_MANAGER {
            return Err(SiiCategoryError::UnexpectedCategory);
        }
        if self.data.len() % SII_SYNC_MANAGER_ENTRY_LEN != 0 {
            return Err(SiiCategoryError::InvalidSyncManagerLength);
        }
        Ok(SiiSyncManagerCategory { data: self.data })
    }

    pub fn pdo(&self) -> Result<SiiPdoCategory<'a>, SiiCategoryError> {
        if self.kind != SII_CATEGORY_TX_PDO && self.kind != SII_CATEGORY_RX_PDO {
            return Err(SiiCategoryError::UnexpectedCategory);
        }
        SiiPdoCategory::parse(self.kind, self.data)
    }
}

pub struct SiiCategoryReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    finished: bool,
}

impl<'a> SiiCategoryReader<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            finished: false,
        }
    }

    pub const fn offset_words(&self) -> usize {
        self.offset / 2
    }

    pub fn next_category(&mut self) -> Result<Option<SiiCategory<'a>>, SiiCategoryError> {
        if self.finished {
            return Ok(None);
        }
        let header_end = self
            .offset
            .checked_add(SII_CATEGORY_HEADER_LEN)
            .ok_or(SiiCategoryError::LengthOverflow)?;
        if header_end > self.bytes.len() {
            return Err(SiiCategoryError::TruncatedHeader);
        }
        let kind = u16::from_le_bytes([self.bytes[self.offset], self.bytes[self.offset + 1]]);
        let word_len =
            u16::from_le_bytes([self.bytes[self.offset + 2], self.bytes[self.offset + 3]]) as usize;
        self.offset = header_end;
        if kind == SII_CATEGORY_END {
            self.finished = true;
            return Ok(None);
        }
        let payload_len = word_len
            .checked_mul(2)
            .ok_or(SiiCategoryError::LengthOverflow)?;
        let payload_end = self
            .offset
            .checked_add(payload_len)
            .ok_or(SiiCategoryError::LengthOverflow)?;
        if payload_end > self.bytes.len() {
            return Err(SiiCategoryError::TruncatedPayload);
        }
        let offset_words = u16::try_from((self.offset - SII_CATEGORY_HEADER_LEN) / 2)
            .map_err(|_| SiiCategoryError::LengthOverflow)?;
        let category = SiiCategory {
            kind,
            offset_words,
            data: &self.bytes[self.offset..payload_end],
        };
        self.offset = payload_end;
        Ok(Some(category))
    }
}

const SII_CATEGORY_HEADER_LEN: usize = 4;
const SII_SYNC_MANAGER_ENTRY_LEN: usize = 8;
const SII_PDO_HEADER_LEN: usize = 8;
const SII_PDO_ENTRY_LEN: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiSyncManager {
    pub start_address: u16,
    pub length: u16,
    pub control: u8,
    pub status: u8,
    pub enable: u8,
}

pub struct SiiSyncManagerCategory<'a> {
    data: &'a [u8],
}

impl<'a> SiiSyncManagerCategory<'a> {
    pub const fn len(&self) -> usize {
        self.data.len() / SII_SYNC_MANAGER_ENTRY_LEN
    }

    pub const fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn get(&self, index: usize) -> Result<SiiSyncManager, SiiCategoryError> {
        let offset = index
            .checked_mul(SII_SYNC_MANAGER_ENTRY_LEN)
            .ok_or(SiiCategoryError::EntryOutOfBounds)?;
        let end = offset
            .checked_add(SII_SYNC_MANAGER_ENTRY_LEN)
            .ok_or(SiiCategoryError::EntryOutOfBounds)?;
        if end > self.data.len() {
            return Err(SiiCategoryError::EntryOutOfBounds);
        }
        Ok(SiiSyncManager {
            start_address: u16::from_le_bytes([self.data[offset], self.data[offset + 1]]),
            length: u16::from_le_bytes([self.data[offset + 2], self.data[offset + 3]]),
            control: self.data[offset + 4],
            status: self.data[offset + 5],
            enable: self.data[offset + 6],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiPdoEntry {
    pub index: u16,
    pub subindex: u8,
    pub bit_length: u8,
    pub name_index: u8,
    pub flags: u8,
}

pub struct SiiPdoCategory<'a> {
    kind: u16,
    data: &'a [u8],
    entry_count: usize,
}

impl<'a> SiiPdoCategory<'a> {
    fn parse(kind: u16, data: &'a [u8]) -> Result<Self, SiiCategoryError> {
        if data.len() < SII_PDO_HEADER_LEN {
            return Err(SiiCategoryError::InvalidPdoHeader);
        }
        let entry_count = data[2] as usize;
        let entries_len = entry_count
            .checked_mul(SII_PDO_ENTRY_LEN)
            .ok_or(SiiCategoryError::LengthOverflow)?;
        let required = SII_PDO_HEADER_LEN
            .checked_add(entries_len)
            .ok_or(SiiCategoryError::LengthOverflow)?;
        if required > data.len() {
            return Err(SiiCategoryError::InvalidPdoHeader);
        }
        Ok(Self {
            kind,
            data,
            entry_count,
        })
    }

    pub const fn kind(&self) -> u16 {
        self.kind
    }

    pub const fn index(&self) -> u16 {
        u16::from_le_bytes([self.data[0], self.data[1]])
    }

    pub const fn sync_manager(&self) -> u8 {
        self.data[3]
    }

    pub const fn name_index(&self) -> u8 {
        self.data[4]
    }

    pub const fn entry_count(&self) -> usize {
        self.entry_count
    }

    pub fn entry(&self, index: usize) -> Result<SiiPdoEntry, SiiCategoryError> {
        if index >= self.entry_count {
            return Err(SiiCategoryError::EntryOutOfBounds);
        }
        let offset = SII_PDO_HEADER_LEN + index * SII_PDO_ENTRY_LEN;
        if offset + SII_PDO_ENTRY_LEN > self.data.len() {
            return Err(SiiCategoryError::InvalidPdoEntry);
        }
        Ok(SiiPdoEntry {
            index: u16::from_le_bytes([self.data[offset], self.data[offset + 1]]),
            subindex: self.data[offset + 2],
            name_index: self.data[offset + 3],
            bit_length: self.data[offset + 4],
            flags: self.data[offset + 5],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiProgress {
    Advanced,
    WordRead(u16),
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiAction {
    pub token: u8,
    pub datagram_index: u8,
    pub generation: u16,
    pub station_address: u16,
    pub word_address: u16,
    pub operation: RegisterOperation,
    pub address: u32,
    pub read_len: u16,
    pub write_payload: [u8; ACTION_PAYLOAD_LEN],
    pub write_len: u8,
    pub deadline_ns: u64,
    pub expected_wkc: u16,
}

impl SiiAction {
    pub fn payload(&self) -> &[u8] {
        &self.write_payload[..self.write_len as usize]
    }

    pub const fn datagram_len(&self) -> usize {
        let read_len = self.read_len as usize;
        let write_len = self.write_len as usize;
        if read_len > write_len {
            read_len
        } else {
            write_len
        }
    }
}

pub struct SiiIdentityReader {
    phase: SiiPhase,
    generation: u16,
    station_address: u16,
    scan_deadline_ns: u64,
    request_timeout_ns: u64,
    word_index: usize,
    words: [u16; 8],
    pending: Option<SiiAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<SiiError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiBlockRequest {
    pub station_address: u16,
    pub start_word: u16,
    pub word_count: usize,
    pub generation: u16,
    pub now_ns: u64,
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
}

/// Fixed-capacity asynchronous reader for an arbitrary contiguous SII word
/// range. It is the control-plane bridge between ESC EEPROM access and the
/// zero-copy [`SiiCategoryReader`] parser.
pub struct SiiBlockReader<const WORDS: usize> {
    phase: SiiPhase,
    generation: u16,
    station_address: u16,
    start_word: u16,
    word_count: usize,
    scan_deadline_ns: u64,
    request_timeout_ns: u64,
    word_index: usize,
    words: [u16; WORDS],
    pending: Option<SiiAction>,
    next_token: u8,
    next_datagram_index: u8,
    last_error: Option<SiiBlockError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiBlockError {
    Busy,
    NotStarted,
    InvalidWordCount,
    AddressOverflow,
    NoPendingAction,
    TokenMismatch,
    GenerationMismatch,
    PayloadLengthMismatch,
    UnexpectedWorkingCounter,
    Timeout,
    EepromError(u16),
    InvalidResponse,
    BufferTooSmall,
    NotComplete,
}

impl<const WORDS: usize> SiiBlockReader<WORDS> {
    pub const fn new() -> Self {
        Self {
            phase: SiiPhase::Idle,
            generation: 0,
            station_address: 0,
            start_word: 0,
            word_count: 0,
            scan_deadline_ns: 0,
            request_timeout_ns: 0,
            word_index: 0,
            words: [0; WORDS],
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> SiiPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<SiiAction> {
        self.pending
    }

    pub const fn word_count(&self) -> usize {
        self.word_count
    }

    pub const fn last_error(&self) -> Option<SiiBlockError> {
        self.last_error
    }

    pub fn words(&self) -> Option<&[u16]> {
        if self.phase == SiiPhase::Complete {
            Some(&self.words[..self.word_count])
        } else {
            None
        }
    }

    /// Copy the completed EEPROM range into a caller-owned byte buffer in
    /// little-endian word order, ready for [`SiiCategoryReader::new`].
    pub fn copy_bytes(&self, destination: &mut [u8]) -> Result<usize, SiiBlockError> {
        if self.phase != SiiPhase::Complete {
            return Err(SiiBlockError::NotComplete);
        }
        let byte_len = self
            .word_count
            .checked_mul(2)
            .ok_or(SiiBlockError::BufferTooSmall)?;
        if destination.len() < byte_len {
            return Err(SiiBlockError::BufferTooSmall);
        }
        for (index, word) in self.words[..self.word_count].iter().copied().enumerate() {
            let offset = index * 2;
            destination[offset..offset + 2].copy_from_slice(&word.to_le_bytes());
        }
        Ok(byte_len)
    }

    pub fn start(&mut self, request: SiiBlockRequest) -> Result<(), SiiBlockError> {
        if !matches!(
            self.phase,
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted
        ) {
            return Err(SiiBlockError::Busy);
        }
        if WORDS == 0 || request.word_count == 0 || request.word_count > WORDS {
            return Err(SiiBlockError::InvalidWordCount);
        }
        let last_offset =
            u16::try_from(request.word_count - 1).map_err(|_| SiiBlockError::InvalidWordCount)?;
        request
            .start_word
            .checked_add(last_offset)
            .ok_or(SiiBlockError::AddressOverflow)?;

        self.phase = SiiPhase::WritingAddress;
        self.generation = request.generation;
        self.station_address = request.station_address;
        self.start_word = request.start_word;
        self.word_count = request.word_count;
        self.scan_deadline_ns = request.now_ns.saturating_add(request.timeout_ns);
        self.request_timeout_ns = request.request_timeout_ns;
        self.word_index = 0;
        self.words = [0; WORDS];
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<SiiAction>, SiiBlockError> {
        if self.phase == SiiPhase::Idle {
            return Err(SiiBlockError::NotStarted);
        }
        if matches!(self.phase, SiiPhase::Complete | SiiPhase::Faulted) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.scan_deadline_ns {
            return self.fail(SiiBlockError::Timeout);
        }
        let offset = match u16::try_from(self.word_index) {
            Ok(offset) => offset,
            Err(_) => return self.fail(SiiBlockError::InvalidWordCount),
        };
        let word_address = match self.start_word.checked_add(offset) {
            Some(word_address) => word_address,
            None => return self.fail(SiiBlockError::AddressOverflow),
        };
        let (operation, address, read_len, payload, write_len) = match self.phase {
            SiiPhase::WritingAddress => (
                RegisterOperation::Write,
                fixed_address(self.station_address, ESC_EEPROM_ADDRESS),
                0,
                (word_address as u32).to_le_bytes(),
                4,
            ),
            SiiPhase::IssuingRead => (
                RegisterOperation::Write,
                fixed_address(self.station_address, ESC_EEPROM_CONTROL),
                0,
                [
                    EEPROM_READ_COMMAND as u8,
                    (EEPROM_READ_COMMAND >> 8) as u8,
                    0,
                    0,
                ],
                2,
            ),
            SiiPhase::Polling => (
                RegisterOperation::Read,
                fixed_address(self.station_address, ESC_EEPROM_CONTROL),
                2,
                [0; ACTION_PAYLOAD_LEN],
                0,
            ),
            SiiPhase::ReadingData => (
                RegisterOperation::Read,
                fixed_address(self.station_address, ESC_EEPROM_DATA),
                2,
                [0; ACTION_PAYLOAD_LEN],
                0,
            ),
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted => return Ok(None),
        };
        let deadline_ns = now_ns
            .saturating_add(self.request_timeout_ns)
            .min(self.scan_deadline_ns);
        let action = SiiAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            word_address,
            operation,
            address,
            read_len,
            write_payload: payload,
            write_len,
            deadline_ns,
            expected_wkc: 1,
        };
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.next_datagram_index = self.next_datagram_index.wrapping_add(1).max(1);
        self.pending = Some(action);
        Ok(Some(action))
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, ControlError> {
        let action = self.pending.ok_or(ControlError::InvalidState)?;
        pool.acquire_with_response_len(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
        )
    }

    pub fn accept(
        &mut self,
        token: u8,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<SiiProgress, SiiBlockError> {
        let action = self.pending.ok_or(SiiBlockError::NoPendingAction)?;
        if action.token != token {
            return Err(SiiBlockError::TokenMismatch);
        }
        if action.generation != generation {
            return self.fail(SiiBlockError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            return self.fail(SiiBlockError::Timeout);
        }
        if working_counter != action.expected_wkc {
            return self.fail(SiiBlockError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.read_len as usize {
            return self.fail(SiiBlockError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            SiiPhase::WritingAddress => {
                self.phase = SiiPhase::IssuingRead;
                SiiProgress::Advanced
            }
            SiiPhase::IssuingRead => {
                self.phase = SiiPhase::Polling;
                SiiProgress::Advanced
            }
            SiiPhase::Polling => {
                let status = u16::from_le_bytes([payload[0], payload[1]]);
                if status & EEPROM_ERROR_MASK != 0 {
                    return self.fail(SiiBlockError::EepromError(status));
                }
                if status & EEPROM_BUSY != 0 {
                    SiiProgress::Advanced
                } else {
                    self.phase = SiiPhase::ReadingData;
                    SiiProgress::Advanced
                }
            }
            SiiPhase::ReadingData => {
                self.words[self.word_index] = u16::from_le_bytes([payload[0], payload[1]]);
                let word = action.word_address;
                self.word_index += 1;
                if self.word_index == self.word_count {
                    self.phase = SiiPhase::Complete;
                    SiiProgress::Complete
                } else {
                    self.phase = SiiPhase::WritingAddress;
                    SiiProgress::WordRead(word)
                }
            }
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted => {
                return Err(SiiBlockError::InvalidResponse);
            }
        };
        self.pending = None;
        Ok(progress)
    }

    pub fn timeout(&mut self, token: u8, now_ns: u64) -> Result<(), SiiBlockError> {
        let action = self.pending.ok_or(SiiBlockError::NoPendingAction)?;
        if action.token != token {
            return Err(SiiBlockError::TokenMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(SiiBlockError::Timeout);
        }
        self.fail(SiiBlockError::Timeout)
    }

    fn fail<T>(&mut self, error: SiiBlockError) -> Result<T, SiiBlockError> {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = SiiPhase::Faulted;
        Err(error)
    }
}

impl<const WORDS: usize> Default for SiiBlockReader<WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

impl SiiIdentityReader {
    pub const fn new() -> Self {
        Self {
            phase: SiiPhase::Idle,
            generation: 0,
            station_address: 0,
            scan_deadline_ns: 0,
            request_timeout_ns: 0,
            word_index: 0,
            words: [0; 8],
            pending: None,
            next_token: 1,
            next_datagram_index: 1,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> SiiPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<SiiAction> {
        self.pending
    }

    pub const fn last_error(&self) -> Option<SiiError> {
        self.last_error
    }

    pub fn identity(&self) -> Option<SlaveIdentity> {
        if self.phase != SiiPhase::Complete {
            return None;
        }
        Some(SlaveIdentity {
            vendor_id: self.words[0] as u32 | ((self.words[1] as u32) << 16),
            product_code: self.words[2] as u32 | ((self.words[3] as u32) << 16),
            revision: self.words[4] as u32 | ((self.words[5] as u32) << 16),
            serial: self.words[6] as u32 | ((self.words[7] as u32) << 16),
        })
    }

    pub fn start(
        &mut self,
        station_address: u16,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
    ) -> Result<(), SiiError> {
        if !matches!(
            self.phase,
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted
        ) {
            return Err(SiiError::Busy);
        }
        self.phase = SiiPhase::WritingAddress;
        self.generation = generation;
        self.station_address = station_address;
        self.scan_deadline_ns = now_ns.saturating_add(timeout_ns);
        self.request_timeout_ns = request_timeout_ns;
        self.word_index = 0;
        self.words = [0; 8];
        self.pending = None;
        self.next_token = 1;
        self.next_datagram_index = 1;
        self.last_error = None;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<SiiAction>, SiiError> {
        if self.phase == SiiPhase::Idle {
            return Err(SiiError::NotStarted);
        }
        if matches!(self.phase, SiiPhase::Complete | SiiPhase::Faulted) {
            return Ok(None);
        }
        if let Some(action) = self.pending {
            return Ok(Some(action));
        }
        if now_ns >= self.scan_deadline_ns {
            self.fail(SiiError::Timeout);
            return Err(SiiError::Timeout);
        }

        let word_address = IDENTITY_WORDS[self.word_index];
        let (operation, address, read_len, payload, write_len) = match self.phase {
            SiiPhase::WritingAddress => (
                RegisterOperation::Write,
                fixed_address(self.station_address, ESC_EEPROM_ADDRESS),
                0,
                (word_address as u32).to_le_bytes(),
                4,
            ),
            SiiPhase::IssuingRead => (
                RegisterOperation::Write,
                fixed_address(self.station_address, ESC_EEPROM_CONTROL),
                0,
                [
                    EEPROM_READ_COMMAND as u8,
                    (EEPROM_READ_COMMAND >> 8) as u8,
                    0,
                    0,
                ],
                2,
            ),
            SiiPhase::Polling => (
                RegisterOperation::Read,
                fixed_address(self.station_address, ESC_EEPROM_CONTROL),
                2,
                [0; ACTION_PAYLOAD_LEN],
                0,
            ),
            SiiPhase::ReadingData => (
                RegisterOperation::Read,
                fixed_address(self.station_address, ESC_EEPROM_DATA),
                2,
                [0; ACTION_PAYLOAD_LEN],
                0,
            ),
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted => return Ok(None),
        };
        let deadline_ns = now_ns
            .saturating_add(self.request_timeout_ns)
            .min(self.scan_deadline_ns);
        let action = SiiAction {
            token: self.next_token,
            datagram_index: self.next_datagram_index,
            generation: self.generation,
            station_address: self.station_address,
            word_address,
            operation,
            address,
            read_len,
            write_payload: payload,
            write_len,
            deadline_ns,
            expected_wkc: 1,
        };
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.next_datagram_index = self.next_datagram_index.wrapping_add(1).max(1);
        self.pending = Some(action);
        Ok(Some(action))
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, ControlError> {
        let action = self.pending.ok_or(ControlError::InvalidState)?;
        pool.acquire_with_response_len(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
        )
    }

    pub fn accept(
        &mut self,
        token: u8,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<SiiProgress, SiiError> {
        let action = self.pending.ok_or(SiiError::NoPendingAction)?;
        if action.token != token {
            return Err(SiiError::TokenMismatch);
        }
        if action.generation != generation {
            self.fail(SiiError::GenerationMismatch);
            return Err(SiiError::GenerationMismatch);
        }
        if now_ns > action.deadline_ns {
            self.fail(SiiError::Timeout);
            return Err(SiiError::Timeout);
        }
        if working_counter != action.expected_wkc {
            self.fail(SiiError::UnexpectedWorkingCounter);
            return Err(SiiError::UnexpectedWorkingCounter);
        }
        if payload.len() != action.read_len as usize {
            self.fail(SiiError::PayloadLengthMismatch);
            return Err(SiiError::PayloadLengthMismatch);
        }

        let progress = match self.phase {
            SiiPhase::WritingAddress => {
                self.phase = SiiPhase::IssuingRead;
                SiiProgress::Advanced
            }
            SiiPhase::IssuingRead => {
                self.phase = SiiPhase::Polling;
                SiiProgress::Advanced
            }
            SiiPhase::Polling => {
                let status = u16::from_le_bytes([payload[0], payload[1]]);
                if status & EEPROM_ERROR_MASK != 0 {
                    let error = SiiError::EepromError(status);
                    self.fail(error);
                    return Err(error);
                }
                if status & EEPROM_BUSY != 0 {
                    SiiProgress::Advanced
                } else {
                    self.phase = SiiPhase::ReadingData;
                    SiiProgress::Advanced
                }
            }
            SiiPhase::ReadingData => {
                self.words[self.word_index] = u16::from_le_bytes([payload[0], payload[1]]);
                let word = IDENTITY_WORDS[self.word_index];
                self.word_index += 1;
                if self.word_index == IDENTITY_WORDS.len() {
                    self.phase = SiiPhase::Complete;
                    SiiProgress::Complete
                } else {
                    self.phase = SiiPhase::WritingAddress;
                    SiiProgress::WordRead(word)
                }
            }
            SiiPhase::Idle | SiiPhase::Complete | SiiPhase::Faulted => {
                return Err(SiiError::InvalidResponse);
            }
        };
        self.pending = None;
        Ok(progress)
    }

    pub fn timeout(&mut self, token: u8, now_ns: u64) -> Result<(), SiiError> {
        let action = self.pending.ok_or(SiiError::NoPendingAction)?;
        if action.token != token {
            return Err(SiiError::TokenMismatch);
        }
        if now_ns < action.deadline_ns {
            return Err(SiiError::Timeout);
        }
        self.fail(SiiError::Timeout);
        Err(SiiError::Timeout)
    }

    fn fail(&mut self, error: SiiError) {
        self.last_error = Some(error);
        self.pending = None;
        self.phase = SiiPhase::Faulted;
    }
}

impl Default for SiiIdentityReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_category(bytes: &mut std::vec::Vec<u8>, kind: u16, data: &[u8]) {
        assert_eq!(data.len() % 2, 0);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&((data.len() / 2) as u16).to_le_bytes());
        bytes.extend_from_slice(data);
    }

    fn accept_write(reader: &mut SiiIdentityReader, action: SiiAction, now_ns: u64) {
        reader
            .accept(action.token, action.generation, &[], 1, now_ns)
            .unwrap();
    }

    fn accept_poll(reader: &mut SiiIdentityReader, action: SiiAction, busy: bool, now_ns: u64) {
        let status = if busy { EEPROM_BUSY } else { 0 };
        reader
            .accept(
                action.token,
                action.generation,
                &status.to_le_bytes(),
                1,
                now_ns,
            )
            .unwrap();
    }

    fn read_word(reader: &mut SiiIdentityReader, value: u16, now_ns: u64) {
        let address = reader.next_action(now_ns).unwrap().unwrap();
        assert_eq!(address.operation, RegisterOperation::Write);
        assert_eq!(address.address as u16, ESC_EEPROM_ADDRESS);
        accept_write(reader, address, now_ns + 1);

        let issue = reader.next_action(now_ns + 2).unwrap().unwrap();
        assert_eq!(
            issue.payload(),
            &[EEPROM_READ_COMMAND as u8, (EEPROM_READ_COMMAND >> 8) as u8]
        );
        accept_write(reader, issue, now_ns + 3);

        let poll = reader.next_action(now_ns + 4).unwrap().unwrap();
        accept_poll(reader, poll, true, now_ns + 5);
        let poll = reader.next_action(now_ns + 6).unwrap().unwrap();
        accept_poll(reader, poll, false, now_ns + 7);

        let data = reader.next_action(now_ns + 8).unwrap().unwrap();
        reader
            .accept(
                data.token,
                data.generation,
                &value.to_le_bytes(),
                1,
                now_ns + 9,
            )
            .unwrap();
    }

    #[test]
    fn identity_reader_walks_eeprom_busy_poll_and_publishes_identity() {
        let mut reader = SiiIdentityReader::new();
        reader.start(0x1000, 7, 0, 10_000, 100).unwrap();
        read_word(&mut reader, 0x1122, 1);
        read_word(&mut reader, 0x3344, 20);
        read_word(&mut reader, 0x5566, 40);
        read_word(&mut reader, 0x7788, 60);
        read_word(&mut reader, 0x99AA, 80);
        read_word(&mut reader, 0xBBCC, 100);
        read_word(&mut reader, 0xDDEE, 120);
        read_word(&mut reader, 0xFF00, 140);

        assert_eq!(reader.phase(), SiiPhase::Complete);
        assert_eq!(
            reader.identity(),
            Some(SlaveIdentity {
                vendor_id: 0x3344_1122,
                product_code: 0x7788_5566,
                revision: 0xBBCC_99AA,
                serial: 0xFF00_DDEE,
            })
        );
    }

    #[test]
    fn block_reader_reads_a_bounded_range_for_category_parsing() {
        let mut reader = SiiBlockReader::<4>::new();
        reader
            .start(SiiBlockRequest {
                station_address: 0x1000,
                start_word: 0x0100,
                word_count: 2,
                generation: 7,
                now_ns: 0,
                timeout_ns: 10_000,
                request_timeout_ns: 100,
            })
            .unwrap();

        for (offset, value) in [(0u16, 0x1234u16), (1u16, 0xABCDu16)] {
            let address = reader.next_action(1 + offset as u64 * 20).unwrap().unwrap();
            assert_eq!(address.word_address, 0x0100 + offset);
            reader
                .accept(address.token, 7, &[], 1, address.deadline_ns - 1)
                .unwrap();

            let issue = reader
                .next_action(address.deadline_ns - 1)
                .unwrap()
                .unwrap();
            reader
                .accept(issue.token, 7, &[], 1, issue.deadline_ns - 1)
                .unwrap();

            let poll = reader.next_action(issue.deadline_ns - 1).unwrap().unwrap();
            reader
                .accept(
                    poll.token,
                    7,
                    &EEPROM_BUSY.to_le_bytes(),
                    1,
                    poll.deadline_ns - 1,
                )
                .unwrap();
            let poll = reader.next_action(poll.deadline_ns - 1).unwrap().unwrap();
            reader
                .accept(poll.token, 7, &0u16.to_le_bytes(), 1, poll.deadline_ns - 1)
                .unwrap();

            let data = reader.next_action(poll.deadline_ns - 1).unwrap().unwrap();
            reader
                .accept(data.token, 7, &value.to_le_bytes(), 1, data.deadline_ns - 1)
                .unwrap();
        }

        assert_eq!(reader.phase(), SiiPhase::Complete);
        assert_eq!(reader.words(), Some(&[0x1234, 0xABCD][..]));
        let mut bytes = [0; 4];
        assert_eq!(reader.copy_bytes(&mut bytes), Ok(4));
        assert_eq!(bytes, [0x34, 0x12, 0xCD, 0xAB]);
    }

    #[test]
    fn block_reader_rejects_capacity_and_address_overflow_before_starting() {
        let mut reader = SiiBlockReader::<2>::new();
        assert_eq!(
            reader.start(SiiBlockRequest {
                station_address: 0x1000,
                start_word: 0,
                word_count: 3,
                generation: 1,
                now_ns: 0,
                timeout_ns: 100,
                request_timeout_ns: 10,
            }),
            Err(SiiBlockError::InvalidWordCount)
        );
        assert_eq!(
            reader.start(SiiBlockRequest {
                station_address: 0x1000,
                start_word: u16::MAX,
                word_count: 2,
                generation: 1,
                now_ns: 0,
                timeout_ns: 100,
                request_timeout_ns: 10,
            }),
            Err(SiiBlockError::AddressOverflow)
        );
        assert_eq!(reader.phase(), SiiPhase::Idle);
    }

    #[test]
    fn eeprom_error_is_latched_and_not_published() {
        let mut reader = SiiIdentityReader::new();
        reader.start(0x1000, 1, 0, 1_000, 100).unwrap();
        let address = reader.next_action(1).unwrap().unwrap();
        accept_write(&mut reader, address, 2);
        let issue = reader.next_action(3).unwrap().unwrap();
        accept_write(&mut reader, issue, 4);
        let poll = reader.next_action(5).unwrap().unwrap();
        assert_eq!(
            reader.accept(poll.token, 1, &0x0800u16.to_le_bytes(), 1, 6),
            Err(SiiError::EepromError(0x0800))
        );
        assert_eq!(reader.phase(), SiiPhase::Faulted);
        assert_eq!(reader.identity(), None);
    }

    #[test]
    fn category_reader_exposes_sync_managers_and_pdo_entries_without_allocating() {
        let mut bytes = std::vec::Vec::new();
        let sync_managers = [
            0x00, 0x10, 0x20, 0x00, 0x26, 0x64, 0x01, 0x00, 0x00, 0x11, 0x20, 0x00, 0x22, 0x60,
            0x01, 0x00,
        ];
        append_category(&mut bytes, SII_CATEGORY_SYNC_MANAGER, &sync_managers);

        let mut rx_pdo = [0u8; 24];
        rx_pdo[0..2].copy_from_slice(&0x1600u16.to_le_bytes());
        rx_pdo[2] = 2;
        rx_pdo[3] = 2;
        rx_pdo[4] = 1;
        rx_pdo[8..10].copy_from_slice(&0x6040u16.to_le_bytes());
        rx_pdo[10] = 0;
        rx_pdo[11] = 2;
        rx_pdo[12] = 16;
        rx_pdo[16..18].copy_from_slice(&0x607Au16.to_le_bytes());
        rx_pdo[18] = 0;
        rx_pdo[19] = 3;
        rx_pdo[20] = 32;
        append_category(&mut bytes, SII_CATEGORY_RX_PDO, &rx_pdo);
        bytes.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let mut reader = SiiCategoryReader::new(&bytes);
        let category = reader.next_category().unwrap().unwrap();
        assert_eq!(category.kind, SII_CATEGORY_SYNC_MANAGER);
        assert_eq!(category.offset_words, 0);
        let sync = category.sync_managers().unwrap();
        assert_eq!(sync.len(), 2);
        assert_eq!(
            sync.get(0).unwrap(),
            SiiSyncManager {
                start_address: 0x1000,
                length: 0x0020,
                control: 0x26,
                status: 0x64,
                enable: 1,
            }
        );

        let category = reader.next_category().unwrap().unwrap();
        let pdo = category.pdo().unwrap();
        assert_eq!(pdo.kind(), SII_CATEGORY_RX_PDO);
        assert_eq!(pdo.index(), 0x1600);
        assert_eq!(pdo.sync_manager(), 2);
        assert_eq!(pdo.entry_count(), 2);
        assert_eq!(
            pdo.entry(1).unwrap(),
            SiiPdoEntry {
                index: 0x607A,
                subindex: 0,
                bit_length: 32,
                name_index: 3,
                flags: 0,
            }
        );
        assert_eq!(reader.next_category().unwrap(), None);
    }

    #[test]
    fn category_reader_rejects_truncated_and_malformed_categories() {
        let mut truncated = std::vec::Vec::new();
        append_category(&mut truncated, SII_CATEGORY_SYNC_MANAGER, &[0; 6]);
        let mut reader = SiiCategoryReader::new(&truncated);
        let category = reader.next_category().unwrap().unwrap();
        assert!(matches!(
            category.sync_managers(),
            Err(SiiCategoryError::InvalidSyncManagerLength)
        ));

        let malformed = [
            SII_CATEGORY_RX_PDO as u8,
            (SII_CATEGORY_RX_PDO >> 8) as u8,
            2,
            0,
        ];
        let mut reader = SiiCategoryReader::new(&malformed);
        assert_eq!(
            reader.next_category(),
            Err(SiiCategoryError::TruncatedPayload)
        );
    }
}
