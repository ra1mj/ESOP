use core::fmt;

pub const IPC_MAGIC: u32 = u32::from_le_bytes(*b"ESIP");
pub const IPC_VERSION: u16 = 1;
pub const HEADER_BYTES: usize = 80;
pub const MAX_PAYLOAD_BYTES: usize = 4096;
pub const MAX_DATAGRAM_BYTES: usize = HEADER_BYTES + MAX_PAYLOAD_BYTES;

const CRC32_POLYNOMIAL: u32 = 0xedb8_8320;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum MessageKind {
    Command = 1,
    State = 2,
    Event = 3,
    Heartbeat = 4,
    Diagnostic = 5,
}

impl TryFrom<u16> for MessageKind {
    type Error = FrameError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Command),
            2 => Ok(Self::State),
            3 => Ok(Self::Event),
            4 => Ok(Self::Heartbeat),
            5 => Ok(Self::Diagnostic),
            _ => Err(FrameError::UnknownMessageKind(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcHeader {
    pub kind: MessageKind,
    pub payload_schema_version: u32,
    pub layout_hash: u64,
    pub robot_id: u64,
    pub boot_id: u64,
    pub source_id: u64,
    pub sequence: u64,
    pub monotonic_time_ns: u64,
    pub quality_bits: u64,
}

impl IpcHeader {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        kind: MessageKind,
        payload_schema_version: u32,
        layout_hash: u64,
        robot_id: u64,
        boot_id: u64,
        source_id: u64,
        sequence: u64,
        monotonic_time_ns: u64,
        quality_bits: u64,
    ) -> Self {
        Self {
            kind,
            payload_schema_version,
            layout_hash,
            robot_id,
            boot_id,
            source_id,
            sequence,
            monotonic_time_ns,
            quality_bits,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpcFrame {
    header: IpcHeader,
    payload: [u8; MAX_PAYLOAD_BYTES],
    payload_len: usize,
}

impl IpcFrame {
    pub fn new(header: IpcHeader, payload: &[u8]) -> Result<Self, FrameError> {
        validate_header(&header)?;
        validate_payload(header.kind, payload.len())?;

        let mut storage = [0; MAX_PAYLOAD_BYTES];
        storage[..payload.len()].copy_from_slice(payload);
        Ok(Self {
            header,
            payload: storage,
            payload_len: payload.len(),
        })
    }

    pub const fn header(&self) -> &IpcHeader {
        &self.header
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.payload_len]
    }

    pub const fn encoded_len(&self) -> usize {
        HEADER_BYTES + self.payload_len
    }

    pub fn encode_into(&self, output: &mut [u8]) -> Result<usize, FrameError> {
        let encoded_len = self.encoded_len();
        if output.len() < encoded_len {
            return Err(FrameError::OutputTooSmall {
                required: encoded_len,
                actual: output.len(),
            });
        }

        output[..encoded_len].fill(0);
        write_u32(output, 0, IPC_MAGIC);
        write_u16(output, 4, IPC_VERSION);
        write_u16(output, 6, HEADER_BYTES as u16);
        write_u16(output, 8, self.header.kind as u16);
        write_u32(output, 12, self.header.payload_schema_version);
        write_u64(output, 16, self.header.layout_hash);
        write_u64(output, 24, self.header.robot_id);
        write_u64(output, 32, self.header.boot_id);
        write_u64(output, 40, self.header.source_id);
        write_u64(output, 48, self.header.sequence);
        write_u64(output, 56, self.header.monotonic_time_ns);
        write_u64(output, 64, self.header.quality_bits);
        write_u32(output, 72, self.payload_len as u32);
        write_u32(output, 76, crc32(self.payload()));
        output[HEADER_BYTES..encoded_len].copy_from_slice(self.payload());
        Ok(encoded_len)
    }

    pub fn decode(datagram: &[u8]) -> Result<Self, FrameError> {
        if datagram.len() < HEADER_BYTES {
            return Err(FrameError::DatagramTooShort {
                minimum: HEADER_BYTES,
                actual: datagram.len(),
            });
        }
        if datagram.len() > MAX_DATAGRAM_BYTES {
            return Err(FrameError::DatagramTooLarge {
                maximum: MAX_DATAGRAM_BYTES,
                actual: datagram.len(),
            });
        }

        let magic = read_u32(datagram, 0);
        if magic != IPC_MAGIC {
            return Err(FrameError::MagicMismatch(magic));
        }
        let version = read_u16(datagram, 4);
        if version != IPC_VERSION {
            return Err(FrameError::ProtocolVersionMismatch(version));
        }
        let header_bytes = read_u16(datagram, 6);
        if usize::from(header_bytes) != HEADER_BYTES {
            return Err(FrameError::HeaderSizeMismatch(header_bytes));
        }
        let kind = MessageKind::try_from(read_u16(datagram, 8))?;
        let reserved = read_u16(datagram, 10);
        if reserved != 0 {
            return Err(FrameError::ReservedBitsSet(reserved));
        }

        let payload_len = read_u32(datagram, 72) as usize;
        if payload_len > MAX_PAYLOAD_BYTES {
            return Err(FrameError::PayloadTooLarge {
                maximum: MAX_PAYLOAD_BYTES,
                actual: payload_len,
            });
        }
        let expected_len = HEADER_BYTES + payload_len;
        if datagram.len() != expected_len {
            return Err(FrameError::LengthMismatch {
                expected: expected_len,
                actual: datagram.len(),
            });
        }

        let header = IpcHeader {
            kind,
            payload_schema_version: read_u32(datagram, 12),
            layout_hash: read_u64(datagram, 16),
            robot_id: read_u64(datagram, 24),
            boot_id: read_u64(datagram, 32),
            source_id: read_u64(datagram, 40),
            sequence: read_u64(datagram, 48),
            monotonic_time_ns: read_u64(datagram, 56),
            quality_bits: read_u64(datagram, 64),
        };
        validate_header(&header)?;
        validate_payload(kind, payload_len)?;

        let payload = &datagram[HEADER_BYTES..];
        let expected_crc = read_u32(datagram, 76);
        let actual_crc = crc32(payload);
        if actual_crc != expected_crc {
            return Err(FrameError::PayloadCrcMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }
        Self::new(header, payload)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    DatagramTooShort { minimum: usize, actual: usize },
    DatagramTooLarge { maximum: usize, actual: usize },
    OutputTooSmall { required: usize, actual: usize },
    MagicMismatch(u32),
    ProtocolVersionMismatch(u16),
    HeaderSizeMismatch(u16),
    UnknownMessageKind(u16),
    ReservedBitsSet(u16),
    ZeroPayloadSchemaVersion,
    ZeroLayoutHash,
    ZeroRobotId,
    ZeroBootId,
    ZeroSourceId,
    ZeroSequence,
    PayloadTooLarge { maximum: usize, actual: usize },
    LengthMismatch { expected: usize, actual: usize },
    HeartbeatPayloadNotEmpty,
    PayloadRequired,
    PayloadCrcMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid ESOP IPC frame: {self:?}")
    }
}

impl std::error::Error for FrameError {}

fn validate_header(header: &IpcHeader) -> Result<(), FrameError> {
    if header.payload_schema_version == 0 {
        return Err(FrameError::ZeroPayloadSchemaVersion);
    }
    if header.layout_hash == 0 {
        return Err(FrameError::ZeroLayoutHash);
    }
    if header.robot_id == 0 {
        return Err(FrameError::ZeroRobotId);
    }
    if header.boot_id == 0 {
        return Err(FrameError::ZeroBootId);
    }
    if header.source_id == 0 {
        return Err(FrameError::ZeroSourceId);
    }
    if header.sequence == 0 {
        return Err(FrameError::ZeroSequence);
    }
    Ok(())
}

fn validate_payload(kind: MessageKind, payload_len: usize) -> Result<(), FrameError> {
    if payload_len > MAX_PAYLOAD_BYTES {
        return Err(FrameError::PayloadTooLarge {
            maximum: MAX_PAYLOAD_BYTES,
            actual: payload_len,
        });
    }
    if kind == MessageKind::Heartbeat {
        if payload_len != 0 {
            return Err(FrameError::HeartbeatPayloadNotEmpty);
        }
    } else if payload_len == 0 {
        return Err(FrameError::PayloadRequired);
    }
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (CRC32_POLYNOMIAL & mask);
        }
    }
    !crc
}

fn write_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([input[offset], input[offset + 1]])
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
        input[offset + 4],
        input[offset + 5],
        input[offset + 6],
        input[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: MessageKind) -> IpcHeader {
        IpcHeader::new(kind, 7, 0x1122, 23, 29, 31, 37, 41, 0x55aa)
    }

    #[test]
    fn maximum_frame_round_trips() {
        let payload = [0xa5; MAX_PAYLOAD_BYTES];
        let frame = IpcFrame::new(header(MessageKind::State), &payload).unwrap();
        let mut encoded = [0; MAX_DATAGRAM_BYTES];
        let len = frame.encode_into(&mut encoded).unwrap();

        assert_eq!(len, MAX_DATAGRAM_BYTES);
        assert_eq!(IpcFrame::decode(&encoded), Ok(frame));
    }

    #[test]
    fn golden_header_uses_stable_little_endian_offsets() {
        let frame = IpcFrame::new(header(MessageKind::Command), b"123456789").unwrap();
        let mut encoded = [0; 128];
        let len = frame.encode_into(&mut encoded).unwrap();

        assert_eq!(len, 89);
        assert_eq!(&encoded[0..4], b"ESIP");
        assert_eq!(&encoded[4..6], &[1, 0]);
        assert_eq!(&encoded[6..8], &[80, 0]);
        assert_eq!(&encoded[8..12], &[1, 0, 0, 0]);
        assert_eq!(&encoded[12..16], &[7, 0, 0, 0]);
        assert_eq!(&encoded[16..24], &0x1122_u64.to_le_bytes());
        assert_eq!(&encoded[24..32], &23_u64.to_le_bytes());
        assert_eq!(&encoded[32..40], &29_u64.to_le_bytes());
        assert_eq!(&encoded[40..48], &31_u64.to_le_bytes());
        assert_eq!(&encoded[48..56], &37_u64.to_le_bytes());
        assert_eq!(&encoded[56..64], &41_u64.to_le_bytes());
        assert_eq!(&encoded[64..72], &0x55aa_u64.to_le_bytes());
        assert_eq!(&encoded[72..76], &9_u32.to_le_bytes());
        assert_eq!(&encoded[76..80], &0xcbf4_3926_u32.to_le_bytes());
        assert_eq!(&encoded[80..89], b"123456789");
    }

    #[test]
    fn malformed_frames_have_typed_errors() {
        let frame = IpcFrame::new(header(MessageKind::Event), b"event").unwrap();
        let mut encoded = [0; 128];
        let len = frame.encode_into(&mut encoded).unwrap();

        let mut bad = encoded[..len].to_vec();
        bad[0] = b'X';
        assert!(matches!(
            IpcFrame::decode(&bad),
            Err(FrameError::MagicMismatch(_))
        ));

        let mut bad = encoded[..len].to_vec();
        bad[4..6].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            IpcFrame::decode(&bad),
            Err(FrameError::ProtocolVersionMismatch(2))
        );

        let mut bad = encoded[..len].to_vec();
        bad[6..8].copy_from_slice(&79_u16.to_le_bytes());
        assert_eq!(
            IpcFrame::decode(&bad),
            Err(FrameError::HeaderSizeMismatch(79))
        );

        let mut bad = encoded[..len].to_vec();
        bad[8..10].copy_from_slice(&99_u16.to_le_bytes());
        assert_eq!(
            IpcFrame::decode(&bad),
            Err(FrameError::UnknownMessageKind(99))
        );

        let mut bad = encoded[..len].to_vec();
        bad[10] = 1;
        assert_eq!(IpcFrame::decode(&bad), Err(FrameError::ReservedBitsSet(1)));

        let mut bad = encoded[..len].to_vec();
        bad[72..76].copy_from_slice(&((MAX_PAYLOAD_BYTES + 1) as u32).to_le_bytes());
        assert_eq!(
            IpcFrame::decode(&bad),
            Err(FrameError::PayloadTooLarge {
                maximum: MAX_PAYLOAD_BYTES,
                actual: MAX_PAYLOAD_BYTES + 1,
            })
        );

        let mut bad = encoded[..len].to_vec();
        bad[80] ^= 1;
        assert!(matches!(
            IpcFrame::decode(&bad),
            Err(FrameError::PayloadCrcMismatch { .. })
        ));

        assert!(matches!(
            IpcFrame::decode(&encoded[..len - 1]),
            Err(FrameError::LengthMismatch { .. })
        ));
        assert_eq!(
            IpcFrame::decode(&encoded[..HEADER_BYTES - 1]),
            Err(FrameError::DatagramTooShort {
                minimum: HEADER_BYTES,
                actual: HEADER_BYTES - 1,
            })
        );
    }

    #[test]
    fn decoder_rejects_every_zero_required_field() {
        let frame = IpcFrame::new(header(MessageKind::State), b"state").unwrap();
        let mut encoded = [0; 128];
        let len = frame.encode_into(&mut encoded).unwrap();
        let cases = [
            (12, 4, FrameError::ZeroPayloadSchemaVersion),
            (16, 8, FrameError::ZeroLayoutHash),
            (24, 8, FrameError::ZeroRobotId),
            (32, 8, FrameError::ZeroBootId),
            (40, 8, FrameError::ZeroSourceId),
            (48, 8, FrameError::ZeroSequence),
        ];

        for (offset, width, expected) in cases {
            let mut bad = encoded[..len].to_vec();
            bad[offset..offset + width].fill(0);
            assert_eq!(IpcFrame::decode(&bad), Err(expected));
        }
    }

    #[test]
    fn decoder_rejects_payload_shape_and_trailing_bytes() {
        let heartbeat = IpcFrame::new(header(MessageKind::Heartbeat), b"").unwrap();
        let mut encoded = [0; 128];
        let heartbeat_len = heartbeat.encode_into(&mut encoded).unwrap();
        encoded[72..76].copy_from_slice(&1_u32.to_le_bytes());
        encoded[heartbeat_len] = 0;
        assert_eq!(
            IpcFrame::decode(&encoded[..heartbeat_len + 1]),
            Err(FrameError::HeartbeatPayloadNotEmpty)
        );

        let state = IpcFrame::new(header(MessageKind::State), b"state").unwrap();
        let state_len = state.encode_into(&mut encoded).unwrap();
        encoded[72..76].fill(0);
        assert_eq!(
            IpcFrame::decode(&encoded[..HEADER_BYTES]),
            Err(FrameError::PayloadRequired)
        );

        state.encode_into(&mut encoded).unwrap();
        encoded[state_len] = 0;
        assert_eq!(
            IpcFrame::decode(&encoded[..state_len + 1]),
            Err(FrameError::LengthMismatch {
                expected: state_len,
                actual: state_len + 1,
            })
        );
    }

    #[test]
    fn semantic_invariants_are_rejected() {
        assert_eq!(
            IpcFrame::new(header(MessageKind::Heartbeat), b"not-empty"),
            Err(FrameError::HeartbeatPayloadNotEmpty)
        );
        assert_eq!(
            IpcFrame::new(header(MessageKind::Diagnostic), b""),
            Err(FrameError::PayloadRequired)
        );

        let mut invalid = header(MessageKind::State);
        invalid.sequence = 0;
        assert_eq!(
            IpcFrame::new(invalid, b"state"),
            Err(FrameError::ZeroSequence)
        );
    }

    #[test]
    fn output_capacity_is_checked_before_writing() {
        let frame = IpcFrame::new(header(MessageKind::State), b"state").unwrap();
        let mut output = [0xaa; HEADER_BYTES];
        assert_eq!(
            frame.encode_into(&mut output),
            Err(FrameError::OutputTooSmall {
                required: HEADER_BYTES + 5,
                actual: HEADER_BYTES,
            })
        );
        assert_eq!(output, [0xaa; HEADER_BYTES]);
    }
}
