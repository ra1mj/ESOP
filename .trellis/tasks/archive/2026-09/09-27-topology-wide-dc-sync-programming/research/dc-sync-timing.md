# DC SYNC timing research

## Repository evidence

- `crates/esop-ethercat-core/src/dc.rs` already defines the exact ESC
  registers, one-station disable/read/start/cycle/activate sequence, and a
  separate topology-wide offset/delay controller.
- `crates/esop-cfggen/src/esi.rs` retains direct SYNC0 cycle, signed SYNC0 and
  SYNC1 shifts, signed cycle factors, and the exact 16-bit `AssignActivate`.
- `crates/esop-product-config/src/lib.rs` freezes product base period, selected
  mode expectation, positions, stations, and reference-clock policy.
- Startup publishes immutable topology and verified online DC mode evidence
  before the PREOP configuration barrier.

## Primary public behavior references

- SOEM `ethercatdc.c` computes the first start after `system_time + SyncDelay`,
  rounds to the next cycle, applies signed shift, writes start/cycle registers,
  and activates the unit. Source:
  `https://github.com/OpenEtherCATsociety/SOEM/blob/master/soem/ethercatdc.c`.
- IgH `fsm_slave_config.c` documents the generated timing relation used by
  EtherCAT masters: direct SYNC0 cycle takes precedence, otherwise positive
  and negative factors scale the application period; SYNC1 uses
  `T1 - cycle0 + shift1`. Source:
  `https://docs.etherlab.org/ethercat/1.6/doxygen/fsm__slave__config_8c_source.html`.
- IgH's public API states that the application time selects the phase of the
  slave clocks and that the first trigger is in the future. Source:
  `https://docs.etherlab.org/ethercat/1.6/doxygen/group__ApplicationInterface.html`.

## ESOP decisions

1. Resolve timing once through a shared Rust contract used by cfggen and the
   `no_std` runtime boundary.
2. Reject inexact negative-factor division rather than silently changing a
   requested nanosecond period.
3. Preserve and finally write the complete `AssignActivate` word at `0x0980`.
4. Disable every slave before configuring any, and activate only after every
   cycle/start register has been accepted.
5. Align one common unshifted epoch to the LCM of all repeat periods, then add
   each verified SYNC0 shift.
6. Publish software evidence only after the complete batch; never describe
   accepted hardware writes as rolled back.
