#![cfg(target_os = "linux")]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    CAPABILITY_BTF, CAPABILITY_PERMISSION, CAPABILITY_RINGBUF, CAPABILITY_VERIFIER,
    CapabilitySnapshot, RuntimeAgent,
};
use esop_ebpf_runtime::{ATTACH_PAGE_FAULT, BpfRuntime, PollReport, RuntimeConfig};

const BOOT_ID: u64 = 0x4553_4f50_4445_4752;
const FIRST_AGENT_EPOCH: u64 = 1;
const SECOND_AGENT_EPOCH: u64 = 2;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const PAGE_FAULT_THRESHOLD: u64 = 1;
const PAGE_FAULT_WINDOW_NS: u64 = 1;
const RINGBUF_BYTES: usize = 1 << 22;
const CHUNK_BYTES: usize = 64 << 20;
const MAX_BATCHES: usize = 8;
const POLL_SLEEP: Duration = Duration::from_millis(2);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const OBSERVATION_HEALTHY: u8 = 0;
const OBSERVATION_DEGRADED: u8 = 1;
const OBSERVATION_FAILED: u8 = 2;
const EVENT_LOSS_FAULT: u32 = 0x4542_1004;
const CAPABILITY_FAULT: u32 = 0x4542_1001;
const LOAD_FAULT: u32 = 0x4542_1002;
const ATTACH_FAULT: u32 = 0x4542_1003;

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn report_temp_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_os_string();
    path.push(".tmp");
    PathBuf::from(path)
}

fn system_page_size() -> io::Result<usize> {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size <= 0 {
        return Err(io::Error::last_os_error());
    }
    usize::try_from(size).map_err(|_| invalid_data("system page size did not fit usize"))
}

fn touch_anonymous_chunk(page_size: usize) -> io::Result<usize> {
    if CHUNK_BYTES % page_size != 0 {
        return Err(invalid_data(
            "qualification chunk was not aligned to the system page size",
        ));
    }
    let mapping = unsafe {
        libc::mmap(
            ptr::null_mut(),
            CHUNK_BYTES,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if mapping == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    let mapping = mapping.cast::<u8>();
    let result = (|| {
        if unsafe { libc::madvise(mapping.cast(), CHUNK_BYTES, libc::MADV_NOHUGEPAGE) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let page_count = CHUNK_BYTES / page_size;
        for page in 0..page_count {
            unsafe { ptr::write_volatile(mapping.add(page * page_size), 1) };
        }
        Ok(page_count)
    })();
    let unmap_result = unsafe { libc::munmap(mapping.cast(), CHUNK_BYTES) };
    if unmap_result != 0 {
        return Err(io::Error::last_os_error());
    }
    result
}

fn add_poll_report(total: &mut PollReport, current: PollReport) {
    total.records_seen = total.records_seen.saturating_add(current.records_seen);
    total.incidents_emitted = total
        .incidents_emitted
        .saturating_add(current.incidents_emitted);
    total.malformed_records = total
        .malformed_records
        .saturating_add(current.malformed_records);
    total.evidence_rejected = total
        .evidence_rejected
        .saturating_add(current.evidence_rejected);
    total.newly_reported_lost_events = total
        .newly_reported_lost_events
        .saturating_add(current.newly_reported_lost_events);
}

fn capability_observation(capability_mask: u64, attach_mask: u64) -> (u8, u32) {
    let mut agent = RuntimeAgent::<1>::new(BOOT_ID, 99, INCIDENT_WINDOW_NS);
    agent
        .health_mut()
        .set_capability_snapshot(CapabilitySnapshot::new(
            0xAA,
            attach_mask,
            ATTACH_PAGE_FAULT,
            capability_mask,
        ));
    let observation = agent.heartbeat(1);
    (observation.state as u8, observation.fault_code)
}

fn config(agent_epoch: u64, tracked_pid: u32) -> RuntimeConfig {
    RuntimeConfig {
        enabled_attach_mask: ATTACH_PAGE_FAULT,
        required_attach_mask: ATTACH_PAGE_FAULT,
        tracked_pid,
        page_fault_threshold: PAGE_FAULT_THRESHOLD,
        page_fault_window_ns: PAGE_FAULT_WINDOW_NS,
        boot_id: BOOT_ID,
        agent_epoch,
        ..RuntimeConfig::default()
    }
}

fn json_number(name: &'static str, value: impl ToString) -> (&'static str, String) {
    (name, value.to_string())
}

fn write_report(output: &Path, fields: Vec<(&'static str, String)>) -> io::Result<()> {
    let temp = report_temp_path(output);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut report = String::from("{\n");
    let last = fields.len().saturating_sub(1);
    for (index, (name, value)) in fields.into_iter().enumerate() {
        let comma = if index == last { "" } else { "," };
        writeln!(&mut report, "  \"{name}\": {value}{comma}")
            .map_err(|_| invalid_data("failed to format qualification report"))?;
    }
    report.push_str("}\n");
    fs::write(&temp, report)?;
    fs::rename(temp, output)?;
    Ok(())
}

fn run(object_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    if std::env::consts::ARCH != "x86_64" {
        return Err(invalid_data("observability degradation qualification requires x86_64").into());
    }
    if unsafe { libc::geteuid() } != 0 {
        return Err(invalid_data("observability degradation qualification requires root").into());
    }
    if !object_path.is_file() {
        return Err(invalid_data("CO-RE BPF object was not a file").into());
    }
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let page_size = system_page_size()?;
    if page_size < 4096 || !page_size.is_power_of_two() || CHUNK_BYTES % page_size != 0 {
        return Err(invalid_data("system page geometry was unsupported").into());
    }
    let chunk_pages = CHUNK_BYTES / page_size;
    let max_pages = chunk_pages
        .checked_mul(MAX_BATCHES)
        .ok_or_else(|| invalid_data("maximum page count overflowed"))?;
    let tracked_pid = std::process::id();
    if tracked_pid == 0 || tracked_pid > i32::MAX as u32 {
        return Err(invalid_data("qualification PID was outside the supported range").into());
    }

    let complete_capabilities =
        CAPABILITY_BTF | CAPABILITY_RINGBUF | CAPABILITY_VERIFIER | CAPABILITY_PERMISSION;
    let (missing_btf_state, missing_btf_fault) =
        capability_observation(complete_capabilities & !CAPABILITY_BTF, ATTACH_PAGE_FAULT);
    let (missing_ringbuf_state, missing_ringbuf_fault) = capability_observation(
        complete_capabilities & !CAPABILITY_RINGBUF,
        ATTACH_PAGE_FAULT,
    );
    let (missing_attach_state, missing_attach_fault) =
        capability_observation(complete_capabilities, 0);
    let (missing_verifier_state, missing_verifier_fault) = capability_observation(
        complete_capabilities & !CAPABILITY_VERIFIER,
        ATTACH_PAGE_FAULT,
    );
    let (missing_permission_state, missing_permission_fault) = capability_observation(
        complete_capabilities & !CAPABILITY_PERMISSION,
        ATTACH_PAGE_FAULT,
    );
    if (missing_btf_state, missing_btf_fault) != (OBSERVATION_DEGRADED, CAPABILITY_FAULT)
        || (missing_ringbuf_state, missing_ringbuf_fault)
            != (OBSERVATION_DEGRADED, CAPABILITY_FAULT)
        || (missing_attach_state, missing_attach_fault) != (OBSERVATION_DEGRADED, ATTACH_FAULT)
        || (missing_verifier_state, missing_verifier_fault) != (OBSERVATION_FAILED, LOAD_FAULT)
        || (missing_permission_state, missing_permission_fault) != (OBSERVATION_FAILED, LOAD_FAULT)
    {
        return Err(invalid_data("capability classification matrix was inconsistent").into());
    }

    let preflight = BpfRuntime::preflight(ATTACH_PAGE_FAULT);
    if !preflight.snapshot.attach_ready() {
        return Err(invalid_data(format!(
            "host preflight could not qualify page-fault observation: {:?}",
            preflight.snapshot
        ))
        .into());
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, FIRST_AGENT_EPOCH, INCIDENT_WINDOW_NS);
    let mut runtime = BpfRuntime::load(&object_path, config(FIRST_AGENT_EPOCH, tracked_pid))?;
    let first_snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_PAGE_FAULT
        || first_snapshot.attach_mask != ATTACH_PAGE_FAULT
        || first_snapshot.required_attach_mask != ATTACH_PAGE_FAULT
        || !first_snapshot.attach_ready()
    {
        return Err(invalid_data("first runtime page-fault capability was incomplete").into());
    }
    runtime.apply_capability_snapshot(&mut agent);
    let initial_observation = agent.heartbeat(1);
    if initial_observation.state as u8 != OBSERVATION_HEALTHY
        || initial_observation.agent_epoch != FIRST_AGENT_EPOCH
        || initial_observation.attach_mask != ATTACH_PAGE_FAULT
        || initial_observation.lost_event_count != 0
        || initial_observation.incident_count != 0
        || initial_observation.fault_code != 0
        || initial_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("initial observer health was inconsistent").into());
    }

    let baseline = runtime.statistics()?;
    if baseline.lost_events != 0 {
        return Err(invalid_data("ring buffer was already losing events before saturation").into());
    }
    let mut batches_run = 0usize;
    let mut pages_touched = 0usize;
    let mut saturation = baseline;
    for batch in 1..=MAX_BATCHES {
        pages_touched = pages_touched
            .checked_add(touch_anonymous_chunk(page_size)?)
            .ok_or_else(|| invalid_data("touched page count overflowed"))?;
        batches_run = batch;
        saturation = runtime.statistics()?;
        if saturation.lost_events != 0 {
            break;
        }
    }
    if saturation.lost_events == 0
        || saturation.page_faults
            < baseline
                .page_faults
                .saturating_add(u64::try_from(pages_touched).unwrap_or(u64::MAX))
        || saturation.emitted_events <= baseline.emitted_events
        || saturation.page_fault_threshold_events <= baseline.page_fault_threshold_events
        || batches_run == 0
        || batches_run > MAX_BATCHES
        || pages_touched != batches_run * chunk_pages
        || pages_touched > max_pages
    {
        return Err(invalid_data(format!(
            "ring-buffer saturation was inconsistent: baseline={baseline:?} final={saturation:?} batches={batches_run} pages={pages_touched}"
        ))
        .into());
    }

    let saturation_poll = runtime.poll(&mut agent, 1)?;
    let expected_lost = u32::try_from(saturation.lost_events).unwrap_or(u32::MAX);
    if saturation_poll.records_seen != 1
        || saturation_poll.incidents_emitted != 0
        || saturation_poll.malformed_records != 0
        || saturation_poll.evidence_rejected != 0
        || saturation_poll.newly_reported_lost_events != expected_lost
    {
        return Err(invalid_data(format!(
            "saturation poll was inconsistent: {saturation_poll:?}"
        ))
        .into());
    }
    let loss_observation = agent.heartbeat(2);
    if loss_observation.state as u8 != OBSERVATION_DEGRADED
        || loss_observation.attach_mask != ATTACH_PAGE_FAULT
        || loss_observation.lost_event_count != expected_lost
        || loss_observation.incident_count != 0
        || loss_observation.fault_code != EVENT_LOSS_FAULT
        || loss_observation.heartbeat_seq != 2
    {
        return Err(invalid_data("event loss did not project into observer health").into());
    }

    runtime.apply_capability_snapshot(&mut agent);
    let reapplied_observation = agent.heartbeat(3);
    if reapplied_observation.state as u8 != OBSERVATION_DEGRADED
        || reapplied_observation.lost_event_count != expected_lost
        || reapplied_observation.fault_code != EVENT_LOSS_FAULT
        || reapplied_observation.heartbeat_seq != 3
    {
        return Err(invalid_data("same-epoch capability refresh cleared event loss").into());
    }

    let first_runtime_attach_mask = runtime.attach_mask();
    drop(runtime);
    let first_runtime_detached = 1u8;

    agent.restart(SECOND_AGENT_EPOCH);
    let restarting_observation = agent.heartbeat(4);
    if restarting_observation.agent_epoch != SECOND_AGENT_EPOCH
        || restarting_observation.state as u8 != OBSERVATION_DEGRADED
        || restarting_observation.attach_mask != 0
        || restarting_observation.lost_event_count != 0
        || restarting_observation.incident_count != 0
        || restarting_observation.fault_code != 0
        || restarting_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("restart retained stale epoch health state").into());
    }

    let mut runtime = BpfRuntime::load(&object_path, config(SECOND_AGENT_EPOCH, tracked_pid))?;
    let second_snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_PAGE_FAULT
        || second_snapshot.attach_mask != ATTACH_PAGE_FAULT
        || second_snapshot.required_attach_mask != ATTACH_PAGE_FAULT
        || !second_snapshot.attach_ready()
        || runtime.kernel_context().agent_epoch != SECOND_AGENT_EPOCH
    {
        return Err(invalid_data("second runtime page-fault capability was incomplete").into());
    }
    runtime.apply_capability_snapshot(&mut agent);
    let recovered_observation = agent.heartbeat(5);
    if recovered_observation.agent_epoch != SECOND_AGENT_EPOCH
        || recovered_observation.state as u8 != OBSERVATION_HEALTHY
        || recovered_observation.attach_mask != ATTACH_PAGE_FAULT
        || recovered_observation.lost_event_count != 0
        || recovered_observation.incident_count != 0
        || recovered_observation.fault_code != 0
        || recovered_observation.heartbeat_seq != 2
    {
        return Err(
            invalid_data("new runtime snapshot did not restore healthy observation").into(),
        );
    }

    let recovery_baseline = runtime.statistics()?;
    if recovery_baseline.lost_events != 0 {
        return Err(invalid_data("new runtime inherited old kernel loss statistics").into());
    }
    let recovery_pages_touched = touch_anonymous_chunk(page_size)?;
    let mut recovery_poll = PollReport::default();
    let recovery_started = Instant::now();
    while recovery_started.elapsed() < POLL_DEADLINE {
        add_poll_report(&mut recovery_poll, runtime.poll(&mut agent, 64)?);
        if recovery_poll.records_seen != 0 {
            break;
        }
        thread::sleep(POLL_SLEEP);
    }
    let recovery = runtime.statistics()?;
    if recovery_poll.records_seen == 0
        || recovery_poll.incidents_emitted != 0
        || recovery_poll.malformed_records != 0
        || recovery_poll.evidence_rejected != 0
        || recovery_poll.newly_reported_lost_events != 0
        || recovery.lost_events != 0
        || recovery.page_faults <= recovery_baseline.page_faults
        || recovery.emitted_events <= recovery_baseline.emitted_events
        || recovery_pages_touched != chunk_pages
    {
        return Err(invalid_data(format!(
            "new-epoch evidence was inconsistent: poll={recovery_poll:?} baseline={recovery_baseline:?} final={recovery:?}"
        ))
        .into());
    }
    let final_observation = agent.heartbeat(6);
    if final_observation.agent_epoch != SECOND_AGENT_EPOCH
        || final_observation.state as u8 != OBSERVATION_HEALTHY
        || final_observation.attach_mask != ATTACH_PAGE_FAULT
        || final_observation.lost_event_count != 0
        || final_observation.incident_count != 0
        || final_observation.fault_code != 0
        || final_observation.heartbeat_seq != 3
    {
        return Err(invalid_data("new-epoch evidence changed healthy observation").into());
    }

    let second_runtime_attach_mask = runtime.attach_mask();
    drop(runtime);
    let second_runtime_detached = 1u8;
    let cleanup_succeeded = 1u8;

    let fields = vec![
        json_number("schema_version", 1),
        ("status", "\"qualified\"".to_owned()),
        ("architecture", "\"x86_64\"".to_owned()),
        json_number("boot_id", BOOT_ID),
        json_number("tracked_pid", tracked_pid),
        json_number("page_size", page_size),
        json_number("ringbuf_bytes", RINGBUF_BYTES),
        json_number("chunk_bytes", CHUNK_BYTES),
        json_number("chunk_pages", chunk_pages),
        json_number("max_batches", MAX_BATCHES),
        json_number("max_pages", max_pages),
        json_number("batches_run", batches_run),
        json_number("pages_touched", pages_touched),
        json_number(
            "preflight_available_attach_mask",
            preflight.available_attach_mask,
        ),
        json_number(
            "preflight_capability_mask",
            preflight.snapshot.capability_mask,
        ),
        json_number(
            "preflight_missing_capabilities",
            preflight.snapshot.missing_capabilities,
        ),
        json_number("missing_btf_state", missing_btf_state),
        json_number("missing_btf_fault", missing_btf_fault),
        json_number("missing_ringbuf_state", missing_ringbuf_state),
        json_number("missing_ringbuf_fault", missing_ringbuf_fault),
        json_number("missing_attach_state", missing_attach_state),
        json_number("missing_attach_fault", missing_attach_fault),
        json_number("missing_verifier_state", missing_verifier_state),
        json_number("missing_verifier_fault", missing_verifier_fault),
        json_number("missing_permission_state", missing_permission_state),
        json_number("missing_permission_fault", missing_permission_fault),
        json_number("first_agent_epoch", FIRST_AGENT_EPOCH),
        json_number("first_runtime_attach_mask", first_runtime_attach_mask),
        json_number(
            "first_runtime_required_attach_mask",
            first_snapshot.required_attach_mask,
        ),
        json_number(
            "first_runtime_capability_mask",
            first_snapshot.capability_mask,
        ),
        json_number("initial_state", initial_observation.state as u8),
        json_number("initial_attach_mask", initial_observation.attach_mask),
        json_number("initial_lost_events", initial_observation.lost_event_count),
        json_number("initial_incidents", initial_observation.incident_count),
        json_number("initial_fault", initial_observation.fault_code),
        json_number("initial_heartbeat_seq", initial_observation.heartbeat_seq),
        json_number("baseline_emitted_events", baseline.emitted_events),
        json_number("baseline_lost_events", baseline.lost_events),
        json_number("baseline_page_faults", baseline.page_faults),
        json_number(
            "baseline_page_fault_threshold_events",
            baseline.page_fault_threshold_events,
        ),
        json_number("saturation_emitted_events", saturation.emitted_events),
        json_number("saturation_lost_events", saturation.lost_events),
        json_number("saturation_page_faults", saturation.page_faults),
        json_number(
            "saturation_page_fault_threshold_events",
            saturation.page_fault_threshold_events,
        ),
        json_number("saturation_records_seen", saturation_poll.records_seen),
        json_number(
            "saturation_incidents_emitted",
            saturation_poll.incidents_emitted,
        ),
        json_number(
            "saturation_malformed_records",
            saturation_poll.malformed_records,
        ),
        json_number(
            "saturation_evidence_rejected",
            saturation_poll.evidence_rejected,
        ),
        json_number(
            "saturation_newly_reported_lost_events",
            saturation_poll.newly_reported_lost_events,
        ),
        json_number("loss_state", loss_observation.state as u8),
        json_number("loss_attach_mask", loss_observation.attach_mask),
        json_number("loss_lost_events", loss_observation.lost_event_count),
        json_number("loss_incidents", loss_observation.incident_count),
        json_number("loss_fault", loss_observation.fault_code),
        json_number("loss_heartbeat_seq", loss_observation.heartbeat_seq),
        json_number("reapplied_state", reapplied_observation.state as u8),
        json_number(
            "reapplied_lost_events",
            reapplied_observation.lost_event_count,
        ),
        json_number("reapplied_fault", reapplied_observation.fault_code),
        json_number(
            "reapplied_heartbeat_seq",
            reapplied_observation.heartbeat_seq,
        ),
        json_number("first_runtime_detached", first_runtime_detached),
        json_number("second_agent_epoch", SECOND_AGENT_EPOCH),
        json_number("restarting_state", restarting_observation.state as u8),
        json_number("restarting_attach_mask", restarting_observation.attach_mask),
        json_number(
            "restarting_lost_events",
            restarting_observation.lost_event_count,
        ),
        json_number(
            "restarting_incidents",
            restarting_observation.incident_count,
        ),
        json_number("restarting_fault", restarting_observation.fault_code),
        json_number(
            "restarting_heartbeat_seq",
            restarting_observation.heartbeat_seq,
        ),
        json_number("second_runtime_attach_mask", second_runtime_attach_mask),
        json_number(
            "second_runtime_required_attach_mask",
            second_snapshot.required_attach_mask,
        ),
        json_number(
            "second_runtime_capability_mask",
            second_snapshot.capability_mask,
        ),
        json_number("recovered_state", recovered_observation.state as u8),
        json_number("recovered_attach_mask", recovered_observation.attach_mask),
        json_number(
            "recovered_lost_events",
            recovered_observation.lost_event_count,
        ),
        json_number("recovered_incidents", recovered_observation.incident_count),
        json_number("recovered_fault", recovered_observation.fault_code),
        json_number(
            "recovered_heartbeat_seq",
            recovered_observation.heartbeat_seq,
        ),
        json_number("recovery_pages_touched", recovery_pages_touched),
        json_number("recovery_records_seen", recovery_poll.records_seen),
        json_number(
            "recovery_incidents_emitted",
            recovery_poll.incidents_emitted,
        ),
        json_number(
            "recovery_malformed_records",
            recovery_poll.malformed_records,
        ),
        json_number(
            "recovery_evidence_rejected",
            recovery_poll.evidence_rejected,
        ),
        json_number(
            "recovery_newly_reported_lost_events",
            recovery_poll.newly_reported_lost_events,
        ),
        json_number("recovery_emitted_events", recovery.emitted_events),
        json_number("recovery_lost_events", recovery.lost_events),
        json_number("recovery_page_faults", recovery.page_faults),
        json_number("final_state", final_observation.state as u8),
        json_number("final_attach_mask", final_observation.attach_mask),
        json_number("final_lost_events", final_observation.lost_event_count),
        json_number("final_incidents", final_observation.incident_count),
        json_number("final_fault", final_observation.fault_code),
        json_number("final_heartbeat_seq", final_observation.heartbeat_seq),
        json_number("second_runtime_detached", second_runtime_detached),
        json_number("cleanup_succeeded", cleanup_succeeded),
    ];
    write_report(&output_path, fields)?;
    println!(
        "observability degradation qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn main() {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let result = (|| {
        let object_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data(
                "usage: observability_degradation_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data(
                "usage: observability_degradation_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected extra qualification argument").into());
        }
        run(object_path, output_path)
    })();
    if let Err(error) = result {
        eprintln!("observability degradation qualification failed: {error}");
        std::process::exit(1);
    }
}
