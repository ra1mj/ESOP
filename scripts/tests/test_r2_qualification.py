"""Regression tests for the fail-closed R2 qualification evidence gate."""

import copy
import hashlib
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR = ROOT / "scripts/validate-r2-qualification.py"
validate_manifest = runpy.run_path(str(VALIDATOR))["validate_manifest"]


class R2QualificationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.commit = "a" * 40
        self.config_hash = "b" * 64

    def tearDown(self):
        self.temporary.cleanup()

    def write_bytes(self, name: str, content: bytes) -> dict:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return {"path": name, "sha256": hashlib.sha256(content).hexdigest()}

    def write_text(self, name: str, content: str) -> dict:
        return self.write_bytes(name, content.encode("utf-8"))

    def write_json(self, name: str, value: dict) -> dict:
        return self.write_text(name, json.dumps(value, indent=2) + "\n")

    @staticmethod
    def devices(axes_per_drive: int) -> list[dict]:
        return [
            {
                "device_id": "drive-a",
                "position": 0,
                "role": "drive",
                "vendor_id": 1,
                "product_code": 101,
                "revision": 1,
                "serial": "A-001",
                "firmware": "1.0.0",
                "axis_count": axes_per_drive,
                "io_channels": 0,
                "qualified": True,
            },
            {
                "device_id": "drive-b",
                "position": 1,
                "role": "drive",
                "vendor_id": 2,
                "product_code": 202,
                "revision": 2,
                "serial": "B-001",
                "firmware": "2.0.0",
                "axis_count": axes_per_drive,
                "io_channels": 0,
                "qualified": True,
            },
            {
                "device_id": "io-a",
                "position": 2,
                "role": "io",
                "vendor_id": 3,
                "product_code": 303,
                "revision": 1,
                "serial": "IO-001",
                "firmware": "3.0.0",
                "axis_count": 0,
                "io_channels": 8,
                "qualified": True,
            },
        ]

    def topology(self) -> dict:
        return {
            "schema_version": "esop.hil-topology.v1",
            "topology_id": "product-a-hil",
            "product_profile": "product-a",
            "profiles": [
                {
                    "scenario": "Q1",
                    "period_ns": 1_000_000,
                    "slave_count": 3,
                    "axis_count": 32,
                    "io_channels": 8,
                    "pdo_bytes": 1024,
                    "frame_count": 2,
                    "dc_enabled": True,
                    "devices": self.devices(16),
                    "qualification": {"passed": True, "failures": []},
                },
                {
                    "scenario": "Q2",
                    "period_ns": 500_000,
                    "slave_count": 3,
                    "axis_count": 16,
                    "io_channels": 8,
                    "pdo_bytes": 512,
                    "frame_count": 1,
                    "dc_enabled": True,
                    "devices": self.devices(8),
                    "qualification": {"passed": True, "failures": []},
                },
            ],
            "qualification": {"passed": True, "failures": []},
        }

    def build_report(self) -> dict:
        return {
            "schema_version": "esop.build.v1",
            "software": {
                "esop_commit": self.commit,
                "config_hash": self.config_hash,
                "compiler": "rustc test",
                "packages": ["esop-ethercat-core"],
            },
            "platform": {
                "board": "product-a-board",
                "soc": "product-a-soc",
                "port": "product-a-dma",
                "rtos_or_kernel": "product-rtos-1",
                "cache_policy": "explicit dma cache maintenance",
            },
            "devices": {
                "declared_slaves": 3,
                "declared_axes": 32,
                "declared_io_channels": 8,
                "source": "product-a-hil",
            },
            "process_data": {
                "pdo_bytes_per_cycle": 1024,
                "frame_count": 2,
                "expected_wkc": 6,
                "copy_bytes_per_cycle": 2048,
            },
            "cycle_budget": {
                "period_ns": 500_000,
                "deadline_ns": 450_000,
                "qualification": "qualified",
            },
            "resources": {
                "workspace_release_artifact_bytes": 1024,
                "procbuf_bytes": 4096,
                "dma_bytes": 8192,
                "rt_stack_peak_bytes": 2048,
                "text_rodata_bytes": 65_536,
            },
            "qualification": {"scenario": "target-build", "passed": True, "failures": []},
        }

    def performance_report(self, scenario: str, topology_hash: str) -> dict:
        profile = next(
            profile for profile in self.topology()["profiles"] if profile["scenario"] == scenario
        )
        period_ns = profile["period_ns"]
        cycles = 1_800 * 1_000_000_000 // period_ns
        stats = {"p50": 100, "p99": 200, "p999": 300, "max": 400}
        return {
            "schema_version": "esop.performance.v1",
            "software": {
                "esop_commit": self.commit,
                "config_hash": self.config_hash,
                "compiler": "rustc test",
                "cflags": [],
            },
            "platform": {
                "board": "product-a-board",
                "soc": "product-a-soc",
                "clock_hz": 200_000_000,
                "port": "product-a-dma",
                "rtos_or_kernel": "product-rtos-1",
                "cache_policy": "explicit dma cache maintenance",
            },
            "topology": {
                "slave_count": profile["slave_count"],
                "axis_count": profile["axis_count"],
                "io_channels": profile["io_channels"],
                "pdo_bytes": profile["pdo_bytes"],
                "frame_count": profile["frame_count"],
                "dc_enabled": profile["dc_enabled"],
                "topology_manifest_hash": topology_hash,
            },
            "run": {
                "period_ns": period_ns,
                "duration_s": 1800,
                "cycles": cycles,
                "temperature_c_min": 20,
                "temperature_c_max": 60,
            },
            "latency_ns": {
                "release_jitter_abs": dict(stats),
                "fast_path": dict(stats),
                "rx_round_trip": dict(stats),
            },
            "errors": {
                "deadline_miss": 0,
                "wkc_mismatch": 0,
                "frame_timeout": 0,
                "rx_overflow": 0,
                "tx_starvation": 0,
                "unmatched": 0,
                "corrupt": 0,
            },
            "copies": {
                "tx_bytes_per_cycle": 1024,
                "rx_bytes_per_cycle": 1024,
                "copy_spans_per_cycle": 1,
            },
            "resources": {
                "core_arena_bytes": 4096,
                "procbuf_bytes": 4096,
                "dma_bytes": 8192,
                "rt_stack_peak_bytes": 2048,
                "text_rodata_bytes": 65_536,
                "rt_cpu_percent": 10,
            },
            "measurement": {
                "source": "hil",
                "cycle_samples": cycles,
                "max_accumulator_samples": cycles,
                "heap_allocations_after_activation": 0,
                "trace_sha256": "c" * 64,
            },
            "workload": {
                "concurrent_sdo_requests": 0,
                "p99_regression_percent": None,
                "baseline_report_sha256": None,
                "baseline_fast_path_p99_ns": None,
            },
            "qualification": {"scenario": scenario, "passed": True, "failures": []},
        }

    @staticmethod
    def axis_policy(index: int) -> dict:
        variants = (
            ("csp", "hold", 0, 0),
            ("csv", "ramp_to_zero", 100, 0),
            ("cst", "ramp_to_zero", 0, 10),
            ("csp", "quick_stop", 0, 0),
            ("csp", "disable", 0, 0),
        )
        mode, action, velocity_step, torque_step = variants[index % len(variants)]
        return {
            "axis_index": index,
            "mode": mode,
            "stop_action": action,
            "qualified": True,
            "limits": {
                "max_velocity_step": velocity_step,
                "max_torque_step": torque_step,
                "max_stationary_velocity": 2,
                "max_zero_torque": 1,
            },
        }

    def qualified_manifest(self) -> dict:
        limitations = self.write_text(
            "KNOWN_LIMITATIONS.md",
            "# Limits\n\n## Functional Safety Boundary\ntext\n\n"
            "## Unsupported Combinations\ntext\n\n## Residual Risks\ntext\n",
        )
        topology = self.write_json("hil_topology.json", self.topology())
        topology_hash = topology["sha256"]
        build = self.write_json("build_report.json", self.build_report())
        q1 = self.write_json("q1.json", self.performance_report("Q1", topology_hash))
        q2 = self.write_json("q2.json", self.performance_report("Q2", topology_hash))
        drive_a = self.write_text("drive-a.trace", "drive-a HIL")
        drive_b = self.write_text("drive-b.trace", "drive-b HIL")
        io = self.write_text("io.trace", "io HIL")
        fault = self.write_text("fault-matrix.trace", "fault matrix")
        reviews = {
            kind: self.write_text(f"{kind}.txt", f"approved {kind}")
            for kind in ("safety_boundary", "license", "source_audit")
        }
        stop_actions = ["quick_stop", "disable", "hold", "ramp_to_zero"]
        modes = ["csp", "csv", "cst"]
        return {
            "schema_version": "esop.r2-qualification.v1",
            "release_stage": "R2",
            "product": {
                "profile_id": "product-a",
                "model": "ESOP Product A",
                "hardware_revision": "rev-a",
                "tested_commit": self.commit,
                "config_sha256": self.config_hash,
            },
            "artifacts": {
                "hil_topology": topology,
                "known_limitations": limitations,
                "build_report": build,
                "performance_reports": [
                    {"scenario": "Q1", "artifact": q1},
                    {"scenario": "Q2", "artifact": q2},
                ],
                "hil_fault_matrix": {"passed": True, "artifact": fault},
            },
            "axis_stop_policies": [self.axis_policy(index) for index in range(32)],
            "compatibility": {
                "drives": [
                    {
                        "combination_id": "drive-a-combination",
                        "vendor_id": 1,
                        "product_code": 101,
                        "revision": 1,
                        "firmware": "1.0.0",
                        "modes": modes,
                        "stop_actions": stop_actions,
                        "io_channels": 0,
                        "qualified": True,
                        "artifact": drive_a,
                    },
                    {
                        "combination_id": "drive-b-combination",
                        "vendor_id": 2,
                        "product_code": 202,
                        "revision": 2,
                        "firmware": "2.0.0",
                        "modes": modes,
                        "stop_actions": stop_actions,
                        "io_channels": 0,
                        "qualified": True,
                        "artifact": drive_b,
                    },
                ],
                "io_modules": [
                    {
                        "combination_id": "io-a-combination",
                        "vendor_id": 3,
                        "product_code": 303,
                        "revision": 1,
                        "firmware": "3.0.0",
                        "modes": [],
                        "stop_actions": [],
                        "io_channels": 8,
                        "qualified": True,
                        "artifact": io,
                    }
                ],
            },
            "reviews": [
                {"kind": kind, "passed": True, "artifact": reviews[kind]}
                for kind in ("safety_boundary", "license", "source_audit")
            ],
            "qualification": {
                "passed": True,
                "approved_by": "quality-owner",
                "approved_at": "2026-09-21T12:00:00Z",
                "failures": [],
            },
        }

    def replace_json_artifact(self, reference: dict, name: str, value: dict) -> None:
        reference.update(self.write_json(name, value))

    def assert_rejected(self, manifest: dict, reason: str) -> None:
        with self.assertRaisesRegex(ValueError, reason):
            validate_manifest(manifest, self.root)

    def test_checked_in_baseline_is_valid_and_unqualified(self):
        baseline = json.loads(
            (ROOT / "qualification/r2_qualification.json").read_text(encoding="utf-8")
        )
        result = validate_manifest(
            baseline,
            ROOT,
            ROOT / "qualification/r2_qualification.json",
        )
        self.assertFalse(result["qualified"])
        self.assertGreaterEqual(len(result["failures"]), 5)

        baseline["axis_stop_policies"] = [self.axis_policy(0)]
        baseline["axis_stop_policies"][0]["qualified"] = False
        result = validate_manifest(baseline, ROOT)
        self.assertFalse(result["qualified"])

    def test_complete_synthetic_evidence_reaches_the_qualified_path(self):
        result = validate_manifest(self.qualified_manifest(), self.root)
        self.assertTrue(result["qualified"])
        self.assertEqual(result["failures"], [])
        self.assertGreaterEqual(result["verified_evidence_files"], 12)

    def test_paths_hashes_and_product_identity_fail_closed(self):
        for mutation, reason in (
            (lambda item: item["artifacts"]["hil_topology"].update(path="../escape.json"), "escapes"),
            (lambda item: item["artifacts"]["hil_topology"].update(sha256="0" * 64), "mismatch"),
            (lambda item: item["product"].update(model="not-qualified"), "non-placeholder"),
            (lambda item: item["qualification"].update(approved_at="today"), "UTC approval"),
        ):
            with self.subTest(reason=reason):
                manifest = self.qualified_manifest()
                mutation(manifest)
                self.assert_rejected(manifest, reason)

        manifest = self.qualified_manifest()
        manifest["artifacts"]["known_limitations"] = self.write_text(
            "stale-limitations.md",
            "# Limits\n\nQualification status: **not qualified**.\n\n"
            "## Functional Safety Boundary\ntext\n\n"
            "## Unsupported Combinations\ntext\n\n## Residual Risks\ntext\n",
        )
        self.assert_rejected(manifest, "still declare")

    def test_stop_policy_and_compatibility_coverage_fail_closed(self):
        cases = []
        manifest = self.qualified_manifest()
        manifest["axis_stop_policies"][1]["limits"]["max_velocity_step"] = 0
        cases.append((manifest, "requires max_velocity_step"))
        manifest = self.qualified_manifest()
        manifest["axis_stop_policies"].pop()
        cases.append((manifest, "cover every qualified axis"))
        manifest = self.qualified_manifest()
        manifest["compatibility"]["drives"][1]["vendor_id"] = 1
        cases.append((manifest, "two distinct drive vendors"))
        manifest = self.qualified_manifest()
        manifest["compatibility"]["io_modules"] = []
        cases.append((manifest, "at least one IO module"))
        manifest = self.qualified_manifest()
        manifest["compatibility"]["drives"][0]["modes"] = ["csp", "csv"]
        cases.append((manifest, "CSP, CSV and CST"))
        for manifest, reason in cases:
            with self.subTest(reason=reason):
                self.assert_rejected(manifest, reason)

    def test_reports_topology_and_reviews_must_agree(self):
        manifest = self.qualified_manifest()
        build_ref = manifest["artifacts"]["build_report"]
        build = json.loads((self.root / build_ref["path"]).read_text(encoding="utf-8"))
        build["platform"]["board"] = "host-development"
        self.replace_json_artifact(build_ref, "bad-build.json", build)
        self.assert_rejected(manifest, "non-placeholder platform")

        manifest = self.qualified_manifest()
        q1_entry = manifest["artifacts"]["performance_reports"][0]
        q1 = json.loads((self.root / q1_entry["artifact"]["path"]).read_text(encoding="utf-8"))
        q1["qualification"] = {"scenario": "Q1", "passed": False, "failures": ["failed"]}
        self.replace_json_artifact(q1_entry["artifact"], "bad-q1.json", q1)
        self.assert_rejected(manifest, "performance report is not qualified")

        manifest = self.qualified_manifest()
        q2_entry = manifest["artifacts"]["performance_reports"][1]
        q2 = json.loads((self.root / q2_entry["artifact"]["path"]).read_text(encoding="utf-8"))
        q2["software"]["esop_commit"] = "d" * 40
        self.replace_json_artifact(q2_entry["artifact"], "wrong-commit-q2.json", q2)
        self.assert_rejected(manifest, "commit does not match")

        manifest = self.qualified_manifest()
        manifest["reviews"][0]["passed"] = False
        self.assert_rejected(manifest, "passed review artifact")

    def test_unqualified_manifest_cannot_make_unbacked_subclaims(self):
        manifest = self.qualified_manifest()
        manifest["qualification"] = {
            "passed": False,
            "approved_by": None,
            "approved_at": None,
            "failures": ["release qualification is incomplete"],
        }
        manifest["compatibility"]["drives"][0]["artifact"] = {
            "path": None,
            "sha256": None,
        }
        self.assert_rejected(manifest, "requires an evidence path")

        manifest = self.qualified_manifest()
        manifest["qualification"] = {
            "passed": False,
            "approved_by": None,
            "approved_at": None,
            "failures": ["release qualification is incomplete"],
        }
        manifest["artifacts"]["hil_fault_matrix"]["artifact"] = {
            "path": None,
            "sha256": None,
        }
        self.assert_rejected(manifest, "requires an evidence path")

        manifest = self.qualified_manifest()
        topology_ref = manifest["artifacts"]["hil_topology"]
        topology = json.loads((self.root / topology_ref["path"]).read_text(encoding="utf-8"))
        topology["qualification"] = {
            "passed": False,
            "failures": ["Q2 remains open"],
        }
        topology["profiles"][0]["devices"][0]["qualified"] = False
        self.replace_json_artifact(topology_ref, "partial-topology.json", topology)
        manifest["qualification"] = {
            "passed": False,
            "approved_by": None,
            "approved_at": None,
            "failures": ["release qualification is incomplete"],
        }
        self.assert_rejected(manifest, "immutable non-placeholder identity")

    def test_cli_writes_normalized_result_and_rejects_forged_pass(self):
        output = self.root / "result.json"
        result = subprocess.run(
            [
                sys.executable,
                str(VALIDATOR),
                str(ROOT / "qualification/r2_qualification.json"),
                "--root",
                str(ROOT),
                "--output",
                str(output),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(json.loads(output.read_text(encoding="utf-8"))["qualified"])

        forged = json.loads(
            (ROOT / "qualification/r2_qualification.json").read_text(encoding="utf-8")
        )
        forged["qualification"] = {
            "passed": True,
            "approved_by": "quality-owner",
            "approved_at": "2026-09-21T12:00:00Z",
            "failures": [],
        }
        forged_path = self.root / "forged.json"
        forged_path.write_text(json.dumps(forged), encoding="utf-8")
        result = subprocess.run(
            [sys.executable, str(VALIDATOR), str(forged_path), "--root", str(self.root)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
