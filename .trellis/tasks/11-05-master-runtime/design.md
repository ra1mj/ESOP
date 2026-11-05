# Design

Reuse existing protocol/FSM/scheduler ownership; no second control transport.
Configuration is generated offline and frozen before activation. Runtime v2
records typed mailbox capability and PDO write policy; absent mailboxes never
produce SDO work. Fixed mapping uses verification, assignment-only changes only
assignment, configurable mapping retains download plus exact upload readback.
Bit segments use byte-cover aggregate datagrams with explicit unowned padding.

Extend runtime state requests with composite AL/OpOnly actions while retaining
the ordinary AL-only public surface. Disable/verify before leaving OP and
enable/verify after entering OP; every operation retains one absolute deadline.

The no_std runtime composes static product validation, startup, controllers and
existing cyclic/lifecycle owner. Callers advance it using supplied port/time and
submit bounded operations; snapshots never format or allocate. Linux wraps this
with build-bound generated products and host-side waiting/reporting. The old
r0_r2 scenarios become callers of this shared composition. Default raw motion
remains inhibited; software results cannot qualify hardware.

Shutdown revokes permits and drives the existing verified-stop path. Missing
feedback or elapsed deadline remains a fault, not a successful stop. Recovery
cannot mint a motion permit. All capacities and configuration policy enter
generated resources/hash/version contracts.
