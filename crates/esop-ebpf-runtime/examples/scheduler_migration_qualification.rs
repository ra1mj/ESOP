#![cfg(target_os = "linux")]

use std::error::Error;
use std::fs;
use std::io;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    AgentState, CycleContext, EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity,
    RecommendedAction, RuntimeAgent,
};
use esop_ebpf_runtime::{ATTACH_SCHED_MIGRATE_TASK, BpfRuntime, PollReport, RuntimeConfig};

const BOOT_ID: u64 = 0x4553_4f50_4d49_4752;
const AGENT_EPOCH: u64 = 1;
const CYCLE_SEQ: u64 = 42;
const TRANSITION_SEQ: u64 = 9;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const SCHEDULER_LATENCY_THRESHOLD_NS: u64 = 1_000_000;
const MIGRATION_THRESHOLD: u64 = 2;
const MIGRATION_WINDOW_NS: u64 = 10_000_000_000;
const POLL_LIMIT: usize = 200;
const POLL_SLEEP: Duration = Duration::from_millis(5);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const WORKER_DEADLINE: Duration = Duration::from_secs(2);
const NO_TARGET: u32 = u32::MAX;
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

fn allowed_cpus() -> io::Result<Vec<u16>> {
    let mut set: libc::cpu_set_t = unsafe { zeroed() };
    let result = unsafe { libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), &mut set) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    let mut cpus = Vec::new();
    for cpu in 0..libc::CPU_SETSIZE as usize {
        if cpu <= u16::MAX as usize && unsafe { libc::CPU_ISSET(cpu, &set) } {
            cpus.push(cpu as u16);
        }
    }
    Ok(cpus)
}

fn set_thread_affinity(tid: u32, cpu: u16) -> io::Result<()> {
    if tid == 0 || tid > i32::MAX as u32 || usize::from(cpu) >= libc::CPU_SETSIZE as usize {
        return Err(invalid_data(
            "worker TID or CPU was outside sched_setaffinity range",
        ));
    }
    let mut set: libc::cpu_set_t = unsafe { zeroed() };
    unsafe {
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(usize::from(cpu), &mut set);
    }
    let result =
        unsafe { libc::sched_setaffinity(tid as libc::pid_t, size_of::<libc::cpu_set_t>(), &set) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn current_cpu() -> io::Result<u16> {
    let cpu = unsafe { libc::sched_getcpu() };
    if cpu < 0 {
        return Err(io::Error::last_os_error());
    }
    u16::try_from(cpu).map_err(|_| invalid_data("current CPU did not fit the evidence ABI"))
}

fn current_tid() -> io::Result<u32> {
    let tid = unsafe { libc::syscall(libc::SYS_gettid) };
    if tid <= 0 || tid > i32::MAX as libc::c_long {
        return Err(invalid_data(
            "worker gettid result was outside the supported range",
        ));
    }
    Ok(tid as u32)
}

#[derive(Debug)]
enum WorkerEvent {
    Ready { tid: u32, cpu: u16 },
    Reached { cpu: u16 },
    Failed(String),
}

struct MigrationWorker {
    tid: u32,
    desired_cpu: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    events: mpsc::Receiver<WorkerEvent>,
    handle: Option<JoinHandle<Result<(), String>>>,
}

impl MigrationWorker {
    fn spawn(initial_cpu: u16) -> Result<Self, Box<dyn Error>> {
        let desired_cpu = Arc::new(AtomicU32::new(u32::from(initial_cpu)));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_desired = Arc::clone(&desired_cpu);
        let worker_stop = Arc::clone(&stop);
        let (sender, events) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = run_worker(initial_cpu, worker_desired, worker_stop, &sender);
            if let Err(error) = &result {
                let _ = sender.send(WorkerEvent::Failed(error.clone()));
            }
            result
        });
        let mut worker = Self {
            tid: 0,
            desired_cpu,
            stop,
            events,
            handle: Some(handle),
        };
        match worker.recv_event(WORKER_DEADLINE)? {
            WorkerEvent::Ready { tid, cpu } if cpu == initial_cpu => {
                worker.tid = tid;
                Ok(worker)
            }
            WorkerEvent::Ready { .. } => {
                Err(invalid_data("worker started on an unexpected CPU").into())
            }
            WorkerEvent::Reached { .. } => {
                Err(invalid_data("worker reached event arrived before readiness").into())
            }
            WorkerEvent::Failed(message) => Err(invalid_data(message).into()),
        }
    }

    fn tid(&self) -> u32 {
        self.tid
    }

    fn move_to(&self, cpu: u16) -> Result<(), Box<dyn Error>> {
        self.desired_cpu.store(u32::from(cpu), Ordering::Release);
        set_thread_affinity(self.tid, cpu)?;
        match self.recv_event(WORKER_DEADLINE)? {
            WorkerEvent::Reached { cpu: reached } if reached == cpu => Ok(()),
            WorkerEvent::Reached { .. } => {
                Err(invalid_data("worker acknowledged an unexpected destination CPU").into())
            }
            WorkerEvent::Ready { .. } => {
                Err(invalid_data("worker emitted duplicate readiness").into())
            }
            WorkerEvent::Failed(message) => Err(invalid_data(message).into()),
        }
    }

    fn recv_event(&self, timeout: Duration) -> Result<WorkerEvent, Box<dyn Error>> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => Ok(event),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err(invalid_data("worker acknowledgement timed out").into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(invalid_data("worker acknowledgement channel disconnected").into())
            }
        }
    }

    fn stop_and_join(&mut self) -> Result<(), Box<dyn Error>> {
        self.stop.store(true, Ordering::Release);
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        match handle.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(invalid_data(message).into()),
            Err(_) => Err(invalid_data("migration worker panicked").into()),
        }
    }
}

impl Drop for MigrationWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_worker(
    initial_cpu: u16,
    desired_cpu: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    sender: &mpsc::Sender<WorkerEvent>,
) -> Result<(), String> {
    let tid = current_tid().map_err(|error| format!("gettid failed: {error}"))?;
    set_thread_affinity(tid, initial_cpu)
        .map_err(|error| format!("initial worker affinity failed: {error}"))?;

    let started = Instant::now();
    loop {
        let cpu = current_cpu().map_err(|error| format!("sched_getcpu failed: {error}"))?;
        if cpu == initial_cpu {
            sender
                .send(WorkerEvent::Ready { tid, cpu })
                .map_err(|_| "worker readiness receiver disconnected".to_owned())?;
            break;
        }
        if started.elapsed() >= WORKER_DEADLINE {
            return Err("worker did not execute on its initial CPU".to_owned());
        }
        thread::yield_now();
    }

    let mut acknowledged_cpu = initial_cpu;
    let mut spins = 0u32;
    while !stop.load(Ordering::Acquire) {
        let desired = desired_cpu.load(Ordering::Acquire);
        if desired != NO_TARGET && desired != u32::from(acknowledged_cpu) {
            let target = u16::try_from(desired)
                .map_err(|_| "worker desired CPU did not fit u16".to_owned())?;
            let cpu = current_cpu().map_err(|error| format!("sched_getcpu failed: {error}"))?;
            if cpu == target {
                acknowledged_cpu = cpu;
                sender
                    .send(WorkerEvent::Reached { cpu })
                    .map_err(|_| "worker destination receiver disconnected".to_owned())?;
            }
        }
        spins = spins.wrapping_add(1);
        if spins & 0x3ff == 0 {
            thread::yield_now();
        } else {
            std::hint::spin_loop();
        }
    }
    Ok(())
}

fn run(object_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let cpus = allowed_cpus()?;
    if cpus.len() < 2 {
        return Err(invalid_data(
            "scheduler migration qualification requires at least two allowed CPUs",
        )
        .into());
    }
    let cpu_a = cpus[0];
    let cpu_b = cpus[1];
    if cpu_a == cpu_b {
        return Err(invalid_data("qualification CPUs must be distinct").into());
    }

    let config = RuntimeConfig {
        enabled_attach_mask: ATTACH_SCHED_MIGRATE_TASK,
        required_attach_mask: ATTACH_SCHED_MIGRATE_TASK,
        tracked_pid: std::process::id(),
        scheduler_latency_threshold_ns: SCHEDULER_LATENCY_THRESHOLD_NS,
        scheduler_migration_threshold: MIGRATION_THRESHOLD,
        scheduler_migration_window_ns: MIGRATION_WINDOW_NS,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };
    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_SCHED_MIGRATE_TASK
        || snapshot.attach_mask != ATTACH_SCHED_MIGRATE_TASK
        || snapshot.required_attach_mask != ATTACH_SCHED_MIGRATE_TASK
        || !snapshot.attach_ready()
    {
        return Err(
            invalid_data("scheduler migration tracepoint capability was incomplete").into(),
        );
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    if agent.health().state() != AgentState::Healthy {
        return Err(invalid_data("scheduler migration observer did not become healthy").into());
    }
    let initial_observation = agent.heartbeat(1);
    if initial_observation.state as u8 != OBSERVATION_HEALTHY
        || initial_observation.attach_mask != ATTACH_SCHED_MIGRATE_TASK
        || initial_observation.lost_event_count != 0
        || initial_observation.incident_count != 0
        || initial_observation.fault_code != 0
        || initial_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("initial scheduler observation was inconsistent").into());
    }

    let mut worker = MigrationWorker::spawn(cpu_a)?;
    let worker_tid = worker.tid();
    runtime.update_scheduler_tracking(
        worker_tid,
        SCHEDULER_LATENCY_THRESHOLD_NS,
        MIGRATION_THRESHOLD,
        MIGRATION_WINDOW_NS,
    )?;
    let cycle = CycleContext {
        boot_id: BOOT_ID,
        cycle_seq: CYCLE_SEQ,
        transition_seq: TRANSITION_SEQ,
        wkc_bad: 1,
        expected_wkc: 1,
        actual_wkc: 0,
        ..CycleContext::EMPTY
    };
    runtime.update_cycle_context(cycle)?;
    agent
        .observe_cycle(cycle)
        .map_err(|error| invalid_data(format!("agent rejected cycle context: {error:?}")))?;

    let baseline_statistics = runtime.statistics()?;
    if baseline_statistics.emitted_events != 0
        || baseline_statistics.lost_events != 0
        || baseline_statistics.scheduler_migrations != 0
        || baseline_statistics.scheduler_migration_threshold_events != 0
    {
        return Err(invalid_data("scheduler migration baseline was not empty").into());
    }

    worker.move_to(cpu_b)?;
    let first_poll = runtime.poll(&mut agent, 64)?;
    let first_statistics = runtime.statistics()?;
    if first_poll.records_seen != 0
        || first_poll.incidents_emitted != 0
        || first_poll.malformed_records != 0
        || first_poll.evidence_rejected != 0
        || first_poll.newly_reported_lost_events != 0
        || first_statistics.emitted_events != 0
        || first_statistics.lost_events != 0
        || first_statistics.scheduler_migrations != 1
        || first_statistics.scheduler_migration_threshold_events != 0
        || !agent.correlator().is_empty()
        || agent.health().state() != AgentState::Healthy
    {
        return Err(invalid_data("first migration was not suppressed below threshold").into());
    }

    worker.move_to(cpu_a)?;
    worker.stop_and_join()?;

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
        return Err(invalid_data(
            "second migration did not produce exactly one record and incident",
        )
        .into());
    }
    if agent.correlator().len() != 1 || agent.correlator().dropped_incidents() != 0 {
        return Err(invalid_data("scheduler migration incident retention was inconsistent").into());
    }
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("scheduler migration incident was missing"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra scheduler migration incident").into());
    }

    let statistics = runtime.statistics()?;
    if poll_total.malformed_records != 0
        || poll_total.evidence_rejected != 0
        || poll_total.newly_reported_lost_events != 0
        || statistics.emitted_events != 1
        || statistics.lost_events != 0
        || statistics.scheduler_migrations != 2
        || statistics.scheduler_migration_threshold_events != 1
    {
        return Err(
            invalid_data("scheduler migration poll or statistics were inconsistent").into(),
        );
    }
    if incident.incident_id == 0
        || incident.boot_id != BOOT_ID
        || incident.agent_epoch != AGENT_EPOCH
        || incident.code != IncidentCode::HostSchedulerStall
        || incident.severity != IncidentSeverity::Warning
        || incident.recommended_action != RecommendedAction::DegradeHostObservation
        || incident.confidence_percent != 60
        || incident.pid != 0
        || incident.tid != worker_tid
        || incident.cpu != cpu_a
        || incident.irq != cpu_b
        || incident.netdev_ifindex != 0
        || incident.cycle_first != CYCLE_SEQ
        || incident.cycle_last != CYCLE_SEQ
        || incident.transition_seq != TRANSITION_SEQ
        || incident.observed_value != MIGRATION_THRESHOLD
        || incident.threshold != MIGRATION_THRESHOLD
        || incident.evidence_count != 1
        || incident.count != MIGRATION_THRESHOLD as u32
        || incident.lost_events != 0
    {
        return Err(invalid_data("scheduler migration incident fields were inconsistent").into());
    }

    let evidence = incident.evidence[0];
    if evidence.evidence_id == 0
        || evidence.boot_id != BOOT_ID
        || evidence.agent_epoch != AGENT_EPOCH
        || evidence.timestamp_ns == 0
        || evidence.pid != 0
        || evidence.tid != worker_tid
        || evidence.cpu != cpu_a
        || evidence.irq != cpu_b
        || evidence.netdev_ifindex != 0
        || evidence.cycle_seq != CYCLE_SEQ
        || evidence.transition_seq != TRANSITION_SEQ
        || evidence.observed_value != MIGRATION_THRESHOLD
        || evidence.threshold != MIGRATION_THRESHOLD
        || evidence.duration_ns == 0
        || evidence.duration_ns >= MIGRATION_WINDOW_NS
        || evidence.count != MIGRATION_THRESHOLD as u32
        || evidence.domain != EvidenceDomain::KernelScheduler
        || evidence.kind != EvidenceKind::CpuMigration
        || evidence.severity != IncidentSeverity::Warning
    {
        return Err(invalid_data("scheduler migration evidence fields were inconsistent").into());
    }
    if incident.first_seen_ns != evidence.timestamp_ns
        || incident.last_seen_ns != evidence.timestamp_ns
    {
        return Err(
            invalid_data("scheduler migration incident timestamps were inconsistent").into(),
        );
    }
    if agent.health().state() != AgentState::Degraded {
        return Err(
            invalid_data("scheduler migration incident did not degrade observer health").into(),
        );
    }
    let observation = agent.heartbeat(evidence.timestamp_ns.saturating_add(1));
    if observation.boot_id != BOOT_ID
        || observation.agent_epoch != AGENT_EPOCH
        || observation.state as u8 != OBSERVATION_DEGRADED
        || observation.attach_mask != ATTACH_SCHED_MIGRATE_TASK
        || observation.lost_event_count != 0
        || observation.incident_count != 1
        || observation.fault_code != DEGRADED_INCIDENT_FAULT
        || observation.heartbeat_seq != 2
    {
        return Err(
            invalid_data("scheduler migration observation fields were inconsistent").into(),
        );
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let report = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"status\": \"qualified\",\n",
            "  \"worker_tid\": {worker_tid},\n",
            "  \"cpu_a\": {cpu_a},\n",
            "  \"cpu_b\": {cpu_b},\n",
            "  \"migration_threshold\": {migration_threshold},\n",
            "  \"migration_window_ns\": {migration_window_ns},\n",
            "  \"runtime_attach_mask\": {runtime_attach_mask},\n",
            "  \"required_attach_mask\": {required_attach_mask},\n",
            "  \"initial_observation_boot_id\": {initial_boot_id},\n",
            "  \"initial_observation_agent_epoch\": {initial_agent_epoch},\n",
            "  \"initial_observation_state\": {initial_state},\n",
            "  \"initial_observation_attach_mask\": {initial_attach_mask},\n",
            "  \"initial_observation_lost_event_count\": {initial_lost},\n",
            "  \"initial_observation_incident_count\": {initial_incidents},\n",
            "  \"initial_observation_fault_code\": {initial_fault},\n",
            "  \"initial_observation_heartbeat_seq\": {initial_heartbeat_seq},\n",
            "  \"baseline_emitted_events\": {baseline_emitted},\n",
            "  \"baseline_lost_events\": {baseline_lost},\n",
            "  \"baseline_scheduler_migrations\": {baseline_migrations},\n",
            "  \"baseline_threshold_events\": {baseline_threshold_events},\n",
            "  \"first_records_seen\": {first_records_seen},\n",
            "  \"first_incidents_emitted\": {first_incidents_emitted},\n",
            "  \"first_malformed_records\": {first_malformed_records},\n",
            "  \"first_evidence_rejected\": {first_evidence_rejected},\n",
            "  \"first_newly_reported_lost_events\": {first_new_lost},\n",
            "  \"first_emitted_events\": {first_emitted},\n",
            "  \"first_lost_events\": {first_lost},\n",
            "  \"first_scheduler_migrations\": {first_migrations},\n",
            "  \"first_threshold_events\": {first_threshold_events},\n",
            "  \"records_seen\": {records_seen},\n",
            "  \"incidents_emitted\": {incidents_emitted},\n",
            "  \"malformed_records\": {malformed_records},\n",
            "  \"evidence_rejected\": {evidence_rejected},\n",
            "  \"newly_reported_lost_events\": {new_lost},\n",
            "  \"emitted_events\": {emitted_events},\n",
            "  \"lost_events\": {lost_events},\n",
            "  \"scheduler_migrations\": {scheduler_migrations},\n",
            "  \"scheduler_migration_threshold_events\": {threshold_events},\n",
            "  \"dropped_incidents\": {dropped_incidents},\n",
            "  \"incident_id\": {incident_id},\n",
            "  \"incident_boot_id\": {incident_boot_id},\n",
            "  \"incident_agent_epoch\": {incident_agent_epoch},\n",
            "  \"incident_code\": {incident_code},\n",
            "  \"incident_severity\": {incident_severity},\n",
            "  \"recommended_action\": {recommended_action},\n",
            "  \"confidence_percent\": {confidence_percent},\n",
            "  \"incident_first_seen_ns\": {incident_first_seen_ns},\n",
            "  \"incident_last_seen_ns\": {incident_last_seen_ns},\n",
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
        worker_tid = worker_tid,
        cpu_a = cpu_a,
        cpu_b = cpu_b,
        migration_threshold = MIGRATION_THRESHOLD,
        migration_window_ns = MIGRATION_WINDOW_NS,
        runtime_attach_mask = runtime.attach_mask(),
        required_attach_mask = snapshot.required_attach_mask,
        initial_boot_id = initial_observation.boot_id,
        initial_agent_epoch = initial_observation.agent_epoch,
        initial_state = initial_observation.state as u8,
        initial_attach_mask = initial_observation.attach_mask,
        initial_lost = initial_observation.lost_event_count,
        initial_incidents = initial_observation.incident_count,
        initial_fault = initial_observation.fault_code,
        initial_heartbeat_seq = initial_observation.heartbeat_seq,
        baseline_emitted = baseline_statistics.emitted_events,
        baseline_lost = baseline_statistics.lost_events,
        baseline_migrations = baseline_statistics.scheduler_migrations,
        baseline_threshold_events = baseline_statistics.scheduler_migration_threshold_events,
        first_records_seen = first_poll.records_seen,
        first_incidents_emitted = first_poll.incidents_emitted,
        first_malformed_records = first_poll.malformed_records,
        first_evidence_rejected = first_poll.evidence_rejected,
        first_new_lost = first_poll.newly_reported_lost_events,
        first_emitted = first_statistics.emitted_events,
        first_lost = first_statistics.lost_events,
        first_migrations = first_statistics.scheduler_migrations,
        first_threshold_events = first_statistics.scheduler_migration_threshold_events,
        records_seen = poll_total.records_seen,
        incidents_emitted = poll_total.incidents_emitted,
        malformed_records = poll_total.malformed_records,
        evidence_rejected = poll_total.evidence_rejected,
        new_lost = poll_total.newly_reported_lost_events,
        emitted_events = statistics.emitted_events,
        lost_events = statistics.lost_events,
        scheduler_migrations = statistics.scheduler_migrations,
        threshold_events = statistics.scheduler_migration_threshold_events,
        dropped_incidents = agent.correlator().dropped_incidents(),
        incident_id = incident.incident_id,
        incident_boot_id = incident.boot_id,
        incident_agent_epoch = incident.agent_epoch,
        incident_code = incident.code as u8,
        incident_severity = incident.severity as u8,
        recommended_action = incident.recommended_action as u8,
        confidence_percent = incident.confidence_percent,
        incident_first_seen_ns = incident.first_seen_ns,
        incident_last_seen_ns = incident.last_seen_ns,
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
        "scheduler migration runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn main() {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let result = (|| -> Result<(), Box<dyn Error>> {
        let object_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data(
                "usage: scheduler_migration_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data(
                "usage: scheduler_migration_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected extra qualification argument").into());
        }
        run(object_path, output_path)
    })();
    if let Err(error) = result {
        eprintln!("scheduler migration runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
