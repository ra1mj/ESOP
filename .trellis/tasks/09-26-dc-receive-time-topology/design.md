# Design: DC receive-time topology and propagation delay

## Architecture

The scanner remains the only owner of discovery register requests. A pure
fixed-capacity projection in `dc.rs` consumes completed `ScanRecord` values;
Startup publishes the projection only after product DC policy and reference
selection succeed.

```text
APRD base data
  -> APWR station address
  -> FPRD 0x0910 (DC only, WKC 0/1 capability probe)
  -> FPRD 0x0900 / 16 (every base-DC slave, exact WKC 1)
  -> FPRD 0x0110 / 2 (every slave, exact WKC 1)
  -> existing ESC Configuration / AL Status
  -> complete scan records
  -> Startup validates DC profiles and stages reference
  -> DcTopology builds physical tree and delay graph
  -> atomic reference + topology publication
  -> identity/SII/AL path
```

## Scan Data Contract

`ScanPortLink` stores the three decoded Data Link Status flags. `ScanRecord`
retains the raw `dl_status` and four `ScanPortLink` values. The raw port
descriptor remains unchanged.

`ScanDcCapabilities` gains `receive_times: Option<[u32; 4]>`. `None` means the
slave did not advertise base DC or the record has not completed; it never
means four measured zero timestamps. A System-Time WKC-0 response advances to
receive-time acquisition rather than skipping it.

`ReadingDcReceiveTimes` is exact WKC 1. `ReadingDataLinkStatus` is exact WKC 1
for every slave. The existing zero-or-one WKC policy stays limited to probing
and `ReadingDcSystemTime`.

## Topology Types

`DcTopology<MAX_SLAVES>` owns a fixed array of `DcTopologySlave` and a length.
Each slave contains:

- scan position and station address;
- DC/System-Time capability flags;
- four `DcTopologyPort` values carrying scan link flags and optional receive
  time;
- optional physical adjacent slave per port;
- optional adjacent DC peer and one-way propagation delay per measurable
  DC-to-DC edge;
- optional cumulative transmission delay from the selected reference.

The public API returns slices and position lookups; no caller receives mutable
access to published evidence.

## Physical Tree Algorithm

The input array is the bounded scan/ring order. An explicit fixed-size stack
walks it as preorder:

1. record zero is the root and has no physical neighbor at port 0;
2. each child's port 0 points to its parent;
3. downstream ports are considered in order `[3, 1, 2]`;
4. a downstream port whose `loop_closed` flag is false consumes the next scan
   record and pushes it as a child;
5. completion requires every record to have been consumed exactly once.

The stack avoids recursion and its capacity is the same generic bound as the
projection. Errors are returned before the local candidate is assigned to
Startup.

## Delay Algorithm

For each DC-capable source and connected downstream port:

1. walk the physical subtree in deterministic port order to find the first
   DC-capable slave;
2. compute source round-trip delta between the current port receive time and
   the previous connected port, using `u32::wrapping_sub`;
3. sum the first downstream DC slave's connected downstream-port deltas with
   checked addition;
4. subtract that internal sum from the source delta with checked subtraction;
5. divide by two and publish the edge symmetrically on source and peer ports.

The previous connected-port order is the inverse EtherCAT order `[2, 3, 1, 0]`
with port 0 as fallback. Integer division follows the established register
algorithm; no fractional nanosecond is invented.

After link construction, an iterative graph walk starts at the selected
reference with cumulative delay zero and follows both upstream and downstream
DC edges. Checked addition rejects cumulative overflow. DC slaves outside this
measurable graph retain `None`.

## Startup Transaction

`enter_identity_phase()` performs these stages using locals:

1. verify scan count;
2. validate every profile and stage explicit/fallback reference;
3. build the complete `DcTopology` candidate using the staged reference;
4. verify every profile requiring DC has cumulative delay evidence;
5. assign selected reference and topology together;
6. enter identity or Ready for an empty topology.

Any error uses `StartupError::DcTopology` or
`StartupError::DcPropagationDelayRequired`, invokes the existing fault latch,
and leaves published fields empty. `start_inner()` and test-only barrier setup
reset topology alongside the existing reference evidence.

## Compatibility

- Public scan records gain additive fields; existing capability predicates and
  product schema remain unchanged.
- Non-DC products receive physical topology with no DC timestamps or delays.
- Optional DC slaves may expose incomplete cumulative evidence without
  blocking startup. A product profile that declares DC requirement is stricter
  and fails before identity if propagation delay is not measurable.
- No ProcBuf, protobuf, cfggen schema, generated artifact, or cyclic frame-plan
  ABI changes are required.

## Failure Semantics

Errors distinguish scan transport/shape failures, topology structure failures,
delay underflow/overflow, invalid reference, and required-but-unmeasurable
product evidence. Candidate construction mutates only a local fixed array;
published Startup state changes after all stages succeed.

## Rollback

The added scan phases and topology types can be reverted together without data
migration. Product policy is unchanged. A partial rollback that keeps receive-
time claims while removing exact DL-status/topology validation is not valid.
