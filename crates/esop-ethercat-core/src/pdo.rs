//! Fixed-layout PDO entry definitions and bit-level process-image access.
//!
//! PDO mapping is configured before activation. Runtime access only performs
//! bounded bit extraction/insertion against caller-owned process-image bytes.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoDirection {
    Rx,
    Tx,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoEntry {
    pub index: u16,
    pub subindex: u8,
    pub bit_offset: usize,
    pub bit_length: u8,
    pub signed: bool,
    pub direction: PdoDirection,
}

impl PdoEntry {
    pub const EMPTY: Self = Self {
        index: 0,
        subindex: 0,
        bit_offset: 0,
        bit_length: 0,
        signed: false,
        direction: PdoDirection::Rx,
    };

    pub fn read_unsigned(&self, image: &[u8]) -> Result<u64, PdoError> {
        self.validate(image.len())?;
        let mut value = 0u64;
        for bit in 0..self.bit_length as usize {
            if image_bit(image, self.bit_offset + bit) {
                value |= 1u64 << bit;
            }
        }
        Ok(value)
    }

    pub fn read_signed(&self, image: &[u8]) -> Result<i64, PdoError> {
        let value = self.read_unsigned(image)?;
        if !self.signed {
            return Ok(value as i64);
        }
        if self.bit_length == 64 {
            return Ok(value as i64);
        }
        let sign_bit = 1u64 << (self.bit_length - 1);
        let mask = (1u64 << self.bit_length) - 1;
        if value & sign_bit != 0 {
            Ok((value | !mask) as i64)
        } else {
            Ok(value as i64)
        }
    }

    pub fn write_unsigned(&self, image: &mut [u8], value: u64) -> Result<(), PdoError> {
        self.validate(image.len())?;
        if self.signed {
            return Err(PdoError::SignedEntryRequiresSignedValue);
        }
        if self.bit_length < 64 && value >= (1u64 << self.bit_length) {
            return Err(PdoError::ValueOutOfRange);
        }
        for bit in 0..self.bit_length as usize {
            set_image_bit(image, self.bit_offset + bit, value & (1u64 << bit) != 0);
        }
        Ok(())
    }

    pub fn write_signed(&self, image: &mut [u8], value: i64) -> Result<(), PdoError> {
        self.validate(image.len())?;
        if !self.signed {
            return Err(PdoError::UnsignedEntryRequiresUnsignedValue);
        }
        if self.bit_length < 64 {
            let min = -(1i64 << (self.bit_length - 1));
            let max = (1i64 << (self.bit_length - 1)) - 1;
            if value < min || value > max {
                return Err(PdoError::ValueOutOfRange);
            }
        }
        let raw = value as u64;
        for bit in 0..self.bit_length as usize {
            set_image_bit(image, self.bit_offset + bit, raw & (1u64 << bit) != 0);
        }
        Ok(())
    }

    fn validate(&self, image_len: usize) -> Result<(), PdoError> {
        if !(1..=64).contains(&self.bit_length) {
            return Err(PdoError::InvalidBitLength);
        }
        let end = self
            .bit_offset
            .checked_add(self.bit_length as usize)
            .ok_or(PdoError::ImageBounds)?;
        if end > image_len.saturating_mul(8) {
            return Err(PdoError::ImageBounds);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdoError {
    CapacityExceeded,
    DuplicateEntry,
    InvalidBitLength,
    ImageBounds,
    BitOverlap,
    UnknownEntry,
    BufferTooSmall,
    ValueOutOfRange,
    SignedEntryRequiresSignedValue,
    UnsignedEntryRequiresUnsignedValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdoLayout<const ENTRIES: usize> {
    entries: [PdoEntry; ENTRIES],
    entry_count: usize,
    total_bits: usize,
}

impl<const ENTRIES: usize> PdoLayout<ENTRIES> {
    pub const fn new() -> Self {
        Self {
            entries: [PdoEntry::EMPTY; ENTRIES],
            entry_count: 0,
            total_bits: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.entry_count
    }

    pub const fn is_empty(&self) -> bool {
        self.entry_count == 0
    }

    pub const fn total_bits(&self) -> usize {
        self.total_bits
    }

    pub const fn total_bytes(&self) -> usize {
        self.total_bits.div_ceil(8)
    }

    pub fn entries(&self) -> &[PdoEntry] {
        &self.entries[..self.entry_count]
    }

    pub fn add(&mut self, entry: PdoEntry) -> Result<usize, PdoError> {
        if ENTRIES == 0 || self.entry_count >= ENTRIES {
            return Err(PdoError::CapacityExceeded);
        }
        if !(1..=64).contains(&entry.bit_length) {
            return Err(PdoError::InvalidBitLength);
        }
        if self.entries().iter().any(|existing| {
            existing.index == entry.index
                && existing.subindex == entry.subindex
                && existing.direction == entry.direction
        }) {
            return Err(PdoError::DuplicateEntry);
        }
        let end = entry
            .bit_offset
            .checked_add(entry.bit_length as usize)
            .ok_or(PdoError::ImageBounds)?;
        if self.entries().iter().any(|existing| {
            if existing.direction != entry.direction {
                return false;
            }
            let existing_end = existing.bit_offset + existing.bit_length as usize;
            entry.bit_offset < existing_end && existing.bit_offset < end
        }) {
            return Err(PdoError::BitOverlap);
        }
        self.entries[self.entry_count] = entry;
        self.entry_count += 1;
        self.total_bits = self.total_bits.max(end);
        Ok(self.entry_count - 1)
    }

    pub fn entry(&self, index: usize) -> Result<PdoEntry, PdoError> {
        self.entries
            .get(index)
            .copied()
            .filter(|_| index < self.entry_count)
            .ok_or(PdoError::UnknownEntry)
    }

    pub fn input_entries(&self) -> impl Iterator<Item = &PdoEntry> {
        self.entries()
            .iter()
            .filter(|entry| entry.direction == PdoDirection::Tx)
    }

    pub fn output_entries(&self) -> impl Iterator<Item = &PdoEntry> {
        self.entries()
            .iter()
            .filter(|entry| entry.direction == PdoDirection::Rx)
    }
}

impl<const ENTRIES: usize> Default for PdoLayout<ENTRIES> {
    fn default() -> Self {
        Self::new()
    }
}

fn image_bit(image: &[u8], bit_offset: usize) -> bool {
    let byte = bit_offset / 8;
    let bit = bit_offset % 8;
    image[byte] & (1u8 << bit) != 0
}

fn set_image_bit(image: &mut [u8], bit_offset: usize, value: bool) {
    let byte = bit_offset / 8;
    let bit = bit_offset % 8;
    let mask = 1u8 << bit;
    if value {
        image[byte] |= mask;
    } else {
        image[byte] &= !mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(bit_offset: usize, bit_length: u8, signed: bool) -> PdoEntry {
        PdoEntry {
            index: 0x6040,
            subindex: 0,
            bit_offset,
            bit_length,
            signed,
            direction: PdoDirection::Rx,
        }
    }

    #[test]
    fn bit_fields_cross_bytes_and_preserve_neighbours() {
        let field = entry(3, 12, false);
        let mut image = [0xA5, 0x5A, 0xC3];
        field.write_unsigned(&mut image, 0xABC).unwrap();
        assert_eq!(field.read_unsigned(&image), Ok(0xABC));
        assert_eq!(image[0] & 0x07, 0x05);
        assert_eq!(image[2] & 0xE0, 0xC0);
    }

    #[test]
    fn signed_fields_are_sign_extended_and_range_checked() {
        let field = entry(0, 8, true);
        let mut image = [0; 1];
        field.write_signed(&mut image, -2).unwrap();
        assert_eq!(field.read_signed(&image), Ok(-2));
        assert_eq!(
            field.write_signed(&mut image, 128),
            Err(PdoError::ValueOutOfRange)
        );
        assert_eq!(
            field.write_signed(&mut image, -129),
            Err(PdoError::ValueOutOfRange)
        );
    }

    #[test]
    fn layout_rejects_same_direction_overlap_but_allows_rx_tx_at_same_bits() {
        let mut layout = PdoLayout::<3>::new();
        layout.add(entry(0, 16, false)).unwrap();
        let mut overlap = entry(8, 8, false);
        overlap.index = 0x6041;
        assert_eq!(layout.add(overlap), Err(PdoError::BitOverlap));
        let mut tx = entry(8, 8, false);
        tx.direction = PdoDirection::Tx;
        layout.add(tx).unwrap();
        assert_eq!(layout.total_bits(), 16);
        assert_eq!(layout.total_bytes(), 2);
    }

    #[test]
    fn invalid_image_and_entry_indices_fail_closed() {
        let field = entry(7, 2, false);
        assert_eq!(field.read_unsigned(&[0]), Err(PdoError::ImageBounds));
        let layout = PdoLayout::<1>::new();
        assert_eq!(layout.entry(0), Err(PdoError::UnknownEntry));
    }
}
