//! Thin TOML deserialisation entry points for the three settings shapes.
//!
//! Each function takes the **content** of one settings file (UTF-8) and
//! returns the strongly-typed value or a [`ParseError`] carrying the
//! underlying `toml::de::Error`. Call sites in `src/commands/workspace/`
//! and elsewhere wrap the error with the file path they have in scope
//! and map to [`crate::error::TomeError::WorkspaceMalformed`] (exit 70)
//! or the equivalent context-specific variant.
//!
//! These functions are intentionally content-only — keeping them
//! path-free lets the resolver tests construct fixtures inline without
//! a `tempfile::TempDir` round-trip.

use std::fmt;

use super::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};

/// Error returned by the parser functions. Path-free by design: the
/// caller knows which file it was reading and wraps with the path
/// before surfacing to the user.
#[derive(Debug)]
pub struct ParseError {
    /// The shape we were trying to deserialise — included in the
    /// formatted message so caller-side logs identify the layer.
    layer: SettingsLayer,
    source: toml::de::Error,
}

#[derive(Debug, Clone, Copy)]
enum SettingsLayer {
    Workspace,
    ProjectMarker,
    Global,
}

impl fmt::Display for SettingsLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace => f.write_str("workspace settings"),
            Self::ProjectMarker => f.write_str("project marker config"),
            Self::Global => f.write_str("global settings"),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.layer, self.source)
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl ParseError {
    /// Access the wrapped TOML deserialiser error. Useful for tests
    /// asserting on the message text (e.g. unknown-field rejection).
    pub fn toml_error(&self) -> &toml::de::Error {
        &self.source
    }
}

/// Parse a workspace settings file (`<root>/workspaces/<name>/settings.toml`).
pub fn parse_workspace(content: &str) -> Result<WorkspaceSettings, ParseError> {
    toml::from_str(content).map_err(|source| ParseError {
        layer: SettingsLayer::Workspace,
        source,
    })
}

/// Parse a project marker config (`<project>/.tome/config.toml`).
pub fn parse_project_marker(content: &str) -> Result<ProjectMarkerConfig, ParseError> {
    toml::from_str(content).map_err(|source| ParseError {
        layer: SettingsLayer::ProjectMarker,
        source,
    })
}

/// Parse the global settings file (`<root>/settings.toml`). The empty
/// string yields an empty [`GlobalSettings`] because every field is
/// `#[serde(default)]`.
pub fn parse_global(content: &str) -> Result<GlobalSettings, ParseError> {
    toml::from_str(content).map_err(|source| ParseError {
        layer: SettingsLayer::Global,
        source,
    })
}
