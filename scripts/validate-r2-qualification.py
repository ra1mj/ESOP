#!/usr/bin/env python3
"""Validate the fail-closed ESOP R2 product qualification evidence set."""

import argparse
import hashlib
import json
import pathlib
import re
import runpy
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = "esop.r2-qualification.v1"
RESULT_SCHEMA = "esop.r2-qualification-result.v1"
TOPOLOGY_SCHEMA = "esop.hil-topology.v1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
REQUIRED_REVIEWS = {"safety_boundary", "license", "source_audit"}
REQUIRED_MODES = {"csp", "csv", "cst"}
DEFAULT_STOP_ACTIONS = {"quick_stop", "disable"}
STOP_ACTIONS = DEFAULT_STOP_ACTIONS | {"hold", "ramp_to_zero"}
PLACEHOLDERS = {
    "",
    "unknown",
    "not-qualified",
    "not-qualified-development",
    "unqualified-development",
    "pending",
    "placeholder",
}
SCENARIOS = {
    "Q1": {
        "period_ns": 1_000_000,
        "axis_count": 32,
        "max_slaves": 32,
        "max_pdo_bytes": 1024,
        "max_frames": 2,
    },
    "Q2": {
        "period_ns": 500_000,
        "axis_count": 16,
        "max_slaves": 24,
        "max_pdo_bytes": 512,
        "max_frames": None,
    },
}
BUILD_VALIDATE = runpy.run_path(str(ROOT / "scripts/validate-robot-build-report.py"))[
    "validate_report"
]
PERFORMANCE_VALIDATE = runpy.run_path(
    str(ROOT / "scripts/validate-performance-report.py")
)["validate_report"]


def fail(message: str) -> None:
    raise ValueError(message)


def require(mapping: dict, key: str, path: str):
    if not isinstance(mapping, dict):
        fail(f"{path} must be an object")
    if key not in mapping:
        fail(f"{path} missing key: {key}")
    return mapping[key]


def require_list(mapping: dict, key: str, path: str) -> list:
    value = require(mapping, key, path)
    if not isinstance(value, list):
        fail(f"{path}.{key} must be a list")
    return value


def require_string(mapping: dict, key: str, path: str) -> str:
    value = require(mapping, key, path)
    if not isinstance(value, str) or not value:
        fail(f"{path}.{key} must be a non-empty string")
    return value


def require_integer(mapping: dict, key: str, path: str, maximum: int | None = None) -> int:
    value = require(mapping, key, path)
    if type(value) is not int or value < 0 or (maximum is not None and value > maximum):
        suffix = f" no greater than {maximum}" if maximum is not None else ""
        fail(f"{path}.{key} must be a non-negative integer{suffix}")
    return value


def qualification_status(mapping: dict, path: str) -> tuple[bool, list[str]]:
    passed = require(mapping, "passed", path)
    failures = require_list(mapping, "failures", path)
    if not isinstance(passed, bool):
        fail(f"{path}.passed must be a boolean")
    if not all(isinstance(reason, str) and reason for reason in failures):
        fail(f"{path}.failures must contain non-empty strings")
    if passed and failures:
        fail(f"{path}: a passed claim cannot contain failures")
    if not passed and not failures:
        fail(f"{path}: an unqualified claim requires at least one failure")
    return passed, failures


def is_placeholder(value: str) -> bool:
    normalized = value.strip().lower()
    return normalized in PLACEHOLDERS or "pending" in normalized


def file_sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class EvidenceResolver:
    def __init__(self, root: pathlib.Path):
        self.root = root.resolve()
        self.verified = 0

    def resolve(self, reference: dict, path: str, required: bool) -> pathlib.Path | None:
        if not isinstance(reference, dict):
            fail(f"{path} must be an object")
        relative = require(reference, "path", path)
        expected_hash = require(reference, "sha256", path)
        if relative is None and expected_hash is None:
            if required:
                fail(f"{path} requires an evidence path and SHA-256")
            return None
        if not isinstance(relative, str) or not relative:
            fail(f"{path}.path must be a non-empty string or null")
        if not isinstance(expected_hash, str) or not HEX64.fullmatch(expected_hash):
            fail(f"{path}.sha256 must be a lowercase SHA-256 or null")
        pure = pathlib.PurePosixPath(relative)
        if pure.is_absolute() or ".." in pure.parts or "\\" in relative:
            fail(f"{path}.path escapes repository: {relative}")
        candidate = (self.root / pathlib.Path(*pure.parts)).resolve()
        if not candidate.is_relative_to(self.root) or not candidate.is_file():
            fail(f"{path}.path is not a repository file: {relative}")
        actual_hash = file_sha256(candidate)
        if actual_hash != expected_hash:
            fail(f"{path} SHA-256 mismatch for {relative}")
        self.verified += 1
        return candidate


def load_json(path: pathlib.Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"{label} is not valid JSON: {error}")
    if not isinstance(value, dict):
        fail(f"{label} must contain a JSON object")
    return value


def validate_device(device: dict, path: str, strict: bool) -> dict:
    device_id = require_string(device, "device_id", path)
    role = require_string(device, "role", path)
    if role not in {"drive", "io"}:
        fail(f"{path}.role must be drive or io")
    position = require_integer(device, "position", path, 65_535)
    vendor_id = require_integer(device, "vendor_id", path, 0xFFFF_FFFF)
    product_code = require_integer(device, "product_code", path, 0xFFFF_FFFF)
    revision = require_integer(device, "revision", path, 0xFFFF_FFFF)
    serial = require_string(device, "serial", path)
    firmware = require_string(device, "firmware", path)
    axis_count = require_integer(device, "axis_count", path, 32)
    io_channels = require_integer(device, "io_channels", path, 65_535)
    qualified = require(device, "qualified", path)
    if not isinstance(qualified, bool):
        fail(f"{path}.qualified must be a boolean")
    if role == "drive" and (axis_count == 0 or io_channels != 0):
        fail(f"{path}: a drive needs axes and cannot claim IO channels")
    if role == "io" and (axis_count != 0 or io_channels == 0):
        fail(f"{path}: an IO device needs channels and cannot claim axes")
    if strict and (
        not qualified
        or vendor_id == 0
        or product_code == 0
        or is_placeholder(device_id)
        or is_placeholder(serial)
        or is_placeholder(firmware)
    ):
        fail(f"{path}: qualified topology devices require immutable non-placeholder identity")
    return {
        "device_id": device_id,
        "position": position,
        "role": role,
        "vendor_id": vendor_id,
        "product_code": product_code,
        "revision": revision,
        "firmware": firmware,
        "axis_count": axis_count,
        "io_channels": io_channels,
    }


def validate_topology(topology: dict, strict: bool) -> dict[str, dict]:
    if require(topology, "schema_version", "topology") != TOPOLOGY_SCHEMA:
        fail(f"topology.schema_version must be {TOPOLOGY_SCHEMA}")
    topology_id = require_string(topology, "topology_id", "topology")
    product_profile = require_string(topology, "product_profile", "topology")
    topology_passed, _ = qualification_status(
        require(topology, "qualification", "topology"), "topology.qualification"
    )
    profiles = require_list(topology, "profiles", "topology")
    if not profiles:
        fail("topology.profiles must not be empty")
    profiles_by_scenario: dict[str, dict] = {}
    for index, profile in enumerate(profiles):
        path = f"topology.profiles[{index}]"
        scenario = require_string(profile, "scenario", path)
        if scenario in profiles_by_scenario:
            fail(f"topology contains duplicate scenario {scenario}")
        period_ns = require_integer(profile, "period_ns", path)
        slave_count = require_integer(profile, "slave_count", path, 65_535)
        axis_count = require_integer(profile, "axis_count", path, 32)
        io_channels = require_integer(profile, "io_channels", path, 65_535)
        pdo_bytes = require_integer(profile, "pdo_bytes", path, 65_535)
        frame_count = require_integer(profile, "frame_count", path, 255)
        dc_enabled = require(profile, "dc_enabled", path)
        if not isinstance(dc_enabled, bool):
            fail(f"{path}.dc_enabled must be a boolean")
        profile_passed, _ = qualification_status(
            require(profile, "qualification", path), f"{path}.qualification"
        )
        devices = require_list(profile, "devices", path)
        checked_devices = [
            validate_device(
                device,
                f"{path}.devices[{device_index}]",
                strict or profile_passed,
            )
            for device_index, device in enumerate(devices)
        ]
        positions = [device["position"] for device in checked_devices]
        device_ids = [device["device_id"] for device in checked_devices]
        if len(set(positions)) != len(positions) or len(set(device_ids)) != len(device_ids):
            fail(f"{path}: device positions and ids must be unique")
        profiles_by_scenario[scenario] = {
            "period_ns": period_ns,
            "slave_count": slave_count,
            "axis_count": axis_count,
            "io_channels": io_channels,
            "pdo_bytes": pdo_bytes,
            "frame_count": frame_count,
            "dc_enabled": dc_enabled,
            "devices": checked_devices,
            "passed": profile_passed,
        }

    if strict:
        if not topology_passed:
            fail("a qualified R2 claim requires a passed topology manifest")
        if is_placeholder(topology_id) or is_placeholder(product_profile):
            fail("a qualified topology requires non-placeholder identity")
        if set(profiles_by_scenario) != set(SCENARIOS):
            fail("a qualified topology requires exactly Q1 and Q2 profiles")
    for scenario, profile in profiles_by_scenario.items():
        if not (strict or profile["passed"]):
            continue
        expected = SCENARIOS.get(scenario)
        if expected is None:
            fail(f"topology has an unsupported qualified scenario: {scenario}")
        if strict and not profile["passed"]:
            fail(f"topology {scenario} profile must be qualified")
        if (
            profile["period_ns"] != expected["period_ns"]
            or profile["axis_count"] != expected["axis_count"]
            or profile["slave_count"] == 0
            or profile["slave_count"] > expected["max_slaves"]
            or profile["pdo_bytes"] == 0
            or profile["pdo_bytes"] > expected["max_pdo_bytes"]
            or profile["frame_count"] == 0
            or not profile["dc_enabled"]
            or profile["io_channels"] == 0
        ):
            fail(f"topology {scenario} profile does not meet the R2 scenario envelope")
        if expected["max_frames"] is not None and profile["frame_count"] > expected["max_frames"]:
            fail(f"topology {scenario} exceeds the frame limit")
        if len(profile["devices"]) != profile["slave_count"]:
            fail(f"topology {scenario} slave_count must match the device list")
        drive_axes = sum(
            device["axis_count"] for device in profile["devices"] if device["role"] == "drive"
        )
        io_channels = sum(
            device["io_channels"] for device in profile["devices"] if device["role"] == "io"
        )
        if drive_axes != profile["axis_count"] or io_channels != profile["io_channels"]:
            fail(f"topology {scenario} device capacities do not match the profile")
    return profiles_by_scenario


def validate_axis_policies(policies: list, expected_axes: int | None) -> set[str]:
    indexes: list[int] = []
    actions: set[str] = set()
    for index, policy in enumerate(policies):
        path = f"axis_stop_policies[{index}]"
        axis_index = require_integer(policy, "axis_index", path, 31)
        mode = require_string(policy, "mode", path)
        action = require_string(policy, "stop_action", path)
        qualified = require(policy, "qualified", path)
        if mode not in REQUIRED_MODES:
            fail(f"{path}.mode must be csp, csv or cst")
        if action not in STOP_ACTIONS:
            fail(f"{path}.stop_action is unsupported")
        if not isinstance(qualified, bool):
            fail(f"{path}.qualified must be a boolean")
        limits = require(policy, "limits", path)
        max_velocity_step = require_integer(limits, "max_velocity_step", f"{path}.limits", 0xFFFF_FFFF)
        max_torque_step = require_integer(limits, "max_torque_step", f"{path}.limits", 0xFFFF)
        require_integer(limits, "max_stationary_velocity", f"{path}.limits", 0xFFFF_FFFF)
        require_integer(limits, "max_zero_torque", f"{path}.limits", 0xFFFF)
        if expected_axes is not None and not qualified:
            fail(f"{path} must be product-qualified")
        if action == "hold" and mode != "csp":
            fail(f"{path}: Hold is CSP-only")
        if action == "ramp_to_zero" and mode not in {"csv", "cst"}:
            fail(f"{path}: RampToZero is CSV/CST-only")
        if action == "ramp_to_zero" and mode == "csv" and max_velocity_step == 0:
            fail(f"{path}: CSV RampToZero requires max_velocity_step")
        if action == "ramp_to_zero" and mode == "cst" and max_torque_step == 0:
            fail(f"{path}: CST RampToZero requires max_torque_step")
        indexes.append(axis_index)
        actions.add(action)
    if expected_axes is not None:
        if len(policies) != expected_axes or sorted(indexes) != list(range(expected_axes)):
            fail("axis_stop_policies must cover every qualified axis exactly once")
    elif len(set(indexes)) != len(indexes):
        fail("axis_stop_policies contains duplicate axis indexes")
    return actions


def validate_combination(
    combination: dict,
    path: str,
    role: str,
    resolver: EvidenceResolver,
    strict: bool,
) -> dict:
    combination_id = require_string(combination, "combination_id", path)
    vendor_id = require_integer(combination, "vendor_id", path, 0xFFFF_FFFF)
    product_code = require_integer(combination, "product_code", path, 0xFFFF_FFFF)
    revision = require_integer(combination, "revision", path, 0xFFFF_FFFF)
    firmware = require_string(combination, "firmware", path)
    qualified = require(combination, "qualified", path)
    if not isinstance(qualified, bool):
        fail(f"{path}.qualified must be a boolean")
    modes = set(require_list(combination, "modes", path))
    stop_actions = set(require_list(combination, "stop_actions", path))
    if not all(isinstance(value, str) for value in modes | stop_actions):
        fail(f"{path}.modes and stop_actions must contain strings")
    if not modes.issubset(REQUIRED_MODES) or not stop_actions.issubset(STOP_ACTIONS):
        fail(f"{path} contains an unsupported mode or stop action")
    io_channels = require_integer(combination, "io_channels", path, 65_535)
    evidence_required = strict or qualified
    evidence = resolver.resolve(
        require(combination, "artifact", path),
        f"{path}.artifact",
        evidence_required,
    )
    if role == "drive" and io_channels != 0:
        fail(f"{path}: drive compatibility cannot claim IO channels")
    if role == "io" and io_channels == 0:
        fail(f"{path}: IO compatibility requires channels")
    if evidence_required and (
        not qualified
        or vendor_id == 0
        or product_code == 0
        or is_placeholder(combination_id)
        or is_placeholder(firmware)
        or evidence is None
    ):
        fail(f"{path}: qualified compatibility requires identity and evidence")
    return {
        "combination_id": combination_id,
        "vendor_id": vendor_id,
        "product_code": product_code,
        "revision": revision,
        "firmware": firmware,
        "modes": modes,
        "stop_actions": stop_actions,
        "io_channels": io_channels,
    }


def validate_compatibility(
    compatibility: dict,
    profiles: dict[str, dict],
    configured_actions: set[str],
    resolver: EvidenceResolver,
    strict: bool,
) -> None:
    drives = [
        validate_combination(entry, f"compatibility.drives[{index}]", "drive", resolver, strict)
        for index, entry in enumerate(require_list(compatibility, "drives", "compatibility"))
    ]
    io_modules = [
        validate_combination(entry, f"compatibility.io_modules[{index}]", "io", resolver, strict)
        for index, entry in enumerate(require_list(compatibility, "io_modules", "compatibility"))
    ]
    if not strict:
        return
    if len({entry["vendor_id"] for entry in drives}) < 2:
        fail("qualified R2 compatibility requires two distinct drive vendors")
    if not io_modules:
        fail("qualified R2 compatibility requires at least one IO module")
    for index, drive in enumerate(drives):
        if drive["modes"] != REQUIRED_MODES:
            fail(f"compatibility.drives[{index}] must qualify CSP, CSV and CST")
        if not (DEFAULT_STOP_ACTIONS | configured_actions).issubset(drive["stop_actions"]):
            fail(f"compatibility.drives[{index}] lacks configured stop-action coverage")
    topology_devices = [
        device for profile in profiles.values() for device in profile["devices"]
    ]
    for role, entries in (("drive", drives), ("io", io_modules)):
        for entry in entries:
            if not any(
                device["role"] == role
                and device["vendor_id"] == entry["vendor_id"]
                and device["product_code"] == entry["product_code"]
                and device["revision"] == entry["revision"]
                and device["firmware"] == entry["firmware"]
                for device in topology_devices
            ):
                fail(f"compatibility {entry['combination_id']} is absent from the HIL topology")


def validate_reviews(reviews: list, resolver: EvidenceResolver, strict: bool) -> None:
    seen: set[str] = set()
    for index, review in enumerate(reviews):
        path = f"reviews[{index}]"
        kind = require_string(review, "kind", path)
        if kind in seen:
            fail(f"duplicate review kind: {kind}")
        seen.add(kind)
        passed = require(review, "passed", path)
        if not isinstance(passed, bool):
            fail(f"{path}.passed must be a boolean")
        artifact = resolver.resolve(
            require(review, "artifact", path),
            f"{path}.artifact",
            strict or passed,
        )
        if strict and (not passed or artifact is None):
            fail(f"{path}: qualified R2 requires a passed review artifact")
    if seen != REQUIRED_REVIEWS:
        fail("reviews must contain safety_boundary, license and source_audit exactly once")


def validate_manifest(
    manifest: dict,
    root: pathlib.Path = ROOT,
    manifest_path: pathlib.Path | None = None,
) -> dict:
    if require(manifest, "schema_version", "manifest") != SCHEMA:
        fail(f"manifest.schema_version must be {SCHEMA}")
    if require(manifest, "release_stage", "manifest") != "R2":
        fail("manifest.release_stage must be R2")
    qualification = require(manifest, "qualification", "manifest")
    passed, failures = qualification_status(qualification, "qualification")

    product = require(manifest, "product", "manifest")
    profile_id = require_string(product, "profile_id", "product")
    model = require_string(product, "model", "product")
    hardware_revision = require_string(product, "hardware_revision", "product")
    tested_commit = require(product, "tested_commit", "product")
    config_hash = require(product, "config_sha256", "product")
    if tested_commit is not None and (
        not isinstance(tested_commit, str) or not HEX40.fullmatch(tested_commit)
    ):
        fail("product.tested_commit must be a lowercase git SHA or null")
    if config_hash is not None and (
        not isinstance(config_hash, str) or not HEX64.fullmatch(config_hash)
    ):
        fail("product.config_sha256 must be a lowercase SHA-256 or null")

    resolver = EvidenceResolver(root)
    artifacts = require(manifest, "artifacts", "manifest")
    topology_path = resolver.resolve(
        require(artifacts, "hil_topology", "artifacts"), "artifacts.hil_topology", True
    )
    limitations_path = resolver.resolve(
        require(artifacts, "known_limitations", "artifacts"),
        "artifacts.known_limitations",
        True,
    )
    assert topology_path is not None and limitations_path is not None
    limitations = limitations_path.read_text(encoding="utf-8")
    for heading in ("## Functional Safety Boundary", "## Unsupported Combinations", "## Residual Risks"):
        if heading not in limitations:
            fail(f"known limitations missing required section: {heading}")
    if passed and "qualification status: **not qualified**" in limitations.lower():
        fail("qualified R2 evidence cannot use limitations that still declare it unqualified")

    topology = load_json(topology_path, "HIL topology")
    topology_claim, _ = qualification_status(
        require(topology, "qualification", "topology"), "topology.qualification"
    )
    profiles = validate_topology(topology, passed or topology_claim)
    max_axes = max((profile["axis_count"] for profile in profiles.values()), default=0)
    policies = require_list(manifest, "axis_stop_policies", "manifest")
    configured_actions = validate_axis_policies(policies, max_axes if passed else None)
    validate_compatibility(
        require(manifest, "compatibility", "manifest"),
        profiles,
        configured_actions,
        resolver,
        passed,
    )
    validate_reviews(require_list(manifest, "reviews", "manifest"), resolver, passed)

    fault_matrix = require(artifacts, "hil_fault_matrix", "artifacts")
    fault_passed = require(fault_matrix, "passed", "artifacts.hil_fault_matrix")
    if not isinstance(fault_passed, bool):
        fail("artifacts.hil_fault_matrix.passed must be a boolean")
    fault_path = resolver.resolve(
        require(fault_matrix, "artifact", "artifacts.hil_fault_matrix"),
        "artifacts.hil_fault_matrix.artifact",
        passed or fault_passed,
    )

    build_reference = require(artifacts, "build_report", "artifacts")
    build_path = resolver.resolve(build_reference, "artifacts.build_report", passed)
    build_report = None
    if build_path is not None:
        build_report = load_json(build_path, "build report")
        BUILD_VALIDATE(build_report)

    performance_reports: dict[str, dict] = {}
    for index, entry in enumerate(require_list(artifacts, "performance_reports", "artifacts")):
        path = f"artifacts.performance_reports[{index}]"
        scenario = require_string(entry, "scenario", path)
        if scenario in performance_reports:
            fail(f"duplicate performance report for {scenario}")
        report_path = resolver.resolve(require(entry, "artifact", path), f"{path}.artifact", True)
        assert report_path is not None
        report = load_json(report_path, f"{scenario} performance report")
        PERFORMANCE_VALIDATE(report)
        performance_reports[scenario] = report

    if not passed:
        return qualification_result(
            manifest,
            manifest_path,
            profile_id,
            tested_commit,
            topology_path,
            resolver.verified,
            False,
            failures,
        )

    if any(is_placeholder(value) for value in (profile_id, model, hardware_revision)):
        fail("a qualified R2 product requires non-placeholder identity")
    if tested_commit is None or config_hash is None:
        fail("a qualified R2 product requires tested_commit and config_sha256")
    approved_by = require(qualification, "approved_by", "qualification")
    approved_at = require(qualification, "approved_at", "qualification")
    if (
        not isinstance(approved_by, str)
        or is_placeholder(approved_by)
        or not isinstance(approved_at, str)
        or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", approved_at)
    ):
        fail("a qualified R2 claim requires approver identity and UTC approval time")
    if not topology_claim:
        fail("a qualified R2 claim requires a qualified HIL topology")
    if not fault_passed or fault_path is None:
        fail("a qualified R2 claim requires a passed HIL fault matrix")
    if build_report is None or build_report["qualification"]["passed"] is not True:
        fail("a qualified R2 claim requires a qualified build report")
    if set(performance_reports) != set(SCENARIOS):
        fail("a qualified R2 claim requires exactly Q1 and Q2 performance reports")

    topology_hash = file_sha256(topology_path)
    for label, report in (("build", build_report), *performance_reports.items()):
        if report["software"]["esop_commit"] != tested_commit:
            fail(f"{label} report commit does not match product.tested_commit")
        if report["software"]["config_hash"] != config_hash:
            fail(f"{label} report config does not match product.config_sha256")
    max_slaves = max(profile["slave_count"] for profile in profiles.values())
    max_axes = max(profile["axis_count"] for profile in profiles.values())
    max_io = max(profile["io_channels"] for profile in profiles.values())
    max_pdo = max(profile["pdo_bytes"] for profile in profiles.values())
    max_frames = max(profile["frame_count"] for profile in profiles.values())
    fastest_period = min(profile["period_ns"] for profile in profiles.values())
    if (
        build_report["devices"]["declared_slaves"] < max_slaves
        or build_report["devices"]["declared_axes"] < max_axes
        or build_report["devices"]["declared_io_channels"] < max_io
        or build_report["process_data"]["pdo_bytes_per_cycle"] < max_pdo
        or build_report["process_data"]["frame_count"] < max_frames
        or build_report["cycle_budget"]["period_ns"] > fastest_period
    ):
        fail("build report does not cover the qualified Q1/Q2 topology envelope")
    for scenario, report in performance_reports.items():
        if report["qualification"]["passed"] is not True:
            fail(f"{scenario} performance report is not qualified")
        if report["qualification"]["scenario"] != scenario:
            fail(f"{scenario} performance report scenario is inconsistent")
        if report["topology"]["topology_manifest_hash"] != topology_hash:
            fail(f"{scenario} performance report topology hash is inconsistent")
        profile = profiles[scenario]
        for key in (
            "slave_count",
            "axis_count",
            "io_channels",
            "pdo_bytes",
            "frame_count",
            "dc_enabled",
        ):
            if report["topology"][key] != profile[key]:
                fail(f"{scenario} performance report topology.{key} is inconsistent")

    return qualification_result(
        manifest,
        manifest_path,
        profile_id,
        tested_commit,
        topology_path,
        resolver.verified,
        True,
        [],
    )


def qualification_result(
    manifest: dict,
    manifest_path: pathlib.Path | None,
    profile_id: str,
    tested_commit: str | None,
    topology_path: pathlib.Path,
    verified_evidence: int,
    qualified: bool,
    failures: list[str],
) -> dict:
    encoded = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode("utf-8")
    manifest_hash = file_sha256(manifest_path) if manifest_path is not None else hashlib.sha256(encoded).hexdigest()
    return {
        "schema_version": RESULT_SCHEMA,
        "release_stage": "R2",
        "profile_id": profile_id,
        "tested_commit": tested_commit,
        "manifest_sha256": manifest_hash,
        "topology_sha256": file_sha256(topology_path),
        "verified_evidence_files": verified_evidence,
        "qualified": qualified,
        "failures": failures,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "manifest",
        nargs="?",
        type=pathlib.Path,
        default=pathlib.Path("qualification/r2_qualification.json"),
    )
    parser.add_argument("--root", type=pathlib.Path, default=ROOT)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--expected-commit")
    args = parser.parse_args()
    root = args.root.resolve()
    manifest_path = args.manifest if args.manifest.is_absolute() else root / args.manifest
    try:
        manifest_path = manifest_path.resolve()
        if not manifest_path.is_relative_to(root) or not manifest_path.is_file():
            fail("manifest must be a file inside the repository root")
        manifest = load_json(manifest_path, "R2 qualification manifest")
        result = validate_manifest(manifest, root, manifest_path)
        if args.expected_commit is not None:
            if not HEX40.fullmatch(args.expected_commit):
                fail("--expected-commit must be a lowercase 40-character git SHA")
            if result["qualified"] and result["tested_commit"] != args.expected_commit:
                fail("qualified R2 evidence does not match --expected-commit")
        if args.output is not None:
            output = args.output if args.output.is_absolute() else root / args.output
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    except (OSError, UnicodeError, TypeError, ValueError, IndexError, KeyError) as error:
        print(f"R2 qualification manifest invalid: {error}", file=sys.stderr)
        return 1
    state = "qualified" if result["qualified"] else "unqualified baseline"
    print(
        f"R2 qualification manifest valid: {state}, "
        f"{result['verified_evidence_files']} evidence file(s) verified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
