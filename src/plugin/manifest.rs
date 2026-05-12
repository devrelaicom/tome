//! Lenient parser for `plugin.json`.
//!
//! Third-party input per FR-013a — unknown fields are ignored without warning.
//! Only `name` is required; everything else is optional with sensible defaults.
//! A missing or malformed `plugin.json` maps to [`TomeError::PluginManifestParseError`]
//! (exit code 22) per FR-013b.
//!
//! Spec: data-model.md §3, plugin-commands.md, FR-013a/b.

use std::path::{Path, PathBuf};

use crate::error::TomeError;

/// Subset of `plugin.json` Tome consumes. Other fields (commands, hooks,
/// mcpServers, etc.) are deliberately omitted — serde will skip them under
/// the lenient parse policy.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<PluginAuthor>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginAuthor {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

impl PluginAuthor {
    /// Render as `Name <email>` when both are known, falling back to whichever
    /// is present. Returns `None` if both fields are absent or empty.
    pub fn display(&self) -> Option<String> {
        let name = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let email = self
            .email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match (name, email) {
            (Some(n), Some(e)) => Some(format!("{n} <{e}>")),
            (Some(n), None) => Some(n.to_owned()),
            (None, Some(e)) => Some(e.to_owned()),
            (None, None) => None,
        }
    }
}

/// Read and parse a `plugin.json` from disk. Returns a structured error that
/// maps to exit code 22 at the command boundary.
pub fn parse_plugin_manifest(path: &Path) -> Result<PluginManifest, TomeError> {
    let bytes = std::fs::read(path).map_err(|cause| TomeError::PluginManifestParseError {
        file: path.to_path_buf(),
        message: format!("could not read file: {cause}"),
    })?;

    parse_plugin_manifest_bytes(path, &bytes)
}

/// Parse a `plugin.json` from in-memory bytes. The `file` path is recorded in
/// any error for diagnostics; it does not have to exist on disk.
pub fn parse_plugin_manifest_bytes(file: &Path, bytes: &[u8]) -> Result<PluginManifest, TomeError> {
    let manifest: PluginManifest =
        serde_json::from_slice(bytes).map_err(|cause| TomeError::PluginManifestParseError {
            file: file.to_path_buf(),
            message: cause.to_string(),
        })?;

    if manifest.name.trim().is_empty() {
        return Err(TomeError::PluginManifestParseError {
            file: file.to_path_buf(),
            message: "`name` field is missing or empty".to_owned(),
        });
    }

    Ok(manifest)
}

/// Conventional location for the manifest relative to the plugin directory.
pub fn manifest_path_for(plugin_dir: &Path) -> PathBuf {
    plugin_dir.join(".claude-plugin").join("plugin.json")
}
