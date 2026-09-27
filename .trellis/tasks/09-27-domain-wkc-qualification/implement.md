# Implementation plan

## 1. Load project contracts

- Read the task artifacts, backend spec index, relevant error/quality conventions, and
  cross-layer guide.
- Inspect exact scheduled-Domain receive routing and existing tests before editing.

## 2. Add engine evidence callback

- Define the typed WKC mismatch observation value.
- Extend `RxDatagramConsumer` with the default observer and forwarding implementations.
- Emit the callback only from the exact WKC mismatch branch.
- Add engine callback-boundary tests.

## 3. Track Domain mismatch episodes

- Add transient and persistent Domain fields and quality projection.
- Attribute actual WKC without accepting or committing rejected payload.
- Implement one-per-due-cycle increment, saturation, reset, and non-due retention.
- Add direct Domain tests.

## 4. Route scheduled mismatches

- Extend the sealed scheduled-Domain interface.
- Route by the fixed index-owner table and preserve concrete Domain checks.
- Add multi-Domain ownership and foreign-index tests.

## 5. Expose ProcBuf diagnostics

- Reuse the ABI v6 reserved field for the counter and bump the semantic ABI to v7.
- Update lifecycle projection and affected fixtures.
- Assert stable record size/offsets, regenerated layout hash, old-version rejection,
  and lifecycle debounce coverage.

## 6. Update evidence and contracts

- Update software PRD and EtherCAT master requirements.
- Add or refine capability-manifest evidence without claiming hardware qualification.
- Record the cross-layer WKC attribution contract in Trellis specs.

## 7. Verify and deliver

- Run formatting and focused tests during implementation.
- Run Trellis quality review and full `make ci`.
- Complete acceptance checkboxes only from passing evidence.
- Commit implementation, archive the task, record the session, push `main`, and verify
  GitHub Actions for the exact final SHA.
