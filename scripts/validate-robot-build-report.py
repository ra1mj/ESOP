#!/usr/bin/env python3
"""Validate the required fields and fail-closed qualification semantics."""

import json
import pathlib
import re
import sys


SCHEMA = "esop.build.v1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX_U16 = re.compile(r"^0x[0-9a-f]{4}$")
CANOPEN_DATA_TYPES = {
    0x0001,
    0x0002,
    0x0003,
    0x0004,
    0x0005,
    0x0006,
    0x0007,
    0x0008,
    0x0009,
    0x000A,
    0x000B,
    0x000C,
    0x000D,
    0x000F,
    0x0010,
    0x0011,
    0x0012,
    0x0013,
    0x0014,
    0x0015,
    0x0016,
    0x0018,
    0x0019,
    0x001A,
    0x001B,
    *range(0x0030, 0x0038),
}
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


def validate_complete_access_devices(value: object, declared_slaves: int) -> None:
    if not isinstance(value, list):
        fail("devices.coe_complete_access must be a list")
    if len(value) != declared_slaves:
        fail("devices.coe_complete_access length must match declared_slaves")

    names: set[str] = set()
    positions: set[int] = set()
    for index, entry in enumerate(value):
        path = f"devices.coe_complete_access[{index}]"
        if not isinstance(entry, dict):
            fail(f"{path} must be an object")
        expected = {"name", "position", "supported", "enabled"}
        if set(entry) != expected:
            fail(f"{path} must contain exactly name, position, supported, enabled")
        name = entry["name"]
        position = entry["position"]
        if not isinstance(name, str) or not name.strip():
            fail(f"{path}.name must be a non-empty string")
        if type(position) is not int or not 0 <= position <= 0xFFFF:
            fail(f"{path}.position must be a non-negative u16 integer")
        if type(entry["supported"]) is not bool:
            fail(f"{path}.supported must be a boolean")
        if type(entry["enabled"]) is not bool:
            fail(f"{path}.enabled must be a boolean")
        if entry["enabled"] and not entry["supported"]:
            fail(f"{path} cannot enable unsupported Complete Access")
        if name in names:
            fail(f"devices.coe_complete_access contains duplicate name: {name}")
        if position in positions:
            fail(f"devices.coe_complete_access contains duplicate position: {position}")
        names.add(name)
        positions.add(position)


def parse_hex_u16(value: object, path: str, *, nonzero: bool = False) -> int:
    if not isinstance(value, str) or not HEX_U16.fullmatch(value):
        fail(f"{path} must be a lowercase 0x-prefixed u16")
    parsed = int(value, 16)
    if nonzero and parsed == 0:
        fail(f"{path} must be nonzero")
    return parsed


def validate_sdo_information_devices(value: object, declared_slaves: int) -> None:
    if not isinstance(value, list):
        fail("devices.coe_sdo_information must be a list")
    if len(value) != declared_slaves:
        fail("devices.coe_sdo_information length must match declared_slaves")

    names: set[str] = set()
    positions: set[int] = set()
    for index, entry in enumerate(value):
        path = f"devices.coe_sdo_information[{index}]"
        if not isinstance(entry, dict):
            fail(f"{path} must be an object")
        expected_keys = {"name", "position", "supported", "enabled", "expectations"}
        if set(entry) != expected_keys:
            fail(
                f"{path} must contain exactly name, position, supported, enabled, expectations"
            )
        name = entry["name"]
        position = entry["position"]
        if not isinstance(name, str) or not name.strip():
            fail(f"{path}.name must be a non-empty string")
        if type(position) is not int or not 0 <= position <= 0xFFFF:
            fail(f"{path}.position must be a non-negative u16 integer")
        if type(entry["supported"]) is not bool:
            fail(f"{path}.supported must be a boolean")
        if type(entry["enabled"]) is not bool:
            fail(f"{path}.enabled must be a boolean")
        if entry["enabled"] and not entry["supported"]:
            fail(f"{path} cannot enable unsupported SDO Information")
        if name in names:
            fail(f"devices.coe_sdo_information contains duplicate name: {name}")
        if position in positions:
            fail(f"devices.coe_sdo_information contains duplicate position: {position}")

        expectations = entry["expectations"]
        if not isinstance(expectations, list):
            fail(f"{path}.expectations must be a list")
        if entry["enabled"] and not expectations:
            fail(f"{path} enabled plan must not be empty")
        if not entry["enabled"] and expectations:
            fail(f"{path} disabled plan must be empty")
        previous: tuple[int, int] | None = None
        for expectation_index, expectation in enumerate(expectations):
            expectation_path = f"{path}.expectations[{expectation_index}]"
            if not isinstance(expectation, dict):
                fail(f"{expectation_path} must be an object")
            expected_expectation_keys = {
                "slave_position",
                "index",
                "subindex",
                "data_type",
                "bit_length",
                "required_access",
            }
            if set(expectation) != expected_expectation_keys:
                fail(f"{expectation_path} has an invalid field set")
            owner = expectation["slave_position"]
            if type(owner) is not int or owner != position:
                fail(f"{expectation_path}.slave_position must match position")
            object_index = parse_hex_u16(
                expectation["index"], f"{expectation_path}.index", nonzero=True
            )
            subindex = expectation["subindex"]
            if type(subindex) is not int or not 0 <= subindex <= 0xFF:
                fail(f"{expectation_path}.subindex must be a non-negative u8 integer")
            data_type = parse_hex_u16(
                expectation["data_type"],
                f"{expectation_path}.data_type",
                nonzero=True,
            )
            if data_type not in CANOPEN_DATA_TYPES:
                fail(f"{expectation_path}.data_type is unsupported")
            bit_length = expectation["bit_length"]
            if type(bit_length) is not int or not 1 <= bit_length <= 64:
                fail(f"{expectation_path}.bit_length must be in 1..64")
            required_access = expectation["required_access"]
            if (
                type(required_access) is not int
                or required_access == 0
                or required_access & ~0x0F
            ):
                fail(
                    f"{expectation_path}.required_access must contain known nonzero flags"
                )
            identity = (object_index, subindex)
            if previous is not None and identity <= previous:
                fail(f"{path}.expectations must be strictly ordered")
            previous = identity

        names.add(name)
        positions.add(position)


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
    complete_access = require(devices, "coe_complete_access", "devices")
    sdo_information = require(devices, "coe_sdo_information", "devices")
    validate_complete_access_devices(complete_access, devices["declared_slaves"])
    validate_sdo_information_devices(sdo_information, devices["declared_slaves"])
    complete_identities = [
        (entry["name"], entry["position"]) for entry in complete_access
    ]
    sdo_identities = [
        (entry["name"], entry["position"]) for entry in sdo_information
    ]
    if complete_identities != sdo_identities:
        fail("devices CoE device identities must match")

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
    if "wire_bytes_per_cycle" in process_data:
        wire_bytes = process_data["wire_bytes_per_cycle"]
        if type(wire_bytes) is not int or wire_bytes < 0:
            fail("process_data.wire_bytes_per_cycle must be a non-negative integer")

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
    if "wire_bytes_per_cycle" in process_data and process_data["wire_bytes_per_cycle"] <= 0:
        fail("a passed report with wire bytes requires a positive value")
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
