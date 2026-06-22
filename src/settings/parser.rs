//! Thin TOML deserialisation entry points for the two settings shapes.
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
//!
//! Note: the former `parse_global` function has been removed (Task 2 /
//! fix-4). The global harness layer now lives in `config.toml` under the
//! `[harness]` section and is loaded via `crate::config::load`. There is
//! no longer a separate `settings.toml` global file.

use std::fmt;
use std::path::Path;

use super::{ProjectMarkerConfig, WorkspaceSettings};
use crate::error::TomeError;

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
}

impl fmt::Display for SettingsLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace => f.write_str("workspace settings"),
            Self::ProjectMarker => f.write_str("project marker config"),
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

// ---------------------------------------------------------------------------
// Polish R-M5: canonical path-aware project-marker reader
// ---------------------------------------------------------------------------

/// Read and parse the project marker at `path`.
///
/// Polish R-M5: consolidates three near-identical readers (previously
/// at `workspace::resolution::read_project_marker`,
/// `harness::sync::read_project_marker`,
/// `commands::harness::list::load_project_marker`) plus two inline
/// read-and-parse pairs (`doctor::mod` + `doctor::binding`).
///
/// IO failures (the file is unreadable for reasons other than absence)
/// surface as [`TomeError::Io`] (exit 7); parse failures surface as
/// [`TomeError::WorkspaceMalformed`] (exit 70) carrying `path`.
///
/// `NotFound` is propagated as `Err(TomeError::Io(_))` rather than
/// `Ok(None)` — caller-side Option-wrapping (e.g.
/// `commands::harness::list::load_project_marker`) is the canonical
/// place to special-case absence. Doctor consumers that want
/// silent-on-error semantics call this and discard via `.ok()`.
pub fn read_project_marker(path: &Path) -> Result<ProjectMarkerConfig, TomeError> {
    let body = crate::util::bounded_read_to_string(path, crate::util::TOME_CONFIG_MAX)?;
    parse_project_marker(&body).map_err(|e| TomeError::WorkspaceMalformed {
        path: path.to_path_buf(),
        reason: format!("parse project marker: {e}"),
    })
}
