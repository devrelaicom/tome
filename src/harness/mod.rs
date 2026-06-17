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

pub mod agents;
pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod gemini;
pub mod guardrails;
pub mod hooks;
pub mod mcp_config;
pub mod opencode;
/// Per-sink reconcilers (hooks / guardrails / agents) extracted from `sync`
/// in Phase 7 (FR-011). Crate-internal: the orchestrator and the doctor are
/// the only callers.
pub(crate) mod reconcile;
pub mod routing;
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

/// How Tome reconciles a plugin's `hooks/hooks.json` for a harness
/// (data-model §2). Only Claude Code returns `RealJson`; every other
/// harness is `GuardrailsOnly`, falling back to the prose `GUARDRAILS.md`
/// region (FR-001, FR-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HooksStrategy {
    /// Merge rewritten hook entries into the harness's machine-local
    /// settings file (`.claude/settings.local.json`).
    RealJson,
    /// No native hook support — render the plugin's `GUARDRAILS.md` prose
    /// into the harness's guardrails target instead.
    GuardrailsOnly,
}

/// Where a harness's guardrails region lands (data-model §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailsPlacement {
    /// A marker-delimited region inside an existing rules file (e.g.
    /// `CLAUDE.md`, `AGENTS.md`). Content outside the markers is preserved.
    InFileRegion { file: PathBuf },
    /// A fully Tome-owned standalone file (Cursor's
    /// `.cursor/rules/TOME_GUARDRAILS.md`). Deleted when no plugin
    /// contributes.
    StandaloneSibling { file: PathBuf },
}

/// The resolved guardrails sink for one harness (data-model §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailsTarget {
    pub placement: GuardrailsPlacement,
    /// `true` only for Claude Code (FR-013): when a plugin ships real JSON
    /// hooks, its `CLAUDE.md` guardrails region is suppressed in favour of
    /// the merged hooks.
    pub suppress_if_hooks_present: bool,
}

/// Serialisation format for a harness's native agent files (data-model
/// §4). `claude-code` / `cursor` / `opencode` use Markdown-with-YAML
/// frontmatter; `codex` uses TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFormat {
    MarkdownYaml,
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

    /// Filesystem path that [`detect`](Self::detect) probes for
    /// existence.
    ///
    /// The default implementation returns `home.join(format!(".{}",
    /// self.name()))`, which matches every harness whose per-user dir
    /// matches its `name()`. Harnesses whose per-user dir name diverges
    /// from `name()` (e.g. `claude-code` -> `~/.claude/`) MUST override
    /// this method so callers reporting "what we probed" don't lie.
    ///
    /// Polish C-M1: `tome harness info` previously computed the probed
    /// path inline as `home.join(format!(".{}", m.name()))`, producing
    /// `~/.claude-code/` for the `claude-code` harness despite its
    /// `detect` actually probing `~/.claude/`. Promoting this to a
    /// trait method (with overrides only where needed) keeps the two
    /// surfaces in lockstep.
    fn detect_path(&self, home: &Path) -> PathBuf {
        home.join(format!(".{}", self.name()))
    }

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

    // -----------------------------------------------------------------------
    // Phase 6 — hooks, guardrails, native agents (harness-modules-p6.md).
    //
    // Every default makes a brand-new harness *safe-by-default*:
    // guardrails-only, no real hooks, no native agents. The five real
    // harness modules inherit these defaults until US1/US2/US3 override
    // them — so adding them here keeps the existing harness tests green.
    // -----------------------------------------------------------------------

    /// How Tome reconciles plugin hooks for this harness (FR-001, FR-013).
    /// Default: no native hook support (prose guardrails fallback only).
    fn hooks_strategy(&self) -> HooksStrategy {
        HooksStrategy::GuardrailsOnly
    }

    /// Machine-local settings file the `RealJson` strategy merges hooks
    /// into (FR-002). `None` for every `GuardrailsOnly` harness. Only
    /// Claude Code overrides this (`.claude/settings.local.json`).
    fn hook_settings_path(&self, _project_root: &Path) -> Option<PathBuf> {
        None
    }

    /// Path to the JSON hooks file into which Tome writes its OWN
    /// `SessionStart` routing hook (the directive deliverer), for harnesses
    /// that support a session-start hook but are NOT routed through the
    /// Claude-Code `RealJson` plugin-hooks pass. `None` (default) means the
    /// harness gets no Tome-owned session hook. This is deliberately separate
    /// from [`Self::hook_settings_path`]: a harness here receives ONLY Tome's
    /// own hook, never plugin hooks (plugin→harness hook mapping is not yet
    /// implemented). Claude Code returns `None` here — its Tome hook rides the
    /// `RealJson` pass alongside plugin hooks.
    fn tome_session_hook_path(&self, _project_root: &Path) -> Option<PathBuf> {
        None
    }

    /// The harness's guardrails sink (FR-011, FR-012). Default: an in-file
    /// region on the harness's own rules-file target, with no
    /// hooks-driven suppression.
    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        GuardrailsTarget {
            placement: GuardrailsPlacement::InFileRegion {
                file: self.rules_file_target(project_root),
            },
            suppress_if_hooks_present: false,
        }
    }

    /// Whether this harness emits native translated agents (FR-030).
    /// Default `false`; Gemini and the Phase 2 harnesses stay `false`.
    fn supports_native_agents(&self) -> bool {
        false
    }

    /// Directory native agent files land in (FR-031). `None` unless
    /// `supports_native_agents()`.
    fn agent_dir(&self, _project_root: &Path) -> Option<PathBuf> {
        None
    }

    /// Native agent serialisation format (FR-030, FR-033). `None` unless
    /// `supports_native_agents()`.
    fn agent_format(&self) -> Option<AgentFormat> {
        None
    }

    /// Translate a canonical agent into this harness's native form (FR-030,
    /// FR-032). Only ever called when `supports_native_agents()` is `true`;
    /// non-supporting harnesses need no override.
    ///
    /// `clashes` is the workspace clash flag for this agent's `<name>`
    /// (FR-041): when `true`, two or more enabled plugins hold the same
    /// name, so the displayed/registered name is plugin-prefixed
    /// (`<plugin>-<name>`). The on-disk filename stays `<plugin>__<name>`
    /// regardless. Chunk C computes the flag once per sync from the shared
    /// clash-set SSOT ([`crate::index::skills::agent_name_clash_set`]) and
    /// passes it per agent.
    ///
    /// The plugin/catalog provenance the filename needs lives on
    /// [`agents::CanonicalAgent`] itself (`plugin`, `catalog`), so the
    /// signature needs only the clash flag beyond the canonical.
    ///
    /// Returns `Result` because translation can fail (e.g. a body that
    /// cannot be rendered into the target format) → `AgentTranslationFailed`
    /// (exit 45). The default panics loudly to surface a dispatch bug rather
    /// than emit a bogus file — it is only reached if a `GuardrailsOnly`
    /// harness is wrongly asked to translate.
    fn translate_agent(
        &self,
        _canonical: &agents::CanonicalAgent,
        _clashes: bool,
    ) -> Result<agents::TranslatedAgent, crate::error::TomeError> {
        unreachable!(
            "translate_agent called on a harness without native agent support: {}",
            self.name()
        )
    }

    // -----------------------------------------------------------------------
    // Phase 9 — native skill emit (harness-skill-emit.md, R-3).
    //
    // Symmetric to the Phase-6 agent methods above. Defaults make every
    // harness — and any future one, and Gemini — safe-by-default: no native
    // skills, no install target. The four supported harnesses override the
    // dir methods with their OWN canonical `skills/` root (never a compat
    // sibling). The format is always a `SKILL.md` folder, so unlike agents
    // there is no format enum.
    // -----------------------------------------------------------------------

    /// Whether this harness consumes native `SKILL.md` skill folders. Default
    /// `false`; Gemini and any future harness stay `false`.
    fn supports_native_skills(&self) -> bool {
        false
    }

    /// Project-scope skills root (`<project_root>/…/skills/`). `None` unless
    /// `supports_native_skills()`.
    fn skill_dir(&self, _project_root: &Path) -> Option<PathBuf> {
        None
    }

    /// Global/user-scope skills root (under `home`). `None` unless
    /// `supports_native_skills()`.
    fn skill_dir_global(&self, _home: &Path) -> Option<PathBuf> {
        None
    }
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
/// Resolves against `SUPPORTED_HARNESSES` only — does NOT consult
/// `HARNESS_MODULES_OVERRIDE` (the function signature promises a
/// `'static` reference, which the boxed test modules can't satisfy).
/// Production code paths should call [`with_effective_modules`] for
/// override-aware iteration / dispatch.
///
/// Polish R-M8: kept `pub` for integration-test reachability under
/// `#[doc(hidden)]` — production code should call
/// [`with_effective_modules`] which honours `HARNESS_MODULES_OVERRIDE`.
/// The `pub`-not-`pub(crate)` accommodates the same constraint
/// documented in F7 (integration tests under `tests/` consume the lib
/// without `cfg(test)` visibility).
#[doc(hidden)]
pub fn lookup(name: &str) -> Option<&'static dyn HarnessModule> {
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
    // Polish R-M3: recover from poison rather than panic. A panicking
    // writer-side test leaves the lock poisoned; the read side's only
    // invariant is "I can see the current Vec" which a poisoned-but-
    // still-readable RwLock still satisfies. Mirrors the discipline at
    // `src/summarise/mod.rs::backend()`.
    let guard = HARNESS_MODULES_OVERRIDE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    #[test]
    fn detect_path_defaults_to_dot_name_under_home() {
        // Default impl: `home.join(format!(".{}", self.name()))`.
        // Codex / Cursor / Gemini / OpenCode all use the default.
        let home = std::path::Path::new("/h");
        for harness in SUPPORTED_HARNESSES {
            if harness.name() == "claude-code" {
                continue;
            }
            assert_eq!(
                harness.detect_path(home),
                home.join(format!(".{}", harness.name())),
                "{} should use the default detect_path",
                harness.name(),
            );
        }
    }

    #[test]
    fn detect_path_for_claude_code_matches_detect_probe() {
        // Polish C-M1: `claude-code`'s `detect` probes `~/.claude/`,
        // not `~/.claude-code/`. The overridden `detect_path` MUST
        // return the same path so `tome harness info`'s `detected_path`
        // doesn't lie to the caller.
        let home = std::path::Path::new("/h");
        let m = lookup("claude-code").unwrap();
        assert_eq!(m.detect_path(home), home.join(".claude"));
    }
}
