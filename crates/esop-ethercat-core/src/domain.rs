use crate::engine::RxDatagramConsumer;
use crate::rx_index::RxMatch;
use crate::wire::DatagramHeader;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainSegment {
    pub datagram_index: u8,
    pub input_offset: usize,
    pub len: usize,
    pub expected_wkc: u16,
}

impl DomainSegment {
    pub const EMPTY: Self = Self {
        datagram_index: 0,
        input_offset: 0,
        len: 0,
        expected_wkc: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainQuality {
    pub expected_wkc: u16,
    pub actual_wkc: u16,
    pub valid: bool,
    pub complete: bool,
    pub last_valid_cycle: u64,
    pub input_age_cycles: u64,
}

impl DomainQuality {
    pub const EMPTY: Self = Self {
        expected_wkc: 0,
        actual_wkc: 0,
        valid: false,
        complete: false,
        last_valid_cycle: 0,
        input_age_cycles: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainError {
    TooManySegments,
    EmptyDomain,
    DuplicateDatagramIndex,
    SegmentOutOfBounds,
    ExpectedWkcOverflow,
    NoActiveReceive,
    GenerationMismatch,
    UnknownDatagramIndex,
    DuplicateDatagram,
    PayloadLengthMismatch,
}

/// Fixed-layout EtherCAT Domain with double staging pages.
///
/// A received page is never published until every configured segment for its
/// generation has been staged. Failed or incomplete receives retain the last
/// committed input image and only update quality metadata.
pub struct Domain<const BYTES: usize, const SEGMENTS: usize> {
    logical_address: u32,
    segments: [DomainSegment; SEGMENTS],
    segment_count: usize,
    expected_wkc: u16,
    output: [u8; BYTES],
    staging: [[u8; BYTES]; 2],
    committed: [u8; BYTES],
    active_generation: Option<u16>,
    active_page: usize,
    received_mask: u64,
    staging_error: bool,
    actual_wkc: u16,
    quality: DomainQuality,
}

impl<const BYTES: usize, const SEGMENTS: usize> Domain<BYTES, SEGMENTS> {
    pub const fn new(logical_address: u32) -> Self {
        Self {
            logical_address,
            segments: [DomainSegment::EMPTY; SEGMENTS],
            segment_count: 0,
            expected_wkc: 0,
            output: [0; BYTES],
            staging: [[0; BYTES]; 2],
            committed: [0; BYTES],
            active_generation: None,
            active_page: 0,
            received_mask: 0,
            staging_error: false,
            actual_wkc: 0,
            quality: DomainQuality::EMPTY,
        }
    }

    pub const fn logical_address(&self) -> u32 {
        self.logical_address
    }

    pub const fn segment_count(&self) -> usize {
        self.segment_count
    }

    pub const fn expected_wkc(&self) -> u16 {
        self.expected_wkc
    }

    pub const fn quality(&self) -> DomainQuality {
        self.quality
    }

    pub fn input(&self) -> &[u8; BYTES] {
        &self.committed
    }

    pub fn output(&self) -> &[u8; BYTES] {
        &self.output
    }

    pub fn output_mut(&mut self) -> &mut [u8; BYTES] {
        &mut self.output
    }

    pub fn add_segment(&mut self, segment: DomainSegment) -> Result<(), DomainError> {
        if SEGMENTS == 0 || SEGMENTS > 64 || self.segment_count >= SEGMENTS {
            return Err(DomainError::TooManySegments);
        }
        if self
            .segments()
            .iter()
            .any(|existing| existing.datagram_index == segment.datagram_index)
        {
            return Err(DomainError::DuplicateDatagramIndex);
        }
        let end = segment
            .input_offset
            .checked_add(segment.len)
            .ok_or(DomainError::SegmentOutOfBounds)?;
        if end > BYTES {
            return Err(DomainError::SegmentOutOfBounds);
        }
        self.expected_wkc = self
            .expected_wkc
            .checked_add(segment.expected_wkc)
            .ok_or(DomainError::ExpectedWkcOverflow)?;
        self.segments[self.segment_count] = segment;
        self.segment_count += 1;
        self.quality.expected_wkc = self.expected_wkc;
        Ok(())
    }

    pub fn segments(&self) -> &[DomainSegment] {
        &self.segments[..self.segment_count]
    }

    pub fn begin_receive(&mut self, generation: u16) -> Result<(), DomainError> {
        if self.segment_count == 0 {
            return Err(DomainError::EmptyDomain);
        }
        self.active_page = (generation as usize) & 1;
        self.staging[self.active_page].copy_from_slice(&self.committed);
        self.active_generation = Some(generation);
        self.received_mask = 0;
        self.staging_error = false;
        self.actual_wkc = 0;
        self.quality.complete = false;
        self.quality.actual_wkc = 0;
        Ok(())
    }

    pub fn stage_datagram(
        &mut self,
        generation: u16,
        header: DatagramHeader,
        payload: &[u8],
        working_counter: u16,
    ) -> Result<(), DomainError> {
        if self.active_generation != Some(generation) {
            return Err(DomainError::GenerationMismatch);
        }
        let segment_index = self
            .segments()
            .iter()
            .position(|segment| segment.datagram_index == header.index)
            .ok_or(DomainError::UnknownDatagramIndex)?;
        let bit = 1u64 << segment_index;
        if self.received_mask & bit != 0 {
            self.staging_error = true;
            return Err(DomainError::DuplicateDatagram);
        }
        let segment = self.segments[segment_index];
        if payload.len() != segment.len {
            self.staging_error = true;
            return Err(DomainError::PayloadLengthMismatch);
        }

        let end = segment.input_offset + segment.len;
        self.staging[self.active_page][segment.input_offset..end].copy_from_slice(payload);
        self.received_mask |= bit;
        self.actual_wkc = self.actual_wkc.saturating_add(working_counter);
        Ok(())
    }

    pub fn finish_receive(&mut self, generation: u16, cycle: u64) -> Result<bool, DomainError> {
        if self.active_generation != Some(generation) {
            return Err(DomainError::NoActiveReceive);
        }
        let expected_mask = if self.segment_count == 64 {
            u64::MAX
        } else {
            (1u64 << self.segment_count) - 1
        };
        let complete = !self.staging_error
            && self.received_mask == expected_mask
            && self.actual_wkc == self.expected_wkc;

        self.quality.actual_wkc = self.actual_wkc;
        self.quality.complete = complete;
        self.active_generation = None;
        if complete {
            self.committed
                .copy_from_slice(&self.staging[self.active_page]);
            self.quality.valid = true;
            self.quality.last_valid_cycle = cycle;
            self.quality.input_age_cycles = 0;
            return Ok(true);
        }

        self.quality.valid = false;
        self.quality.input_age_cycles = self.quality.input_age_cycles.saturating_add(1);
        Ok(false)
    }
}

impl<const BYTES: usize, const SEGMENTS: usize> Default for Domain<BYTES, SEGMENTS> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<const BYTES: usize, const SEGMENTS: usize> RxDatagramConsumer for Domain<BYTES, SEGMENTS> {
    fn accept(
        &mut self,
        _: u64,
        _: u64,
        completion: RxMatch,
        header: DatagramHeader,
        payload: &[u8],
    ) -> bool {
        self.stage_datagram(
            completion.generation,
            header,
            payload,
            completion.working_counter,
        )
        .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Command;

    fn header(index: u8) -> DatagramHeader {
        DatagramHeader {
            command: Command::Lrw,
            index,
            address: 0x1000,
            length: 2,
            last: true,
        }
    }

    #[test]
    fn domain_commits_only_a_complete_generation() {
        let mut domain = Domain::<4, 2>::new(0x1000);
        domain
            .add_segment(DomainSegment {
                datagram_index: 1,
                input_offset: 0,
                len: 2,
                expected_wkc: 1,
            })
            .unwrap();
        domain
            .add_segment(DomainSegment {
                datagram_index: 2,
                input_offset: 2,
                len: 2,
                expected_wkc: 1,
            })
            .unwrap();

        domain.begin_receive(7).unwrap();
        domain.stage_datagram(7, header(1), &[1, 2], 1).unwrap();
        assert!(!domain.finish_receive(7, 10).unwrap());
        assert_eq!(domain.input(), &[0, 0, 0, 0]);
        assert!(!domain.quality().valid);
        assert_eq!(domain.quality().input_age_cycles, 1);

        domain.begin_receive(8).unwrap();
        domain.stage_datagram(8, header(1), &[3, 4], 1).unwrap();
        domain.stage_datagram(8, header(2), &[5, 6], 1).unwrap();
        assert!(domain.finish_receive(8, 11).unwrap());
        assert_eq!(domain.input(), &[3, 4, 5, 6]);
        assert!(domain.quality().valid);
        assert_eq!(domain.quality().last_valid_cycle, 11);
        assert_eq!(domain.quality().input_age_cycles, 0);
    }

    #[test]
    fn domain_rejects_duplicate_segments_and_payloads() {
        let mut domain = Domain::<2, 1>::new(0);
        let segment = DomainSegment {
            datagram_index: 1,
            input_offset: 0,
            len: 2,
            expected_wkc: 1,
        };
        domain.add_segment(segment).unwrap();
        assert_eq!(
            domain.add_segment(segment),
            Err(DomainError::TooManySegments)
        );
        domain.begin_receive(1).unwrap();
        domain.stage_datagram(1, header(1), &[1, 2], 1).unwrap();
        assert_eq!(
            domain.stage_datagram(1, header(1), &[1, 2], 1),
            Err(DomainError::DuplicateDatagram)
        );
        assert!(!domain.finish_receive(1, 1).unwrap());
    }
}
