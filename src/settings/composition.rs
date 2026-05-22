//! Composition reference parsing.
//!
//! Per research §R-9 + FR-450, the composition reference forms
//! (`[workspace]`, `[workspaces.<name>]`, `[global]`) are **TOML string
//! values** containing brackets, NOT TOML table headers. The settings
//! files declare `harnesses: Vec<String>` and the resolver parses each
//! string into a [`CompositionRef`] via the [`CompositionRef::parse`]
//! ladder below.
//!
//! Parse rules (FR-443):
//!
//! | Form | Variant |
//! |------|---------|
//! | `"[workspace]"` | [`CompositionRef::CurrentWorkspace`] |
//! | `"[global]"` | [`CompositionRef::Global`] |
//! | `"[workspaces.<name>]"` | [`CompositionRef::NamedWorkspace`] |
//! | `"!<name>"` | [`CompositionRef::Exclude`] |
//! | `"<name>"` (anything else) | [`CompositionRef::Include`] |
//!
//! Per FR-448, a `!`-prefix immediately followed by a `[` (i.e.
//! `![workspace]`, `![global]`, `![workspaces.<name>]`) is rejected at
//! parse time as [`CompositionErrorKind::BadExclusion`] — composition
//! references describe scopes, and "exclude a scope" is not a defined
//! operation.

use crate::error::CompositionErrorKind;
use crate::workspace::WorkspaceName;

/// A single entry in a `harnesses` array after string-pattern parsing.
///
/// The variant set mirrors data-model §9 verbatim. Equality is
/// structural so resolver tests can `assert_eq!` against fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionRef {
    /// Plain harness name — include this harness in the effective list.
    Include(String),
    /// Exclude a harness from the union (subtract from inclusion set).
    Exclude(String),
    /// Reference the bound workspace's directly-declared list. Valid
    /// only in project scope (FR-449); the resolver enforces this.
    CurrentWorkspace,
    /// Reference a named workspace's directly-declared list. The
    /// resolver consults the central registry to confirm the workspace
    /// exists (UnknownWorkspace → exit 13 when surfaced via the
    /// workspace-resolution surface, 17 elsewhere).
    NamedWorkspace(WorkspaceName),
    /// Reference the global settings' directly-declared list. Terminal
    /// — the resolver does not recurse further from `[global]`.
    Global,
}

impl CompositionRef {
    /// Parse a single harness-array string into a [`CompositionRef`].
    ///
    /// See the module-level docs for the full parse table. Returns
    /// [`CompositionErrorKind::BadExclusion`] for `!`-prefixed
    /// bracketed forms (`![global]`, `![workspace]`,
    /// `![workspaces.x]`) per FR-448.
    pub fn parse(s: &str) -> Result<Self, CompositionErrorKind> {
        // 1. `!`-prefixed exclusion. Bracketed targets are rejected.
        if let Some(rest) = s.strip_prefix('!') {
            if rest.starts_with('[') {
                return Err(CompositionErrorKind::BadExclusion(s.to_owned()));
            }
            return Ok(CompositionRef::Exclude(rest.to_owned()));
        }

        // 2. `[workspace]` — current workspace reference.
        if s == "[workspace]" {
            return Ok(CompositionRef::CurrentWorkspace);
        }

        // 3. `[global]` — global scope reference.
        if s == "[global]" {
            return Ok(CompositionRef::Global);
        }

        // 4. `[workspaces.<name>]` — named workspace reference.
        if let Some(rest) = s.strip_prefix("[workspaces.")
            && let Some(name) = rest.strip_suffix(']')
        {
            // F8 SKELETON: no validation of the inner name. F10's
            // `WorkspaceName::parse` will enforce FR-347 character /
            // length / reserved-name rules.
            return Ok(CompositionRef::NamedWorkspace(
                WorkspaceName::from_string_unvalidated(name.to_owned()),
            ));
        }

        // 5. Anything else is a plain inclusion. Unknown-harness
        // validation lives in the resolver, not here.
        Ok(CompositionRef::Include(s.to_owned()))
    }
}
