#!/usr/bin/env python3
"""Validate performance report completeness and qualification invariants."""

import json
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


def require(mapping: dict, key: str, path: str):
    if key not in mapping:
        raise ValueError(f"{path} missing key: {key}")
    return mapping[key]


def validate_stats(stats: dict, path: str) -> None:
    for key in LATENCY_KEYS:
        value = require(stats, key, path)
        if not isinstance(value, (int, float)) or value < 0:
            raise ValueError(f"{path}.{key} must be non-negative")


def main() -> int:
    report_path = pathlib.Path(sys.argv[1]) if len(sys.argv) == 2 else pathlib.Path(
        "build/performance_report.json"
    )
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
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

        topology = require(report, "topology", "report")
        for key in ("slave_count", "axis_count", "pdo_bytes", "frame_count"):
            if not isinstance(require(topology, key, "topology"), int):
                raise ValueError(f"topology.{key} must be an integer")
        topology_hash = require(topology, "topology_manifest_hash", "topology")
        if not isinstance(topology_hash, str) or not HEX64.fullmatch(topology_hash):
            raise ValueError("topology.topology_manifest_hash must be a SHA-256")

        run = require(report, "run", "report")
        for key in ("period_ns", "duration_s", "cycles"):
            value = require(run, key, "run")
            if not isinstance(value, int) or value < 0:
                raise ValueError(f"run.{key} must be a non-negative integer")

        latency = require(report, "latency_ns", "report")
        for key in ("release_jitter_abs", "fast_path", "rx_round_trip"):
            validate_stats(require(latency, key, "latency_ns"), f"latency_ns.{key}")

        errors = require(report, "errors", "report")
        for key in ERROR_KEYS:
            value = require(errors, key, "errors")
            if not isinstance(value, int) or value < 0:
                raise ValueError(f"errors.{key} must be a non-negative integer")

        copies = require(report, "copies", "report")
        for key in ("tx_bytes_per_cycle", "rx_bytes_per_cycle", "copy_spans_per_cycle"):
            value = require(copies, key, "copies")
            if not isinstance(value, int) or value < 0:
                raise ValueError(f"copies.{key} must be a non-negative integer")

        resources = require(report, "resources", "report")
        for key in RESOURCE_KEYS:
            value = require(resources, key, "resources")
            if value is not None and (not isinstance(value, (int, float)) or value < 0):
                raise ValueError(f"resources.{key} must be non-negative or null")

        qualification = require(report, "qualification", "report")
        passed = require(qualification, "passed", "qualification")
        failures = require(qualification, "failures", "qualification")
        if not isinstance(passed, bool) or not isinstance(failures, list):
            raise ValueError("qualification.passed must be bool and failures must be list")
        if passed:
            if failures or run["cycles"] <= 0:
                raise ValueError("a passed report requires cycles and no failures")
            if any(stats["max"] < stats["p99"] for stats in latency.values()):
                raise ValueError("latency max must be at least p99")
            if any(resources[key] is None for key in RESOURCE_KEYS):
                raise ValueError("a passed report requires all resource measurements")
    except (OSError, json.JSONDecodeError, TypeError, ValueError, IndexError) as error:
        print(f"performance report invalid: {error}", file=sys.stderr)
        return 1

    print(f"performance report valid: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
