//! Plugin manifests — both the legacy lenient `plugin.json` reader and the
//! Phase 8 native strict [`TomePluginManifest`] (`tome-plugin.toml`).
//!
//! The legacy `plugin.json` parser (third-party input per FR-013a) ignores
//! unknown fields without warning; only `name` is required. A missing or
//! malformed `plugin.json` maps to [`TomeError::PluginManifestParseError`]
//! (exit 22) per FR-013b. After the Phase 8 cutover this lenient reader is no
//! longer the manifest source — it survives only so `convert`/`doctor` can
//! recognise an *unconverted* plugin (`.claude-plugin/plugin.json` present, no
//! `tome-plugin.toml`).
//!
//! [`TomePluginManifest`] is the native, strict (`deny_unknown_fields`),
//! Tome-owned manifest read from `<plugin>/tome-plugin.toml`. It is the single
//! source of truth shared by the US1 reader and the [`crate::authoring`]
//! emitter (`data-model.md §1`).
//!
//! Spec: data-model.md §1 + §3, plugin-commands.md, manifest-cutover.md.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
///
/// `plugin.json` is third-party (FR-013a); an oversized file must not be
/// read into memory unbounded. The read is capped at
/// [`crate::util::PLUGIN_MANIFEST_MAX`] (FR-006, F-PLUGIN-MANIFEST-DOS) — an
/// over-cap file fails as a `PluginManifestParseError`, the same class this
/// site already produces for a missing or malformed manifest.
pub fn parse_plugin_manifest(path: &Path) -> Result<PluginManifest, TomeError> {
    let bytes =
        crate::util::bounded_read(path, crate::util::PLUGIN_MANIFEST_MAX).map_err(|cause| {
            TomeError::PluginManifestParseError {
                file: path.to_path_buf(),
                message: format!("could not read file: {cause}"),
            }
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

/// Conventional location for the legacy manifest relative to the plugin
/// directory.
pub fn manifest_path_for(plugin_dir: &Path) -> PathBuf {
    plugin_dir.join(".claude-plugin").join("plugin.json")
}

// ---------------------------------------------------------------------------
// Phase 8 — native strict `tome-plugin.toml` manifest.
// ---------------------------------------------------------------------------

/// The native Tome plugin manifest, read from `<plugin>/tome-plugin.toml`.
///
/// Tome-owned input → **strict** (`deny_unknown_fields`): an unknown top-level
/// field is a parse error (principle IV). `name` + `version` are required;
/// `description`, `license`, and `[author]` are optional. The single source of
/// truth shared by the US1 reader ([`Self::read`]) and the
/// [`crate::authoring`] emitter (which serialises this struct). See
/// `data-model.md §1`.
///
/// Optional fields use `skip_serializing_if` so the emitter omits absent
/// fields — keeping emitted manifests minimal and byte-stable (FR-027).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TomePluginManifest {
    /// Plugin name. REQUIRED. A safe path segment; `name == <plugin dir>` is
    /// the convention (a mismatch is a `lint` warning, not a manifest error).
    pub name: String,
    /// Plugin version. REQUIRED, valid semver.
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// SPDX licence string. Non-empty when present (`lint` may warn on an
    /// unrecognised SPDX id; the manifest does not reject it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<TomeAuthor>,
}

/// `[author]` table on a [`TomePluginManifest`]. Both fields optional; reused
/// by the authoring IR ([`crate::authoring::ir`]) as the single author type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TomeAuthor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Conventional location for the native manifest relative to the plugin dir.
pub fn tome_manifest_path_for(plugin_dir: &Path) -> PathBuf {
    plugin_dir.join("tome-plugin.toml")
}

impl TomePluginManifest {
    /// Read and validate `<plugin>/tome-plugin.toml`. The bounded read caps the
    /// file at [`crate::util::PLUGIN_MANIFEST_MAX`] (Tome-owned, but still
    /// untrusted on the `convert`/`lint` source side). Every failure maps to
    /// [`TomeError::PluginManifestParseError`] (exit 22), naming the file +
    /// the offending field (principle V).
    pub fn read(plugin_dir: &Path) -> Result<Self, TomeError> {
        let path = tome_manifest_path_for(plugin_dir);
        let bytes = crate::util::bounded_read(&path, crate::util::PLUGIN_MANIFEST_MAX).map_err(
            |cause| TomeError::PluginManifestParseError {
                file: path.clone(),
                message: format!("could not read file: {cause}"),
            },
        )?;
        Self::parse_and_validate(&path, &bytes)
    }

    /// Parse + validate a `tome-plugin.toml` from in-memory bytes. `file` is
    /// recorded in any error; it need not exist on disk (used by the emitter's
    /// round-trip tests).
    pub fn parse_and_validate(file: &Path, bytes: &[u8]) -> Result<Self, TomeError> {
        let err = |message: String| TomeError::PluginManifestParseError {
            file: file.to_path_buf(),
            message,
        };

        let text = std::str::from_utf8(bytes).map_err(|e| err(format!("not valid UTF-8: {e}")))?;
        // `deny_unknown_fields` + the absence of `#[serde(default)]` on `name`
        // and `version` makes toml reject unknown / missing-required fields
        // here; the message names the field.
        let manifest: Self = toml::from_str(text).map_err(|e| err(e.to_string()))?;
        manifest.validate(file)?;
        Ok(manifest)
    }

    /// Semantic validation in declared order (data-model §1).
    fn validate(&self, file: &Path) -> Result<(), TomeError> {
        let err = |message: String| TomeError::PluginManifestParseError {
            file: file.to_path_buf(),
            message,
        };

        // 2. `name` non-empty + a safe path segment. Control chars are also
        //    rejected: the name is copied verbatim into the emitted manifest +
        //    a directory name, so a newline/NUL is unusable input (the same
        //    boundary discipline as `catalog::manifest`).
        if self.name.trim().is_empty() {
            return Err(err("`name` must not be empty".to_owned()));
        }
        if self.name.chars().any(char::is_control) {
            return Err(err("`name` must not contain control characters".to_owned()));
        }
        crate::plugin::identity::validate_segment(&self.name)
            .map_err(|kind| err(format!("`name` is not a safe path segment: {kind}")))?;

        // 3. `version` valid semver.
        semver::Version::parse(&self.version)
            .map_err(|_| err(format!("`version` is not valid semver: {}", self.version)))?;

        // 4. `author.email` structural if present + non-empty.
        if let Some(author) = &self.author
            && let Some(email) = author.email.as_deref().map(str::trim)
            && !email.is_empty()
            && !crate::catalog::manifest::looks_like_email(email)
        {
            return Err(err(format!("`author.email` is not a valid email: {email}")));
        }

        // 5. `license` non-empty when present.
        if let Some(license) = &self.license
            && license.trim().is_empty()
        {
            return Err(err("`license` must not be empty when present".to_owned()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tome_manifest_tests {
    use super::*;

    fn parse(s: &str) -> Result<TomePluginManifest, TomeError> {
        TomePluginManifest::parse_and_validate(Path::new("tome-plugin.toml"), s.as_bytes())
    }

    #[test]
    fn minimal_valid_manifest_parses() {
        let m = parse("name = \"foo\"\nversion = \"1.2.3\"\n").expect("valid");
        assert_eq!(m.name, "foo");
        assert_eq!(m.version, "1.2.3");
        assert!(m.description.is_none());
        assert!(m.author.is_none());
    }

    #[test]
    fn full_manifest_round_trips_through_serialize() {
        let src = "name = \"foo\"\nversion = \"1.2.3\"\ndescription = \"a plugin\"\nlicense = \"MIT\"\n\n[author]\nname = \"Jo\"\nemail = \"jo@example.com\"\n";
        let m = parse(src).expect("valid");
        let serialized = toml::to_string(&m).expect("serialize");
        let reparsed = parse(&serialized).expect("re-parse");
        assert_eq!(m, reparsed);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = parse("name = \"foo\"\nversion = \"1.0.0\"\nhomepage = \"x\"\n").unwrap_err();
        assert!(matches!(err, TomeError::PluginManifestParseError { .. }));
    }

    #[test]
    fn missing_version_is_rejected() {
        let err = parse("name = \"foo\"\n").unwrap_err();
        assert!(matches!(err, TomeError::PluginManifestParseError { .. }));
    }

    #[test]
    fn bad_semver_is_rejected() {
        let err = parse("name = \"foo\"\nversion = \"not-semver\"\n").unwrap_err();
        match err {
            TomeError::PluginManifestParseError { message, .. } => {
                assert!(message.contains("semver"), "got: {message}");
            }
            other => panic!("expected parse error, got {other:?}"),
        }
    }

    #[test]
    fn unsafe_name_is_rejected() {
        for bad in ["..", "a/b", ".hidden", ""] {
            let src = format!("name = \"{bad}\"\nversion = \"1.0.0\"\n");
            assert!(parse(&src).is_err(), "name `{bad}` should be rejected");
        }
    }

    #[test]
    fn control_char_in_name_is_rejected() {
        let err = parse("name = \"foo\\nbar\"\nversion = \"1.0.0\"\n").unwrap_err();
        assert!(matches!(err, TomeError::PluginManifestParseError { .. }));
    }

    #[test]
    fn malformed_author_email_is_rejected() {
        let err = parse("name = \"foo\"\nversion = \"1.0.0\"\n\n[author]\nemail = \"noatsign\"\n")
            .unwrap_err();
        assert!(matches!(err, TomeError::PluginManifestParseError { .. }));
    }

    #[test]
    fn empty_license_is_rejected() {
        let err = parse("name = \"foo\"\nversion = \"1.0.0\"\nlicense = \"  \"\n").unwrap_err();
        assert!(matches!(err, TomeError::PluginManifestParseError { .. }));
    }
}
