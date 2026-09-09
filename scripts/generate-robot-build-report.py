#!/usr/bin/env python3
"""Generate the evidence envelope for one ESOP workspace build."""

import argparse
import hashlib
import json
import pathlib
import platform
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


def release_artifact_bytes() -> int:
    release = ROOT / "target" / "release"
    if not release.is_dir():
        return 0
    return sum(path.stat().st_size for path in release.rglob("*") if path.is_file())


def package_names() -> list[str]:
    metadata = json.loads(command("cargo", "metadata", "--no-deps", "--format-version", "1"))
    return sorted(package["name"] for package in metadata["packages"])


def build_report() -> dict:
    return {
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    output = args.output if args.output.is_absolute() else ROOT / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(build_report(), indent=2) + "\n", encoding="utf-8")
    print(f"robot build report generated: {output.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
