//! Source → IR importers (the per-harness parsers behind `convert`).
//!
//! Every importer treats the source tree as **untrusted**: reads/copies stay
//! within the resolved source root, symlinked components and `..`/absolute
//! escapes are refused, copied filenames pass safe-segment validation, reads
//! are bounded by the per-class cap, and bodies must be valid UTF-8
//! (fail-closed). Third-party inputs (Codex `config.toml`, source frontmatter)
//! are parsed **leniently** — unknown fields warn, never abort (the strictness
//! boundary, principle IV).
//!
//! Tier 1 (port closely): Claude Code marketplaces/plugins + native `SKILL.md`
//! from Cursor/OpenCode/Cline/generic Agent Skills. Tier 2 (best-effort
//! synthesis): Codex projects.
//!
//! Populated in Phase 4 (US2): `claude_code`, `agent_skills` (+ thin
//! `cursor`/`opencode`/`cline`), `codex`.

pub mod claude_code;
pub mod codex;
pub mod native_skill;

/// Per-conversion remote-plugin fetch context (`catalog convert`): the
/// `--no-fetch` policy plus the temp-clone keepalive. The clones MUST outlive
/// `emit` — planned `Copy` files are read from the clone at landing time — so
/// the context is owned by `convert::run`'s scope and dropped (cleaning up
/// every clone) only after the emit completes.
pub struct FetchContext {
    /// `false` under `--no-fetch`: every remote source degrades to the
    /// hermetic warn-and-skip behaviour.
    pub enabled: bool,
    /// Owned temp clones, one per successfully fetched plugin.
    pub keepalive: Vec<tempfile::TempDir>,
}

impl FetchContext {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            keepalive: Vec::new(),
        }
    }
}

/// Shared `convert` diagnostic rule ids — the single source of truth for the
/// per-importer diagnostics, so the `--strict` blocking-set
/// ([`crate::authoring::convert`]) and every importer agree on one stable
/// vocabulary. Promoted here when Codex became the second consumer of the
/// originally-CC-local set.
pub mod rule {
    // Manifest / project level.
    pub const MISSING_VERSION: &str = "convert/missing-version";
    pub const DROPPED_MANIFEST_FIELD: &str = "convert/dropped-manifest-field";
    pub const UNSUPPORTED_MANIFEST_FIELD: &str = "convert/unsupported-manifest-field";
    pub const UNSUPPORTED_COMPONENT: &str = "convert/unsupported-component";
    // Entry / frontmatter level.
    pub const DROPPED_FRONTMATTER: &str = "convert/dropped-frontmatter";
    pub const TOOL_RESTRICTION_DROPPED: &str = "convert/tool-restriction-dropped";
    pub const AGENT_LOSSY: &str = "convert/agent-lossy";
    pub const SKIPPED_ENTRY: &str = "convert/skipped-entry";
    pub const MALFORMED_MCP: &str = "convert/malformed-mcp-server";
    // Codex (Tier-2 synthesis) specific.
    pub const CODEX_SYNTHESIZED_VERSION: &str = "convert/codex-synthesized-version";
    pub const CODEX_UNSUPPORTED: &str = "convert/codex-unsupported";
    // Catalog / marketplace level.
    pub const REMOTE_PLUGIN_SKIPPED: &str = "convert/remote-plugin-skipped";
    /// A remote-source plugin was fetched (shallow clone) and vendored.
    pub const REMOTE_PLUGIN_FETCHED: &str = "convert/remote-plugin-fetched";
    /// A remote-source plugin could not be fetched/imported; skipped
    /// (forward-progress). Strict-blocking.
    pub const REMOTE_PLUGIN_FETCH_FAILED: &str = "convert/remote-plugin-fetch-failed";
    pub const CATALOG_SYNTHESIZED_FIELD: &str = "convert/catalog-synthesized-field";
    /// `hooks/hooks.json` exists but could not be read as UTF-8 text; copied
    /// verbatim, unvalidated. Strict-blocking.
    pub const HOOKS_UNREADABLE: &str = "convert/hooks-unreadable";
    /// A top-level plugin-root directory (or file) was not in the known set and
    /// not in the UNSUPPORTED_COMPONENTS list — it was silently ignored, but any
    /// `${CLAUDE_PLUGIN_ROOT}`-relative references to it will break at runtime.
    pub const UNRECOGNISED_PLUGIN_DIR: &str = "convert/unrecognised-plugin-dir";
    /// An agent body contains a `${TOME_PLUGIN_DIR}` (or similar) token that
    /// the native-agent writer copies verbatim — the substitution layer only
    /// fires on the MCP-served path and not in native agent files. Emitted as
    /// a Warning so the author knows the token will not be resolved at runtime.
    pub const AGENT_UNRESOLVED_TOKEN: &str = "convert/agent-unresolved-token";
    /// A nested `.md` file inside a `commands/` or `agents/` subdirectory was
    /// flattened to a top-level entry name (`<sub>-<stem>`). Info.
    pub const NESTED_ENTRY_FLATTENED: &str = "convert/nested-entry-flattened";
    /// A nested `.md` file was skipped — either its flat name is unsafe or it
    /// collides with another flat name. Warn (content loss).
    pub const NESTED_ENTRY_SKIPPED: &str = "convert/nested-entry-skipped";
    /// A `componentPaths` override was found in `plugin.json` but refers to an
    /// unrecognised component. Dropped with Info.
    pub const COMPONENT_PATH_OVERRIDE_UNRECOGNISED: &str =
        "convert/component-path-override-unrecognised";
}
