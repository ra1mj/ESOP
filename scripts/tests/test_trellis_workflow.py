"""Regression tests for session isolation and the actual per-turn hook."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / ".trellis/scripts"
sys.path.insert(0, str(SCRIPTS))
try:
    from common.active_task import clear_active_task, resolve_active_task, set_active_task
finally:
    sys.path.pop(0)


class TrellisWorkflowTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        # Never inherit the developer's live chat identity or hook overrides.
        self.env = {"PYTHONPATH": str(SCRIPTS), "PYTHONDONTWRITEBYTECODE": "1"}
        environment = patch.dict(os.environ, self.env, clear=True)
        environment.start()
        self.addCleanup(environment.stop)
        self.sessions = self.root / ".trellis/.runtime/sessions"
        self.sessions.mkdir(parents=True)
        self.task = ".trellis/tasks/current"
        self.other_task = ".trellis/tasks/other"
        self.write_task(self.task)
        self.write_task(self.other_task)

    def write_task(self, task, status="planning"):
        path = self.root / task / "task.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"id": Path(task).name, "status": status}), encoding="utf-8")

    def bind(self, task, session="current"):
        active = set_active_task(task, self.root, {"session_id": session}, "codex")
        self.assertIsNotNone(active)
        return active

    def resolve(self, session="current"):
        return resolve_active_task(self.root, {"session_id": session}, "codex")

    def test_known_unbound_session_never_borrows_another_task(self):
        self.bind(self.other_task, "other")
        active = self.resolve()
        self.assertIsNone(active.task_path)
        self.assertEqual(active.source_type, "none")
        self.assertEqual(active.context_key, "codex_current")

    def test_native_environment_identity_is_also_isolated(self):
        self.bind(self.other_task, "other")
        for key in ("CODEX_THREAD_ID", "CODEX_SESSION_ID", "TRELLIS_CONTEXT_ID"):
            with self.subTest(key=key), patch.dict(os.environ, {key: "current"}):
                self.assertIsNone(resolve_active_task(self.root).task_path)

    def test_invalid_or_empty_own_pointer_never_borrows_another_task(self):
        self.bind(self.other_task, "other")
        own_path = self.sessions / "codex_current.json"
        for payload in ("{", "[]", "{}", '{"current_task": ""}', '{"current_task": null}'):
            with self.subTest(payload=payload):
                own_path.write_text(payload, encoding="utf-8")
                active = self.resolve()
                self.assertIsNone(active.task_path)
                self.assertEqual(active.context_key, "codex_current")

    def test_binding_only_changes_own_pointer_not_task_phase(self):
        self.bind(self.other_task, "other")
        other_path = self.sessions / "codex_other.json"
        previous_other = other_path.read_bytes()
        task_path = self.root / self.task / "task.json"
        previous_task = task_path.read_bytes()
        expected = self.bind(self.task)
        self.assertEqual(self.resolve(), expected)
        self.assertEqual(expected.source, "session:codex_current")
        self.assertEqual(task_path.read_bytes(), previous_task)
        self.assertEqual(other_path.read_bytes(), previous_other)

    def test_clearing_own_pointer_does_not_resurrect_another_task(self):
        self.bind(self.other_task, "other")
        own = self.bind(self.task)
        previous = clear_active_task(self.root, {"session_id": "current"}, "codex")
        self.assertEqual(previous, own)
        self.assertIsNone(self.resolve().task_path)
        self.assertFalse((self.sessions / "codex_current.json").exists())
        self.assertEqual(self.resolve("other").task_path, self.other_task)

    def test_stale_own_pointer_remains_visible_without_fallback(self):
        self.bind(self.other_task, "other")
        self.bind(self.task)
        (self.root / self.task / "task.json").unlink()
        (self.root / self.task).rmdir()
        active = self.resolve()
        self.assertEqual(active.task_path, self.task)
        self.assertEqual(active.source, "session:codex_current")
        self.assertTrue(active.stale)

    def test_identityless_fallback_requires_exactly_one_session(self):
        self.assertIsNone(resolve_active_task(self.root, platform="codex").task_path)
        self.bind(self.other_task, "other")
        active = resolve_active_task(self.root, platform="codex")
        self.assertEqual(active.task_path, self.other_task)
        self.assertEqual(active.source, "session-fallback:codex_other")
        self.bind(self.task)
        self.assertIsNone(resolve_active_task(self.root, platform="codex").task_path)

    def hook(self, prompt="Continue"):
        workflow = self.root / ".trellis/workflow.md"
        workflow.write_bytes((ROOT / ".trellis/workflow.md").read_bytes())
        result = subprocess.run(
            [sys.executable, str(ROOT / ".codex/hooks/inject-workflow-state.py")],
            input=json.dumps({"cwd": str(self.root), "session_id": "current", "prompt": prompt}),
            text=True, capture_output=True, cwd=self.root, env=self.env, timeout=5,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        output = json.loads(result.stdout)["hookSpecificOutput"]
        self.assertEqual(output["hookEventName"], "UserPromptSubmit")
        return output["additionalContext"]

    def test_hook_reports_no_task_for_known_unbound_session(self):
        self.bind(self.other_task, "other")
        context = self.hook()
        self.assertIn("Status: no_task", context)
        self.assertNotIn("Task: other", context)
        self.assertFalse((self.sessions / "codex_current.json").exists())

    def test_hook_routes_current_phase_and_dispatch_mode(self):
        self.bind(self.other_task, "other")
        self.bind(self.task)
        config = self.root / ".trellis/config.yaml"
        for mode in ("inline", "sub-agent"):
            config.write_text(f"codex:\n  dispatch_mode: {mode}\n", encoding="utf-8")
            for phase in ("planning", "in_progress"):
                with self.subTest(mode=mode, phase=phase):
                    self.write_task(self.task, phase)
                    context = self.hook()
                    self.assertIn(f"Task: current ({phase})", context)
                    self.assertIn(f"<codex-mode>{mode}:", context)
                    self.assertNotIn("Task: other", context)
                    # The hook must inject the selected source-of-truth block.
                    tag = phase + ("-inline" if mode == "inline" else "")
                    workflow = (ROOT / ".trellis/workflow.md").read_text(encoding="utf-8")
                    body = workflow.split(f"[workflow-state:{tag}]\n", 1)[1]
                    body = body.split(f"[/workflow-state:{tag}]", 1)[0].strip()
                    self.assertIn(body, context)
                    self.assertNotIn("Refer to workflow.md for current step.", context)

    def test_hook_never_turns_prompt_text_into_task_approval(self):
        self.bind(self.task)
        before = {p: p.read_bytes() for p in self.sessions.glob("*.json")}
        task_path = self.root / self.task / "task.json"
        previous_task = task_path.read_bytes()
        for prompt in ("请开始实现", "yes", "Automatic goal continuation", "修复工作流卡顿"):
            with self.subTest(prompt=prompt):
                self.assertIn("Task: current (planning)", self.hook(prompt))
                self.assertEqual(task_path.read_bytes(), previous_task)
                self.assertEqual({p: p.read_bytes() for p in self.sessions.glob("*.json")}, before)


if __name__ == "__main__":
    unittest.main()
