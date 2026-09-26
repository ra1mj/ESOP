# Design

## Boundaries

The change stays inside `esop-ethercat-core` plus simulations and documentation.
The scanner owns capability discovery, the AL controller owns the protocol
handshake, and Startup owns per-slave policy selection and user-facing fault
context. Production scheduling continues to treat Startup as one bounded
service and requires no new readiness input.

## Scan Contract

Add `ESC_CONFIGURATION = 0x0141` and
`ESC_DEVICE_EMULATION = 0x01`. `ScanController` gains a
`ReadingEscConfiguration` phase between fixed-address assignment and AL Status
read. That action is a one-byte FPRD with expected WKC 1 and the same generation
and deadline rules as every other scan action.

`ScanRecord::device_emulation` is populated only from a successful exact-length
response. Startup copies the value into a fixed `[bool; MAX_SLAVES]` array when
the corresponding identity is accepted. This avoids changing the public
`SlaveRecord` layout used by generated product descriptions while keeping the
capability available after scan completion and the PREOP barrier.

## AL Controller Contract

Preserve `start(request)` as the compatibility path with acknowledgement
disabled. Add a policy-aware start method that accepts the currently observed
`AlStatus` and `AlErrorAcknowledgePolicy`.

The controller adds two phases:

1. `WritingErrorAcknowledge`: write `(fault_state as u16) | 0x0010` to AL
   Control.
2. `ReadingErrorAcknowledge`: poll AL Status and code until Error Indication
   clears.

The first AL error freezes an `AlFaultRecord` containing the original status,
the requested target, and acknowledgement state. Further polls never replace
that evidence. With policy disabled, the first error faults immediately. With
policy enabled, the controller enters the two acknowledgement phases. Clearing
the bit marks acknowledgement complete and then deliberately returns the
original `AlErrorCode`; it never retries the transition. Deadline, generation,
WKC, token, and payload failures use existing errors while preserving the
frozen fault record.

## Startup Contract

Startup selects acknowledgement policy from the discovered capability:

- `device_emulation == false`: acknowledgement enabled.
- `device_emulation == true`: acknowledgement disabled.

The policy-aware start path also handles an AL error already present in the
scan result, so startup acknowledges before issuing a new state request.
Whenever AL terminates, Startup snapshots a `StartupAlFault` with position,
station address, requested/actual state, code, capability, and acknowledgement
result before entering `Faulted`. Restart clears both the capability array and
fault snapshot.

## Compatibility

- Existing `AlTransitionRequest` literals remain unchanged.
- Existing direct AL callers retain immediate error latching through `start`.
- `SlaveRecord` and generated product configuration layouts do not change.
- One additional scan request per discovered slave is intentional and bounded.

## Rollback

The changes are isolated to constants, scan sequencing, AL phases, Startup
diagnostics, simulations, and docs. Reverting the task restores the old scan
sequence and immediate AL fault behavior without data migration.
