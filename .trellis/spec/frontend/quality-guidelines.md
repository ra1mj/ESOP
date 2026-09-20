# Quality Guidelines

> Not applicable: there is no frontend package or frontend CI in this repo.

## Current Scope

The canonical quality command is the Rust workspace `make ci`; it does not
build or test a browser application.

## Forbidden Patterns

Do not add frontend tooling or dependencies to the real-time crates without a
separate package and task.

## Required Patterns

Not applicable.

## Testing Requirements

If a frontend is introduced, define its lint, type-check, test, and
accessibility gates in this file before implementation.

## Code Review Checklist

For current changes, review `make ci`, no_std boundaries, fixed-capacity
contracts, and public regression tests as described in the backend quality
guidelines.
