# Result: eBPF observability degradation runtime qualification

Date: 2026-09-25

## Delivered

- Added a dedicated event-loss fault (`0x45421004`) and made positive loss
  sticky within one agent epoch. A healthy capability refresh no longer hides
  known loss, while hard verifier/permission failure retains Failed precedence.
- Reset attach, loss, incident, fault, heartbeat, and event-time state at agent
  restart so a new epoch begins conservatively and can recover only after a new
  runtime publishes a complete capability snapshot.
- Added a Linux/x86_64 qualification fixture that loads the production CO-RE
  object with only `exceptions:page_fault_user`, keeps the shipped 4 MiB
  ringbuf, and fills it through bounded 64 MiB anonymous first-write batches
  without polling the event channel.
- Proved exact kernel-loss projection into `RuntimeAgent`, same-epoch Degraded
  stickiness, runtime detach, epoch-2 reset, production-object reload, Healthy
  recovery, and accepted new-epoch evidence with zero new loss or rejection.
- Added a closed-schema validator, six mutation tests, Make targets, a dedicated
  privileged GitHub Actions job and artifact, cross-layer MLG regression, and
  synchronized README/PRD/runtime/capability/Trellis quality contracts.
- Preserved the claim boundary: missing BTF/ringbuf/required attach and
  verifier/permission states are deterministic policy tests, not hosted fault
  injection; production pressure, target kernels, overhead/WCET, and long
  duration remain open.

## Verification

- Work commit: `9db984178f79157d177d2688e665b902d434d204`.
- GitHub Actions run: `36151603726` (`quality`, run 133).
- Run URL: `https://github.com/ra1mj/ESOP/actions/runs/36151603726`.
- All 12 jobs passed, including `ebpf-observability-degradation-runtime`, all
  existing privileged eBPF qualifications, CO-RE build, and Rust/Zenoh quality.
- Artifact: `esop-ebpf-observability-degradation-qualification`, artifact ID
  `10871913161`.
- Downloaded ZIP SHA-256:
  `0b14673d0de0636304e5bcbb47e063902d2e56ddd68fd8b48ab0cded91e28c52`;
  this exactly matched the GitHub artifact digest.
- Extracted report SHA-256:
  `341941b35f30dfd1f6780ce2c32b03fa96dc2facdf403b5eed89e9178c3da3fc`.
- The downloaded report independently passed
  `scripts/validate-ebpf-observability-degradation-qualification.py`.
- Local `make ci`, formatting, workspace tests, Clippy, release/no-std builds,
  BPF C syntax, capability validation, Python mutation/compile tests, shell
  syntax, example build, and `git diff --check` passed.
- The local privileged target could not run because this workstation has no
  Clang executable, root session, or passwordless sudo. No local runtime claim
  was made; the dedicated hosted runner supplied the privileged evidence.

## Hosted measurements

| Field | Value |
| --- | ---: |
| architecture | `x86_64` |
| production ringbuf | `4,194,304 bytes` |
| chunk size | `67,108,864 bytes` |
| saturation batches | `3 / 8` |
| pages touched | `49,152` |
| successful emitted events | `40,329` |
| kernel lost events | `8,823` |
| page faults | `49,152` |
| saturation poll records | `1` |
| projected new loss | `8,823` |
| loss/reapplied state | `Degraded / Degraded` |
| loss fault | `0x45421004` |
| second agent epoch | `2` |
| Restarting attach/loss | `0 / 0` |
| recovered/final state | `Healthy / Healthy` |
| recovery records | `64` |
| recovery loss/rejection | `0 / 0` |
| both runtimes detached / cleanup | `yes / yes` |

The first runtime's loss count was projected exactly once and remained visible
after the epoch-1 capability refresh. Restart cleared every epoch-local health
field before runtime 2 loaded. Runtime 2 accepted new evidence, retained zero
loss, and published the report only after both runtimes detached.

## Residual limits

This result qualifies one hosted x86_64 page-fault-driven saturation and
unload/reload recovery path. It does not establish actual missing-BTF,
missing-ringbuf, missing-attach, verifier-rejection, or permission-denial host
behavior; exact ringbuf record capacity; production event pressure; portable
target-kernel behavior; CPU/RAM overhead; WCET; ringbuf watermarks; sustained
load; or long-duration HIL stability.
