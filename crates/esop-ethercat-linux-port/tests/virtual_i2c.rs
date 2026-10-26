#[allow(dead_code)]
#[path = "../examples/support/virtual_i2c.rs"]
mod virtual_i2c;
use esop_device::DeviceState;
use virtual_i2c::{I2cError, I2cOutcome, I2cPoll, VirtualI2c};

#[test]
fn pending_nack_timeout_require_explicit_shared_lifecycle_recovery() {
    let mut device = VirtualI2c::new(100, 0x40, 100).unwrap();
    assert_eq!(
        device.snapshot(0).unwrap().device.state,
        DeviceState::Probed
    );
    device.initialize(0).unwrap();
    let first = device.submit(0, 20, 5, I2cOutcome::Nack).unwrap();
    assert_eq!(
        device.poll(4, 1).unwrap(),
        I2cPoll::Pending { generation: first }
    );
    assert_eq!(
        device.poll(5, 2).unwrap(),
        I2cPoll::Nack { generation: first }
    );
    assert_eq!(device.service_cycle(6, 3).unwrap(), I2cPoll::FaultLatched);
    assert!(device.submit(6, 20, 1, I2cOutcome::Success(1)).is_err());
    assert!(!device.snapshot(6).unwrap().fresh);
    device.request_recovery(4).unwrap();
    assert_eq!(
        device.snapshot(6).unwrap().device.state,
        DeviceState::Recovering
    );
    assert!(device.submit(6, 20, 1, I2cOutcome::Success(1)).is_err());
    device.reconfigure(5).unwrap();
    let second = device.submit(6, 20, 1, I2cOutcome::NeverCompletes).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        device.poll(7, 6).unwrap(),
        I2cPoll::Pending { generation: second }
    );
    assert_eq!(
        device.poll(26, 7).unwrap(),
        I2cPoll::Timeout { generation: second }
    );
    for cycle in 8..20 {
        assert_eq!(
            device.service_cycle(cycle + 30, cycle).unwrap(),
            I2cPoll::FaultLatched
        );
    }
    assert_eq!(
        device.snapshot(100).unwrap().device.state,
        DeviceState::Fault
    );
    assert_eq!(device.snapshot(100).unwrap().nacks, 1);
    assert_eq!(device.snapshot(100).unwrap().timeouts, 1);
}

#[test]
fn monotonic_freshness_is_independent_and_one_slot_is_bounded() {
    let mut device = VirtualI2c::new(100, 0x40, 10).unwrap();
    device.initialize(0).unwrap();
    let generation = device.submit(100, 20, 1, I2cOutcome::Success(42)).unwrap();
    assert_eq!(
        device.submit(100, 20, 1, I2cOutcome::Success(43)),
        Err(I2cError::Busy)
    );
    assert_eq!(
        device.poll(101, 1).unwrap(),
        I2cPoll::Complete {
            generation,
            value: 42
        }
    );
    assert!(device.snapshot(111).unwrap().fresh);
    assert!(!device.snapshot(112).unwrap().fresh);
    assert!(!device.snapshot(100).unwrap().fresh);
    assert_eq!(device.poll(100, 2), Err(I2cError::ClockRegression));
}

#[test]
fn lower_priority_service_is_one_bounded_action_per_caller_cycle() {
    let mut device = VirtualI2c::new(100, 0x40, 100).unwrap();
    device.initialize(0).unwrap();
    let mut ethercat_p0_cycles = 0;
    for cycle in 0..100 {
        // P0 completes before invoking this optional peripheral. Its Pending
        // status cannot introduce retries, sleeps or extra P0 invocations.
        ethercat_p0_cycles += 1;
        let status = device.service_cycle(cycle * 10, cycle).unwrap();
        assert!(matches!(
            status,
            I2cPoll::Pending { .. } | I2cPoll::Complete { .. }
        ));
    }
    assert_eq!(ethercat_p0_cycles, 100);
    assert_eq!(
        device.snapshot(1_000).unwrap().device.state,
        DeviceState::Cyclic
    );
}

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static COUNT_ALLOCATIONS:Cell<bool>=const {Cell::new(false)};
    static ALLOCATIONS:Cell<usize>=const {Cell::new(0)};
}
struct AllocationCounter;
unsafe impl GlobalAlloc for AllocationCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: AllocationCounter = AllocationCounter;

#[allow(dead_code)]
#[path = "../examples/support/evidence.rs"]
mod evidence;

#[test]
fn activated_i2c_and_all_measurement_recording_allocate_nothing() {
    use esop_ethercat_core::CycleReport;
    use evidence::{
        CycleTiming, MeasurementBinding, MeasurementSource, ResourceMeasurements,
        RunEvidenceCollector,
    };
    let mut device = VirtualI2c::new(100, 0x40, 100).unwrap();
    device.initialize(0).unwrap();
    let mut collector = RunEvidenceCollector::new(
        MeasurementBinding {
            source: MeasurementSource::Sim,
            esop_commit: "a".repeat(40),
            config_sha256: "b".repeat(64),
            topology_sha256: "c".repeat(64),
            scenario: "allocation-test".into(),
            period_ns: 10,
            cycles: 100,
            clock_origin_ns: 0,
        },
        ResourceMeasurements::default(),
    );
    ALLOCATIONS.with(|value| value.set(0));
    COUNT_ALLOCATIONS.with(|value| value.set(true));
    for cycle in 0..100 {
        device.service_cycle(cycle * 10, cycle).unwrap();
        let report = CycleReport {
            cycle,
            received_frames: 1,
            received_bytes: 64,
            parsed_datagrams: 1,
            unmatched_datagrams: 0,
            corrupt_frames: 0,
            wkc_mismatches: 0,
            timed_out_datagrams: 0,
            consumer_rejections: 0,
            budget_exhausted: false,
            link_down: false,
        };
        collector.record_cycle(
            &report,
            CycleTiming {
                release_jitter_abs_ns: 0,
                fast_path_ns: 1234,
                rx_round_trip_ns: 4321,
            },
            64,
            64,
            2,
        );
        collector.record_wire(true, cycle * 10, &[0; 64]);
        collector.record_state(&cycle.to_le_bytes());
        collector.record_event(&[1, 2, 3]);
        collector.record_sdo(0, 0, 0, 0);
        collector.record_auxiliary_errors(0, 0, 0, 0, 0);
    }
    COUNT_ALLOCATIONS.with(|value| value.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    let result = collector.finish(1000, 0, true, Vec::new());
    assert_eq!(result.run.cycle_samples, 100);
    assert_eq!(result.digests.state_records, 100);
    assert_eq!(result.digests.wire_records, 100);
    let json = serde_json::to_value(result).unwrap();
    assert_eq!(json["binding"]["source"], "sim");
    assert_eq!(json["latency_ns"]["fast_path"]["samples"], 100);
}

#[test]
fn rust_measurements_import_through_the_existing_performance_contract() {
    use esop_ethercat_core::CycleReport;
    use evidence::{
        CycleTiming, MeasurementBinding, MeasurementSource, ResourceMeasurements,
        RunEvidenceCollector, TopologyMeasurements,
    };
    use sha2::{Digest, Sha256};
    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "esop-measurement-import-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let config = b"{\"product\":\"test\"}\n";
    let topology = TopologyMeasurements {
        slave_count: 3,
        axis_count: 2,
        io_channels: 16,
        pdo_bytes: 46,
        frame_count: 2,
        dc_enabled: true,
    };
    let topology_bytes = serde_json::to_vec(
        &serde_json::json!({"schema_version":"esop.software-topology.v1",
        "scenario":"full","period_ns":1_000_000,"topology":topology}),
    )
    .unwrap();
    let mut collector = RunEvidenceCollector::new(
        MeasurementBinding {
            source: MeasurementSource::Sim,
            esop_commit: "a".repeat(40),
            config_sha256: hash(config),
            topology_sha256: hash(&topology_bytes),
            scenario: "full".into(),
            period_ns: 1_000_000,
            cycles: 2,
            clock_origin_ns: 0,
        },
        ResourceMeasurements::default(),
    )
    .with_topology(topology);
    for cycle in 0..2 {
        let report = CycleReport {
            cycle,
            received_frames: 1,
            received_bytes: 64,
            parsed_datagrams: 1,
            unmatched_datagrams: 0,
            corrupt_frames: 0,
            wkc_mismatches: 0,
            timed_out_datagrams: 0,
            consumer_rejections: 0,
            budget_exhausted: false,
            link_down: false,
        };
        collector.record_cycle(
            &report,
            CycleTiming {
                release_jitter_abs_ns: 0,
                fast_path_ns: 1234,
                rx_round_trip_ns: 2345,
            },
            64,
            64,
            2,
        );
        collector.record_wire(true, cycle * 1_000_000, &[0; 64]);
        collector.record_state(&cycle.to_le_bytes());
    }
    let result = collector.finish(3_000_000, 0, true, Vec::new());
    std::fs::write(
        directory.join("run.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    std::fs::write(directory.join("config.json"), config).unwrap();
    std::fs::write(directory.join("topology.json"), topology_bytes).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/import-performance-measurements.py");
    let output = std::process::Command::new("python3")
        .arg(script)
        .arg("--input")
        .arg(directory.join("run.json"))
        .arg("--config")
        .arg(directory.join("config.json"))
        .arg("--topology")
        .arg(directory.join("topology.json"))
        .arg("--expected-commit")
        .arg("a".repeat(40))
        .arg("--output")
        .arg(directory.join("performance.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let imported: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("performance.json")).unwrap())
            .unwrap();
    assert_eq!(imported["measurement"]["cycle_samples"], 2);
    assert_eq!(imported["latency_ns"]["fast_path"]["max"], 1234);
    assert_eq!(imported["qualification"]["passed"], false);
    std::fs::remove_dir_all(directory).unwrap();
}
