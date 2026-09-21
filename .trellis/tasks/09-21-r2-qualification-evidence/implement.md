# R2 Qualification Evidence Gate Implementation

1. Refactor the build report validator to expose its existing validation as a
   reusable function without weakening CLI behavior.
2. Add the unqualified R2 and HIL topology baselines plus known limitations.
3. Implement the R2 validator and normalized report output.
4. Add positive and fail-closed regression fixtures covering path, digest,
   topology, stop-policy, compatibility, report, and review invariants.
5. Add the Make/CI artifact integration.
6. Update capability, README, PRD, lifecycle, and Trellis quality guidance.
7. Run focused Python tests, `make r2-qualification`, `make ci`, simulated HIL,
   and the live Zenoh test.
8. Review the diff, commit, push to `ra1mj/ESOP`, and verify GitHub Actions.
