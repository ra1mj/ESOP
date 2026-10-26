"""Closed-schema, artifact-bound software measurement import regressions."""
import copy
import hashlib
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest

ROOT=pathlib.Path(__file__).resolve().parents[2]
SCRIPT=ROOT/"scripts/import-performance-measurements.py"
MODULE=runpy.run_path(str(SCRIPT))
IMPORT=MODULE["import_measurements"]
EMPTY_HASH=hashlib.sha256(b"").hexdigest()


def histogram(samples=10,value=1234):
    buckets=[0]*512
    overflow=0
    if value//1000<512:
        buckets[value//1000]=samples
    else:
        overflow=samples
    return {"bucket_width_ns":1000,"buckets":buckets,"overflow":overflow,"max_ns":value,"samples":samples}


def measurements():
    return {
        "schema_version":"esop.run-measurements.v1",
        "binding":{"source":"sim","esop_commit":"a"*40,"config_sha256":"b"*64,"topology_sha256":"c"*64,
            "scenario":"Q4","period_ns":1_000_000,"cycles":10,"clock_origin_ns":0},
        "topology":{"slave_count":3,"axis_count":2,"io_channels":16,"pdo_bytes":46,"frame_count":2,"dc_enabled":True},
        "run":{"elapsed_monotonic_ns":500_000,"cycle_samples":10,"max_accumulator_samples":10,"heap_allocations_after_activation":0},
        "latency_ns":{name:histogram() for name in MODULE["LATENCY_KEYS"]},
        "errors":dict.fromkeys(MODULE["ERROR_KEYS"],0),
        "copies":{"tx_bytes_total":640,"rx_bytes_total":640,"copy_spans_total":20},
        "resources":dict.fromkeys(MODULE["RESOURCE_KEYS"]),
        "workload":{"concurrent_sdo_requests":8,"sdo_submitted":8,"sdo_completed":8,"sdo_failed":0,"sdo_slots":8,"sdo_object_bytes":1024},
        "digests":{"wire_sha256":"d"*64,"wire_records":20,"state_sha256":"e"*64,"state_records":10,"event_sha256":EMPTY_HASH,"event_records":0},
        "software_scenario":{"completed":True,"failures":[]},
    }


class ImportTests(unittest.TestCase):
    def test_virtual_port_rtt_and_host_execution_duration_use_distinct_clocks(self):
        value = measurements()
        value["latency_ns"]["rx_round_trip"] = histogram(value=1_000_000)
        value["run"]["elapsed_monotonic_ns"] = 500_000
        result = self.call(value)
        self.assertEqual(result["latency_ns"]["rx_round_trip"]["max"], 1_000_000)
        self.assertFalse(result["qualification"]["passed"])
        value["latency_ns"]["fast_path"] = histogram(value=1_000_000)
        with self.assertRaises(ValueError):
            self.call(value)

    def test_bounded_capture_prefix_retains_full_measurements_without_qualifying_hardware(self):
        value = measurements()
        value["errors"]["capture_lost"] = 1000
        result = self.call(value)
        self.assertTrue(result["software_evidence"]["scenario"]["completed"])
        self.assertEqual(result["errors"]["capture_lost"], 1000)
        self.assertEqual(result["measurement"]["cycle_samples"], 10)
        self.assertFalse(result["qualification"]["passed"])
        self.assertTrue(any("prefix" in item for item in result["qualification"]["failures"]))

    def call(self,value,**extra):
        return IMPORT(value,expected_commit="a"*40,config_sha256="b"*64,topology_sha256="c"*64,source_sha256="f"*64,**extra)

    def test_valid_software_sources_preserve_counts_and_can_never_qualify(self):
        for source in ("sim","pcap_replay","linux_raw"):
            value=measurements();value["binding"]["source"]=source
            result=self.call(value)
            self.assertFalse(result["qualification"]["passed"])
            self.assertEqual(result["measurement"]["source"],source)
            self.assertEqual(result["latency_ns"]["fast_path"]["max"],1234)
            self.assertEqual(result["workload"]["concurrent_sdo_requests"],8)
            self.assertIsNone(result["workload"]["p99_regression_percent"])
            self.assertEqual(result["software_evidence"]["digests"],value["digests"])

    def test_unknown_qualification_fields_hil_claims_and_missing_fields_rejected(self):
        for mutate in (
            lambda value:value.update(passed=True),
            lambda value:value.update(qualification={"passed":True}),
            lambda value:value["binding"].update(source="hil"),
            lambda value:value["software_scenario"].update(hardware_qualified=True),
            lambda value:value["digests"].pop("wire_sha256"),
            lambda value:value["errors"].pop("corrupt"),
        ):
            value=measurements();mutate(value)
            with self.assertRaises(ValueError):self.call(value)

    def test_metadata_histograms_and_counts_cannot_be_tampered(self):
        for mutate in (
            lambda value:value["binding"].update(config_sha256="0"*64),
            lambda value:value["binding"].update(esop_commit="0"*40),
            lambda value:value["binding"].update(topology_sha256="0"*64),
            lambda value:value["run"].update(cycle_samples=9),
            lambda value:value["latency_ns"]["fast_path"].update(samples=9),
            lambda value:value["latency_ns"]["fast_path"]["buckets"].__setitem__(0,1),
            lambda value:value["latency_ns"]["fast_path"].update(max_ns=100),
            lambda value:value["workload"].update(sdo_completed=9),
            lambda value:value["workload"].update(sdo_slots=7),
            lambda value:value["workload"].update(sdo_object_bytes=512),
            lambda value:value["digests"].update(state_records=9),
            lambda value:value["digests"].update(event_sha256="1"*64),
            lambda value:value["run"].update(heap_allocations_after_activation=True),
            lambda value:value["run"].update(heap_allocations_after_activation=1),
        ):
            value=measurements();mutate(value)
            with self.assertRaises(ValueError):self.call(value)

    def test_p99_regression_is_computed_from_actual_compatible_baseline(self):
        value=measurements();value["binding"]["scenario"]="Q4-baseline"
        value["workload"].update(concurrent_sdo_requests=0,sdo_submitted=0,sdo_completed=0)
        baseline=self.call(value)
        loaded=measurements()
        loaded["latency_ns"]["fast_path"]=histogram(value=2468)
        report=self.call(loaded,baseline=baseline,baseline_sha256="1"*64)
        self.assertEqual(report["workload"]["baseline_fast_path_p99_ns"],1234)
        self.assertEqual(report["workload"]["p99_regression_percent"],100)
        self.assertFalse(report["qualification"]["passed"])
        baseline["software"]["config_hash"]="2"*64
        with self.assertRaisesRegex(ValueError,"baseline metadata"):
            self.call(loaded,baseline=baseline,baseline_sha256="1"*64)

    def test_cli_binds_real_artifact_bytes_and_topology_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)
            config=root/"config.json";config.write_text('{"product":"fixture"}')
            value=measurements()
            topology=root/"topology.json"
            topology.write_text(json.dumps({"schema_version":"esop.software-topology.v1","scenario":"Q4",
                "period_ns":1_000_000,"topology":value["topology"]}))
            value["binding"].update(config_sha256=MODULE["file_hash"](config),topology_sha256=MODULE["file_hash"](topology))
            source=root/"run.json";source.write_text(json.dumps(value))
            output=root/"performance.json"
            command=[sys.executable,str(SCRIPT),"--input",str(source),"--config",str(config),"--topology",str(topology),
                "--expected-commit","a"*40,"--output",str(output)]
            result=subprocess.run(command,capture_output=True,text=True,check=False)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertFalse(json.loads(output.read_text())["qualification"]["passed"])
            value["topology"]["axis_count"]=3;source.write_text(json.dumps(value))
            result=subprocess.run(command,capture_output=True,text=True,check=False)
            self.assertNotEqual(result.returncode,0)
            value["topology"]["axis_count"]=2;source.write_text(json.dumps(value))
            config.write_text('{"product":"tampered"}')
            result=subprocess.run(command,capture_output=True,text=True,check=False)
            self.assertNotEqual(result.returncode,0)

    def test_duplicate_json_and_nonfinite_numbers_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/"bad.json"
            for content in ('{"source":"sim","source":"hil"}', '{"value":NaN}'):
                path.write_text(content)
                with self.assertRaises(ValueError):MODULE["strict_json"](path)


if __name__=="__main__":unittest.main()
