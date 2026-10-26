//! Portable independent simulation and exact replay of the Linux master path.
#[path = "support/activation.rs"]
mod activation;
#[path = "support/capture.rs"]
mod capture;
#[path = "support/evidence.rs"]
mod evidence;
#[path = "support/scenario.rs"]
mod scenario;
#[path = "support/virtual_i2c.rs"]
mod virtual_i2c;
#[global_allocator]
static ALLOCATOR: activation::CountingAllocator = activation::CountingAllocator;
use capture::{CaptureBinding, CaptureTrace, RecordingPort, ReplayPort};
use esop_ethercat_core::{EthercatPort, LinkState, RxPoll, wire::MAX_ETHERNET_FRAME_LEN};
use esop_ethercat_sim::{Fault, VirtualBus, fixtures};
use scenario::ScenarioPort;
use std::path::PathBuf;

#[derive(Debug)]
enum BackendError {
    Sim(esop_ethercat_sim::SimError),
    Replay(capture::ReplayError),
    LinkDown,
    #[cfg(target_os = "linux")]
    Raw(std::io::Error),
}
impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sim(e) => write!(f, "{e:?}"),
            Self::Replay(e) => write!(f, "{e:?}"),
            Self::LinkDown => write!(f, "link down"),
            #[cfg(target_os = "linux")]
            Self::Raw(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for BackendError {}

trait SimModel: EthercatPort<Error = esop_ethercat_sim::SimError> {
    fn advance_model(&mut self, now: u64) -> Result<(), esop_ethercat_sim::SimError>;
    fn inject_model(&mut self, fault: Fault) -> Result<(), esop_ethercat_sim::SimError>;
    fn emergency_model(&mut self) -> Result<(), esop_ethercat_sim::SimError>;
    fn trace(&self) -> &CaptureTrace;
    fn reset_observer(&mut self);
}
impl<const SLAVES: usize> SimModel for RecordingPort<VirtualBus<SLAVES>> {
    fn advance_model(&mut self, now: u64) -> Result<(), esop_ethercat_sim::SimError> {
        self.port.advance_to(now)
    }
    fn inject_model(&mut self, fault: Fault) -> Result<(), esop_ethercat_sim::SimError> {
        self.port.inject(fault)
    }
    fn emergency_model(&mut self) -> Result<(), esop_ethercat_sim::SimError> {
        self.port.emergency(0, 0x1000, 1, [1, 2, 3, 4, 5])
    }
    fn trace(&self) -> &CaptureTrace {
        &self.trace
    }
    fn reset_observer(&mut self) {
        self.trace.reset_round_trip();
    }
}
enum Backend {
    Sim(Box<dyn SimModel>),
    Replay(Box<ReplayPort>, bool),
    #[cfg(target_os = "linux")]
    Raw(Box<RecordingPort<esop_ethercat_linux_port::LinuxRawPort>>),
}
impl EthercatPort for Backend {
    type Error = BackendError;
    fn link_state(&self) -> LinkState {
        match self {
            Self::Sim(p) => p.link_state(),
            Self::Replay(_, up) => {
                if *up {
                    LinkState::Up
                } else {
                    LinkState::Down
                }
            }
            #[cfg(target_os = "linux")]
            Self::Raw(p) => p.link_state(),
        }
    }
    fn now_ns(&self) -> u64 {
        match self {
            Self::Sim(p) => p.now_ns(),
            Self::Replay(p, _) => p.now_ns(),
            #[cfg(target_os = "linux")]
            Self::Raw(p) => p.now_ns(),
        }
    }
    fn tx_submit(&mut self, frame: &[u8]) -> Result<(), BackendError> {
        match self {
            Self::Sim(p) => p.tx_submit(frame).map_err(BackendError::Sim),
            Self::Replay(p, up) => {
                if *up {
                    p.tx_submit(frame).map_err(BackendError::Replay)
                } else {
                    Err(BackendError::LinkDown)
                }
            }
            #[cfg(target_os = "linux")]
            Self::Raw(p) => p.tx_submit(frame).map_err(BackendError::Raw),
        }
    }
    fn rx_poll(
        &mut self,
        destination: &mut [u8; MAX_ETHERNET_FRAME_LEN],
    ) -> Result<RxPoll, BackendError> {
        match self {
            Self::Sim(p) => p.rx_poll(destination).map_err(BackendError::Sim),
            Self::Replay(p, up) => {
                if *up {
                    p.rx_poll(destination).map_err(BackendError::Replay)
                } else {
                    Ok(RxPoll::LinkDown)
                }
            }
            #[cfg(target_os = "linux")]
            Self::Raw(p) => p.rx_poll(destination).map_err(BackendError::Raw),
        }
    }
}
impl ScenarioPort for Backend {
    fn advance_to(&mut self, now: u64) -> Result<(), String> {
        match self {
            Self::Sim(p) => p.advance_model(now).map_err(|e| format!("{e:?}")),
            Self::Replay(p, _) => p.advance_to(now).map_err(|e| format!("{e:?}")),
            #[cfg(target_os = "linux")]
            Self::Raw(p) => {
                let current = p.now_ns();
                if current < now {
                    std::thread::sleep(std::time::Duration::from_nanos(now - current));
                }
                Ok(())
            }
        }
    }
    fn fault(&mut self, fault: Fault) -> Result<(), String> {
        match self {
            Self::Sim(p) => p.inject_model(fault).map_err(|e| format!("{e:?}")),
            Self::Replay(_, up) => {
                if let Fault::Link(value) = fault {
                    *up = value;
                }
                Ok(())
            }
            #[cfg(target_os = "linux")]
            Self::Raw(_) => Err("fault injection requires sim or replay".into()),
        }
    }
    fn emergency(&mut self) -> Result<(), String> {
        match self {
            Self::Sim(p) => p.emergency_model().map_err(|e| format!("{e:?}")),
            Self::Replay(_, _) => Ok(()),
            #[cfg(target_os = "linux")]
            Self::Raw(_) => Err("raw emergency injection unavailable".into()),
        }
    }
    fn begin_cycle(&mut self) {
        match self {
            Self::Sim(p) => p.reset_observer(),
            Self::Replay(p, _) => p.reset_round_trip(),
            #[cfg(target_os = "linux")]
            Self::Raw(p) => p.trace.reset_round_trip(),
        }
    }
    fn observed_round_trip_ns(&self) -> Option<u64> {
        match self {
            Self::Sim(p) => p.trace().observed_round_trip_ns(),
            Self::Replay(p, _) => p.observed_round_trip_ns(),
            #[cfg(target_os = "linux")]
            Self::Raw(p) => p.trace.observed_round_trip_ns(),
        }
    }
    fn motion_allowed(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Raw(_) => false,
            _ => true,
        }
    }
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("r0_r2: {error}");
        std::process::exit(1);
    }
}

const USAGE: &str = "Usage: r0_r2 [OPTIONS]
  --backend sim|replay|raw       Default: sim (raw requires Linux and --iface)
  --scenario NAME               full, Q1, Q2, Q4, Q4-baseline, command_expiry,
                                dc_fault, wkc_fault, link_fault, drive_fault,
                                supervisor_restart (default: full)
  --mode csp|csv|cst             Default: csp; Q workloads use their CSP product
  --cycles N                    Default: 10000; at most one hour of virtual time
  --output-dir PATH             Default: build/r0-r2/sim
  --pcap PATH --sidecar PATH     Required for replay
  --iface NAME                  Required for raw; motion remains inhibited
  --capture-limit N             Fixed capture capacity (at most 500000 frames)
  --help, -h                    Show this help";

fn execute() -> Result<(), String> {
    let mut mode = "csp".to_owned();
    let mut backend = "sim".to_owned();
    let mut scenario = "full".to_owned();
    let mut cycles = 10_000u64;
    let mut output = PathBuf::from("build/r0-r2/sim");
    let mut pcap = None;
    let mut sidecar = None;
    let mut iface = None;
    let mut capture_limit = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--help" | "-h") {
            println!("{USAGE}");
            return Ok(());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--mode" => mode = value,
            "--backend" => backend = value,
            "--scenario" => scenario = value,
            "--cycles" => cycles = value.parse().map_err(|_| "invalid cycles")?,
            "--output-dir" => output = PathBuf::from(value),
            "--pcap" => pcap = Some(PathBuf::from(value)),
            "--sidecar" => sidecar = Some(PathBuf::from(value)),
            "--iface" => iface = Some(value),
            "--capture-limit" => {
                capture_limit = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| "invalid capture limit")?,
                )
            }
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if cycles == 0 {
        return Err("cycles must be nonzero".into());
    }
    let scenario = scenario::canonical_scenario(&scenario)?.to_owned();
    let shape = scenario::shape(&scenario, &mode)?;
    if cycles > 3_600_000_000_000 / shape.metadata.base_period_ns {
        return Err("requested virtual run exceeds one hour".into());
    }
    let hash = shape
        .metadata
        .config_sha256
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let binding = CaptureBinding::new(
        hash,
        scenario.clone(),
        shape.metadata.base_period_ns,
        cycles,
    );
    let mut port = match backend.as_str() {
        "sim" => {
            let trace = CaptureTrace::new(
                binding.clone(),
                capture_limit.unwrap_or(
                    (cycles as usize * 12 + shape.topology.slave_count as usize * 4096 + 20_000)
                        .min(500_000),
                ),
            )
            .map_err(|e| format!("{e:?}"))?;
            match scenario.as_str() {
                "Q1" | "q1" => Backend::Sim(Box::new(RecordingPort {
                    port: VirtualBus::<32>::new(core::array::from_fn(|i| {
                        if i == 0 {
                            fixtures::load_drive_with_io(1, 0x41)
                        } else {
                            fixtures::load_drive(i as u32 + 1, 0x41 + i as u16)
                        }
                    }))
                    .map_err(|e| format!("{e:?}"))?,
                    trace,
                })),
                "Q2" | "q2" | "Q4" | "q4" | "Q4-baseline" | "q4-baseline" => {
                    Backend::Sim(Box::new(RecordingPort {
                        port: VirtualBus::<17>::new(core::array::from_fn(|i| {
                            if i < 16 {
                                fixtures::load_drive(i as u32 + 1, 0x41 + i as u16)
                            } else {
                                let mut io = fixtures::io();
                                io.serial = 17;
                                io.requesting_id = 0x51;
                                io
                            }
                        }))
                        .map_err(|e| format!("{e:?}"))?,
                        trace,
                    }))
                }
                _ => Backend::Sim(Box::new(RecordingPort {
                    port: VirtualBus::new(fixtures::extended_dual_axis_io())
                        .map_err(|e| format!("{e:?}"))?,
                    trace,
                })),
            }
        }
        "replay" => Backend::Replay(
            Box::new(ReplayPort::new(
                CaptureTrace::load(
                    &pcap.ok_or("--pcap required")?,
                    &sidecar.ok_or("--sidecar required")?,
                    &binding,
                )
                .map_err(|e| format!("{e:?}"))?,
            )),
            true,
        ),
        "raw" => {
            #[cfg(target_os = "linux")]
            {
                Backend::Raw(Box::new(RecordingPort {
                    port: esop_ethercat_linux_port::LinuxRawPort::open(
                        &iface.ok_or("--iface required")?,
                    )
                    .map_err(|e| format!("{e:?}"))?,
                    trace: CaptureTrace::new(
                        binding.clone(),
                        capture_limit.unwrap_or(
                            (cycles as usize * 12
                                + shape.topology.slave_count as usize * 4096
                                + 20_000)
                                .min(500_000),
                        ),
                    )
                    .map_err(|e| format!("{e:?}"))?,
                }))
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = iface;
                return Err("raw requires Linux".into());
            }
        }
        _ => return Err("backend must be sim, replay or raw".into()),
    };
    let config = shape.config_bytes;
    let topology = shape.topology;
    let topology_scenario = match scenario.as_str() {
        "Q4" | "q4" | "Q4-baseline" | "q4-baseline" => "Q4",
        _ => scenario.as_str(),
    };
    let topology_bytes=serde_json::to_vec_pretty(&serde_json::json!({"schema_version":"esop.software-topology.v1","scenario":topology_scenario,"period_ns":shape.metadata.base_period_ns,"topology":topology})).map_err(|e|e.to_string())?;
    use sha2::{Digest, Sha256};
    let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| e.to_string())?;
    let commit = String::from_utf8(commit.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    let mut collector = evidence::RunEvidenceCollector::new(
        evidence::MeasurementBinding {
            source: match backend.as_str() {
                "sim" => evidence::MeasurementSource::Sim,
                "replay" => evidence::MeasurementSource::PcapReplay,
                _ => evidence::MeasurementSource::LinuxRaw,
            },
            esop_commit: commit,
            config_sha256: hex(&Sha256::digest(config)),
            topology_sha256: hex(&Sha256::digest(&topology_bytes)),
            scenario: scenario.clone(),
            period_ns: shape.metadata.base_period_ns,
            cycles,
            clock_origin_ns: 0,
        },
        evidence::ResourceMeasurements {
            core_arena_bytes: None,
            procbuf_bytes: Some(shape.procbuf_bytes),
            ..Default::default()
        },
    )
    .with_topology(topology)
    .with_sdo_capacity(8, 1024);
    activation::reset();
    let started = std::time::Instant::now();
    let mut result = scenario::run(&mut port, &scenario, cycles, &mode, &mut collector)?;
    let allocations = activation::counts();
    result.software_scenario_passed &= allocations.total() == 0;
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    match &port {
        Backend::Sim(p) => {
            p.trace()
                .write(&output.join("trace.pcap"), &output.join("trace.json"))
                .map_err(|e| format!("{e:?}"))?;
        }
        Backend::Replay(p, _) => p.finish().map_err(|e| format!("{e:?}"))?,
        #[cfg(target_os = "linux")]
        Backend::Raw(p) => {
            p.trace
                .write(&output.join("trace.pcap"), &output.join("trace.json"))
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    match &port {
        Backend::Sim(p) => {
            collector.adopt_wire_summary(p.trace().digest_bytes(), p.trace().total_records());
            collector.record_capture_loss(p.trace().lost_records());
            let _ = p.trace().digest();
        }
        Backend::Replay(p, _) => {
            collector.adopt_wire_summary(p.digest_bytes(), p.total_records());
            let _ = p.digest();
        }
        #[cfg(target_os = "linux")]
        Backend::Raw(p) => {
            collector.adopt_wire_summary(p.trace.digest_bytes(), p.trace.total_records());
            collector.record_capture_loss(p.trace.lost_records());
        }
    }
    let measurements = collector.finish(
        started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
        allocations.allocations,
        result.software_scenario_passed,
        Vec::new(),
    );
    std::fs::write(
        output.join("allocations.json"),
        serde_json::to_vec_pretty(&allocations).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        output.join("run.json"),
        serde_json::to_vec_pretty(&measurements).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(output.join("config.json"), config).map_err(|e| e.to_string())?;
    std::fs::write(output.join("topology.json"), &topology_bytes).map_err(|e| e.to_string())?;
    let capture_policy = match &port {
        Backend::Sim(p) => {
            serde_json::json!({"policy":if p.trace().lost_records()==0 {"complete"}else{"bounded_prefix"},"recorded_frames":p.trace().frames().len(),"total_frames":p.trace().total_records(),"lost_records":p.trace().lost_records()})
        }
        Backend::Replay(p, _) => {
            serde_json::json!({"policy":"complete","recorded_frames":p.frames().len(),"total_frames":p.total_records(),"lost_records":0})
        }
        #[cfg(target_os = "linux")]
        Backend::Raw(p) => {
            serde_json::json!({"policy":if p.trace.lost_records()==0 {"complete"}else{"bounded_prefix"},"recorded_frames":p.trace.frames().len(),"total_frames":p.trace.total_records(),"lost_records":p.trace.lost_records()})
        }
    };
    std::fs::write(
        output.join("capture-policy.json"),
        serde_json::to_vec_pretty(&capture_policy).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?;
    std::fs::write(output.join("scenario.json"), &json).map_err(|e| e.to_string())?;
    println!("{}", String::from_utf8_lossy(&json));
    if backend != "raw" && !result.software_scenario_passed {
        return Err("software scenario acceptance failed; artifacts were written".into());
    }
    Ok(())
}
