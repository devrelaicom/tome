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
pub mod parser;
pub mod resolver;

pub use composition::CompositionRef;
pub use resolver::{EffectiveHarness, EffectiveHarnessList, ScopeKind, resolve_effective_list};

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
}
