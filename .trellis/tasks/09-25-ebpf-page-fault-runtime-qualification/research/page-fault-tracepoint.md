# Page-fault tracepoint research

## Production path already present

- `bpf/esop_runtime.bpf.c` attaches `esop_page_fault_user` to
  `exceptions:page_fault_user`, filters by tracked TGID, increments
  `page_faults`, aggregates in the fixed `ESOP_PAGE_FAULTS` LRU map keyed by
  CPU/TGID, and emits only the first threshold crossing.
- The emitted detail is the saturated architecture `error_code`; the existing
  fixed evidence record does not carry tracepoint address or instruction
  pointer.
- `crates/esop-ebpf-agent/src/lib.rs` requires count threshold plus cycle risk
  and maps qualifying evidence to Warning `HostPageFault`,
  `DegradeHostObservation`, confidence 65.

## Kernel source facts

- Linux `include/trace/events/exceptions.h` declares a shared exception event
  with `address`, `ip`, and `error_code`, then defines `page_fault_user` from
  that class:
  https://github.com/torvalds/linux/blob/master/include/trace/events/exceptions.h
- Linux trace-event generation materializes the raw event class fields in the
  `trace_event_raw_*` record consumed by BPF:
  https://github.com/torvalds/linux/blob/master/include/trace/trace_events.h
- The x86 page-fault handler treats `error_code` as the hardware reason bits;
  a user write to a non-present anonymous page is expected to carry user and
  write bits without the protection/present bit:
  https://github.com/torvalds/linux/blob/master/arch/x86/mm/fault.c

## Qualification decision

- Use an exec child that is fully prepared before BPF load, then track only its
  TGID. This isolates the parent runtime/allocator from page-fault statistics.
- Pin the child to one CPU because production aggregation is per CPU/TGID.
- First-write 16 untouched anonymous pages and independently require
  `ru_minflt` delta 16. The BPF record remains an exception-count observation;
  the supporting process counter must not be used to expand production event
  semantics.
- Scope the exact error detail to hosted x86_64. Architecture portability and
  major/minor attribution remain open.
