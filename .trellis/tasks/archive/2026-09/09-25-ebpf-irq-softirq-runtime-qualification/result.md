# Result

- Completed: 2026-09-25
- Work commits: `73d05e7`, `275a816`
- GitHub Actions: `quality` run `36127109470` completed successfully for commit
  `275a816999f36e40701db764d8660f2498e89d86`.
- The dedicated `ebpf-softirq-runtime` job completed successfully in 1 minute
  41 seconds and uploaded `esop-ebpf-softirq-runtime-qualification`.
- The downloaded `ebpf_softirq_qualification.json` independently passed
  `scripts/validate-ebpf-softirq-qualification.py`.

## Measured hosted evidence

- Target CPU: 0; quiet-window `NET_RX` delta: 0.
- Calibration duration: 44,867 ns; derived formal threshold: 5,608 ns.
- Formal duration: 44,446 ns.
- Formal statistics: one softirq sample, one overrun, one emitted event, zero
  hard-IRQ samples/overruns, and zero lost events.
- Correlation: Error `HostIrqStorm`, `ControlledStop`, confidence 70, followed
  by Degraded observation fault `0x45422001`.

## Validation

- Local `make ci` passed.
- User-local Clang 14 was installed under `~/.local/opt/clang14`; the production
  BPF source compiled into a valid eBPF ELF object. Local privileged execution
  remained unavailable because the environment has neither root nor
  passwordless sudo, so the hosted Actions runner supplied the required real
  verifier/load/attach qualification.
