//! Template scaffolding for `create`. Resolves a template (built-in name,
//! embedded via `include_str!` → local dir / `owner/repo` / git URL via
//! `catalog::source::resolve`), fetches remote templates through the `git`
//! shell-out into a `TempDir` (cleanup guaranteed on every exit path), and
//! renders placeholder variables with **minijinja** into IR/files.
//!
//! minijinja's `{{ }}` delimiters never collide with the emitted runtime
//! `${TOME_*}`/`$ARGUMENTS` tokens, so those survive into the scaffold
//! verbatim. The `name == directory` invariant is enforced at creation.
//!
//! Populated in Phase 6 (US4).
