#!/usr/bin/env python3
"""Generate a complete, explicitly unqualified performance report baseline."""

import argparse
import hashlib
import json
import pathlib
import subprocess
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parents[1]


def command(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def config_hash() -> str:
    digest = hashlib.sha256()
    for path in (ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "capability_manifest.json"):
        digest.update(path.name.encode("utf-8"))
        digest.update(path.read_bytes())
    return digest.hexdigest()


def latency_stats() -> dict[str, int]:
    return {"p50": 0, "p99": 0, "p999": 0, "max": 0}


def performance_report() -> dict:
    return {
        "schema_version": "esop.performance.v1",
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "software": {
            "esop_commit": command("git", "rev-parse", "HEAD"),
            "config_hash": config_hash(),
            "compiler": command("rustc", "--version"),
            "cflags": [],
        },
        "platform": {
            "board": "host-development",
            "soc": "unknown",
            "clock_hz": 0,
            "port": "linux-raw-and-simulator",
            "rtos_or_kernel": "not-qualified",
            "cache_policy": "host-managed; target cache qualification pending",
        },
        "topology": {
            "slave_count": 0,
            "axis_count": 0,
            "pdo_bytes": 0,
            "frame_count": 0,
            "dc_enabled": False,
            "topology_manifest_hash": config_hash(),
        },
        "run": {
            "period_ns": 0,
            "duration_s": 0,
            "cycles": 0,
            "temperature_c_min": 0,
            "temperature_c_max": 0,
        },
        "latency_ns": {
            "release_jitter_abs": latency_stats(),
            "fast_path": latency_stats(),
            "rx_round_trip": latency_stats(),
        },
        "errors": {
            "deadline_miss": 0,
            "wkc_mismatch": 0,
            "frame_timeout": 0,
            "rx_overflow": 0,
            "tx_starvation": 0,
            "unmatched": 0,
            "corrupt": 0,
        },
        "copies": {
            "tx_bytes_per_cycle": 0,
            "rx_bytes_per_cycle": 0,
            "copy_spans_per_cycle": 0,
        },
        "resources": {
            "core_arena_bytes": None,
            "procbuf_bytes": None,
            "dma_bytes": None,
            "rt_stack_peak_bytes": None,
            "text_rodata_bytes": None,
            "rt_cpu_percent": None,
        },
        "qualification": {
            "scenario": "host-build-baseline",
            "passed": False,
            "failures": [
                "no target board, topology, cycle trace or resource map supplied"
            ],
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    output = args.output if args.output.is_absolute() else ROOT / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(performance_report(), indent=2) + "\n", encoding="utf-8")
    print(f"performance report generated: {output.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
