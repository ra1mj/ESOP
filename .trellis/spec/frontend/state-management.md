# State Management

> Not applicable: the repository has no frontend state-management layer.

## Current Scope

Runtime state is owned by typed Rust structs, fixed-capacity rings, and
caller-owned buffers. ProcBuf is an ABI/data exchange layer, not browser or
server state management.

## State Categories

There are no frontend local, global, URL, or server-state categories.

## When to Use Global State

Not applicable.

## Server State

Not applicable.

## Common Mistakes

Common mistake: treating ProcBuf snapshots as an unrestricted shared-state
store or adding an unbounded cache to the real-time path.
