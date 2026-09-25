#![cfg(target_os = "linux")]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::mem::{size_of, zeroed};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    AgentState, CycleContext, EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity,
    RecommendedAction, RuntimeAgent, RuntimeIncident,
};
use esop_ebpf_runtime::{ATTACH_SOFTIRQ, BpfRuntime, KernelStats, PollReport, RuntimeConfig};

const BOOT_ID: u64 = 0x4553_4f50_534f_4654;
const AGENT_EPOCH: u64 = 1;
const CALIBRATION_CYCLE_SEQ: u64 = 41;
const FORMAL_CYCLE_SEQ: u64 = 42;
const TRANSITION_SEQ: u64 = 9;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const CALIBRATION_THRESHOLD_NS: u64 = 1;
const CALIBRATION_DIVISOR: u64 = 8;
const NET_RX_VECTOR: u32 = 3;
const UDP_SEGMENT_BYTES: usize = 1_200;
const UDP_SEGMENT_COUNT: usize = 54;
const UDP_PAYLOAD_BYTES: usize = UDP_SEGMENT_BYTES * UDP_SEGMENT_COUNT;
const SOCKET_BUFFER_BYTES: libc::c_int = 4 * 1024 * 1024;
const QUIET_SAMPLE: Duration = Duration::from_millis(25);
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_SLEEP: Duration = Duration::from_millis(2);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const OBSERVATION_HEALTHY: u8 = 0;
const OBSERVATION_DEGRADED: u8 = 1;
const DEGRADED_INCIDENT_FAULT: u32 = 0x4542_2001;

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn report_temp_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_os_string();
    path.push(".tmp");
    PathBuf::from(path)
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

fn current_tid() -> io::Result<u32> {
    let tid = unsafe { libc::syscall(libc::SYS_gettid) };
    if tid <= 0 || tid > i32::MAX as libc::c_long {
        return Err(invalid_data(
            "gettid result was outside the supported range",
        ));
    }
    Ok(tid as u32)
}

fn current_cpu() -> io::Result<u16> {
    let cpu = unsafe { libc::sched_getcpu() };
    if cpu < 0 {
        return Err(io::Error::last_os_error());
    }
    u16::try_from(cpu).map_err(|_| invalid_data("current CPU did not fit the evidence ABI"))
}

fn allowed_cpus() -> io::Result<Vec<u16>> {
    let mut set: libc::cpu_set_t = unsafe { zeroed() };
    if unsafe { libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), &mut set) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut cpus = Vec::new();
    for cpu in 0..libc::CPU_SETSIZE as usize {
        if cpu < usize::from(u16::MAX) && unsafe { libc::CPU_ISSET(cpu, &set) } {
            cpus.push(cpu as u16);
        }
    }
    if cpus.is_empty() {
        return Err(invalid_data(
            "no allowed CPU fits the interrupt evidence ABI",
        ));
    }
    Ok(cpus)
}

fn set_current_thread_affinity(cpu: u16) -> io::Result<()> {
    if usize::from(cpu) >= libc::CPU_SETSIZE as usize {
        return Err(invalid_data(
            "target CPU was outside sched_setaffinity range",
        ));
    }
    let mut set: libc::cpu_set_t = unsafe { zeroed() };
    unsafe {
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(usize::from(cpu), &mut set);
    }
    if unsafe { libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if current_cpu()? != cpu {
        return Err(invalid_data("fixture did not execute on the selected CPU"));
    }
    Ok(())
}

fn net_rx_counts() -> io::Result<Vec<u64>> {
    let contents = fs::read_to_string("/proc/softirqs")?;
    let line = contents
        .lines()
        .find(|line| line.trim_start().starts_with("NET_RX:"))
        .ok_or_else(|| invalid_data("/proc/softirqs did not expose NET_RX"))?;
    let (_, values) = line
        .split_once(':')
        .ok_or_else(|| invalid_data("NET_RX softirq line was malformed"))?;
    let counts = values
        .split_whitespace()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| invalid_data("NET_RX softirq count was not an integer"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if counts.is_empty() {
        return Err(invalid_data("NET_RX softirq line had no CPU counts"));
    }
    Ok(counts)
}

#[derive(Clone, Copy)]
struct CpuSelection {
    cpu: u16,
    before: u64,
    after: u64,
    delta: u64,
}

fn select_quiet_cpu() -> io::Result<CpuSelection> {
    let allowed = allowed_cpus()?;
    let before = net_rx_counts()?;
    thread::sleep(QUIET_SAMPLE);
    let after = net_rx_counts()?;
    let mut selected = None;
    for cpu in allowed {
        let index = usize::from(cpu);
        let (Some(&start), Some(&end)) = (before.get(index), after.get(index)) else {
            continue;
        };
        let candidate = CpuSelection {
            cpu,
            before: start,
            after: end,
            delta: end.saturating_sub(start),
        };
        if selected.is_none_or(|current: CpuSelection| {
            (candidate.delta, candidate.cpu) < (current.delta, current.cpu)
        }) {
            selected = Some(candidate);
        }
    }
    selected.ok_or_else(|| invalid_data("allowed CPUs were absent from /proc/softirqs"))
}

struct GsoSockets {
    sender: UdpSocket,
    receiver: UdpSocket,
}

impl GsoSockets {
    fn new() -> io::Result<Self> {
        let receiver = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        receiver.set_read_timeout(Some(IO_TIMEOUT))?;
        set_receive_buffer(&receiver)?;
        let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        sender.connect(receiver.local_addr()?)?;
        Ok(Self { sender, receiver })
    }

    fn inject(self, cpu: u16) -> Result<InjectionResult, Box<dyn Error>> {
        if current_cpu()? != cpu {
            return Err(invalid_data("fixture migrated before softirq injection").into());
        }
        let counts_before = net_rx_counts()?;
        let net_rx_before = *counts_before
            .get(usize::from(cpu))
            .ok_or_else(|| invalid_data("selected CPU had no NET_RX counter"))?;
        let payload = vec![0x5a; UDP_PAYLOAD_BYTES];
        let sent_bytes = send_udp_segment(&self.sender, &payload, UDP_SEGMENT_BYTES as u16)?;
        if sent_bytes != UDP_PAYLOAD_BYTES {
            return Err(invalid_data(format!(
                "UDP GSO send was partial: {sent_bytes}/{UDP_PAYLOAD_BYTES}"
            ))
            .into());
        }
        if current_cpu()? != cpu {
            return Err(invalid_data("fixture migrated during softirq injection").into());
        }
        let counts_after = net_rx_counts()?;
        let net_rx_after = *counts_after
            .get(usize::from(cpu))
            .ok_or_else(|| invalid_data("selected CPU lost its NET_RX counter"))?;
        let net_rx_delta = net_rx_after.saturating_sub(net_rx_before);
        if net_rx_delta != 1 {
            return Err(invalid_data(format!(
                "expected one target-CPU NET_RX execution, observed {net_rx_delta}"
            ))
            .into());
        }

        let mut received_datagrams = 0usize;
        let mut buffer = [0u8; UDP_SEGMENT_BYTES];
        while received_datagrams < UDP_SEGMENT_COUNT {
            let received = self.receiver.recv(&mut buffer)?;
            if received != UDP_SEGMENT_BYTES {
                return Err(invalid_data(format!(
                    "UDP GSO segment had length {received}, expected {UDP_SEGMENT_BYTES}"
                ))
                .into());
            }
            received_datagrams += 1;
        }
        Ok(InjectionResult {
            sent_bytes,
            received_datagrams,
            net_rx_before,
            net_rx_after,
            net_rx_delta,
        })
    }
}

fn set_receive_buffer(socket: &UdpSocket) -> io::Result<()> {
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&SOCKET_BUFFER_BYTES as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn send_udp_segment(socket: &UdpSocket, payload: &[u8], segment_size: u16) -> io::Result<usize> {
    let mut iovec = libc::iovec {
        iov_base: payload.as_ptr().cast_mut().cast(),
        iov_len: payload.len(),
    };
    let control_len = unsafe { libc::CMSG_SPACE(size_of::<u16>() as libc::c_uint) as usize };
    let mut control = [0usize; 4];
    if control_len > size_of::<[usize; 4]>() {
        return Err(invalid_data("UDP_SEGMENT control buffer was too small"));
    }
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control_len;
    let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
    if header.is_null() {
        return Err(invalid_data("UDP_SEGMENT control header was unavailable"));
    }
    unsafe {
        (*header).cmsg_level = libc::SOL_UDP;
        (*header).cmsg_type = libc::UDP_SEGMENT;
        (*header).cmsg_len = libc::CMSG_LEN(size_of::<u16>() as libc::c_uint) as usize;
        std::ptr::write_unaligned(libc::CMSG_DATA(header).cast::<u16>(), segment_size);
    }
    let sent = unsafe { libc::sendmsg(socket.as_raw_fd(), &message, 0) };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    usize::try_from(sent).map_err(|_| invalid_data("sendmsg returned an invalid byte count"))
}

#[derive(Clone, Copy)]
struct InjectionResult {
    sent_bytes: usize,
    received_datagrams: usize,
    net_rx_before: u64,
    net_rx_after: u64,
    net_rx_delta: u64,
}

#[derive(Clone, Copy)]
struct ObservationSnapshot {
    boot_id: u64,
    agent_epoch: u64,
    state: u8,
    attach_mask: u64,
    lost_event_count: u32,
    incident_count: u32,
    fault_code: u32,
    heartbeat_seq: u64,
}

#[derive(Clone, Copy)]
struct PhaseResult {
    threshold_ns: u64,
    runtime_attach_mask: u64,
    required_attach_mask: u64,
    injection: InjectionResult,
    initial_observation: ObservationSnapshot,
    baseline: KernelStats,
    poll: PollReport,
    statistics: KernelStats,
    dropped_incidents: u32,
    incident: RuntimeIncident,
    observation: ObservationSnapshot,
}

fn run_phase(
    object_path: &Path,
    cpu: u16,
    fixture_pid: u32,
    fixture_tid: u32,
    cycle_seq: u64,
    threshold_ns: u64,
) -> Result<PhaseResult, Box<dyn Error>> {
    let sockets = GsoSockets::new()?;
    let config = RuntimeConfig {
        enabled_attach_mask: ATTACH_SOFTIRQ,
        required_attach_mask: ATTACH_SOFTIRQ,
        irq_duration_threshold_ns: u64::MAX,
        softirq_duration_threshold_ns: threshold_ns,
        interrupt_filter_cpu: cpu,
        interrupt_filter_vector: NET_RX_VECTOR,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };
    let mut runtime = BpfRuntime::load(object_path, config)?;
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_SOFTIRQ
        || snapshot.attach_mask != ATTACH_SOFTIRQ
        || snapshot.required_attach_mask != ATTACH_SOFTIRQ
        || !snapshot.attach_ready()
        || runtime.kernel_context().interrupt_filter_cpu != cpu
        || runtime.kernel_context().interrupt_filter_vector != NET_RX_VECTOR
    {
        return Err(invalid_data("softirq tracepoint capability or filter was incomplete").into());
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    if agent.health().state() != AgentState::Healthy {
        return Err(invalid_data("softirq observer did not become healthy").into());
    }
    let initial = agent.heartbeat(1);
    let initial_observation = ObservationSnapshot {
        boot_id: initial.boot_id,
        agent_epoch: initial.agent_epoch,
        state: initial.state as u8,
        attach_mask: initial.attach_mask,
        lost_event_count: initial.lost_event_count,
        incident_count: initial.incident_count,
        fault_code: initial.fault_code,
        heartbeat_seq: initial.heartbeat_seq,
    };
    if initial_observation.state != OBSERVATION_HEALTHY
        || initial_observation.attach_mask != ATTACH_SOFTIRQ
        || initial_observation.lost_event_count != 0
        || initial_observation.incident_count != 0
        || initial_observation.fault_code != 0
        || initial_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("initial softirq observation was inconsistent").into());
    }

    let cycle = CycleContext {
        boot_id: BOOT_ID,
        cycle_seq,
        transition_seq: TRANSITION_SEQ,
        deadline_miss: 1,
        expected_wkc: 1,
        actual_wkc: 0,
        ..CycleContext::EMPTY
    };
    runtime.update_cycle_context(cycle)?;
    agent
        .observe_cycle(cycle)
        .map_err(|error| invalid_data(format!("agent rejected cycle context: {error:?}")))?;

    let baseline = runtime.statistics()?;
    require_empty_interrupt_statistics(baseline, "softirq baseline")?;
    let injection = sockets.inject(cpu)?;

    let mut poll = PollReport::default();
    let started = Instant::now();
    while started.elapsed() < POLL_DEADLINE {
        add_poll_report(&mut poll, runtime.poll(&mut agent, 64)?);
        if poll.records_seen >= 1 && poll.incidents_emitted >= 1 {
            break;
        }
        thread::sleep(POLL_SLEEP);
    }
    if poll.records_seen != 1 || poll.incidents_emitted != 1 {
        return Err(invalid_data(format!(
            "softirq injection produced records/incidents {}/{}",
            poll.records_seen, poll.incidents_emitted
        ))
        .into());
    }
    if poll.malformed_records != 0
        || poll.evidence_rejected != 0
        || poll.newly_reported_lost_events != 0
        || agent.correlator().len() != 1
        || agent.correlator().dropped_incidents() != 0
    {
        return Err(invalid_data("softirq poll or incident retention was inconsistent").into());
    }
    let dropped_incidents = agent.correlator().dropped_incidents();
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("softirq incident was missing"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra softirq incident").into());
    }
    require_softirq_incident(
        incident,
        cpu,
        fixture_pid,
        fixture_tid,
        cycle_seq,
        threshold_ns,
    )?;

    let statistics = runtime.statistics()?;
    if statistics.emitted_events != 1
        || statistics.lost_events != 0
        || statistics.irq_samples != 0
        || statistics.irq_overruns != 0
        || statistics.softirq_samples != 1
        || statistics.softirq_overruns != 1
    {
        return Err(invalid_data(format!(
            "softirq statistics were inconsistent: {statistics:?}"
        ))
        .into());
    }
    if agent.health().state() != AgentState::Degraded {
        return Err(invalid_data("softirq incident did not degrade observer health").into());
    }
    let heartbeat = agent.heartbeat(incident.last_seen_ns.saturating_add(1));
    let observation = ObservationSnapshot {
        boot_id: heartbeat.boot_id,
        agent_epoch: heartbeat.agent_epoch,
        state: heartbeat.state as u8,
        attach_mask: heartbeat.attach_mask,
        lost_event_count: heartbeat.lost_event_count,
        incident_count: heartbeat.incident_count,
        fault_code: heartbeat.fault_code,
        heartbeat_seq: heartbeat.heartbeat_seq,
    };
    if observation.state != OBSERVATION_DEGRADED
        || observation.attach_mask != ATTACH_SOFTIRQ
        || observation.lost_event_count != 0
        || observation.incident_count != 1
        || observation.fault_code != DEGRADED_INCIDENT_FAULT
        || observation.heartbeat_seq != 2
    {
        return Err(invalid_data("final softirq observation was inconsistent").into());
    }

    Ok(PhaseResult {
        threshold_ns,
        runtime_attach_mask: runtime.attach_mask(),
        required_attach_mask: snapshot.required_attach_mask,
        injection,
        initial_observation,
        baseline,
        poll,
        statistics,
        dropped_incidents,
        incident,
        observation,
    })
}

fn require_empty_interrupt_statistics(stats: KernelStats, label: &str) -> io::Result<()> {
    if stats.emitted_events != 0
        || stats.lost_events != 0
        || stats.irq_samples != 0
        || stats.irq_overruns != 0
        || stats.softirq_samples != 0
        || stats.softirq_overruns != 0
    {
        return Err(invalid_data(format!("{label} was not empty: {stats:?}")));
    }
    Ok(())
}

fn require_softirq_incident(
    incident: RuntimeIncident,
    cpu: u16,
    fixture_pid: u32,
    fixture_tid: u32,
    cycle_seq: u64,
    threshold_ns: u64,
) -> Result<(), Box<dyn Error>> {
    if incident.incident_id == 0
        || incident.boot_id != BOOT_ID
        || incident.agent_epoch != AGENT_EPOCH
        || incident.code != IncidentCode::HostIrqStorm
        || incident.severity != IncidentSeverity::Error
        || incident.recommended_action != RecommendedAction::ControlledStop
        || incident.confidence_percent != 70
        || incident.pid != fixture_pid
        || incident.tid != fixture_tid
        || incident.cpu != cpu
        || incident.irq != NET_RX_VECTOR as u16
        || incident.netdev_ifindex != 0
        || incident.cycle_first != cycle_seq
        || incident.cycle_last != cycle_seq
        || incident.transition_seq != TRANSITION_SEQ
        || incident.observed_value <= threshold_ns
        || incident.threshold != threshold_ns
        || incident.evidence_count != 1
        || incident.count != 1
        || incident.lost_events != 0
    {
        return Err(invalid_data(format!(
            "softirq incident fields were inconsistent: {incident:?}"
        ))
        .into());
    }
    let evidence = incident.evidence[0];
    if evidence.evidence_id == 0
        || evidence.evidence_id != evidence.timestamp_ns
        || evidence.boot_id != BOOT_ID
        || evidence.agent_epoch != AGENT_EPOCH
        || evidence.pid != fixture_pid
        || evidence.tid != fixture_tid
        || evidence.cpu != cpu
        || evidence.irq != NET_RX_VECTOR as u16
        || evidence.netdev_ifindex != 0
        || evidence.cycle_seq != cycle_seq
        || evidence.transition_seq != TRANSITION_SEQ
        || evidence.observed_value != evidence.duration_ns
        || evidence.threshold != threshold_ns
        || evidence.duration_ns <= threshold_ns
        || evidence.count != 1
        || evidence.domain != EvidenceDomain::KernelIrq
        || evidence.kind != EvidenceKind::SoftirqCpuTime
        || evidence.severity != IncidentSeverity::Error
        || evidence.detail != 0
        || incident.first_seen_ns != evidence.timestamp_ns
        || incident.last_seen_ns != evidence.timestamp_ns
        || incident.observed_value != evidence.duration_ns
    {
        return Err(invalid_data(format!(
            "softirq evidence fields were inconsistent: {evidence:?}"
        ))
        .into());
    }
    Ok(())
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
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let fixture_pid = std::process::id();
    let fixture_tid = current_tid()?;
    let selection = select_quiet_cpu()?;
    set_current_thread_affinity(selection.cpu)?;

    let calibration = run_phase(
        &object_path,
        selection.cpu,
        fixture_pid,
        fixture_tid,
        CALIBRATION_CYCLE_SEQ,
        CALIBRATION_THRESHOLD_NS,
    )?;
    let calibration_duration_ns = calibration.incident.evidence[0].duration_ns;
    let formal_threshold_ns = (calibration_duration_ns / CALIBRATION_DIVISOR).max(1);
    let formal = run_phase(
        &object_path,
        selection.cpu,
        fixture_pid,
        fixture_tid,
        FORMAL_CYCLE_SEQ,
        formal_threshold_ns,
    )?;
    let incident = formal.incident;
    let evidence = incident.evidence[0];

    let fields = vec![
        json_number("schema_version", 1),
        ("status", "\"qualified\"".to_owned()),
        json_number("fixture_pid", fixture_pid),
        json_number("fixture_tid", fixture_tid),
        json_number("target_cpu", selection.cpu),
        json_number("quiet_net_rx_before", selection.before),
        json_number("quiet_net_rx_after", selection.after),
        json_number("quiet_net_rx_delta", selection.delta),
        json_number("softirq_vector", NET_RX_VECTOR),
        json_number("udp_segment_bytes", UDP_SEGMENT_BYTES),
        json_number("udp_segment_count", UDP_SEGMENT_COUNT),
        json_number("udp_payload_bytes", UDP_PAYLOAD_BYTES),
        json_number("calibration_divisor", CALIBRATION_DIVISOR),
        json_number("calibration_threshold_ns", calibration.threshold_ns),
        json_number("calibration_duration_ns", calibration_duration_ns),
        json_number("calibration_sent_bytes", calibration.injection.sent_bytes),
        json_number(
            "calibration_received_datagrams",
            calibration.injection.received_datagrams,
        ),
        json_number(
            "calibration_net_rx_before",
            calibration.injection.net_rx_before,
        ),
        json_number(
            "calibration_net_rx_after",
            calibration.injection.net_rx_after,
        ),
        json_number(
            "calibration_net_rx_delta",
            calibration.injection.net_rx_delta,
        ),
        json_number(
            "calibration_runtime_attach_mask",
            calibration.runtime_attach_mask,
        ),
        json_number(
            "calibration_required_attach_mask",
            calibration.required_attach_mask,
        ),
        json_number(
            "calibration_baseline_emitted_events",
            calibration.baseline.emitted_events,
        ),
        json_number(
            "calibration_baseline_lost_events",
            calibration.baseline.lost_events,
        ),
        json_number(
            "calibration_baseline_irq_samples",
            calibration.baseline.irq_samples,
        ),
        json_number(
            "calibration_baseline_irq_overruns",
            calibration.baseline.irq_overruns,
        ),
        json_number(
            "calibration_baseline_softirq_samples",
            calibration.baseline.softirq_samples,
        ),
        json_number(
            "calibration_baseline_softirq_overruns",
            calibration.baseline.softirq_overruns,
        ),
        json_number("calibration_records_seen", calibration.poll.records_seen),
        json_number(
            "calibration_incidents_emitted",
            calibration.poll.incidents_emitted,
        ),
        json_number(
            "calibration_malformed_records",
            calibration.poll.malformed_records,
        ),
        json_number(
            "calibration_evidence_rejected",
            calibration.poll.evidence_rejected,
        ),
        json_number(
            "calibration_newly_reported_lost_events",
            calibration.poll.newly_reported_lost_events,
        ),
        json_number(
            "calibration_emitted_events",
            calibration.statistics.emitted_events,
        ),
        json_number(
            "calibration_lost_events",
            calibration.statistics.lost_events,
        ),
        json_number(
            "calibration_irq_samples",
            calibration.statistics.irq_samples,
        ),
        json_number(
            "calibration_irq_overruns",
            calibration.statistics.irq_overruns,
        ),
        json_number(
            "calibration_softirq_samples",
            calibration.statistics.softirq_samples,
        ),
        json_number(
            "calibration_softirq_overruns",
            calibration.statistics.softirq_overruns,
        ),
        json_number(
            "calibration_dropped_incidents",
            calibration.dropped_incidents,
        ),
        json_number("formal_threshold_ns", formal.threshold_ns),
        json_number("formal_sent_bytes", formal.injection.sent_bytes),
        json_number(
            "formal_received_datagrams",
            formal.injection.received_datagrams,
        ),
        json_number("formal_net_rx_before", formal.injection.net_rx_before),
        json_number("formal_net_rx_after", formal.injection.net_rx_after),
        json_number("formal_net_rx_delta", formal.injection.net_rx_delta),
        json_number("runtime_attach_mask", formal.runtime_attach_mask),
        json_number("required_attach_mask", formal.required_attach_mask),
        json_number("interrupt_filter_cpu", selection.cpu),
        json_number("interrupt_filter_vector", NET_RX_VECTOR),
        json_number(
            "initial_observation_boot_id",
            formal.initial_observation.boot_id,
        ),
        json_number(
            "initial_observation_agent_epoch",
            formal.initial_observation.agent_epoch,
        ),
        json_number(
            "initial_observation_state",
            formal.initial_observation.state,
        ),
        json_number(
            "initial_observation_attach_mask",
            formal.initial_observation.attach_mask,
        ),
        json_number(
            "initial_observation_lost_event_count",
            formal.initial_observation.lost_event_count,
        ),
        json_number(
            "initial_observation_incident_count",
            formal.initial_observation.incident_count,
        ),
        json_number(
            "initial_observation_fault_code",
            formal.initial_observation.fault_code,
        ),
        json_number(
            "initial_observation_heartbeat_seq",
            formal.initial_observation.heartbeat_seq,
        ),
        json_number("baseline_emitted_events", formal.baseline.emitted_events),
        json_number("baseline_lost_events", formal.baseline.lost_events),
        json_number("baseline_irq_samples", formal.baseline.irq_samples),
        json_number("baseline_irq_overruns", formal.baseline.irq_overruns),
        json_number("baseline_softirq_samples", formal.baseline.softirq_samples),
        json_number(
            "baseline_softirq_overruns",
            formal.baseline.softirq_overruns,
        ),
        json_number("records_seen", formal.poll.records_seen),
        json_number("incidents_emitted", formal.poll.incidents_emitted),
        json_number("malformed_records", formal.poll.malformed_records),
        json_number("evidence_rejected", formal.poll.evidence_rejected),
        json_number(
            "newly_reported_lost_events",
            formal.poll.newly_reported_lost_events,
        ),
        json_number("emitted_events", formal.statistics.emitted_events),
        json_number("lost_events", formal.statistics.lost_events),
        json_number("irq_samples", formal.statistics.irq_samples),
        json_number("irq_overruns", formal.statistics.irq_overruns),
        json_number("softirq_samples", formal.statistics.softirq_samples),
        json_number("softirq_overruns", formal.statistics.softirq_overruns),
        json_number("dropped_incidents", formal.dropped_incidents),
        json_number("incident_id", incident.incident_id),
        json_number("incident_boot_id", incident.boot_id),
        json_number("incident_agent_epoch", incident.agent_epoch),
        json_number("incident_code", incident.code as u8),
        json_number("incident_severity", incident.severity as u8),
        json_number("recommended_action", incident.recommended_action as u8),
        json_number("confidence_percent", incident.confidence_percent),
        json_number("incident_first_seen_ns", incident.first_seen_ns),
        json_number("incident_last_seen_ns", incident.last_seen_ns),
        json_number("pid", incident.pid),
        json_number("tid", incident.tid),
        json_number("cpu", incident.cpu),
        json_number("irq", incident.irq),
        json_number("netdev_ifindex", incident.netdev_ifindex),
        json_number("cycle_first", incident.cycle_first),
        json_number("cycle_last", incident.cycle_last),
        json_number("transition_seq", incident.transition_seq),
        json_number("incident_observed_value", incident.observed_value),
        json_number("incident_threshold", incident.threshold),
        json_number("incident_evidence_count", incident.evidence_count),
        json_number("incident_event_count", incident.count),
        json_number("incident_lost_events", incident.lost_events),
        json_number("evidence_id", evidence.evidence_id),
        json_number("evidence_boot_id", evidence.boot_id),
        json_number("evidence_agent_epoch", evidence.agent_epoch),
        json_number("evidence_timestamp_ns", evidence.timestamp_ns),
        json_number("evidence_domain", evidence.domain as u8),
        json_number("evidence_kind", evidence.kind as u8),
        json_number("evidence_severity", evidence.severity as u8),
        json_number("evidence_pid", evidence.pid),
        json_number("evidence_tid", evidence.tid),
        json_number("evidence_cpu", evidence.cpu),
        json_number("evidence_irq", evidence.irq),
        json_number("evidence_netdev_ifindex", evidence.netdev_ifindex),
        json_number("evidence_cycle_seq", evidence.cycle_seq),
        json_number("evidence_transition_seq", evidence.transition_seq),
        json_number("evidence_observed_value", evidence.observed_value),
        json_number("evidence_threshold", evidence.threshold),
        json_number("evidence_duration_ns", evidence.duration_ns),
        json_number("evidence_event_count", evidence.count),
        json_number("evidence_detail", evidence.detail),
        json_number("observation_boot_id", formal.observation.boot_id),
        json_number("observation_agent_epoch", formal.observation.agent_epoch),
        json_number("observation_state", formal.observation.state),
        json_number("observation_attach_mask", formal.observation.attach_mask),
        json_number(
            "observation_lost_event_count",
            formal.observation.lost_event_count,
        ),
        json_number(
            "observation_incident_count",
            formal.observation.incident_count,
        ),
        json_number("observation_fault_code", formal.observation.fault_code),
        json_number(
            "observation_heartbeat_seq",
            formal.observation.heartbeat_seq,
        ),
    ];
    write_report(&output_path, fields)?;
    println!(
        "softirq runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn main() {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let result = (|| -> Result<(), Box<dyn Error>> {
        let object_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data("usage: softirq_qualification <esop_runtime.bpf.o> <qualification.json>")
        })?;
        let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data("usage: softirq_qualification <esop_runtime.bpf.o> <qualification.json>")
        })?;
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected extra qualification argument").into());
        }
        run(object_path, output_path)
    })();
    if let Err(error) = result {
        eprintln!("softirq runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
