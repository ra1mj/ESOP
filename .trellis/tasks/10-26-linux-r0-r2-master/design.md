# Design

## Boundaries and ownership

The existing no_std EtherCAT core remains the protocol authority. The frozen
Domain/DC plan and single production scheduler retain transport ownership.
New request services emit bounded actions and immutable typed results. No
activated protocol path allocates, sleeps, logs text or owns a thread.

## Wire and scan

M=0 denotes the final datagram. Physical addresses encode ADP then ADO as
little-endian WORDs. Initial BRD ESC_TYPE/2 counts the bus: exact ADO,
returned ADP delta equals WKC, both bounded by MAX_SLAVES. Freeze that count;
AP/BR then require exactly its position increment. Fixed/logical remain exact.
Share typed policy checks in RX, control completion and DC, preserving exact
old entry points and RxExpectation literals. Timeouts always fault scans.

## Simulation and hosted integration

A separate fixed-capacity no_std-compatible simulator owns its ESC model;
frames/time/faults are its only master-facing inputs. Independent static SII
ROM and command semantics prevent mirroring controller expectations. Hosted
runner/backend/capture/measurement code lives outside the activated core.
Classic PCAP sidecars explicitly bind direction, product, scenario and time;
no replay rewriting of indices/addresses/WKC is permitted.

## Mailbox

One transport authority per physical slave owns counters/repeat/receive state.
TX counters cycle 1..7 and persist across retransmission. RX counters are
independent and may be zero/nonsequential; zero is not counter-deduplicated.
Correlate SDO by owning transfer/service/object/toggle, not TX-counter equality.
Same-slave FIFO logical ownership and cross-slave wire-action round robin use
eight stable slot/generation requests. Preserve the old object-size default;
new scenarios freeze 1024 bytes per object. Oversize is typed failure.
Full mailbox writes are zero-padded. MRL toggles SM Activation +6 bit1, polls
PDI Control +7 bit1 ACK, then rereads; preserve other bits and the absolute
deadline. Emergencies use a caller-owned non-blocking bounded sink. Runtime
terminal faults release scheduler selection; configuration faults remain gates.

## Monitoring and evidence

Monitoring uses frozen cadence/age/budget and the common control pool; stale,
unknown or bad AL/DL observations cannot qualify lifecycle gates. Contextual
events and snapshots project through existing ABI owners. Non-EtherCAT I2C
simulation is lower-priority bounded work. Reports distinguish scenario success
from qualified hardware and retain missing physical evidence as failures.

## Compatibility and rollout

Maintain old exact RX entry points and simple SimulatedPort tests. Correcting
physical address numeric packing is intentional; update consumers and generated
fixtures with independent golden validation. Protobuf additions are additive;
fixed ProcBuf layout/semantic changes require an ABI bump and regenerated hashes.
Each milestone receives focused validation before whole-workspace gates.
