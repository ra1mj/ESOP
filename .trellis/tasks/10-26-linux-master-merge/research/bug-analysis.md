# Boundary review findings

## 1. Root Cause

Implicit lifecycle/ownership assumptions remained at terminal and time boundaries:
idle mailbox failure did not relinquish its scheduler flag; a monitor's fresh
boolean outlived its evaluation time; a reused wire index overwrote a pending
RTT timestamp; and a latency-complete baseline was assumed to be a successful
unloaded software run. Unknown CLI scenarios reached an unintended product.

## 2. Why Previous Coverage Missed These Cases

Successful idle receive, freshness at report creation, loss-free RTT matching,
successful generated baselines and known scenario names all passed. Those tests
did not delay optional work beyond idle deadlines, advance time between monitor
evaluation and actual output, overlap unresolved wire indices, or import an
actual failed baseline with valid histogram/sample metadata.

## 3. Prevention

- Terminal idle transport failure releases idle ownership without extending SDO
  deadlines or granting lifecycle recovery. Test real optional-service delay.
- Carry monitor evaluation and expiry times, and recheck through the actual
  cycle owner while retaining accepted TX/Controlword evidence.
- Keep ambiguous RTT unavailable until a new observer session; count actual
  observed cycles instead of filling missing timing with zero.
- Require successful allocation-free, zero-SDO baseline evidence; validate
  reported RTT coverage against its histogram, including zero and legacy cases.
- Canonicalize supported scenarios before port setup; bind the same name into
  capture, run and imported topology artifacts.

## 4. Scope

These fixes improve software contracts. None establishes real hardware source
authenticity, physical NIC timestamps, target timing or MCU qualification.

## 5. Knowledge Capture

Mailbox, monitoring and software-run evidence specifications are updated beside
the executable regressions. No generated Trellis template copy applies here.
