# SII DC layout research

## Public behavior evidence

- EtherCAT SII category `0x003c` is represented as fixed 24-byte operation-mode
  entries by independent open-source EEPROM tooling.
- The observed field sequence is SYNC0 cycle time, SYNC0 shift, SYNC1 shift,
  SYNC1 cycle factor, AssignActivate, SYNC0 cycle factor, name index,
  description index and four reserved bytes.
- ESI exposes the corresponding values under `Device/Dc/OpMode`; the first
  mode is commonly treated as a default, but ESOP selects by explicit product
  name to avoid hidden ordering policy.
- The category does not carry a direct `CycleTimeSync1` value. ESOP's strict
  verifiable subset therefore accepts only zero direct content and preserves
  the factor represented online.

## Sources consulted

- KickCAT `SIIBuilder` implementation/tests, including the 24-byte entry and
  generated name/description indices:
  `https://github.com/leducp/KickCAT/tree/089f8cad4e852ea3b9e1d0b407425abe7440e3f3`
- Synapticon `siitool` SII parser and ESI conversion behavior:
  `https://github.com/synapticon/siitool/tree/c46570adb1228eada44ae1f38bd61a81fab99c98`
- SOEM DC pulse programming behavior remains the reference for the subsequent
  start-time task, not for this parser:
  `https://github.com/OpenEtherCATsociety/SOEM/tree/88e8ed46efba7dfa7b94d08a512db25a33e3f8d5`

## Independence boundary

These sources are used only to identify public protocol behavior and compare
interoperability expectations. ESOP code, type names, comments, validation
structure and tests are independently authored. No external source file,
structure definition, comment, or byte vector is copied into the project.
