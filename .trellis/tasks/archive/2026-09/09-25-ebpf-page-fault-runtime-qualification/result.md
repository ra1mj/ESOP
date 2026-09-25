# Result: eBPF page-fault runtime qualification

## Outcome

Completed on 2026-09-25 in work commit
`ba26760e7750a4cd149b3e9021a0007d5eef470c`.

The task added a Linux-only controlled page-fault fixture, closed-schema report
validator and regression tests, Make integration, a dedicated privileged
GitHub Actions job, capability evidence, and synchronized PRD/runtime/quality
documentation. Production BPF programs, maps, fixed event/context ABI,
runtime APIs, and incident policy were not changed.

## Hosted qualification evidence

- GitHub Actions run: `36130887979` (`quality`), completed successfully with
  all nine jobs passing.
- Dedicated job: `ebpf-page-fault-runtime`, completed in 2m42s and uploaded
  `esop-ebpf-page-fault-runtime-qualification`.
- Downloaded report:
  `build/ci-page-fault-36130887979/ebpf_page_fault_qualification.json`.
- The downloaded artifact independently passed
  `scripts/validate-ebpf-page-fault-qualification.py`.

Measured report values:

| Field | Value |
| --- | ---: |
| architecture | `x86_64` |
| tracked child PID | `7822` |
| target CPU | `0` |
| page size / page count | `4096` / `16` |
| child `ru_minflt` delta | `16` |
| BPF page faults | `16` |
| threshold events / emitted records / incidents | `1` / `1` / `1` |
| lost events | `0` |
| x86 error-code detail | `6` |
| count-window duration | `44,827 ns` |
| incident | Warning `HostPageFault`, confidence `65` |
| action | `DegradeHostObservation` |
| final observation | Degraded, fault `0x45422001`, heartbeat sequence `2` |

## Verification

- `make ci` passed locally.
- Focused format, Clippy, Rust example build, Python unit/compile, shell syntax,
  BPF C syntax, capability manifest, CO-RE object build, and `git diff --check`
  passed.
- The local privileged target rebuilt the CO-RE object and fixture, then
  correctly refused to claim qualification because the workstation has neither
  root execution nor passwordless sudo.
- The hosted privileged target completed real verifier/load, required
  `exceptions:page_fault_user` attach, exact injection, ringbuf decode,
  statistics, incident correlation, health projection, validation, and artifact
  upload.

## Preserved limits

This result qualifies one controlled hosted x86_64 anonymous-page first-write
count-window chain only. It does not expose fault address/IP, classify
major/minor outcome from BPF, measure fault-handler duration, or qualify memory
pressure, swap/storage, natural workload causality, product thresholds,
production kernels, overhead, WCET, or long-running HIL.
