# Research: ESM timeout and OpOnly requirements

Research performed 2026-09-27. Sources are used to define implementation behavior, not
to claim conformance or certification.

## Primary sources

### ETG.1500 EtherCAT Master Classes, V1.0.2

Source:
`https://www.ethercat.org/download/documents/ETG1500_V1i0i2_D_R_MasterClasses.pdf`

- The master requirements state that ESM transition timeouts shall come from ESI/SII
  and use ETG.1020 defaults when absent.
- They require all `OpOnly` output SyncManagers to be disabled while a slave is not in
  Operational state.
- Both ESI and SII provide an `OpOnly` indication.

### ETG.2000 Slave Information Interface, V1.0.15

Source:
`https://www.ethercat.org/download/documents/ETG2000_V1i0i15_S_R_SlaveInformationInterface.pdf`

- The SII SyncManager category encodes the initial enable flag and the `OpOnly` flag in
  the activation byte.
- Activation bit 0 is enable; bit 3 is `OpOnly`. They must not be collapsed into a
  single boolean.

### EtherCATInfo schema evidence

Source:
`https://github.com/DiamondLightSource/ethercat/blob/master/etc/xml/EtherCATInfo.xsd`

This is a public repository copy of the EtherCATInfo schema, used as schema evidence
because current ETG.1020 content is member-restricted. It defines:

- `PreopTimeout` default: 3000 ms;
- `SafeopOpTimeout` default: 10000 ms;
- `BackToInitTimeout` default: 5000 ms;
- `BackToSafeopTimeout` default: 200 ms;
- SyncManager `OpOnly` as a boolean attribute mapped to the SII flag.

The implementation records these values in a named versioned constant. A future update
requires an explicit code/documentation change.

### Beckhoff EtherCAT ESC technology data sheet

Source:
`https://download.beckhoff.com/download/document/io/ethercat-development-products/ethercat_esc_datasheet_sec1_technology_2i3.pdf`

- SyncManager register blocks use an 8-byte stride.
- The activation byte is at offset 6 within the block.
- Activation bit 0 controls SyncManager enable.

## Decisions

- Use per-transition profiles with a non-zero uniform compatibility override.
- Share one absolute deadline across OpOnly and AL work for a transition step.
- Disable before leaving OP, but enable only after OP has been observed; startup readiness
  remains held until readback succeeds.
- Treat `OpOnly` on an input/TxPDO SyncManager as invalid configuration.
- Preserve raw SII activation flags and derive enable/OpOnly via bit helpers.
- Keep physical HIL and device interoperability as open evidence after software tests.

## Deferred questions

- Device-specific init-command sequences may impose additional ordering constraints;
  those require physical HIL evidence.
- Complete SM/FMMU/DC descriptor discovery and mailbox configuration are separate PRD
  increments and are not bundled into this task.
