# Software Run Evidence and Optional I2C

## 1. Scope / Trigger

Use these contracts for hosted R0-R2 software scenarios, measurement export,
offline regression reports, and optional non-EtherCAT peripheral work. Software
scenario success cannot qualify physical interoperability, target WCET or DC
accuracy. The activated cycle must allocate nothing to collect evidence.

## 2. Signatures

- `RunEvidenceCollector::new(MeasurementBinding, ResourceMeasurements)` freezes
  hosted metadata before activation. Optional builders freeze topology and SDO
  capacities. `record_cycle`, `record_state`, `record_event`,
  `record_sdo`, and auxiliary/error methods use fixed storage and streaming SHA.
- `adopt_wire_summary(digest, total_records)` imports the capture layer's full
  streaming summary at teardown. Production wire observations belong to the
  recording port; collector `record_wire` exists only for focused tests.
- `finish(elapsed_monotonic_ns, allocations, software_completed, failures)` is
  teardown work and returns serializable `esop.run-measurements.v1`.
- `python3 scripts/import-performance-measurements.py --input run.json
  --config product_config.json --topology topology.json --expected-commit SHA
  --output performance.json [--baseline unloaded-performance.json]`.
- `python3 scripts/validate-r2-qualification.py --require-qualified` is the
  explicit release gate. `make r2-release-gate` binds it to the current commit.
- `VirtualI2c::new(id, address, max_age_ns)`, `initialize(cycle)`,
  `submit(now, timeout, complete_after, outcome)`, `poll(now, cycle)`,
  `request_recovery(cycle)`, `reconfigure(cycle)`, and `snapshot(now)` expose
  one fixed transaction through the existing `DeviceManager<1>` lifecycle.

## 3. Contracts

### Evidence

- Bind source (`sim`, `pcap_replay`, `linux_raw`), Git commit, exact configuration
  artifact SHA-256, exact topology artifact SHA-256, scenario, period, cycle
  count and clock origin. Configuration artifact byte hashes are distinct from
  cfggen's semantic configuration identity; do not silently substitute one.
- Topology artifact schema is `esop.software-topology.v1`, with exactly
  `schema_version`, `scenario`, `period_ns`, and `topology`. The topology object
  contains slave/axis/IO/PDO/frame counts and DC policy. Import verifies both
  artifact hash and equality of period/scenario/counts with the measurements.
- Record each clock domain explicitly: release jitter uses port monotonic
  time, fast-path duration uses host monotonic time, and RX round-trip is the
  port's observed poll time after the matching TX. Virtual bus time is not host
  execution time; port-observed round-trip need not fit host elapsed time.
  Histograms use 512 fixed 1000 ns buckets plus overflow, sample count and exact
  observed maximum. Imported percentiles are documented bucket upper bounds
  clamped to that maximum; no latency is synthesized from the configured period.
- Every cycle contributes release/fast-path samples and one state record. An
  RX round-trip histogram records one per-cycle maximum only when unambiguous
  pairs were observed. Export its
  actual `run.rx_round_trip_samples`, which can be zero; unavailable observations
  are not zero-latency samples. Legacy v1 files without this additive count remain
  importable, with their coverage explicitly unknown rather than assumed complete.
  Retain all maxima, wire/state/event digests, complete error
  counters, copy totals, allocation count, resources and SDO capacity/workload.
  RX budget exhaustion and hardware RX overflow are separate counters.
- Reusing a wire index with an outstanding TX makes its RTT ambiguous. Keep
  that index unavailable until a new observer session; cycle reset only clears
  the current maximum and never invents a matching generation. Deterministic
  replay must preserve this conservative timing decision as well as frame order.
- A bounded capture may retain only a prefix. Its loss count is explicit,
  while streaming wire counts/digest and cycle/error/max accumulators still
  cover the full run. Import preserves that loss reason; replay requires a
  complete capture and must not silently treat a prefix as the complete run.
- Unknown physical resource/temperature/compiler facts remain explicitly
  unrecorded. Zero thermal placeholders in the legacy report shape do not
  become measurements or qualification evidence. A sum of hosted object sizes
  is not a measured core arena, target stack peak, DMA or CPU resource result.
- Import is closed-schema, rejects duplicate JSON keys, nonfinite values,
  unknown qualification fields, HIL sources, metadata mismatches and inconsistent
  histogram/counts. It always emits `qualification.passed = false` in the
  existing `esop.performance.v1` format and calls its existing validator.
- A measured unloaded baseline must match commit, config, topology/counts,
  period, source and histogram precision, with full positive sample coverage.
  Require its successful software completion, zero activated allocations and
  zero submitted/completed/failed SDO work; a failed or partly loaded run is
  not a valid reference even when its latency histogram is complete.
  Compute P99 regression from its actual P99; retain `null` when no baseline
  exists. Completed Q4 software evidence needs at least eight concurrent,
  completed SDO requests with at least 1 KiB object capacity.

### I2C

- Only the caller polls this optional service, after EtherCAT Domain/DC and
  lifecycle work. Pending produces no retry loop, sleep, allocation or extra
  EtherCAT action. Freshness uses the caller's monotonic time, independently
  from DC qualification.
- NACK or request deadline expiry clears pending/value/fresh evidence and
  latches `DeviceState::Fault`. Removing the injected fault does not recover.
  `request_recovery` plus explicit reconfiguration must traverse the common
  lifecycle before another transaction is accepted.
- At most one transaction exists; a second submission returns `Busy`.
  Nonmonotonic time and deadline overflow are typed errors.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Missing/extra source field, forged passed/HIL, duplicate JSON | Import exits nonzero |
| Config/topology hash or explicit commit mismatch | Import exits nonzero |
| Histogram sum/sample/max mismatch, missing cycle state | Import exits nonzero |
| Missing/ambiguous RX timing | No timing sample; explicit unavailable coverage |
| Failed/allocating/partly SDO-loaded baseline | Reject regression reference |
| Baseline absent | No invented regression; Q4 reason remains unqualified |
| Valid software run | Existing report format, qualification always false |
| Valid unqualified R2 baseline in development mode | Validator exits zero |
| Same baseline with `--require-qualified` | Release gate exits nonzero |
| Qualified R2 missing Q1/Q2/Q4 or Q4 unloaded artifact | Reject qualified claim |
| I2C pending / busy / NACK / timeout | Fixed typed result, fault requires recovery |

## 5. Good / Base / Bad Cases

- Good: actual monotonic software samples import with zero allocations and
  verified artifact identities; software completion and hardware qualification
  remain separate fields.
- Base: an unloaded baseline is unavailable; report retains null regression.
- Bad: changing topology counts while retaining the old artifact hash is
  rejected by body comparison, even if the digest string remains well formed.
- Bad: Q4 names a fabricated baseline hash; qualified R2 requires the actual
  `baseline_artifact` file and verifies its SHA, P99, HIL sample coverage and
  matching metadata.

## 6. Tests Required

- `scripts/tests/test_import_performance_measurements.py`: all software sources,
  missing/extra fields, forged HIL/pass, hashes, counts, finite values and
  computed baseline regression.
- `scripts/tests/test_r2_qualification.py`: valid development baseline, explicit
  release rejection, complete Q1/Q2/Q4 path and missing/forged unloaded evidence.
- `crates/esop-ethercat-linux-port/tests/virtual_i2c.rs`: Pending/NACK/timeout,
  explicit recovery, monotonic age, bounded slot, allocation counter covering all activated record
  methods, and Rust JSON -> offline importer -> legacy performance validator.

## 7. Wrong vs Correct

Wrong: infer timing from `cycles * virtual_period`, fill unknown resources with
plausible numbers, or turn a successful simulator scenario into a passed HIL
performance report.

Correct: record actual host monotonic observations, preserve unknown evidence,
and keep software imports permanently unqualified until a separate physical
qualification path supplies verified Q1/Q2/Q4 artifacts.
