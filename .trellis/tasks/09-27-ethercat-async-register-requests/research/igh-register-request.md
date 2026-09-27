# IgH Register Request Reference

## Primary Source

IgH EtherCAT Master 1.6 `reg_request.c` implements reusable request objects
with a caller-visible state machine (`UNUSED`, `BUSY`, `SUCCESS`, `ERROR`), a
fixed data buffer allocated at creation, and explicit asynchronous read/write
queueing. Reads record station address, register offset and transfer size;
writes reuse the configured buffer and configured size.

Source inspected on 2026-09-27:
`https://docs.etherlab.org/ethercat/1.6/doxygen/reg__request_8c_source.html`

## ESOP Adaptation

- Preserve the observable asynchronous state shape, but use caller-owned
  fixed-capacity arrays instead of allocation.
- Add stable generation handles because a shared controller owns multiple
  reusable slots.
- Keep explicit per-request deadlines and exact WKC/action validation required
  by the existing ESOP control plane.
- Execute through the existing production service scheduler so register access
  cannot create an unbudgeted side channel around PDO/DC traffic.
