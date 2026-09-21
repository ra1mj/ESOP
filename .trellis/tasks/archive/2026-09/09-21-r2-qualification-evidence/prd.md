# R2 Qualification Evidence Gate

## Goal

Add a fail-closed product, HIL topology, controlled-stop limit, compatibility, and WCET evidence gate for the R2 release criteria.

## Requirements

1. Add a versioned R2 qualification manifest whose checked-in baseline is
   explicitly unqualified and names every missing external qualification
   artifact.
2. Add a separate versioned HIL topology manifest and a checked-in known
   limitations document, so topology and limitations are release artifacts
   rather than prose-only claims.
3. Validate all repository-relative evidence paths and SHA-256 digests. A
   qualified claim must fail when an artifact is missing, escapes the
   repository, has changed, or contradicts another artifact.
4. A qualified R2 claim must bind one exact ESOP commit/configuration to:
   product-frozen per-axis stop policies and raw-unit limits, two distinct
   CiA 402 drive vendors, at least one EtherCAT IO module, CSP/CSV/CST and
   configured stop-action HIL coverage, qualified Q1 and Q2 performance
   reports, a qualified build report, and the required safety/license/source
   reviews.
5. Reuse the existing build/performance report semantics instead of creating
   weaker duplicate qualification rules.
6. Produce a normalized CI report that states whether the manifest is merely
   structurally valid or is a fully qualified R2 claim.
7. Keep simulated Linux tests clearly separated from real-device HIL and
   target-hardware WCET evidence.

## Acceptance Criteria

- [x] The checked-in qualification and topology manifests validate but cannot
      be changed to `passed: true` without complete external evidence.
- [x] Tests reject forged qualification through missing evidence, path escape,
      hash mismatch, placeholder product data, insufficient vendors/IO,
      missing mode/stop coverage, invalid raw limits, unqualified build/Q1/Q2
      reports, mismatched commit/config/topology, or incomplete reviews.
- [x] A complete synthetic fixture proves the positive validation path without
      being installed as product evidence.
- [x] `make ci` runs the new gate and writes
      `build/r2_qualification_report.json` for GitHub artifact upload.
- [x] Capability, README, PRD, lifecycle, and Trellis quality documentation
      describe the new gate and preserve the remaining real-hardware blockers.
- [x] The full local quality gate passes before the implementation commit and
      GitHub Actions passes after it is pushed.

## Notes

- This gate verifies evidence coherence; it does not create HIL or WCET
  evidence and must not turn simulation results into a product claim.
