//! Bounded run measurements. Construction and JSON export are hosted setup /
//! teardown work; every record method uses only fixed storage and streaming SHA.
use esop_ethercat_core::CycleReport;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const HISTOGRAM_BUCKETS: usize = 512;
pub const HISTOGRAM_BUCKET_WIDTH_NS: u64 = 1_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSource {
    Sim,
    PcapReplay,
    LinuxRaw,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementBinding {
    pub source: MeasurementSource,
    pub esop_commit: String,
    pub config_sha256: String,
    pub topology_sha256: String,
    pub scenario: String,
    pub period_ns: u64,
    pub cycles: u64,
    pub clock_origin_ns: u64,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyMeasurements {
    pub slave_count: u64,
    pub axis_count: u64,
    pub io_channels: u64,
    pub pdo_bytes: u64,
    pub frame_count: u64,
    pub dc_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceMeasurements {
    pub core_arena_bytes: Option<u64>,
    pub procbuf_bytes: Option<u64>,
    pub dma_bytes: Option<u64>,
    pub rt_stack_peak_bytes: Option<u64>,
    pub text_rodata_bytes: Option<u64>,
    pub rt_cpu_percent: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub struct CycleTiming {
    pub release_jitter_abs_ns: u64,
    pub fast_path_ns: u64,
    /// Absent when no unambiguous TX/RX timing pair was observed this cycle.
    pub rx_round_trip_ns: Option<u64>,
}

#[derive(Serialize)]
pub struct Histogram {
    pub bucket_width_ns: u64,
    // Serialize slice explicitly: fixed arrays larger than32 are unsupported
    // by some serde versions. The activated collector itself is always fixed.
    #[serde(serialize_with = "serialize_buckets")]
    buckets: [u64; HISTOGRAM_BUCKETS],
    pub overflow: u64,
    pub max_ns: u64,
    pub samples: u64,
}
fn serialize_buckets<S: serde::Serializer>(
    value: &[u64; HISTOGRAM_BUCKETS],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.as_slice().serialize(serializer)
}
impl Histogram {
    const fn new() -> Self {
        Self {
            bucket_width_ns: HISTOGRAM_BUCKET_WIDTH_NS,
            buckets: [0; HISTOGRAM_BUCKETS],
            overflow: 0,
            max_ns: 0,
            samples: 0,
        }
    }
    fn record(&mut self, value: u64) {
        let index = value / self.bucket_width_ns;
        if index < HISTOGRAM_BUCKETS as u64 {
            self.buckets[index as usize] = self.buckets[index as usize].saturating_add(1);
        } else {
            self.overflow = self.overflow.saturating_add(1);
        }
        self.samples = self.samples.saturating_add(1);
        self.max_ns = self.max_ns.max(value);
    }
}

#[derive(Default, Serialize)]
pub struct ErrorMeasurements {
    pub deadline_miss: u64,
    pub wkc_mismatch: u64,
    pub frame_timeout: u64,
    pub rx_overflow: u64,
    pub tx_starvation: u64,
    pub unmatched: u64,
    pub corrupt: u64,
    pub consumer_rejections: u64,
    pub link_down: u64,
    pub mailbox_failures: u64,
    pub dc_failures: u64,
    pub al_failures: u64,
    pub i2c_nack: u64,
    pub i2c_timeout: u64,
    pub capture_lost: u64,
    pub rx_budget_exhausted: u64,
}

#[derive(Default, Serialize)]
pub struct CopyMeasurements {
    pub tx_bytes_total: u64,
    pub rx_bytes_total: u64,
    pub copy_spans_total: u64,
}
#[derive(Default, Serialize)]
pub struct WorkloadMeasurements {
    pub concurrent_sdo_requests: u64,
    pub sdo_submitted: u64,
    pub sdo_completed: u64,
    pub sdo_failed: u64,
    pub sdo_slots: u64,
    pub sdo_object_bytes: u64,
}
#[derive(Serialize)]
pub struct RunSampleCounts {
    pub elapsed_monotonic_ns: u64,
    pub cycle_samples: u64,
    pub max_accumulator_samples: u64,
    pub rx_round_trip_samples: u64,
    pub heap_allocations_after_activation: u64,
}
#[derive(Serialize)]
pub struct LatencyHistograms {
    pub release_jitter_abs: Histogram,
    pub fast_path: Histogram,
    pub rx_round_trip: Histogram,
}
#[derive(Serialize)]
pub struct EvidenceDigests {
    pub wire_sha256: String,
    pub wire_records: u64,
    pub state_sha256: String,
    pub state_records: u64,
    pub event_sha256: String,
    pub event_records: u64,
}
#[derive(Serialize)]
pub struct SoftwareScenarioResult {
    pub completed: bool,
    pub failures: Vec<String>,
}
#[derive(Serialize)]
pub struct RunMeasurements {
    pub schema_version: &'static str,
    pub binding: MeasurementBinding,
    pub topology: TopologyMeasurements,
    pub run: RunSampleCounts,
    pub latency_ns: LatencyHistograms,
    pub errors: ErrorMeasurements,
    pub copies: CopyMeasurements,
    pub resources: ResourceMeasurements,
    pub workload: WorkloadMeasurements,
    pub digests: EvidenceDigests,
    pub software_scenario: SoftwareScenarioResult,
}

pub struct RunEvidenceCollector {
    binding: MeasurementBinding,
    topology: TopologyMeasurements,
    resources: ResourceMeasurements,
    latency: LatencyHistograms,
    errors: ErrorMeasurements,
    copies: CopyMeasurements,
    workload: WorkloadMeasurements,
    wire: Sha256,
    state: Sha256,
    event: Sha256,
    wire_records: u64,
    wire_summary: Option<([u8; 32], u64)>,
    state_records: u64,
    event_records: u64,
    cycle_samples: u64,
}
impl RunEvidenceCollector {
    pub fn new(binding: MeasurementBinding, resources: ResourceMeasurements) -> Self {
        Self {
            binding,
            resources,
            topology: TopologyMeasurements::default(),
            latency: LatencyHistograms {
                release_jitter_abs: Histogram::new(),
                fast_path: Histogram::new(),
                rx_round_trip: Histogram::new(),
            },
            errors: ErrorMeasurements::default(),
            copies: CopyMeasurements::default(),
            workload: WorkloadMeasurements::default(),
            wire: Sha256::new(),
            state: Sha256::new(),
            event: Sha256::new(),
            wire_records: 0,
            wire_summary: None,
            state_records: 0,
            event_records: 0,
            cycle_samples: 0,
        }
    }
    pub fn with_topology(mut self, topology: TopologyMeasurements) -> Self {
        self.topology = topology;
        self
    }
    pub fn with_sdo_capacity(mut self, slots: u64, object_bytes: u64) -> Self {
        self.workload.sdo_slots = slots;
        self.workload.sdo_object_bytes = object_bytes;
        self
    }
    pub fn record_cycle(
        &mut self,
        report: &CycleReport,
        timing: CycleTiming,
        tx_bytes: u64,
        rx_bytes: u64,
        copy_spans: u64,
    ) {
        self.cycle_samples = self.cycle_samples.saturating_add(1);
        self.latency
            .release_jitter_abs
            .record(timing.release_jitter_abs_ns);
        self.latency.fast_path.record(timing.fast_path_ns);
        if let Some(round_trip_ns) = timing.rx_round_trip_ns {
            self.latency.rx_round_trip.record(round_trip_ns);
        }
        self.errors.wkc_mismatch = self
            .errors
            .wkc_mismatch
            .saturating_add(report.wkc_mismatches as u64);
        self.errors.frame_timeout = self
            .errors
            .frame_timeout
            .saturating_add(report.timed_out_datagrams as u64);
        self.errors.unmatched = self
            .errors
            .unmatched
            .saturating_add(report.unmatched_datagrams as u64);
        self.errors.corrupt = self
            .errors
            .corrupt
            .saturating_add(report.corrupt_frames as u64);
        self.errors.consumer_rejections = self
            .errors
            .consumer_rejections
            .saturating_add(report.consumer_rejections as u64);
        self.errors.link_down = self
            .errors
            .link_down
            .saturating_add(u64::from(report.link_down));
        self.errors.rx_budget_exhausted = self
            .errors
            .rx_budget_exhausted
            .saturating_add(u64::from(report.budget_exhausted));
        self.copies.tx_bytes_total = self.copies.tx_bytes_total.saturating_add(tx_bytes);
        self.copies.rx_bytes_total = self.copies.rx_bytes_total.saturating_add(rx_bytes);
        self.copies.copy_spans_total = self.copies.copy_spans_total.saturating_add(copy_spans);
    }
    pub fn record_deadline_miss(&mut self) {
        self.errors.deadline_miss = self.errors.deadline_miss.saturating_add(1);
    }
    pub fn record_tx_starvation(&mut self) {
        self.errors.tx_starvation = self.errors.tx_starvation.saturating_add(1);
    }
    pub fn record_capture_loss(&mut self, lost: u64) {
        self.errors.capture_lost = self.errors.capture_lost.saturating_add(lost);
    }
    pub fn record_auxiliary_errors(
        &mut self,
        i2c_nack: u64,
        i2c_timeout: u64,
        mailbox: u64,
        dc: u64,
        al: u64,
    ) {
        self.errors.i2c_nack = self.errors.i2c_nack.saturating_add(i2c_nack);
        self.errors.i2c_timeout = self.errors.i2c_timeout.saturating_add(i2c_timeout);
        self.errors.mailbox_failures = self.errors.mailbox_failures.saturating_add(mailbox);
        self.errors.dc_failures = self.errors.dc_failures.saturating_add(dc);
        self.errors.al_failures = self.errors.al_failures.saturating_add(al);
    }
    pub fn record_sdo(&mut self, submitted: u64, completed: u64, failed: u64, concurrent: u64) {
        self.workload.sdo_submitted = self.workload.sdo_submitted.saturating_add(submitted);
        self.workload.sdo_completed = self.workload.sdo_completed.saturating_add(completed);
        self.workload.sdo_failed = self.workload.sdo_failed.saturating_add(failed);
        self.workload.concurrent_sdo_requests =
            self.workload.concurrent_sdo_requests.max(concurrent);
    }
    #[cfg(test)]
    pub fn record_wire(&mut self, is_tx: bool, time_ns: u64, bytes: &[u8]) {
        self.wire.update([u8::from(is_tx)]);
        self.wire.update(time_ns.to_le_bytes());
        digest_record(&mut self.wire, bytes);
        self.wire_records = self.wire_records.saturating_add(1);
    }
    /// Adopt the bounded recorder's full streaming digest at hosted teardown.
    /// Its PCAP may retain only a prefix; full counters/maxima remain intact.
    pub fn adopt_wire_summary(&mut self, digest: [u8; 32], total_records: u64) {
        self.wire_summary = Some((digest, total_records));
    }
    pub fn record_state(&mut self, bytes: &[u8]) {
        digest_record(&mut self.state, bytes);
        self.state_records = self.state_records.saturating_add(1);
    }
    pub fn record_event(&mut self, bytes: &[u8]) {
        digest_record(&mut self.event, bytes);
        self.event_records = self.event_records.saturating_add(1);
    }
    /// Called only after deactivation. Digest string formatting and the supplied
    /// scenario failure strings intentionally belong to hosted teardown.
    pub fn finish(
        self,
        elapsed_monotonic_ns: u64,
        allocations: u64,
        software_completed: bool,
        failures: Vec<String>,
    ) -> RunMeasurements {
        let rx_round_trip_samples = self.latency.rx_round_trip.samples;
        RunMeasurements {
            schema_version: "esop.run-measurements.v1",
            binding: self.binding,
            topology: self.topology,
            resources: self.resources,
            latency_ns: self.latency,
            errors: self.errors,
            copies: self.copies,
            workload: self.workload,
            run: RunSampleCounts {
                elapsed_monotonic_ns,
                cycle_samples: self.cycle_samples,
                max_accumulator_samples: self.cycle_samples,
                rx_round_trip_samples,
                heap_allocations_after_activation: allocations,
            },
            digests: EvidenceDigests {
                wire_sha256: self
                    .wire_summary
                    .map_or_else(|| hex(&self.wire.finalize()), |(digest, _)| hex(&digest)),
                wire_records: self
                    .wire_summary
                    .map_or(self.wire_records, |(_, records)| records),
                state_sha256: hex(&self.state.finalize()),
                state_records: self.state_records,
                event_sha256: hex(&self.event.finalize()),
                event_records: self.event_records,
            },
            software_scenario: SoftwareScenarioResult {
                completed: software_completed,
                failures,
            },
        }
    }
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 15) as usize] as char);
    }
    encoded
}
fn digest_record(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_stream_summary_replaces_a_bounded_prefix_without_losing_counts() {
        let binding = MeasurementBinding {
            source: MeasurementSource::Sim,
            esop_commit: "0".repeat(40),
            config_sha256: "1".repeat(64),
            topology_sha256: "2".repeat(64),
            scenario: "test".into(),
            period_ns: 1000,
            cycles: 1,
            clock_origin_ns: 0,
        };
        let mut collector = RunEvidenceCollector::new(binding, ResourceMeasurements::default());
        collector.record_wire(true, 0, &[0x88, 0xa4]);
        collector.adopt_wire_summary([0x42; 32], 100);
        let report = collector.finish(1000, 0, true, Vec::new());
        assert_eq!(report.digests.wire_records, 100);
        assert_eq!(report.digests.wire_sha256, "42".repeat(32));
    }
    #[test]
    fn histograms_cover_every_sample_and_preserve_outliers() {
        let mut histogram = Histogram::new();
        for value in [0, 999, 1000, 511_999, 512_000, 1_000_000] {
            histogram.record(value);
        }
        assert_eq!(histogram.samples, 6);
        assert_eq!(
            histogram.buckets.iter().sum::<u64>() + histogram.overflow,
            6
        );
        assert_eq!(histogram.overflow, 2);
        assert_eq!(histogram.max_ns, 1_000_000);
    }

    #[test]
    fn round_trip_coverage_excludes_missing_samples_and_preserves_observed_zero() {
        fn collect(samples: &[Option<u64>]) -> RunMeasurements {
            let mut collector = RunEvidenceCollector::new(
                MeasurementBinding {
                    source: MeasurementSource::Sim,
                    esop_commit: "0".repeat(40),
                    config_sha256: "1".repeat(64),
                    topology_sha256: "2".repeat(64),
                    scenario: "rtt-coverage".into(),
                    period_ns: 1000,
                    cycles: samples.len() as u64,
                    clock_origin_ns: 0,
                },
                ResourceMeasurements::default(),
            );
            for (index, sample) in samples.iter().copied().enumerate() {
                collector.record_cycle(
                    &CycleReport {
                        cycle: index as u64 + 1,
                        received_frames: 0,
                        received_bytes: 0,
                        parsed_datagrams: 0,
                        unmatched_datagrams: 0,
                        corrupt_frames: 0,
                        wkc_mismatches: 0,
                        timed_out_datagrams: 0,
                        consumer_rejections: 0,
                        budget_exhausted: false,
                        link_down: false,
                    },
                    CycleTiming {
                        release_jitter_abs_ns: 0,
                        fast_path_ns: 100,
                        rx_round_trip_ns: sample,
                    },
                    0,
                    0,
                    0,
                );
            }
            collector.finish(samples.len() as u64 * 1000, 0, true, Vec::new())
        }
        let partial = collect(&[None, Some(0), Some(2345), None]);
        assert_eq!(partial.run.cycle_samples, 4);
        assert_eq!(partial.run.rx_round_trip_samples, 2);
        assert_eq!(partial.latency_ns.fast_path.samples, 4);
        assert_eq!(partial.latency_ns.release_jitter_abs.samples, 4);
        assert_eq!(partial.latency_ns.rx_round_trip.samples, 2);
        assert_eq!(partial.latency_ns.rx_round_trip.buckets[0], 1);
        assert_eq!(partial.latency_ns.rx_round_trip.buckets[2], 1);
        assert_eq!(partial.latency_ns.rx_round_trip.max_ns, 2345);
        let json = serde_json::to_value(partial).unwrap();
        assert_eq!(json["run"]["rx_round_trip_samples"], 2);

        let absent = collect(&[None, None]);
        assert_eq!(absent.run.cycle_samples, 2);
        assert_eq!(absent.run.rx_round_trip_samples, 0);
        assert_eq!(absent.latency_ns.rx_round_trip.samples, 0);
        assert_eq!(
            absent.latency_ns.rx_round_trip.buckets.iter().sum::<u64>(),
            0
        );
        assert_eq!(absent.latency_ns.rx_round_trip.overflow, 0);
        assert_eq!(absent.latency_ns.rx_round_trip.max_ns, 0);
    }
}
