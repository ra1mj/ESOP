# Implementation Plan

1. Inspect and preserve the current Zenoh projector and command adapter behavior; identify all public exports and tests that must remain compatible.
2. Add the optional `payloads` feature and dependencies to `esop-ipc`.
3. Implement bounded robot identity/configuration, shared ProcBuf projector, Protobuf encoding, quality summary helpers, and state/event `IpcFrame` construction.
4. Implement command-frame construction helpers for tests/producers, structural decoding, authenticated-source validation, and `CommandIngress` delegation.
5. Refactor the Zenoh projector and runtime command decoder to delegate to the shared module without changing transport or policy behavior.
6. Add focused unit tests for projection, message bounds, identity mismatches, stale/non-finite state, command decoding, and pre-ingress rejection behavior.
7. Add real Unix datagram integration tests for state, event, valid command admission, and inconsistent command rejection.
8. Update Cargo/workspace metadata, capability manifest, product plan, IPC documentation, and applicable Trellis specs.
9. Run formatting, focused crate tests, dependency/layer checks, and full `make ci`.
10. Commit, push to `ra1mj/ESOP`, monitor GitHub Actions, archive the task, and record the developer journal entry.
