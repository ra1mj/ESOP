# Implementation plan

## 1. Core SDO contract

- Add `SdoAccess` and `SdoAccessPolicy` with a fail-closed default.
- Replace raw Complete Access booleans at SDO start call sites.
- Validate authorization/subindex before state mutation and encode bit `0x10`.
- Parse and verify upload response access mode while retaining standard
  initiate-download response behavior.
- Add exact-byte and state-machine unit tests.

## 2. ESI and manifest model

- Parse direct CoE `CompleteAccess` capability for both empty and non-empty XML
  elements.
- Add the optional strict per-slave `coe.complete_access` product contract.
- Make the simulator ESI advertise support and authorize one drive only.
- Reject product enablement for unsupported devices.

## 3. Generated artifacts and identity

- Carry ESI support and product enablement through generator slave records.
- Include both fields in semantic/config hashes and normalized product,
  inventory, robot build input, C, and Rust output.
- Regenerate committed example artifacts and update golden expectations.

## 4. Product runtime

- Add generated slave fields and central validation of `enabled => supported`.
- Expose a typed per-slave SDO access policy from static/activated product data.
- Add anti-tamper and policy lookup tests.
- Keep PDO configuration transfers explicitly single-subindex.

## 5. Integration and regression

- Add a mailbox/master round-trip test for the exact Complete Access request.
- Run focused core, cfggen, product-config, and Linux integration tests.
- Run formatting, generated checks, `no_std`, clippy, and full `make ci`.

## 6. Documentation and delivery

- Update FR-017 / MBX-002 status and capability evidence with a clear
  software-versus-HIL boundary.
- Update the Trellis product configuration contract.
- Complete acceptance checks, commit implementation, archive the task, record
  the journal, push to `ra1mj/ESOP`, and verify the exact final GitHub Actions
  run.
