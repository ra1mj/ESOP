//! Bounded discovery of the variable-length SII category stream.
//!
//! The reader keeps one fixed image buffer and reuses the existing EEPROM
//! block transaction for each two-word header and payload word. The image is
//! visible only after the standard end category has been accepted.

use crate::control::{ControlError, ControlRequestPool, RequestHandle};
use crate::sii::{
    SII_CATEGORY_END, SiiAction, SiiBlockError, SiiBlockReader, SiiBlockRequest, SiiPhase,
};

pub const SII_CATEGORY_START_WORD: u16 = 0x0040;

const CATEGORY_HEADER_WORDS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiCategoryStreamRequest {
    pub station_address: u16,
    pub start_word: u16,
    pub generation: u16,
    pub now_ns: u64,
    pub timeout_ns: u64,
    pub request_timeout_ns: u64,
}

impl SiiCategoryStreamRequest {
    pub const fn standard(
        station_address: u16,
        generation: u16,
        now_ns: u64,
        timeout_ns: u64,
        request_timeout_ns: u64,
    ) -> Self {
        Self {
            station_address,
            start_word: SII_CATEGORY_START_WORD,
            generation,
            now_ns,
            timeout_ns,
            request_timeout_ns,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiCategoryStreamPhase {
    Idle,
    ReadingHeader,
    ReadingPayload,
    Complete,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiCategoryStreamProgress {
    Advanced,
    CategoryComplete {
        kind: u16,
        payload_words: u16,
        category_count: usize,
    },
    Complete {
        category_count: usize,
        word_count: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiCategoryStreamError {
    Busy,
    NotStarted,
    InvalidCapacity,
    NotComplete,
    BufferTooSmall,
    CategoryCapacityExceeded {
        kind: u16,
        payload_words: u16,
        remaining_words: usize,
    },
    EndMarkerNotFound,
    AddressOverflow,
    Block(SiiBlockError),
    Control(ControlError),
}

pub struct SiiCategoryStreamReader<const IMAGE_WORDS: usize> {
    block: SiiBlockReader<CATEGORY_HEADER_WORDS>,
    phase: SiiCategoryStreamPhase,
    station_address: u16,
    start_word: u16,
    generation: u16,
    scan_deadline_ns: u64,
    request_timeout_ns: u64,
    image: [u16; IMAGE_WORDS],
    image_word_count: usize,
    category_count: usize,
    current_kind: u16,
    current_payload_words: u16,
    remaining_payload_words: usize,
    next_header_word: u16,
    last_error: Option<SiiCategoryStreamError>,
}

impl<const IMAGE_WORDS: usize> SiiCategoryStreamReader<IMAGE_WORDS> {
    pub const fn new() -> Self {
        Self {
            block: SiiBlockReader::new(),
            phase: SiiCategoryStreamPhase::Idle,
            station_address: 0,
            start_word: 0,
            generation: 0,
            scan_deadline_ns: 0,
            request_timeout_ns: 0,
            image: [0; IMAGE_WORDS],
            image_word_count: 0,
            category_count: 0,
            current_kind: 0,
            current_payload_words: 0,
            remaining_payload_words: 0,
            next_header_word: 0,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> SiiCategoryStreamPhase {
        self.phase
    }

    pub const fn pending(&self) -> Option<SiiAction> {
        self.block.pending()
    }

    pub const fn start_word(&self) -> u16 {
        self.start_word
    }

    pub const fn last_error(&self) -> Option<SiiCategoryStreamError> {
        self.last_error
    }

    pub const fn completed_word_count(&self) -> Option<usize> {
        if matches!(self.phase, SiiCategoryStreamPhase::Complete) {
            Some(self.image_word_count)
        } else {
            None
        }
    }

    pub const fn completed_category_count(&self) -> Option<usize> {
        if matches!(self.phase, SiiCategoryStreamPhase::Complete) {
            Some(self.category_count)
        } else {
            None
        }
    }

    pub fn words(&self) -> Option<&[u16]> {
        if self.phase == SiiCategoryStreamPhase::Complete {
            Some(&self.image[..self.image_word_count])
        } else {
            None
        }
    }

    pub fn copy_bytes(&self, destination: &mut [u8]) -> Result<usize, SiiCategoryStreamError> {
        if self.phase != SiiCategoryStreamPhase::Complete {
            return Err(SiiCategoryStreamError::NotComplete);
        }
        let byte_len = self
            .image_word_count
            .checked_mul(2)
            .ok_or(SiiCategoryStreamError::BufferTooSmall)?;
        if destination.len() < byte_len {
            return Err(SiiCategoryStreamError::BufferTooSmall);
        }
        for (index, word) in self.image[..self.image_word_count]
            .iter()
            .copied()
            .enumerate()
        {
            let offset = index * 2;
            destination[offset..offset + 2].copy_from_slice(&word.to_le_bytes());
        }
        Ok(byte_len)
    }

    pub fn start(
        &mut self,
        request: SiiCategoryStreamRequest,
    ) -> Result<(), SiiCategoryStreamError> {
        if !matches!(
            self.phase,
            SiiCategoryStreamPhase::Idle
                | SiiCategoryStreamPhase::Complete
                | SiiCategoryStreamPhase::Faulted
        ) {
            return Err(SiiCategoryStreamError::Busy);
        }
        self.last_error = None;
        if IMAGE_WORDS < CATEGORY_HEADER_WORDS {
            return self.fail(SiiCategoryStreamError::InvalidCapacity);
        }
        if request.start_word.checked_add(1).is_none() {
            return self.fail(SiiCategoryStreamError::AddressOverflow);
        }

        let mut block = SiiBlockReader::new();
        if let Err(error) = block.start(SiiBlockRequest {
            station_address: request.station_address,
            start_word: request.start_word,
            word_count: CATEGORY_HEADER_WORDS,
            generation: request.generation,
            now_ns: request.now_ns,
            timeout_ns: request.timeout_ns,
            request_timeout_ns: request.request_timeout_ns,
        }) {
            return self.fail(SiiCategoryStreamError::Block(error));
        }

        self.block = block;
        self.phase = SiiCategoryStreamPhase::ReadingHeader;
        self.station_address = request.station_address;
        self.start_word = request.start_word;
        self.generation = request.generation;
        self.scan_deadline_ns = request.now_ns.saturating_add(request.timeout_ns);
        self.request_timeout_ns = request.request_timeout_ns;
        self.image = [0; IMAGE_WORDS];
        self.image_word_count = 0;
        self.category_count = 0;
        self.current_kind = 0;
        self.current_payload_words = 0;
        self.remaining_payload_words = 0;
        self.next_header_word = request.start_word;
        self.last_error = None;
        Ok(())
    }

    pub fn next_action(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<SiiAction>, SiiCategoryStreamError> {
        match self.phase {
            SiiCategoryStreamPhase::Idle => return Err(SiiCategoryStreamError::NotStarted),
            SiiCategoryStreamPhase::Complete => return Ok(None),
            SiiCategoryStreamPhase::Faulted => {
                return Err(self
                    .last_error
                    .unwrap_or(SiiCategoryStreamError::NotStarted));
            }
            SiiCategoryStreamPhase::ReadingHeader | SiiCategoryStreamPhase::ReadingPayload => {}
        }
        match self.block.next_action(now_ns) {
            Ok(action) => Ok(action),
            Err(error) => self.fail(SiiCategoryStreamError::Block(error)),
        }
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, SiiCategoryStreamError> {
        self.block
            .enqueue_pending(pool)
            .map_err(SiiCategoryStreamError::Control)
    }

    pub fn accept(
        &mut self,
        token: u8,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<SiiCategoryStreamProgress, SiiCategoryStreamError> {
        if !matches!(
            self.phase,
            SiiCategoryStreamPhase::ReadingHeader | SiiCategoryStreamPhase::ReadingPayload
        ) {
            return Err(self.inactive_error());
        }
        match self
            .block
            .accept(token, generation, payload, working_counter, now_ns)
        {
            Ok(_) if self.block.phase() == SiiPhase::Complete => self.finish_block(now_ns),
            Ok(_) => Ok(SiiCategoryStreamProgress::Advanced),
            Err(error) if self.block.phase() == SiiPhase::Faulted => {
                self.fail(SiiCategoryStreamError::Block(error))
            }
            Err(error) => Err(SiiCategoryStreamError::Block(error)),
        }
    }

    pub fn timeout(&mut self, token: u8, now_ns: u64) -> Result<(), SiiCategoryStreamError> {
        if !matches!(
            self.phase,
            SiiCategoryStreamPhase::ReadingHeader | SiiCategoryStreamPhase::ReadingPayload
        ) {
            return Err(self.inactive_error());
        }
        match self.block.timeout(token, now_ns) {
            Ok(()) => Ok(()),
            Err(error) if self.block.phase() == SiiPhase::Faulted => {
                self.fail(SiiCategoryStreamError::Block(error))
            }
            Err(error) => Err(SiiCategoryStreamError::Block(error)),
        }
    }

    fn finish_block(
        &mut self,
        now_ns: u64,
    ) -> Result<SiiCategoryStreamProgress, SiiCategoryStreamError> {
        match self.phase {
            SiiCategoryStreamPhase::ReadingHeader => self.finish_header(now_ns),
            SiiCategoryStreamPhase::ReadingPayload => self.finish_payload_word(now_ns),
            SiiCategoryStreamPhase::Idle
            | SiiCategoryStreamPhase::Complete
            | SiiCategoryStreamPhase::Faulted => Err(SiiCategoryStreamError::NotComplete),
        }
    }

    fn finish_header(
        &mut self,
        now_ns: u64,
    ) -> Result<SiiCategoryStreamProgress, SiiCategoryStreamError> {
        let header_start = self.block.start_word();
        let Some(header) = self.block.words() else {
            return self.fail(SiiCategoryStreamError::NotComplete);
        };
        let kind = header[0];
        let payload_words = header[1];

        if self.image_word_count + CATEGORY_HEADER_WORDS > IMAGE_WORDS {
            return self.fail(SiiCategoryStreamError::EndMarkerNotFound);
        }
        if kind == SII_CATEGORY_END {
            self.image[self.image_word_count] = kind;
            self.image[self.image_word_count + 1] = payload_words;
            self.image_word_count += CATEGORY_HEADER_WORDS;
            self.phase = SiiCategoryStreamPhase::Complete;
            return Ok(SiiCategoryStreamProgress::Complete {
                category_count: self.category_count,
                word_count: self.image_word_count,
            });
        }

        let payload_len = usize::from(payload_words);
        let Some(required_words) = self
            .image_word_count
            .checked_add(CATEGORY_HEADER_WORDS)
            .and_then(|count| count.checked_add(payload_len))
        else {
            return self.fail(SiiCategoryStreamError::CategoryCapacityExceeded {
                kind,
                payload_words,
                remaining_words: IMAGE_WORDS.saturating_sub(self.image_word_count),
            });
        };
        if required_words > IMAGE_WORDS {
            return self.fail(SiiCategoryStreamError::CategoryCapacityExceeded {
                kind,
                payload_words,
                remaining_words: IMAGE_WORDS
                    .saturating_sub(self.image_word_count + CATEGORY_HEADER_WORDS),
            });
        }

        let Some(next_header) = u32::from(header_start)
            .checked_add(CATEGORY_HEADER_WORDS as u32)
            .and_then(|address| address.checked_add(u32::from(payload_words)))
        else {
            return self.fail(SiiCategoryStreamError::AddressOverflow);
        };
        if next_header >= u32::from(u16::MAX) {
            return self.fail(SiiCategoryStreamError::AddressOverflow);
        }
        self.next_header_word = next_header as u16;
        self.current_kind = kind;
        self.current_payload_words = payload_words;
        self.remaining_payload_words = payload_len;
        self.image[self.image_word_count] = kind;
        self.image[self.image_word_count + 1] = payload_words;
        self.image_word_count += CATEGORY_HEADER_WORDS;

        if payload_len == 0 {
            self.category_count += 1;
            if let Err(error) = self.start_header(self.next_header_word, now_ns) {
                return self.fail(error);
            }
            return Ok(SiiCategoryStreamProgress::CategoryComplete {
                kind,
                payload_words,
                category_count: self.category_count,
            });
        }

        let Some(payload_start) = header_start.checked_add(CATEGORY_HEADER_WORDS as u16) else {
            return self.fail(SiiCategoryStreamError::AddressOverflow);
        };
        self.phase = SiiCategoryStreamPhase::ReadingPayload;
        if let Err(error) = self.continue_block(payload_start, 1, now_ns) {
            return self.fail(error);
        }
        Ok(SiiCategoryStreamProgress::Advanced)
    }

    fn finish_payload_word(
        &mut self,
        now_ns: u64,
    ) -> Result<SiiCategoryStreamProgress, SiiCategoryStreamError> {
        let payload_address = self.block.start_word();
        let Some(word) = self.block.words().and_then(|words| words.first()).copied() else {
            return self.fail(SiiCategoryStreamError::NotComplete);
        };
        self.image[self.image_word_count] = word;
        self.image_word_count += 1;
        self.remaining_payload_words -= 1;

        if self.remaining_payload_words != 0 {
            let Some(next_payload) = payload_address.checked_add(1) else {
                return self.fail(SiiCategoryStreamError::AddressOverflow);
            };
            if let Err(error) = self.continue_block(next_payload, 1, now_ns) {
                return self.fail(error);
            }
            return Ok(SiiCategoryStreamProgress::Advanced);
        }

        self.category_count += 1;
        let progress = SiiCategoryStreamProgress::CategoryComplete {
            kind: self.current_kind,
            payload_words: self.current_payload_words,
            category_count: self.category_count,
        };
        if let Err(error) = self.start_header(self.next_header_word, now_ns) {
            return self.fail(error);
        }
        Ok(progress)
    }

    fn start_header(&mut self, start_word: u16, now_ns: u64) -> Result<(), SiiCategoryStreamError> {
        if IMAGE_WORDS.saturating_sub(self.image_word_count) < CATEGORY_HEADER_WORDS {
            return self.fail(SiiCategoryStreamError::EndMarkerNotFound);
        }
        if start_word.checked_add(1).is_none() {
            return self.fail(SiiCategoryStreamError::AddressOverflow);
        }
        self.phase = SiiCategoryStreamPhase::ReadingHeader;
        self.continue_block(start_word, CATEGORY_HEADER_WORDS, now_ns)
    }

    fn inactive_error(&self) -> SiiCategoryStreamError {
        match self.phase {
            SiiCategoryStreamPhase::Idle => SiiCategoryStreamError::NotStarted,
            SiiCategoryStreamPhase::Faulted => self
                .last_error
                .unwrap_or(SiiCategoryStreamError::NotStarted),
            SiiCategoryStreamPhase::Complete
            | SiiCategoryStreamPhase::ReadingHeader
            | SiiCategoryStreamPhase::ReadingPayload => SiiCategoryStreamError::NotComplete,
        }
    }

    fn continue_block(
        &mut self,
        start_word: u16,
        word_count: usize,
        now_ns: u64,
    ) -> Result<(), SiiCategoryStreamError> {
        if now_ns >= self.scan_deadline_ns {
            return self.fail(SiiCategoryStreamError::Block(SiiBlockError::Timeout));
        }
        let timeout_ns = self.scan_deadline_ns - now_ns;
        let request = SiiBlockRequest {
            station_address: self.station_address,
            start_word,
            word_count,
            generation: self.generation,
            now_ns,
            timeout_ns,
            request_timeout_ns: self.request_timeout_ns,
        };
        match self.block.continue_with(request) {
            Ok(()) => Ok(()),
            Err(error) => self.fail(SiiCategoryStreamError::Block(error)),
        }
    }

    fn fail<T>(&mut self, error: SiiCategoryStreamError) -> Result<T, SiiCategoryStreamError> {
        let terminal = self.last_error.unwrap_or(error);
        self.last_error = Some(terminal);
        self.phase = SiiCategoryStreamPhase::Faulted;
        Err(terminal)
    }
}

impl<const IMAGE_WORDS: usize> Default for SiiCategoryStreamReader<IMAGE_WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{ControlError, ControlRequestPool};
    use crate::registers::{ESC_EEPROM_CONTROL, ESC_EEPROM_DATA, register_from_address};
    use std::vec::Vec;

    fn request(
        start_word: u16,
        generation: u16,
        timeout_ns: u64,
        request_timeout_ns: u64,
    ) -> SiiCategoryStreamRequest {
        SiiCategoryStreamRequest {
            station_address: 0x1000,
            start_word,
            generation,
            now_ns: 0,
            timeout_ns,
            request_timeout_ns,
        }
    }

    fn response(action: SiiAction, start_word: u16, image: &[u16]) -> Vec<u8> {
        if action.read_len == 0 {
            return Vec::new();
        }
        match register_from_address(action.address) {
            ESC_EEPROM_CONTROL => 0u16.to_le_bytes().to_vec(),
            ESC_EEPROM_DATA => {
                let offset = usize::from(action.word_address - start_word);
                image[offset].to_le_bytes().to_vec()
            }
            register => panic!("unexpected EEPROM register {register:#06x}"),
        }
    }

    fn drive_to_completion<const WORDS: usize>(
        reader: &mut SiiCategoryStreamReader<WORDS>,
        start_word: u16,
        image: &[u16],
    ) -> (Vec<SiiAction>, Vec<SiiCategoryStreamProgress>) {
        let mut actions = Vec::new();
        let mut progress = Vec::new();
        let mut now_ns = 1;
        while reader.phase() != SiiCategoryStreamPhase::Complete {
            assert!(reader.words().is_none());
            assert_eq!(reader.completed_word_count(), None);
            assert_eq!(reader.completed_category_count(), None);
            let action = reader.next_action(now_ns).unwrap().unwrap();
            let payload = response(action, start_word, image);
            actions.push(action);
            progress.push(
                reader
                    .accept(
                        action.token,
                        action.generation,
                        &payload,
                        action.expected_wkc,
                        now_ns,
                    )
                    .unwrap(),
            );
            now_ns += 1;
        }
        (actions, progress)
    }

    #[test]
    fn discovers_unknown_and_zero_length_categories_through_end() {
        let image = [0x1234, 2, 0xAAAA, 0xBBBB, 0x5678, 0, SII_CATEGORY_END, 0];
        let mut reader = SiiCategoryStreamReader::<8>::new();
        reader.start(request(0x0040, 7, 1_000, 20)).unwrap();

        let (_, progress) = drive_to_completion(&mut reader, 0x0040, &image);

        assert_eq!(reader.words(), Some(&image[..]));
        assert_eq!(reader.completed_word_count(), Some(image.len()));
        assert_eq!(reader.completed_category_count(), Some(2));
        assert!(
            progress.contains(&SiiCategoryStreamProgress::CategoryComplete {
                kind: 0x1234,
                payload_words: 2,
                category_count: 1,
            })
        );
        assert!(
            progress.contains(&SiiCategoryStreamProgress::CategoryComplete {
                kind: 0x5678,
                payload_words: 0,
                category_count: 2,
            })
        );
        assert_eq!(
            progress.last(),
            Some(&SiiCategoryStreamProgress::Complete {
                category_count: 2,
                word_count: image.len(),
            })
        );
        let mut bytes = [0; 16];
        assert_eq!(reader.copy_bytes(&mut bytes), Ok(16));
        assert_eq!(bytes[0..4], [0x34, 0x12, 0x02, 0x00]);
    }

    #[test]
    fn continuation_preserves_action_cursors_and_absolute_deadline() {
        let image = [0x1234, 0, SII_CATEGORY_END, 0];
        let mut reader = SiiCategoryStreamReader::<4>::new();
        reader.start(request(0x0040, 7, 50, 100)).unwrap();

        let (actions, _) = drive_to_completion(&mut reader, 0x0040, &image);

        assert_eq!(actions.len(), image.len() * 4);
        for (offset, action) in actions.iter().enumerate() {
            assert_eq!(action.token, (offset + 1) as u8);
            assert_eq!(action.datagram_index, (offset + 1) as u8);
            assert_eq!(action.deadline_ns, 50);
        }
        assert_eq!(actions[8].word_address, 0x0042);
        assert_eq!(actions[8].token, 9);
    }

    #[test]
    fn rejects_category_capacity_before_requesting_payload() {
        let image = [0x1234, 3];
        let mut reader = SiiCategoryStreamReader::<4>::new();
        reader.start(request(0x0040, 1, 100, 10)).unwrap();

        let mut error = None;
        for now_ns in 1..=8 {
            let action = reader.next_action(now_ns).unwrap().unwrap();
            assert!(action.word_address <= 0x0041);
            let payload = response(action, 0x0040, &image);
            if let Err(next) = reader.accept(
                action.token,
                action.generation,
                &payload,
                action.expected_wkc,
                now_ns,
            ) {
                error = Some(next);
                break;
            }
        }

        let expected = SiiCategoryStreamError::CategoryCapacityExceeded {
            kind: 0x1234,
            payload_words: 3,
            remaining_words: 2,
        };
        assert_eq!(error, Some(expected));
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::Faulted);
        assert_eq!(reader.last_error(), Some(expected));
        assert!(reader.words().is_none());
    }

    #[test]
    fn faults_when_capacity_is_exhausted_without_end_marker() {
        let image = [0x1234, 2, 0xAAAA, 0xBBBB];
        let mut reader = SiiCategoryStreamReader::<4>::new();
        reader.start(request(0x0040, 1, 100, 10)).unwrap();

        let mut error = None;
        for now_ns in 1..=16 {
            let action = reader.next_action(now_ns).unwrap().unwrap();
            let payload = response(action, 0x0040, &image);
            if let Err(next) = reader.accept(
                action.token,
                action.generation,
                &payload,
                action.expected_wkc,
                now_ns,
            ) {
                error = Some(next);
                break;
            }
        }

        assert_eq!(error, Some(SiiCategoryStreamError::EndMarkerNotFound));
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::Faulted);
        assert!(reader.words().is_none());
    }

    #[test]
    fn latches_address_overflow_at_start_and_between_headers() {
        let mut invalid = SiiCategoryStreamReader::<2>::new();
        assert_eq!(
            invalid.start(request(u16::MAX, 1, 100, 10)),
            Err(SiiCategoryStreamError::AddressOverflow)
        );
        assert_eq!(invalid.phase(), SiiCategoryStreamPhase::Faulted);

        let image = [0x1234, 0];
        let mut reader = SiiCategoryStreamReader::<4>::new();
        reader.start(request(u16::MAX - 1, 1, 100, 10)).unwrap();
        let mut error = None;
        for now_ns in 1..=8 {
            let action = reader.next_action(now_ns).unwrap().unwrap();
            let payload = response(action, u16::MAX - 1, &image);
            if let Err(next) = reader.accept(
                action.token,
                action.generation,
                &payload,
                action.expected_wkc,
                now_ns,
            ) {
                error = Some(next);
                break;
            }
        }
        assert_eq!(error, Some(SiiCategoryStreamError::AddressOverflow));
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::Faulted);
    }

    #[test]
    fn request_pool_keeps_the_pending_action_ownership() {
        let mut reader = SiiCategoryStreamReader::<2>::new();
        reader.start(request(0x0040, 9, 100, 10)).unwrap();
        let action = reader.next_action(1).unwrap().unwrap();
        let mut pool = ControlRequestPool::<1>::new();

        let handle = reader.enqueue_pending(&mut pool).unwrap();
        let owned = pool.get(handle).unwrap();
        assert!(owned.matches_action(
            action.datagram_index,
            action.generation,
            action.address,
            action.operation,
            action.payload(),
            action.datagram_len(),
            action.deadline_ns,
        ));
        assert_eq!(
            reader.enqueue_pending(&mut pool),
            Err(SiiCategoryStreamError::Control(ControlError::SlotBusy))
        );
        assert_eq!(reader.pending(), Some(action));
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::ReadingHeader);
    }

    #[test]
    fn stale_token_is_retryable_but_generation_mismatch_is_terminal() {
        let mut reader = SiiCategoryStreamReader::<2>::new();
        reader.start(request(0x0040, 9, 100, 10)).unwrap();
        let action = reader.next_action(1).unwrap().unwrap();

        assert_eq!(
            reader.accept(action.token + 1, action.generation, &[], 1, 2),
            Err(SiiCategoryStreamError::Block(SiiBlockError::TokenMismatch))
        );
        assert_eq!(reader.pending(), Some(action));
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::ReadingHeader);
        assert_eq!(
            reader.accept(action.token, action.generation + 1, &[], 1, 2),
            Err(SiiCategoryStreamError::Block(
                SiiBlockError::GenerationMismatch
            ))
        );
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::Faulted);
        assert_eq!(
            reader.next_action(3),
            Err(SiiCategoryStreamError::Block(
                SiiBlockError::GenerationMismatch
            ))
        );
    }

    #[test]
    fn working_counter_and_payload_shape_fail_closed() {
        let mut wkc = SiiCategoryStreamReader::<2>::new();
        wkc.start(request(0x0040, 1, 100, 10)).unwrap();
        let action = wkc.next_action(1).unwrap().unwrap();
        assert_eq!(
            wkc.accept(action.token, action.generation, &[], 0, 2),
            Err(SiiCategoryStreamError::Block(
                SiiBlockError::UnexpectedWorkingCounter
            ))
        );
        assert_eq!(wkc.phase(), SiiCategoryStreamPhase::Faulted);

        let mut payload = SiiCategoryStreamReader::<2>::new();
        payload.start(request(0x0040, 1, 100, 10)).unwrap();
        let action = payload.next_action(1).unwrap().unwrap();
        assert_eq!(
            payload.accept(action.token, action.generation, &[0], 1, 2),
            Err(SiiCategoryStreamError::Block(
                SiiBlockError::PayloadLengthMismatch
            ))
        );
        assert_eq!(payload.phase(), SiiCategoryStreamPhase::Faulted);
    }

    #[test]
    fn early_timeout_keeps_request_pending_and_expiry_latches() {
        let mut reader = SiiCategoryStreamReader::<2>::new();
        reader.start(request(0x0040, 1, 100, 10)).unwrap();
        let action = reader.next_action(1).unwrap().unwrap();
        assert_eq!(action.deadline_ns, 11);

        assert_eq!(
            reader.timeout(action.token, 2),
            Err(SiiCategoryStreamError::Block(SiiBlockError::Timeout))
        );
        assert_eq!(reader.pending(), Some(action));
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::ReadingHeader);
        assert_eq!(
            reader.timeout(action.token, action.deadline_ns),
            Err(SiiCategoryStreamError::Block(SiiBlockError::Timeout))
        );
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::Faulted);
        assert_eq!(
            reader.last_error(),
            Some(SiiCategoryStreamError::Block(SiiBlockError::Timeout))
        );
    }

    #[test]
    fn invalid_capacity_is_terminal_until_explicit_restart() {
        let mut reader = SiiCategoryStreamReader::<1>::new();
        assert_eq!(
            reader.start(request(0x0040, 1, 100, 10)),
            Err(SiiCategoryStreamError::InvalidCapacity)
        );
        assert_eq!(reader.phase(), SiiCategoryStreamPhase::Faulted);
        assert_eq!(
            reader.next_action(1),
            Err(SiiCategoryStreamError::InvalidCapacity)
        );
    }
}
