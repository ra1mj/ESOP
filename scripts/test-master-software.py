#!/usr/bin/env python3
"""Execute and verify the real software master, without hardware qualification."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def read_json(path: Path) -> dict:
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def validate_case(directory: Path, expected: dict) -> dict:
    scenario = read_json(directory / "scenario.json")
    measurements = read_json(directory / "run.json")
    allocations = read_json(directory / "allocations.json")
    if scenario.get("hardware_qualified") is not False:
        raise ValueError("software scenarios cannot claim hardware qualification")
    if scenario.get("software_scenario_passed") is not True:
        raise ValueError(f"scenario failed: {directory}")
    if scenario.get("startup_ready") is not True or scenario.get("confirmed_stops", 0) < 2:
        raise ValueError("wire startup and confirmed stops were not both proved")
    if scenario.get("final_state") not in (0, 1, 4, 5):
        raise ValueError("the final published lifecycle is not inhibited")
    if any(allocations.get(field) != 0 for field in
           ("allocations", "deallocations", "reallocations")):
        raise ValueError("activated master performed heap operations")
    if measurements["run"]["cycle_samples"] != expected["cycles"]:
        raise ValueError("incomplete full-cycle measurement")
    topology = measurements["topology"]
    for field in ("slave_count", "axis_count"):
        if topology[field] != expected[field]:
            raise ValueError(f"wrong workload {field}: {topology[field]}")
    if topology["io_channels"] < 1 or not topology["dc_enabled"]:
        raise ValueError("the scenario must exercise IO and DC")
    if topology["pdo_bytes"] > expected["max_pdo_bytes"]:
        raise ValueError("workload exceeds its frozen PDO budget")
    if measurements["binding"]["period_ns"] != expected["period_ns"]:
        raise ValueError("wrong workload period")
    if scenario["active_cycles"] == 0 or scenario["stopping_cycles"] == 0:
        raise ValueError("motion and stopping were not both exercised")
    if scenario.get("implicit_rearm") is not False or not scenario.get("explicit_rearm"):
        raise ValueError("explicit fresh-permit recovery was not proved")
    if expected.get("sdo") and (scenario["sdo_completed"] < 16
                                or scenario.get("sdo_verified", 0) < 8):
        raise ValueError("eight real one-KiB download/upload requests were not verified")
    if expected.get("fault") and not scenario.get("injected_fault_observed"):
        raise ValueError("the requested fault was not observed by the lifecycle")
    return scenario


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "build/r0-r2/qualification")
    parser.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    command = [args.cargo, "run", "--quiet", "-p", "esop-ethercat-linux-port",
               "--example", "r0_r2", "--"]
    cases = [
        dict(name=f"full-{mode}", scenario="full", mode=mode, cycles=3000,
             slave_count=3, axis_count=2, period_ns=1_000_000,
             max_pdo_bytes=128, sdo=True)
        for mode in ("csp", "csv", "cst")
    ]
    cases += [
        dict(name="Q1", scenario="Q1", mode="csp", cycles=1000,
             slave_count=32, axis_count=32, period_ns=1_000_000, max_pdo_bytes=1024),
        dict(name="Q2", scenario="Q2", mode="csp", cycles=1000,
             slave_count=17, axis_count=16, period_ns=500_000, max_pdo_bytes=512),
        dict(name="Q4-baseline", scenario="Q4-baseline", mode="csp", cycles=1000,
             slave_count=17, axis_count=16, period_ns=1_000_000, max_pdo_bytes=768),
        dict(name="Q4", scenario="Q4", mode="csp", cycles=6000,
             slave_count=17, axis_count=16, period_ns=1_000_000,
             max_pdo_bytes=768, sdo=True),
    ]
    cases += [dict(name=name, scenario=name, mode="csp", cycles=100,
                   slave_count=3, axis_count=2, period_ns=1_000_000,
                   max_pdo_bytes=128, fault=True)
              for name in ("command_expiry", "dc_fault", "wkc_fault", "link_fault",
                           "drive_fault", "supervisor_restart")]
    evidence = []
    try:
        for case in cases:
            output = args.output / case["name"]
            subprocess.run(command + ["--backend", "sim", "--scenario", case["scenario"],
                           "--mode", case["mode"], "--cycles", str(case["cycles"]),
                           "--output-dir", str(output)], cwd=ROOT, check=True,
                           stdout=subprocess.PIPE, text=True)
            result = validate_case(output, case)
            commit = read_json(output / "run.json")["binding"]["esop_commit"]
            baseline = (["--baseline", str(args.output / "Q4-baseline/performance.json")]
                        if case["scenario"] == "Q4" else [])
            subprocess.run([sys.executable, str(ROOT / "scripts/import-performance-measurements.py"),
                            "--input", str(output / "run.json"), "--config", str(output / "config.json"),
                            "--topology", str(output / "topology.json"),
                            "--expected-commit", commit, "--output", str(output / "performance.json")] + baseline,
                           cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
            if read_json(output / "performance.json")["qualification"]["passed"] is not False:
                raise ValueError("software import granted hardware qualification")
            evidence.append({"case": case, "scenario": result,
                             "run_sha256": hashlib.sha256((output / "run.json").read_bytes()).hexdigest()})
            print(f"software master passed: {case['name']}", flush=True)
        # The same production code must reproduce a complete capture exactly.
        for name in ("full-csp", "link_fault"):
            case = next(case for case in cases if case["name"] == name)
            source = args.output / name
            replay = args.output / f"replay-{name}"
            subprocess.run(command + ["--backend", "replay", "--scenario", case["scenario"],
                           "--mode", case["mode"], "--cycles", str(case["cycles"]),
                           "--pcap", str(source / "trace.pcap"), "--sidecar", str(source / "trace.json"),
                           "--output-dir", str(replay)], cwd=ROOT, check=True,
                           stdout=subprocess.PIPE, text=True)
            result = validate_case(replay, case)
            if result != read_json(source / "scenario.json"):
                raise ValueError(f"capture replay differs: {name}")
            print(f"exact replay passed: {name}", flush=True)
        # A bounded recorder must not change the master or lose full counters.
        case = next(case for case in cases if case["name"] == "command_expiry")
        prefix = args.output / "bounded-prefix"
        subprocess.run(command + ["--backend", "sim", "--scenario", "command_expiry",
                       "--cycles", "100", "--capture-limit", "100", "--output-dir", str(prefix)],
                       cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True)
        if validate_case(prefix, case) != read_json(args.output / "command_expiry/scenario.json"):
            raise ValueError("capture overflow changed master behavior")
        prefix_measurements = read_json(prefix / "run.json")
        if prefix_measurements["errors"]["capture_lost"] == 0 or prefix_measurements["digests"]["wire_records"] <= 100:
            raise ValueError("capture prefix did not retain full-stream loss/count evidence")
        rejected = subprocess.run(command + ["--backend", "replay", "--scenario", "command_expiry",
                                  "--cycles", "100", "--pcap", str(prefix / "trace.pcap"),
                                  "--sidecar", str(prefix / "trace.json"), "--output-dir", str(prefix / "replay")],
                                  cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        if rejected.returncode == 0 or "IncompleteCapture" not in rejected.stderr:
            raise ValueError("incomplete capture was accepted as a full replay")
        # Re-signing a changed local capture must still fail exact TX matching.
        source = args.output / "full-csp"
        changed = args.output / "changed-tx"
        changed.mkdir(exist_ok=True)
        pcap = bytearray((source / "trace.pcap").read_bytes())
        pcap[24 + 16] ^= 1  # First Ethernet destination byte, container remains valid.
        (changed / "trace.pcap").write_bytes(pcap)
        sidecar = read_json(source / "trace.json")
        sidecar["pcap_sha256"] = hashlib.sha256(pcap).hexdigest()
        (changed / "trace.json").write_text(json.dumps(sidecar))
        rejected = subprocess.run(command + ["--backend", "replay", "--scenario", "full",
                                  "--cycles", "3000", "--pcap", str(changed / "trace.pcap"),
                                  "--sidecar", str(changed / "trace.json"), "--output-dir", str(changed / "replay")],
                                  cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        if rejected.returncode == 0 or "TxMismatch" not in rejected.stderr:
            raise ValueError("changed TX was not rejected by the actual runner")
        report = {"schema_version": "esop.software-master-qualification.v1",
                  "software_scenarios_passed": True, "hardware_qualified": False,
                  "cases": evidence}
        (args.output / "qualification.json").write_text(json.dumps(report, indent=2) + "\n")
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"software master qualification failed: {error}", file=sys.stderr)
        return 1
    print(f"software master qualification: {args.output / 'qualification.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
