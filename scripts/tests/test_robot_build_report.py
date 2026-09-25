"""Regression tests for generated product build-report integration."""

import copy
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
generator_module = runpy.run_path(
    str(ROOT / "scripts/generate-robot-build-report.py")
)
build_report = generator_module["build_report"]
validate_product_input = generator_module["validate_product_input"]
validate_report = runpy.run_path(
    str(ROOT / "scripts/validate-robot-build-report.py")
)["validate_report"]


def product_input() -> dict:
    return {
        "schema_version": "esop.product-build-input.v1",
        "config_sha256": "1" * 64,
        "platform": {
            "board": "host-simulator",
            "soc": "generic-x86_64",
            "port": "deterministic-linux-simulator",
            "rtos_or_kernel": "host Linux; not target qualified",
            "cache_policy": "host managed; target cache qualification pending",
        },
        "devices": {
            "declared_slaves": 3,
            "declared_axes": 2,
            "declared_io_channels": 16,
            "source": "esop-cfggen validated product manifest and ESI subset",
        },
        "process_data": {
            "pdo_bytes_per_cycle": 36,
            "frame_count": 2,
            "expected_wkc": 6,
            "copy_bytes_per_cycle": 20,
            "wire_bytes_per_cycle": 180,
        },
        "cycle_budget": {
            "period_ns": 1_000_000,
            "deadline_ns": 800_000,
            "qualification": "generated_not_measured",
        },
        "resources": {
            "procbuf_bytes": 4144,
            "dma_bytes": None,
            "rt_stack_peak_bytes": None,
            "text_rodata_bytes": None,
        },
        "qualification": {
            "scenario": "generated-simulator-product",
            "passed": False,
            "failures": [
                "generated topology is not physical HIL evidence",
                "cycle deadline and target resource use are not measured",
            ],
        },
    }


class RobotBuildReportTests(unittest.TestCase):
    def assert_product_rejected(self, value, reason):
        with self.assertRaisesRegex(ValueError, reason):
            validate_product_input(value)

    def test_default_report_stays_host_unqualified(self):
        report = build_report()
        validate_report(report)
        self.assertEqual(report["software"]["config_hash"], generator_module["config_hash"]())
        self.assertEqual(report["platform"]["board"], "host-development")
        self.assertEqual(report["devices"]["declared_slaves"], 0)
        self.assertFalse(report["qualification"]["passed"])

    def test_product_input_projects_exact_generated_evidence(self):
        source = product_input()
        report = build_report(source)
        validate_report(report)

        self.assertEqual(report["software"]["config_hash"], source["config_sha256"])
        self.assertEqual(report["platform"], source["platform"])
        self.assertEqual(report["devices"], source["devices"])
        self.assertEqual(report["process_data"], source["process_data"])
        self.assertEqual(report["cycle_budget"], source["cycle_budget"])
        self.assertEqual(report["resources"]["procbuf_bytes"], 4144)
        self.assertFalse(report["qualification"]["passed"])

    def test_product_input_schema_and_values_fail_closed(self):
        cases = []

        unknown = product_input()
        unknown["unexpected"] = True
        cases.append((unknown, "unknown keys"))

        missing = product_input()
        del missing["process_data"]["expected_wkc"]
        cases.append((missing, "missing keys"))

        forged = product_input()
        forged["qualification"] = {
            "scenario": "generated-simulator-product",
            "passed": True,
            "failures": [],
        }
        cases.append((forged, "must remain unqualified"))

        bad_hash = product_input()
        bad_hash["config_sha256"] = "not-a-hash"
        cases.append((bad_hash, "lowercase SHA-256"))

        bad_budget = product_input()
        bad_budget["cycle_budget"]["deadline_ns"] = 1_000_001
        cases.append((bad_budget, "must not exceed"))

        boolean_metric = product_input()
        boolean_metric["process_data"]["frame_count"] = True
        cases.append((boolean_metric, "non-negative integer"))

        platform_extra = product_input()
        platform_extra["platform"]["dma_bytes"] = None
        cases.append((platform_extra, "unknown keys"))

        for value, reason in cases:
            with self.subTest(reason=reason):
                self.assert_product_rejected(value, reason)

        invalid_wire_report = build_report(product_input())
        invalid_wire_report["process_data"]["wire_bytes_per_cycle"] = True
        with self.assertRaisesRegex(ValueError, "wire_bytes_per_cycle"):
            validate_report(invalid_wire_report)

    def test_build_report_does_not_mutate_product_input(self):
        source = product_input()
        expected = copy.deepcopy(source)
        build_report(source)
        self.assertEqual(source, expected)

    def test_cli_accepts_generated_input_and_rejects_forgery(self):
        script = ROOT / "scripts/generate-robot-build-report.py"
        with tempfile.TemporaryDirectory() as directory:
            directory = pathlib.Path(directory)
            input_path = directory / "product.json"
            output_path = directory / "report.json"
            input_path.write_text(json.dumps(product_input()), encoding="utf-8")

            accepted = subprocess.run(
                [
                    sys.executable,
                    str(script),
                    "--product-input",
                    str(input_path),
                    "--output",
                    str(output_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(accepted.returncode, 0, accepted.stderr)
            validate_report(json.loads(output_path.read_text(encoding="utf-8")))

            forged = product_input()
            forged["qualification"]["passed"] = True
            input_path.write_text(json.dumps(forged), encoding="utf-8")
            rejected = subprocess.run(
                [
                    sys.executable,
                    str(script),
                    "--product-input",
                    str(input_path),
                    "--output",
                    str(output_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("must remain unqualified", rejected.stderr)


if __name__ == "__main__":
    unittest.main()
