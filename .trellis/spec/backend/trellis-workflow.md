# Local Trellis Workflow Contract

## Scope / Trigger

Apply when changing `.trellis/scripts/common/active_task.py`, platform workflow
hooks, `.trellis/workflow.md`, or the start/continue/brainstorm entry skills.
The workflow file owns routing policy; hooks render its state blocks. Keep
phase walkthroughs and entry skills consistent with those blocks.

## Signatures

- `resolve_active_task(repo_root, platform_input=None, platform=None) -> ActiveTask`
- `set_active_task(task_path, repo_root, platform_input=None, platform=None) -> ActiveTask | None`
- `clear_active_task(repo_root, platform_input=None, platform=None) -> ActiveTask`
- `python3 .codex/hooks/inject-workflow-state.py` reads hook JSON from stdin.
- `make workflow-check` runs the session and hook regression suite; `make ci`
  includes this gate.

## Contracts

- Identity comes from hook input, `TRELLIS_CONTEXT_ID`, or platform environment
  such as `CODEX_SESSION_ID` / `CODEX_THREAD_ID`. Pointers live in
  `.trellis/.runtime/sessions/<context_key>.json` under `current_task`.
- A known identity resolves only its own pointer. Identity-less readers retain
  the single-session fallback; writers still require identity.
- Binding or clearing a pointer does not change `task.json.status`. Use binding
  for established session recovery, not `task.py start`, which also sets
  `in_progress`. Preserve other sessions and task artifacts during recovery.
- The Codex hook returns `hookSpecificOutput` with `hookEventName` equal to
  `UserPromptSubmit` and `additionalContext` containing the current state block.
  Inline mode selects the `-inline` block when available. The hook is read-only,
  reads the workflow file on every invocation, and never parses approval words.
- Authorization persists in conversation context and recorded task decisions.
  Ready planning plus an existing request to implement can advance through
  Phase 1.4. Planning-only requests, explicit review requirements, and unresolved
  scope decisions remain boundaries. Automatic wakeups create no new consent.
- Phase 3.4 applies the same authorization rule to scoped commits and remote
  delivery. An existing GitHub delivery request can cover those actions;
  explicit review/manual-commit constraints still win. Merely committing or
  pushing does not authorize a merge or release. Unrelated dirty paths stay
  untouched and do not force a new approval for the known batch.
- Archive/journal bookkeeping honors `session_auto_commit: false` and ignore
  policy. Do not force-add local records to satisfy a fixed number of commits.
- Small explicit fixes can use the direct path with development guidelines and
  quality checks. A workflow repair does not activate an unrelated larger task.
- Keep template hashes unchanged after local customization so future Trellis
  updates can identify local edits.

## Validation & Error Matrix

| Identity / pointer | Expected resolution |
| --- | --- |
| Known / valid own pointer | Own task, `source_type=session` |
| Known / missing, empty, or malformed own pointer | No task, own context key retained |
| Known / own task directory removed | Own reference with `stale=True` |
| Unknown / exactly one runtime session file with a task reference | `session-fallback` (including stale references) |
| Unknown / zero or multiple session files, or sole file without a valid reference | No task |

## Good / Base / Bad Cases

- Good: chat A is unbound and chat B owns a planning task; A's hook says
  `Status: no_task`. Finishing A's own task later also leaves A unbound.
- Base: A is bound to its planning task; the hook reports planning. After an
  authorized lifecycle transition, it reports `in_progress` on the next call.
- Bad: A silently borrows B's task, or a hook changes phase because a prompt
  contains `yes`, an implementation request, or an automatic continuation.

## Tests Required

`scripts/tests/test_trellis_workflow.py` uses isolated temporary repositories
and a cleared environment. Assert isolation through payload and native env
identity, pointer clearing, malformed/stale state, and identity-less ambiguity.
Run the actual hook subprocess to check state selection, both Codex dispatch
modes, source-of-truth block injection, and no task/runtime mutation from prompts.

Review policy text separately: tests verify routing and state isolation, not
the model's interpretation of user intent. Check workflow blocks, Phase 1.4,
and all three entry skills for unconditional interview or repeated approval
rules when changing authorization policy. Include both in-progress blocks and
Phase 3.4 so the same approval loop cannot reappear during delivery.

## Wrong vs Correct

- Wrong: when a known chat has no pointer, fall through to the sole other
  session. Correct: return no active task for that chat immediately.
- Wrong: inject "stay in planning" on every turn and reopen authorization.
  Correct: show remaining work and an explicit path to activation using
  existing authorization after artifacts are ready.
