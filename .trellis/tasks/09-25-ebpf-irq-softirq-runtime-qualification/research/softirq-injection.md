# Softirq injection research

## Repository facts

- `bpf/esop_runtime.bpf.c` already measures complete hard-IRQ and softirq
  entry/exit duration, emits fixed evidence above independent thresholds, and
  records per-CPU samples/overruns.
- `crates/esop-ebpf-runtime/src/linux.rs` already attaches each pair atomically,
  decodes `SoftirqCpuTime`, aggregates statistics, and updates both thresholds.
- `crates/esop-ebpf-agent/src/lib.rs` currently merges every `HostIrqStorm`
  inside the time window without comparing interrupt class, CPU, or vector.
- The context ABI has an adjacent reserved `u16` and `u32` suitable for an
  optional CPU/vector filter without changing its 176-byte size.
- The shared interrupt emitter includes current TGID/TID. A bounded loopback
  burst handled inline should therefore retain the fixture PID/TID; those
  fields describe execution context, not interrupt ownership.

## Kernel source evidence

- Linux kernel `kernel/softirq.c` defines vector 3 as `NET_RX` and calls
  `trace_softirq_entry(vec_nr)` immediately before `h->action()` and
  `trace_softirq_exit(vec_nr)` immediately after it. The tracepoint duration is
  therefore the complete vector action, not raise-to-run latency:
  https://github.com/torvalds/linux/blob/master/kernel/softirq.c
- The kernel tracepoint documentation states that `softirq_entry` and
  `softirq_exit` are paired to determine the handler routine:
  https://www.kernel.org/doc/html/v5.0/core-api/tracepoint.html
- Linux loopback transmit passes the skb to `__netif_rx`, which schedules the
  receive path consumed by `NET_RX` softirq:
  https://github.com/torvalds/linux/blob/master/drivers/net/loopback.c

## Local injection experiment

Environment: 32 CPUs, no passwordless sudo, so the experiment used
`/proc/softirqs` rather than loading the BPF object.

- Connected loopback UDP with ordinary `sendmmsg` produced a target-CPU
  `NET_RX` delta equal to the message count (32 and 128 tested). It does not
  produce one isolated bounded handler and is unsuitable for exact evidence.
- Connected loopback UDP with one `sendmsg` and `UDP_SEGMENT=1200` produced a
  target-CPU `NET_RX` delta of exactly one for 8, 32, 40, 48, and 54 receive
  datagrams across repeated runs.
- 64 x 1200 bytes exceeded the UDP message-size limit. 54 x 1200 = 64,800 bytes
  stayed within the limit and maximized the tested bounded burst.
- Background `NET_RX` activity was visible on other CPUs. Exact BPF CPU/vector
  filtering is therefore required for a stable hosted qualification; global
  counts or a one-nanosecond global threshold would be noisy and could overflow
  the bounded incident evidence.

## Decision

Use one 54-segment loopback UDP GSO send on a quiet allowed CPU, filter BPF
entry state to that CPU and vector 3, calibrate one handler duration in a
disposable runtime, then verify a fresh formal run above one eighth of the
measured calibration duration. Claim only the controlled hosted loopback
softirq chain.
