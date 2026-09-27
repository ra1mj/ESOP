# Existing Startup Reuse Evidence

## Confirmed Authorities

- `ScanController` is restartable from Idle, Complete, or Faulted and clears
  its fixed record array before probing.
- `StartupController::start_inner` already performs the complete synchronous
  evidence reset required by rescan: scan/SII/register/AL controllers, slave
  table, verified arrays, reference clock, DC topology, OpOnly gates,
  configuration state, and faults.
- Startup retains the expected-slave and profile arrays after Ready/Faulted, so
  rescan can copy and reuse the compiled product plan without caller-provided
  dynamic data.
- Startup already accepts restarts from Ready and Faulted, but its public start
  APIs require callers to resupply the plan and can target OP. A named rescan
  wrapper must freeze PREOP and an empty configuration-service barrier.
- The production scheduler already gives Startup highest service priority and
  preserves accepted request ownership across cycles. Named Rescan projection
  can reuse the exact Startup action path.
- Lifecycle projection already clears topology readiness whenever Startup is
  not Ready. Named Rescan still needs an explicit service arm so diagnostics
  and terminal readiness cannot be confused with an ordinary cold start.

## Missing Contract

- No stable rescan handle/status/result API exists.
- No whole-operation absolute Startup deadline exists; nested configured
  timeouts can otherwise exceed an explicit recovery deadline.
- Production reports cannot distinguish cold Startup from explicit rescan.
- Capability/docs currently state that rescan is planned.
