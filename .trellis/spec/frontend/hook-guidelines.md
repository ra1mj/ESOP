# Hook Guidelines

> Not applicable: no frontend or hook runtime exists in this repository.

## Current Scope

There are no custom hooks, data-fetching libraries, or browser lifecycle APIs.
Keep future UI state integration outside the `no_std` crates.

## Custom Hook Patterns

Not applicable.

## Data Fetching

Not applicable.

## Naming Conventions

Not applicable.

## Common Mistakes

Do not introduce a hook library as an indirect dependency of protocol or
lifecycle code.
