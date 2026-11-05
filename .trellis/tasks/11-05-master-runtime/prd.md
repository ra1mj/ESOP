# Reusable software master completion

User approved implementation of the generic software-master plan. HPM is a
future single-axis physical slave; its model/firmware is not a prerequisite.

## Requirements

- Typed optional mailbox/CoE capability, static IO without CoE, fixed mapping,
  assignment-only and fully configurable PDO plans with appropriate readback.
- Non-byte-aligned PDOs retain real bit locations and padding, byte-cover
  aggregate transport, complete-only publication and exact WKC.
- OpOnly runtime AL transitions reuse existing controllers, shared transport,
  absolute deadlines, exact readback, first faults and explicit re-admission.
- A no_std esop-master-runtime owns reusable startup/cyclic/maintenance/stop
  composition and bounded SDO/state/rescan/reconfigure/snapshot/event APIs.
- Linux esop-master supports generated build-bound products, single-axis sim
  default at1ms, sim/replay/raw, finite/continuous runs and bounded shutdown.
- Keep text/files/wait outside activated runtime, preserve zero allocation,
  motion policy/permit gates, and existing ProcBuf semantics/read-only esop CLI.
- Migrate/version generated runtime configuration and regenerate all fixtures.
- Refactor r0_r2 to the shared runtime while preserving regression evidence.

## Acceptance

- [ ] Pure IO/no CoE, fixed/assignment/configurable, bit PDO and 1/8/32 mixed
  independent virtual networks pass through public runtime APIs.
- [ ] OpOnly paths, deadlines, queue saturation, stale handles and failed/normal
  shutdown preserve ownership, truthful evidence and explicit recovery.
- [ ] Eight-request SDO coexists with PDO/DC, monitoring and fault isolation.
- [ ] Wire/replay, Q1/Q2/Q4, virtual soak, allocation, no_std and Linux make ci pass.
- [ ] Documentation, capabilities, resources and source/evidence mappings agree.
- [ ] Work is committed, pushed and integrated to main; paseo.json preserved.

Hardware qualification, FoE/SoE/EoE, redundancy, ROS2 and MCU master ports remain
outside this increment. README bookkeeping was synchronized before starting.
