#![cfg(target_os = "linux")]

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    AgentState, EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity, RecommendedAction,
    RuntimeAgent,
};
use esop_ebpf_runtime::{ATTACH_OOM_KILL, BpfRuntime, PollReport, RuntimeConfig};

const BOOT_ID: u64 = 0x4553_4f50_4f4f_4d51;
const AGENT_EPOCH: u64 = 1;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const MEMORY_LIMIT_BYTES: u64 = 32 * 1024 * 1024;
const MAPPING_BYTES: usize = 128 * 1024 * 1024;
const POLL_LIMIT: usize = 400;
const POLL_SLEEP: Duration = Duration::from_millis(5);
const POLL_DEADLINE: Duration = Duration::from_secs(4);
const CHILD_DEADLINE: Duration = Duration::from_secs(6);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(2);
const CHILD_WARMUP: u8 = 0x57;
const CHILD_READY: u8 = 0x52;
const CHILD_RELEASE: u8 = 0x4f;
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

fn token_present(contents: &str, expected: &str) -> bool {
    contents
        .split_ascii_whitespace()
        .any(|item| item == expected)
}

fn read_u64(path: &Path) -> io::Result<u64> {
    fs::read_to_string(path)?
        .trim()
        .parse::<u64>()
        .map_err(|error| invalid_data(format!("{} was not a u64: {error}", path.display())))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MemoryEvents {
    max: u64,
    oom: u64,
    oom_kill: u64,
    oom_group_kill: u64,
}

impl MemoryEvents {
    fn read(path: &Path) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        let mut events = Self::default();
        let mut found_max = false;
        let mut found_oom = false;
        let mut found_oom_kill = false;
        let mut found_oom_group_kill = false;
        for line in contents.lines() {
            let mut fields = line.split_ascii_whitespace();
            let Some(key) = fields.next() else {
                continue;
            };
            let Some(raw_value) = fields.next() else {
                return Err(invalid_data(
                    "memory.events.local contained a key without a value",
                ));
            };
            if fields.next().is_some() {
                return Err(invalid_data("memory.events.local contained an invalid row"));
            }
            let value = raw_value.parse::<u64>().map_err(|error| {
                invalid_data(format!("memory.events.local {key} was invalid: {error}"))
            })?;
            match key {
                "max" => {
                    events.max = value;
                    found_max = true;
                }
                "oom" => {
                    events.oom = value;
                    found_oom = true;
                }
                "oom_kill" => {
                    events.oom_kill = value;
                    found_oom_kill = true;
                }
                "oom_group_kill" => {
                    events.oom_group_kill = value;
                    found_oom_group_kill = true;
                }
                _ => {}
            }
        }
        if !found_max || !found_oom || !found_oom_kill || !found_oom_group_kill {
            return Err(invalid_data(
                "memory.events.local did not expose all required OOM counters",
            ));
        }
        Ok(events)
    }

    fn checked_delta(self, baseline: Self) -> io::Result<Self> {
        Ok(Self {
            max: self
                .max
                .checked_sub(baseline.max)
                .ok_or_else(|| invalid_data("memory max event counter regressed"))?,
            oom: self
                .oom
                .checked_sub(baseline.oom)
                .ok_or_else(|| invalid_data("memory OOM event counter regressed"))?,
            oom_kill: self
                .oom_kill
                .checked_sub(baseline.oom_kill)
                .ok_or_else(|| invalid_data("memory OOM-kill counter regressed"))?,
            oom_group_kill: self
                .oom_group_kill
                .checked_sub(baseline.oom_group_kill)
                .ok_or_else(|| invalid_data("memory group-OOM counter regressed"))?,
        })
    }
}

struct QualificationCgroup {
    path: PathBuf,
    swap_limit_present: bool,
    cleaned: bool,
}

impl QualificationCgroup {
    fn create(parent_pid: u32) -> io::Result<Self> {
        let root = Path::new(CGROUP_ROOT);
        if !root.join("cgroup.controllers").is_file() {
            return Err(invalid_data("cgroup v2 root was unavailable"));
        }
        let controllers = fs::read_to_string(root.join("cgroup.controllers"))?;
        let subtree = fs::read_to_string(root.join("cgroup.subtree_control"))?;
        if !token_present(&controllers, "memory") || !token_present(&subtree, "memory") {
            return Err(invalid_data(
                "cgroup v2 memory controller was not available and enabled at the root",
            ));
        }

        let path = root.join(format!("esop-oom-qualification-{parent_pid}"));
        if path.exists() {
            return Err(invalid_data(format!(
                "qualification cgroup already existed: {}",
                path.display()
            )));
        }
        fs::create_dir(&path)?;
        let mut cgroup = Self {
            path,
            swap_limit_present: false,
            cleaned: false,
        };
        if let Err(error) = cgroup.configure() {
            let _ = cgroup.force_cleanup();
            return Err(error);
        }
        Ok(cgroup)
    }

    fn configure(&mut self) -> io::Result<()> {
        fs::write(self.path.join("memory.max"), MEMORY_LIMIT_BYTES.to_string())?;
        fs::write(self.path.join("memory.oom.group"), "0")?;
        let swap_path = self.path.join("memory.swap.max");
        if swap_path.exists() {
            fs::write(&swap_path, "0")?;
            self.swap_limit_present = true;
        }
        if read_u64(&self.path.join("memory.max"))? != MEMORY_LIMIT_BYTES {
            return Err(invalid_data(
                "memory.max did not retain the qualification limit",
            ));
        }
        if read_u64(&self.path.join("memory.oom.group"))? != 0 {
            return Err(invalid_data("memory.oom.group was not disabled"));
        }
        if self.swap_limit_present && read_u64(&swap_path)? != 0 {
            return Err(invalid_data("memory.swap.max was not disabled"));
        }
        Ok(())
    }

    fn add_pid(&self, pid: u32) -> io::Result<()> {
        fs::write(self.path.join("cgroup.procs"), format!("{pid}\n"))
    }

    fn members(&self) -> io::Result<Vec<u32>> {
        let contents = fs::read_to_string(self.path.join("cgroup.procs"))?;
        contents
            .split_ascii_whitespace()
            .map(|raw| {
                raw.parse::<u32>().map_err(|error| {
                    invalid_data(format!("cgroup.procs contained an invalid PID: {error}"))
                })
            })
            .collect()
    }

    fn memory_events(&self) -> io::Result<MemoryEvents> {
        MemoryEvents::read(&self.path.join("memory.events.local"))
    }

    fn populated(&self) -> io::Result<bool> {
        let contents = fs::read_to_string(self.path.join("cgroup.events"))?;
        for line in contents.lines() {
            let mut fields = line.split_ascii_whitespace();
            if fields.next() == Some("populated") {
                return match fields.next() {
                    Some("0") => Ok(false),
                    Some("1") => Ok(true),
                    _ => Err(invalid_data("cgroup.events populated value was invalid")),
                };
            }
        }
        Err(invalid_data("cgroup.events did not contain populated"))
    }

    fn wait_until_empty(&self, deadline: Duration) -> io::Result<()> {
        let started = Instant::now();
        loop {
            if self.members()?.is_empty() && !self.populated()? {
                return Ok(());
            }
            if started.elapsed() >= deadline {
                return Err(invalid_data("qualification cgroup did not become empty"));
            }
            thread::sleep(POLL_SLEEP);
        }
    }

    fn cleanup(&mut self) -> io::Result<()> {
        self.force_cleanup()?;
        self.cleaned = true;
        Ok(())
    }

    fn force_cleanup(&mut self) -> io::Result<()> {
        if !self.path.exists() {
            self.cleaned = true;
            return Ok(());
        }
        if self.populated().unwrap_or(true) {
            let kill_path = self.path.join("cgroup.kill");
            if kill_path.exists() {
                let _ = fs::write(kill_path, "1");
            }
        }
        self.wait_until_empty(CLEANUP_DEADLINE)?;
        fs::remove_dir(&self.path)?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for QualificationCgroup {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.force_cleanup();
        }
    }
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
            .arg("--oom-child")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let Some(mut input) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid_data("OOM child stdin was unavailable").into());
        };
        let Some(output) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid_data("OOM child stdout was unavailable").into());
        };
        if let Err(error) = set_nonblocking(output.as_raw_fd()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        if let Err(error) = input
            .write_all(&[CHILD_WARMUP])
            .and_then(|()| input.flush())
        {
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

    fn wait_ready(&mut self) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        let mut ready = [0u8; 1];
        loop {
            match self.output.read(&mut ready) {
                Ok(1) if ready[0] == CHILD_READY => return Ok(()),
                Ok(1) => return Err(invalid_data("OOM child sent an invalid ready byte").into()),
                Ok(0) => return Err(invalid_data("OOM child closed before readiness").into()),
                Ok(_) => unreachable!("one-byte readiness buffer"),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            if let Some(status) = self.child.try_wait()? {
                self.finished = true;
                return Err(
                    invalid_data(format!("OOM child exited before readiness: {status}")).into(),
                );
            }
            if started.elapsed() >= CHILD_DEADLINE {
                return Err(invalid_data("OOM child readiness timed out").into());
            }
            thread::sleep(POLL_SLEEP);
        }
    }

    fn release(&mut self) -> Result<(), Box<dyn Error>> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| invalid_data("OOM child command pipe was closed"))?;
        input.write_all(&[CHILD_RELEASE])?;
        input.flush()?;
        self.input.take();
        Ok(())
    }

    fn wait_for_sigkill(&mut self) -> Result<i32, Box<dyn Error>> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.finished = true;
                let signal = status.signal().ok_or_else(|| {
                    invalid_data(format!("OOM child exited without a signal: {status}"))
                })?;
                if signal != libc::SIGKILL {
                    return Err(invalid_data(format!(
                        "OOM child exited with signal {signal}, expected SIGKILL"
                    ))
                    .into());
                }
                return Ok(signal);
            }
            if started.elapsed() >= CHILD_DEADLINE {
                return Err(invalid_data("OOM child termination timed out").into());
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

fn anonymous_mapping(bytes: usize) -> io::Result<*mut u8> {
    let mapping = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            bytes,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
            -1,
            0,
        )
    };
    if mapping == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(mapping.cast::<u8>())
}

fn run_oom_child() -> Result<(), Box<dyn Error>> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(io::Error::last_os_error().into());
    }
    let page_size = usize::try_from(page_size)?;

    let warm = anonymous_mapping(page_size)?;
    unsafe {
        std::ptr::write_volatile(warm, 0xa5);
        if libc::munmap(warm.cast(), page_size) != 0 {
            return Err(io::Error::last_os_error().into());
        }
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let mut command = [0u8; 1];
    input.read_exact(&mut command)?;
    if command[0] != CHILD_WARMUP {
        return Err(invalid_data("OOM child received an invalid warm-up byte").into());
    }
    output.write_all(&[CHILD_READY])?;
    output.flush()?;
    input.read_exact(&mut command)?;
    if command[0] != CHILD_RELEASE {
        return Err(invalid_data("OOM child received an invalid release byte").into());
    }

    let mapping = anonymous_mapping(MAPPING_BYTES)?;
    for offset in (0..MAPPING_BYTES).step_by(page_size) {
        unsafe { std::ptr::write_volatile(mapping.add(offset), 0x5a) };
    }
    unsafe {
        let _ = libc::munmap(mapping.cast(), MAPPING_BYTES);
    }
    Err(invalid_data("OOM child survived the bounded memory-pressure injection").into())
}

fn run_parent(object_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let parent_pid = std::process::id();
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(io::Error::last_os_error().into());
    }
    let page_size = u64::try_from(page_size)?;
    if MAPPING_BYTES as u64 <= MEMORY_LIMIT_BYTES || MAPPING_BYTES as u64 % page_size != 0 {
        return Err(invalid_data("OOM mapping geometry was invalid").into());
    }

    let mut cgroup = QualificationCgroup::create(parent_pid)?;
    let executable = fs::canonicalize(std::env::current_exe()?)?;
    let mut child = TrackedChild::spawn(&executable)?;
    child.wait_ready()?;
    let child_pid = child.pid();
    if child_pid == 0 || child_pid == parent_pid {
        return Err(invalid_data("tracked OOM child PID was invalid").into());
    }
    cgroup.add_pid(child_pid)?;
    let initial_members = cgroup.members()?;
    if initial_members != [child_pid] {
        return Err(invalid_data("qualification cgroup did not contain exactly the child").into());
    }
    let baseline_memory = cgroup.memory_events()?;
    if baseline_memory != MemoryEvents::default() {
        return Err(invalid_data("new qualification cgroup had nonzero OOM counters").into());
    }

    let config = RuntimeConfig {
        enabled_attach_mask: ATTACH_OOM_KILL,
        required_attach_mask: ATTACH_OOM_KILL,
        tracked_pid: child_pid,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };
    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_OOM_KILL
        || snapshot.attach_mask != ATTACH_OOM_KILL
        || snapshot.required_attach_mask != ATTACH_OOM_KILL
        || !snapshot.attach_ready()
    {
        return Err(invalid_data("OOM tracepoint capability was not complete").into());
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    if agent.health().state() != AgentState::Healthy {
        return Err(invalid_data("OOM observer did not become healthy").into());
    }
    let initial_observation = agent.heartbeat(1);
    if initial_observation.boot_id != BOOT_ID
        || initial_observation.agent_epoch != AGENT_EPOCH
        || initial_observation.state as u8 != OBSERVATION_HEALTHY
        || initial_observation.attach_mask != ATTACH_OOM_KILL
        || initial_observation.lost_event_count != 0
        || initial_observation.incident_count != 0
        || initial_observation.fault_code != 0
        || initial_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("initial OOM observation fields were inconsistent").into());
    }
    let baseline_poll = runtime.poll(&mut agent, 64)?;
    let baseline_statistics = runtime.statistics()?;
    if baseline_poll != PollReport::default()
        || baseline_statistics.emitted_events != 0
        || baseline_statistics.lost_events != 0
        || baseline_statistics.process_exits != 0
        || baseline_statistics.oom_events != 0
        || !agent.correlator().is_empty()
    {
        return Err(invalid_data("OOM baseline was not quiet").into());
    }

    child.release()?;
    let child_signal = child.wait_for_sigkill()?;
    cgroup.wait_until_empty(CLEANUP_DEADLINE)?;
    let final_memory = cgroup.memory_events()?;
    let memory_delta = final_memory.checked_delta(baseline_memory)?;
    if memory_delta.max == 0
        || memory_delta.oom == 0
        || memory_delta.oom_kill != 1
        || memory_delta.oom_group_kill != 0
    {
        return Err(invalid_data("memcg OOM counters were inconsistent").into());
    }

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
        return Err(invalid_data("OOM victim did not produce exactly one incident").into());
    }
    if agent.correlator().len() != 1 || agent.correlator().dropped_incidents() != 0 {
        return Err(invalid_data("OOM incident retention was inconsistent").into());
    }
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("OOM incident was missing"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra OOM incident was retained").into());
    }
    let statistics = runtime.statistics()?;
    if poll_total.malformed_records != 0
        || poll_total.evidence_rejected != 0
        || poll_total.newly_reported_lost_events != 0
        || statistics.emitted_events != 1
        || statistics.lost_events != 0
        || statistics.process_exits != 0
        || statistics.oom_events != 1
    {
        return Err(invalid_data("OOM poll or kernel statistics were inconsistent").into());
    }
    if incident.incident_id == 0
        || incident.boot_id != BOOT_ID
        || incident.agent_epoch != AGENT_EPOCH
        || incident.code != IncidentCode::HostOom
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
        return Err(invalid_data("OOM incident fields were inconsistent").into());
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
        || evidence.domain != EvidenceDomain::KernelMemory
        || evidence.kind != EvidenceKind::OomKill
        || evidence.severity != IncidentSeverity::Critical
        || evidence.detail != 0
    {
        return Err(invalid_data("OOM evidence fields were inconsistent").into());
    }
    if incident.first_seen_ns != evidence.timestamp_ns
        || incident.last_seen_ns != evidence.timestamp_ns
    {
        return Err(invalid_data("OOM incident timestamps were inconsistent").into());
    }
    if agent.health().state() != AgentState::Failed {
        return Err(invalid_data("OOM incident did not fail the observer lease").into());
    }
    let observation = agent.heartbeat(evidence.timestamp_ns.saturating_add(1));
    if observation.boot_id != BOOT_ID
        || observation.agent_epoch != AGENT_EPOCH
        || observation.state as u8 != OBSERVATION_FAILED
        || observation.attach_mask != ATTACH_OOM_KILL
        || observation.lost_event_count != 0
        || observation.incident_count != 1
        || observation.fault_code != LATCHED_INCIDENT_FAULT
        || observation.heartbeat_seq != 2
    {
        return Err(invalid_data("final OOM observation fields were inconsistent").into());
    }

    let final_member_count = cgroup.members()?.len();
    if final_member_count != 0 {
        return Err(invalid_data("qualification cgroup retained a member").into());
    }
    let swap_limit_present = u8::from(cgroup.swap_limit_present);
    drop(runtime);
    cgroup.cleanup()?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let report = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"status\": \"qualified\",\n",
            "  \"cgroup_version\": 2,\n",
            "  \"memory_controller_enabled\": 1,\n",
            "  \"memory_limit_bytes\": {memory_limit_bytes},\n",
            "  \"mapping_bytes\": {mapping_bytes},\n",
            "  \"page_size\": {page_size},\n",
            "  \"mapping_pages\": {mapping_pages},\n",
            "  \"swap_limit_present\": {swap_limit_present},\n",
            "  \"swap_limit_bytes\": 0,\n",
            "  \"oom_group\": 0,\n",
            "  \"tracked_child_pid\": {tracked_child_pid},\n",
            "  \"initial_member_count\": 1,\n",
            "  \"final_member_count\": {final_member_count},\n",
            "  \"child_signal\": {child_signal},\n",
            "  \"cleanup_succeeded\": 1,\n",
            "  \"baseline_memory_max_events\": {baseline_memory_max},\n",
            "  \"baseline_memory_oom_events\": {baseline_memory_oom},\n",
            "  \"baseline_memory_oom_kill_events\": {baseline_memory_oom_kill},\n",
            "  \"baseline_memory_oom_group_kill_events\": {baseline_memory_group_kill},\n",
            "  \"memory_max_events\": {memory_max},\n",
            "  \"memory_oom_events\": {memory_oom},\n",
            "  \"memory_oom_kill_events\": {memory_oom_kill},\n",
            "  \"memory_oom_group_kill_events\": {memory_group_kill},\n",
            "  \"memory_max_delta\": {memory_max_delta},\n",
            "  \"memory_oom_delta\": {memory_oom_delta},\n",
            "  \"memory_oom_kill_delta\": {memory_oom_kill_delta},\n",
            "  \"memory_oom_group_kill_delta\": {memory_group_kill_delta},\n",
            "  \"runtime_attach_mask\": {runtime_attach_mask},\n",
            "  \"required_attach_mask\": {required_attach_mask},\n",
            "  \"initial_observation_boot_id\": {initial_observation_boot_id},\n",
            "  \"initial_observation_agent_epoch\": {initial_observation_agent_epoch},\n",
            "  \"initial_observation_state\": {initial_observation_state},\n",
            "  \"initial_observation_attach_mask\": {initial_observation_attach_mask},\n",
            "  \"initial_observation_lost_event_count\": {initial_observation_lost},\n",
            "  \"initial_observation_incident_count\": {initial_observation_incidents},\n",
            "  \"initial_observation_fault_code\": {initial_observation_fault},\n",
            "  \"initial_observation_heartbeat_seq\": {initial_observation_heartbeat_seq},\n",
            "  \"baseline_emitted_events\": {baseline_emitted_events},\n",
            "  \"baseline_lost_events\": {baseline_lost_events},\n",
            "  \"baseline_process_exits\": {baseline_process_exits},\n",
            "  \"baseline_oom_events\": {baseline_oom_events},\n",
            "  \"records_seen\": {records_seen},\n",
            "  \"incidents_emitted\": {incidents_emitted},\n",
            "  \"malformed_records\": {malformed_records},\n",
            "  \"evidence_rejected\": {evidence_rejected},\n",
            "  \"newly_reported_lost_events\": {new_lost},\n",
            "  \"emitted_events\": {emitted_events},\n",
            "  \"lost_events\": {lost_events},\n",
            "  \"process_exits\": {process_exits},\n",
            "  \"oom_events\": {oom_events},\n",
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
        memory_limit_bytes = MEMORY_LIMIT_BYTES,
        mapping_bytes = MAPPING_BYTES,
        page_size = page_size,
        mapping_pages = MAPPING_BYTES as u64 / page_size,
        swap_limit_present = swap_limit_present,
        tracked_child_pid = child_pid,
        final_member_count = final_member_count,
        child_signal = child_signal,
        baseline_memory_max = baseline_memory.max,
        baseline_memory_oom = baseline_memory.oom,
        baseline_memory_oom_kill = baseline_memory.oom_kill,
        baseline_memory_group_kill = baseline_memory.oom_group_kill,
        memory_max = final_memory.max,
        memory_oom = final_memory.oom,
        memory_oom_kill = final_memory.oom_kill,
        memory_group_kill = final_memory.oom_group_kill,
        memory_max_delta = memory_delta.max,
        memory_oom_delta = memory_delta.oom,
        memory_oom_kill_delta = memory_delta.oom_kill,
        memory_group_kill_delta = memory_delta.oom_group_kill,
        runtime_attach_mask = ATTACH_OOM_KILL,
        required_attach_mask = snapshot.required_attach_mask,
        initial_observation_boot_id = initial_observation.boot_id,
        initial_observation_agent_epoch = initial_observation.agent_epoch,
        initial_observation_state = initial_observation.state as u8,
        initial_observation_attach_mask = initial_observation.attach_mask,
        initial_observation_lost = initial_observation.lost_event_count,
        initial_observation_incidents = initial_observation.incident_count,
        initial_observation_fault = initial_observation.fault_code,
        initial_observation_heartbeat_seq = initial_observation.heartbeat_seq,
        baseline_emitted_events = baseline_statistics.emitted_events,
        baseline_lost_events = baseline_statistics.lost_events,
        baseline_process_exits = baseline_statistics.process_exits,
        baseline_oom_events = baseline_statistics.oom_events,
        records_seen = poll_total.records_seen,
        incidents_emitted = poll_total.incidents_emitted,
        malformed_records = poll_total.malformed_records,
        evidence_rejected = poll_total.evidence_rejected,
        new_lost = poll_total.newly_reported_lost_events,
        emitted_events = statistics.emitted_events,
        lost_events = statistics.lost_events,
        process_exits = statistics.process_exits,
        oom_events = statistics.oom_events,
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
        "OOM runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let first = arguments.next().ok_or_else(|| {
        invalid_data("usage: oom_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    if first == OsStr::new("--oom-child") {
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected OOM child argument").into());
        }
        return run_oom_child();
    }
    let object_path = PathBuf::from(first);
    let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        invalid_data("usage: oom_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    if arguments.next().is_some() {
        return Err(invalid_data("unexpected extra qualification argument").into());
    }
    run_parent(object_path, output_path)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("OOM runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
