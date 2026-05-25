//! Harness module trait + static registry.
//!
//! Phase 4 introduces the harness abstraction: every coding harness Tome
//! integrates with (Claude Code, Codex, Gemini, Cursor, OpenCode) is exposed
//! as a `HarnessModule` impl living in its own file under this directory.
//! The sync algorithm, settings composition resolver, and harness commands
//! all dispatch through the trait — adding (or rewriting) a harness changes
//! one file under `src/harness/`, never `commands/`, `sync/`, or the
//! settings parser.
//!
//! ## Module layout
//!
//! - `mod.rs` (this file) — the `HarnessModule` trait, the four shape
//!   enums (`RulesFileStrategy`, `BlockBodyStyle`, `McpConfigFormat`),
//!   the `MCP_CONFIG_KEY` static, the `SUPPORTED_HARNESSES` registry,
//!   `lookup`, and the `HARNESS_MODULES_OVERRIDE` test-injection hook.
//! - `rules_file.rs` — the read/modify/write helpers for the two
//!   rules-file strategies (`BlockInExistingFile` + `StandaloneFile`).
//!   Skeleton only — production wiring lands in US3.c / US4.
//! - `mcp_config.rs` — the read/modify/write helpers for harness MCP
//!   configuration files (JSON + TOML). Skeleton only.
//! - `claude_code.rs`, `codex.rs`, `cursor.rs`, `gemini.rs`,
//!   `opencode.rs` — the five concrete `HarnessModule` impls. Each
//!   harness's path / format / parent-key decisions are pinned per
//!   research §R-8 and verified against the upstream harness docs at
//!   implementation time.
//!
//! Sync-only — `tests/sync_boundary.rs` enforces the constitution's
//! sync discipline on this tree. No async runtime imports, no await
//! points.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod gemini;
pub mod mcp_config;
pub mod opencode;
pub mod rules_file;
pub mod stub;
pub mod sync;

use claude_code::CLAUDE_CODE;
use codex::CODEX;
use cursor::CURSOR;
use gemini::GEMINI;
use opencode::OPENCODE;

#[doc(hidden)]
pub use stub::StubHarness;

/// Standardised MCP entry key written by Tome across every harness.
///
/// JSON harnesses nest it under `mcpServers`; TOML harnesses (Codex) nest
/// it under `mcp_servers`. The leaf key is always `"tome"`.
pub static MCP_CONFIG_KEY: &str = "tome";

/// Rules-file integration strategy per data-model §10.
///
/// Most harnesses (`claude-code`, `codex`, `gemini`, `opencode`) own a
/// delimited block inside an existing rules file. Cursor instead owns a
/// dedicated standalone file under `.cursor/rules/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulesFileStrategy {
    /// Tome owns a `<!-- tome:begin --> … <!-- tome:end -->` block
    /// inside an existing developer-authored rules file. Content outside
    /// the markers is preserved verbatim across syncs.
    BlockInExistingFile,
    /// Tome owns a complete file at the harness's chosen path. Removal
    /// deletes the file. No markers, no surrounding content.
    StandaloneFile,
}

/// Body content style for the `BlockInExistingFile` strategy.
///
/// Harnesses that support `@`-style includes (`claude-code`, `codex`,
/// `gemini`) get a single-line include directive pointing at the project
/// marker's `RULES.md`. Harnesses without documented include support
/// (`opencode`) get the full rules text inlined between the markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockBodyStyle {
    /// `@<relative-path-to-.tome/RULES.md>` — the sync algorithm
    /// computes the relative path at write time.
    AtInclude,
    /// Full rules content verbatim — the block must be rewritten on
    /// every summary regeneration.
    Inline,
}

/// Serialisation format for the harness's MCP configuration file.
///
/// JSON harnesses use `serde_json` with the project-wide `preserve_order`
/// feature; the TOML harness (Codex) uses `toml_edit` to preserve
/// comments and key order. See `mcp_config.rs` for the strict-vs-lenient
/// boundary commentary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConfigFormat {
    /// `serde_json` with the `preserve_order` feature.
    Json,
    /// `toml_edit` (comment- and order-preserving).
    Toml,
}

/// One coding harness Tome can integrate with.
///
/// Trait methods cover three concerns:
///
/// 1. Identity (`name`, `description`).
/// 2. Detection (`detect`) — filesystem existence-only per FR-167.
/// 3. Per-harness specifics (`rules_file_*`, `mcp_config_*`) consulted by
///    the sync algorithm and the settings composition resolver.
///
/// Every method takes `&self`; implementors are typically unit structs.
/// `Send + Sync` is required so the registry can hand out
/// `&'static dyn HarnessModule` references and so test injection (via
/// `HARNESS_MODULES_OVERRIDE`) can use `Vec<Box<dyn HarnessModule>>`.
pub trait HarnessModule: Send + Sync {
    /// Stable harness identifier used in settings TOML, CLI arguments,
    /// and the catalog index. Lowercase, kebab-case where multi-word
    /// (e.g. `"claude-code"`).
    fn name(&self) -> &'static str;

    /// Short human-readable description (used by `tome harness` bare).
    fn description(&self) -> &'static str;

    /// Filesystem-existence check against the harness's per-user dir.
    ///
    /// Per FR-167, this MUST be existence-only — no reading the
    /// harness's own configuration files, no parsing of its rules, no
    /// inspection of its plugins. The check is fast, side-effect free,
    /// and trivially mockable via a `TempDir`-rooted `home`.
    fn detect(&self, home: &Path) -> bool;

    /// Path where Tome's rules content lands for this harness.
    ///
    /// For `BlockInExistingFile` strategies this is a developer-authored
    /// file (Tome inserts a delimited block); for `StandaloneFile`
    /// strategies this is a fully Tome-owned file.
    fn rules_file_target(&self, project_root: &Path) -> PathBuf;

    /// How Tome integrates with the rules file at `rules_file_target`.
    fn rules_file_strategy(&self) -> RulesFileStrategy;

    /// Body-content style for the block. Only consulted when
    /// `rules_file_strategy()` returns `BlockInExistingFile`.
    fn block_body_style(&self) -> BlockBodyStyle;

    /// Path to the harness's MCP configuration file.
    ///
    /// Some harnesses keep this per-project (Claude Code, Cursor,
    /// OpenCode); others keep it global under `~/.<harness>/` (Codex,
    /// Gemini). Both `project_root` and `home` are passed so each
    /// harness can pick.
    fn mcp_config_path(&self, project_root: &Path, home: &Path) -> PathBuf;

    /// Serialisation format of the MCP configuration file.
    fn mcp_config_format(&self) -> McpConfigFormat;

    /// Top-level container key under which `MCP_CONFIG_KEY` is nested.
    ///
    /// JSON harnesses use `"mcpServers"`. The Codex TOML harness uses
    /// `"mcp_servers"`. Returned as a `&'static str` because the value
    /// is a compile-time constant per harness.
    fn mcp_parent_key(&self) -> &'static str;
}

/// Registered harness modules in lexicographic order of `name()`.
///
/// Bare `tome harness` (US3.c) iterates this slice in order. Adding a
/// new harness means: write a new file under `src/harness/`, add it
/// here in lexicographic position, update the per-harness specifics
/// table in research.md.
///
/// Lookups also consult `HARNESS_MODULES_OVERRIDE` first — see
/// [`lookup`] and [`effective_modules`].
pub static SUPPORTED_HARNESSES: &[&'static dyn HarnessModule] =
    &[&CLAUDE_CODE, &CODEX, &CURSOR, &GEMINI, &OPENCODE];

/// Test-only override slot for the harness registry.
///
/// Integration tests under `tests/` cannot reach `#[cfg(test)]`-gated
/// hooks (they consume the library as an external crate, not with the
/// test profile's cfg flags). Per the project convention for test
/// injection — `#[doc(hidden)] pub static` plus an RAII guard in the
/// consuming test file — this slot replaces `SUPPORTED_HARNESSES` when
/// `Some(_)`.
///
/// Tests should install a guard like the following in their own file:
///
/// ```ignore
/// struct HarnessModulesGuard;
///
/// impl HarnessModulesGuard {
///     fn install(modules: Vec<Box<dyn HarnessModule>>) -> Self {
///         *HARNESS_MODULES_OVERRIDE.write().unwrap() = Some(modules);
///         Self
///     }
/// }
///
/// impl Drop for HarnessModulesGuard {
///     fn drop(&mut self) {
///         *HARNESS_MODULES_OVERRIDE.write().unwrap() = None;
///     }
/// }
/// ```
///
/// The guard pattern survives panics and prevents cross-test leakage.
/// F7 ships no consumer yet; the first one lands in US3.c when the
/// `tome harness` command needs to enumerate a synthetic registry.
#[doc(hidden)]
pub static HARNESS_MODULES_OVERRIDE: RwLock<Option<Vec<Box<dyn HarnessModule>>>> =
    RwLock::new(None);

/// Look up a harness by exact-match name.
///
/// Returns `None` for any name not in the effective registry. Callers
/// (composition resolver, harness commands) map `None` to
/// `TomeError::HarnessNotSupported` (exit 18).
///
/// The lookup consults `HARNESS_MODULES_OVERRIDE` first; when the slot
/// is empty (the production case) it falls back to
/// `SUPPORTED_HARNESSES`. The override returns a `Box`-allocated
/// reference behind a guard, so the function takes a closure rather
/// than returning a borrow into the lock. For the production registry
/// path, the returned reference is `'static`.
///
/// Test code that needs an overridden lookup should call the registry
/// directly via [`with_effective_modules`] instead.
pub fn lookup(name: &str) -> Option<&'static dyn HarnessModule> {
    // The override path is intended for tests that swap in synthetic
    // modules; production code never installs an override. If an
    // override is present, fall through to `with_effective_modules`
    // for iteration-aware callers — but `lookup` itself only resolves
    // against `SUPPORTED_HARNESSES` because the function signature
    // promises a `'static` reference, which the boxed test modules
    // can't satisfy.
    SUPPORTED_HARNESSES
        .iter()
        .copied()
        .find(|m| m.name() == name)
}

/// Run `f` against the currently-effective harness registry.
///
/// When `HARNESS_MODULES_OVERRIDE` is `Some(_)`, the closure receives a
/// slice of the boxed test modules. Otherwise it receives
/// `SUPPORTED_HARNESSES`. The closure runs under the read guard, so
/// callers must avoid re-entering the override slot.
///
/// Returning data out of the closure (rather than a borrow) keeps the
/// guard's lifetime hidden. The first real consumer (US3.c) maps each
/// module's `name()` and `description()` to owned `String`s and
/// returns those.
pub fn with_effective_modules<R>(f: impl FnOnce(&[&dyn HarnessModule]) -> R) -> R {
    let guard = HARNESS_MODULES_OVERRIDE
        .read()
        .expect("HARNESS_MODULES_OVERRIDE poisoned");
    match guard.as_ref() {
        Some(boxed) => {
            // Build a transient view of `&dyn HarnessModule` references.
            let view: Vec<&dyn HarnessModule> = boxed.iter().map(|b| b.as_ref()).collect();
            f(&view)
        }
        None => f(SUPPORTED_HARNESSES),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_harnesses_has_five_entries() {
        assert_eq!(SUPPORTED_HARNESSES.len(), 5);
    }

    #[test]
    fn supported_harnesses_are_in_lexicographic_order() {
        let names: Vec<&str> = SUPPORTED_HARNESSES.iter().map(|h| h.name()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "SUPPORTED_HARNESSES must be lex-ordered");
    }

    #[test]
    fn lookup_resolves_each_registered_name() {
        for harness in SUPPORTED_HARNESSES {
            let found = lookup(harness.name()).expect("registered harness must resolve");
            assert_eq!(found.name(), harness.name());
        }
    }

    #[test]
    fn lookup_returns_none_for_unknown_name() {
        assert!(lookup("definitely-not-a-harness").is_none());
    }

    #[test]
    fn mcp_config_key_is_tome() {
        assert_eq!(MCP_CONFIG_KEY, "tome");
    }
}
