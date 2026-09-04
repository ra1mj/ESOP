//! EtherCAT wire primitives.
//!
//! The wire format is encoded explicitly instead of using packed structs. This
//! keeps byte order and bounds checks visible on every target architecture.

pub const ETHERCAT_ETHERTYPE: u16 = 0x88A4;
pub const ETHERNET_HEADER_LEN: usize = 14;
pub const ETHERCAT_FRAME_HEADER_LEN: usize = 2;
pub const DATAGRAM_HEADER_LEN: usize = 10;
pub const WORKING_COUNTER_LEN: usize = 2;
pub const MIN_ETHERNET_FRAME_LEN: usize = 64;
pub const MAX_ETHERNET_FRAME_LEN: usize = 1518;
const DATAGRAM_LENGTH_MASK: u16 = 0x07FF;
const DATAGRAM_LAST_BIT: u16 = 0x8000;
const FRAME_TYPE_MASK: u16 = 0xF000;
const FRAME_LENGTH_MASK: u16 = 0x07FF;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Command {
    Aprd = 0x01,
    Apwr = 0x02,
    Aprw = 0x03,
    Fprd = 0x04,
    Fpwr = 0x05,
    Fprw = 0x06,
    Brd = 0x07,
    Bwr = 0x08,
    Brw = 0x09,
    Lrd = 0x0A,
    Lwr = 0x0B,
    Lrw = 0x0C,
    Armw = 0x0D,
    Frmw = 0x0E,
}

impl Command {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::Aprd,
            0x02 => Self::Apwr,
            0x03 => Self::Aprw,
            0x04 => Self::Fprd,
            0x05 => Self::Fpwr,
            0x06 => Self::Fprw,
            0x07 => Self::Brd,
            0x08 => Self::Bwr,
            0x09 => Self::Brw,
            0x0A => Self::Lrd,
            0x0B => Self::Lwr,
            0x0C => Self::Lrw,
            0x0D => Self::Armw,
            0x0E => Self::Frmw,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    BufferTooSmall,
    FrameTooShort,
    FrameLengthMismatch,
    UnsupportedFrameType,
    DatagramHeaderTruncated,
    DatagramLengthOutOfBounds,
    InvalidCommand,
    TooManyDatagrams,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatagramHeader {
    pub command: Command,
    pub index: u8,
    pub address: u32,
    pub length: u16,
    pub last: bool,
}

impl DatagramHeader {
    pub const fn new(command: Command, index: u8, address: u32, length: u16) -> Self {
        Self {
            command,
            index,
            address,
            length,
            last: true,
        }
    }

    pub fn encode(self, dst: &mut [u8]) -> Result<(), WireError> {
        if dst.len() < DATAGRAM_HEADER_LEN {
            return Err(WireError::BufferTooSmall);
        }
        if self.length as usize > DATAGRAM_LENGTH_MASK as usize {
            return Err(WireError::DatagramLengthOutOfBounds);
        }

        dst[0] = self.command as u8;
        dst[1] = self.index;
        dst[2..6].copy_from_slice(&self.address.to_le_bytes());
        let mut length = self.length & DATAGRAM_LENGTH_MASK;
        if self.last {
            length |= DATAGRAM_LAST_BIT;
        }
        dst[6..8].copy_from_slice(&length.to_le_bytes());
        dst[8] = 0;
        dst[9] = 0;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> Result<Self, WireError> {
        if src.len() < DATAGRAM_HEADER_LEN {
            return Err(WireError::DatagramHeaderTruncated);
        }
        let command = Command::from_u8(src[0]).ok_or(WireError::InvalidCommand)?;
        let length_word = u16::from_le_bytes([src[6], src[7]]);
        Ok(Self {
            command,
            index: src[1],
            address: u32::from_le_bytes([src[2], src[3], src[4], src[5]]),
            length: length_word & DATAGRAM_LENGTH_MASK,
            last: length_word & DATAGRAM_LAST_BIT != 0,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Datagram<'a> {
    pub header: DatagramHeader,
    pub payload: &'a [u8],
    pub working_counter: u16,
}

pub struct DatagramIter<'a> {
    bytes: &'a [u8],
    offset: usize,
    remaining: usize,
    done: bool,
}

impl<'a> Iterator for DatagramIter<'a> {
    type Item = Result<Datagram<'a>, WireError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.offset >= self.bytes.len() || self.remaining == 0 {
            return None;
        }
        let header_end = self.offset.saturating_add(DATAGRAM_HEADER_LEN);
        if header_end > self.bytes.len() {
            self.done = true;
            return Some(Err(WireError::DatagramHeaderTruncated));
        }
        let header = match DatagramHeader::decode(&self.bytes[self.offset..header_end]) {
            Ok(header) => header,
            Err(error) => {
                self.done = true;
                return Some(Err(error));
            }
        };
        let payload_start = header_end;
        let payload_end = payload_start.saturating_add(header.length as usize);
        let wkc_end = payload_end.saturating_add(WORKING_COUNTER_LEN);
        if wkc_end > self.bytes.len() {
            self.done = true;
            return Some(Err(WireError::DatagramLengthOutOfBounds));
        }

        let datagram = Datagram {
            header,
            payload: &self.bytes[payload_start..payload_end],
            working_counter: u16::from_le_bytes([
                self.bytes[payload_end],
                self.bytes[payload_end + 1],
            ]),
        };
        self.offset = wkc_end;
        self.remaining -= 1;
        self.done = header.last;
        Some(Ok(datagram))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameView<'a> {
    pub destination: [u8; 6],
    pub source: [u8; 6],
    pub payload: &'a [u8],
    pub frame_type: u8,
    datagram_count: usize,
}

impl<'a> FrameView<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, WireError> {
        if bytes.len() < ETHERNET_HEADER_LEN + ETHERCAT_FRAME_HEADER_LEN {
            return Err(WireError::FrameTooShort);
        }
        let ether_type = u16::from_be_bytes([bytes[12], bytes[13]]);
        if ether_type != ETHERCAT_ETHERTYPE {
            return Err(WireError::UnsupportedFrameType);
        }
        let frame_word = u16::from_le_bytes([bytes[14], bytes[15]]);
        let frame_payload_len = (frame_word & FRAME_LENGTH_MASK) as usize;
        let payload_start = ETHERNET_HEADER_LEN + ETHERCAT_FRAME_HEADER_LEN;
        let payload_end = payload_start.saturating_add(frame_payload_len);
        if payload_end > bytes.len()
            || bytes.len() > MAX_ETHERNET_FRAME_LEN
            || (bytes.len() != payload_end && bytes.len() != MIN_ETHERNET_FRAME_LEN)
        {
            return Err(WireError::FrameLengthMismatch);
        }
        let frame_type = ((frame_word & FRAME_TYPE_MASK) >> 12) as u8;
        if frame_type != 1 {
            return Err(WireError::UnsupportedFrameType);
        }

        let payload = &bytes[payload_start..payload_end];
        let mut iter = DatagramIter {
            bytes: payload,
            offset: 0,
            remaining: 256,
            done: false,
        };
        let mut datagram_count = 0;
        for result in iter.by_ref() {
            result?;
            datagram_count += 1;
        }
        if datagram_count == 0 {
            return Err(WireError::DatagramHeaderTruncated);
        }
        if !iter.done || iter.offset != payload.len() {
            return Err(WireError::DatagramLengthOutOfBounds);
        }

        let mut destination = [0u8; 6];
        destination.copy_from_slice(&bytes[0..6]);
        let mut source = [0u8; 6];
        source.copy_from_slice(&bytes[6..12]);

        Ok(Self {
            destination,
            source,
            payload,
            frame_type,
            datagram_count,
        })
    }

    pub const fn datagram_count(&self) -> usize {
        self.datagram_count
    }

    pub fn datagrams(&self) -> DatagramIter<'a> {
        DatagramIter {
            bytes: self.payload,
            offset: 0,
            remaining: self.datagram_count,
            done: false,
        }
    }
}

pub struct FrameBuilder<'a> {
    buffer: &'a mut [u8],
    offset: usize,
    datagram_count: usize,
    previous_header_offset: Option<usize>,
}

impl<'a> FrameBuilder<'a> {
    pub fn new(
        buffer: &'a mut [u8],
        destination: [u8; 6],
        source: [u8; 6],
    ) -> Result<Self, WireError> {
        if buffer.len() < ETHERNET_HEADER_LEN + ETHERCAT_FRAME_HEADER_LEN {
            return Err(WireError::BufferTooSmall);
        }
        buffer[0..6].copy_from_slice(&destination);
        buffer[6..12].copy_from_slice(&source);
        buffer[12..14].copy_from_slice(&ETHERCAT_ETHERTYPE.to_be_bytes());
        buffer[14..16].fill(0);
        Ok(Self {
            buffer,
            offset: ETHERNET_HEADER_LEN + ETHERCAT_FRAME_HEADER_LEN,
            datagram_count: 0,
            previous_header_offset: None,
        })
    }

    pub fn push(
        &mut self,
        command: Command,
        index: u8,
        address: u32,
        payload: &[u8],
    ) -> Result<(), WireError> {
        if payload.len() > DATAGRAM_LENGTH_MASK as usize {
            return Err(WireError::DatagramLengthOutOfBounds);
        }
        if self.datagram_count >= 256 {
            return Err(WireError::TooManyDatagrams);
        }
        let required = DATAGRAM_HEADER_LEN
            .saturating_add(payload.len())
            .saturating_add(WORKING_COUNTER_LEN);
        let end = self.offset.saturating_add(required);
        if end > self.buffer.len() {
            return Err(WireError::BufferTooSmall);
        }

        let header = DatagramHeader {
            command,
            index,
            address,
            length: payload.len() as u16,
            last: true,
        };
        header.encode(&mut self.buffer[self.offset..self.offset + DATAGRAM_HEADER_LEN])?;
        let payload_start = self.offset + DATAGRAM_HEADER_LEN;
        self.buffer[payload_start..payload_start + payload.len()].copy_from_slice(payload);
        self.buffer[payload_start + payload.len()..end].fill(0);
        if let Some(previous_header_offset) = self.previous_header_offset {
            let previous_word = u16::from_le_bytes([
                self.buffer[previous_header_offset + 6],
                self.buffer[previous_header_offset + 7],
            ]) & !DATAGRAM_LAST_BIT;
            self.buffer[previous_header_offset + 6..previous_header_offset + 8]
                .copy_from_slice(&previous_word.to_le_bytes());
        }
        self.previous_header_offset = Some(self.offset);
        self.offset = end;
        self.datagram_count += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<usize, WireError> {
        if self.datagram_count == 0 {
            return Err(WireError::DatagramHeaderTruncated);
        }
        let payload_len = self.offset - ETHERNET_HEADER_LEN - ETHERCAT_FRAME_HEADER_LEN;
        if payload_len > FRAME_LENGTH_MASK as usize {
            return Err(WireError::FrameLengthMismatch);
        }
        let frame_word = (payload_len as u16) | (1 << 12);
        self.buffer[14..16].copy_from_slice(&frame_word.to_le_bytes());
        let frame_len = self.offset.max(MIN_ETHERNET_FRAME_LEN);
        if frame_len > self.buffer.len() {
            return Err(WireError::BufferTooSmall);
        }
        self.buffer[self.offset..frame_len].fill(0);
        Ok(frame_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip_handles_multiple_datagrams() {
        let mut bytes = [0u8; 128];
        let mut builder = FrameBuilder::new(&mut bytes, [0xFF; 6], [1, 2, 3, 4, 5, 6]).unwrap();
        builder
            .push(Command::Lrw, 7, 0x1122_3344, &[1, 2, 3])
            .unwrap();
        builder
            .push(Command::Fprd, 8, 0x5566_7788, &[4, 5])
            .unwrap();
        let len = builder.finish().unwrap();
        let view = FrameView::parse(&bytes[..len]).unwrap();
        assert_eq!(view.datagram_count(), 2);
        let mut datagrams = view.datagrams();
        let first = datagrams.next().unwrap().unwrap();
        let second = datagrams.next().unwrap().unwrap();
        assert!(datagrams.next().is_none());
        assert_eq!(first.header.command, Command::Lrw);
        assert_eq!(first.payload, &[1, 2, 3]);
        assert_eq!(second.header.address, 0x5566_7788);
    }

    #[test]
    fn parser_rejects_bytes_after_last_datagram() {
        let mut bytes = [0u8; 128];
        let mut builder = FrameBuilder::new(&mut bytes, [0xFF; 6], [1, 2, 3, 4, 5, 6]).unwrap();
        builder.push(Command::Lrd, 1, 0x1000, &[1]).unwrap();
        let length = builder.finish().unwrap();
        let frame_word = u16::from_le_bytes([bytes[14], bytes[15]]);
        bytes[14..16].copy_from_slice(&(frame_word + 1).to_le_bytes());
        assert_eq!(
            FrameView::parse(&bytes[..length]),
            Err(WireError::DatagramLengthOutOfBounds)
        );
    }
}
