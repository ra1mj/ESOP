# EtherCAT Wire Compatibility

## 1. Scope / Trigger

Use for wire/register/address/RX/scan/DC changes or new frame fixtures.

## 2. Signatures

```rust
DatagramHeader::new(command, index, address, length); // last=true => M=0
fixed_address(station, ado); // station low WORD, ADO high WORD
EthercatMaster::new_with_bus_transit_count(config, count);
DcCyclicSync::new_with_bus_transit_count(config, monitor, count);
```

Existing RX APIs retain exact defaults. Explicit response policies distinguish
exact address, known topology position increment, and initial bounded BRD count.

## 3. Contracts

- M=0 means last; M=1 means more. Encode/decode/iteration/FrameBuilder must agree
  with independent literal bytes. Physical ADP then ADO are little-endian WORDs.
- First scan BRD reads ESC Type with exact ADO/length/index/generation/deadline;
  require returned position delta == WKC <= capacity. A returned empty bus is
  different from no response. Freeze the count for this scan generation.
- AP/BR returned ADP increments by that whole-bus count, independently of WKC.
  Fixed/logical addresses remain exact. In particular DC BRD WKC may count only
  DC-capable slaves while its ADP passes every slave.
- Ordinary probes before N require WKC1; terminal AP positionN requires WKC0.
  Early zero, changed count, malformed reply and timeout fault the scan.
- Check the same response policy in RX and control completion. Use control wire
  leases for active cyclic coexistence; see ethercat-control-index-leases.md.
- LRD/LWR WKC counts a successful direction once per physical ESC/datagram,
  not once per FMMU. A PDO input plus mapped mailbox-status FMMU on one ESC still
  contributes1 to LRD. LRW contributes read1/write2; ARMW/FRMW each selected read
  or nonselected write contributes1.
- Keep raw SII bytes. Only when a frozen odd expected FMMU usage count explains
  exactly one final Unused byte may Startup normalize WORD alignment before
  signature comparison. Preserve internal/explicit unused slots and reject
  nonzero padding or other count mismatches.

## 4. Validation & Error Matrix

Wrong command/address/size/generation/WKC/deadline rejects before publication.
Missing BRD/AP response faults rather than completing a partial scan. Excess
transit count rejects before static storage is indexed. Wrong FMMU usage
signature remains a configuration fault after alignment normalization.

## 5. Good / Base / Bad Cases

Good: a bus of8 with2 DC-capable slaves returns a DC BRD ADP increment8 and WKC2.
Base: a fixed-station read retains its exact full address. Bad: infer bus count
from each datagram's WKC, or test a codec only by decoding its own encoding.

## 6. Tests Required

Independent golden bytes for all 14 supported non-NOP opcodes, multi-datagram M handling, physical
address order, malformed frames, counts0/1/8/32, changed count/early disappearance,
explicit transit-vs-WKC distinction, all-or-nothing Domain input, and odd SII
padding with nonzero/internal-unused negative controls.

## 7. Wrong vs Correct

Wrong: update an echo simulator to reproduce an incorrect master layout.
Correct: literal bytes and an independent ESC model identify which side violates
the protocol, then update code and affected generated evidence together.
