"""Regression tests for fail-closed performance qualification."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
validate_report = runpy.run_path(str(ROOT / "scripts/validate-performance-report.py"))[
    "validate_report"
]
performance_report = runpy.run_path(str(ROOT / "scripts/generate-performance-report.py"))[
    "performance_report"
]


class PerformanceReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.baseline = performance_report()

    def qualified(self, scenario="Q2"):
        report = copy.deepcopy(self.baseline)
        specs = {
            "Q1": (1_000_000, 32, 1024, 2),
            "Q2": (500_000, 16, 512, 1),
            "Q3": (250_000, 8, 256, 1),
            "Q4": (1_000_000, 16, 768, 2),
        }
        period, axes, pdo_bytes, frames = specs[scenario]
        report["platform"].update({
            "board": "test-board",
            "soc": "test-soc",
            "port": "test-dma-port",
            "clock_hz": 100_000_000,
            "rtos_or_kernel": "test-rtos-v1",
            "cache_policy": "explicit DMA cache maintenance",
        })
        report["topology"].update({
            "slave_count": axes,
            "axis_count": axes,
            "io_channels": 8 if scenario in ("Q1", "Q4") else 0,
            "pdo_bytes": pdo_bytes,
            "frame_count": frames,
            "dc_enabled": scenario != "Q3",
            "topology_manifest_hash": "1" * 64,
        })
        report["run"].update({
            "period_ns": period,
            "duration_s": 1800,
            "cycles": 1_800_000_000_000 // period,
        })
        for stats in report["latency_ns"].values():
            stats.update({"p50": 100, "p99": 200, "p999": 300, "max": 400})
        report["copies"].update({"tx_bytes_per_cycle": 100, "rx_bytes_per_cycle": 100})
        report["resources"].update({key: 10 for key in report["resources"]})
        report["measurement"].update({
            "source": "hil",
            "cycle_samples": report["run"]["cycles"],
            "max_accumulator_samples": report["run"]["cycles"],
            "heap_allocations_after_activation": 0,
            "trace_sha256": "2" * 64,
        })
        report["qualification"] = {"scenario": scenario, "passed": True, "failures": []}
        if scenario == "Q4":
            report["latency_ns"]["fast_path"]["p99"] = 220
            report["workload"].update({
                "concurrent_sdo_requests": 8,
                "p99_regression_percent": 10,
                "baseline_report_sha256": "3" * 64,
                "baseline_fast_path_p99_ns": 200,
            })
        return report

    def assert_rejected(self, report, reason):
        with self.assertRaisesRegex(ValueError, reason):
            validate_report(report)

    def test_generated_baseline_is_unqualified_but_well_formed(self):
        validate_report(copy.deepcopy(self.baseline))
        report = copy.deepcopy(self.baseline)
        report["qualification"]["passed"] = True
        self.assert_rejected(report, "known Q1/Q2/Q3/Q4 scenario")
        report["qualification"]["passed"] = False
        report["qualification"]["failures"] = []
        self.assert_rejected(report, "requires a reason")

    def test_scenario_boundaries_have_a_valid_control_case(self):
        for scenario in ("Q1", "Q2", "Q3", "Q4"):
            with self.subTest(scenario=scenario):
                validate_report(self.qualified(scenario))

    def test_threshold_equality_is_accepted(self):
        thresholds = {
            "Q1": (2_000, 10_000, 50_000, 250_000, 50),
            "Q2": (1_000, 5_000, 25_000, 125_000, 60),
            "Q3": (1_000, 3_000, 15_000, 75_000, 70),
            "Q4": (2_000, 11_000, 55_000, 275_000, 55),
        }
        for scenario, (p50, p99, maximum, fast_p99, cpu) in thresholds.items():
            with self.subTest(scenario=scenario):
                report = self.qualified(scenario)
                report["latency_ns"]["release_jitter_abs"].update({
                    "p50": p50, "p99": p99, "p999": p99, "max": maximum,
                })
                report["latency_ns"]["fast_path"].update({
                    "p99": fast_p99, "p999": fast_p99, "max": fast_p99,
                })
                report["resources"]["rt_cpu_percent"] = cpu
                if scenario == "Q4":
                    report["workload"]["baseline_fast_path_p99_ns"] = 250_000
                validate_report(report)

    def test_missing_required_evidence_never_qualifies(self):
        for parent, key in (
            ("software", "config_hash"),
            ("run", "cycles"),
            ("latency_ns", "fast_path"),
            ("errors", "wkc_mismatch"),
            ("measurement", "max_accumulator_samples"),
        ):
            with self.subTest(field=f"{parent}.{key}"):
                report = self.qualified()
                del report[parent][key]
                self.assert_rejected(report, "missing key")
        report = self.qualified()
        del report["latency_ns"]["fast_path"]["max"]
        self.assert_rejected(report, "missing key")

    def test_duration_and_cycle_count_are_consistent(self):
        for field, value in (
            ("period_ns", 0), ("duration_s", 0), ("cycles", 0),
            ("cycles", 3_599_999), ("cycles", 3_602_001),
        ):
            with self.subTest(field=field, value=value):
                report = self.qualified()
                report["run"][field] = value
                self.assert_rejected(report, "period|1800 seconds|cover")

    def test_topology_platform_and_dc_must_match_scenario(self):
        for section, key, value in (
            ("platform", "board", "host-development"),
            ("platform", "port", "linux-raw"),
            ("platform", "clock_hz", 0),
            ("topology", "slave_count", 25),
            ("topology", "axis_count", 1),
            ("topology", "pdo_bytes", 513),
            ("topology", "dc_enabled", False),
            ("topology", "topology_manifest_hash", self.baseline["software"]["config_hash"]),
        ):
            with self.subTest(section=section, key=key):
                report = self.qualified()
                report[section][key] = value
                self.assert_rejected(report, "platform|topology|DC")
        report = self.qualified("Q1")
        report["topology"]["frame_count"] = 3
        self.assert_rejected(report, "topology")
        report = self.qualified("Q1")
        report["topology"]["io_channels"] = 0
        self.assert_rejected(report, "Q1 requires")
        report = self.qualified("Q3")
        report["topology"]["frame_count"] = 2
        self.assert_rejected(report, "topology")

    def test_latency_stats_and_thresholds_are_fail_closed(self):
        for section, key, value in (
            ("release_jitter_abs", "p99", 5_001),
            ("release_jitter_abs", "max", 25_001),
            ("fast_path", "p99", 125_001),
            ("fast_path", "max", 500_001),
            ("fast_path", "p99", float("nan")),
            ("fast_path", "p99", True),
            ("fast_path", "max", 10 ** 400),
        ):
            with self.subTest(section=section, key=key, value=value):
                report = self.qualified()
                report["latency_ns"][section][key] = value
                self.assert_rejected(report, "finite|percentiles|threshold")
        report = self.qualified()
        report["latency_ns"]["rx_round_trip"]["p999"] = 401
        self.assert_rejected(report, "percentiles")
        report = self.qualified()
        report["latency_ns"]["fast_path"] = dict.fromkeys(("p50", "p99", "p999", "max"), 0)
        self.assert_rejected(report, "nonzero measured latency maxima")

    def test_runtime_errors_and_resources_block_qualification(self):
        for key in self.qualified()["errors"]:
            with self.subTest(error=key):
                report = self.qualified()
                report["errors"][key] = 1
                self.assert_rejected(report, "zero runtime errors")
        for key, value in (
            ("rt_cpu_percent", 61), ("dma_bytes", None), ("rt_stack_peak_bytes", 0),
        ):
            with self.subTest(resource=key):
                report = self.qualified()
                report["resources"][key] = value
                self.assert_rejected(report, "resources")
        report = self.qualified()
        report["errors"]["deadline_miss"] = False
        self.assert_rejected(report, "non-negative integer")

    def test_cycle_provenance_and_q4_baseline_are_required(self):
        for key, value in (
            ("source", "not_measured"),
            ("cycle_samples", 1),
            ("max_accumulator_samples", 1),
            ("heap_allocations_after_activation", 1),
            ("trace_sha256", None),
        ):
            with self.subTest(field=key):
                report = self.qualified()
                report["measurement"][key] = value
                self.assert_rejected(report, "HIL evidence")
        for section, key, value in (
            ("topology", "io_channels", 0),
            ("workload", "concurrent_sdo_requests", 7),
            ("workload", "p99_regression_percent", 10.1),
            ("workload", "baseline_report_sha256", None),
            ("workload", "baseline_fast_path_p99_ns", None),
        ):
            with self.subTest(section=section, key=key):
                report = self.qualified("Q4")
                report[section][key] = value
                self.assert_rejected(report, "Q4 requires")
        report = self.qualified("Q4")
        report["workload"]["p99_regression_percent"] = 5
        self.assert_rejected(report, "does not match")

    def test_cli_rejects_a_forged_pass_and_accepts_the_unqualified_baseline(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "report.json"
            forged = dict(self.baseline, qualification={
                "scenario": "Q2", "passed": True, "failures": [],
            })
            for report, expected in ((self.baseline, 0), (forged, 1)):
                path.write_text(json.dumps(report), encoding="utf-8")
                result = subprocess.run(
                    [sys.executable, str(ROOT / "scripts/validate-performance-report.py"),
                     str(path)],
                    capture_output=True, text=True, check=False,
                )
                self.assertEqual(result.returncode, expected, result.stderr)


if __name__ == "__main__":
    unittest.main()
