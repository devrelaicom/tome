//! Lint rule registry + runner — the shared validation core for `lint` and
//! `convert` (which folds lint diagnostics into its report).
//!
//! The [`rules`] module holds the individual rules (each an `id` / `severity` /
//! `scope` / `autofixable` + a fn producing `Vec<Diagnostic>`). The runner here
//! visits **all** nested IR (catalog → plugins → entries) — it never stops at
//! the first failure — aggregates the diagnostics, and computes one verdict
//! (errors > strict-warnings > clean). `--autofix` applies the `autofixable`
//! fixes per-file via atomic replace with `first_error` forward-progress; the
//! `--json` shape is a single `{ findings[], summary }` object.
//!
//! Framework lands in Phase 2 (Foundational); concrete rules + autofix in
//! Phase 5 (US3).

pub mod rules;
