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
//!
//! ## Phase 11 — error reuse (FR-034), NO new exit code
//!
//! Phase 11 widens the harness set (~5 → ~16) and adds three trait
//! generalizations — G1 `McpDialect`, G2 `SessionSteering`, G3
//! rules-delivery extensions — that all arrive as **defaulted** trait
//! methods so the existing modules compile unchanged. Per the constitution
//! gate this phase adds **NO new dependency**, **NO new top-level module**
//! (the new code lives in `harness/` submodules), and **NO new exit code**.
//!
//! Every new failure class in Phase 11 maps onto an existing closed-set
//! `TomeError` variant — we deliberately do NOT reach for a new code, because
//! each failure is semantically the same as one Phase ≤10 already names, and
//! the closed error set's value is exactly that it has no `Other` arm to
//! absorb novelty. The verbatim mapping a contributor must reuse:
//!
//! - [`crate::error::TomeError::HarnessClash`] (exit 19) — a non-Tome-owned
//!   MCP entry already occupies the `tome` key (the MCP-ownership predicate
//!   refuses to clobber a foreign server named `tome`).
//! - [`crate::error::TomeError::HarnessNotSupported`] (exit 18) — an unknown
//!   harness name (after alias resolution); the `lookup`/registry miss.
//! - [`crate::error::TomeError::Io`] (exit 7) — symlink refusal or generic IO
//!   at a sink (rules file, MCP config, shim, session-hook file).
//! - [`crate::error::TomeError::HookSettingsWriteFailed`] (exit 44) — a
//!   hook/session-hook file write failure (the G2 `CommandHook` sink).
//! - [`crate::error::TomeError::HookSpecParseError`] (exit 43) — a malformed
//!   existing hook file we must merge into.
//!
//! When you add a Phase-11 failure path, pick the variant above whose meaning
//! matches — do not promote a new code for a failure the set already names.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub mod agents;
pub mod antigravity;
pub mod claude_code;
pub mod cline;
pub mod codex;
pub mod copilot;
pub mod copilot_cli;
pub mod crush;
pub mod cursor;
pub mod devin;
pub mod gemini;
pub mod guardrails;
pub mod hooks;
pub mod jetbrains_ai;
pub mod junie;
pub mod kiro;
pub mod mcp_config;
pub mod opencode;
pub mod pi;
/// Embedded harness-plugin (TypeScript shim) registry (Phase 11, R6). Defines
/// the `include!` target shapes and exposes the `build.rs`-generated
/// `HARNESS_PLUGINS` slice. Harness-runtime-executed — the sync boundary holds.
pub mod plugin_assets;
/// Per-sink reconcilers (hooks / guardrails / agents) extracted from `sync`
/// in Phase 7 (FR-011). Crate-internal: the orchestrator and the doctor are
/// the only callers.
pub(crate) mod reconcile;
pub mod routing;
pub mod rules_file;
pub mod stub;
pub mod sync;
pub mod zed;

use antigravity::ANTIGRAVITY;
use claude_code::CLAUDE_CODE;
use cline::CLINE;
use codex::CODEX;
use copilot::COPILOT;
use copilot_cli::COPILOT_CLI;
use crush::CRUSH;
use cursor::CURSOR;
use devin::DEVIN;
use gemini::GEMINI;
use jetbrains_ai::JETBRAINS_AI;
use junie::JUNIE;
use kiro::KIRO;
use opencode::OPENCODE;
use pi::PI;
use zed::ZED;

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

// =====================================================================
// Phase 11 — G1: the MCP dialect (FR-008, R2/R3, contract mcp-dialects.md).
//
// `McpDialect` generalizes the two scalars the Phase ≤10 trait exposed
// (`mcp_config_format()` + `mcp_parent_key()`) into one value that also
// captures the per-harness *body shape*: the entry-body template, the
// optional `type` discriminator, the empty-`env` policy, and any
// always-present mandated fields (`tools`, `enabled`, …). The single
// dialect-aware read/write/remove in `mcp_config.rs` is driven by this
// value; `TomeEntry { command, args, env }` stays the uniform in-memory
// model regardless of dialect.
//
// All new state is closed (enums / `&'static` slices) — there is no
// free-form escape hatch — so every harness's wire shape is a
// compile-time constant the byte-stable pins can pin exactly.
// =====================================================================

/// On-disk serialisation family for a harness MCP config file.
///
/// `Json` and `Jsonc` both route through the `serde_json` read/write
/// path (Tome always *emits* plain JSON); `Jsonc` is a distinct variant
/// only to document that the harness *tolerates* comments in the file it
/// reads (OpenCode's `opencode.json`), so a future reader knows the
/// lenient parse is intentional. `Toml` routes through `toml_edit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    /// `serde_json` with the `preserve_order` feature.
    Json,
    /// JSON-with-comments — same `serde_json` read/write path as `Json`;
    /// the variant documents comment-tolerance of the harness's reader.
    Jsonc,
    /// `toml_edit` (comment- and order-preserving).
    Toml,
}

/// Closed set of entry-body templates (FR-008).
///
/// Determines how `command`/`args` serialise:
///
/// - `CommandArgs` — `command` is a string, `args` is a string array
///   (the Phase ≤10 shape every existing harness uses).
/// - `CommandArray` — `command` is a single array `[launcher, ...args]`
///   with NO separate `args` key (OpenCode's `opencode.json`). On read
///   the array is normalised back to `command = arr[0]`, `args =
///   arr[1..]` so `TomeEntry` and `is_tome_owned` are shape-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryShape {
    /// `"command": "<launcher>", "args": ["mcp", …]`.
    CommandArgs,
    /// `"command": ["<launcher>", "mcp", …]` — no `args` key.
    CommandArray,
}

/// The server-`type` discriminator some harnesses require on each entry.
///
/// Serialises to `"type": "local"` / `"type": "stdio"`. Re-derived from
/// the dialect on every rewrite (a stale/edited value self-heals).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerType {
    /// `"type": "local"` (OpenCode, copilot-cli).
    Local,
    /// `"type": "stdio"` (copilot VS Code, crush).
    Stdio,
}

impl ServerType {
    /// The literal `type` string this discriminator serialises to.
    pub fn as_str(self) -> &'static str {
        match self {
            ServerType::Local => "local",
            ServerType::Stdio => "stdio",
        }
    }
}

/// Closed value domain for a mandated [`ExtraField`].
///
/// Covers exactly the two always-present field values across the Phase
/// 11 harness set: OpenCode's `"enabled": true` and copilot-cli's
/// `"tools": ["*"]`. No free-form escape hatch — extending the set is a
/// deliberate enum edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtraValue {
    /// A JSON/TOML boolean (e.g. OpenCode `enabled: true`).
    Bool(bool),
    /// A JSON/TOML array of strings (e.g. copilot-cli `tools: ["*"]`).
    StringArray(&'static [&'static str]),
}

/// One always-present mandated field appended to an emitted entry.
///
/// Re-derived from the dialect on every rewrite so a developer's edit of
/// a mandated value self-heals back to the dialect's canonical form
/// (R3/m6). Appended LAST, in slice order, after `env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtraField {
    pub key: &'static str,
    pub value: ExtraValue,
}

/// The full per-harness MCP wire-shape (the G1 generalization).
///
/// Replaces the Phase ≤10 `mcp_config_format()` + `mcp_parent_key()`
/// scalar pair with one value the shared `mcp_config` read/write/remove
/// is driven by. The legacy behaviour is the default dialect (see
/// [`HarnessModule::mcp_dialect`]); only OpenCode (the G1 canary)
/// diverges among the five existing modules.
///
/// ## Emitted field order (load-bearing for byte-stable pins)
///
/// `type` (iff `entry_type` is `Some`) → `command` →
/// `args` (`CommandArgs` only) → `env` (iff a developer env is present,
/// OR `emit_env` and no env) → each `extra_fields` entry, in slice
/// order.
///
/// ## Ownership predicate (R2/B1)
///
/// After the per-shape extraction of `(command, args)`, the SINGLE
/// existing [`mcp_config::is_tome_owned`] predicate applies (`command ==
/// "tome" && args[0] == "mcp"`). For `CommandArray` this is `arr[0] ==
/// "tome" && arr[1] == "mcp"` by construction of the normalisation.
///
/// [`mcp_config::is_tome_owned`]: crate::harness::mcp_config::is_tome_owned
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpDialect {
    /// On-disk serialisation family.
    pub file_format: FileFormat,
    /// Top-level container key under which `MCP_CONFIG_KEY` is nested
    /// (`"mcpServers"`, `"mcp_servers"`, `"mcp"`, `"servers"`,
    /// `"context_servers"`, …).
    pub parent_key: &'static str,
    /// Entry-body template (`command`/`args` vs single `command` array).
    pub entry_shape: EntryShape,
    /// The `type` discriminator, if the harness mandates one.
    pub entry_type: Option<ServerType>,
    /// Whether to emit `"env": {}` when no developer env exists. `false`
    /// preserves the Phase ≤10 writer's "omit empty env" behaviour.
    pub emit_env: bool,
    /// Always-present mandated fields, appended last in slice order.
    pub extra_fields: &'static [ExtraField],
}

impl McpDialect {
    /// The legacy dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
    /// no empty-`env` emission, no extra fields. Reproduces the Phase
    /// ≤10 byte output exactly. This is the value [`HarnessModule`]'s
    /// default `mcp_dialect()` returns.
    pub const LEGACY: McpDialect = McpDialect {
        file_format: FileFormat::Json,
        parent_key: "mcpServers",
        entry_shape: EntryShape::CommandArgs,
        entry_type: None,
        emit_env: false,
        extra_fields: &[],
    };

    /// Build a legacy-shaped dialect (`CommandArgs`, no `type`, no empty
    /// `env`, no extras) for a given format + parent key.
    ///
    /// Bridges the Phase ≤10 `(McpConfigFormat, parent_key)` pair to a
    /// `McpDialect` — used by call sites (and tests) that still think in
    /// the old scalar terms. The `Json`/`Toml` mapping is exact;
    /// `Jsonc → serde_json` is not reachable here (no legacy harness was
    /// Jsonc).
    pub const fn from_format(format: McpConfigFormat, parent_key: &'static str) -> McpDialect {
        let file_format = match format {
            McpConfigFormat::Json => FileFormat::Json,
            McpConfigFormat::Toml => FileFormat::Toml,
        };
        McpDialect {
            file_format,
            parent_key,
            entry_shape: EntryShape::CommandArgs,
            entry_type: None,
            emit_env: false,
            extra_fields: &[],
        }
    }

    /// The coarse [`McpConfigFormat`] this dialect maps to (`Json`/`Jsonc`
    /// → `Json`; `Toml` → `Toml`). Kept so the Phase ≤10
    /// `mcp_config_format()` accessor can be derived from the dialect.
    pub fn config_format(&self) -> McpConfigFormat {
        match self.file_format {
            FileFormat::Json | FileFormat::Jsonc => McpConfigFormat::Json,
            FileFormat::Toml => McpConfigFormat::Toml,
        }
    }
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

// =====================================================================
// Phase 11 — G2: session steering (contract session-steering.md).
//
// `SessionSteering` generalizes the Phase ≤10 "Tome owns its own
// session-start hook" wiring (the claude `RealJson` SessionStart entry +
// the codex `tome_session_hook_path`) into one descriptor covering THREE
// transports: a command hook (the new-harness JSON `CommandHook` sink),
// an embedded TypeScript plugin shim (`TsPlugin`), or no native steering
// (`None`, the rules-file-only floor every existing module keeps).
//
// All state is closed (enums / `PathBuf`) — there is no free-form escape
// hatch, so each harness's session-start wire shape is a compile-time
// constant the byte-stable pins can pin exactly.
//
// IMPORTANT: this is SEPARATE from [`HooksStrategy`], which governs
// PLUGIN-hook merging (the `RealJson` Claude Code path). Claude Code and
// Codex keep their dedicated Phase ≤10 session-hook path and continue to
// return [`SessionSteering::None`] here, so the new `CommandHook`
// reconciler naturally excludes them and their byte output is unchanged.
// =====================================================================

/// Closed set of stdout envelopes a harness wraps the directive in
/// (selected by `tome harness session-start --harness <name>`).
///
/// The directive bytes are identical across envelopes — only the JSON
/// wrapper differs. See `wrap_in_envelope` in the `session_start` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Envelope {
    /// `{ "hookSpecificOutput": { "hookEventName": "SessionStart",
    /// "additionalContext": "<directive>" } }` — claude-code, devin, gemini.
    ClaudeNested,
    /// `{ "additionalContext": "<directive>" }` — copilot-cli.
    FlatAdditionalContext,
    /// `{ "injectSteps": [ { "ephemeralMessage": "<directive>" } ] }` —
    /// antigravity.
    AntigravityInjectSteps,
}

/// The hook event a `CommandHook` session-start entry registers under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// `SessionStart` (devin, copilot-cli, gemini).
    SessionStart,
    /// `PreInvocation` (antigravity's `.agents/hooks.json` list form).
    PreInvocation,
}

/// Which embedded TypeScript shim a `TsPlugin` harness ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShimKind {
    Cline,
    Pi,
    OpenCode,
}

/// Closed set of new-harness hook-file shapes the `CommandHook` reconciler
/// writes (contract session-steering.md §CommandHook file specs).
///
/// `ClaudeSettingsLocal` / `CodexHooks` name the two Phase ≤10 sinks for
/// completeness, but they are UNREACHABLE through the new `CommandHook`
/// reconciler — claude-code/codex keep [`SessionSteering::None`] and their
/// dedicated path, so the new reconciler never sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFileSpec {
    /// Devin `.devin/hooks.v1.json` (no wrapper key).
    DevinHooksV1,
    /// Copilot CLI `.github/hooks/<file>.json` (`version` + `hooks` wrapper).
    CopilotHooks,
    /// Gemini `.gemini/settings.json` (`hooks` section, nested `hooks` array).
    GeminiSettings,
    /// Antigravity `.agents/hooks.json` (named `tome` block, `PreInvocation`).
    AntigravityHooks,
    /// Claude Code `.claude/settings.local.json` — Phase ≤10 sink, NOT
    /// reachable via the new `CommandHook` reconciler.
    ClaudeSettingsLocal,
    /// Codex `.codex/hooks.json` — Phase ≤10 sink, NOT reachable via the new
    /// `CommandHook` reconciler.
    CodexHooks,
}

// =====================================================================
// Phase 11 — G3: rules-delivery extensions (contract rules-delivery.md).
//
// Two additive, default-backed surfaces let a new harness diverge from the
// `rules_file_target` + `BlockInExistingFile` floor without touching the
// five existing modules:
//
//   * `RulesFrontmatter` — a Tome-owned YAML front-matter header written
//     ABOVE the verbatim directive on a `StandaloneFile` (kiro's
//     `inclusion: always`, jetbrains-ai's apply-mode marker). Its bytes are
//     pinned separately from the directive body (m3).
//
//   * `rules_namespaced_file` — a dedicated, namespaced standalone file used
//     INSTEAD of `rules_file_target` (cline's `.clinerules/tome.md`, zed's
//     `.rules`, kiro/jetbrains steering files), so Tome never clobbers a
//     developer-authored shared rules file.
//
// Both arrive as DEFAULTED trait methods (`None`), so every Phase ≤10 module
// inherits the unchanged behaviour. The `RulesFrontmatter.fields` slice is a
// closed `&'static` constant per harness → the front-matter wire shape is a
// compile-time value the byte-stable pins can pin exactly.
// =====================================================================

/// A Tome-owned YAML front-matter header for a `StandaloneFile` rules sink
/// (G3, FR-026).
///
/// `fields` is rendered, in slice order, between two `---` fences ABOVE the
/// verbatim directive body. The key order is the slice order (deterministic),
/// so the emitted header is byte-stable. Values are emitted verbatim (Tome
/// owns every value — they are not third-party content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RulesFrontmatter {
    /// `(key, value)` pairs rendered as `key: value` lines, in slice order.
    /// e.g. `&[("inclusion", "always")]` for kiro; `&[("apply", "always")]`
    /// for jetbrains-ai's Always apply-mode.
    pub fields: &'static [(&'static str, &'static str)],
}

/// How a harness receives Tome's session-start steering directive (G2).
///
/// The default ([`SessionSteering::None`]) keeps the rules-file-only floor
/// every existing module relies on; the new harnesses override
/// [`HarnessModule::session_steering`] with `CommandHook` or `TsPlugin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSteering {
    /// No native session steering — the directive rides the rules file only.
    /// Claude Code and Codex stay here (their Tome session hook uses the
    /// dedicated Phase ≤10 path, NOT this descriptor).
    None,
    /// A Tome-owned hook entry written into the harness's JSON hook file,
    /// invoking `tome harness session-start --workspace <ws> --harness <name>`
    /// and wrapping the stdout in `envelope`.
    CommandHook {
        file_spec: HookFileSpec,
        event: HookEvent,
        envelope: Envelope,
    },
    /// An embedded TypeScript shim written into the harness's plugin dir; the
    /// shim runs `tome harness session-start … --harness <name>` and injects
    /// its (raw) stdout via the harness's own injection API.
    TsPlugin { dir: PathBuf, kind: ShimKind },
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
    ///
    /// Default [`BlockBodyStyle::Inline`] (G3) — a new harness gets the
    /// verbatim-rules-inline body unless it documents `@`-include support and
    /// overrides this. Among the five existing modules every one explicitly
    /// declares its style (`claude-code`/`codex`/`gemini` → `AtInclude`,
    /// `opencode` → `Inline`, `cursor` → `Inline` though it is a `StandaloneFile`
    /// and so never consulted), so adding the default changes none of them.
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }

    /// A dedicated, namespaced standalone rules file used INSTEAD of
    /// [`Self::rules_file_target`] when `Some` (G3, FR-024).
    ///
    /// Returned by harnesses that own a Tome-specific file under their own
    /// directory (`cline` → `.clinerules/tome.md`, `zed` → `.rules`,
    /// `kiro` → `.kiro/steering/tome.md`, `jetbrains-ai` →
    /// `.aiassistant/rules/tome.md`) so Tome never inserts a block into a
    /// developer-authored shared rules file. `None` (default) means the
    /// harness's rules content lands at `rules_file_target` per its
    /// `rules_file_strategy`. The five existing modules return `None`.
    fn rules_namespaced_file(&self, _project_root: &Path) -> Option<PathBuf> {
        None
    }

    /// The Tome-owned YAML front-matter header for this harness's
    /// `StandaloneFile` rules sink (G3, FR-026), or `None` (default) for no
    /// front-matter.
    ///
    /// Only meaningful for a `StandaloneFile` (or a `rules_namespaced_file`)
    /// sink: the header is written ABOVE the verbatim directive. `kiro`
    /// returns `inclusion: always`; `jetbrains-ai` returns its Always
    /// apply-mode marker. The five existing modules return `None`.
    fn rules_frontmatter(&self) -> Option<RulesFrontmatter> {
        None
    }

    /// Path to the harness's MCP configuration file.
    ///
    /// Some harnesses keep this per-project (Claude Code, Cursor,
    /// OpenCode); others keep it global under `~/.<harness>/` (Codex,
    /// Gemini). Both `project_root` and `home` are passed so each
    /// harness can pick.
    fn mcp_config_path(&self, project_root: &Path, home: &Path) -> PathBuf;

    /// The harness's full MCP wire-shape (Phase 11, G1).
    ///
    /// The single value driving the shared `mcp_config` read/write/remove.
    /// The default reproduces the Phase ≤10 behaviour exactly
    /// ([`McpDialect::LEGACY`]: JSON `mcpServers` + `CommandArgs`, no
    /// `type`, omit-empty-`env`, no extra fields). A harness whose wire
    /// shape diverges overrides this ONE method — among the five existing
    /// modules only OpenCode (the G1 canary) does so.
    ///
    /// IMPORTANT: `emit_env` stays `false` in the default — the Phase ≤10
    /// writer omits an empty `env`, so flipping it would move every JSON
    /// harness's byte-stable pin.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect::LEGACY
    }

    /// Serialisation format of the MCP configuration file.
    ///
    /// Derived from [`Self::mcp_dialect`] by default
    /// (`Json`/`Jsonc → Json`, `Toml → Toml`). A harness should override
    /// `mcp_dialect()`, not this — this accessor exists only for the
    /// surfaces that still think in the coarse format scalar.
    fn mcp_config_format(&self) -> McpConfigFormat {
        self.mcp_dialect().config_format()
    }

    /// Top-level container key under which `MCP_CONFIG_KEY` is nested.
    ///
    /// JSON harnesses use `"mcpServers"`. The Codex TOML harness uses
    /// `"mcp_servers"`. Returned as a `&'static str` because the value
    /// is a compile-time constant per harness. Derived from
    /// [`Self::mcp_dialect`] by default — override `mcp_dialect()`, not
    /// this.
    fn mcp_parent_key(&self) -> &'static str {
        self.mcp_dialect().parent_key
    }

    /// Whether this harness has NO writable MCP configuration file (Phase
    /// 11, contract mcp-dialects.md § "Manual-only (no file write)").
    ///
    /// When `true`, the sync orchestrator MUST NOT read, write, or remove
    /// the harness's MCP config — the harness configures MCP through a
    /// UI-only surface (jetbrains-ai) with no file Tome can own. The
    /// harness still receives its rules-file integration; only the MCP
    /// sink is skipped. The user-facing "paste this snippet" notice is a
    /// SEPARATE US5 concern — this predicate only governs the file write.
    ///
    /// Default `false` — every Phase ≤10 module (and every harness with a
    /// real writable MCP file) keeps writing its entry, so the existing
    /// byte output is unchanged. `pi` keeps the default `false` in US1: it
    /// DOES write `~/.pi/agent/mcp.json` (the adapter-install notice is a
    /// US5 fast-follow), so only `jetbrains-ai` overrides this to `true`.
    fn mcp_manual_only(&self) -> bool {
        false
    }

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
    // Phase 11 — G2: session steering (contract session-steering.md).
    //
    // The default makes a brand-new harness — and every existing module —
    // rules-file-only: no native session-start transport. New harnesses
    // override this with `CommandHook` (the JSON hook reconciler) or
    // `TsPlugin` (an embedded shim). Claude Code and Codex DELIBERATELY keep
    // the default `None`: their Tome session hook uses the dedicated Phase ≤10
    // path (the `RealJson` SessionStart entry / `tome_session_hook_path`), so
    // the new `CommandHook` reconciler never touches them and their byte
    // output is unchanged.
    // -----------------------------------------------------------------------

    /// How this harness receives Tome's session-start steering directive
    /// (FR-014–FR-021, G2). Default [`SessionSteering::None`] — rules-file
    /// only. Overridden by the new Phase 11 harnesses; the five existing
    /// modules keep `None`.
    fn session_steering(&self) -> SessionSteering {
        SessionSteering::None
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
pub static SUPPORTED_HARNESSES: &[&'static dyn HarnessModule] = &[
    &ANTIGRAVITY,
    &CLAUDE_CODE,
    &CLINE,
    &CODEX,
    &COPILOT,
    &COPILOT_CLI,
    &CRUSH,
    &CURSOR,
    &DEVIN,
    &GEMINI,
    &JETBRAINS_AI,
    &JUNIE,
    &KIRO,
    &OPENCODE,
    &PI,
    &ZED,
];

// =====================================================================
// Phase 11 — registry alias layer + real-vs-opt-in partition
// (data-model §HarnessAlias, §Registry partition; FR-039).
// =====================================================================

/// A registry alias: an alternate CLI name that resolves to a canonical
/// harness module's `name()` before any lookup / dedupe.
///
/// e.g. `antigravity-cli` is a thin alias for `gemini` — it shares the
/// Gemini module entirely. Aliases are resolved by [`resolve_alias`] /
/// [`lookup`] first, so a user can say either name and reach the same
/// module; multi-harness selection (US6) dedupes on the resolved canonical
/// identity so naming a harness twice (once by alias) collapses to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessAlias {
    /// The alternate name the user may type.
    pub name: &'static str,
    /// The canonical [`HarnessModule::name`] it resolves to.
    pub target: &'static str,
}

/// Registered name aliases. Resolved before any registry lookup.
///
/// `antigravity-cli → gemini`: the Antigravity CLI consumes Gemini's
/// configuration surface, so it shares the `gemini` module rather than
/// carrying a duplicate. Empty otherwise; new aliases are a deliberate edit.
pub static HARNESS_ALIASES: &[HarnessAlias] = &[HarnessAlias {
    name: "antigravity-cli",
    target: "gemini",
}];

/// Opt-in targets: modules that are LOOKUP-ABLE by name but NEVER
/// auto-detected and NEVER included in `--all` (data-model §Registry
/// partition).
///
/// These are the `generic` (AGENTS.md + `./mcp.json`) and `generic-op`
/// (Open Plugins `tome-op`) write targets a user opts into explicitly via
/// `tome harness use <name>` — they have no detectable per-user dir, so
/// detection would never surface them and `--all` must not write them. The
/// slice is empty until US4 populates it; it is declared now so the
/// partition-aware `lookup` + the disjointness invariant exist from the
/// foundation.
///
/// INVARIANT: `OPT_IN_TARGETS` is disjoint from [`SUPPORTED_HARNESSES`] (a
/// module is either auto-detectable or opt-in, never both). Asserted in the
/// unit tests.
pub static OPT_IN_TARGETS: &[&'static dyn HarnessModule] = &[];

/// Resolve a possibly-aliased harness name to its canonical name.
///
/// Returns the alias `target` when `name` matches a [`HARNESS_ALIASES`]
/// entry, else `name` unchanged. This is the resolution primitive
/// multi-harness selection (US6) dedupes on — call it before comparing /
/// deduplicating user-supplied harness names. Pure; no registry access.
pub fn resolve_alias(name: &str) -> &str {
    HARNESS_ALIASES
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.target)
        .unwrap_or(name)
}

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
/// Resolves against `SUPPORTED_HARNESSES` + `OPT_IN_TARGETS` only — does NOT
/// consult `HARNESS_MODULES_OVERRIDE` (the function signature promises a
/// `'static` reference, which the boxed test modules can't satisfy).
/// Production code paths should call [`with_effective_modules`] for
/// override-aware iteration / dispatch.
///
/// Phase 11: the supplied name is first run through [`resolve_alias`] (so
/// `antigravity-cli` resolves to the `gemini` module), then matched against
/// `SUPPORTED_HARNESSES` and finally `OPT_IN_TARGETS` (the opt-in `generic` /
/// `generic-op` write targets, which are lookup-able but never auto-detected
/// or in `--all`).
///
/// Polish R-M8: kept `pub` for integration-test reachability under
/// `#[doc(hidden)]` — production code should call
/// [`with_effective_modules`] which honours `HARNESS_MODULES_OVERRIDE`.
/// The `pub`-not-`pub(crate)` accommodates the same constraint
/// documented in F7 (integration tests under `tests/` consume the lib
/// without `cfg(test)` visibility).
#[doc(hidden)]
pub fn lookup(name: &str) -> Option<&'static dyn HarnessModule> {
    let canonical = resolve_alias(name);
    SUPPORTED_HARNESSES
        .iter()
        .copied()
        .find(|m| m.name() == canonical)
        .or_else(|| {
            OPT_IN_TARGETS
                .iter()
                .copied()
                .find(|m| m.name() == canonical)
        })
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
    fn supported_harnesses_has_sixteen_entries() {
        // Phase 11 widened the registry from 5 to 16 (the 5 Phase ≤10 modules
        // plus the 11 US1 harnesses).
        assert_eq!(SUPPORTED_HARNESSES.len(), 16);
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
    fn resolve_alias_maps_antigravity_cli_to_gemini() {
        assert_eq!(resolve_alias("antigravity-cli"), "gemini");
    }

    #[test]
    fn resolve_alias_passes_through_unknown_names_unchanged() {
        assert_eq!(resolve_alias("gemini"), "gemini");
        assert_eq!(resolve_alias("not-an-alias"), "not-an-alias");
    }

    #[test]
    fn lookup_resolves_alias_to_canonical_module() {
        let via_alias = lookup("antigravity-cli").expect("alias must resolve");
        assert_eq!(
            via_alias.name(),
            "gemini",
            "antigravity-cli alias must resolve to the gemini module",
        );
        // Same module identity as a direct lookup of the canonical name.
        let direct = lookup("gemini").expect("gemini must resolve");
        assert_eq!(via_alias.name(), direct.name());
    }

    #[test]
    fn every_alias_target_resolves_to_a_real_module() {
        for alias in HARNESS_ALIASES {
            assert!(
                SUPPORTED_HARNESSES.iter().any(|m| m.name() == alias.target),
                "alias {} → {} must target a registered harness",
                alias.name,
                alias.target,
            );
            // The alias name itself must NOT collide with a real module name.
            assert!(
                !SUPPORTED_HARNESSES.iter().any(|m| m.name() == alias.name),
                "alias name {} must not shadow a real module",
                alias.name,
            );
        }
    }

    #[test]
    fn opt_in_targets_are_disjoint_from_supported() {
        for opt in OPT_IN_TARGETS {
            assert!(
                !SUPPORTED_HARNESSES.iter().any(|m| m.name() == opt.name()),
                "{} must not be both an opt-in target and auto-detected",
                opt.name(),
            );
        }
    }

    #[test]
    fn lookup_finds_opt_in_targets() {
        // Every opt-in target is lookup-able by name even though it is never
        // auto-detected or in `--all`. (Empty until US4 — this still guards the
        // partition-aware lookup branch the moment a target lands.)
        for opt in OPT_IN_TARGETS {
            let found = lookup(opt.name()).expect("opt-in target must be lookup-able");
            assert_eq!(found.name(), opt.name());
        }
    }

    #[test]
    fn mcp_config_key_is_tome() {
        assert_eq!(MCP_CONFIG_KEY, "tome");
    }

    #[test]
    fn detect_path_defaults_to_dot_name_under_home() {
        // Default impl: `home.join(format!(".{}", self.name()))`.
        // Harnesses whose per-user dir diverges from `name()` override
        // `detect_path` (claude-code → ~/.claude, the XDG/per-user
        // overriders below) and are excluded here.
        const OVERRIDERS: &[&str] = &[
            "claude-code",  // ~/.claude
            "crush",        // ~/.config/crush
            "zed",          // ~/.config/zed
            "jetbrains-ai", // ~/.aiassistant
            "copilot-cli",  // ~/.copilot
            "copilot",      // ~/.vscode
            "antigravity",  // ~/.gemini
        ];
        let home = std::path::Path::new("/h");
        for harness in SUPPORTED_HARNESSES {
            if OVERRIDERS.contains(&harness.name()) {
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
