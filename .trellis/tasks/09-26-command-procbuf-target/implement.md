# Implementation Plan

1. Preserve the existing permit-only decoder/admission APIs and factor their
   internal Protobuf decode/identity checks so the strict target path reuses
   them without duplicate field mapping.
2. Add fixed prepared/admitted command types, retryable publication, and typed target errors
   under `esop-ipc::payloads`.
3. Implement zero-based target coverage, mode, capacity, finite-value, and
   non-negative-limit validation before policy admission.
4. Build `CommandPage` only from the returned `MotionPermit` plus validated
   fixed target arrays; bind the value to destination robot/boot/layout.
5. Add retryable publication that borrows the admitted value and returns the
   ProcBuf publication sequence.
6. Add `policy_version` to `CommandPage`, make its reserved bytes explicit,
   bump ProcBuf ABI v4 to v5, tighten malformed command checks, and add the
   lifecycle ProcBuf-to-permit conversion helper.
7. Expose IPC-frame and Zenoh-payload entry points that delegate to the shared
   strict mapper while retaining existing public behavior.
8. Add focused unit tests for ABI-v4 rejection, CSP/CSV/CST mappings, every
   target error, ingress non-mutation, destination mismatch, and retryable
   publication.
9. Extend real Unix datagram integration coverage through ProcBuf readback and
   lifecycle permit acceptance; keep Zenoh compatibility tests green.
10. Update README, capability manifest, command-gateway/IPC/Zenoh docs,
    software PRD, robotics plan, and backend safety guidance.
11. Run formatting, focused all-feature tests, strict Clippy, dependency/no-std
    checks, Protobuf/capability validators, and full `make ci`.
12. Commit and push to `ra1mj/ESOP`, monitor GitHub Actions, archive the task,
    record the journal entry, push the final metadata commits, and verify the
    final workflow is green.
