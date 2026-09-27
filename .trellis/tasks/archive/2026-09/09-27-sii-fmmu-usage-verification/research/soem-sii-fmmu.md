# SOEM SII FMMU reference

Research date: 2026-09-27

Primary implementation inspected: OpenEtherCATsociety/SOEM, current default
branch clone at research time.

- `include/soem/ec_type.h` declares SII category 40 (`0x0028`) as FMMU.
- `src/ec_main.c::ecx_siiFMMU` reads the category word length, converts it to a
  byte count, and reads one function byte per FMMU index.
- `include/soem/ec_main.h` documents function values as `0=unused`,
  `1=outputs`, `2=inputs`, `3=SM status`.
- `src/ec_config.c` treats `0xff` as a marker that does not override the
  implementation's existing/default function value.

Source locations:

- https://github.com/OpenEtherCATsociety/SOEM/blob/master/src/ec_main.c
- https://github.com/OpenEtherCATsociety/SOEM/blob/master/include/soem/ec_main.h
- https://github.com/OpenEtherCATsociety/SOEM/blob/master/include/soem/ec_type.h

Implication for ESOP: category `0x0028` is capability/use metadata, not a source
of logical or physical address ranges. ESOP should verify ordered FMMU index
usage while continuing to derive addresses from its deterministic Domain/PDO
configuration.
