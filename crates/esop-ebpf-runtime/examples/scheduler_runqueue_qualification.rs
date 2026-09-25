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
use esop_ebpf_runtime::{
    ATTACH_SCHED_SWITCH, ATTACH_SCHED_WAKEUP, BpfRuntime, PollReport, RuntimeConfig,
};

const BOOT_ID: u64 = 0x4553_4f50_5251_5545;
const AGENT_EPOCH: u64 = 1;
const CYCLE_SEQ: u64 = 42;
const TRANSITION_SEQ: u64 = 9;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const SCHEDULER_ATTACH_MASK: u64 = ATTACH_SCHED_WAKEUP | ATTACH_SCHED_SWITCH;
const SCHEDULER_LATENCY_THRESHOLD_NS: u64 = 5_000_000;
const BLOCKER_HOLD_NS: u64 = 25_000_000;
const MAX_QUALIFIED_LATENCY_NS: u64 = 1_000_000_000;
const POLL_LIMIT: usize = 200;
const POLL_SLEEP: Duration = Duration::from_millis(5);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const THREAD_DEADLINE: Duration = Duration::from_secs(2);
const OBSERVATION_HEALTHY: u8 = 0;
const OBSERVATION_DEGRADED: u8 = 1;
const DEGRADED_INCIDENT_FAULT: u32 = 0x4542_2001;
const FUTEX_WAIT_PRIVATE: libc::c_int = libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG;
const FUTEX_WAKE_PRIVATE: libc::c_int = libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG;

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

fn set_thread_affinity(tid: u32, cpu: u16) -> io::Result<()> {
    if tid == 0 || tid > i32::MAX as u32 || usize::from(cpu) >= libc::CPU_SETSIZE as usize {
        return Err(invalid_data(
            "TID or CPU was outside sched_setaffinity range",
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

fn wait_for_current_cpu(expected: u16, deadline: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        if current_cpu()? == expected {
            return Ok(());
        }
        if started.elapsed() >= deadline {
            return Err(invalid_data("thread did not execute on its requested CPU"));
        }
        thread::yield_now();
    }
}

fn fifo_priority_range() -> io::Result<(i32, i32)> {
    let minimum = unsafe { libc::sched_get_priority_min(libc::SCHED_FIFO) };
    if minimum < 0 {
        return Err(io::Error::last_os_error());
    }
    let maximum = unsafe { libc::sched_get_priority_max(libc::SCHED_FIFO) };
    if maximum < 0 {
        return Err(io::Error::last_os_error());
    }
    if minimum == 0 || minimum > maximum {
        return Err(invalid_data(
            "host returned an invalid SCHED_FIFO priority range",
        ));
    }
    Ok((minimum, maximum))
}

fn set_fifo_scheduler(tid: u32, priority: i32) -> io::Result<()> {
    if tid == 0 || tid > i32::MAX as u32 || priority <= 0 {
        return Err(invalid_data("invalid TID or SCHED_FIFO priority"));
    }
    let mut parameter: libc::sched_param = unsafe { zeroed() };
    parameter.sched_priority = priority;
    if unsafe { libc::sched_setscheduler(tid as libc::pid_t, libc::SCHED_FIFO, &parameter) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let policy = unsafe { libc::sched_getscheduler(tid as libc::pid_t) };
    if policy < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut actual: libc::sched_param = unsafe { zeroed() };
    if unsafe { libc::sched_getparam(tid as libc::pid_t, &mut actual) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if policy != libc::SCHED_FIFO || actual.sched_priority != priority {
        return Err(invalid_data("SCHED_FIFO policy verification failed"));
    }
    Ok(())
}

fn futex_wait(word: &AtomicU32, expected: u32) -> io::Result<()> {
    loop {
        let result = unsafe {
            libc::syscall(
                libc::SYS_futex,
                word.as_ptr(),
                FUTEX_WAIT_PRIVATE,
                expected,
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0_u32,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EAGAIN) if word.load(Ordering::Acquire) != expected => return Ok(()),
            _ => return Err(error),
        }
    }
}

fn futex_wake_one(word: &AtomicU32) -> io::Result<u32> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_futex,
            word.as_ptr(),
            FUTEX_WAKE_PRIVATE,
            1_u32,
            std::ptr::null::<libc::timespec>(),
            std::ptr::null::<u32>(),
            0_u32,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    u32::try_from(result).map_err(|_| invalid_data("futex wake count did not fit u32"))
}

fn task_state(tid: u32) -> io::Result<u8> {
    let path = format!("/proc/self/task/{tid}/stat");
    let stat = fs::read(path)?;
    let marker = stat
        .windows(2)
        .rposition(|window| window == b") ")
        .ok_or_else(|| invalid_data("task stat record had no command terminator"))?;
    stat.get(marker + 2)
        .copied()
        .ok_or_else(|| invalid_data("task stat record had no state"))
}

fn wait_for_task_sleep(tid: u32, deadline: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        if task_state(tid)? == b'S' {
            return Ok(());
        }
        if started.elapsed() >= deadline {
            return Err(invalid_data(
                "target did not enter interruptible futex sleep",
            ));
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Debug)]
enum TargetEvent {
    Ready { tid: u32, cpu: u16 },
    Failed(String),
}

struct RunqueueTarget {
    tid: u32,
    gate: Arc<AtomicU32>,
    completed: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<(), String>>>,
}

impl RunqueueTarget {
    fn spawn(cpu: u16) -> Result<Self, Box<dyn Error>> {
        let gate = Arc::new(AtomicU32::new(0));
        let completed = Arc::new(AtomicBool::new(false));
        let worker_gate = Arc::clone(&gate);
        let worker_completed = Arc::clone(&completed);
        let (sender, events) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let tid =
                    current_tid().map_err(|error| format!("target gettid failed: {error}"))?;
                set_thread_affinity(tid, cpu)
                    .map_err(|error| format!("target affinity failed: {error}"))?;
                wait_for_current_cpu(cpu, THREAD_DEADLINE)
                    .map_err(|error| format!("target CPU acknowledgement failed: {error}"))?;
                sender
                    .send(TargetEvent::Ready { tid, cpu })
                    .map_err(|_| "target readiness receiver disconnected".to_owned())?;
                futex_wait(&worker_gate, 0)
                    .map_err(|error| format!("target futex wait failed: {error}"))?;
                worker_completed.store(true, Ordering::Release);
                Ok(())
            })();
            if let Err(error) = &result {
                let _ = sender.send(TargetEvent::Failed(error.clone()));
            }
            result
        });

        let event = match events.recv_timeout(THREAD_DEADLINE) {
            Ok(event) => event,
            Err(error) => {
                gate.store(1, Ordering::Release);
                let _ = futex_wake_one(&gate);
                let _ = handle.join();
                return Err(invalid_data(format!("target readiness failed: {error}")).into());
            }
        };
        match event {
            TargetEvent::Ready { tid, cpu: actual } if actual == cpu => Ok(Self {
                tid,
                gate,
                completed,
                handle: Some(handle),
            }),
            TargetEvent::Ready { .. } => {
                gate.store(1, Ordering::Release);
                let _ = futex_wake_one(&gate);
                let _ = handle.join();
                Err(invalid_data("target reported an unexpected CPU").into())
            }
            TargetEvent::Failed(message) => {
                let _ = handle.join();
                Err(invalid_data(message).into())
            }
        }
    }

    fn tid(&self) -> u32 {
        self.tid
    }

    fn completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    fn wake_one(&self) -> io::Result<u32> {
        self.gate.store(1, Ordering::Release);
        futex_wake_one(&self.gate)
    }

    fn wait_and_join(&mut self) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        while !self.completed() {
            if self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
                break;
            }
            if started.elapsed() >= THREAD_DEADLINE {
                return Err(invalid_data("target completion timed out").into());
            }
            thread::sleep(Duration::from_millis(1));
        }
        let handle = self
            .handle
            .take()
            .ok_or_else(|| invalid_data("target handle was already joined"))?;
        match handle.join() {
            Ok(Ok(())) if self.completed() => Ok(()),
            Ok(Ok(())) => Err(invalid_data("target exited without completion").into()),
            Ok(Err(message)) => Err(invalid_data(message).into()),
            Err(_) => Err(invalid_data("target thread panicked").into()),
        }
    }
}

impl Drop for RunqueueTarget {
    fn drop(&mut self) {
        self.gate.store(1, Ordering::Release);
        let _ = futex_wake_one(&self.gate);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct FifoBlocker {
    ready: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<io::Result<()>>>,
}

impl FifoBlocker {
    fn spawn(cpu: u16, priority: i32) -> Result<Self, Box<dyn Error>> {
        let ready = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_ready = Arc::clone(&ready);
        let worker_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let tid = current_tid()?;
            set_thread_affinity(tid, cpu)?;
            wait_for_current_cpu(cpu, THREAD_DEADLINE)?;
            set_fifo_scheduler(tid, priority)?;
            worker_ready.store(true, Ordering::Release);
            while !worker_stop.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            Ok(())
        });
        let mut blocker = Self {
            ready,
            stop,
            handle: Some(handle),
        };
        blocker.wait_until_ready()?;
        Ok(blocker)
    }

    fn wait_until_ready(&mut self) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        loop {
            if self.ready.load(Ordering::Acquire) {
                return Ok(());
            }
            if self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
                let handle = self
                    .handle
                    .take()
                    .ok_or_else(|| invalid_data("blocker handle disappeared"))?;
                return match handle.join() {
                    Ok(Ok(())) => Err(invalid_data("blocker exited before readiness").into()),
                    Ok(Err(error)) => Err(error.into()),
                    Err(_) => Err(invalid_data("blocker thread panicked").into()),
                };
            }
            if started.elapsed() >= THREAD_DEADLINE {
                return Err(invalid_data("FIFO blocker readiness timed out").into());
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn stop_and_join(&mut self) -> Result<(), Box<dyn Error>> {
        self.stop.store(true, Ordering::Release);
        let started = Instant::now();
        while self
            .handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
        {
            if started.elapsed() >= THREAD_DEADLINE {
                return Err(invalid_data("FIFO blocker stop timed out").into());
            }
            thread::sleep(Duration::from_millis(1));
        }
        let handle = self
            .handle
            .take()
            .ok_or_else(|| invalid_data("blocker handle was already joined"))?;
        match handle.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => Err(invalid_data("blocker thread panicked").into()),
        }
    }
}

impl Drop for FifoBlocker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(object_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let cpus = allowed_cpus()?;
    if cpus.len() < 2 {
        return Err(invalid_data(
            "scheduler runqueue qualification requires at least two allowed CPUs",
        )
        .into());
    }
    let cpu_a = cpus[0];
    let cpu_b = cpus[1];
    if cpu_a == cpu_b {
        return Err(invalid_data("qualification CPUs must be distinct").into());
    }
    let controller_tid = current_tid()?;
    set_thread_affinity(controller_tid, cpu_b)?;
    wait_for_current_cpu(cpu_b, THREAD_DEADLINE)?;

    let (fifo_priority_min, fifo_priority_max) = fifo_priority_range()?;
    let fifo_priority = fifo_priority_min;
    let mut target = RunqueueTarget::spawn(cpu_a)?;
    wait_for_task_sleep(target.tid(), THREAD_DEADLINE)?;
    let target_sleep_confirmed = 1_u32;

    let config = RuntimeConfig {
        enabled_attach_mask: SCHEDULER_ATTACH_MASK,
        required_attach_mask: SCHEDULER_ATTACH_MASK,
        tracked_pid: std::process::id(),
        scheduler_latency_threshold_ns: SCHEDULER_LATENCY_THRESHOLD_NS,
        scheduler_tid: target.tid(),
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };
    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != SCHEDULER_ATTACH_MASK
        || snapshot.attach_mask != SCHEDULER_ATTACH_MASK
        || snapshot.required_attach_mask != SCHEDULER_ATTACH_MASK
        || !snapshot.attach_ready()
    {
        return Err(invalid_data("scheduler tracepoint capability was incomplete").into());
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    if agent.health().state() != AgentState::Healthy {
        return Err(invalid_data("scheduler observer did not become healthy").into());
    }
    let initial_observation = agent.heartbeat(1);
    if initial_observation.state as u8 != OBSERVATION_HEALTHY
        || initial_observation.attach_mask != SCHEDULER_ATTACH_MASK
        || initial_observation.lost_event_count != 0
        || initial_observation.incident_count != 0
        || initial_observation.fault_code != 0
        || initial_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("initial scheduler observation was inconsistent").into());
    }

    let cycle = CycleContext {
        boot_id: BOOT_ID,
        cycle_seq: CYCLE_SEQ,
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

    let baseline_statistics = runtime.statistics()?;
    if baseline_statistics.emitted_events != 0
        || baseline_statistics.lost_events != 0
        || baseline_statistics.wakeups != 0
        || baseline_statistics.scheduler_stalls != 0
        || baseline_statistics.scheduler_migrations != 0
    {
        return Err(invalid_data("scheduler runqueue baseline was not empty").into());
    }

    let mut blocker = FifoBlocker::spawn(cpu_a, fifo_priority)?;
    let futex_wake_count = target.wake_one()?;
    if futex_wake_count != 1 {
        return Err(invalid_data(format!(
            "expected one futex waiter, woke {futex_wake_count}"
        ))
        .into());
    }

    let hold_started = Instant::now();
    while hold_started.elapsed().as_nanos() < u128::from(BLOCKER_HOLD_NS) {
        if target.completed() {
            return Err(invalid_data("target executed while FIFO blocker was active").into());
        }
        thread::sleep(Duration::from_millis(1));
    }
    let target_completed_during_hold = u32::from(target.completed());
    if target_completed_during_hold != 0 {
        return Err(invalid_data("target completed during blocker hold").into());
    }
    blocker.stop_and_join()?;
    target.wait_and_join()?;

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
            "controlled scheduler wakeup did not produce exactly one record and incident",
        )
        .into());
    }
    if agent.correlator().len() != 1 || agent.correlator().dropped_incidents() != 0 {
        return Err(invalid_data("scheduler incident retention was inconsistent").into());
    }
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("scheduler incident was missing"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra scheduler incident").into());
    }

    let statistics = runtime.statistics()?;
    if poll_total.malformed_records != 0
        || poll_total.evidence_rejected != 0
        || poll_total.newly_reported_lost_events != 0
        || statistics.emitted_events != 1
        || statistics.lost_events != 0
        || statistics.wakeups != 1
        || statistics.scheduler_stalls != 1
        || statistics.scheduler_migrations != 0
    {
        return Err(invalid_data("scheduler poll or statistics were inconsistent").into());
    }
    if incident.incident_id == 0
        || incident.boot_id != BOOT_ID
        || incident.agent_epoch != AGENT_EPOCH
        || incident.code != IncidentCode::HostSchedulerStall
        || incident.severity != IncidentSeverity::Error
        || incident.recommended_action != RecommendedAction::ControlledStop
        || incident.confidence_percent != 70
        || incident.pid != 0
        || incident.tid != target.tid()
        || incident.cpu != cpu_a
        || incident.irq != 0
        || incident.netdev_ifindex != 0
        || incident.cycle_first != CYCLE_SEQ
        || incident.cycle_last != CYCLE_SEQ
        || incident.transition_seq != TRANSITION_SEQ
        || incident.observed_value <= SCHEDULER_LATENCY_THRESHOLD_NS
        || incident.observed_value >= MAX_QUALIFIED_LATENCY_NS
        || incident.threshold != SCHEDULER_LATENCY_THRESHOLD_NS
        || incident.evidence_count != 1
        || incident.count != 1
        || incident.lost_events != 0
    {
        return Err(invalid_data("scheduler incident fields were inconsistent").into());
    }

    let evidence = incident.evidence[0];
    if evidence.evidence_id == 0
        || evidence.boot_id != BOOT_ID
        || evidence.agent_epoch != AGENT_EPOCH
        || evidence.timestamp_ns == 0
        || evidence.evidence_id >= evidence.timestamp_ns
        || evidence.pid != 0
        || evidence.tid != target.tid()
        || evidence.cpu != cpu_a
        || evidence.irq != 0
        || evidence.netdev_ifindex != 0
        || evidence.cycle_seq != CYCLE_SEQ
        || evidence.transition_seq != TRANSITION_SEQ
        || evidence.observed_value != evidence.duration_ns
        || evidence.threshold != SCHEDULER_LATENCY_THRESHOLD_NS
        || evidence.duration_ns < BLOCKER_HOLD_NS
        || evidence.duration_ns >= MAX_QUALIFIED_LATENCY_NS
        || evidence.count != 1
        || evidence.domain != EvidenceDomain::KernelScheduler
        || evidence.kind != EvidenceKind::SchedulerRunqueueLatency
        || evidence.severity != IncidentSeverity::Error
        || evidence.detail != 0
    {
        return Err(invalid_data("scheduler evidence fields were inconsistent").into());
    }
    if incident.observed_value != evidence.observed_value
        || incident.first_seen_ns != evidence.timestamp_ns
        || incident.last_seen_ns != evidence.timestamp_ns
    {
        return Err(invalid_data("scheduler incident timing was inconsistent").into());
    }
    if agent.health().state() != AgentState::Degraded {
        return Err(invalid_data("scheduler incident did not degrade observer health").into());
    }
    let observation = agent.heartbeat(evidence.timestamp_ns.saturating_add(1));
    if observation.boot_id != BOOT_ID
        || observation.agent_epoch != AGENT_EPOCH
        || observation.state as u8 != OBSERVATION_DEGRADED
        || observation.attach_mask != SCHEDULER_ATTACH_MASK
        || observation.lost_event_count != 0
        || observation.incident_count != 1
        || observation.fault_code != DEGRADED_INCIDENT_FAULT
        || observation.heartbeat_seq != 2
    {
        return Err(invalid_data("scheduler observation fields were inconsistent").into());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let report = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"status\": \"qualified\",\n",
            "  \"target_tid\": {target_tid},\n",
            "  \"cpu_a\": {cpu_a},\n",
            "  \"cpu_b\": {cpu_b},\n",
            "  \"target_sleep_confirmed\": {target_sleep_confirmed},\n",
            "  \"fifo_policy\": {fifo_policy},\n",
            "  \"fifo_priority_min\": {fifo_priority_min},\n",
            "  \"fifo_priority_max\": {fifo_priority_max},\n",
            "  \"fifo_priority\": {fifo_priority},\n",
            "  \"scheduler_latency_threshold_ns\": {latency_threshold},\n",
            "  \"blocker_hold_ns\": {blocker_hold_ns},\n",
            "  \"futex_wake_count\": {futex_wake_count},\n",
            "  \"target_completed_during_hold\": {target_completed_during_hold},\n",
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
            "  \"baseline_wakeups\": {baseline_wakeups},\n",
            "  \"baseline_scheduler_stalls\": {baseline_stalls},\n",
            "  \"baseline_scheduler_migrations\": {baseline_migrations},\n",
            "  \"records_seen\": {records_seen},\n",
            "  \"incidents_emitted\": {incidents_emitted},\n",
            "  \"malformed_records\": {malformed_records},\n",
            "  \"evidence_rejected\": {evidence_rejected},\n",
            "  \"newly_reported_lost_events\": {new_lost},\n",
            "  \"emitted_events\": {emitted_events},\n",
            "  \"lost_events\": {lost_events},\n",
            "  \"wakeups\": {wakeups},\n",
            "  \"scheduler_stalls\": {scheduler_stalls},\n",
            "  \"scheduler_migrations\": {scheduler_migrations},\n",
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
        target_tid = target.tid(),
        cpu_a = cpu_a,
        cpu_b = cpu_b,
        target_sleep_confirmed = target_sleep_confirmed,
        fifo_policy = libc::SCHED_FIFO,
        fifo_priority_min = fifo_priority_min,
        fifo_priority_max = fifo_priority_max,
        fifo_priority = fifo_priority,
        latency_threshold = SCHEDULER_LATENCY_THRESHOLD_NS,
        blocker_hold_ns = BLOCKER_HOLD_NS,
        futex_wake_count = futex_wake_count,
        target_completed_during_hold = target_completed_during_hold,
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
        baseline_wakeups = baseline_statistics.wakeups,
        baseline_stalls = baseline_statistics.scheduler_stalls,
        baseline_migrations = baseline_statistics.scheduler_migrations,
        records_seen = poll_total.records_seen,
        incidents_emitted = poll_total.incidents_emitted,
        malformed_records = poll_total.malformed_records,
        evidence_rejected = poll_total.evidence_rejected,
        new_lost = poll_total.newly_reported_lost_events,
        emitted_events = statistics.emitted_events,
        lost_events = statistics.lost_events,
        wakeups = statistics.wakeups,
        scheduler_stalls = statistics.scheduler_stalls,
        scheduler_migrations = statistics.scheduler_migrations,
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
        "scheduler runqueue runtime qualification passed: {}",
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
                "usage: scheduler_runqueue_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
            invalid_data(
                "usage: scheduler_runqueue_qualification <esop_runtime.bpf.o> <qualification.json>",
            )
        })?;
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected extra qualification argument").into());
        }
        run(object_path, output_path)
    })();
    if let Err(error) = result {
        eprintln!("scheduler runqueue runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
