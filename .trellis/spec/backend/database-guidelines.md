# Database Guidelines

> This project currently has no database or persistent backend service.

## Current Scope

There is no ORM, SQL driver, migration directory, or database package in the
workspace. Runtime state is held in fixed-capacity Rust structs and caller-
owned buffers such as `DomainRegistry`, `DeviceManager`, and `ProcBuf`.
Do not add database dependencies to a `no_std` crate to persist cycle data.

## Query Patterns

Not applicable. Read/write operations are typed methods on in-memory state;
they return `Result` and validate before publishing changes.

## Migrations

Not applicable. A future persistence feature must be a separate package with
an explicit design and migration policy before it is introduced.

## Naming Conventions

There are no table or column names. Rust package names use the `esop-` prefix;
domain fields use explicit snake_case names.

## Common Mistakes

- Treating ProcBuf or a process image as a database. They are fixed-layout
  real-time exchange buffers, not durable storage.
- Introducing heap-backed caches or a blocking persistence call into the
  activated EtherCAT cycle.
