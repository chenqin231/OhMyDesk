---
name: rust-patterns
description: Rust patterns for ownership, error handling, traits, async code, Cargo workspaces, and safe systems programming.
triggers:
  keywords:
    primary: [Rust, rust, cargo, ownership, borrow checker]
    secondary: [trait, lifetime, Result, Option, tokio, serde]
  context_boost: [.rs, Cargo.toml, Cargo.lock]
  context_penalty: [.go, .py, .ts, .java]
  priority: high
tier: optional
stacks: [rust]
---

# Rust Development Patterns

Use this skill when writing, reviewing, or refactoring Rust code.

## Core Principles

- Model ownership explicitly; prefer borrowing over cloning when it keeps lifetimes simple.
- Use `Result<T, E>` for recoverable failures and reserve `panic!` for invariant violations.
- Keep public APIs small and trait bounds readable; move complex bounds into helper traits or where clauses.
- Prefer domain-specific error enums for libraries and contextual error wrapping for applications.
- Use iterators for clear data transformations, but choose straightforward loops when control flow matters.
- Keep unsafe code isolated, documented, and covered by tests that exercise boundary conditions.

## Cargo And Module Layout

- Put shared library logic in `src/lib.rs`; keep `src/main.rs` thin for binaries.
- In workspaces, centralize dependency versions in the root `Cargo.toml` when supported.
- Keep feature flags additive; avoid features that silently change behavior in incompatible ways.
- Run `cargo fmt`, `cargo clippy --all-targets --all-features`, and `cargo test --all-features` before delivery.

## Async Rust

- Do not block inside async tasks; use runtime-specific blocking helpers for CPU or blocking IO work.
- Pass cancellation through task boundaries with explicit signals or dropped futures.
- Keep lock scopes short, especially across `.await`.

## Review Checklist

- Ownership choices are intentional and do not hide unnecessary clones.
- Error messages preserve enough context to debug failures.
- Public types and traits are documented when exported outside the crate.
- Tests cover both success paths and representative error paths.
