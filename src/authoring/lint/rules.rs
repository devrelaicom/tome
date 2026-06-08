//! The concrete lint rules (`data-model.md §9`): manifest validity, skill
//! `name == dir` (autofixable), Agent-Skills conformance (description
//! present / ≤ 1024 chars), residual harness-isms + legacy `$1..$9`
//! (autofixable subset), unsupported-component directories, and
//! `--into`/catalog name consistency.
//!
//! Each rule declares `id: &'static str`, `severity`, `scope`
//! (`Catalog`/`Plugin`/`Entry`), and `autofixable: bool`. Populated in
//! Phase 5 (US3).

use super::Rule;

/// The full rule registry shared by `convert` (folds the diagnostics into its
/// report) and `lint` (the command). The concrete rules land in US3; until
/// then this is empty, so `lint::run` aggregates only the IR-carried
/// import/rewrite diagnostics — exactly what `convert` needs in the interim.
pub fn all() -> Vec<Box<dyn Rule>> {
    Vec::new()
}
