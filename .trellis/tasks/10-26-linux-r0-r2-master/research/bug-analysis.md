# Bug Analysis: self-consistent protocol fixtures hid wire incompatibilities

## 1. Root Cause Category

D (coverage gaps), E (implicit protocol assumptions), and B (cross-layer
ownership): echoed frames reused the same encoder/parser and expected WKC as
the implementation. M-bit inversion, physical WORD reversal, independent
mailbox counters, SM terminal-byte ownership and per-ESC WKC were untested.

## 2. Why Earlier Evidence Was Insufficient

Codec roundtrips proved internal agreement, not EtherCAT bytes. Scripted
controller completions did not consume real mailbox buffers or mutate AP/BR
positions. Short examples never exhausted the logical control-index namespace.
One optional service plus one yield flag did not exercise mixed-class starvation.

## 3. Prevention Mechanisms

| Priority | Mechanism | Status |
| --- | --- | --- |
| P0 | Literal independent wire golden tests for every command | Implemented |
| P0 | Wire-only virtual ESC with actual WKC, SM/PDO/CoE behavior | Implemented |
| P0 | Reserved logical-to-wire control index leases and 1024-action coexistence | Implemented |
| P0 | Lazy runtime action creation and bounded optional-class round robin | Implemented |
| P0 | Mapped mailbox evidence sampled after shared RX commit | Implemented |
| P0 | Per-request terminal event queue, observable overflow and exact handles | Implemented |
| P0 | Full generated-product startup, motion, stop/reset/rearm and replay | Implemented; qualification work ongoing |

## 4. Systematic Expansion

Keep protocol shape, product metadata, resource reports and simulator semantics
independently checked. Do not make the simulated device follow a wrong master
expectation to turn a regression green. Keep wire count separate from topology
count and logical action identity separate from a leased wire index. Freeze
absolute request deadlines; budget delays cannot silently extend them.

## 5. Knowledge Capture

Executable contracts live in backend wire/control-index/mailbox/monitoring/
maintenance/evidence specifications. No generated Trellis template tree exists
in this application repository, so no template-copy mutation applies here.
