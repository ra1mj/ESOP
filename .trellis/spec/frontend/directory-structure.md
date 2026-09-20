# Directory Structure

> No frontend code exists in this repository.

## Current Scope

The workspace contains Rust crates under `crates/`, BPF C sources under
`bpf/`, and documentation under `docs/`. There is no web application,
component tree, asset directory, or frontend build tool.

## Directory Layout

See [backend directory structure](../backend/directory-structure.md). A future
frontend must be a separately planned package with its own build and test
commands.

## Module Organization

Not applicable until a frontend package is introduced.

## Naming Conventions

No frontend naming convention is established. Do not infer React/Vue or
TypeScript conventions from this Rust repository.

## Examples

There are no frontend examples. The closest user-facing boundary is the Linux
observation executable at `crates/esop-ebpf-runtime/examples/observe.rs`.
