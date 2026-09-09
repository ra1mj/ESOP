#!/usr/bin/env python3
"""Validate the evidence-bound capability manifest used by release checks."""

import json
import pathlib
import sys


ALLOWED_STATUSES = {"implemented", "development_hil", "partial", "planned"}
REQUIRED_CAPABILITY_KEYS = {"id", "status", "scope", "evidence", "limitation"}


def fail(message: str) -> None:
    raise ValueError(message)


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[1]
    manifest_path = root / "capability_manifest.json"
    try:
        with manifest_path.open(encoding="utf-8") as manifest_file:
            manifest = json.load(manifest_file)

        if manifest.get("schema_version") != 1:
            fail("schema_version must be 1")
        if manifest.get("product") != "ESOP":
            fail("product must be ESOP")
        if manifest.get("manifest_kind") != "capability":
            fail("manifest_kind must be capability")
        if manifest.get("claim_policy") != "evidence_bound":
            fail("claim_policy must be evidence_bound")

        capabilities = manifest.get("capabilities")
        if not isinstance(capabilities, list) or not capabilities:
            fail("capabilities must be a non-empty list")

        seen_ids = set()
        for index, capability in enumerate(capabilities):
            if not isinstance(capability, dict):
                fail(f"capabilities[{index}] must be an object")
            missing = REQUIRED_CAPABILITY_KEYS - capability.keys()
            if missing:
                fail(f"capabilities[{index}] missing keys: {sorted(missing)}")
            capability_id = capability["id"]
            if not isinstance(capability_id, str) or not capability_id:
                fail(f"capabilities[{index}].id must be a non-empty string")
            if capability_id in seen_ids:
                fail(f"duplicate capability id: {capability_id}")
            seen_ids.add(capability_id)
            if capability["status"] not in ALLOWED_STATUSES:
                fail(f"{capability_id}: unsupported status {capability['status']!r}")
            if not isinstance(capability["scope"], str) or not capability["scope"]:
                fail(f"{capability_id}: scope must be a non-empty string")
            if not isinstance(capability["limitation"], str) or not capability["limitation"]:
                fail(f"{capability_id}: limitation must be a non-empty string")
            evidence = capability["evidence"]
            if not isinstance(evidence, list) or not evidence:
                fail(f"{capability_id}: evidence must be a non-empty list")
            for evidence_path in evidence:
                if not isinstance(evidence_path, str) or not evidence_path:
                    fail(f"{capability_id}: evidence path must be a non-empty string")
                path = pathlib.PurePosixPath(evidence_path)
                if path.is_absolute() or ".." in path.parts:
                    fail(f"{capability_id}: evidence path escapes repository: {evidence_path}")
                if not (root / pathlib.Path(evidence_path)).is_file():
                    fail(f"{capability_id}: evidence file does not exist: {evidence_path}")
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"capability manifest invalid: {error}", file=sys.stderr)
        return 1

    print(f"capability manifest valid: {len(capabilities)} capabilities")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
