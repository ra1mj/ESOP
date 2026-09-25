# Result: eBPF softirq qualification temporal gating

Date: 2026-09-25

## Delivered

- Changed the hosted softirq qualification fixture so exact CPU/vector
  matching is treated as spatial filtering only. Each calibration and formal
  runtime now starts on qualification-only closed vector `u32::MAX - 1`, opens
  `NET_RX` vector 3 only across the counter-bounded GSO send, and closes before
  socket draining or ring-buffer polling.
- Kept the production runtime API, kernel-context ABI, BPF maps, attach masks,
  all-vector sentinel, evidence ABI, and incident policy unchanged by reusing
  `BpfRuntime::update_interrupt_filter`.
- Preserved the strict exact-one contract for target-CPU `NET_RX`, records,
  incidents, samples, overruns, and emitted events with zero loss. No retry or
  relaxed count was introduced.
- Added seven closed/open/closed report fields, strict schema validation,
  mutation tests, synchronized README/PRD/runtime/capability documentation, and
  a durable backend quality rule covering temporal qualification isolation.

## Verification

- Work commit: `d2b7eeb9641fec87444f2aabc2ae6ba7855ae414`.
- GitHub Actions `quality` run `36160745503` (run 135) completed successfully;
  all 12 jobs passed, including dedicated `ebpf-softirq-runtime` job
  `108156404240`, CO-RE build, every other privileged eBPF qualification, and
  the complete Rust/Zenoh gate.
- Artifact `esop-ebpf-softirq-runtime-qualification`, artifact ID
  `10876025194`, was downloaded independently.
- ZIP SHA-256 `ca16d3489eddffd05666a870d4efc6a9c0195b817bc477b70e2a8ee4135261e7`
  exactly matched the GitHub artifact digest.
- Extracted report SHA-256:
  `bc8836fc0d39e32e81089de80cf918d9d32bb6673d1643322e57a456ec832eab`.
- The downloaded report independently passed
  `scripts/validate-ebpf-softirq-qualification.py`.
- Local `make ci`, formatting, focused Rust tests/build/Clippy, eight Python
  validator tests, Python/shell/BPF syntax, capability validation, release and
  no-std checks, and `git diff --check` passed.

## Hosted measurements

| Field | Value |
| --- | ---: |
| target CPU | `0` |
| quiet target-CPU `NET_RX` delta | `0` |
| closed vector | `4294967294` |
| calibration gate | `4294967294 -> 3 -> 4294967294` |
| formal gate | `4294967294 -> 3 -> 4294967294` |
| calibration/formal `NET_RX` delta | `1 / 1` |
| calibration duration | `70,390 ns` |
| derived formal threshold | `8,798 ns` |
| formal duration | `40,395 ns` |
| sent bytes / received datagrams per phase | `64,800 / 54` |
| formal records / incidents | `1 / 1` |
| formal softirq samples / overruns / emitted | `1 / 1 / 1` |
| hard-IRQ samples / overruns | `0 / 0` |
| lost / newly reported lost events | `0 / 0` |
| malformed / rejected / dropped incidents | `0 / 0 / 0` |
| observation state | `Healthy -> Degraded` |
| observation fault | `0x45422001` |

## Root cause and residual limits

The failed predecessor run admitted unrelated vector-3 activity because the
filter remained open from load through polling. The exact `/proc/softirqs`
delta proved the controlled send itself still produced one target action, so
the correction narrowed the admission lifetime instead of weakening evidence.

This qualifies one controlled hosted loopback `NET_RX` softirq-duration chain.
It does not establish hard-IRQ behavior, physical NIC/driver/NAPI behavior,
production interrupt budgets, sustained pressure, causal task ownership,
target-kernel portability, overhead/WCET, or long-duration HIL stability.
