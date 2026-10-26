#!/usr/bin/env python3
"""Import bounded software-run measurements without granting HIL qualification."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import pathlib
import re
import runpy
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
VALIDATE_REPORT = runpy.run_path(str(ROOT / "scripts/validate-performance-report.py"))["validate_report"]
SCHEMA = "esop.run-measurements.v1"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
U64_MAX = 2**64 - 1
SOURCES = {"sim", "pcap_replay", "linux_raw"}
ERROR_KEYS = {
    "deadline_miss", "wkc_mismatch", "frame_timeout", "rx_overflow", "tx_starvation",
    "unmatched", "corrupt", "consumer_rejections", "link_down", "mailbox_failures",
    "dc_failures", "al_failures", "i2c_nack", "i2c_timeout", "capture_lost", "rx_budget_exhausted",
}
RESOURCE_KEYS = {"core_arena_bytes", "procbuf_bytes", "dma_bytes", "rt_stack_peak_bytes", "text_rodata_bytes", "rt_cpu_percent"}
LATENCY_KEYS = {"release_jitter_abs", "fast_path", "rx_round_trip"}


def closed(value, keys, path):
    if not isinstance(value, dict):
        raise ValueError(f"{path} must be an object")
    missing, extra = set(keys) - value.keys(), value.keys() - set(keys)
    if missing or extra:
        raise ValueError(f"{path} fields mismatch: missing={sorted(missing)}, unknown={sorted(extra)}")
    return value


def integer(value, path, minimum=0):
    if type(value) is not int or not minimum <= value <= U64_MAX:
        raise ValueError(f"{path} must be an integer in [{minimum}, {U64_MAX}]")
    return value


def digest(value, path, pattern=HEX64):
    if not isinstance(value, str) or not pattern.fullmatch(value):
        raise ValueError(f"{path} must be a lowercase hexadecimal digest")
    return value


def strict_json(path):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result
    return json.loads(pathlib.Path(path).read_text(encoding="utf-8"), object_pairs_hook=pairs,
        parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"non-finite JSON number: {value}")))


def file_hash(path):
    result = hashlib.sha256()
    with pathlib.Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            result.update(block)
    return result.hexdigest()


def histogram_stats(value, expected_samples, path):
    closed(value, {"bucket_width_ns", "buckets", "overflow", "max_ns", "samples"}, path)
    width = integer(value["bucket_width_ns"], f"{path}.bucket_width_ns", 1)
    if width != 1000 or not isinstance(value["buckets"], list) or len(value["buckets"]) != 512:
        raise ValueError(f"{path} requires 512 fixed 1000 ns buckets")
    buckets = [integer(count, f"{path}.buckets[{index}]") for index, count in enumerate(value["buckets"])]
    overflow = integer(value["overflow"], f"{path}.overflow")
    maximum = integer(value["max_ns"], f"{path}.max_ns")
    samples = integer(value["samples"], f"{path}.samples")
    if samples != expected_samples or sum(buckets) + overflow != samples:
        raise ValueError(f"{path} histogram sample counts do not match the run")
    if samples == 0:
        if maximum != 0:
            raise ValueError(f"{path} empty histogram cannot have a measured maximum")
        return dict.fromkeys(("p50", "p99", "p999", "max"), 0)
    if any(count and index * width > maximum for index, count in enumerate(buckets)):
        raise ValueError(f"{path} nonempty bucket exceeds its measured max")
    if overflow and maximum < len(buckets) * width:
        raise ValueError(f"{path} overflow is inconsistent with measured max")
    if not overflow and maximum >= len(buckets) * width:
        raise ValueError(f"{path} max requires an overflow sample")
    if not overflow and buckets[maximum // width] == 0:
        raise ValueError(f"{path} measured max must belong to a populated bucket")
    def quantile(per_mille):
        rank = max(1, (samples * per_mille + 999) // 1000)
        accumulated = 0
        for index, count in enumerate(buckets):
            accumulated += count
            if accumulated >= rank:
                return min(maximum, (index + 1) * width - 1)
        return maximum
    return {"p50": quantile(500), "p99": quantile(990), "p999": quantile(999), "max": maximum}


def validate_measurements(value, expected_commit, config_sha256, topology_sha256):
    closed(value, {"schema_version", "binding", "topology", "run", "latency_ns", "errors", "copies",
        "resources", "workload", "digests", "software_scenario"}, "measurements")
    if value["schema_version"] != SCHEMA:
        raise ValueError(f"schema_version must be {SCHEMA}")
    binding = closed(value["binding"], {"source", "esop_commit", "config_sha256", "topology_sha256", "scenario",
        "period_ns", "cycles", "clock_origin_ns"}, "binding")
    if binding["source"] not in SOURCES:
        raise ValueError("binding.source must be sim, pcap_replay or linux_raw; HIL imports are unsupported")
    digest(binding["esop_commit"], "binding.esop_commit", HEX40)
    digest(binding["config_sha256"], "binding.config_sha256")
    digest(binding["topology_sha256"], "binding.topology_sha256")
    if (binding["esop_commit"] != expected_commit or binding["config_sha256"] != config_sha256
        or binding["topology_sha256"] != topology_sha256):
        raise ValueError("measurement commit/config/topology metadata does not match the supplied artifacts")
    if not isinstance(binding["scenario"], str) or not binding["scenario"].strip():
        raise ValueError("binding.scenario must be nonempty")
    cycles = integer(binding["cycles"], "binding.cycles", 1)
    integer(binding["period_ns"], "binding.period_ns", 1)
    integer(binding["clock_origin_ns"], "binding.clock_origin_ns")
    topology = closed(value["topology"], {"slave_count", "axis_count", "io_channels", "pdo_bytes", "frame_count", "dc_enabled"}, "topology")
    for key in topology.keys() - {"dc_enabled"}:
        integer(topology[key], f"topology.{key}", 1 if key in {"slave_count", "pdo_bytes", "frame_count"} else 0)
    if type(topology["dc_enabled"]) is not bool or topology["axis_count"] > 32:
        raise ValueError("topology requires boolean DC policy and at most 32 axes")
    run = value["run"]
    if not isinstance(run, dict):
        raise ValueError("run must be an object")
    run_keys = {"elapsed_monotonic_ns", "cycle_samples", "max_accumulator_samples", "heap_allocations_after_activation"}
    if "rx_round_trip_samples" in run:
        run_keys.add("rx_round_trip_samples")
    closed(run, run_keys, "run")
    for key, number in run.items():
        integer(number, f"run.{key}", 1 if key == "elapsed_monotonic_ns" else 0)
    if run["cycle_samples"] != cycles or run["max_accumulator_samples"] != cycles:
        raise ValueError("run sample counts must equal binding.cycles")
    round_trip_samples = run.get("rx_round_trip_samples", cycles)
    if round_trip_samples > cycles:
        raise ValueError("RX round-trip sample count cannot exceed cycle count")
    closed(value["latency_ns"], LATENCY_KEYS, "latency_ns")
    stats = {key: histogram_stats(value["latency_ns"][key],
        round_trip_samples if key == "rx_round_trip" else cycles,
        f"latency_ns.{key}") for key in LATENCY_KEYS}
    # Port time is virtual in simulation/replay and can advance faster than
    # wall time; only fast-path samples share the host elapsed clock.
    if stats["fast_path"]["max"] > run["elapsed_monotonic_ns"]:
        raise ValueError("host fast-path maximum cannot exceed the measured host duration")
    for section, keys in (("errors", ERROR_KEYS), ("copies", {"tx_bytes_total", "rx_bytes_total", "copy_spans_total"}),
        ("workload", {"concurrent_sdo_requests", "sdo_submitted", "sdo_completed", "sdo_failed", "sdo_slots", "sdo_object_bytes"})):
        for key, number in closed(value[section], keys, section).items():
            integer(number, f"{section}.{key}")
    if value["errors"]["deadline_miss"] > cycles or value["errors"]["link_down"] > cycles:
        raise ValueError("per-cycle error counts cannot exceed cycle count")
    workload = value["workload"]
    if (workload["sdo_completed"] + workload["sdo_failed"] > workload["sdo_submitted"]
        or workload["concurrent_sdo_requests"] > min(workload["sdo_submitted"], workload["sdo_slots"])):
        raise ValueError("SDO counts or concurrent capacity are inconsistent")
    for key, number in closed(value["resources"], RESOURCE_KEYS, "resources").items():
        if number is None:
            continue
        if key == "rt_cpu_percent":
            if type(number) not in (int, float) or not math.isfinite(number) or not 0 <= number <= 100:
                raise ValueError("resources.rt_cpu_percent must be finite and within [0, 100]")
        else:
            integer(number, f"resources.{key}")
    evidence = closed(value["digests"], {"wire_sha256", "wire_records", "state_sha256", "state_records", "event_sha256", "event_records"}, "digests")
    for kind in ("wire", "state", "event"):
        digest(evidence[f"{kind}_sha256"], f"digests.{kind}_sha256")
        records = integer(evidence[f"{kind}_records"], f"digests.{kind}_records")
        if records == 0 and evidence[f"{kind}_sha256"] != hashlib.sha256(b"").hexdigest():
            raise ValueError(f"digests.{kind} empty stream digest mismatch")
    if evidence["wire_records"] < cycles or evidence["state_records"] != cycles:
        raise ValueError("wire/state evidence counts must cover every recorded cycle")
    scenario = closed(value["software_scenario"], {"completed", "failures"}, "software_scenario")
    if type(scenario["completed"]) is not bool or not isinstance(scenario["failures"], list) or not all(
        isinstance(failure, str) and failure for failure in scenario["failures"]):
        raise ValueError("software_scenario requires completed bool and failure strings")
    if scenario["completed"] and scenario["failures"]:
        raise ValueError("a completed software scenario cannot contain failures")
    if scenario["completed"] and run["heap_allocations_after_activation"] != 0:
        raise ValueError("a completed software scenario requires allocation-free activation")
    if binding["scenario"] == "Q4" and scenario["completed"] and (
        workload["concurrent_sdo_requests"] < 8 or workload["sdo_completed"] < 8 or workload["sdo_object_bytes"] < 1024):
        raise ValueError("completed Q4 requires eight concurrent completed SDO requests with 1 KiB capacity")
    return stats


def import_measurements(value, *, expected_commit, config_sha256, topology_sha256, source_sha256, baseline=None, baseline_sha256=None):
    digest(expected_commit, "expected_commit", HEX40)
    digest(config_sha256, "expected config SHA-256")
    digest(topology_sha256, "expected topology SHA-256")
    digest(source_sha256, "source SHA-256")
    stats = validate_measurements(value, expected_commit, config_sha256, topology_sha256)
    binding, run, copies, workload = value["binding"], value["run"], value["copies"], value["workload"]
    baseline_p99, regression = None, None
    if baseline is not None:
        VALIDATE_REPORT(baseline)
        digest(baseline_sha256, "baseline SHA-256")
        evidence = baseline.get("software_evidence")
        if not isinstance(evidence, dict):
            raise ValueError("baseline requires completed software evidence")
        scenario = closed(evidence.get("scenario"), {"completed", "failures"}, "baseline software scenario")
        sdo = closed(evidence.get("sdo"), set(workload), "baseline SDO workload")
        if (scenario["completed"] is not True or scenario["failures"] != []
            or baseline["measurement"]["heap_allocations_after_activation"] != 0):
            raise ValueError("baseline must be a completed, allocation-free software run")
        for key in ("concurrent_sdo_requests", "sdo_submitted", "sdo_completed", "sdo_failed"):
            if integer(sdo[key], f"baseline SDO workload.{key}") != 0:
                raise ValueError("baseline must have no submitted SDO workload")
        if (baseline["software"]["esop_commit"] != expected_commit or baseline["software"]["config_hash"] != config_sha256
            or baseline["topology"]["topology_manifest_hash"] != topology_sha256 or baseline["run"]["period_ns"] != binding["period_ns"]
            or baseline["measurement"]["source"] != binding["source"] or baseline["workload"]["concurrent_sdo_requests"] != 0
            or {key:baseline["topology"][key] for key in value["topology"]} != value["topology"]
            or baseline["run"]["cycles"] == 0 or baseline["measurement"]["cycle_samples"] != baseline["run"]["cycles"]
            or baseline["measurement"]["max_accumulator_samples"] != baseline["run"]["cycles"]
            or baseline["measurement"]["trace_sha256"] is None
            or baseline["measurement"].get("histogram_bucket_width_ns") != 1000):
            raise ValueError("baseline metadata/period/source/cadence does not match, or baseline is not an unloaded run")
        baseline_p99 = baseline["latency_ns"]["fast_path"]["p99"]
        if baseline_p99 <= 0:
            raise ValueError("baseline fast-path p99 must be measured and positive")
        regression = max(0, (stats["fast_path"]["p99"] / baseline_p99 - 1) * 100)
    failures = [f"{binding['source']} software evidence cannot qualify HIL, target WCET or physical DC accuracy"]
    if value["errors"]["capture_lost"]:
        failures.append("bounded capture retained only a prefix; full streaming counters/digests remain available")
    failures.extend(value["software_scenario"]["failures"])
    if not value["software_scenario"]["completed"]:
        failures.append("software scenario did not complete")
    if binding["scenario"] == "Q4" and baseline is None:
        failures.append("Q4 unloaded-run p99 baseline was not supplied")
    report = {
        "schema_version": "esop.performance.v1", "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "software": {"esop_commit": expected_commit, "config_hash": config_sha256, "compiler": "unrecorded hosted run compiler", "cflags": []},
        "platform": {"board": "host-development", "soc": "unknown", "clock_hz": 0, "port": binding["source"],
            "rtos_or_kernel": "not-qualified", "cache_policy": "host-managed; target qualification pending"},
        "topology": dict(value["topology"], topology_manifest_hash=topology_sha256),
        "run": {"period_ns": binding["period_ns"], "duration_s": run["elapsed_monotonic_ns"] // 1_000_000_000,
            "cycles": binding["cycles"], "temperature_c_min": 0, "temperature_c_max": 0},
        "latency_ns": stats, "errors": value["errors"],
        "copies": {key: copies[total] // binding["cycles"] for key, total in (
            ("tx_bytes_per_cycle", "tx_bytes_total"), ("rx_bytes_per_cycle", "rx_bytes_total"), ("copy_spans_per_cycle", "copy_spans_total"))},
        "resources": value["resources"],
        "measurement": {"source": binding["source"], "cycle_samples": run["cycle_samples"], "max_accumulator_samples": run["max_accumulator_samples"],
            "heap_allocations_after_activation": run["heap_allocations_after_activation"], "trace_sha256": source_sha256,
            "rx_round_trip_samples": run.get("rx_round_trip_samples"),
            "rx_round_trip_unavailable_samples": (binding["cycles"] - run["rx_round_trip_samples"]
                if "rx_round_trip_samples" in run else None),
            "rx_round_trip_coverage": ("observed samples" if "rx_round_trip_samples" in run
                else "legacy coverage unrecorded; histogram may include zero fill"),
            "histogram_bucket_width_ns": 1000, "percentiles": "bucket upper bounds clamped to observed max", "time_basis": {"release_jitter_abs": "port_monotonic", "fast_path": "host_monotonic", "rx_round_trip": "observed_port_poll_after_tx"}, "copy_measurement_semantics": "observed byte volume and payload spans; not full memcpy or DMA instrumentation"},
        "workload": {"concurrent_sdo_requests": workload["concurrent_sdo_requests"], "p99_regression_percent": regression,
            "baseline_report_sha256": baseline_sha256, "baseline_fast_path_p99_ns": baseline_p99},
        "software_evidence": {"binding": binding, "digests": value["digests"], "copies": copies, "sdo": workload,
            "scenario": value["software_scenario"]},
        "qualification": {"scenario": binding["scenario"], "passed": False, "failures": failures},
    }
    VALIDATE_REPORT(report)
    return report


def validate_topology_artifact(artifact, measurements):
    closed(artifact, {"schema_version", "scenario", "period_ns", "topology"}, "topology artifact")
    if artifact["schema_version"] != "esop.software-topology.v1":
        raise ValueError("topology artifact must use esop.software-topology.v1")
    scenario = measurements["binding"]["scenario"]
    topology_scenario = "Q4" if scenario in ("Q4", "Q4-baseline") else scenario
    if (artifact["scenario"] != topology_scenario
        or artifact["period_ns"] != measurements["binding"]["period_ns"]
        or artifact["topology"] != measurements["topology"]):
        raise ValueError("topology artifact scenario/period/counts do not match measurements")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("input", "config", "topology", "output"):
        parser.add_argument(f"--{name}", type=pathlib.Path, required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--baseline", type=pathlib.Path)
    args = parser.parse_args()
    try:
        measurements = strict_json(args.input)
        validate_topology_artifact(strict_json(args.topology), measurements)
        report = import_measurements(measurements, expected_commit=args.expected_commit,
            config_sha256=file_hash(args.config), topology_sha256=file_hash(args.topology), source_sha256=file_hash(args.input),
            baseline=strict_json(args.baseline) if args.baseline else None,
            baseline_sha256=file_hash(args.baseline) if args.baseline else None)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    except (OSError, UnicodeError, ValueError, TypeError, KeyError, OverflowError) as error:
        print(f"software measurements invalid: {error}", file=sys.stderr)
        return 1
    print(f"software measurements imported, hardware qualification remains false: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
