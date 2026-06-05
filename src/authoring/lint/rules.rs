//! The concrete lint rules (`data-model.md §9`): manifest validity, skill
//! `name == dir` (autofixable), Agent-Skills conformance (description
//! present / ≤ 1024 chars), residual harness-isms + legacy `$1..$9`
//! (autofixable subset), unsupported-component directories, and
//! `--into`/catalog name consistency.
//!
//! Each rule declares `id: &'static str`, `severity`, `scope`
//! (`Catalog`/`Plugin`/`Entry`), and `autofixable: bool`. Populated in
//! Phase 5 (US3).
