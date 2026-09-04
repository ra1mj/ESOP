#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RxSlotState {
    Empty = 0,
    Armed = 1,
    Complete = 2,
    Rejected = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxIndexEntry {
    pub slot_id: u16,
    pub generation: u16,
    pub deadline_ns: u64,
    pub expected_address: u32,
    pub expected_size: u16,
    pub expected_type: u8,
    pub expected_wkc: u16,
    pub state: RxSlotState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxExpectation {
    pub generation: u16,
    pub deadline_ns: u64,
    pub expected_address: u32,
    pub expected_size: u16,
    pub expected_type: u8,
    pub expected_wkc: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxResponse {
    pub generation: u16,
    pub address: u32,
    pub payload_size: u16,
    pub command: u8,
    pub working_counter: u16,
    pub received_at_ns: u64,
}

impl RxIndexEntry {
    pub const EMPTY: Self = Self {
        slot_id: 0,
        generation: 0,
        deadline_ns: 0,
        expected_address: 0,
        expected_size: 0,
        expected_type: 0,
        expected_wkc: 0,
        state: RxSlotState::Empty,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RxIndexError {
    AlreadyArmed,
    InvalidDeadline,
    UnknownIndex,
    NotArmed,
    GenerationMismatch,
    AddressMismatch,
    SizeMismatch,
    TypeMismatch,
    WorkingCounterMismatch,
    DeadlineExceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxMatch {
    pub slot_id: u16,
    pub generation: u16,
    pub working_counter: u16,
}

/// Fixed bitmap of RX indices that crossed their deadline during one bounded
/// sweep. The master can emit one diagnostic per index without allocating or
/// searching its plan again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxExpiry {
    words: [u64; 4],
    count: u16,
}

impl RxExpiry {
    const EMPTY: Self = Self {
        words: [0; 4],
        count: 0,
    };

    pub const fn count(self) -> usize {
        self.count as usize
    }

    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    pub fn indices(self) -> RxExpiryIndices {
        RxExpiryIndices {
            words: self.words,
            word_index: 0,
        }
    }

    fn record(&mut self, index: u8) {
        let word = (index / 64) as usize;
        let bit = index % 64;
        self.words[word] |= 1u64 << bit;
        self.count = self.count.saturating_add(1);
    }
}

pub struct RxExpiryIndices {
    words: [u64; 4],
    word_index: usize,
}

impl Iterator for RxExpiryIndices {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        while self.word_index < self.words.len() {
            let word = self.words[self.word_index];
            if word == 0 {
                self.word_index += 1;
                continue;
            }
            let bit = word.trailing_zeros() as usize;
            self.words[self.word_index] &= !(1u64 << bit);
            return Some((self.word_index * 64 + bit) as u8);
        }
        None
    }
}

pub struct RxIndexTable {
    entries: [RxIndexEntry; 256],
}

impl RxIndexTable {
    pub const fn new() -> Self {
        Self {
            entries: [RxIndexEntry::EMPTY; 256],
        }
    }

    pub fn arm(
        &mut self,
        index: u8,
        slot_id: u16,
        expectation: RxExpectation,
    ) -> Result<(), RxIndexError> {
        let entry = &mut self.entries[index as usize];
        if entry.state == RxSlotState::Armed {
            return Err(RxIndexError::AlreadyArmed);
        }
        if expectation.deadline_ns == 0 {
            return Err(RxIndexError::InvalidDeadline);
        }
        *entry = RxIndexEntry {
            slot_id,
            generation: expectation.generation,
            deadline_ns: expectation.deadline_ns,
            expected_address: expectation.expected_address,
            expected_size: expectation.expected_size,
            expected_type: expectation.expected_type,
            expected_wkc: expectation.expected_wkc,
            state: RxSlotState::Armed,
        };
        Ok(())
    }

    pub fn validate_and_complete(
        &mut self,
        index: u8,
        response: RxResponse,
    ) -> Result<RxMatch, RxIndexError> {
        let entry = &mut self.entries[index as usize];
        if entry.state == RxSlotState::Empty {
            return Err(RxIndexError::UnknownIndex);
        }
        if entry.state != RxSlotState::Armed {
            return Err(RxIndexError::NotArmed);
        }
        if response.received_at_ns > entry.deadline_ns {
            entry.state = RxSlotState::Rejected;
            return Err(RxIndexError::DeadlineExceeded);
        }
        if entry.generation != response.generation {
            entry.state = RxSlotState::Rejected;
            return Err(RxIndexError::GenerationMismatch);
        }
        if entry.expected_address != response.address {
            entry.state = RxSlotState::Rejected;
            return Err(RxIndexError::AddressMismatch);
        }
        if entry.expected_size != response.payload_size {
            entry.state = RxSlotState::Rejected;
            return Err(RxIndexError::SizeMismatch);
        }
        if entry.expected_type != response.command {
            entry.state = RxSlotState::Rejected;
            return Err(RxIndexError::TypeMismatch);
        }
        if entry.expected_wkc != response.working_counter {
            entry.state = RxSlotState::Rejected;
            return Err(RxIndexError::WorkingCounterMismatch);
        }
        entry.state = RxSlotState::Complete;
        Ok(RxMatch {
            slot_id: entry.slot_id,
            generation: entry.generation,
            working_counter: response.working_counter,
        })
    }

    pub fn entry(&self, index: u8) -> RxIndexEntry {
        self.entries[index as usize]
    }

    /// Mark every still-armed index whose deadline is in the past as
    /// rejected. This is deliberately a fixed 256-entry scan performed only
    /// at explicit receive boundaries, never an unbounded wait for traffic.
    pub fn expire_armed(&mut self, now_ns: u64) -> RxExpiry {
        let mut expiry = RxExpiry::EMPTY;
        for (index, entry) in self.entries.iter_mut().enumerate() {
            if entry.state == RxSlotState::Armed && now_ns > entry.deadline_ns {
                entry.state = RxSlotState::Rejected;
                expiry.record(index as u8);
            }
        }
        expiry
    }

    /// Cancel every expectation owned by one frame slot.
    ///
    /// This is used when TX submission fails after the frame has already been
    /// built and armed. Clearing the whole slot ownership prevents a failed
    /// send from leaking indices into the next cycle.
    pub fn cancel_slot(&mut self, slot_id: u16) -> usize {
        let mut canceled = 0;
        for entry in &mut self.entries {
            if entry.state != RxSlotState::Empty && entry.slot_id == slot_id {
                *entry = RxIndexEntry::EMPTY;
                canceled += 1;
            }
        }
        canceled
    }

    pub fn reset_complete(&mut self) {
        for entry in &mut self.entries {
            if matches!(entry.state, RxSlotState::Complete | RxSlotState::Rejected) {
                entry.state = RxSlotState::Empty;
            }
        }
    }
}

impl Default for RxIndexTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn index_lookup_is_generation_and_wkc_checked() {
        let mut table = RxIndexTable::new();
        table
            .arm(
                9,
                3,
                RxExpectation {
                    generation: 17,
                    deadline_ns: 100,
                    expected_address: 0x1000,
                    expected_size: 4,
                    expected_type: 0x0C,
                    expected_wkc: 2,
                },
            )
            .unwrap();
        assert_eq!(
            table.validate_and_complete(
                9,
                RxResponse {
                    generation: 16,
                    address: 0x1000,
                    payload_size: 4,
                    command: 0x0C,
                    working_counter: 2,
                    received_at_ns: 10,
                },
            ),
            Err(RxIndexError::GenerationMismatch)
        );
        assert_eq!(table.entry(9).state, RxSlotState::Rejected);
        table
            .arm(
                9,
                3,
                RxExpectation {
                    generation: 17,
                    deadline_ns: 100,
                    expected_address: 0x1000,
                    expected_size: 4,
                    expected_type: 0x0C,
                    expected_wkc: 2,
                },
            )
            .unwrap();
        assert_eq!(
            table.validate_and_complete(
                9,
                RxResponse {
                    generation: 17,
                    address: 0x1000,
                    payload_size: 4,
                    command: 0x0C,
                    working_counter: 1,
                    received_at_ns: 10,
                },
            ),
            Err(RxIndexError::WorkingCounterMismatch)
        );
        assert_eq!(table.entry(9).state, RxSlotState::Rejected);
        table
            .arm(
                9,
                3,
                RxExpectation {
                    generation: 17,
                    deadline_ns: 100,
                    expected_address: 0x1000,
                    expected_size: 4,
                    expected_type: 0x0C,
                    expected_wkc: 2,
                },
            )
            .unwrap();
        assert_eq!(
            table.validate_and_complete(
                9,
                RxResponse {
                    generation: 17,
                    address: 0x1000,
                    payload_size: 4,
                    command: 0x0C,
                    working_counter: 2,
                    received_at_ns: 10,
                },
            ),
            Ok(RxMatch {
                slot_id: 3,
                generation: 17,
                working_counter: 2,
            })
        );
        assert_eq!(table.entry(9).state, RxSlotState::Complete);
    }

    #[test]
    fn index_lookup_rejects_wrong_address() {
        let mut table = RxIndexTable::new();
        table
            .arm(
                1,
                2,
                RxExpectation {
                    generation: 3,
                    deadline_ns: 100,
                    expected_address: 0x2000,
                    expected_size: 1,
                    expected_type: 0x0B,
                    expected_wkc: 0,
                },
            )
            .unwrap();
        assert_eq!(
            table.validate_and_complete(
                1,
                RxResponse {
                    generation: 3,
                    address: 0x2001,
                    payload_size: 1,
                    command: 0x0B,
                    working_counter: 0,
                    received_at_ns: 10,
                },
            ),
            Err(RxIndexError::AddressMismatch)
        );
    }

    #[test]
    fn expired_indices_are_reported_and_late_frames_are_rejected() {
        let mut table = RxIndexTable::new();
        table
            .arm(
                3,
                1,
                RxExpectation {
                    generation: 9,
                    deadline_ns: 20,
                    expected_address: 0x1000,
                    expected_size: 2,
                    expected_type: 0x04,
                    expected_wkc: 1,
                },
            )
            .unwrap();
        let expired = table.expire_armed(21);
        assert_eq!(expired.count(), 1);
        assert_eq!(expired.indices().collect::<std::vec::Vec<_>>(), vec![3]);
        assert_eq!(table.entry(3).state, RxSlotState::Rejected);

        table
            .arm(
                4,
                1,
                RxExpectation {
                    generation: 9,
                    deadline_ns: 20,
                    expected_address: 0x1000,
                    expected_size: 2,
                    expected_type: 0x04,
                    expected_wkc: 1,
                },
            )
            .unwrap();
        assert_eq!(
            table.validate_and_complete(
                4,
                RxResponse {
                    generation: 9,
                    address: 0x1000,
                    payload_size: 2,
                    command: 0x04,
                    working_counter: 1,
                    received_at_ns: 21,
                },
            ),
            Err(RxIndexError::DeadlineExceeded)
        );
    }

    #[test]
    fn cancel_slot_releases_all_owned_expectations() {
        let mut table = RxIndexTable::new();
        let expectation = RxExpectation {
            generation: 1,
            deadline_ns: 100,
            expected_address: 0,
            expected_size: 0,
            expected_type: 0,
            expected_wkc: 0,
        };
        table.arm(1, 9, expectation).unwrap();
        table.arm(2, 9, expectation).unwrap();
        table.arm(3, 10, expectation).unwrap();
        assert_eq!(table.cancel_slot(9), 2);
        assert_eq!(table.entry(1).state, RxSlotState::Empty);
        assert_eq!(table.entry(2).state, RxSlotState::Empty);
        assert_eq!(table.entry(3).state, RxSlotState::Armed);
    }
}
