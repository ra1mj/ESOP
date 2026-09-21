# ESOP R2 Known Limitations

Qualification status: **not qualified** as of 2026-09-21.

## Functional Safety Boundary

ESOP motion lifecycle handling is an ordinary control safeguard. It is not a
certified functional-safety component and does not replace STO, FSoE, a safety
PLC, guarded machinery, braking analysis, or a product risk assessment.

## Missing Qualification Evidence

- No product board, PHY, DMA/cache policy, RTOS, or target clock is frozen.
- No product-specific CiA 402 raw-unit scaling or controlled-stop limits are
  frozen for any axis.
- No two-vendor real-drive plus EtherCAT IO topology has completed the R2 HIL
  startup, cyclic, fault, stop, recovery, and soak matrix.
- No target-hardware Q1 or Q2 WCET/performance report is supplied.
- No target build resource map, stack watermark, or post-activation allocator
  trace is supplied.
- No release-specific license, source-origin, or safety-boundary approval is
  supplied.

## Unsupported Combinations

All board, PHY, RTOS/kernel, drive, firmware, IO, PDO, mode, cycle-time, and
controlled-stop combinations remain unsupported for production use until a
release qualification manifest binds the exact combination to passing
evidence. The Linux simulator and AF_PACKET development port are not target
hardware qualification evidence.

## Residual Risks

Drive firmware differences, PDO scaling, braking behavior, mechanical load,
network fault response, cache maintenance, interrupt latency, and external
safety-chain behavior can invalidate a software-only result. A structurally
valid manifest is not permission to energize a machine.
