# SyncManager Policy Metadata and Physical Activation

## 1. Scope / Trigger

Use this contract whenever OpOnly policy generates a physical SyncManager
activation write or exact readback. SII/ESI policy metadata and ESC register
contents are different representations; identical byte offsets do not make
them interchangeable.

## 2. Signatures

- `OpOnlySyncManagerProfile::activation_template(index)` and
  `activation_templates()` return the retained raw policy templates.
- `OpOnlySyncManagerProfile::activation_for(index, enabled)` produces the
  physical ESC activation byte used by Startup and MappingConfig.
- `OpOnlySyncManagerController` writes and exactly reads back
  `0x0806 + sync_manager * 8` through the common control-request owner.

## 3. Contracts

- Keep raw OpOnly bit 3 in the frozen SII profile and configuration signature.
  Do not rewrite generated SII templates or clear the OpOnly policy mask.
- Before generating physical activation, apply the ESC writable-bit mask
  `0xC3`, then set/clear Enable bit 0 according to the requested state.
  SII-only OpOnly bit 3 and physical reserved bits 5:2 must never be written.
- Physical bits 0/1/6/7 are Enable, Repeat Request and the two latch-event
  flags. Existing explicit activation-template flags in that physical mask
  remain available; raw metadata accessors do not become wire encoders.
- MappingConfig writes OpOnly output SyncManagers disabled in PREOP using the
  same conversion. Startup disables and verifies before non-OP transitions,
  and enables/verifies only at its existing OP sequencing boundary.
- Exact readback compares the complete returned physical byte with the
  generated physical byte. Never mask a bad readback to hide reserved bits.

Authoritative register definition: Beckhoff Section II v3.3, §2.14.5,
[SyncManager activate (0x0806), II-103](https://download.beckhoff.com/download/Document/io/ethercat-development-products/ethercat_esc_datasheet_sec2_registers_v3.3.pdf).
The independent IgH implementation also derives physical enable instead of
copying SII enable metadata directly:
[`ec_sync_page`, sync.c](https://docs.etherlab.org/ethercat/1.6/doxygen/sync_8c_source.html).

## 4. Validation & Error Matrix

| Input/observation | Result |
| --- | --- |
| Raw template 0x09 or 0x29, disabled | Physical 0x00; raw template retained |
| Raw template 0x09 or 0x29, enabled | Physical 0x01; raw template retained |
| Explicit template 0xCB or 0xFF | Physical 0xC2 disabled / 0xC3 enabled |
| Returned physical byte has unexpected bit 3 | `ReadbackMismatch` |
| Wrong WKC/generation/length/deadline | Existing exact typed fault |

## 5. Good / Base / Bad Cases

- Good: raw SII template 9 remains part of signature verification while a
  physical disabled/OP-enabled register reads back 0/1.
- Base: an ordinary non-OpOnly SyncManager keeps its existing Boolean enable
  register encoding.
- Bad: send template 0x29 directly, then teach the simulator to retain reserved
  register bits. Real hardware reads reserved bits as zero and exposes the bug.

## 6. Tests Required

- `wire_qualification` independently spells FPWR header/address/payload bytes
  for templates 0x09, 0x29, 0xCB and 0xFF, both enable states, through a leased
  control index. It verifies bits 5:2 are absent and raw metadata is unchanged.
- MappingConfig asserts a literal complete disabled SyncManager register image.
- OpOnly/Startup/reconfiguration tests retain strict good and mismatched
  readbacks, ordering and lifecycle gating.
- Stateful ESC tests mask physical activation reserved bits independently.

## 7. Wrong vs Correct

Wrong: `physical_activation = sii_template | enable`, or mask returned data
before readback comparison.

Correct: retain SII metadata, generate the physical writable byte explicitly,
and compare its exact hardware readback.
