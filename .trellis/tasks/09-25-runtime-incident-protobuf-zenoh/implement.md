# Implementation plan

- [x] Add backward-compatible incident/evidence fields to `esop.proto` and
      extend current-versus-frozen reader/writer compatibility tests.
- [x] Add the optional agent dependency to the gateway's `zenoh` feature and a
      host-only projection module with stable mappings and explicit errors.
- [x] Validate every agent identity/window/evidence invariant and preserve the
      exact full-width values while saturating only the legacy value field.
- [x] Add `ZenohGateway::publish_agent_incident` and distinguish adapter errors
      from schema/route/transport failures.
- [x] Tighten query reply incident validation without applying state sequence
      cursor semantics to incidents.
- [x] Add pure projection/rejection tests and replace the hand-built live-router
      incident with the production projection path.
- [x] Update PRD, Protobuf/Zenoh docs, capability manifest and backend quality
      guidance with the cross-layer contract and limitations.
- [x] Run focused default/feature builds, Rust tests/Clippy, proto/capability
      validators, loopback Zenoh tests, formatting, `git diff --check` and
      complete `make ci`.
- [ ] Review compatibility and trust-boundary behavior, commit, push, and track
      the final GitHub Actions run before archive/journal completion.
