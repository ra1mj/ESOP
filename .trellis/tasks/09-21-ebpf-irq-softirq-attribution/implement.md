# Implementation Plan

1. Extend `EvidenceKind` and the incident classifier; add hard IRQ/softirq
   correlation tests.
2. Extend the Linux runtime attach masks, configuration validation, map ABI
   mirrors, decoder, statistics aggregation, and update API; add unit tests.
3. Add bounded IRQ/softirq maps and tracepoint programs to the BPF bundle and
   update the fallback `vmlinux.h`.
4. Update README, product PRD status, eBPF design status, capability manifest,
   and backend quality requirements.
5. Run formatting, targeted tests, BPF syntax checks, capability validation,
   and `make ci`.
6. Review the diff, commit and push to `ra1mj/ESOP`, then wait for GitHub
   Actions to complete.
