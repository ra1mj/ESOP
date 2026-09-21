#!/usr/bin/env python3
"""Validate the required fields and fail-closed qualification semantics."""

import json
import pathlib
import re
import sys


SCHEMA = "esop.build.v1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")
PLACEHOLDER_PLATFORM_VALUES = {
    "",
    "unknown",
    "not-qualified",
    "host-development",
    "linux-raw-and-simulator",
    "linux-raw",
    "linux-af-packet",
    "af_packet",
}


def fail(message: str) -> None:
    raise ValueError(message)


def require(mapping: dict, key: str, path: str):
    if not isinstance(mapping, dict):
        fail(f"{path} must be an object")
    if key not in mapping:
        fail(f"{path} missing key: {key}")
    return mapping[key]


def validate_report(report: dict) -> None:
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

    platform = require(report, "platform", "report")
    for key in ("board", "soc", "port", "rtos_or_kernel", "cache_policy"):
        if not isinstance(require(platform, key, "platform"), str):
            fail(f"platform.{key} must be a string")

    devices = require(report, "devices", "report")
    for key in ("declared_slaves", "declared_axes", "declared_io_channels"):
        value = require(devices, key, "devices")
        if type(value) is not int or value < 0:
            fail(f"devices.{key} must be a non-negative integer")
    if not isinstance(require(devices, "source", "devices"), str):
        fail("devices.source must be a string")

    process_data = require(report, "process_data", "report")
    for key in (
        "pdo_bytes_per_cycle",
        "frame_count",
        "expected_wkc",
        "copy_bytes_per_cycle",
    ):
        value = require(process_data, key, "process_data")
        if type(value) is not int or value < 0:
            fail(f"process_data.{key} must be a non-negative integer")

    cycle_budget = require(report, "cycle_budget", "report")
    for key in ("period_ns", "deadline_ns", "qualification"):
        require(cycle_budget, key, "cycle_budget")
    for key in ("period_ns", "deadline_ns"):
        if type(cycle_budget[key]) is not int or cycle_budget[key] < 0:
            fail(f"cycle_budget.{key} must be a non-negative integer")
    if not isinstance(cycle_budget["qualification"], str):
        fail("cycle_budget.qualification must be a string")

    resources = require(report, "resources", "report")
    for key in (
        "workspace_release_artifact_bytes",
        "procbuf_bytes",
        "dma_bytes",
        "rt_stack_peak_bytes",
        "text_rodata_bytes",
    ):
        value = require(resources, key, "resources")
        if value is not None and (type(value) is not int or value < 0):
            fail(f"resources.{key} must be a non-negative integer or null")

    qualification = require(report, "qualification", "report")
    passed = require(qualification, "passed", "qualification")
    failures = require(qualification, "failures", "qualification")
    if not isinstance(passed, bool):
        fail("qualification.passed must be a boolean")
    if not isinstance(failures, list) or not all(
        isinstance(reason, str) and reason for reason in failures
    ):
        fail("qualification.failures must be a list of non-empty strings")
    if not passed:
        if not failures:
            fail("an unqualified report requires a reason in qualification.failures")
        return
    if failures:
        fail("a passed report cannot contain failures")
    if cycle_budget["period_ns"] <= 0 or cycle_budget["deadline_ns"] <= 0:
        fail("a passed report requires positive cycle period and deadline")
    if cycle_budget["deadline_ns"] > cycle_budget["period_ns"]:
        fail("a passed report requires deadline_ns not to exceed period_ns")
    if cycle_budget["qualification"] != "qualified":
        fail("a passed report requires a qualified cycle budget")
    if any(
        not platform[key].strip()
        or platform[key].strip().lower() in PLACEHOLDER_PLATFORM_VALUES
        or "pending" in platform[key].lower()
        for key in ("board", "soc", "port", "rtos_or_kernel", "cache_policy")
    ):
        fail("a passed report requires a qualified, non-placeholder platform")
    if (
        devices["declared_slaves"] <= 0
        or devices["declared_axes"] <= 0
        or devices["declared_io_channels"] <= 0
        or not devices["source"].strip()
        or "no hardware" in devices["source"].lower()
    ):
        fail("a passed report requires a target hardware topology")
    if any(
        process_data[key] <= 0
        for key in (
            "pdo_bytes_per_cycle",
            "frame_count",
            "expected_wkc",
            "copy_bytes_per_cycle",
        )
    ):
        fail("a passed report requires measured nonzero process data")
    for key in ("procbuf_bytes", "dma_bytes", "rt_stack_peak_bytes", "text_rodata_bytes"):
        if resources[key] is None or resources[key] <= 0:
            fail(f"a passed report requires positive resources.{key}")
    if resources["workspace_release_artifact_bytes"] <= 0:
        fail("a passed report requires a nonempty release artifact")


def main() -> int:
    report_path = pathlib.Path(sys.argv[1]) if len(sys.argv) == 2 else pathlib.Path(
        "build/robot_build_report.json"
    )
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
        validate_report(report)
    except (OSError, json.JSONDecodeError, TypeError, ValueError, IndexError) as error:
        print(f"robot build report invalid: {error}", file=sys.stderr)
        return 1

    print(f"robot build report valid: {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
