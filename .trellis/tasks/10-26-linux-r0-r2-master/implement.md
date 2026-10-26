# Implementation and validation

User approved the plan and requested implementation on 2026-10-26.

1. [x] Correct wire/address/RX/control/scan/DC and independent golden tests.
2. [x] Build independent stateful simulator and exact PCAP replay.
3. [x] Complete CoE/mailbox MRL/Emergency/eight-request integration.
4. [x] Add bounded AL/DL monitoring, contextual evidence and virtual I2C.
5. [x] Compose r0_r2 examples, fault scenarios, capture/measurement/report import.
6. [x] Exercise Q1/Q2/Q4 and long-soak entry; map every applicable P0.
7. [ ] Review, full quality gates, update specifications/capabilities/journal.

## Validation

Use ~/.cargo/bin in PATH. Run focused core/product/guard/simulator unit and
integration tests as work lands; Linux-port and full make ci require Linux.
Run cargo fmt --check, clippy -D warnings, no_std checks, Python report tests,
capability/proto validation and deterministic end-to-end sim/replay scenarios.
Keep software/HIL qualification separate, add a require-qualified release gate.

## Risks

The existing echoed response fixtures encode incorrect wire assumptions; update
these with independent expected bytes rather than regenerating from the codec.
Mailbox transport changes affect PDO configuration and reconfiguration and must
share physical ownership. Preserve user-owned untracked paseo.json.
