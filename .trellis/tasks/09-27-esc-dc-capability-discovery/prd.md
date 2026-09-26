# ESC base and DC capability discovery

## Goal

Close the first software boundary of PRD FR-018 / DC-001 by making online
scan decode the ESC base register block exactly, probe Distributed Clocks
System Time capability, and give Startup a deterministic, product-validated
reference-clock selection before any AL transition action is emitted.

The generated product configuration must be able to require DC System Time on
selected slaves and optionally nominate one reference clock. A required
capability mismatch must fail closed before identity/SII/AL progression.

## Background

- The existing scanner reads `0x0000..0x0008`, but decodes several fields at
  the wrong widths: type/revision are combined, build is split into revision
  and build bytes, and RAM size is combined with the port descriptor.
- ESC Features Supported at `0x0008` exposes FMMU bit operation, DC support,
  and the 32/64-bit DC range. It is not represented by `ScanRecord` today.
- iGH/EtherLab and SOEM both use Features Supported bit 2 as the base DC
  capability. iGH additionally probes `0x0910` and requires a successful
  System Time register response before using a slave as a reference clock.
- ESOP already has bounded `DcController`, `DcCyclicSync`, reference-clock
  FRMW, lock monitoring, and Startup configuration barriers. Those mechanisms
  currently depend on caller-selected stations rather than scan evidence.
- Product configuration is the correct owner of whether a slave is required
  to provide DC and whether it is the explicitly selected reference clock.

## Requirements

### R1. Exact ESC base-data decoding

1. Scan shall read one fixed base-data block beginning at `0x0000` that
   contains type, revision, 16-bit build, FMMU count, SyncManager count,
   RAM-size byte, port descriptor, and the 16-bit Features Supported register.
2. `ScanRecord` shall preserve those fields at their protocol widths and expose
   FMMU bit-operation support, base DC support, and 32/64-bit DC range without
   mixing adjacent bytes.
3. Short payload, stale generation, timeout, or non-unit WKC for the base block
   shall remain a typed terminal scan failure with no partial record.

### R2. DC System Time capability probe

1. After assigning a fixed station address, a base-DC-capable slave shall be
   probed at `0x0910` using four bytes for 32-bit DC and eight bytes for 64-bit
   DC.
2. WKC `1` shall publish System Time capability and the sampled register value.
   WKC `0` shall mean delay-measurement-only capability and shall continue scan
   without pretending the slave can be a reference clock. Any WKC greater than
   one, malformed payload, generation mismatch, ownership mismatch, or timeout
   shall fail scan.
3. Non-DC slaves shall not receive the System Time probe and shall continue
   through the existing ESC Configuration / AL Status path unchanged.
4. A reference-clock candidate shall require both base DC support and a
   successful System Time register probe.

### R3. Startup evidence and selection

1. Startup shall retain position-keyed DC capability evidence from the exact
   scan records and clear it on every restart.
2. Startup profiles shall support three compatible policies: no requirement,
   System Time required, and explicitly selected reference clock.
3. Required capability mismatches, an explicit reference on a non-capable
   slave, or multiple explicit references shall fail before identity SII reads
   or AL actions and publish no selected reference station.
4. With one explicit valid reference, Startup shall select it. Without an
   explicit reference, Startup shall deterministically select the first
   System-Time-capable slave in scan order, matching the established iGH/SOEM
   fallback policy. A topology with no capable slave remains valid when no
   profile requires DC.
5. Startup shall expose the selected station/position and immutable observed
   capability evidence for later DC topology/configuration tasks.

### R4. Generated product policy

1. `esop.product.v1` slave entries shall accept an optional strict `dc` object
   with `required` and `reference_clock` booleans; omission means no product
   requirement and reference implies required.
2. Product validation shall reject reference-without-required and more than one
   selected reference before output publication.
3. Normalized JSON, C, Rust, semantic/config hashes, and runtime
   `ProductSlaveConfig` shall carry the policy without deriving it from slave
   kind or unparsed ESI text.
4. `StaticProductConfig::startup_profiles()` shall propagate the exact policy
   and reject invalid static combinations before mutating Startup.
5. The checked-in simulator product shall require DC on both drives, select the
   left drive as reference, and leave the IO module non-DC.

### R5. Compatibility, boundedness, and claims

1. Existing callers and generated products that omit `dc` shall preserve the
   prior no-requirement behavior.
2. Core behavior remains `no_std`, fixed-capacity, non-blocking, allocation
   free, and owned by the existing Startup/production request path.
3. Documentation and capability claims shall state that software simulation
   proves capability decoding and selection only, not response authenticity,
   propagation delay, clock offset compensation, SYNC configuration,
   interoperability, target timing, HIL, or functional safety.

## Acceptance Criteria

- [x] Exact byte-pattern tests prove type/revision/build/RAM/port/features are
      decoded at the correct widths and the previous mixed-field behavior is
      impossible.
- [x] 32-bit and 64-bit DC slaves issue the correct `0x0910` read length;
      WKC 1, WKC 0, malformed response, timeout, and invalid WKC paths are
      covered.
- [x] Non-DC scan emits no System Time action and completes through the legacy
      path.
- [x] Startup selects an explicit valid reference, otherwise the first capable
      slave, and rejects every required/multiple-reference mismatch before any
      identity/SII/AL action.
- [x] Restart clears reference selection and capability evidence; first
      terminal scan/Startup errors remain stable.
- [x] cfggen rejects invalid DC policy, emits it in all artifacts, changes the
      configuration hash when policy changes, and remains deterministic under
      JSON/XML formatting changes.
- [x] The generated simulator module starts with two DC-required drives and a
      selected left reference while the IO slave remains optional/non-DC.
- [ ] Focused core, cfggen, product-config, scheduler/simulator and no-std tests
      pass, followed by the repository `make ci` quality gate and GitHub
      Actions on the exact pushed commit.

## Out Of Scope

- DC port receive-time topology and propagation-delay calculation.
- Writing System Time Offset/Delay, configuring SYNC0/SYNC1, or synchronizing
  all slave clocks.
- Parsing ESI `Device/Dc/AssignActivate` or SII DC category payloads.
- Physical response provenance, real device interoperability, target WCET,
  long-duration HIL, conformance, or functional-safety qualification.

## Notes

- The task fixes a pre-existing base-register decode defect because DC
  capability evidence cannot be trustworthy while adjacent base fields are
  misinterpreted.
- Product DC policy is explicit rather than inferred from `Cia402Drive`; not
  every drive configuration may be DC-qualified, and reference selection is a
  product decision.
