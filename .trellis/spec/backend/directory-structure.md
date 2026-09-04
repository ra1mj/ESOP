# Directory Structure

This repository is a Rust workspace for the EtherCAT core and its platform,
profile, lifecycle, and observability adapters. There is no HTTP backend or
database layer.

## Directory Layout

```text
Cargo.toml                         workspace members and shared metadata
Makefile                           canonical local quality-gate commands
crates/esop-ethercat-core/         protocol core; must remain no_std
crates/esop-ethercat-linux-port/   Linux AF_PACKET and deterministic simulator
crates/esop-procbuf/               fixed-layout no_std command/state ABI
crates/esop-device/                fixed-capacity device lifecycle contract
crates/esop-ebpf-agent/            fixed ABI and incident correlation facade
crates/esop-ebpf-runtime/          Linux-only Aya loader and ringbuf decoder
crates/esop-lifecycle-guard/       fail-closed lifecycle and permit guard
crates/esop-profile-cia402/        CiA 402 profile logic
crates/*/src/                       library implementation and unit tests
crates/*/tests/                     public integration and HIL tests
bpf/                                kernel BPF source and CO-RE build entrypoint
docs/                               requirements, ADRs, and PRDs
.github/workflows/                  CI quality-gate definitions
```

## Module Organization

Protocol and real-time behavior belongs in `crates/esop-ethercat-core/src/`.
Platform code belongs in a separate crate and implements the small port traits
from the core; for example, `src/lib.rs` in the Linux port owns raw socket
details while `src/sim.rs` owns deterministic test behavior. Linux eBPF loading
must stay in `crates/esop-ebpf-runtime/` and must not become a dependency of
the `no_std` core.

Use `src/` unit tests for bounded module invariants and `tests/` for public
cross-module flows. The DMA TX/RX lifecycle is covered by
`crates/esop-ethercat-core/tests/cycle.rs`; the public simulator flow is
covered by `crates/esop-ethercat-linux-port/tests/master_sim.rs`.

## Naming Conventions

Use snake_case for Rust modules and files, PascalCase for types, and explicit
domain names such as `DmaDescriptorRing`, `EthercatPort`, and
`DmaReceiveCycle`. Keep workspace package names prefixed with `esop-`. Keep
platform-specific dependencies behind the relevant crate or target
configuration.
