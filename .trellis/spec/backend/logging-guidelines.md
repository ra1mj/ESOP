# Logging Guidelines

> The real-time workspace has no logging dependency; diagnostics are typed
> data and fixed-capacity event queues.

## Overview

The `no_std` crates do not use `log`, `tracing`, `println!`, or a text logger.
Protocol and lifecycle faults are returned as enums or placed in bounded
diagnostic/event rings (`esop-ethercat-core/src/diag.rs` and
`esop-procbuf/src/lib.rs`).

## Log Levels

Not applicable inside libraries. The Linux-only observation example uses
`eprintln!` at the process boundary for preflight, incident, and poll reports.

## Structured Logging

Use typed records with fixed fields when information must cross a boundary;
do not format text in the activated cycle. `RuntimeEvidence` and diagnostic
events are the reference structured forms.

## What to Log

At the outer Linux/application boundary, report operational failures and
bounded observation summaries. Inside the core, expose a typed diagnostic
event or error instead.

## What Not to Log

- Do not add text logging, formatting, or unbounded output to cycle, DMA, PDO,
  lifecycle, or ProcBuf paths.
- Do not log raw process-image buffers or secrets merely for debugging.
