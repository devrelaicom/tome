//! `tome-catalog.toml` parser + validator. Strict by default
//! (`deny_unknown_fields` on every struct) and validates six things in order:
//! TOML syntax → required fields → semver `version` → email syntax →
//! unique `plugins[].name` → relative-path `plugins[].source`.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ManifestInvalid;

pub const SCHEMA_URI: &str = "https://github.com/aaronbassett/tome/blob/main/specs/001-phase-1-foundations/contracts/catalog-manifest.schema.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogManifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub owner: Owner,
    #[serde(default)]
    pub plugins: Vec<PluginDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginDeclaration {
    pub name: String,
    pub source: String,
}

impl CatalogManifest {
    /// Parse + validate a `tome-catalog.toml`. `manifest_path` is the path of
    /// the file (used in error messages); `catalog_root` is its parent (used
    /// to validate `plugins[].source` paths).
    pub fn parse_and_validate(
        manifest_path: &Path,
        catalog_root: &Path,
        bytes: &[u8],
    ) -> Result<Self, ManifestInvalid> {
        let text = std::str::from_utf8(bytes).map_err(|e| ManifestInvalid::TomlParse {
            file: manifest_path.to_path_buf(),
            message: format!("manifest is not valid UTF-8: {}", e),
        })?;

        let manifest: Self = toml::from_str(text).map_err(|e| {
            let msg = e.to_string();
            // toml::de::Error wraps unknown-field, missing-field, and parse
            // errors under one type; we differentiate by inspecting the
            // message string. The exhaustive negative corpus in Phase 4
            // confirms each path is exercised.
            classify_toml_error(manifest_path, &msg)
        })?;

        validate_semantic(&manifest, manifest_path)?;

        for plugin in &manifest.plugins {
            validate_source(catalog_root, manifest_path, &plugin.source)?;
        }

        Ok(manifest)
    }
}

fn classify_toml_error(file: &Path, msg: &str) -> ManifestInvalid {
    let lower = msg.to_ascii_lowercase();
    if let Some(key) = extract_unknown_field(&lower, msg) {
        return ManifestInvalid::UnknownField {
            file: file.to_path_buf(),
            key,
            expected_schema_uri: SCHEMA_URI.to_string(),
        };
    }
    if let Some(key) = extract_missing_field(&lower, msg) {
        return ManifestInvalid::MissingField {
            file: file.to_path_buf(),
            key,
        };
    }
    ManifestInvalid::TomlParse {
        file: file.to_path_buf(),
        message: msg.to_string(),
    }
}

fn extract_unknown_field(lower: &str, original: &str) -> Option<String> {
    let marker = "unknown field `";
    let start = lower.find(marker)? + marker.len();
    let rest = &original[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

fn extract_missing_field(lower: &str, original: &str) -> Option<String> {
    let marker = "missing field `";
    let start = lower.find(marker)? + marker.len();
    let rest = &original[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

fn validate_semantic(m: &CatalogManifest, file: &Path) -> Result<(), ManifestInvalid> {
    if m.name.trim().is_empty() {
        return Err(ManifestInvalid::MissingField {
            file: file.to_path_buf(),
            key: "name".into(),
        });
    }
    if m.description.trim().is_empty() {
        return Err(ManifestInvalid::MissingField {
            file: file.to_path_buf(),
            key: "description".into(),
        });
    }
    if semver::Version::parse(&m.version).is_err() {
        return Err(ManifestInvalid::InvalidVersion {
            file: file.to_path_buf(),
            got: m.version.clone(),
        });
    }
    if !looks_like_email(&m.owner.email) {
        return Err(ManifestInvalid::InvalidEmail {
            file: file.to_path_buf(),
            got: m.owner.email.clone(),
        });
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for plugin in &m.plugins {
        if !seen.insert(plugin.name.as_str()) {
            return Err(ManifestInvalid::DuplicatePluginName {
                file: file.to_path_buf(),
                name: plugin.name.clone(),
            });
        }
    }
    Ok(())
}

fn looks_like_email(s: &str) -> bool {
    // Cheap structural check — a single `@`, non-empty local part, non-empty
    // domain with at least one `.`. We do not verify deliverability.
    let mut parts = s.splitn(2, '@');
    let local = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if s.matches('@').count() != 1 {
        return false;
    }
    domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// Validate one `plugins[].source` value. The algorithm follows
/// data-model.md §3 step 6 verbatim.
pub fn validate_source(
    catalog_root: &Path,
    manifest_file: &Path,
    source: &str,
) -> Result<PathBuf, ManifestInvalid> {
    if source.contains("://") || source.starts_with("git@") {
        return Err(ManifestInvalid::SourceLooksLikeUrl {
            file: manifest_file.to_path_buf(),
            value: source.to_string(),
        });
    }
    // Windows drive prefix (e.g. `C:`).
    let bytes = source.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(ManifestInvalid::SourceAbsolute {
            file: manifest_file.to_path_buf(),
            value: source.to_string(),
        });
    }
    let p = Path::new(source);
    if p.is_absolute() {
        return Err(ManifestInvalid::SourceAbsolute {
            file: manifest_file.to_path_buf(),
            value: source.to_string(),
        });
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ManifestInvalid::SourceParentTraversal {
            file: manifest_file.to_path_buf(),
            value: source.to_string(),
        });
    }
    let joined = catalog_root.join(p);
    let resolved = joined
        .canonicalize()
        .map_err(|cause| ManifestInvalid::SourceUnresolvable {
            file: manifest_file.to_path_buf(),
            value: source.to_string(),
            cause,
        })?;
    let root_resolved =
        catalog_root
            .canonicalize()
            .map_err(|cause| ManifestInvalid::CatalogRootUnresolvable {
                root: catalog_root.to_path_buf(),
                cause,
            })?;
    if !resolved.starts_with(&root_resolved) {
        return Err(ManifestInvalid::SourceEscapesRoot {
            file: manifest_file.to_path_buf(),
            value: source.to_string(),
        });
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation_accepts_basic_addresses() {
        assert!(looks_like_email("a@b.co"));
        assert!(looks_like_email("plugins@midnight.network"));
    }

    #[test]
    fn email_validation_rejects_obvious_garbage() {
        assert!(!looks_like_email(""));
        assert!(!looks_like_email("noatsign"));
        assert!(!looks_like_email("a@b"));
        assert!(!looks_like_email("@b.co"));
        assert!(!looks_like_email("a@b.co@c"));
        assert!(!looks_like_email("a@.co"));
        assert!(!looks_like_email("a@b."));
    }
}
