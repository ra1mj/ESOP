#![cfg(target_os = "linux")]

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    AgentState, EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity, RecommendedAction,
    RuntimeAgent,
};
use esop_ebpf_runtime::{ATTACH_PROCESS_EXIT, BpfRuntime, PollReport, RuntimeConfig};

const BOOT_ID: u64 = 0x4553_4f50_5058_5155;
const AGENT_EPOCH: u64 = 1;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const POLL_LIMIT: usize = 200;
const POLL_SLEEP: Duration = Duration::from_millis(5);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const CHILD_DEADLINE: Duration = Duration::from_secs(2);
const WORKER_RELEASE: u8 = 0x57;
const WORKER_ACK: u8 = 0x41;
const LEADER_RELEASE: u8 = 0x4c;
const OBSERVATION_HEALTHY: u8 = 0;
const OBSERVATION_FAILED: u8 = 2;
const LATCHED_INCIDENT_FAULT: u32 = 0x4542_2002;

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
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

fn set_nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct TrackedChild {
    child: Child,
    input: Option<ChildStdin>,
    output: ChildStdout,
    finished: bool,
}

impl TrackedChild {
    fn spawn(executable: &Path) -> Result<Self, Box<dyn Error>> {
        let mut child = Command::new(executable)
            .arg("--tracked-child")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let Some(input) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid_data("tracked child stdin was unavailable").into());
        };
        let Some(output) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid_data("tracked child stdout was unavailable").into());
        };
        if let Err(error) = set_nonblocking(output.as_raw_fd()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        Ok(Self {
            child,
            input: Some(input),
            output,
            finished: false,
        })
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn send(&mut self, command: u8) -> Result<(), Box<dyn Error>> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| invalid_data("tracked child command pipe was closed"))?;
        input.write_all(&[command])?;
        input.flush()?;
        Ok(())
    }

    fn wait_for_worker_ack(&mut self) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        let mut ack = [0u8; 1];
        loop {
            match self.output.read(&mut ack) {
                Ok(1) if ack[0] == WORKER_ACK => return Ok(()),
                Ok(1) => {
                    return Err(invalid_data("tracked child sent an invalid worker ack").into());
                }
                Ok(0) => {
                    return Err(invalid_data("tracked child closed before worker ack").into());
                }
                Ok(_) => unreachable!("one-byte worker acknowledgement buffer"),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            if let Some(status) = self.child.try_wait()? {
                self.finished = true;
                return Err(invalid_data(format!(
                    "tracked child exited before worker ack: {status}"
                ))
                .into());
            }
            if started.elapsed() >= CHILD_DEADLINE {
                return Err(invalid_data("tracked child worker ack timed out").into());
            }
            thread::sleep(POLL_SLEEP);
        }
    }

    fn wait_for_success(&mut self) -> Result<(), Box<dyn Error>> {
        self.input.take();
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.finished = true;
                if status.success() {
                    return Ok(());
                }
                return Err(invalid_data(format!("tracked child failed: {status}")).into());
            }
            if started.elapsed() >= CHILD_DEADLINE {
                return Err(invalid_data("tracked child leader exit timed out").into());
            }
            thread::sleep(POLL_SLEEP);
        }
    }
}

impl Drop for TrackedChild {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn read_child_command(input: &mut impl Read, expected: u8) -> Result<(), Box<dyn Error>> {
    let mut command = [0u8; 1];
    input.read_exact(&mut command)?;
    if command[0] != expected {
        return Err(invalid_data("tracked child received an invalid command").into());
    }
    Ok(())
}

fn run_tracked_child() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();

    read_child_command(&mut input, WORKER_RELEASE)?;
    thread::spawn(|| {})
        .join()
        .map_err(|_| invalid_data("tracked child worker panicked"))?;
    output.write_all(&[WORKER_ACK])?;
    output.flush()?;

    read_child_command(&mut input, LEADER_RELEASE)?;
    Ok(())
}

fn run_parent(object_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let parent_pid = std::process::id();
    let config = RuntimeConfig {
        enabled_attach_mask: ATTACH_PROCESS_EXIT,
        required_attach_mask: ATTACH_PROCESS_EXIT,
        tracked_pid: parent_pid,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };
    let scheduler_threshold = config.scheduler_latency_threshold_ns;
    let network_threshold = config.network_drop_threshold;
    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_PROCESS_EXIT
        || snapshot.attach_mask != ATTACH_PROCESS_EXIT
        || snapshot.required_attach_mask != ATTACH_PROCESS_EXIT
        || !snapshot.attach_ready()
    {
        return Err(invalid_data("process-exit tracepoint capability was not complete").into());
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    if agent.health().state() != AgentState::Healthy {
        return Err(invalid_data("process-exit observer did not become healthy").into());
    }

    let executable = fs::canonicalize(std::env::current_exe()?)?;
    let mut child = TrackedChild::spawn(&executable)?;
    let child_pid = child.pid();
    if child_pid == 0 || child_pid == parent_pid {
        return Err(invalid_data("tracked child PID was invalid").into());
    }
    runtime.update_tracking(child_pid, scheduler_threshold, network_threshold)?;

    child.send(WORKER_RELEASE)?;
    child.wait_for_worker_ack()?;

    let worker_poll = runtime.poll(&mut agent, 64)?;
    let worker_statistics = runtime.statistics()?;
    if worker_poll.records_seen != 0
        || worker_poll.incidents_emitted != 0
        || worker_poll.malformed_records != 0
        || worker_poll.evidence_rejected != 0
        || worker_poll.newly_reported_lost_events != 0
        || worker_statistics.emitted_events != 0
        || worker_statistics.lost_events != 0
        || worker_statistics.process_exits != 0
        || worker_statistics.thread_exits_ignored != 1
        || worker_statistics.oom_events != 0
        || !agent.correlator().is_empty()
        || agent.correlator().dropped_incidents() != 0
        || agent.health().state() != AgentState::Healthy
    {
        return Err(invalid_data("worker-thread exit suppression was inconsistent").into());
    }
    let worker_observation = agent.heartbeat(1);
    if worker_observation.state as u8 != OBSERVATION_HEALTHY
        || worker_observation.attach_mask != ATTACH_PROCESS_EXIT
        || worker_observation.lost_event_count != 0
        || worker_observation.incident_count != 0
        || worker_observation.fault_code != 0
        || worker_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("worker-thread exit degraded the observer lease").into());
    }

    child.send(LEADER_RELEASE)?;
    child.wait_for_success()?;

    let mut poll_total = PollReport::default();
    let poll_started = Instant::now();
    for _ in 0..POLL_LIMIT {
        if poll_started.elapsed() >= POLL_DEADLINE {
            break;
        }
        let report = runtime.poll(&mut agent, 64)?;
        add_poll_report(&mut poll_total, report);
        if poll_total.records_seen >= 1 && poll_total.incidents_emitted >= 1 {
            break;
        }
        thread::sleep(POLL_SLEEP);
    }
    if poll_total.records_seen != 1 || poll_total.incidents_emitted != 1 {
        return Err(invalid_data("leader exit did not produce exactly one incident").into());
    }
    if agent.correlator().len() != 1 || agent.correlator().dropped_incidents() != 0 {
        return Err(invalid_data("leader exit incident retention was inconsistent").into());
    }
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("leader exit incident was missing"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra leader exit incident was retained").into());
    }
    let statistics = runtime.statistics()?;
    if poll_total.malformed_records != 0
        || poll_total.evidence_rejected != 0
        || poll_total.newly_reported_lost_events != 0
        || statistics.emitted_events != 1
        || statistics.lost_events != 0
        || statistics.process_exits != 1
        || statistics.thread_exits_ignored != 1
        || statistics.oom_events != 0
    {
        return Err(invalid_data("leader exit poll or kernel statistics were inconsistent").into());
    }
    if incident.incident_id == 0
        || incident.boot_id != BOOT_ID
        || incident.agent_epoch != AGENT_EPOCH
        || incident.code != IncidentCode::UserComponentExit
        || incident.severity != IncidentSeverity::Critical
        || incident.recommended_action != RecommendedAction::LatchFault
        || incident.confidence_percent != 100
        || incident.pid != child_pid
        || incident.tid != child_pid
        || incident.irq != 0
        || incident.netdev_ifindex != 0
        || incident.cycle_first != 0
        || incident.cycle_last != 0
        || incident.transition_seq != 0
        || incident.observed_value != 1
        || incident.threshold != 1
        || incident.evidence_count != 1
        || incident.count != 1
        || incident.lost_events != 0
    {
        return Err(invalid_data("leader exit incident fields were inconsistent").into());
    }
    let evidence = incident.evidence[0];
    if evidence.evidence_id == 0
        || evidence.boot_id != BOOT_ID
        || evidence.agent_epoch != AGENT_EPOCH
        || evidence.timestamp_ns == 0
        || evidence.pid != child_pid
        || evidence.tid != child_pid
        || evidence.cpu != incident.cpu
        || evidence.irq != 0
        || evidence.netdev_ifindex != 0
        || evidence.cycle_seq != 0
        || evidence.transition_seq != 0
        || evidence.observed_value != 1
        || evidence.threshold != 1
        || evidence.duration_ns != 0
        || evidence.count != 1
        || evidence.domain != EvidenceDomain::KernelProcess
        || evidence.kind != EvidenceKind::ProcessExit
        || evidence.severity != IncidentSeverity::Critical
        || evidence.detail != 0
    {
        return Err(invalid_data("leader exit evidence fields were inconsistent").into());
    }
    if incident.first_seen_ns != evidence.timestamp_ns
        || incident.last_seen_ns != evidence.timestamp_ns
    {
        return Err(invalid_data("leader exit incident timestamps were inconsistent").into());
    }
    if agent.health().state() != AgentState::Failed {
        return Err(invalid_data("leader exit did not fail the observer lease").into());
    }
    let observation = agent.heartbeat(evidence.timestamp_ns.saturating_add(1));
    if observation.boot_id != BOOT_ID
        || observation.agent_epoch != AGENT_EPOCH
        || observation.state as u8 != OBSERVATION_FAILED
        || observation.attach_mask != ATTACH_PROCESS_EXIT
        || observation.lost_event_count != 0
        || observation.incident_count != 1
        || observation.fault_code != LATCHED_INCIDENT_FAULT
        || observation.heartbeat_seq != 2
    {
        return Err(invalid_data("leader exit observation fields were inconsistent").into());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let report = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"status\": \"qualified\",\n",
            "  \"tracked_child_pid\": {tracked_child_pid},\n",
            "  \"runtime_attach_mask\": {runtime_attach_mask},\n",
            "  \"required_attach_mask\": {required_attach_mask},\n",
            "  \"worker_records_seen\": {worker_records_seen},\n",
            "  \"worker_incidents_emitted\": {worker_incidents_emitted},\n",
            "  \"worker_malformed_records\": {worker_malformed_records},\n",
            "  \"worker_evidence_rejected\": {worker_evidence_rejected},\n",
            "  \"worker_newly_reported_lost_events\": {worker_new_lost},\n",
            "  \"worker_thread_exits_ignored\": {worker_ignored},\n",
            "  \"worker_observation_state\": {worker_state},\n",
            "  \"worker_observation_attach_mask\": {worker_attach_mask},\n",
            "  \"worker_observation_lost_event_count\": {worker_lost},\n",
            "  \"worker_observation_incident_count\": {worker_incidents},\n",
            "  \"worker_observation_fault_code\": {worker_fault},\n",
            "  \"worker_observation_heartbeat_seq\": {worker_heartbeat_seq},\n",
            "  \"records_seen\": {records_seen},\n",
            "  \"incidents_emitted\": {incidents_emitted},\n",
            "  \"malformed_records\": {malformed_records},\n",
            "  \"evidence_rejected\": {evidence_rejected},\n",
            "  \"newly_reported_lost_events\": {new_lost},\n",
            "  \"emitted_events\": {emitted_events},\n",
            "  \"lost_events\": {lost_events},\n",
            "  \"process_exits\": {process_exits},\n",
            "  \"thread_exits_ignored\": {thread_exits_ignored},\n",
            "  \"oom_events\": {oom_events},\n",
            "  \"dropped_incidents\": {dropped_incidents},\n",
            "  \"incident_id\": {incident_id},\n",
            "  \"incident_boot_id\": {incident_boot_id},\n",
            "  \"incident_agent_epoch\": {incident_agent_epoch},\n",
            "  \"incident_code\": {incident_code},\n",
            "  \"incident_severity\": {incident_severity},\n",
            "  \"recommended_action\": {recommended_action},\n",
            "  \"confidence_percent\": {confidence_percent},\n",
            "  \"pid\": {pid},\n",
            "  \"tid\": {tid},\n",
            "  \"cpu\": {cpu},\n",
            "  \"irq\": {irq},\n",
            "  \"netdev_ifindex\": {netdev_ifindex},\n",
            "  \"cycle_first\": {cycle_first},\n",
            "  \"cycle_last\": {cycle_last},\n",
            "  \"transition_seq\": {transition_seq},\n",
            "  \"incident_observed_value\": {incident_observed_value},\n",
            "  \"incident_threshold\": {incident_threshold},\n",
            "  \"incident_evidence_count\": {incident_evidence_count},\n",
            "  \"incident_event_count\": {incident_event_count},\n",
            "  \"incident_lost_events\": {incident_lost_events},\n",
            "  \"evidence_id\": {evidence_id},\n",
            "  \"evidence_boot_id\": {evidence_boot_id},\n",
            "  \"evidence_agent_epoch\": {evidence_agent_epoch},\n",
            "  \"evidence_timestamp_ns\": {evidence_timestamp_ns},\n",
            "  \"evidence_domain\": {evidence_domain},\n",
            "  \"evidence_kind\": {evidence_kind},\n",
            "  \"evidence_severity\": {evidence_severity},\n",
            "  \"evidence_pid\": {evidence_pid},\n",
            "  \"evidence_tid\": {evidence_tid},\n",
            "  \"evidence_cpu\": {evidence_cpu},\n",
            "  \"evidence_irq\": {evidence_irq},\n",
            "  \"evidence_netdev_ifindex\": {evidence_netdev_ifindex},\n",
            "  \"evidence_cycle_seq\": {evidence_cycle_seq},\n",
            "  \"evidence_transition_seq\": {evidence_transition_seq},\n",
            "  \"evidence_observed_value\": {evidence_observed_value},\n",
            "  \"evidence_threshold\": {evidence_threshold},\n",
            "  \"evidence_duration_ns\": {evidence_duration_ns},\n",
            "  \"evidence_event_count\": {evidence_event_count},\n",
            "  \"evidence_detail\": {evidence_detail},\n",
            "  \"observation_boot_id\": {observation_boot_id},\n",
            "  \"observation_agent_epoch\": {observation_agent_epoch},\n",
            "  \"observation_state\": {observation_state},\n",
            "  \"observation_attach_mask\": {observation_attach_mask},\n",
            "  \"observation_lost_event_count\": {observation_lost},\n",
            "  \"observation_incident_count\": {observation_incidents},\n",
            "  \"observation_fault_code\": {observation_fault},\n",
            "  \"observation_heartbeat_seq\": {observation_heartbeat_seq}\n",
            "}}\n"
        ),
        tracked_child_pid = child_pid,
        runtime_attach_mask = runtime.attach_mask(),
        required_attach_mask = snapshot.required_attach_mask,
        worker_records_seen = worker_poll.records_seen,
        worker_incidents_emitted = worker_poll.incidents_emitted,
        worker_malformed_records = worker_poll.malformed_records,
        worker_evidence_rejected = worker_poll.evidence_rejected,
        worker_new_lost = worker_poll.newly_reported_lost_events,
        worker_ignored = worker_statistics.thread_exits_ignored,
        worker_state = worker_observation.state as u8,
        worker_attach_mask = worker_observation.attach_mask,
        worker_lost = worker_observation.lost_event_count,
        worker_incidents = worker_observation.incident_count,
        worker_fault = worker_observation.fault_code,
        worker_heartbeat_seq = worker_observation.heartbeat_seq,
        records_seen = poll_total.records_seen,
        incidents_emitted = poll_total.incidents_emitted,
        malformed_records = poll_total.malformed_records,
        evidence_rejected = poll_total.evidence_rejected,
        new_lost = poll_total.newly_reported_lost_events,
        emitted_events = statistics.emitted_events,
        lost_events = statistics.lost_events,
        process_exits = statistics.process_exits,
        thread_exits_ignored = statistics.thread_exits_ignored,
        oom_events = statistics.oom_events,
        dropped_incidents = agent.correlator().dropped_incidents(),
        incident_id = incident.incident_id,
        incident_boot_id = incident.boot_id,
        incident_agent_epoch = incident.agent_epoch,
        incident_code = incident.code as u8,
        incident_severity = incident.severity as u8,
        recommended_action = incident.recommended_action as u8,
        confidence_percent = incident.confidence_percent,
        pid = incident.pid,
        tid = incident.tid,
        cpu = incident.cpu,
        irq = incident.irq,
        netdev_ifindex = incident.netdev_ifindex,
        cycle_first = incident.cycle_first,
        cycle_last = incident.cycle_last,
        transition_seq = incident.transition_seq,
        incident_observed_value = incident.observed_value,
        incident_threshold = incident.threshold,
        incident_evidence_count = incident.evidence_count,
        incident_event_count = incident.count,
        incident_lost_events = incident.lost_events,
        evidence_id = evidence.evidence_id,
        evidence_boot_id = evidence.boot_id,
        evidence_agent_epoch = evidence.agent_epoch,
        evidence_timestamp_ns = evidence.timestamp_ns,
        evidence_domain = evidence.domain as u8,
        evidence_kind = evidence.kind as u8,
        evidence_severity = evidence.severity as u8,
        evidence_pid = evidence.pid,
        evidence_tid = evidence.tid,
        evidence_cpu = evidence.cpu,
        evidence_irq = evidence.irq,
        evidence_netdev_ifindex = evidence.netdev_ifindex,
        evidence_cycle_seq = evidence.cycle_seq,
        evidence_transition_seq = evidence.transition_seq,
        evidence_observed_value = evidence.observed_value,
        evidence_threshold = evidence.threshold,
        evidence_duration_ns = evidence.duration_ns,
        evidence_event_count = evidence.count,
        evidence_detail = evidence.detail,
        observation_boot_id = observation.boot_id,
        observation_agent_epoch = observation.agent_epoch,
        observation_state = observation.state as u8,
        observation_attach_mask = observation.attach_mask,
        observation_lost = observation.lost_event_count,
        observation_incidents = observation.incident_count,
        observation_fault = observation.fault_code,
        observation_heartbeat_seq = observation.heartbeat_seq,
    );
    fs::write(&temp_path, report)?;
    fs::rename(&temp_path, &output_path)?;
    println!(
        "process-exit runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let first = arguments.next().ok_or_else(|| {
        invalid_data("usage: process_exit_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    if first == OsStr::new("--tracked-child") {
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected tracked child argument").into());
        }
        return run_tracked_child();
    }
    let object_path = PathBuf::from(first);
    let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        invalid_data("usage: process_exit_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    if arguments.next().is_some() {
        return Err(invalid_data("unexpected extra qualification argument").into());
    }
    run_parent(object_path, output_path)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("process-exit runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
