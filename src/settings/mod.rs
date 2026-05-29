//! Layered settings + composition resolution.
//!
//! Phase 4 introduces three settings layers that combine to produce the
//! effective harness list for a given (project, workspace, global) context:
//!
//! 1. **Project** — `<project>/.tome/config.toml` ([`ProjectMarkerConfig`])
//! 2. **Workspace** — `<root>/workspaces/<name>/settings.toml`
//!    ([`WorkspaceSettings`])
//! 3. **Global** — `<root>/settings.toml` ([`GlobalSettings`])
//!
//! The resolver walks these in priority order, stops at the **first
//! scope that declares a `harnesses` key** (FR-441), and follows any
//! composition references inside that scope's list to other scopes'
//! **directly-declared** lists (FR-449 — the "one-level reference, not
//! a re-entrant resolver" rule).
//!
//! F8 ships the parser + composition type + resolver skeleton. US3
//! wires the resolver into the CLI surface and lights up central-DB
//! lookups for `[workspaces.<name>]` references.
//!
//! All settings types live behind `#[serde(deny_unknown_fields)]` —
//! they are Tome-owned declarative inputs and fall on the strict side
//! of the FR-013a strictness boundary.

use serde::{Deserialize, Serialize};

use crate::workspace::WorkspaceName;

pub mod composition;
pub mod edit;
pub mod parser;
pub mod resolver;

pub use composition::CompositionRef;
pub use resolver::{EffectiveHarness, EffectiveHarnessList, ScopeKind, resolve_effective_list};

/// First-declarer-wins priority walk for a Phase 6 scalar `bool` setting
/// (FR-053, R-12). The nearest scope that **declares** the field
/// (`Some(v)`) wins — project, then workspace, then global; an absent
/// key (`None`) at a scope falls through to the next. When no scope
/// declares the field the default is `false`.
///
/// This is deliberately NOT the `harnesses` composition grammar
/// (`resolve_effective_list`): there is no list to union/subtract and no
/// `[workspace]` / `[global]` / `!name` references — a project `false`
/// simply overrides a global `true` (`settings-p6.md`).
///
/// The three arguments are the field's already-extracted value at each
/// scope, in priority order. Call sites extract via a per-scope accessor
/// (e.g. `project.map(|p| p.expose_agents_as_personas).flatten()`); the
/// extraction is the "field accessor" seam, so a second Phase 6 scalar
/// (US5's `strip_plugin_agent_privileges`) reuses this resolver verbatim
/// by passing its own field's three values. See [`resolve_scalar_with`]
/// for the closure-based form that takes the structs directly.
pub fn resolve_scalar(
    project: Option<bool>,
    workspace: Option<bool>,
    global: Option<bool>,
) -> bool {
    project.or(workspace).or(global).unwrap_or(false)
}

/// Closure-based form of [`resolve_scalar`]: takes the three settings
/// scopes plus one field accessor per scope and applies the
/// first-declarer-wins walk. Keeps the resolver generic over the field
/// being resolved (the accessor is the only thing that changes between
/// `expose_agents_as_personas` and US5's `strip_plugin_agent_privileges`),
/// so a second scalar adds a one-line call site, not a second resolver.
pub fn resolve_scalar_with<FP, FW, FG>(
    project: Option<&ProjectMarkerConfig>,
    workspace: Option<&WorkspaceSettings>,
    global: &GlobalSettings,
    project_field: FP,
    workspace_field: FW,
    global_field: FG,
) -> bool
where
    FP: Fn(&ProjectMarkerConfig) -> Option<bool>,
    FW: Fn(&WorkspaceSettings) -> Option<bool>,
    FG: Fn(&GlobalSettings) -> Option<bool>,
{
    resolve_scalar(
        project.and_then(project_field),
        workspace.and_then(workspace_field),
        global_field(global),
    )
}

/// Contents of `<root>/workspaces/<name>/settings.toml`.
///
/// Mirrors data-model §6. The `name` field MUST match the on-disk
/// directory name; the parser does not enforce that invariant — callers
/// in US2 (workspace commands) cross-check.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSettings {
    pub name: WorkspaceName,
    #[serde(default)]
    pub summaries: Option<CachedSummaries>,
    #[serde(default)]
    pub catalogs: Vec<CatalogEntry>,
    /// `None` = no `harnesses` key declared in the file (fall-through to
    /// the next scope per FR-441). `Some(vec)` = declared (vec may be
    /// empty, which opts out of the priority walk entirely).
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,
    /// Phase 6 (FR-060/FR-067): expose each enabled agent as a
    /// `<name>-persona` MCP prompt plus one global `drop-persona`. `None`
    /// = key absent (fall through to the next scope in the
    /// first-declarer-wins walk); `Some(v)` = declared at this scope and
    /// terminates the walk. See `agent-personas.md` / `settings-p6.md`.
    #[serde(default)]
    pub expose_agents_as_personas: Option<bool>,
}

/// Cached short + long summaries with their generation timestamp.
/// Regenerated on enable / disable / reindex / explicit regen-summary
/// triggers per US4. Distinct from `WorkspaceSettings::summaries` only
/// in that the type is the wire shape of the `[summaries]` table.
///
/// `generated_at` accepts either TOML's native datetime literal (e.g.
/// `2026-05-14T15:00:00Z` unquoted) or an RFC 3339 string literal
/// (e.g. `"2026-05-14T15:00:00Z"`). The internal codec round-trips
/// through `toml::value::Datetime` on the TOML side and serialises as
/// an RFC 3339 string on the JSON side.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CachedSummaries {
    pub short: String,
    pub long: String,
    #[serde(with = "toml_or_rfc3339")]
    pub generated_at: time::OffsetDateTime,
}

/// Bridge serde module: accepts both TOML datetime literals (deserialised
/// as `toml::value::Datetime`) and RFC 3339 strings. Emits RFC 3339.
mod toml_or_rfc3339 {
    use serde::de::{Deserializer, Error as _};
    use serde::ser::{Error as _, Serializer};
    use serde::{Deserialize, Serialize};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    pub fn serialize<S: Serializer>(
        value: &OffsetDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let formatted = value
            .format(&Rfc3339)
            .map_err(|e| S::Error::custom(format!("rfc3339 format failed: {e}")))?;
        formatted.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        // `Repr` accepts either a bare RFC 3339 string or a
        // `toml::value::Datetime` (which itself serialises as a
        // string-shaped helper struct). Using `serde_json::Value` as a
        // catch-all would pull in extra work; `toml::value::Datetime`
        // is the upstream type and has a `Display` we can re-parse.
        #[derive(Deserialize)] // not-strict: `#[serde(untagged)]` is mutually exclusive with `deny_unknown_fields`
        #[serde(untagged)]
        enum Repr {
            Toml(toml::value::Datetime),
            Str(String),
        }

        let raw = Repr::deserialize(deserializer)?;
        let s = match raw {
            Repr::Toml(dt) => dt.to_string(),
            Repr::Str(s) => s,
        };
        OffsetDateTime::parse(&s, &Rfc3339)
            .map_err(|e| D::Error::custom(format!("invalid RFC 3339 datetime `{s}`: {e}")))
    }
}

/// A single `[[catalogs]]` entry in the workspace settings file.
///
/// The `ref` field uses a raw identifier because `ref` is a Rust keyword.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub name: String,
    pub url: String,
    pub r#ref: String,
}

/// Contents of `<project>/.tome/config.toml` (the project marker).
///
/// Mirrors data-model §7. The `workspace` field is the binding pointer
/// from the project to its workspace; it is required because the
/// project marker exists *because* a project was bound to a workspace
/// via `tome workspace use`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectMarkerConfig {
    pub workspace: WorkspaceName,
    /// Project-scope harness declaration. Composition references
    /// (`[workspace]`, `[global]`, `[workspaces.<name>]`) are allowed
    /// here; the parser does not validate them — the resolver does.
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,
    /// Phase 6 (FR-060/FR-067): see [`WorkspaceSettings::expose_agents_as_personas`].
    /// Declared here it wins the first-declarer-wins walk (nearest scope).
    #[serde(default)]
    pub expose_agents_as_personas: Option<bool>,
}

/// Contents of `<root>/settings.toml` (global Tome settings).
///
/// Mirrors data-model §8. `Default` is provided so callers can treat an
/// absent file as the empty case.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GlobalSettings {
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,
    /// Phase 6 (FR-060/FR-067): see [`WorkspaceSettings::expose_agents_as_personas`].
    /// Declared at global scope it is the org-wide default when nearer
    /// scopes leave the key absent.
    #[serde(default)]
    pub expose_agents_as_personas: Option<bool>,
}

#[cfg(test)]
mod scalar_resolver_tests {
    use super::*;

    #[test]
    fn defaults_false_when_nowhere_declared() {
        assert!(!resolve_scalar(None, None, None));
    }

    #[test]
    fn project_declaration_wins_over_global() {
        // project `false` overrides global `true` — the defining behaviour.
        assert!(!resolve_scalar(Some(false), None, Some(true)));
        assert!(resolve_scalar(Some(true), None, Some(false)));
    }

    #[test]
    fn workspace_wins_when_project_absent() {
        assert!(resolve_scalar(None, Some(true), Some(false)));
        assert!(!resolve_scalar(None, Some(false), Some(true)));
    }

    #[test]
    fn falls_through_to_global() {
        assert!(resolve_scalar(None, None, Some(true)));
        assert!(!resolve_scalar(None, None, Some(false)));
    }

    #[test]
    fn closure_form_threads_the_accessor() {
        let global = GlobalSettings {
            harnesses: None,
            expose_agents_as_personas: Some(true),
        };
        let resolved = resolve_scalar_with(
            None,
            None,
            &global,
            |p| p.expose_agents_as_personas,
            |w| w.expose_agents_as_personas,
            |g| g.expose_agents_as_personas,
        );
        assert!(resolved);
    }
}
