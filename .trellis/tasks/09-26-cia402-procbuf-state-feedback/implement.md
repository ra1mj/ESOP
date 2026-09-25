# Implementation Plan: CiA 402 Feedback to ProcBuf State

## 1. Freeze Contracts

- [x] Add `error_code` and axis-quality flag constants to ProcBuf `JointState`.
- [x] Advance ProcBuf ABI to v6 and update ABI mismatch/layout tests.
- [x] Add Protobuf `drive_error_code = 11` and host payload projection.
- [x] Extend compatibility tests so old fixtures decode and new payloads round trip.

Validation:

```bash
cargo test -p esop-procbuf -p esop-ipc
```

Rollback point: revert only the ABI/API contract changes before lifecycle integration begins.

## 2. Implement Bounded Feedback Projection

- [x] Add inverse SI conversion helpers using `Cia402AxisCommandPolicy`.
- [x] Define typed axis feedback errors with axis and source context.
- [x] Decode current PDO inputs into fixed staged `[JointState; AXES]` storage.
- [x] Implement current/stale and optional-field quality semantics.
- [x] Preserve prior State values on stale evidence and assign staged axes only after all axes pass.
- [x] Add unit tests for conversions, quality flags, retention, and transactional failure.

Validation:

```bash
cargo test -p esop-lifecycle-guard
```

Rollback point: projection remains isolated behind a helper and can be removed without changing EtherCAT transport behavior.

## 3. Integrate the Stop Cycle

- [x] Supply immutable axis policies through `StopCycleContext`.
- [x] Track only transport-accepted active/fallback outputs.
- [x] Apply accepted controlwords and current input feedback before State publication.
- [x] Update monotonic and verified DC timestamps with retention semantics.
- [x] Propagate typed feedback failures without partially publishing axes.
- [x] Update all internal context constructors and tests.

Validation:

```bash
cargo test -p esop-lifecycle-guard -p esop-linux-port
```

Rollback point: stop-cycle call site can return to prior State publication while keeping contract/unit work available.

## 4. Add End-to-End Coverage

- [x] Verify simulated PDO feedback reaches ProcBuf and Protobuf.
- [x] Verify stale WKC/domain evidence clears quality and retains values.
- [x] Verify accepted active, accepted fallback stop, and rejected output controlword cases.
- [x] Verify DC timestamp update/retention behavior.
- [x] Cover a drive fault/error-code observation without automatic reset.

Validation:

```bash
cargo test --workspace
```

## 5. Update Product Contracts

- [x] Update README, software PRD, robotics plan, lifecycle/IPC/Zenoh/gateway docs, and capability manifest for ABI v6.
- [x] Add `cia402_procbuf_feedback` as implemented.
- [x] Document v5-to-v6 shared-memory recreation and retained HIL/WCET/scale limitations.
- [x] Search for stale ABI v5 claims and classify historical references before editing.

Validation:

```bash
rg -n "ABI_VERSION|ABI v5|ABI v6|procbuf_abi|cia402_procbuf_feedback" README.md docs capability_manifest.json crates proto
```

## 6. Quality Gate and Delivery

- [x] Run formatting and workspace checks.
- [x] Run the repository's complete local CI-equivalent commands, including BPF/eBPF checks where available.
- [x] Review the diff for unintended ABI, lifecycle, or documentation changes.
- [ ] Update task acceptance checkboxes and Trellis journal.
- [ ] Commit and push to `origin/main`.
- [ ] Monitor the resulting GitHub Actions run to success; fix and repush any failure.

Validation:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```
