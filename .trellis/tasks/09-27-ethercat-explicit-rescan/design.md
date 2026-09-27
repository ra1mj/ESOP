# Design: EtherCAT Explicit Rescan

## Architecture

Rescan is a named run mode of the existing `StartupController`, not a second
scanner:

```text
application/operator start_rescan(deadline)
                 |
                 v
 StartupController retained expected/profile plan
                 |
                 +-- validate request transactionally
                 +-- publish stable RescanHandle/status
                 +-- synchronously clear retained readiness evidence
                 +-- restart existing Startup pipeline with target PREOP
                 |
                 v
 production scheduler: Rescan service at Startup priority
                 |
                 +-- cyclic Domain/DC work remains first
                 +-- one shared control request per cycle
                 +-- no in-flight service preemption/retransmission
                 |
                 v
 typed Rescan progress/result + lifecycle topology gate
```

The Startup controller already owns every protocol authority required by the
parent PRD. Keeping rescan metadata inside it avoids aliasing two mutable owners
of the same pending action and guarantees exact action/control-pool matching is
unchanged.

## Public Contract

Add fixed-size public types in a dedicated `rescan` module:

- `RescanHandle`: nonzero monotonically changing stable operation handle.
- `RescanPhase`: Idle, Active, Complete, or Faulted.
- `RescanStatus`: handle, phase, current Startup phase, generation, absolute
  deadline, expected count, discovered count, and optional first error.
- `RescanProgress`: named projection of reused Startup progress plus Complete.
- `RescanResult`: terminal summary containing the frozen request identity,
  expected/discovered counts, final Startup phase, and fault.
- `RescanError`: invalid handle/deadline/state plus wrapped Startup failure.

`StartupController::start_rescan` owns submission and returns the handle.
`rescan_status` and `rescan_result` never expose mutable protocol internals.
Starting another explicit rescan replaces the prior handle/result only after
the new request has fully validated.

## Transactional Invalidation

Submission first snapshots and validates the retained expected/profile plan,
phase, deadline, and timeout configuration. Only then does it assign a new
rescan handle and restart Startup. The existing `start_inner` reset sequence is
the single invalidation authority: it replaces the scan controller, slave
table, all verified SII/register/mailbox/DC arrays, reference clock, topology,
AL/OpOnly controllers, configuration barrier state, and prior fault evidence
before starting the scan.

The rescan restart copies the retained arrays through local fixed-size values,
sets `target_state = PREOP`, clears `configuration_services`, and preserves all
other product timeouts and profile expectations. A failed preflight leaves the
old Ready/Faulted evidence and prior result untouched.

## Absolute Deadline

Startup gains an optional operation deadline used only for explicit rescan.
Before producing work, it fails if the absolute deadline has elapsed. Every
nested controller start receives:

```text
operation_timeout = min(configured_timeout, deadline - now)
request_timeout   = min(configured_request_timeout, deadline - now)
```

AL/OpOnly step deadlines are similarly capped. Normal cold-start behavior uses
no overall deadline and remains byte-for-byte compatible with its existing
configured timeout policy.

## Scheduler Integration

The scheduler classifies Startup actions as `Rescan` whenever the controller's
rescan phase is Active or Faulted. The wire action remains a `StartupAction`, so
request creation, exact matching, timeout, terminal consumption, and Startup
error handling reuse the existing path. Only progress/fault projection changes
to the named rescan types.

Priority remains:

```text
Startup / Rescan
  -> PDO / Watchdog / Mapping / DC configuration
  -> State Request
  -> Mailbox
  -> Register Request
```

The scheduler's existing ownership rule prevents a newly submitted rescan from
preempting an already accepted service request.

## Lifecycle Contract

`start_rescan` changes Startup from Ready to Scanning and clears DC/topology
evidence before returning. Existing lifecycle projection therefore revokes the
topology gate in the same cycle, before any APRD. The explicit Rescan service
arm also requires `service_ready` for terminal restoration. Completion ends at
PREOP, so it cannot independently satisfy drive, DC lock, domain, command-age,
supervisor, safety, or motion-permit gates.

## Compatibility and Rollback

The API is additive. Normal `start`/`start_with_profiles` clear rescan metadata
and preserve existing behavior. Removing the rescan module and scheduler
projection returns the system to cold startup plus explicit state requests;
no process-image or generated-product format migration is introduced.

## Verification Strategy

- Controller/Startup unit tests cover handles, validation, evidence clearing,
  deadline caps, PREOP-only completion, retained-plan reuse, and faults.
- Scheduler tests cover named priority, action identity, nonpreemption,
  terminal progress, and no implicit trigger.
- Linux simulation delays an empty-topology APRD response across a cycle while
  checking LRW/FRMW ordering, no duplicate request, fixed pool use, and
  fail-closed lifecycle projection.
- Full repository gates preserve no_std, release, generated artifacts, eBPF,
  protocol, and host integration behavior.
