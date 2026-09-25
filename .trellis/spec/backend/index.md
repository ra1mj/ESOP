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
| [Quality Guidelines](./quality-guidelines.md) | Code standards, forbidden patterns | Filled |
| [Logging Guidelines](./logging-guidelines.md) | Structured logging, log levels | Filled |
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
