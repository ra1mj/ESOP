//! Bounded SII read-to-configuration orchestration.
//!
//! [`SiiBlockReader`](crate::SiiBlockReader) owns the EEPROM transaction while
//! [`SiiConfigurationCandidate`](crate::SiiConfigurationCandidate) owns the
//! validated PDO/SyncManager projection. This controller joins them without
//! allocating or applying a partial image.

use crate::control::{ControlError, ControlRequestPool, RequestHandle};
use crate::sii::{SiiAction, SiiBlockError, SiiBlockReader, SiiBlockRequest, SiiProgress};
use crate::sii_config::{SiiConfigurationCandidate, SiiConfigurationError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiDiscoveryPhase {
    Idle,
    Reading,
    Projecting,
    Ready,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiiDiscoveryError {
    Busy,
    NotStarted,
    NotReady,
    Block(SiiBlockError),
    Configuration(SiiConfigurationError),
    Control(ControlError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiiDiscoveryRequest {
    pub block: SiiBlockRequest,
    pub signed: bool,
}

/// Fixed-capacity SII discovery and PDO projection pipeline.
pub struct SiiDiscoveryController<
    const WORDS: usize,
    const SMS: usize,
    const FMMUS: usize,
    const RX_ENTRIES: usize,
    const TX_ENTRIES: usize,
> {
    reader: SiiBlockReader<WORDS>,
    candidate: SiiConfigurationCandidate<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>,
    phase: SiiDiscoveryPhase,
    signed: bool,
    last_error: Option<SiiDiscoveryError>,
}

impl<
    const WORDS: usize,
    const SMS: usize,
    const FMMUS: usize,
    const RX_ENTRIES: usize,
    const TX_ENTRIES: usize,
> SiiDiscoveryController<WORDS, SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>
{
    pub const fn new() -> Self {
        Self {
            reader: SiiBlockReader::new(),
            candidate: SiiConfigurationCandidate::new(),
            phase: SiiDiscoveryPhase::Idle,
            signed: false,
            last_error: None,
        }
    }

    pub const fn phase(&self) -> SiiDiscoveryPhase {
        self.phase
    }

    pub const fn signed(&self) -> bool {
        self.signed
    }

    pub const fn last_error(&self) -> Option<SiiDiscoveryError> {
        self.last_error
    }

    pub const fn pending(&self) -> Option<SiiAction> {
        self.reader.pending()
    }

    pub const fn reader(&self) -> &SiiBlockReader<WORDS> {
        &self.reader
    }

    pub fn candidate(
        &self,
    ) -> Option<&SiiConfigurationCandidate<SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>> {
        if self.phase == SiiDiscoveryPhase::Ready {
            Some(&self.candidate)
        } else {
            None
        }
    }

    pub fn start(&mut self, request: SiiDiscoveryRequest) -> Result<(), SiiDiscoveryError> {
        if !matches!(
            self.phase,
            SiiDiscoveryPhase::Idle | SiiDiscoveryPhase::Ready | SiiDiscoveryPhase::Faulted
        ) {
            return Err(SiiDiscoveryError::Busy);
        }
        self.reader
            .start(request.block)
            .map_err(SiiDiscoveryError::Block)?;
        self.candidate = SiiConfigurationCandidate::new();
        self.phase = SiiDiscoveryPhase::Reading;
        self.signed = request.signed;
        self.last_error = None;
        Ok(())
    }

    pub fn next_action(&mut self, now_ns: u64) -> Result<Option<SiiAction>, SiiDiscoveryError> {
        match self.phase {
            SiiDiscoveryPhase::Idle => return Err(SiiDiscoveryError::NotStarted),
            SiiDiscoveryPhase::Projecting | SiiDiscoveryPhase::Ready => return Ok(None),
            SiiDiscoveryPhase::Faulted => {
                return Err(self.last_error.unwrap_or(SiiDiscoveryError::NotReady));
            }
            SiiDiscoveryPhase::Reading => {}
        }

        match self.reader.next_action(now_ns) {
            Ok(Some(action)) => Ok(Some(action)),
            Ok(None) if self.reader.phase() == crate::sii::SiiPhase::Complete => {
                self.phase = SiiDiscoveryPhase::Projecting;
                Ok(None)
            }
            Ok(None) => Ok(None),
            Err(error) => Err(self.fail(SiiDiscoveryError::Block(error))),
        }
    }

    pub fn enqueue_pending<const REQUESTS: usize>(
        &self,
        pool: &mut ControlRequestPool<REQUESTS>,
    ) -> Result<RequestHandle, SiiDiscoveryError> {
        self.reader
            .enqueue_pending(pool)
            .map_err(SiiDiscoveryError::Control)
    }

    pub fn accept(
        &mut self,
        token: u8,
        generation: u16,
        payload: &[u8],
        working_counter: u16,
        now_ns: u64,
    ) -> Result<SiiProgress, SiiDiscoveryError> {
        if self.phase != SiiDiscoveryPhase::Reading {
            return Err(SiiDiscoveryError::NotReady);
        }
        match self
            .reader
            .accept(token, generation, payload, working_counter, now_ns)
        {
            Ok(progress) => {
                if progress == SiiProgress::Complete {
                    self.phase = SiiDiscoveryPhase::Projecting;
                }
                Ok(progress)
            }
            Err(error) => Err(self.fail(SiiDiscoveryError::Block(error))),
        }
    }

    pub fn timeout(&mut self, token: u8, now_ns: u64) -> Result<(), SiiDiscoveryError> {
        if self.phase != SiiDiscoveryPhase::Reading {
            return Err(SiiDiscoveryError::NotReady);
        }
        match self.reader.timeout(token, now_ns) {
            Ok(()) => Ok(()),
            Err(error) if self.reader.phase() == crate::sii::SiiPhase::Faulted => {
                Err(self.fail(SiiDiscoveryError::Block(error)))
            }
            Err(error) => Err(SiiDiscoveryError::Block(error)),
        }
    }

    /// Project the completed SII image atomically into the candidate.
    ///
    /// The caller owns `scratch`; its size is checked by the underlying block
    /// reader and the bytes are never retained after this call.
    pub fn finalize(&mut self, scratch: &mut [u8]) -> Result<usize, SiiDiscoveryError> {
        if self.phase != SiiDiscoveryPhase::Projecting {
            return Err(SiiDiscoveryError::NotReady);
        }
        let mut next = self.candidate;
        let applied = next
            .apply_completed_block(&self.reader, scratch)
            .map_err(|error| self.fail(SiiDiscoveryError::Configuration(error)))?;
        self.candidate = next;
        self.phase = SiiDiscoveryPhase::Ready;
        Ok(applied)
    }

    fn fail(&mut self, error: SiiDiscoveryError) -> SiiDiscoveryError {
        self.last_error = Some(error);
        self.phase = SiiDiscoveryPhase::Faulted;
        error
    }
}

impl<
    const WORDS: usize,
    const SMS: usize,
    const FMMUS: usize,
    const RX_ENTRIES: usize,
    const TX_ENTRIES: usize,
> Default for SiiDiscoveryController<WORDS, SMS, FMMUS, RX_ENTRIES, TX_ENTRIES>
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sii::{SII_CATEGORY_END, SII_CATEGORY_RX_PDO, SII_CATEGORY_SYNC_MANAGER, SiiPhase};
    use std::vec;

    fn append_category(bytes: &mut std::vec::Vec<u8>, kind: u16, payload: &[u8]) {
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&((payload.len() / 2) as u16).to_le_bytes());
        bytes.extend_from_slice(payload);
    }

    fn drive<
        const WORDS: usize,
        const SMS: usize,
        const FMMUS: usize,
        const RX: usize,
        const TX: usize,
    >(
        controller: &mut SiiDiscoveryController<WORDS, SMS, FMMUS, RX, TX>,
        image_start_word: u16,
        image: &[u8],
    ) {
        loop {
            let action = controller.next_action(1).unwrap();
            let Some(action) = action else {
                assert_eq!(controller.phase(), SiiDiscoveryPhase::Projecting);
                break;
            };
            let payload = if action.read_len == 0 {
                std::vec::Vec::new()
            } else if controller.reader().phase() == SiiPhase::Polling {
                std::vec::Vec::from([0, 0])
            } else if action.word_address >= image_start_word {
                let offset = ((action.word_address - image_start_word) as usize) * 2;
                image[offset..offset + action.read_len as usize].to_vec()
            } else {
                panic!("unexpected SII word address");
            };
            controller
                .accept(
                    action.token,
                    action.generation,
                    &payload,
                    action.expected_wkc,
                    1,
                )
                .unwrap();
            if controller.reader().phase() == SiiPhase::Complete {
                assert_eq!(controller.phase(), SiiDiscoveryPhase::Projecting);
                break;
            }
        }
    }

    #[test]
    fn reads_complete_image_then_projects_candidate_atomically() {
        let mut image = std::vec::Vec::new();
        append_category(
            &mut image,
            SII_CATEGORY_SYNC_MANAGER,
            &[0x00, 0x10, 0x08, 0x00, 0x26, 0x00, 0x01, 0x00],
        );
        let mut pdo = vec![0x00, 0x16, 0x01, 0x02, 0x01, 0x00, 0x00, 0x00];
        pdo.extend_from_slice(&[0x01, 0x60, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00]);
        append_category(&mut image, SII_CATEGORY_RX_PDO, &pdo);
        image.extend_from_slice(&SII_CATEGORY_END.to_le_bytes());
        image.extend_from_slice(&0u16.to_le_bytes());

        let start_word = 0x80;
        assert_eq!(image.len() % 2, 0);
        let mut controller = SiiDiscoveryController::<32, 4, 4, 8, 8>::new();
        controller
            .start(SiiDiscoveryRequest {
                block: SiiBlockRequest {
                    station_address: 1,
                    start_word,
                    word_count: image.len() / 2,
                    generation: 7,
                    now_ns: 0,
                    timeout_ns: 10_000,
                    request_timeout_ns: 100,
                },
                signed: false,
            })
            .unwrap();
        drive(&mut controller, start_word, &image);

        let mut scratch = [0; 64];
        assert_eq!(controller.finalize(&mut scratch).unwrap(), 2);
        assert_eq!(controller.phase(), SiiDiscoveryPhase::Ready);
        let candidate = controller.candidate().unwrap();
        assert_eq!(candidate.mapping().sync_manager_count(), 1);
        assert_eq!(candidate.rx_pdo_count(), 1);
        assert_eq!(candidate.rx_layout().total_bits(), 16);
    }

    #[test]
    fn projection_failure_does_not_publish_a_partial_candidate() {
        let image = [SII_CATEGORY_END as u8, (SII_CATEGORY_END >> 8) as u8, 0, 0];
        let mut controller = SiiDiscoveryController::<4, 1, 1, 1, 1>::new();
        controller
            .start(SiiDiscoveryRequest {
                block: SiiBlockRequest {
                    station_address: 1,
                    start_word: 0x80,
                    word_count: 2,
                    generation: 1,
                    now_ns: 0,
                    timeout_ns: 100,
                    request_timeout_ns: 10,
                },
                signed: false,
            })
            .unwrap();
        drive(&mut controller, 0x80, &image);
        let mut scratch = [0; 1];
        assert_eq!(
            controller.finalize(&mut scratch),
            Err(SiiDiscoveryError::Configuration(
                SiiConfigurationError::Block(SiiBlockError::BufferTooSmall)
            ))
        );
        assert_eq!(controller.phase(), SiiDiscoveryPhase::Faulted);
        assert!(controller.candidate().is_none());
    }

    #[test]
    fn early_timeout_does_not_fault_the_discovery_controller() {
        let mut controller = SiiDiscoveryController::<4, 1, 1, 1, 1>::new();
        controller
            .start(SiiDiscoveryRequest {
                block: SiiBlockRequest {
                    station_address: 1,
                    start_word: 0x80,
                    word_count: 2,
                    generation: 1,
                    now_ns: 0,
                    timeout_ns: 100,
                    request_timeout_ns: 10,
                },
                signed: false,
            })
            .unwrap();
        let action = controller.next_action(1).unwrap().unwrap();
        assert_eq!(
            controller.timeout(action.token, 2),
            Err(SiiDiscoveryError::Block(SiiBlockError::Timeout))
        );
        assert_eq!(controller.phase(), SiiDiscoveryPhase::Reading);
        assert_eq!(controller.pending(), Some(action));
    }
}
