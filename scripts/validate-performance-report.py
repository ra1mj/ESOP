#!/usr/bin/env python3
"""Validate performance report completeness and qualification invariants."""

import json
import math
import pathlib
import re
import sys


SCHEMA = "esop.performance.v1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")
LATENCY_KEYS = ("p50", "p99", "p999", "max")
ERROR_KEYS = (
    "deadline_miss",
    "wkc_mismatch",
    "frame_timeout",
    "rx_overflow",
    "tx_starvation",
    "unmatched",
    "corrupt",
)
RESOURCE_KEYS = (
    "core_arena_bytes",
    "procbuf_bytes",
    "dma_bytes",
    "rt_stack_peak_bytes",
    "text_rodata_bytes",
    "rt_cpu_percent",
)
SCENARIOS = {
    "Q1": {
        "period_ns": 1_000_000, "axes": 32, "slaves": 32, "pdo_bytes": 1024,
        "frames": 2, "jitter_p50": 2_000, "jitter_p99": 10_000,
        "jitter_max": 50_000, "fast_p99": 250_000, "cpu_percent": 50,
    },
    "Q2": {
        "period_ns": 500_000, "axes": 16, "slaves": 24, "pdo_bytes": 512,
        "frames": None, "jitter_p50": 1_000, "jitter_p99": 5_000,
        "jitter_max": 25_000, "fast_p99": 125_000, "cpu_percent": 60,
    },
    "Q3": {
        "period_ns": 250_000, "axes": 8, "slaves": 16, "pdo_bytes": 256,
        "frames": 1, "jitter_p50": 1_000, "jitter_p99": 3_000,
        "jitter_max": 15_000, "fast_p99": 75_000, "cpu_percent": 70,
    },
    "Q4": {
        "period_ns": 1_000_000, "axes": 16, "slaves": 32, "pdo_bytes": 768,
        "frames": None, "jitter_p50": 2_000, "jitter_p99": 11_000,
        "jitter_max": 55_000, "fast_p99": 275_000, "cpu_percent": 55,
    },
}


def require(mapping: dict, key: str, path: str):
    if not isinstance(mapping, dict):
        raise ValueError(f"{path} must be an object")
    if key not in mapping:
        raise ValueError(f"{path} missing key: {key}")
    return mapping[key]


def nonnegative_number(value, path: str) -> None:
    if (isinstance(value, bool) or not isinstance(value, (int, float))
        or (isinstance(value, float) and not math.isfinite(value)) or value < 0):
        raise ValueError(f"{path} must be a finite non-negative number")


def nonnegative_integer(value, path: str) -> None:
    if type(value) is not int or value < 0:
        raise ValueError(f"{path} must be a non-negative integer")


def validate_stats(stats: dict, path: str) -> None:
    for key in LATENCY_KEYS:
        value = require(stats, key, path)
        nonnegative_number(value, f"{path}.{key}")
    if not (stats["p50"] <= stats["p99"] <= stats["p999"] <= stats["max"]):
        raise ValueError(f"{path} percentiles must be ordered and bounded by max")


def validate_report(report: dict) -> None:
    if require(report, "schema_version", "report") != SCHEMA:
        raise ValueError(f"schema_version must be {SCHEMA}")
    software = require(report, "software", "report")
    commit = require(software, "esop_commit", "software")
    config = require(software, "config_hash", "software")
    if not isinstance(commit, str) or not HEX40.fullmatch(commit):
        raise ValueError("software.esop_commit must be a 40-character lowercase git SHA")
    if not isinstance(config, str) or not HEX64.fullmatch(config):
        raise ValueError("software.config_hash must be a 64-character lowercase SHA-256")
    if not isinstance(require(software, "cflags", "software"), list):
        raise ValueError("software.cflags must be a list")

    platform = require(report, "platform", "report")
    nonnegative_integer(require(platform, "clock_hz", "platform"), "platform.clock_hz")
    for key in ("board", "soc", "port", "rtos_or_kernel", "cache_policy"):
        if not isinstance(require(platform, key, "platform"), str):
            raise ValueError(f"platform.{key} must be a string")

    topology = require(report, "topology", "report")
    for key in ("slave_count", "axis_count", "pdo_bytes", "frame_count", "io_channels"):
        nonnegative_integer(require(topology, key, "topology"), f"topology.{key}")
    if not isinstance(require(topology, "dc_enabled", "topology"), bool):
        raise ValueError("topology.dc_enabled must be a boolean")
    topology_hash = require(topology, "topology_manifest_hash", "topology")
    if not isinstance(topology_hash, str) or not HEX64.fullmatch(topology_hash):
        raise ValueError("topology.topology_manifest_hash must be a SHA-256")

    run = require(report, "run", "report")
    for key in ("period_ns", "duration_s", "cycles"):
        nonnegative_integer(require(run, key, "run"), f"run.{key}")
    for key in ("temperature_c_min", "temperature_c_max"):
        value = require(run, key, "run")
        if (isinstance(value, bool) or not isinstance(value, (int, float))
            or (isinstance(value, float) and not math.isfinite(value))):
            raise ValueError(f"run.{key} must be a finite temperature")
    if run["temperature_c_min"] > run["temperature_c_max"]:
        raise ValueError("run.temperature_c_min must not exceed temperature_c_max")

    latency = require(report, "latency_ns", "report")
    for key in ("release_jitter_abs", "fast_path", "rx_round_trip"):
        validate_stats(require(latency, key, "latency_ns"), f"latency_ns.{key}")

    errors = require(report, "errors", "report")
    for key in ERROR_KEYS:
        nonnegative_integer(require(errors, key, "errors"), f"errors.{key}")

    copies = require(report, "copies", "report")
    for key in ("tx_bytes_per_cycle", "rx_bytes_per_cycle", "copy_spans_per_cycle"):
        nonnegative_integer(require(copies, key, "copies"), f"copies.{key}")

    resources = require(report, "resources", "report")
    for key in RESOURCE_KEYS:
        value = require(resources, key, "resources")
        if value is not None:
            nonnegative_number(value, f"resources.{key}")

    measurement = require(report, "measurement", "report")
    for key in ("cycle_samples", "max_accumulator_samples"):
        nonnegative_integer(require(measurement, key, "measurement"), f"measurement.{key}")
    allocations = require(measurement, "heap_allocations_after_activation", "measurement")
    if allocations is not None:
        nonnegative_integer(allocations, "measurement.heap_allocations_after_activation")
    trace_hash = require(measurement, "trace_sha256", "measurement")
    if trace_hash is not None and (
        not isinstance(trace_hash, str) or not HEX64.fullmatch(trace_hash)
    ):
        raise ValueError("measurement.trace_sha256 must be a SHA-256 or null")
    source = require(measurement, "source", "measurement")
    if not isinstance(source, str):
        raise ValueError("measurement.source must be a string")

    workload = require(report, "workload", "report")
    nonnegative_integer(
        require(workload, "concurrent_sdo_requests", "workload"),
        "workload.concurrent_sdo_requests",
    )
    regression = require(workload, "p99_regression_percent", "workload")
    if regression is not None:
        nonnegative_number(regression, "workload.p99_regression_percent")
    baseline_hash = require(workload, "baseline_report_sha256", "workload")
    if baseline_hash is not None and (
        not isinstance(baseline_hash, str) or not HEX64.fullmatch(baseline_hash)
    ):
        raise ValueError("workload.baseline_report_sha256 must be a SHA-256 or null")
    baseline_p99 = require(workload, "baseline_fast_path_p99_ns", "workload")
    if baseline_p99 is not None:
        nonnegative_number(baseline_p99, "workload.baseline_fast_path_p99_ns")

    qualification = require(report, "qualification", "report")
    passed = require(qualification, "passed", "qualification")
    failures = require(qualification, "failures", "qualification")
    if not isinstance(passed, bool) or not isinstance(failures, list) or not all(
        isinstance(failure, str) and failure for failure in failures
    ):
        raise ValueError("qualification.passed must be bool and failures must be nonempty strings")
    if not passed:
        if not failures:
            raise ValueError("an unqualified report requires a reason in qualification.failures")
        return

    scenario_name = require(qualification, "scenario", "qualification")
    scenario = SCENARIOS.get(scenario_name) if isinstance(scenario_name, str) else None
    if scenario is None:
        raise ValueError("a passed report requires a known Q1/Q2/Q3/Q4 scenario")
    if failures:
        raise ValueError("a passed report cannot contain failures")
    if run["period_ns"] != scenario["period_ns"] or run["duration_s"] < 1800:
        raise ValueError("a passed report requires the scenario period and at least 1800 seconds")
    elapsed_ns = run["cycles"] * run["period_ns"]
    if (elapsed_ns < run["duration_s"] * 1_000_000_000
        or elapsed_ns > (run["duration_s"] + 1) * 1_000_000_000):
        raise ValueError("run.cycles must cover the measured duration without a large gap")
    if platform["clock_hz"] == 0 or any(
        not platform[key].strip()
        or platform[key].strip().lower() in (
            "unknown", "not-qualified", "host-development",
            "linux-raw-and-simulator", "linux-raw", "linux-af-packet", "af_packet",
        )
        or "pending" in platform[key].lower()
        for key in ("board", "soc", "port", "rtos_or_kernel", "cache_policy")
    ):
        raise ValueError("a passed report requires a qualified, non-placeholder platform")
    if (topology["slave_count"] == 0
        or topology["slave_count"] > scenario["slaves"]
        or topology["axis_count"] != scenario["axes"]
        or topology["pdo_bytes"] == 0
        or topology["pdo_bytes"] > scenario["pdo_bytes"]
        or topology["frame_count"] == 0
        or (scenario["frames"] is not None and topology["frame_count"] > scenario["frames"])
        or topology_hash == config):
        raise ValueError("a passed report requires the scenario topology and a separate topology manifest hash")
    if scenario_name != "Q3" and not topology["dc_enabled"]:
        raise ValueError("this scenario requires DC to be enabled")
    if scenario_name == "Q1" and topology["io_channels"] == 0:
        raise ValueError("Q1 requires a measured IO workload")
    jitter = latency["release_jitter_abs"]
    if any(latency[key]["max"] == 0 for key in ("release_jitter_abs", "fast_path", "rx_round_trip")):
        raise ValueError("a passed report requires nonzero measured latency maxima")
    if (jitter["p50"] > scenario["jitter_p50"]
        or jitter["p99"] > scenario["jitter_p99"]
        or jitter["max"] > scenario["jitter_max"]
        or latency["fast_path"]["p99"] > scenario["fast_p99"]
        or latency["fast_path"]["max"] > run["period_ns"]):
        raise ValueError("latency exceeds the scenario threshold")
    if any(errors[key] for key in ERROR_KEYS):
        raise ValueError("a passed fault-free report requires zero runtime errors")
    if copies["tx_bytes_per_cycle"] == 0 or copies["rx_bytes_per_cycle"] == 0:
        raise ValueError("a passed report requires measured cyclic TX and RX bytes")
    if (any(resources[key] is None or resources[key] <= 0 for key in RESOURCE_KEYS)
        or resources["rt_cpu_percent"] > scenario["cpu_percent"]):
        raise ValueError("a passed report requires measured resources within the CPU limit")
    if (source != "hil" or trace_hash is None or allocations != 0
        or measurement["cycle_samples"] != run["cycles"]
        or measurement["max_accumulator_samples"] != run["cycles"]):
        raise ValueError("a passed report requires full-cycle HIL evidence and zero heap allocations")
    if scenario_name == "Q4" and (topology["io_channels"] == 0
        or workload["concurrent_sdo_requests"] < 8
        or regression is None or regression > 10
        or baseline_hash is None or baseline_p99 is None
        or baseline_p99 == 0):
        raise ValueError("Q4 requires IO, eight concurrent SDO requests and a measured baseline regression <= 10%")
    if scenario_name == "Q4":
        expected_regression = max(0, (latency["fast_path"]["p99"] / baseline_p99 - 1) * 100)
        if abs(regression - expected_regression) > 0.05:
            raise ValueError("Q4 p99_regression_percent does not match the measured baseline")


def main() -> int:
    report_path = pathlib.Path(sys.argv[1]) if len(sys.argv) == 2 else pathlib.Path(
        "build/performance_report.json"
    )
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
        validate_report(report)
    except (OSError, json.JSONDecodeError, TypeError, ValueError, IndexError) as error:
        print(f"performance report invalid: {error}", file=sys.stderr)
        return 1

    print(f"performance report valid: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
