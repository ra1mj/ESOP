# Protocol Research

## Requesting ID Sequence

Primary vendor documentation describes Requesting ID as an INIT-state sequence:

1. Set AL Control register `0x0120` ID Request bit 5.
2. Wait for AL Status register `0x0130` ID Loaded bit 5.
3. Read the identification value from AL Status Code register `0x0134`.

The write must retain the current AL state bits. The 16-bit value at `0x0134`
is product-configured and used to distinguish same-model devices.

Sources consulted 2026-09-27:

- Beckhoff EtherCAT system documentation, “Explicit Device Identification”.
- Beckhoff AX8000 diagnostics, “Identifying slave devices”.
- Microchip EtherCAT conformance test record for explicit requesting ID.
- HPMicro SSC ESI examples declaring
  `Device/Info/IdentificationReg134=true`.
- EtherCATInfo XSD copies showing optional `IdentificationReg134` boolean and
  separate optional `IdentificationAdo` value.

## Scope Decision

Only Requesting ID through `0x0134` is implemented in this task. Configured
Station Alias and arbitrary Data Word identification are separate mechanisms
and remain explicit limitations. This avoids collapsing three different wire
contracts into one ambiguous configuration field.
