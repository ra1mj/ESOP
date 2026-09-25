# Implementation plan

- [x] Refactor the softirq fixture so `GsoSockets::inject` owns the bounded
      filter-open/send/counter/filter-close sequence and exposes verified
      before/active/after vector snapshots.
- [x] Configure both fresh runtimes with the closed vector and preserve exact
      attach, baseline, poll, statistics, incident, and health assertions.
- [x] Add the seven gate fields to atomic report output, strict validation, and
      the canonical Python fixture; add targeted rejection mutations.
- [x] Update README, software PRD, observability design, capability manifest,
      and backend quality guidance with the temporal-gating contract.
- [x] Run `cargo fmt --check`, focused Rust/Python checks, BPF/shell/Python
      syntax checks, capability validation, `git diff --check`, and `make ci`.
- [ ] Review the diff for unchanged production ABI/behavior and strict failure
      cleanup, then commit and push.
- [ ] Track the final GitHub Actions run to completion, inspect the softirq job,
      download and hash the artifact, and independently rerun its validator.
- [ ] Complete/archive the Trellis task and record the session journal only
      after remote qualification succeeds.
