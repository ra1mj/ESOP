#!/usr/bin/env python3
"""Generate the evidence envelope for one ESOP workspace build."""

import argparse
import hashlib
import json
import pathlib
import platform
import re
import subprocess
import sys
from datetime import datetime, timezone


ROOT = pathlib.Path(__file__).resolve().parents[1]
PRODUCT_INPUT_SCHEMA = "esop.product-build-input.v1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
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


def require_exact_object(value: object, keys: set[str], path: str) -> dict:
    if not isinstance(value, dict):
        raise ValueError(f"{path} must be an object")
    missing = sorted(keys - value.keys())
    unknown = sorted(value.keys() - keys)
    if missing:
        raise ValueError(f"{path} missing keys: {', '.join(missing)}")
    if unknown:
        raise ValueError(f"{path} has unknown keys: {', '.join(unknown)}")
    return value


def require_nonempty_string(value: object, path: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{path} must be a non-empty string")
    return value


def require_nonnegative_int(value: object, path: str) -> int:
    if type(value) is not int or value < 0:
        raise ValueError(f"{path} must be a non-negative integer")
    return value


def require_optional_nonnegative_int(value: object, path: str) -> int | None:
    if value is None:
        return None
    return require_nonnegative_int(value, path)


def validate_complete_access_devices(
    value: object, declared_slaves: int, path: str
) -> list[dict]:
    if not isinstance(value, list):
        raise ValueError(f"{path} must be a list")
    if len(value) != declared_slaves:
        raise ValueError(f"{path} length must match declared_slaves")

    names: set[str] = set()
    positions: set[int] = set()
    checked: list[dict] = []
    for index, entry in enumerate(value):
        item_path = f"{path}[{index}]"
        item = require_exact_object(
            entry, {"name", "position", "supported", "enabled"}, item_path
        )
        name = require_nonempty_string(item["name"], f"{item_path}.name")
        position = require_nonnegative_int(item["position"], f"{item_path}.position")
        if position > 0xFFFF:
            raise ValueError(f"{item_path}.position must fit in u16")
        if type(item["supported"]) is not bool:
            raise ValueError(f"{item_path}.supported must be a boolean")
        if type(item["enabled"]) is not bool:
            raise ValueError(f"{item_path}.enabled must be a boolean")
        if item["enabled"] and not item["supported"]:
            raise ValueError(f"{item_path} cannot enable unsupported Complete Access")
        if name in names:
            raise ValueError(f"{path} contains duplicate name: {name}")
        if position in positions:
            raise ValueError(f"{path} contains duplicate position: {position}")
        names.add(name)
        positions.add(position)
        checked.append(item)
    return checked


def require_hex_u16(value: object, path: str, *, nonzero: bool = False) -> int:
    if not isinstance(value, str) or not HEX_U16.fullmatch(value):
        raise ValueError(f"{path} must be a lowercase 0x-prefixed u16")
    parsed = int(value, 16)
    if nonzero and parsed == 0:
        raise ValueError(f"{path} must be nonzero")
    return parsed


def validate_requesting_id_devices(
    value: object, declared_slaves: int, path: str
) -> list[dict]:
    if not isinstance(value, list):
        raise ValueError(f"{path} must be a list")
    if len(value) != declared_slaves:
        raise ValueError(f"{path} length must match declared_slaves")

    names: set[str] = set()
    positions: set[int] = set()
    checked: list[dict] = []
    for index, entry in enumerate(value):
        item_path = f"{path}[{index}]"
        item = require_exact_object(
            entry, {"name", "position", "supported", "expected"}, item_path
        )
        name = require_nonempty_string(item["name"], f"{item_path}.name")
        position = require_nonnegative_int(item["position"], f"{item_path}.position")
        if position > 0xFFFF:
            raise ValueError(f"{item_path}.position must fit in u16")
        if type(item["supported"]) is not bool:
            raise ValueError(f"{item_path}.supported must be a boolean")
        expected = item["expected"]
        if expected is not None:
            require_hex_u16(expected, f"{item_path}.expected")
            if not item["supported"]:
                raise ValueError(
                    f"{item_path} cannot expect an unsupported Requesting ID"
                )
        if name in names:
            raise ValueError(f"{path} contains duplicate name: {name}")
        if position in positions:
            raise ValueError(f"{path} contains duplicate position: {position}")
        names.add(name)
        positions.add(position)
        checked.append(item)
    return checked


def validate_sdo_information_devices(
    value: object, declared_slaves: int, path: str
) -> list[dict]:
    if not isinstance(value, list):
        raise ValueError(f"{path} must be a list")
    if len(value) != declared_slaves:
        raise ValueError(f"{path} length must match declared_slaves")

    names: set[str] = set()
    positions: set[int] = set()
    checked: list[dict] = []
    for index, entry in enumerate(value):
        item_path = f"{path}[{index}]"
        item = require_exact_object(
            entry,
            {"name", "position", "supported", "enabled", "expectations"},
            item_path,
        )
        name = require_nonempty_string(item["name"], f"{item_path}.name")
        position = require_nonnegative_int(item["position"], f"{item_path}.position")
        if position > 0xFFFF:
            raise ValueError(f"{item_path}.position must fit in u16")
        if type(item["supported"]) is not bool:
            raise ValueError(f"{item_path}.supported must be a boolean")
        if type(item["enabled"]) is not bool:
            raise ValueError(f"{item_path}.enabled must be a boolean")
        if item["enabled"] and not item["supported"]:
            raise ValueError(f"{item_path} cannot enable unsupported SDO Information")
        if name in names:
            raise ValueError(f"{path} contains duplicate name: {name}")
        if position in positions:
            raise ValueError(f"{path} contains duplicate position: {position}")

        expectations = item["expectations"]
        if not isinstance(expectations, list):
            raise ValueError(f"{item_path}.expectations must be a list")
        if item["enabled"] and not expectations:
            raise ValueError(f"{item_path} enabled plan must not be empty")
        if not item["enabled"] and expectations:
            raise ValueError(f"{item_path} disabled plan must be empty")
        previous: tuple[int, int] | None = None
        for expectation_index, expectation in enumerate(expectations):
            expectation_path = f"{item_path}.expectations[{expectation_index}]"
            expected = require_exact_object(
                expectation,
                {
                    "slave_position",
                    "index",
                    "subindex",
                    "data_type",
                    "bit_length",
                    "required_access",
                },
                expectation_path,
            )
            owner = require_nonnegative_int(
                expected["slave_position"], f"{expectation_path}.slave_position"
            )
            if owner != position:
                raise ValueError(f"{expectation_path}.slave_position must match position")
            object_index = require_hex_u16(
                expected["index"], f"{expectation_path}.index", nonzero=True
            )
            subindex = require_nonnegative_int(
                expected["subindex"], f"{expectation_path}.subindex"
            )
            if subindex > 0xFF:
                raise ValueError(f"{expectation_path}.subindex must fit in u8")
            data_type = require_hex_u16(
                expected["data_type"], f"{expectation_path}.data_type", nonzero=True
            )
            if data_type not in CANOPEN_DATA_TYPES:
                raise ValueError(f"{expectation_path}.data_type is unsupported")
            bit_length = require_nonnegative_int(
                expected["bit_length"], f"{expectation_path}.bit_length"
            )
            if not 1 <= bit_length <= 64:
                raise ValueError(f"{expectation_path}.bit_length must be in 1..64")
            required_access = require_nonnegative_int(
                expected["required_access"], f"{expectation_path}.required_access"
            )
            if required_access == 0 or required_access & ~0x0F:
                raise ValueError(
                    f"{expectation_path}.required_access must contain known nonzero flags"
                )
            identity = (object_index, subindex)
            if previous is not None and identity <= previous:
                raise ValueError(f"{item_path}.expectations must be strictly ordered")
            previous = identity

        names.add(name)
        positions.add(position)
        checked.append(item)
    return checked


def validate_product_input(value: object) -> dict:
    product = require_exact_object(
        value,
        {
            "schema_version",
            "config_sha256",
            "platform",
            "devices",
            "process_data",
            "cycle_budget",
            "resources",
            "qualification",
        },
        "product input",
    )
    if product["schema_version"] != PRODUCT_INPUT_SCHEMA:
        raise ValueError(f"product input schema_version must be {PRODUCT_INPUT_SCHEMA}")
    config_sha256 = product["config_sha256"]
    if not isinstance(config_sha256, str) or not HEX64.fullmatch(config_sha256):
        raise ValueError("product input config_sha256 must be a lowercase SHA-256")

    platform_input = require_exact_object(
        product["platform"],
        {"board", "soc", "port", "rtos_or_kernel", "cache_policy"},
        "product input platform",
    )
    for key in platform_input:
        require_nonempty_string(platform_input[key], f"product input platform.{key}")

    devices = require_exact_object(
        product["devices"],
        {
            "declared_slaves",
            "declared_axes",
            "declared_io_channels",
            "coe_complete_access",
            "coe_sdo_information",
            "requesting_id",
            "source",
        },
        "product input devices",
    )
    for key in ("declared_slaves", "declared_axes", "declared_io_channels"):
        require_nonnegative_int(devices[key], f"product input devices.{key}")
    if devices["declared_slaves"] == 0:
        raise ValueError("product input devices.declared_slaves must be positive")
    complete_access = validate_complete_access_devices(
        devices["coe_complete_access"],
        devices["declared_slaves"],
        "product input devices.coe_complete_access",
    )
    sdo_information = validate_sdo_information_devices(
        devices["coe_sdo_information"],
        devices["declared_slaves"],
        "product input devices.coe_sdo_information",
    )
    requesting_id = validate_requesting_id_devices(
        devices["requesting_id"],
        devices["declared_slaves"],
        "product input devices.requesting_id",
    )
    if [
        (entry["name"], entry["position"]) for entry in complete_access
    ] != [(entry["name"], entry["position"]) for entry in sdo_information]:
        raise ValueError("product input CoE device identities must match")
    if [
        (entry["name"], entry["position"]) for entry in complete_access
    ] != [(entry["name"], entry["position"]) for entry in requesting_id]:
        raise ValueError("product input Requesting ID device identities must match")
    require_nonempty_string(devices["source"], "product input devices.source")

    process_data = require_exact_object(
        product["process_data"],
        {
            "pdo_bytes_per_cycle",
            "frame_count",
            "expected_wkc",
            "copy_bytes_per_cycle",
            "wire_bytes_per_cycle",
        },
        "product input process_data",
    )
    for key in process_data:
        value = require_nonnegative_int(
            process_data[key], f"product input process_data.{key}"
        )
        if value == 0:
            raise ValueError(f"product input process_data.{key} must be positive")

    cycle_budget = require_exact_object(
        product["cycle_budget"],
        {"period_ns", "deadline_ns", "qualification"},
        "product input cycle_budget",
    )
    period_ns = require_nonnegative_int(
        cycle_budget["period_ns"], "product input cycle_budget.period_ns"
    )
    deadline_ns = require_nonnegative_int(
        cycle_budget["deadline_ns"], "product input cycle_budget.deadline_ns"
    )
    if period_ns == 0 or deadline_ns == 0:
        raise ValueError("product input cycle budget must be positive")
    if deadline_ns > period_ns:
        raise ValueError("product input deadline_ns must not exceed period_ns")
    if cycle_budget["qualification"] != "generated_not_measured":
        raise ValueError(
            "product input cycle_budget.qualification must be generated_not_measured"
        )

    resources = require_exact_object(
        product["resources"],
        {"procbuf_bytes", "dma_bytes", "rt_stack_peak_bytes", "text_rodata_bytes"},
        "product input resources",
    )
    if require_nonnegative_int(
        resources["procbuf_bytes"], "product input resources.procbuf_bytes"
    ) == 0:
        raise ValueError("product input resources.procbuf_bytes must be positive")
    for key in ("dma_bytes", "rt_stack_peak_bytes", "text_rodata_bytes"):
        require_optional_nonnegative_int(resources[key], f"product input resources.{key}")

    qualification = require_exact_object(
        product["qualification"],
        {"scenario", "passed", "failures"},
        "product input qualification",
    )
    require_nonempty_string(
        qualification["scenario"], "product input qualification.scenario"
    )
    if qualification["passed"] is not False:
        raise ValueError("generated product input must remain unqualified")
    failures = qualification["failures"]
    if not isinstance(failures, list) or not failures:
        raise ValueError("product input qualification.failures must be non-empty")
    for index, failure in enumerate(failures):
        require_nonempty_string(
            failure, f"product input qualification.failures[{index}]"
        )

    return product


def command(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def config_hash() -> str:
    digest = hashlib.sha256()
    for path in (ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "capability_manifest.json"):
        digest.update(path.name.encode("utf-8"))
        digest.update(path.read_bytes())
    return digest.hexdigest()


def release_artifact_bytes() -> int:
    release = ROOT / "target" / "release"
    if not release.is_dir():
        return 0
    return sum(path.stat().st_size for path in release.rglob("*") if path.is_file())


def package_names() -> list[str]:
    metadata = json.loads(command("cargo", "metadata", "--no-deps", "--format-version", "1"))
    return sorted(package["name"] for package in metadata["packages"])


def build_report(product_input: object | None = None) -> dict:
    report = {
        "schema_version": "esop.build.v1",
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "software": {
            "esop_commit": command("git", "rev-parse", "HEAD"),
            "config_hash": config_hash(),
            "compiler": command("rustc", "--version"),
            "packages": package_names(),
        },
        "platform": {
            "board": "host-development",
            "soc": "unknown",
            "port": "linux-raw-and-simulator",
            "rtos_or_kernel": f"{platform.system()} {platform.release()}",
            "cache_policy": "host-managed; target cache qualification pending",
        },
        "devices": {
            "declared_slaves": 0,
            "declared_axes": 0,
            "declared_io_channels": 0,
            "coe_complete_access": [],
            "coe_sdo_information": [],
            "requesting_id": [],
            "source": "no hardware topology supplied",
        },
        "process_data": {
            "pdo_bytes_per_cycle": 0,
            "frame_count": 0,
            "expected_wkc": 0,
            "copy_bytes_per_cycle": 0,
        },
        "cycle_budget": {
            "period_ns": 0,
            "deadline_ns": 0,
            "qualification": "not_measured",
        },
        "resources": {
            "workspace_release_artifact_bytes": release_artifact_bytes(),
            "procbuf_bytes": None,
            "dma_bytes": None,
            "rt_stack_peak_bytes": None,
            "text_rodata_bytes": None,
        },
        "qualification": {
            "scenario": "host-build",
            "passed": False,
            "failures": [
                "target board, topology, cycle measurements and hardware resource map are not supplied"
            ],
        },
    }
    if product_input is None:
        return report

    product = validate_product_input(product_input)
    report["software"]["config_hash"] = product["config_sha256"]
    report["platform"] = dict(product["platform"])
    report["devices"] = dict(product["devices"])
    report["process_data"] = dict(product["process_data"])
    report["cycle_budget"] = dict(product["cycle_budget"])
    report["resources"] = {
        "workspace_release_artifact_bytes": release_artifact_bytes(),
        **product["resources"],
    }
    report["qualification"] = {
        "scenario": product["qualification"]["scenario"],
        "passed": False,
        "failures": list(product["qualification"]["failures"]),
    }
    return report


def display_path(path: pathlib.Path) -> pathlib.Path:
    try:
        return path.relative_to(ROOT)
    except ValueError:
        return path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--product-input", type=pathlib.Path)
    args = parser.parse_args()
    try:
        product_input = None
        if args.product_input is not None:
            input_path = (
                args.product_input
                if args.product_input.is_absolute()
                else ROOT / args.product_input
            )
            product_input = json.loads(input_path.read_text(encoding="utf-8"))
        report = build_report(product_input)
        output = args.output if args.output.is_absolute() else ROOT / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    except (OSError, json.JSONDecodeError, TypeError, ValueError) as error:
        print(f"robot build report generation failed: {error}", file=sys.stderr)
        return 1
    print(f"robot build report generated: {display_path(output)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
