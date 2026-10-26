//! Hosted capture and exact deterministic replay. Allocation is confined to
//! construction/loading/export; recording uses one preallocated bounded array.
use esop_ethercat_core::{
    EthercatPort, LinkState, RxPoll,
    wire::{FrameView, MAX_ETHERNET_FRAME_LEN},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const PCAP_HEADER: usize = 24;
const RECORD_HEADER: usize = 16;
const MAX_CAPTURE_RECORDS: usize = 500_000;
const MAX_SIDECAR_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Tx,
    Rx,
}

#[derive(Clone)]
pub struct CapturedFrame {
    pub timestamp_ns: u64,
    pub direction: Direction,
    len: usize,
    bytes: [u8; MAX_ETHERNET_FRAME_LEN],
}

impl CapturedFrame {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBinding {
    pub schema_version: String,
    pub product_sha256: String,
    pub scenario: String,
    pub clock_origin_ns: u64,
    pub period_ns: u64,
    pub cycles: u64,
}

impl CaptureBinding {
    pub fn new(product_sha256: String, scenario: String, period_ns: u64, cycles: u64) -> Self {
        Self {
            schema_version: "esop.capture.v1".into(),
            product_sha256,
            scenario,
            clock_origin_ns: 0,
            period_ns,
            cycles,
        }
    }
    fn validate(&self) -> Result<(), CaptureError> {
        if self.schema_version != "esop.capture.v1"
            || self.period_ns == 0
            || self.cycles == 0
            || self.scenario.is_empty()
            || self.scenario.len() > 128
            || self.product_sha256.len() != 64
            || !self
                .product_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(CaptureError::Binding);
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sidecar {
    binding: CaptureBinding,
    pcap_sha256: String,
    record_count: usize,
    lost_records: u64,
    directions: Vec<Direction>,
}

#[derive(Debug)]
pub enum CaptureError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Binding,
    Capacity,
    Container,
    HashMismatch,
    DirectionMismatch,
    TimeRegression,
    IncompleteCapture,
}
impl From<std::io::Error> for CaptureError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for CaptureError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

pub struct CaptureTrace {
    binding: CaptureBinding,
    frames: Vec<CapturedFrame>,
    limit: usize,
    lost: u64,
    streaming: Sha256,
    total_records: u64,
    round_trip: RoundTripObserver,
}

struct RoundTripObserver {
    transmitted_at: [Option<u64>; 256],
    // The wire index carries no generation. Once two unanswered TXs share an
    // index, no later RX can establish which TX it answered. Keep that index
    // unavailable for the rest of this observer/session, including cycle reset.
    tainted: [bool; 256],
    max_ns: u64,
    samples: u64,
}
impl RoundTripObserver {
    const fn new() -> Self {
        Self {
            transmitted_at: [None; 256],
            tainted: [false; 256],
            max_ns: 0,
            samples: 0,
        }
    }
    fn record(&mut self, is_tx: bool, now_ns: u64, bytes: &[u8]) {
        let Ok(frame) = FrameView::parse(bytes) else {
            return;
        };
        for datagram in frame.datagrams() {
            let Ok(datagram) = datagram else {
                break;
            };
            let index = usize::from(datagram.header.index);
            if self.tainted[index] {
                continue;
            }
            let slot = &mut self.transmitted_at[index];
            if is_tx {
                if slot.is_some() {
                    self.tainted[index] = true;
                    *slot = None;
                } else {
                    *slot = Some(now_ns);
                }
            } else if let Some(sent) = slot.take() {
                if let Some(elapsed) = now_ns.checked_sub(sent) {
                    self.max_ns = self.max_ns.max(elapsed);
                    self.samples = self.samples.saturating_add(1);
                }
            }
        }
    }
    fn reset(&mut self) {
        self.max_ns = 0;
        self.samples = 0;
    }
    fn observed(&self) -> Option<u64> {
        (self.samples != 0).then_some(self.max_ns)
    }
}
impl CaptureTrace {
    pub fn new(binding: CaptureBinding, limit: usize) -> Result<Self, CaptureError> {
        binding.validate()?;
        if limit == 0 || limit > MAX_CAPTURE_RECORDS {
            return Err(CaptureError::Capacity);
        }
        Ok(Self {
            binding,
            frames: Vec::with_capacity(limit),
            limit,
            lost: 0,
            streaming: Sha256::new(),
            total_records: 0,
            round_trip: RoundTripObserver::new(),
        })
    }
    pub fn record(&mut self, timestamp_ns: u64, direction: Direction, bytes: &[u8]) {
        if bytes.len() <= MAX_ETHERNET_FRAME_LEN {
            self.streaming
                .update([u8::from(direction == Direction::Tx)]);
            self.streaming.update(timestamp_ns.to_le_bytes());
            self.streaming.update((bytes.len() as u64).to_le_bytes());
            self.streaming.update(bytes);
            self.total_records = self.total_records.saturating_add(1);
            self.round_trip
                .record(direction == Direction::Tx, timestamp_ns, bytes);
        }
        if bytes.len() > MAX_ETHERNET_FRAME_LEN
            || self.frames.len() == self.limit
            || self
                .frames
                .last()
                .is_some_and(|f| f.timestamp_ns > timestamp_ns)
        {
            self.lost = self.lost.saturating_add(1);
            return;
        }
        let mut frame = CapturedFrame {
            timestamp_ns,
            direction,
            len: bytes.len(),
            bytes: [0; MAX_ETHERNET_FRAME_LEN],
        };
        frame.bytes[..bytes.len()].copy_from_slice(bytes);
        self.frames.push(frame);
    }
    pub fn frames(&self) -> &[CapturedFrame] {
        &self.frames
    }
    pub const fn lost_records(&self) -> u64 {
        self.lost
    }
    pub const fn total_records(&self) -> u64 {
        self.total_records
    }
    pub fn digest_bytes(&self) -> [u8; 32] {
        let mut digest = [0; 32];
        digest.copy_from_slice(&self.streaming.clone().finalize());
        digest
    }
    pub fn reset_round_trip(&mut self) {
        self.round_trip.reset();
    }
    pub fn observed_round_trip_ns(&self) -> Option<u64> {
        self.round_trip.observed()
    }
    pub fn digest(&self) -> String {
        hex(&self.digest_bytes())
    }
    pub fn write(&self, pcap: &Path, sidecar: &Path) -> Result<(), CaptureError> {
        let mut bytes = Vec::with_capacity(
            PCAP_HEADER
                + self
                    .frames
                    .iter()
                    .map(|f| RECORD_HEADER + f.len)
                    .sum::<usize>(),
        );
        // Nanosecond-resolution little-endian classic Ethernet PCAP.
        bytes.extend_from_slice(&0xa1b23c4du32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&(MAX_ETHERNET_FRAME_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for frame in &self.frames {
            let relative = frame
                .timestamp_ns
                .checked_sub(self.binding.clock_origin_ns)
                .ok_or(CaptureError::TimeRegression)?;
            let seconds =
                u32::try_from(relative / 1_000_000_000).map_err(|_| CaptureError::Container)?;
            bytes.extend_from_slice(&seconds.to_le_bytes());
            bytes.extend_from_slice(&((relative % 1_000_000_000) as u32).to_le_bytes());
            bytes.extend_from_slice(&(frame.len as u32).to_le_bytes());
            bytes.extend_from_slice(&(frame.len as u32).to_le_bytes());
            bytes.extend_from_slice(frame.bytes());
        }
        let metadata = Sidecar {
            binding: self.binding.clone(),
            pcap_sha256: hash(&bytes),
            record_count: self.frames.len(),
            lost_records: self.lost,
            directions: self.frames.iter().map(|f| f.direction).collect(),
        };
        fs::write(pcap, &bytes)?;
        fs::write(sidecar, serde_json::to_vec_pretty(&metadata)?)?;
        Ok(())
    }
    pub fn load(
        pcap: &Path,
        sidecar: &Path,
        expected: &CaptureBinding,
    ) -> Result<Self, CaptureError> {
        if fs::metadata(sidecar)?.len() > MAX_SIDECAR_BYTES {
            return Err(CaptureError::Capacity);
        }
        let metadata: Sidecar = serde_json::from_slice(&fs::read(sidecar)?)?;
        metadata.binding.validate()?;
        if &metadata.binding != expected {
            return Err(CaptureError::Binding);
        }
        if metadata.lost_records != 0 {
            return Err(CaptureError::IncompleteCapture);
        }
        if metadata.record_count == 0
            || metadata.record_count > MAX_CAPTURE_RECORDS
            || metadata.directions.len() != metadata.record_count
        {
            return Err(CaptureError::DirectionMismatch);
        }
        let maximum_bytes =
            PCAP_HEADER + metadata.record_count * (RECORD_HEADER + MAX_ETHERNET_FRAME_LEN);
        if fs::metadata(pcap)?.len() > maximum_bytes as u64 {
            return Err(CaptureError::Container);
        }
        let bytes = fs::read(pcap)?;
        if hash(&bytes) != metadata.pcap_sha256 {
            return Err(CaptureError::HashMismatch);
        }
        if bytes.len() < PCAP_HEADER
            || bytes[..4] != 0xa1b23c4du32.to_le_bytes()
            || read_u16(&bytes, 4)? != 2
            || read_u16(&bytes, 6)? != 4
            || read_u32(&bytes, 8)? != 0
            || read_u32(&bytes, 12)? != 0
            || read_u32(&bytes, 16)? != MAX_ETHERNET_FRAME_LEN as u32
            || read_u32(&bytes, 20)? != 1
        {
            return Err(CaptureError::Container);
        }
        let mut trace = Self::new(metadata.binding, metadata.record_count)?;
        let mut offset = PCAP_HEADER;
        for direction in metadata.directions {
            let seconds = read_u32(&bytes, offset)?;
            let nanos = read_u32(&bytes, offset + 4)?;
            let len = read_u32(&bytes, offset + 8)? as usize;
            if nanos >= 1_000_000_000
                || len > MAX_ETHERNET_FRAME_LEN
                || len != read_u32(&bytes, offset + 12)? as usize
            {
                return Err(CaptureError::Container);
            }
            offset = offset
                .checked_add(RECORD_HEADER)
                .ok_or(CaptureError::Container)?;
            let end = offset.checked_add(len).ok_or(CaptureError::Container)?;
            let payload = bytes.get(offset..end).ok_or(CaptureError::Container)?;
            let timestamp = trace
                .binding
                .clock_origin_ns
                .checked_add(u64::from(seconds) * 1_000_000_000)
                .and_then(|n| n.checked_add(u64::from(nanos)))
                .ok_or(CaptureError::Container)?;
            if trace
                .frames
                .last()
                .is_some_and(|f| f.timestamp_ns > timestamp)
            {
                return Err(CaptureError::TimeRegression);
            }
            trace.record(timestamp, direction, payload);
            offset = end;
        }
        if offset != bytes.len() {
            return Err(CaptureError::Container);
        }
        Ok(trace)
    }
}
fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, CaptureError> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or(CaptureError::Container)?
            .try_into()
            .map_err(|_| CaptureError::Container)?,
    ))
}
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, CaptureError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(CaptureError::Container)?
            .try_into()
            .map_err(|_| CaptureError::Container)?,
    ))
}

pub struct RecordingPort<P> {
    pub port: P,
    pub trace: CaptureTrace,
}
impl<P: EthercatPort> EthercatPort for RecordingPort<P> {
    type Error = P::Error;
    fn link_state(&self) -> LinkState {
        self.port.link_state()
    }
    fn now_ns(&self) -> u64 {
        self.port.now_ns()
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.port.tx_submit(frame)?;
        self.trace.record(self.port.now_ns(), Direction::Tx, frame);
        Ok(())
    }
    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        let result = self.port.rx_poll(destination)?;
        if let RxPoll::Frame(len) = result {
            if let Some(bytes) = destination.get(..len) {
                self.trace.record(self.port.now_ns(), Direction::Rx, bytes);
            } else {
                self.trace.lost = self.trace.lost.saturating_add(1);
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayError {
    TimeRegression,
    UnexpectedTx,
    TxMismatch,
    TxTimeMismatch,
    UnconsumedRecords,
}
pub struct ReplayPort {
    trace: CaptureTrace,
    cursor: usize,
    now_ns: u64,
    round_trip: RoundTripObserver,
}
impl ReplayPort {
    pub fn new(trace: CaptureTrace) -> Self {
        let now_ns = trace.binding.clock_origin_ns;
        Self {
            trace,
            cursor: 0,
            now_ns,
            round_trip: RoundTripObserver::new(),
        }
    }
    pub fn advance_to(&mut self, now_ns: u64) -> Result<(), ReplayError> {
        if now_ns < self.now_ns {
            return Err(ReplayError::TimeRegression);
        }
        self.now_ns = now_ns;
        Ok(())
    }
    pub fn finish(&self) -> Result<(), ReplayError> {
        if self.cursor == self.trace.frames.len() {
            Ok(())
        } else {
            Err(ReplayError::UnconsumedRecords)
        }
    }
    pub fn digest(&self) -> String {
        self.trace.digest()
    }
    pub fn frames(&self) -> &[CapturedFrame] {
        self.trace.frames()
    }
    pub fn total_records(&self) -> u64 {
        self.trace.total_records()
    }
    pub fn digest_bytes(&self) -> [u8; 32] {
        self.trace.digest_bytes()
    }
    pub fn reset_round_trip(&mut self) {
        self.round_trip.reset();
    }
    pub fn observed_round_trip_ns(&self) -> Option<u64> {
        self.round_trip.observed()
    }
}
impl EthercatPort for ReplayPort {
    type Error = ReplayError;
    fn link_state(&self) -> LinkState {
        LinkState::Up
    }
    fn now_ns(&self) -> u64 {
        self.now_ns
    }
    fn tx_submit(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let frame = self
            .trace
            .frames
            .get(self.cursor)
            .ok_or(ReplayError::UnexpectedTx)?;
        if frame.direction != Direction::Tx {
            return Err(ReplayError::UnexpectedTx);
        }
        if frame.timestamp_ns != self.now_ns {
            return Err(ReplayError::TxTimeMismatch);
        }
        if frame.bytes() != bytes {
            return Err(ReplayError::TxMismatch);
        }
        self.round_trip.record(true, self.now_ns, bytes);
        self.cursor += 1;
        Ok(())
    }
    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, Self::Error> {
        let Some(frame) = self.trace.frames.get(self.cursor) else {
            return Ok(RxPoll::Empty);
        };
        if frame.direction != Direction::Rx || frame.timestamp_ns > self.now_ns {
            return Ok(RxPoll::Empty);
        }
        destination[..frame.len].copy_from_slice(frame.bytes());
        self.round_trip.record(false, self.now_ns, frame.bytes());
        self.cursor += 1;
        Ok(RxPoll::Frame(frame.len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> CaptureBinding {
        CaptureBinding::new("42".repeat(32), "test".into(), 1000, 2)
    }
    #[test]
    fn exact_tx_and_timestamped_rx_never_patch_recorded_bytes() {
        let mut trace = CaptureTrace::new(binding(), 4).unwrap();
        trace.record(0, Direction::Tx, &[1, 2]);
        trace.record(10, Direction::Rx, &[3, 0]);
        let mut port = ReplayPort::new(trace);
        let mut bytes = [0; MAX_ETHERNET_FRAME_LEN];
        assert_eq!(port.tx_submit(&[1, 9]), Err(ReplayError::TxMismatch));
        assert_eq!(port.tx_submit(&[1, 2]), Ok(()));
        assert_eq!(port.rx_poll(&mut bytes), Ok(RxPoll::Empty));
        assert_eq!(port.finish(), Err(ReplayError::UnconsumedRecords));
        port.advance_to(10).unwrap();
        assert_eq!(port.rx_poll(&mut bytes), Ok(RxPoll::Frame(2)));
        assert_eq!(&bytes[..2], &[3, 0]);
        assert_eq!(port.finish(), Ok(()));
    }
    #[test]
    fn preallocated_overflow_is_counted_and_time_regression_rejected() {
        let mut trace = CaptureTrace::new(binding(), 1).unwrap();
        trace.record(4, Direction::Tx, &[1]);
        trace.record(3, Direction::Rx, &[2]);
        assert_eq!(trace.frames.len(), 1);
        assert_eq!(trace.lost_records(), 1);
    }
    #[test]
    fn full_stream_digest_and_count_survive_bounded_capture_overflow() {
        let mut prefix = CaptureTrace::new(binding(), 1).unwrap();
        let mut full = CaptureTrace::new(binding(), 4).unwrap();
        for (time, direction, bytes) in [
            (0, Direction::Tx, &[1u8, 2][..]),
            (1, Direction::Rx, &[3u8, 4][..]),
            (2, Direction::Tx, &[5u8, 6][..]),
        ] {
            prefix.record(time, direction, bytes);
            full.record(time, direction, bytes);
        }
        assert_eq!(prefix.frames().len(), 1);
        assert_eq!(prefix.lost_records(), 2);
        assert_eq!(prefix.total_records(), 3);
        assert_eq!(prefix.digest_bytes(), full.digest_bytes());
    }
    #[test]
    fn pcap_roundtrip_rejects_binding_tamper_and_loss() {
        let directory = std::env::temp_dir().join(format!("esop-pcap-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let pcap = directory.join("trace.pcap");
        let sidecar = directory.join("trace.json");
        let mut trace = CaptureTrace::new(binding(), 4).unwrap();
        trace.record(0, Direction::Tx, &[0, 1]);
        trace.record(1, Direction::Rx, &[2, 3]);
        trace.write(&pcap, &sidecar).unwrap();
        let loaded = CaptureTrace::load(&pcap, &sidecar, &binding()).unwrap();
        assert_eq!(loaded.digest(), trace.digest());
        let mut wrong = binding();
        wrong.cycles += 1;
        assert!(matches!(
            CaptureTrace::load(&pcap, &sidecar, &wrong),
            Err(CaptureError::Binding)
        ));
        let mut bytes = fs::read(&pcap).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&pcap, bytes).unwrap();
        assert!(matches!(
            CaptureTrace::load(&pcap, &sidecar, &binding()),
            Err(CaptureError::HashMismatch)
        ));
        fs::remove_dir_all(directory).unwrap();
    }
    fn literal_datagram(index: u8) -> [u8; 64] {
        let mut bytes = [0u8; 64];
        bytes[..30].copy_from_slice(&[
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1, 2, 3, 4, 5, 6, 0x88, 0xa4, 0x0e, 0x10, 0x0a,
            index, 0, 0x10, 0, 0, 2, 0, 0, 0, 0xaa, 0xbb, 1, 0,
        ]);
        bytes
    }

    #[test]
    fn literal_rtt_matches_indices_not_poll_count_or_previous_rx() {
        let mut trace = CaptureTrace::new(binding(), 16).unwrap();
        trace.record(100, Direction::Tx, &literal_datagram(7));
        trace.record(110, Direction::Tx, &literal_datagram(9));
        trace.record(125, Direction::Rx, &literal_datagram(8)); // no TX for index8
        assert_eq!(trace.observed_round_trip_ns(), None);
        trace.record(150, Direction::Rx, &literal_datagram(7));
        assert_eq!(trace.observed_round_trip_ns(), Some(50));
        trace.record(160, Direction::Rx, &literal_datagram(7)); // duplicate cannot match twice
        assert_eq!(trace.round_trip.samples, 1);
        trace.record(200, Direction::Rx, &literal_datagram(9));
        assert_eq!(trace.observed_round_trip_ns(), Some(90));
        trace.record(210, Direction::Tx, &literal_datagram(3));
        trace.reset_round_trip(); // retain a prior-cycle transmitted PDO timestamp
        assert_eq!(trace.observed_round_trip_ns(), None);
        trace.record(250, Direction::Rx, &literal_datagram(3));
        assert_eq!(trace.observed_round_trip_ns(), Some(40));
    }

    #[test]
    fn reused_wire_index_cannot_shorten_an_old_responses_round_trip() {
        let old = literal_datagram(7);
        let mut new = literal_datagram(7);
        new[18..22].copy_from_slice(&0x2000u32.to_le_bytes());
        let mut trace = CaptureTrace::new(binding(), 8).unwrap();
        for (time, direction, bytes) in [
            (0, Direction::Tx, &old),
            (5, Direction::Tx, &new),
            (10, Direction::Rx, &old),
            (15, Direction::Rx, &new),
        ] {
            trace.record(time, direction, bytes);
        }
        let recorded_round_trip = trace.observed_round_trip_ns();
        let mut replay = ReplayPort::new(trace);
        let mut bytes = [0; MAX_ETHERNET_FRAME_LEN];
        replay.tx_submit(&old).unwrap();
        replay.advance_to(5).unwrap();
        replay.tx_submit(&new).unwrap();
        replay.advance_to(10).unwrap();
        assert_eq!(replay.rx_poll(&mut bytes), Ok(RxPoll::Frame(64)));
        assert_eq!(&bytes[..64], &old);
        replay.advance_to(15).unwrap();
        assert_eq!(replay.rx_poll(&mut bytes), Ok(RxPoll::Frame(64)));
        assert_eq!(&bytes[..64], &new);
        assert_eq!(replay.finish(), Ok(()));
        assert_eq!(replay.observed_round_trip_ns(), recorded_round_trip);
        // Each pair takes ten ns, but the bounded index-only observer cannot
        // establish ownership after overlap. It must not invent a five-ns RTT.
        assert_eq!(recorded_round_trip, None);
    }

    #[test]
    fn overlapping_index_taint_survives_cycle_reset_and_later_rx_drain() {
        let reused = literal_datagram(7);
        let other = literal_datagram(9);
        let mut trace = CaptureTrace::new(binding(), 16).unwrap();
        trace.record(0, Direction::Tx, &reused);
        trace.record(5, Direction::Tx, &reused);
        trace.reset_round_trip();
        trace.record(10, Direction::Rx, &reused);
        trace.record(15, Direction::Rx, &reused);
        trace.record(20, Direction::Tx, &reused);
        trace.record(25, Direction::Rx, &reused);
        assert_eq!(trace.observed_round_trip_ns(), None);
        trace.record(30, Direction::Tx, &other);
        trace.record(40, Direction::Rx, &other);
        assert_eq!(trace.observed_round_trip_ns(), Some(10));
        trace.reset_round_trip();
        trace.record(50, Direction::Tx, &reused);
        trace.record(60, Direction::Rx, &reused);
        assert_eq!(trace.observed_round_trip_ns(), None);

        let mut replay = ReplayPort::new(trace);
        let mut bytes = [0; MAX_ETHERNET_FRAME_LEN];
        replay.tx_submit(&reused).unwrap();
        replay.advance_to(5).unwrap();
        replay.tx_submit(&reused).unwrap();
        replay.reset_round_trip();
        for time in [10, 15] {
            replay.advance_to(time).unwrap();
            assert_eq!(replay.rx_poll(&mut bytes), Ok(RxPoll::Frame(64)));
        }
        replay.advance_to(20).unwrap();
        replay.tx_submit(&reused).unwrap();
        replay.advance_to(25).unwrap();
        replay.rx_poll(&mut bytes).unwrap();
        assert_eq!(replay.observed_round_trip_ns(), None);
        replay.advance_to(30).unwrap();
        replay.tx_submit(&other).unwrap();
        replay.advance_to(40).unwrap();
        replay.rx_poll(&mut bytes).unwrap();
        assert_eq!(replay.observed_round_trip_ns(), Some(10));
        replay.reset_round_trip();
        replay.advance_to(50).unwrap();
        replay.tx_submit(&reused).unwrap();
        replay.advance_to(60).unwrap();
        replay.rx_poll(&mut bytes).unwrap();
        assert_eq!(replay.observed_round_trip_ns(), None);
        assert_eq!(replay.finish(), Ok(()));

        let mut fresh = CaptureTrace::new(binding(), 2).unwrap();
        fresh.record(0, Direction::Tx, &reused);
        fresh.record(10, Direction::Rx, &reused);
        assert_eq!(fresh.observed_round_trip_ns(), Some(10));
    }

    #[test]
    fn recording_port_uses_actual_post_operation_timestamps_and_rejects_oversize_rx_without_panicking()
     {
        struct TimedPort {
            now: u64,
            pending: bool,
            oversize: bool,
        }
        impl EthercatPort for TimedPort {
            type Error = core::convert::Infallible;
            fn now_ns(&self) -> u64 {
                self.now
            }
            fn link_state(&self) -> LinkState {
                LinkState::Up
            }
            fn tx_submit(&mut self, _: &[u8]) -> Result<(), Self::Error> {
                self.now += 10;
                self.pending = true;
                Ok(())
            }
            fn rx_poll(
                &mut self,
                destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
            ) -> Result<RxPoll, Self::Error> {
                if !self.pending {
                    return Ok(RxPoll::Empty);
                }
                self.now += 30;
                self.pending = false;
                if self.oversize {
                    return Ok(RxPoll::Frame(MAX_ETHERNET_FRAME_LEN + 1));
                }
                destination[..64].copy_from_slice(&literal_datagram(7));
                Ok(RxPoll::Frame(64))
            }
        }
        let mut port = RecordingPort {
            port: TimedPort {
                now: 0,
                pending: false,
                oversize: false,
            },
            trace: CaptureTrace::new(binding(), 4).unwrap(),
        };
        port.tx_submit(&literal_datagram(7)).unwrap();
        let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
        assert_eq!(port.rx_poll(&mut scratch), Ok(RxPoll::Frame(64)));
        assert_eq!(port.trace.frames()[0].timestamp_ns, 10);
        assert_eq!(port.trace.frames()[1].timestamp_ns, 40);
        assert_eq!(port.trace.observed_round_trip_ns(), Some(30));
        port.port.oversize = true;
        port.tx_submit(&literal_datagram(8)).unwrap();
        assert_eq!(
            port.rx_poll(&mut scratch),
            Ok(RxPoll::Frame(MAX_ETHERNET_FRAME_LEN + 1))
        );
        assert_eq!(port.trace.lost_records(), 1);
        assert_eq!(port.trace.frames().len(), 3);
    }

    #[test]
    fn malformed_container_with_matching_digest_cannot_bypass_structural_validation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "esop-pcap-container-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let pcap = directory.join("trace.pcap");
        let sidecar = directory.join("trace.json");
        let mut trace = CaptureTrace::new(binding(), 4).unwrap();
        trace.record(0, Direction::Tx, &literal_datagram(7));
        trace.record(1, Direction::Rx, &literal_datagram(7));
        trace.write(&pcap, &sidecar).unwrap();
        let original = fs::read(&pcap).unwrap();
        for case in 0..5 {
            let mut bytes = original.clone();
            match case {
                0 => bytes[20..24].copy_from_slice(&0u32.to_le_bytes()), // non-Ethernet network type
                1 => bytes[28..32].copy_from_slice(&1_000_000_000u32.to_le_bytes()), // invalid nanos
                2 => bytes[32..36].copy_from_slice(&65u32.to_le_bytes()), // incl_len != orig_len
                3 => {
                    bytes.pop();
                }   // truncated captured payload
                _ => bytes.push(0), // bytes after the declared final record
            }
            fs::write(&pcap, &bytes).unwrap();
            let metadata = Sidecar {
                binding: binding(),
                pcap_sha256: hash(&bytes),
                record_count: 2,
                lost_records: 0,
                directions: vec![Direction::Tx, Direction::Rx],
            };
            fs::write(&sidecar, serde_json::to_vec(&metadata).unwrap()).unwrap();
            assert!(
                matches!(
                    CaptureTrace::load(&pcap, &sidecar, &binding()),
                    Err(CaptureError::Container)
                ),
                "container case {case}"
            );
        }
        trace.write(&pcap, &sidecar).unwrap();
        let mut metadata: Sidecar = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        metadata.lost_records = 1;
        fs::write(&sidecar, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert!(matches!(
            CaptureTrace::load(&pcap, &sidecar, &binding()),
            Err(CaptureError::IncompleteCapture)
        ));
        metadata.lost_records = 0;
        metadata.directions.pop();
        fs::write(&sidecar, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert!(matches!(
            CaptureTrace::load(&pcap, &sidecar, &binding()),
            Err(CaptureError::DirectionMismatch)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replay_enforces_tx_time_and_cannot_retime_late_rx() {
        let tx = literal_datagram(7);
        let rx = literal_datagram(7);
        let mut trace = CaptureTrace::new(binding(), 4).unwrap();
        trace.record(0, Direction::Tx, &tx);
        trace.record(10, Direction::Rx, &rx);
        let mut replay = ReplayPort::new(trace);
        replay.advance_to(1).unwrap();
        assert_eq!(replay.tx_submit(&tx), Err(ReplayError::TxTimeMismatch));
        assert_eq!(replay.advance_to(0), Err(ReplayError::TimeRegression));
        let mut trace = CaptureTrace::new(binding(), 4).unwrap();
        trace.record(0, Direction::Tx, &tx);
        trace.record(10, Direction::Rx, &rx);
        let mut replay = ReplayPort::new(trace);
        replay.tx_submit(&tx).unwrap();
        replay.advance_to(100).unwrap();
        let mut scratch = [0; MAX_ETHERNET_FRAME_LEN];
        assert_eq!(replay.rx_poll(&mut scratch), Ok(RxPoll::Frame(64)));
        assert_eq!(replay.now_ns(), 100); // late data is not backdated into a valid deadline
        assert_eq!(&scratch[..64], &rx);
        assert_eq!(replay.observed_round_trip_ns(), Some(100));
    }

    #[test]
    fn late_replayed_rx_reaches_master_validation_at_the_actual_poll_time() {
        use esop_ethercat_core::{
            DatagramPlan, Domain, DomainSegment, EthercatMaster, FramePlan, MasterConfig,
            wire::Command,
        };
        let rx = literal_datagram(7);
        let mut tx = rx;
        tx[26..30].fill(0); // transmitted read payload and WKC are initially zero
        let mut trace = CaptureTrace::new(binding(), 4).unwrap();
        trace.record(0, Direction::Tx, &tx);
        trace.record(100, Direction::Rx, &rx);
        let mut replay = ReplayPort::new(trace);
        let mut master = EthercatMaster::<1, MAX_ETHERNET_FRAME_LEN>::new(MasterConfig::new(
            [0xff; 6],
            [1, 2, 3, 4, 5, 6],
        ));
        let mut domain = Domain::<2, 1>::new(0x1000);
        domain
            .add_segment(DomainSegment {
                datagram_index: 7,
                input_offset: 0,
                len: 2,
                expected_wkc: 1,
            })
            .unwrap();
        domain.begin_receive(1).unwrap();
        let mut plan = FramePlan::<1>::new();
        plan.push(DatagramPlan {
            command: Command::Lrd,
            index: 7,
            address: 0x1000,
            payload_offset: 0,
            payload_len: 2,
            expected_wkc: 1,
        })
        .unwrap();
        let frame = master.acquire_frame(1, 50).unwrap();
        master
            .build_and_arm_frame_from_plan(frame, &plan, &[0, 0])
            .unwrap();
        master.submit_frame(&mut replay, frame).unwrap();
        replay.advance_to(100).unwrap();
        let mut bytes = [0; MAX_ETHERNET_FRAME_LEN];
        let report = master
            .cycle_receive_with_consumer(&mut replay, &mut bytes, 1, &mut domain)
            .unwrap();
        assert_eq!(report.timed_out_datagrams, 1);
        assert_eq!(report.unmatched_datagrams, 1);
        assert!(!domain.finish_receive(1, report.cycle).unwrap());
        assert_eq!(domain.input(), &[0, 0]);
        assert_eq!(replay.finish(), Ok(()));
    }

    #[test]
    fn preload_bounds_are_enforced_before_json_or_container_allocation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("esop-pcap-bounds-{}-{unique}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let pcap = directory.join("trace.pcap");
        let sidecar = directory.join("trace.json");
        let maximum = PCAP_HEADER + 2 * (RECORD_HEADER + MAX_ETHERNET_FRAME_LEN);
        let bytes = vec![0u8; maximum + 1];
        fs::write(&pcap, &bytes).unwrap();
        let metadata = Sidecar {
            binding: binding(),
            pcap_sha256: hash(&bytes),
            record_count: 2,
            lost_records: 0,
            directions: vec![Direction::Tx, Direction::Rx],
        };
        fs::write(&sidecar, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert!(matches!(
            CaptureTrace::load(&pcap, &sidecar, &binding()),
            Err(CaptureError::Container)
        ));
        // A sparse oversize sidecar should fail its size bound, before the
        // zero-filled content can be read and reported as a JSON parse error.
        fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&sidecar)
            .unwrap()
            .set_len(MAX_SIDECAR_BYTES + 1)
            .unwrap();
        assert!(matches!(
            CaptureTrace::load(&pcap, &sidecar, &binding()),
            Err(CaptureError::Capacity)
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
