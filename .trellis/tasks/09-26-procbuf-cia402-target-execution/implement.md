# Implementation Plan

1. Expose a non-mutating ProcBuf command-record validation entry so the RT
   adapter can fail closed even when called outside `ProcBuf::read_command`.
2. Add fixed per-axis SI scaling/mechanical policy, prepared command, and typed
   preparation errors to the lifecycle ProcBuf adapter under the CiA 402
   feature.
3. Implement deterministic target rounding, conservative limit quantization,
   signed scale/position offset handling, mechanical and command/product cap
   checks, complete permit equality, mode/mask/expiry checks, and staged array
   construction.
4. Add a desired-target EtherCAT submission helper that derives no-target,
   actual-hold, or desired-target behavior from verified current-cycle CiA 402
   feedback, then delegates to `submit_active_frame`.
5. Extend internal motion inputs without changing existing raw-target methods;
   add direct, deadline-checked, scheduled, and unified production-service
   ProcBuf command entry points.
6. Add focused unit tests for policy validation, all modes, inverted scales,
   offsets, rounding, conservative limits, raw overflow, command/product caps,
   mechanical bounds, zero period, mask/mode/permit/expiry failures, and
   preparation non-mutation.
7. Add integration coverage for ProcBuf publish/read, exact permit rearm,
   handshake target suppression, enable-edge actual hold, subsequent desired
   PDO target, TX failure retry, fallback stop behavior, and simulated feedback.
8. Keep existing profile, lifecycle, ProcBuf, Linux-port, IPC, and Zenoh tests
   green; add no-std/all-feature strict Clippy coverage for changed crates.
9. Update README, capability manifest, IPC, lifecycle, software PRD, robotics
   plan, qualification limitations, and backend quality guidance with the new
   software boundary and remaining HIL/WCET/safety limitations.
10. Run formatting, focused tests, strict Clippy, `make ci`, commit and push to
    `ra1mj/ESOP`, monitor GitHub Actions, archive the task, record the journal,
    push final metadata commits, and verify final HEAD is green.
