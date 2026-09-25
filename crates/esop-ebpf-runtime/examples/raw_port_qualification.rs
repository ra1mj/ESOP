#![cfg(target_os = "linux")]

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    CycleContext, IncidentCode, IncidentSeverity, RecommendedAction, RuntimeAgent,
};
use esop_ebpf_runtime::{ATTACH_RAW_PORT, BpfRuntime, PollReport, RuntimeConfig};
use esop_ethercat_linux_port::{RawPortOperation, RawPortRxOutcome};

unsafe extern "C" {
    fn esop_linux_raw_port_operation_begin_v1(ifindex: u32, operation: u32);
    fn esop_linux_raw_port_operation_end_v1(ifindex: u32, operation: u32, outcome: u32);
}

const BOOT_ID: u64 = 0x4553_4f50_5155_414c;
const AGENT_EPOCH: u64 = 1;
const CYCLE_SEQ: u64 = 1;
const IFINDEX: u32 = 7;
const RAW_PORT_THRESHOLD_NS: u64 = 5_000_000;
const INJECTED_DELAY_NS: u64 = 25_000_000;
const POLL_LIMIT: usize = 200;
const POLL_SLEEP: Duration = Duration::from_millis(5);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const RX_FRAME_DETAIL: u8 = (RawPortRxOutcome::Frame as u8) << 4 | RawPortOperation::Rx as u8;

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn current_tid() -> Result<u32, Box<dyn Error>> {
    let tid = unsafe { libc::syscall(libc::SYS_gettid) };
    Ok(u32::try_from(tid).map_err(|_| invalid_data("gettid returned an invalid value"))?)
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

fn report_temp_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_os_string();
    path.push(".tmp");
    PathBuf::from(path)
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let object_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        invalid_data("usage: raw_port_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        invalid_data("usage: raw_port_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    if arguments.next().is_some() {
        return Err(invalid_data("unexpected extra qualification argument").into());
    }

    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let pid = std::process::id();
    let pid_filter = i32::try_from(pid).map_err(|_| invalid_data("PID exceeds i32"))?;
    let tid = current_tid()?;
    let config = RuntimeConfig {
        enabled_attach_mask: 0,
        required_attach_mask: 0,
        tracked_pid: pid,
        raw_port_stall_threshold_ns: RAW_PORT_THRESHOLD_NS,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };

    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let executable = fs::canonicalize(std::env::current_exe()?)?;
    if !runtime.attach_raw_port_probes(&executable, Some(pid_filter), true)? {
        return Err(invalid_data("required raw-port uprobe pair was unavailable").into());
    }
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_RAW_PORT
        || snapshot.attach_mask != ATTACH_RAW_PORT
        || snapshot.required_attach_mask != ATTACH_RAW_PORT
        || !snapshot.attach_ready()
    {
        return Err(invalid_data("raw-port uprobe capability was not complete").into());
    }

    let cycle = CycleContext {
        boot_id: BOOT_ID,
        cycle_seq: CYCLE_SEQ,
        transition_seq: 1,
        wkc_bad: 1,
        expected_wkc: 1,
        actual_wkc: 0,
        ..CycleContext::EMPTY
    };
    runtime.update_cycle_context(cycle)?;
    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, 100_000_000);
    runtime.apply_capability_snapshot(&mut agent);
    agent
        .observe_cycle(cycle)
        .map_err(|error| invalid_data(format!("agent rejected cycle context: {error:?}")))?;

    let (receiver, mut sender) = UnixStream::pair()?;
    let release = thread::spawn(move || -> io::Result<()> {
        thread::sleep(Duration::from_nanos(INJECTED_DELAY_NS));
        sender.write_all(&[0x5a])
    });

    unsafe {
        esop_linux_raw_port_operation_begin_v1(IFINDEX, RawPortOperation::Rx as u32);
    }
    let mut byte = [0u8; 1];
    let received = unsafe {
        libc::recv(
            receiver.as_raw_fd(),
            byte.as_mut_ptr().cast(),
            byte.len(),
            0,
        )
    };
    let receive_error = (received < 0).then(io::Error::last_os_error);
    let outcome = if received == 1 {
        RawPortRxOutcome::Frame
    } else {
        RawPortRxOutcome::SyscallError
    };
    unsafe {
        esop_linux_raw_port_operation_end_v1(IFINDEX, RawPortOperation::Rx as u32, outcome as u32);
    }
    release
        .join()
        .map_err(|_| invalid_data("delayed recv release thread panicked"))??;
    if let Some(error) = receive_error {
        return Err(error.into());
    }
    if received != 1 || byte[0] != 0x5a {
        return Err(invalid_data("delayed recv did not return the expected byte").into());
    }

    let mut poll_total = PollReport::default();
    let poll_started = Instant::now();
    let mut incident = None;
    for _ in 0..POLL_LIMIT {
        if poll_started.elapsed() > POLL_DEADLINE {
            break;
        }
        let report = runtime.poll(&mut agent, 64)?;
        add_poll_report(&mut poll_total, report);
        if let Some(observed) = agent.pop_incident() {
            incident = Some(observed);
            break;
        }
        thread::sleep(POLL_SLEEP);
    }
    let incident = incident.ok_or_else(|| invalid_data("raw-port incident poll timed out"))?;
    let statistics = runtime.statistics()?;

    if poll_total.records_seen == 0
        || poll_total.incidents_emitted != 1
        || poll_total.malformed_records != 0
        || poll_total.evidence_rejected != 0
        || poll_total.newly_reported_lost_events != 0
        || statistics.lost_events != 0
        || statistics.raw_port_probe_begins != 1
        || statistics.raw_port_probe_completions != 1
        || statistics.raw_port_stalls != 1
        || statistics.raw_port_probe_mismatches != 0
    {
        return Err(invalid_data("raw-port poll or kernel statistics were inconsistent").into());
    }
    if incident.code != IncidentCode::HostPortStall
        || incident.severity != IncidentSeverity::Error
        || incident.recommended_action != RecommendedAction::ControlledStop
        || incident.confidence_percent != 80
        || incident.pid != pid
        || incident.tid != tid
        || incident.netdev_ifindex != IFINDEX
        || incident.cycle_first != CYCLE_SEQ
        || incident.cycle_last != CYCLE_SEQ
        || incident.evidence_count != 1
        || incident.count != 1
        || incident.lost_events != 0
    {
        return Err(invalid_data("raw-port incident fields were inconsistent").into());
    }
    let evidence = incident.evidence[0];
    if evidence.evidence_id == 0
        || evidence.pid != pid
        || evidence.tid != tid
        || evidence.netdev_ifindex != IFINDEX
        || evidence.cycle_seq != CYCLE_SEQ
        || evidence.detail != RX_FRAME_DETAIL
        || evidence.threshold != RAW_PORT_THRESHOLD_NS
        || evidence.duration_ns <= evidence.threshold
        || evidence.observed_value != evidence.duration_ns
        || evidence.count != 1
    {
        return Err(invalid_data("raw-port evidence fields were inconsistent").into());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let report = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"status\": \"qualified\",\n",
            "  \"threshold_ns\": {threshold},\n",
            "  \"injected_delay_ns\": {delay},\n",
            "  \"runtime_attach_mask\": {attach_mask},\n",
            "  \"required_attach_mask\": {required_mask},\n",
            "  \"records_seen\": {records_seen},\n",
            "  \"incidents_emitted\": {incidents_emitted},\n",
            "  \"malformed_records\": {malformed_records},\n",
            "  \"evidence_rejected\": {evidence_rejected},\n",
            "  \"newly_reported_lost_events\": {new_lost},\n",
            "  \"lost_events\": {lost_events},\n",
            "  \"raw_port_probe_begins\": {begins},\n",
            "  \"raw_port_probe_completions\": {completions},\n",
            "  \"raw_port_stalls\": {stalls},\n",
            "  \"raw_port_probe_mismatches\": {mismatches},\n",
            "  \"incident_code\": {incident_code},\n",
            "  \"incident_severity\": {incident_severity},\n",
            "  \"recommended_action\": {recommended_action},\n",
            "  \"confidence_percent\": {confidence},\n",
            "  \"pid\": {pid},\n",
            "  \"tid\": {tid},\n",
            "  \"netdev_ifindex\": {ifindex},\n",
            "  \"cycle_seq\": {cycle_seq},\n",
            "  \"detail\": {detail},\n",
            "  \"incident_evidence_count\": {incident_evidence_count},\n",
            "  \"event_count\": {event_count},\n",
            "  \"evidence_id\": {evidence_id},\n",
            "  \"observed_value\": {observed_value},\n",
            "  \"evidence_threshold\": {evidence_threshold},\n",
            "  \"duration_ns\": {duration_ns}\n",
            "}}\n"
        ),
        threshold = RAW_PORT_THRESHOLD_NS,
        delay = INJECTED_DELAY_NS,
        attach_mask = runtime.attach_mask(),
        required_mask = snapshot.required_attach_mask,
        records_seen = poll_total.records_seen,
        incidents_emitted = poll_total.incidents_emitted,
        malformed_records = poll_total.malformed_records,
        evidence_rejected = poll_total.evidence_rejected,
        new_lost = poll_total.newly_reported_lost_events,
        lost_events = statistics.lost_events,
        begins = statistics.raw_port_probe_begins,
        completions = statistics.raw_port_probe_completions,
        stalls = statistics.raw_port_stalls,
        mismatches = statistics.raw_port_probe_mismatches,
        incident_code = incident.code as u8,
        incident_severity = incident.severity as u8,
        recommended_action = incident.recommended_action as u8,
        confidence = incident.confidence_percent,
        pid = incident.pid,
        tid = incident.tid,
        ifindex = incident.netdev_ifindex,
        cycle_seq = incident.cycle_first,
        detail = evidence.detail,
        incident_evidence_count = incident.evidence_count,
        event_count = evidence.count,
        evidence_id = evidence.evidence_id,
        observed_value = evidence.observed_value,
        evidence_threshold = evidence.threshold,
        duration_ns = evidence.duration_ns,
    );
    fs::write(&temp_path, report)?;
    fs::rename(&temp_path, &output_path)?;
    println!(
        "raw-port runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("raw-port runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
