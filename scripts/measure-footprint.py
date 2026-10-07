#!/usr/bin/env python3
"""Measure linked host artifacts; never infer embedded RAM, flash or WCET."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def binary_measurement(path):
    # Keep any named production probes when measuring a stripped copy. Never
    # mutate the original or assume that ELF inspection qualifies attachment.
    symbols = command("nm", "--defined-only", str(path))
    markers = sorted(set(re.findall(r"\b(esop_(?:zenoh_gateway|linux_raw_port)_\w+_v1)\b", symbols)))
    with tempfile.TemporaryDirectory(prefix="esop-footprint-") as temporary:
        stripped = Path(temporary) / path.name
        shutil.copyfile(path, stripped)
        subprocess.run(
            ["strip", "--strip-all", *(f"--keep-symbol={name}" for name in markers), str(stripped)],
            check=True,
        )
        if markers:
            kept = command("nm", "--defined-only", str(stripped))
            assert all(name in kept.split() for name in markers), "probe marker lost"
        stripped_bytes = stripped.stat().st_size
    section_line = command("size", str(path)).splitlines()[-1].split()
    return {
        "file": path.name,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "file_bytes": path.stat().st_size,
        "stripped_copy_bytes": stripped_bytes,
        "elf_sections_bytes": dict(zip(("text", "data", "bss"), map(int, section_line[:3]))),
        "preserved_markers": markers,
    }


def dependency_measurement(package):
    tree = command("cargo", "tree", "--locked", "-p", package,
                   "--edges", "normal,build", "--prefix", "none", "--format", "{p}")
    dependencies = sorted({
        line.split(" (")[0] for line in tree.splitlines()
        if not line.startswith("esop-")
    })
    return {"external_normal_build_packages": len(dependencies), "packages": dependencies}


def startup_wait_measurement(binary):
    if not Path("/proc/self/status").exists():
        return {"measured": False, "reason": "Linux /proc is required"}
    with tempfile.TemporaryDirectory(prefix="esop-startup-") as temporary, socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        path = Path(temporary) / "zenoh.json"
        path.write_text(json.dumps({
            "mode": "client", "listen": {"endpoints": []},
            "connect": {"endpoints": [f"tcp/127.0.0.1:{listener.getsockname()[1]}"], "timeout_ms": 60000},
            "scouting": {"multicast": {"enabled": False}, "gossip": {"enabled": False}},
        }))
        started = time.monotonic()
        process = subprocess.Popen(
            [str(binary), "--robot", "footprint", "--config", str(path),
             "--timeout-ms", "2000", "status"], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        )
        peak_rss = peak_threads = samples = 0
        try:
            while process.poll() is None:
                if time.monotonic() - started > 5:
                    raise RuntimeError("CLI failed its startup deadline")
                try:
                    status = Path(f"/proc/{process.pid}/status").read_text()
                except FileNotFoundError:
                    continue
                rss = re.search(r"^VmRSS:\s+(\d+)", status, re.M)
                threads = re.search(r"^Threads:\s+(\d+)", status, re.M)
                if rss and threads:
                    samples += 1
                    peak_rss = max(peak_rss, int(rss[1]))
                    peak_threads = max(peak_threads, int(threads[1]))
                time.sleep(0.005)
            _, stderr = process.communicate(timeout=1)
            if process.returncode != 5 or b"session opening timed out" not in stderr:
                raise RuntimeError(f"unexpected startup result: {process.returncode}, {stderr!r}")
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()
            process.stderr.close()
        return {
            "measured": True, "scenario": "waiting for a nonresponsive loopback Zenoh handshake",
            "samples": samples, "sample_interval_ms": 5,
            "peak_sampled_rss_kib": peak_rss, "peak_sampled_threads": peak_threads,
            "elapsed_seconds": round(time.monotonic() - started, 3), "exit_code": process.returncode,
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "build/footprint_report.json")
    args = parser.parse_args()
    metadata = json.loads(command("cargo", "metadata", "--no-deps", "--format-version", "1"))
    release = Path(metadata["target_directory"]) / "release"
    report = {
        "schema": "esop.host-footprint.v1",
        "commit": command("git", "rev-parse", "HEAD"),
        "worktree_dirty": bool(command("git", "status", "--porcelain")),
        "rustc": command("rustc", "-Vv"), "host": os.uname().machine,
        "profile": "release, default features",
        "artifacts": [binary_measurement(release / name) for name in (
            "esop", "esop-cfggen", "examples/wire_round_trip")],
        "dependencies": {package: dependency_measurement(package) for package in (
            "esop-ethercat-core", "esop-product-config", "esop-cli")},
        "cli_startup_wait": startup_wait_measurement(release / "esop"),
        "limits": ["wire_round_trip links only the wire layer plus host std; it is not a full master",
                   "sampled startup RSS is not a steady-state, peak-bound or WCET claim",
                   "embedded flash/RAM/stack, complete distribution and HIL are not measured",
                   "stripped copies are measurement artifacts, not qualified release packages"],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Wrote measured host footprint to {args.output}")


if __name__ == "__main__":
    main()
