#![cfg(target_os = "linux")]

use std::error::Error;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::mem::{size_of, zeroed};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};

use esop_ebpf_agent::{
    AgentState, CycleContext, EvidenceDomain, EvidenceKind, IncidentCode, IncidentSeverity,
    RecommendedAction, RuntimeAgent,
};
use esop_ebpf_runtime::{ATTACH_PAGE_FAULT, BpfRuntime, PollReport, RuntimeConfig};

const BOOT_ID: u64 = 0x4553_4f50_5046_4155;
const AGENT_EPOCH: u64 = 1;
const CYCLE_SEQ: u64 = 42;
const TRANSITION_SEQ: u64 = 9;
const INCIDENT_WINDOW_NS: u64 = 1_000_000_000;
const PAGE_FAULT_THRESHOLD: u64 = 16;
const PAGE_FAULT_WINDOW_NS: u64 = 1_000_000_000;
const PAGE_COUNT: usize = PAGE_FAULT_THRESHOLD as usize;
const POLL_SLEEP: Duration = Duration::from_millis(2);
const POLL_DEADLINE: Duration = Duration::from_secs(2);
const CHILD_DEADLINE: Duration = Duration::from_secs(2);
const READY_BYTES: usize = 24;
const RESULT_BYTES: usize = 32;
const READY_MAGIC: [u8; 4] = *b"PFRD";
const RESULT_MAGIC: [u8; 4] = *b"PFRS";
const INJECT_COMMAND: u8 = 0x49;
const FINISH_COMMAND: u8 = 0x46;
const X86_USER_WRITE_NOT_PRESENT: u8 = 6;
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
        return Err(invalid_data("no allowed CPU fits the evidence ABI"));
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
        return Err(invalid_data("child did not execute on the selected CPU"));
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
            "gettid result was outside the supported range",
        ));
    }
    Ok(tid as u32)
}

fn system_page_size() -> io::Result<usize> {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size <= 0 {
        return Err(io::Error::last_os_error());
    }
    usize::try_from(size).map_err(|_| invalid_data("system page size did not fit usize"))
}

fn minor_faults() -> io::Result<u64> {
    let mut usage: libc::rusage = unsafe { zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return Err(io::Error::last_os_error());
    }
    u64::try_from(usage.ru_minflt)
        .map_err(|_| invalid_data("getrusage returned a negative minor-fault count"))
}

struct IsolatedPages {
    bases: [*mut u8; PAGE_COUNT],
    mapping_len: usize,
}

impl IsolatedPages {
    fn allocate(page_size: usize) -> io::Result<Self> {
        let mapping_len = page_size
            .checked_mul(2)
            .ok_or_else(|| invalid_data("guarded page mapping size overflowed"))?;
        let mut pages = Self {
            bases: [ptr::null_mut(); PAGE_COUNT],
            mapping_len,
        };
        for base in &mut pages.bases {
            let mapping = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    mapping_len,
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
            *base = mapping;
            let guard = unsafe { mapping.add(page_size) };
            if unsafe { libc::mprotect(guard.cast(), page_size, libc::PROT_NONE) } != 0 {
                return Err(io::Error::last_os_error());
            }
            if unsafe { libc::madvise(mapping.cast(), page_size, libc::MADV_NOHUGEPAGE) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(pages)
    }

    fn touch(&mut self) {
        for (index, base) in self.bases.iter_mut().enumerate() {
            unsafe { ptr::write_volatile(*base, (index as u8).wrapping_add(1)) };
        }
    }
}

impl Drop for IsolatedPages {
    fn drop(&mut self) {
        for base in self.bases {
            if !base.is_null() {
                let _ = unsafe { libc::munmap(base.cast(), self.mapping_len) };
            }
        }
    }
}

fn raw_read_exact(fd: libc::c_int, buffer: &mut [u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < buffer.len() {
        let result = unsafe {
            libc::read(
                fd,
                buffer[offset..].as_mut_ptr().cast(),
                buffer.len() - offset,
            )
        };
        if result > 0 {
            offset += result as usize;
            continue;
        }
        if result == 0 {
            return Err(invalid_data("control pipe closed unexpectedly"));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

fn raw_write_all(fd: libc::c_int, buffer: &[u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < buffer.len() {
        let result =
            unsafe { libc::write(fd, buffer[offset..].as_ptr().cast(), buffer.len() - offset) };
        if result > 0 {
            offset += result as usize;
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct ChildReady {
    cpu: u16,
    page_count: u16,
    page_size: u64,
    tid: u32,
}

fn encode_ready(ready: ChildReady) -> [u8; READY_BYTES] {
    let mut bytes = [0u8; READY_BYTES];
    bytes[0..4].copy_from_slice(&READY_MAGIC);
    bytes[4..6].copy_from_slice(&ready.cpu.to_le_bytes());
    bytes[6..8].copy_from_slice(&ready.page_count.to_le_bytes());
    bytes[8..16].copy_from_slice(&ready.page_size.to_le_bytes());
    bytes[16..20].copy_from_slice(&ready.tid.to_le_bytes());
    bytes
}

fn decode_ready(bytes: [u8; READY_BYTES]) -> io::Result<ChildReady> {
    if bytes[0..4] != READY_MAGIC {
        return Err(invalid_data("child readiness magic was invalid"));
    }
    Ok(ChildReady {
        cpu: u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
        page_count: u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
        page_size: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        tid: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    })
}

#[derive(Clone, Copy, Debug)]
struct ChildResult {
    cpu: u16,
    page_count: u16,
    minor_faults_before: u64,
    minor_faults_after: u64,
    minor_faults_delta: u64,
}

fn encode_result(result: ChildResult) -> [u8; RESULT_BYTES] {
    let mut bytes = [0u8; RESULT_BYTES];
    bytes[0..4].copy_from_slice(&RESULT_MAGIC);
    bytes[4..6].copy_from_slice(&result.cpu.to_le_bytes());
    bytes[6..8].copy_from_slice(&result.page_count.to_le_bytes());
    bytes[8..16].copy_from_slice(&result.minor_faults_before.to_le_bytes());
    bytes[16..24].copy_from_slice(&result.minor_faults_after.to_le_bytes());
    bytes[24..32].copy_from_slice(&result.minor_faults_delta.to_le_bytes());
    bytes
}

fn decode_result(bytes: [u8; RESULT_BYTES]) -> io::Result<ChildResult> {
    if bytes[0..4] != RESULT_MAGIC {
        return Err(invalid_data("child result magic was invalid"));
    }
    Ok(ChildResult {
        cpu: u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
        page_count: u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
        minor_faults_before: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        minor_faults_after: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        minor_faults_delta: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
    })
}

fn run_tracked_child(cpu: u16) -> Result<(), Box<dyn Error>> {
    set_current_thread_affinity(cpu)?;
    let page_size = system_page_size()?;
    let mut warm_pages = IsolatedPages::allocate(page_size)?;
    let mut formal_pages = IsolatedPages::allocate(page_size)?;
    warm_pages.touch();

    let tid = current_tid()?;
    let _ = minor_faults()?;
    let _ = minor_faults()?;
    let ready = ChildReady {
        cpu: current_cpu()?,
        page_count: PAGE_COUNT as u16,
        page_size: page_size as u64,
        tid,
    };
    let ready_bytes = encode_ready(ready);
    let mut command = [0u8; 1];
    let _ = encode_result(ChildResult {
        cpu,
        page_count: PAGE_COUNT as u16,
        minor_faults_before: 0,
        minor_faults_after: 0,
        minor_faults_delta: 0,
    });
    raw_write_all(libc::STDOUT_FILENO, &ready_bytes)?;
    raw_read_exact(libc::STDIN_FILENO, &mut command)?;
    if command[0] != INJECT_COMMAND {
        return Err(invalid_data("child received an invalid injection command").into());
    }

    let minor_faults_before = minor_faults()?;
    formal_pages.touch();
    let minor_faults_after = minor_faults()?;
    let minor_faults_delta = minor_faults_after
        .checked_sub(minor_faults_before)
        .ok_or_else(|| invalid_data("child minor-fault count moved backwards"))?;
    let result = ChildResult {
        cpu: current_cpu()?,
        page_count: PAGE_COUNT as u16,
        minor_faults_before,
        minor_faults_after,
        minor_faults_delta,
    };
    raw_write_all(libc::STDOUT_FILENO, &encode_result(result))?;

    raw_read_exact(libc::STDIN_FILENO, &mut command)?;
    if command[0] != FINISH_COMMAND {
        return Err(invalid_data("child received an invalid finish command").into());
    }
    Ok(())
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
    fn spawn(executable: &Path, cpu: u16) -> Result<Self, Box<dyn Error>> {
        let mut child = Command::new(executable)
            .arg("--tracked-child")
            .arg(cpu.to_string())
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

    fn read_deadline<const N: usize>(&mut self, label: &str) -> Result<[u8; N], Box<dyn Error>> {
        let started = Instant::now();
        let mut bytes = [0u8; N];
        let mut offset = 0;
        while offset < N {
            match self.output.read(&mut bytes[offset..]) {
                Ok(0) => return Err(invalid_data(format!("child closed before {label}")).into()),
                Ok(read) => offset += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            if offset == N {
                break;
            }
            if let Some(status) = self.child.try_wait()? {
                self.finished = true;
                return Err(
                    invalid_data(format!("tracked child exited before {label}: {status}")).into(),
                );
            }
            if started.elapsed() >= CHILD_DEADLINE {
                return Err(invalid_data(format!("tracked child {label} timed out")).into());
            }
            thread::sleep(POLL_SLEEP);
        }
        Ok(bytes)
    }

    fn wait_ready(&mut self) -> Result<ChildReady, Box<dyn Error>> {
        decode_ready(self.read_deadline("readiness")?).map_err(Into::into)
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

    fn wait_result(&mut self) -> Result<ChildResult, Box<dyn Error>> {
        decode_result(self.read_deadline("result")?).map_err(Into::into)
    }

    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.send(FINISH_COMMAND)?;
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
                return Err(invalid_data("tracked child exit timed out").into());
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

fn run_parent(object_path: PathBuf, output_path: PathBuf) -> Result<(), Box<dyn Error>> {
    if std::env::consts::ARCH != "x86_64" {
        return Err(invalid_data("page-fault qualification requires x86_64").into());
    }
    let temp_path = report_temp_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&temp_path);

    let target_cpu = allowed_cpus()?[0];
    let executable = fs::canonicalize(std::env::current_exe()?)?;
    let mut child = TrackedChild::spawn(&executable, target_cpu)?;
    let child_pid = child.pid();
    if child_pid == 0 || child_pid > i32::MAX as u32 {
        return Err(invalid_data("tracked child PID was invalid").into());
    }
    let ready = child.wait_ready()?;
    if ready.cpu != target_cpu
        || ready.page_count != PAGE_COUNT as u16
        || ready.tid != child_pid
        || ready.page_size < 4096
        || !ready.page_size.is_power_of_two()
    {
        return Err(invalid_data(format!(
            "tracked child readiness was inconsistent: {ready:?}"
        ))
        .into());
    }

    let config = RuntimeConfig {
        enabled_attach_mask: ATTACH_PAGE_FAULT,
        required_attach_mask: ATTACH_PAGE_FAULT,
        tracked_pid: child_pid,
        page_fault_threshold: PAGE_FAULT_THRESHOLD,
        page_fault_window_ns: PAGE_FAULT_WINDOW_NS,
        boot_id: BOOT_ID,
        agent_epoch: AGENT_EPOCH,
        ..RuntimeConfig::default()
    };
    let mut runtime = BpfRuntime::load(&object_path, config)?;
    let snapshot = runtime.capability_snapshot();
    if runtime.attach_mask() != ATTACH_PAGE_FAULT
        || snapshot.attach_mask != ATTACH_PAGE_FAULT
        || snapshot.required_attach_mask != ATTACH_PAGE_FAULT
        || !snapshot.attach_ready()
        || runtime.kernel_context().tracked_pid != child_pid
        || runtime.kernel_context().page_fault_threshold != PAGE_FAULT_THRESHOLD
        || runtime.kernel_context().page_fault_window_ns != PAGE_FAULT_WINDOW_NS
    {
        return Err(invalid_data("page-fault tracepoint capability was incomplete").into());
    }

    let mut agent = RuntimeAgent::<8>::new(BOOT_ID, AGENT_EPOCH, INCIDENT_WINDOW_NS);
    runtime.apply_capability_snapshot(&mut agent);
    if agent.health().state() != AgentState::Healthy {
        return Err(invalid_data("page-fault observer did not become healthy").into());
    }
    let initial_observation = agent.heartbeat(1);
    if initial_observation.state as u8 != OBSERVATION_HEALTHY
        || initial_observation.attach_mask != ATTACH_PAGE_FAULT
        || initial_observation.lost_event_count != 0
        || initial_observation.incident_count != 0
        || initial_observation.fault_code != 0
        || initial_observation.heartbeat_seq != 1
    {
        return Err(invalid_data("initial page-fault observation was inconsistent").into());
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

    let baseline = runtime.statistics()?;
    if baseline.emitted_events != 0
        || baseline.lost_events != 0
        || baseline.page_faults != 0
        || baseline.page_fault_threshold_events != 0
    {
        return Err(
            invalid_data(format!("page-fault baseline was not empty: {baseline:?}")).into(),
        );
    }

    child.send(INJECT_COMMAND)?;
    let child_result = child.wait_result()?;
    if child_result.cpu != target_cpu
        || child_result.page_count != PAGE_COUNT as u16
        || child_result.minor_faults_after < child_result.minor_faults_before
        || child_result.minor_faults_after - child_result.minor_faults_before
            != child_result.minor_faults_delta
        || child_result.minor_faults_delta != PAGE_FAULT_THRESHOLD
    {
        return Err(invalid_data(format!(
            "tracked child fault result was inconsistent: {child_result:?}"
        ))
        .into());
    }

    let mut poll = PollReport::default();
    let poll_started = Instant::now();
    while poll_started.elapsed() < POLL_DEADLINE {
        add_poll_report(&mut poll, runtime.poll(&mut agent, 64)?);
        if poll.records_seen >= 1 && poll.incidents_emitted >= 1 {
            break;
        }
        thread::sleep(POLL_SLEEP);
    }
    if poll.records_seen != 1 || poll.incidents_emitted != 1 {
        return Err(invalid_data(format!(
            "page-fault injection produced records/incidents {}/{}",
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
        return Err(invalid_data("page-fault poll or retention was inconsistent").into());
    }
    let dropped_incidents = agent.correlator().dropped_incidents();
    let incident = agent
        .pop_incident()
        .ok_or_else(|| invalid_data("page-fault incident was missing"))?;
    if agent.pop_incident().is_some() {
        return Err(invalid_data("unexpected extra page-fault incident").into());
    }
    if incident.incident_id == 0
        || incident.boot_id != BOOT_ID
        || incident.agent_epoch != AGENT_EPOCH
        || incident.code != IncidentCode::HostPageFault
        || incident.severity != IncidentSeverity::Warning
        || incident.recommended_action != RecommendedAction::DegradeHostObservation
        || incident.confidence_percent != 65
        || incident.pid != child_pid
        || incident.tid != child_pid
        || incident.cpu != target_cpu
        || incident.irq != 0
        || incident.netdev_ifindex != 0
        || incident.cycle_first != CYCLE_SEQ
        || incident.cycle_last != CYCLE_SEQ
        || incident.transition_seq != TRANSITION_SEQ
        || incident.observed_value != PAGE_FAULT_THRESHOLD
        || incident.threshold != PAGE_FAULT_THRESHOLD
        || incident.evidence_count != 1
        || incident.count != PAGE_COUNT as u32
        || incident.lost_events != 0
    {
        return Err(invalid_data(format!(
            "page-fault incident fields were inconsistent: {incident:?}"
        ))
        .into());
    }
    let evidence = incident.evidence[0];
    if evidence.evidence_id == 0
        || evidence.evidence_id != evidence.timestamp_ns
        || evidence.boot_id != BOOT_ID
        || evidence.agent_epoch != AGENT_EPOCH
        || evidence.pid != child_pid
        || evidence.tid != child_pid
        || evidence.cpu != target_cpu
        || evidence.irq != 0
        || evidence.netdev_ifindex != 0
        || evidence.cycle_seq != CYCLE_SEQ
        || evidence.transition_seq != TRANSITION_SEQ
        || evidence.observed_value != PAGE_FAULT_THRESHOLD
        || evidence.threshold != PAGE_FAULT_THRESHOLD
        || evidence.duration_ns == 0
        || evidence.duration_ns >= PAGE_FAULT_WINDOW_NS
        || evidence.count != PAGE_COUNT as u32
        || evidence.domain != EvidenceDomain::KernelMemory
        || evidence.kind != EvidenceKind::PageFault
        || evidence.severity != IncidentSeverity::Warning
        || evidence.detail != X86_USER_WRITE_NOT_PRESENT
        || incident.first_seen_ns != evidence.timestamp_ns
        || incident.last_seen_ns != evidence.timestamp_ns
    {
        return Err(invalid_data(format!(
            "page-fault evidence fields were inconsistent: {evidence:?}"
        ))
        .into());
    }

    let statistics = runtime.statistics()?;
    if statistics.emitted_events != 1
        || statistics.lost_events != 0
        || statistics.page_faults != PAGE_FAULT_THRESHOLD
        || statistics.page_fault_threshold_events != 1
    {
        return Err(invalid_data(format!(
            "page-fault statistics were inconsistent: {statistics:?}"
        ))
        .into());
    }
    if agent.health().state() != AgentState::Degraded {
        return Err(invalid_data("page-fault incident did not degrade observer health").into());
    }
    let observation = agent.heartbeat(evidence.timestamp_ns.saturating_add(1));
    if observation.boot_id != BOOT_ID
        || observation.agent_epoch != AGENT_EPOCH
        || observation.state as u8 != OBSERVATION_DEGRADED
        || observation.attach_mask != ATTACH_PAGE_FAULT
        || observation.lost_event_count != 0
        || observation.incident_count != 1
        || observation.fault_code != DEGRADED_INCIDENT_FAULT
        || observation.heartbeat_seq != 2
    {
        return Err(invalid_data("final page-fault observation was inconsistent").into());
    }

    let runtime_attach_mask = runtime.attach_mask();
    let required_attach_mask = snapshot.required_attach_mask;
    drop(runtime);
    child.finish()?;

    let fields = vec![
        json_number("schema_version", 1),
        ("status", "\"qualified\"".to_owned()),
        ("architecture", "\"x86_64\"".to_owned()),
        json_number("tracked_child_pid", child_pid),
        json_number("target_cpu", target_cpu),
        json_number("ready_cpu", ready.cpu),
        json_number("result_cpu", child_result.cpu),
        json_number("page_size", ready.page_size),
        json_number("page_count", ready.page_count),
        json_number("guarded_mapping_bytes", ready.page_size * 2),
        json_number("minor_faults_before", child_result.minor_faults_before),
        json_number("minor_faults_after", child_result.minor_faults_after),
        json_number("minor_faults_delta", child_result.minor_faults_delta),
        json_number("page_fault_threshold", PAGE_FAULT_THRESHOLD),
        json_number("page_fault_window_ns", PAGE_FAULT_WINDOW_NS),
        json_number("runtime_attach_mask", runtime_attach_mask),
        json_number("required_attach_mask", required_attach_mask),
        json_number("initial_observation_boot_id", initial_observation.boot_id),
        json_number(
            "initial_observation_agent_epoch",
            initial_observation.agent_epoch,
        ),
        json_number("initial_observation_state", initial_observation.state as u8),
        json_number(
            "initial_observation_attach_mask",
            initial_observation.attach_mask,
        ),
        json_number(
            "initial_observation_lost_event_count",
            initial_observation.lost_event_count,
        ),
        json_number(
            "initial_observation_incident_count",
            initial_observation.incident_count,
        ),
        json_number(
            "initial_observation_fault_code",
            initial_observation.fault_code,
        ),
        json_number(
            "initial_observation_heartbeat_seq",
            initial_observation.heartbeat_seq,
        ),
        json_number("baseline_emitted_events", baseline.emitted_events),
        json_number("baseline_lost_events", baseline.lost_events),
        json_number("baseline_page_faults", baseline.page_faults),
        json_number(
            "baseline_page_fault_threshold_events",
            baseline.page_fault_threshold_events,
        ),
        json_number("records_seen", poll.records_seen),
        json_number("incidents_emitted", poll.incidents_emitted),
        json_number("malformed_records", poll.malformed_records),
        json_number("evidence_rejected", poll.evidence_rejected),
        json_number(
            "newly_reported_lost_events",
            poll.newly_reported_lost_events,
        ),
        json_number("emitted_events", statistics.emitted_events),
        json_number("lost_events", statistics.lost_events),
        json_number("page_faults", statistics.page_faults),
        json_number(
            "page_fault_threshold_events",
            statistics.page_fault_threshold_events,
        ),
        json_number("dropped_incidents", dropped_incidents),
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
        json_number("observation_boot_id", observation.boot_id),
        json_number("observation_agent_epoch", observation.agent_epoch),
        json_number("observation_state", observation.state as u8),
        json_number("observation_attach_mask", observation.attach_mask),
        json_number("observation_lost_event_count", observation.lost_event_count),
        json_number("observation_incident_count", observation.incident_count),
        json_number("observation_fault_code", observation.fault_code),
        json_number("observation_heartbeat_seq", observation.heartbeat_seq),
    ];
    write_report(&output_path, fields)?;
    println!(
        "page-fault runtime qualification passed: {}",
        output_path.display()
    );
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let first = arguments.next().ok_or_else(|| {
        invalid_data("usage: page_fault_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    if first == OsStr::new("--tracked-child") {
        let cpu = arguments
            .next()
            .ok_or_else(|| invalid_data("tracked child CPU was missing"))?
            .to_string_lossy()
            .parse::<u16>()
            .map_err(|_| invalid_data("tracked child CPU was invalid"))?;
        if arguments.next().is_some() {
            return Err(invalid_data("unexpected tracked child argument").into());
        }
        return run_tracked_child(cpu);
    }
    let object_path = PathBuf::from(first);
    let output_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        invalid_data("usage: page_fault_qualification <esop_runtime.bpf.o> <qualification.json>")
    })?;
    if arguments.next().is_some() {
        return Err(invalid_data("unexpected extra qualification argument").into());
    }
    run_parent(object_path, output_path)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("page-fault runtime qualification failed: {error}");
        std::process::exit(1);
    }
}
