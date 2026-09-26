## Purpose

Guide agents and contributors making changes in this Rust repository.

## Goals

* Keep changes small, clear, focused, and easy to review.
* Favor readability, correctness, and maintainability over cleverness.
* Preserve behavior unless fixing a clear defect, reliability issue, or unsafe behavior.
* Document important assumptions, tradeoffs, risks, and unresolved issues.

## General Guidelines

* Read relevant files, call sites, public APIs, and documentation before changing code.
* Prefer targeted fixes over broad rewrites, unrelated cleanup, or speculative refactors.
* Follow existing structure and conventions unless they cause a significant problem.
* Fix root causes when practical.
* Do not rename public APIs, files, modules, functions, types, or configuration fields without a clear reason.
* Avoid new dependencies unless they provide a clear, justified benefit.

## Rust Design and Code Health

* Use idiomatic, safe Rust and the type system to make invalid states difficult to represent.
* Prefer compile-time guarantees over runtime checks.
* Use enums instead of boolean mode flags and newtypes when primitives could be confused.
* Keep public APIs small and intentional; prefer private or `pub(crate)` visibility.
* Use `#[must_use]` when ignoring a return value is likely to be a bug.
* Keep files, modules, functions, and implementations focused on one responsibility.
* Split large components only when it creates clearer boundaries.
* Remove duplicated, dead, obsolete, or unnecessary code.
* Reduce deep nesting with early returns, pattern matching, or focused helpers.
* Replace meaningful magic values with named constants.
* Avoid unnecessary abstractions, builders, helpers, and re-exports.
* Prefer straightforward loops when iterator chains reduce clarity.

## Ownership and Performance

* Prefer borrowing over cloning when ownership is unnecessary.
* Use `&str`, slices, and references for borrowed inputs; use owned values for stored data.
* Do not use `clone()` merely to satisfy the borrow checker; improve ownership boundaries.
* Avoid unnecessary allocations, conversions, parsing, computation, and intermediate collections.
* Preallocate collections when size is known or reliably estimated.
* Move invariant work outside loops and hot paths.
* Prioritize algorithmic, allocation, I/O, and concurrency improvements over micro-optimizations.
* Do not claim performance gains without clear evidence or measurement.

## Error Handling

* Do not use `unwrap()` in production runtime code.
* Use `expect()` only for true invariants and include a specific explanation.
* Prefer `Result`, `?`, and explicit handling over panics, unchecked indexing, or discarded errors.
* Preserve useful context and distinguish invalid input, recoverable failures, invariants, and fatal conditions.
* Prefer structured errors when callers may need to inspect them.
* Use `thiserror` for domain errors and `anyhow` at application boundaries when appropriate.
* Do not log and return the same error at multiple layers unless each adds context.

## Async and Concurrency

* Use the async runtime already adopted by the project.
* Do not block executor threads with synchronous I/O, sleep, or CPU-heavy work.
* Do not hold synchronous locks or unnecessary guards across `.await`.
* Review spawned tasks for cancellation, ownership, error propagation, and bounded concurrency.
* Avoid detached tasks, unbounded queues, buffers, channels, or task creation.
* Minimize lock scope and prefer clearer ownership or message passing when practical.

## Documentation and Safety

* Update documentation when behavior, setup, configuration, architecture, or usage changes.
* Keep documentation focused on high-level concepts, important behavior, and operational guidance. Link to source for implementation details; avoid recording routine UI layout, styling, or control-placement changes unless explicitly requested.
* Keep README examples accurate.
* Document public API behavior, errors, panics, safety requirements, and non-obvious invariants.
* Add comments only for reasoning, tradeoffs, invariants, or non-obvious behavior.
* Do not introduce `unsafe` unless essential; keep blocks small and document soundness.
* Do not commit secrets, tokens, credentials, or sensitive data.
* Validate input at system boundaries and call out destructive or risky changes.

## Testing and Validation

* Do not add or modify tests unless explicitly requested.
* Run the most relevant existing checks and do not ignore failures.
* Do not silence Clippy or add broad lint allowances without documented justification.
* Before submitting, run as applicable:

```bash
rtk cargo fmt -- --check
rtk cargo check
rtk cargo clippy --all-targets --all-features -- -D warnings
rtk cargo build
rtk cargo test
```

## Git and Final Report

* Keep commits and pull requests scoped to one logical change with clear messages.
* Report what changed, why, validation, behavior changes, significant improvements, dependencies, unresolved findings, risks, and follow-up work.
* Do not include fluff or a file-by-file narration of trivial edits.

## graphify

* Read `graphify-out/GRAPH_REPORT.md` before architecture or codebase analysis.
* Use `graphify-out/wiki/index.md` when available instead of raw files.
* Prefer `graphify query`, `graphify path`, or `graphify explain` for cross-module questions.
* After modifying code, run `rtk graphify update .`.

## When Unsure

* Ask for clarification on product intent rather than guessing.
* Otherwise choose the smallest safe change and document assumptions.
