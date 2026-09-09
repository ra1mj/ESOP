#!/usr/bin/env python3
"""Validate the required fields and fail-closed qualification semantics."""

import json
import pathlib
import re
import sys


SCHEMA = "esop.build.v1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")


def fail(message: str) -> None:
    raise ValueError(message)


def require(mapping: dict, key: str, path: str):
    if key not in mapping:
        fail(f"{path} missing key: {key}")
    return mapping[key]


def main() -> int:
    report_path = pathlib.Path(sys.argv[1]) if len(sys.argv) == 2 else pathlib.Path(
        "build/robot_build_report.json"
    )
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if require(report, "schema_version", "report") != SCHEMA:
            fail(f"schema_version must be {SCHEMA}")

        software = require(report, "software", "report")
        commit = require(software, "esop_commit", "software")
        config = require(software, "config_hash", "software")
        if not isinstance(commit, str) or not HEX40.fullmatch(commit):
            fail("software.esop_commit must be a 40-character lowercase git SHA")
        if not isinstance(config, str) or not HEX64.fullmatch(config):
            fail("software.config_hash must be a 64-character lowercase SHA-256")
        if not isinstance(require(software, "packages", "software"), list):
            fail("software.packages must be a list")

        devices = require(report, "devices", "report")
        for key in ("declared_slaves", "declared_axes", "declared_io_channels"):
            if not isinstance(require(devices, key, "devices"), int):
                fail(f"devices.{key} must be an integer")

        process_data = require(report, "process_data", "report")
        for key in (
            "pdo_bytes_per_cycle",
            "frame_count",
            "expected_wkc",
            "copy_bytes_per_cycle",
        ):
            if not isinstance(require(process_data, key, "process_data"), int):
                fail(f"process_data.{key} must be an integer")

        cycle_budget = require(report, "cycle_budget", "report")
        for key in ("period_ns", "deadline_ns", "qualification"):
            require(cycle_budget, key, "cycle_budget")

        resources = require(report, "resources", "report")
        for key in (
            "workspace_release_artifact_bytes",
            "procbuf_bytes",
            "dma_bytes",
            "rt_stack_peak_bytes",
            "text_rodata_bytes",
        ):
            value = require(resources, key, "resources")
            if value is not None and (not isinstance(value, int) or value < 0):
                fail(f"resources.{key} must be a non-negative integer or null")

        qualification = require(report, "qualification", "report")
        if not isinstance(require(qualification, "failures", "qualification"), list):
            fail("qualification.failures must be a list")
        if qualification.get("passed") is True:
            failures = qualification["failures"]
            if failures:
                fail("a passed report cannot contain failures")
            if cycle_budget["period_ns"] <= 0 or cycle_budget["deadline_ns"] <= 0:
                fail("a passed report requires positive cycle period and deadline")
            for key in ("procbuf_bytes", "dma_bytes", "rt_stack_peak_bytes", "text_rodata_bytes"):
                if resources[key] is None:
                    fail(f"a passed report requires resources.{key}")
    except (OSError, json.JSONDecodeError, TypeError, ValueError, IndexError) as error:
        print(f"robot build report invalid: {error}", file=sys.stderr)
        return 1

    print(f"robot build report valid: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
