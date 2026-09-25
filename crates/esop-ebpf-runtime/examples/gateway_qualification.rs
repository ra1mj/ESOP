#![cfg(target_os = "linux")]

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    CycleContext, EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity, RecommendedAction,
    RuntimeAgent, RuntimeEvidence,
};
use esop_ebpf_runtime::{
    ATTACH_GATEWAY_CALLBACK, ATTACH_GATEWAY_PUBLISH, BpfRuntime, PollReport, RuntimeConfig,
};
use esop_zenoh_gateway::RouteKind;
use esop_zenoh_gateway::runtime::{
    GatewayCallbackOutcome, GatewayPublishOutcome, esop_zenoh_gateway_callback_begin_v1,
    esop_zenoh_gateway_callback_end_v1, esop_zenoh_gateway_publish_begin_v1,
    esop_zenoh_gateway_publish_end_v1,
};

const BOOT_ID: u64 = 0x4553_4f50_4757_5155;
const AGENT_EPOCH: u64 = 1;
const CYCLE_SEQ: u64 = 1;
const PUBLISH_REQUEST_ID: u64 = 101;
const CALLBACK_REQUEST_ID: u64 = 202;
const GATEWAY_THRESHOLD_NS: u64 = 5_000_000;
const INJECTED_DELAY_NS: u64 = 25_000_000;
const POLL_LIMIT: usize = 200;
const POLL_SLEEP: Duration = Duration::from_millis(5);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const REQUIRED_GATEWAY_ATTACH_MASK: u64 = ATTACH_GATEWAY_PUBLISH | ATTACH_GATEWAY_CALLBACK;
const PUBLISH_DETAIL: u8 =
    (GatewayPublishOutcome::Success as u8) << 4 | RouteKind::Diagnostic as u8;
const CALLBACK_DETAIL: u8 =
    (GatewayCallbackOutcome::Completed as u8) << 4 | RouteKind::Command as u8;

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

fn require_gateway_evidence(
    evidence: RuntimeEvidence,
    request_id: u64,
    detail: u8,
    pid: u32,
    tid: u32,
) -> Result<RuntimeEvidence, Box<dyn Error>> {
    if evidence.evidence_id != request_id
        || evidence.boot_id != BOOT_ID
        || evidence.agent_epoch != AGENT_EPOCH
        || evidence.cycle_seq != CYCLE_SEQ
        || evidence.pid != pid
        || evidence.tid != tid
        || evidence.netdev_ifindex != 0
        || evidence.observed_value != evidence.duration_ns
        || evidence.threshold != GATEWAY_THRESHOLD_NS
        || evidence.duration_ns <= evidence.threshold
        || evidence.count != 1
        || evidence.domain != EvidenceDomain::UserZenoh
        || evidence.kind != EvidenceKind::GatewayStall
        || evidence.severity != IncidentSeverity::Error
        || evidence.detail != detail
    {
        return Err(invalid_data(format!(
            "gateway evidence {request_id} fields were inconsistent"
        ))
        .into());
    }
    Ok(evidence)
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let object_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        invalid_data("usage: gateway_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        invalid_data("usage: gateway_qualification <esop_runtime.bpf.o> <qualification.json>")
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
        gateway_stall_threshold_ns: GATEWAY_THRESHOLD_NS,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };

    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let executable = fs::canonicalize(std::env::current_exe()?)?;
    if !runtime.attach_gateway_publish_probes(&executable, Some(pid_filter), true)? {
        return Err(invalid_data("required gateway publish uprobe pair was unavailable").into());
    }
    if !runtime.attach_gateway_callback_probes(&executable, Some(pid_filter), true)? {
        return Err(invalid_data("required gateway callback uprobe pair was unavailable").into());
    }
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != REQUIRED_GATEWAY_ATTACH_MASK
        || snapshot.attach_mask != REQUIRED_GATEWAY_ATTACH_MASK
        || snapshot.required_attach_mask != REQUIRED_GATEWAY_ATTACH_MASK
        || !snapshot.attach_ready()
    {
        return Err(invalid_data("gateway uprobe capability was not complete").into());
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
    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    agent
        .observe_cycle(cycle)
        .map_err(|error| invalid_data(format!("agent rejected cycle context: {error:?}")))?;

    esop_zenoh_gateway_publish_begin_v1(PUBLISH_REQUEST_ID, RouteKind::Diagnostic as u32);
    thread::sleep(Duration::from_nanos(INJECTED_DELAY_NS));
    esop_zenoh_gateway_publish_end_v1(
        PUBLISH_REQUEST_ID,
        RouteKind::Diagnostic as u32,
        GatewayPublishOutcome::Success as u32,
    );

    esop_zenoh_gateway_callback_begin_v1(CALLBACK_REQUEST_ID, RouteKind::Command as u32);
    thread::sleep(Duration::from_nanos(INJECTED_DELAY_NS));
    esop_zenoh_gateway_callback_end_v1(
        CALLBACK_REQUEST_ID,
        RouteKind::Command as u32,
        GatewayCallbackOutcome::Completed as u32,
    );

    let mut poll_total = PollReport::default();
    let poll_started = Instant::now();
    for _ in 0..POLL_LIMIT {
        if poll_started.elapsed() > POLL_DEADLINE {
            break;
        }
        let report = runtime.poll(&mut agent, 64)?;
        add_poll_report(&mut poll_total, report);
        if poll_total.records_seen >= 2 && poll_total.incidents_emitted >= 2 {
            break;
        }
        thread::sleep(POLL_SLEEP);
    }
    if poll_total.records_seen != 2 || poll_total.incidents_emitted != 2 {
        return Err(
            invalid_data("gateway evidence poll did not produce exactly two records").into(),
        );
    }
    if agent.correlator().len() != 1 || agent.correlator().dropped_incidents() != 0 {
        return Err(
            invalid_data("gateway evidence did not merge into one retained incident").into(),
        );
    }
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("gateway incident poll timed out"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra gateway incident was retained").into());
    }
    let statistics = runtime.statistics()?;

    if poll_total.malformed_records != 0
        || poll_total.evidence_rejected != 0
        || poll_total.newly_reported_lost_events != 0
        || statistics.lost_events != 0
        || statistics.gateway_probe_begins != 2
        || statistics.gateway_probe_completions != 2
        || statistics.gateway_stalls != 2
        || statistics.gateway_probe_mismatches != 0
    {
        return Err(invalid_data("gateway poll or kernel statistics were inconsistent").into());
    }
    if incident.code != IncidentCode::GatewayStall
        || incident.severity != IncidentSeverity::Error
        || incident.recommended_action != RecommendedAction::ControlledStop
        || incident.confidence_percent != 75
        || incident.pid != pid
        || incident.tid != tid
        || incident.netdev_ifindex != 0
        || incident.cycle_first != CYCLE_SEQ
        || incident.cycle_last != CYCLE_SEQ
        || incident.evidence_count != 2
        || incident.count != 2
        || incident.lost_events != 0
    {
        return Err(invalid_data("gateway incident fields were inconsistent").into());
    }

    let evidence = &incident.evidence[..usize::from(incident.evidence_count)];
    let publish = evidence
        .iter()
        .copied()
        .find(|item| item.evidence_id == PUBLISH_REQUEST_ID)
        .ok_or_else(|| invalid_data("publish gateway evidence was missing"))?;
    let callback = evidence
        .iter()
        .copied()
        .find(|item| item.evidence_id == CALLBACK_REQUEST_ID)
        .ok_or_else(|| invalid_data("callback gateway evidence was missing"))?;
    let publish = require_gateway_evidence(publish, PUBLISH_REQUEST_ID, PUBLISH_DETAIL, pid, tid)?;
    let callback =
        require_gateway_evidence(callback, CALLBACK_REQUEST_ID, CALLBACK_DETAIL, pid, tid)?;

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
            "  \"gateway_probe_begins\": {begins},\n",
            "  \"gateway_probe_completions\": {completions},\n",
            "  \"gateway_stalls\": {stalls},\n",
            "  \"gateway_probe_mismatches\": {mismatches},\n",
            "  \"dropped_incidents\": {dropped_incidents},\n",
            "  \"incident_code\": {incident_code},\n",
            "  \"incident_severity\": {incident_severity},\n",
            "  \"recommended_action\": {recommended_action},\n",
            "  \"confidence_percent\": {confidence},\n",
            "  \"pid\": {pid},\n",
            "  \"tid\": {tid},\n",
            "  \"cycle_first\": {cycle_first},\n",
            "  \"cycle_last\": {cycle_last},\n",
            "  \"incident_evidence_count\": {incident_evidence_count},\n",
            "  \"incident_event_count\": {incident_event_count},\n",
            "  \"incident_lost_events\": {incident_lost_events},\n",
            "  \"publish_request_id\": {publish_request_id},\n",
            "  \"publish_domain\": {publish_domain},\n",
            "  \"publish_kind\": {publish_kind},\n",
            "  \"publish_severity\": {publish_severity},\n",
            "  \"publish_detail\": {publish_detail},\n",
            "  \"publish_pid\": {publish_pid},\n",
            "  \"publish_tid\": {publish_tid},\n",
            "  \"publish_cycle_seq\": {publish_cycle_seq},\n",
            "  \"publish_event_count\": {publish_event_count},\n",
            "  \"publish_observed_value\": {publish_observed_value},\n",
            "  \"publish_evidence_threshold\": {publish_evidence_threshold},\n",
            "  \"publish_duration_ns\": {publish_duration_ns},\n",
            "  \"callback_request_id\": {callback_request_id},\n",
            "  \"callback_domain\": {callback_domain},\n",
            "  \"callback_kind\": {callback_kind},\n",
            "  \"callback_severity\": {callback_severity},\n",
            "  \"callback_detail\": {callback_detail},\n",
            "  \"callback_pid\": {callback_pid},\n",
            "  \"callback_tid\": {callback_tid},\n",
            "  \"callback_cycle_seq\": {callback_cycle_seq},\n",
            "  \"callback_event_count\": {callback_event_count},\n",
            "  \"callback_observed_value\": {callback_observed_value},\n",
            "  \"callback_evidence_threshold\": {callback_evidence_threshold},\n",
            "  \"callback_duration_ns\": {callback_duration_ns}\n",
            "}}\n"
        ),
        threshold = GATEWAY_THRESHOLD_NS,
        delay = INJECTED_DELAY_NS,
        attach_mask = runtime.attach_mask(),
        required_mask = snapshot.required_attach_mask,
        records_seen = poll_total.records_seen,
        incidents_emitted = poll_total.incidents_emitted,
        malformed_records = poll_total.malformed_records,
        evidence_rejected = poll_total.evidence_rejected,
        new_lost = poll_total.newly_reported_lost_events,
        lost_events = statistics.lost_events,
        begins = statistics.gateway_probe_begins,
        completions = statistics.gateway_probe_completions,
        stalls = statistics.gateway_stalls,
        mismatches = statistics.gateway_probe_mismatches,
        dropped_incidents = agent.correlator().dropped_incidents(),
        incident_code = incident.code as u8,
        incident_severity = incident.severity as u8,
        recommended_action = incident.recommended_action as u8,
        confidence = incident.confidence_percent,
        pid = incident.pid,
        tid = incident.tid,
        cycle_first = incident.cycle_first,
        cycle_last = incident.cycle_last,
        incident_evidence_count = incident.evidence_count,
        incident_event_count = incident.count,
        incident_lost_events = incident.lost_events,
        publish_request_id = publish.evidence_id,
        publish_domain = publish.domain as u8,
        publish_kind = publish.kind as u8,
        publish_severity = publish.severity as u8,
        publish_detail = publish.detail,
        publish_pid = publish.pid,
        publish_tid = publish.tid,
        publish_cycle_seq = publish.cycle_seq,
        publish_event_count = publish.count,
        publish_observed_value = publish.observed_value,
        publish_evidence_threshold = publish.threshold,
        publish_duration_ns = publish.duration_ns,
        callback_request_id = callback.evidence_id,
        callback_domain = callback.domain as u8,
        callback_kind = callback.kind as u8,
        callback_severity = callback.severity as u8,
        callback_detail = callback.detail,
        callback_pid = callback.pid,
        callback_tid = callback.tid,
        callback_cycle_seq = callback.cycle_seq,
        callback_event_count = callback.count,
        callback_observed_value = callback.observed_value,
        callback_evidence_threshold = callback.threshold,
        callback_duration_ns = callback.duration_ns,
    );
    fs::write(&temp_path, report)?;
    fs::rename(&temp_path, &output_path)?;
    println!(
        "gateway runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("gateway runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
