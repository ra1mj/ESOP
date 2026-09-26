# EtherCAT DC offset/delay research

## Primary references

- Beckhoff ET1100 EtherCAT Slave Controller datasheet, register descriptions
  for System Time Offset `0x0920:0x0927` and System Time Delay
  `0x0928:0x092B`.
- Beckhoff Distributed Clocks documentation, propagation-delay and local-time
  offset behavior.
- EtherLab/IgH EtherCAT master FSM source, used only as an external behavior
  reference for reading System Time plus old offset and calculating 32/64-bit
  corrections. The ESOP implementation remains independently structured.

## Derived implementation constraints

- Offset is a signed correction represented in a 64-bit ESC register; delay is
  an unsigned 32-bit nanosecond value.
- Offset and delay should be written together in one access so a slave does not
  observe a mixed pair.
- Reading 24 bytes from `0x0910` yields the current System Time and prior offset
  needed for incremental correction.
- A 64-bit clock uses the full signed application-minus-system delta.
- A 32-bit clock uses the signed low-32-bit wrapping delta.
- Elapsed monotonic time between the caller's application-time sample and the
  accepted ESC sample must be added before calculating the correction.

## Claim boundary

These register operations initialize the software-visible DC relationship;
they do not prove the external application-time source, cable/device timing
precision, ongoing clock lock, or hardware conformance.
