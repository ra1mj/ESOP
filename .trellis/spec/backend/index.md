# Backend Development Guidelines

> Best practices for backend development in this project.

---

## Overview

This directory documents conventions for the Rust workspace. The project has
no HTTP server, database, or conventional request/response backend; most code
runs in `no_std` protocol, lifecycle, and profile crates.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | Filled |
| [Database Guidelines](./database-guidelines.md) | Persistence and database scope | Not applicable |
| [Error Handling](./error-handling.md) | Error types, handling strategies | Filled |
| [Wire Compatibility](./ethercat-wire-compatibility.md) | Literal wire semantics, topology-aware RX and independent fixtures | Filled |
| [Runtime Mailbox](./ethercat-mailbox-runtime.md) | MRL, CoE, shared transport authority, bounded SDO and Emergency | Filled |
| [Quality Guidelines](./quality-guidelines.md) | Code standards, forbidden patterns | Filled |
| [EtherCAT Control Index Leases](./ethercat-control-index-leases.md) | Frozen wire namespace independent of wrapping logical FSM indices | Filled |
| [EtherCAT Register Requests](./ethercat-register-requests.md) | Fixed-capacity asynchronous ESC register request contract | Filled |
| [EtherCAT State Requests](./ethercat-state-requests.md) | Bounded explicit runtime ESM request contract | Filled |
| [EtherCAT Explicit Rescan](./ethercat-rescan.md) | Bounded retained-plan PREOP topology verification contract | Filled |
| [EtherCAT Single-Slave Reconfiguration](./ethercat-reconfiguration.md) | Target-isolated retained-evidence PREOP reconfiguration contract | Filled |
| [EtherCAT SyncManager Activation](./ethercat-sync-manager-activation.md) | Separate SII OpOnly metadata from physical ESC writable activation bits | Filled |
| [EtherCAT Slave Monitoring](./ethercat-slave-monitoring.md) | Bounded age-qualified runtime AL/DL observations and lifecycle projection | Filled |
| [EtherCAT Maintenance Fault Reset](./ethercat-maintenance-fault-reset.md) | Explicit owner-bound PDS reset pulse without motion authority | Filled |
| [EtherCAT Recovery Integration](./ethercat-recovery-integration.md) | Unified immutable snapshots, diagnostics and mixed-load evidence | Filled |
| [Logging Guidelines](./logging-guidelines.md) | Structured logging, log levels | Filled |
| [Software Run Evidence](./software-run-evidence.md) | Bounded measurement import, explicit release qualification and optional I2C | Filled |
| [Product Configuration](./product-configuration.md) | cfggen, static artifacts and build-report contract | Filled |

---

## Maintenance

For each guideline file:

1. Document your project's **actual conventions** (not ideals)
2. Include **code examples** from your codebase
3. List **forbidden patterns** and why
4. Add **common mistakes** your team has made

Keep these documents aligned with the code and update them when a new crate,
boundary, or quality gate is introduced.

---

**Language**: All documentation should be written in **English**.
