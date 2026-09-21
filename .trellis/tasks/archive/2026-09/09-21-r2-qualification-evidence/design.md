# R2 Qualification Evidence Gate Design

## Boundaries

The qualification layer is host-side release tooling. It does not enter the
`no_std` or activated EtherCAT cycle path. The source of truth is a versioned
JSON manifest plus separately hashed evidence files. CI validates the
checked-in unqualified baseline; release qualification supplies measured
artifacts and changes the claim only after every cross-artifact check passes.

## Artifacts

- `qualification/r2_qualification.json`: product, stop policy, compatibility,
  report, trace, and review bindings plus the final claim.
- `qualification/hil_topology.json`: exact devices and Q1/Q2 topology profiles.
- `qualification/KNOWN_LIMITATIONS.md`: mandatory hashed limitation/safety
  boundary record.
- `scripts/validate-r2-qualification.py`: schema and cross-artifact validator.
- `build/r2_qualification_report.json`: normalized CI result generated only
  after validation.

## Validation Model

All evidence references use a repository-relative path and SHA-256. The
validator resolves the path under an explicit repository root, rejects path
escape and non-files, then checks the digest before interpreting content.

An unqualified manifest needs a non-empty failure list and remains useful as a
complete release checklist. A qualified manifest additionally requires:

1. Non-placeholder product identity, exact tested commit/configuration, and a
   qualified HIL topology manifest.
2. Frozen per-axis mode/action policy. Hold is CSP-only with finite signed
   position bounds; RampToZero is CSV/CST-only with a positive bounded step;
   QuickStop/Disable carry no controlled target limit.
3. Two distinct qualified drive vendors, one qualified IO module, all three
   motion modes, default stop coverage, configured controlled-stop coverage,
   and immutable device/firmware identity.
4. Existing build-report validation plus existing performance-report
   validation for one Q1 and one Q2 report. Commit, config, and topology hashes
   must agree with the qualification manifest.
5. HIL fault-matrix trace and safety-boundary, license, and source-audit
   evidence with matching digests.

The validator emits no partial success: malformed or contradictory evidence
is an error. A valid unqualified manifest produces a report with
`qualified: false`; only the complete qualified branch produces `true`.

## Compatibility

Existing report schemas remain unchanged. The build report validator gains a
callable `validate_report` function while retaining the same CLI behavior.
The new Make target is additive and becomes part of `make ci`.

## Rollback

Removing the new Make target and qualification files restores the old release
tooling without changing runtime code or ABI. No generated artifact is treated
as runtime configuration.
