#![cfg(target_os = "linux")]

use std::error::Error;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use esop_ebpf_agent::{CycleContext, RuntimeAgent};
use esop_ebpf_runtime::{BpfRuntime, RuntimeConfig};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let object_path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: cargo run -p esop-ebpf-runtime --example observe -- <object.o> [polls]")?;
    let max_polls = arguments
        .next()
        .map(|value| value.to_string_lossy().parse::<usize>())
        .transpose()?
        .unwrap_or(0);

    let config = RuntimeConfig {
        // Production ESOP supplies these values from its boot/lifecycle
        // coordinator. The example uses a deterministic standalone identity.
        boot_id: 1,
        agent_epoch: 1,
        ..RuntimeConfig::default()
    };
    let preflight = BpfRuntime::preflight(config.required_attach_mask);
    eprintln!("eBPF preflight: {:?}", preflight);

    let mut runtime = BpfRuntime::load(object_path, config)?;
    let mut agent = RuntimeAgent::<64>::new(config.boot_id, config.agent_epoch, 2_000_000);
    runtime.apply_capability_snapshot(&mut agent);
    runtime.update_cycle_context(CycleContext {
        boot_id: config.boot_id,
        cycle_seq: 1,
        ..CycleContext::EMPTY
    })?;

    let mut polls = 0;
    loop {
        let report = runtime.poll(&mut agent, 256)?;
        while let Some(incident) = agent.pop_incident() {
            eprintln!("incident: {:?}", incident);
        }
        if report.records_seen != 0 || report.newly_reported_lost_events != 0 {
            eprintln!("poll: {:?}", report);
        }
        polls += 1;
        if max_polls != 0 && polls >= max_polls {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
