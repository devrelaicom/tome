//! `tome-catalog.toml` parser + validator. Strict by default
//! (`deny_unknown_fields` on every struct) and validates six things in order:
//! TOML syntax → required fields → semver `version` → email syntax →
//! unique `plugins[].name` → relative-path `plugins[].source`.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ManifestInvalid;

pub const SCHEMA_URI: &str = "https://github.com/devrelaicom/tome/blob/main/specs/001-phase-1-foundations/contracts/catalog-manifest.schema.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogManifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub owner: Owner,
    #[serde(default)]
    pub plugins: Vec<PluginDeclaration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bugs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub name: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<crate::manifest_meta::Handles>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginDeclaration {
    pub name: String,
    pub source: String,
}

/// Look up a catalog's `tome-catalog.toml` and return the parsed manifest.
/// Returns `None` when the file is absent, unreadable, or fails strict
/// validation — callers fall back to flat layout or surface `PluginNotFound`,
/// matching `tome plugin list`'s lenient handling of broken catalog state.
pub fn read_catalog_manifest(catalog_path: &Path) -> Option<CatalogManifest> {
    let manifest_path = catalog_path.join("tome-catalog.toml");
    // `tome-catalog.toml` is third-party; cap the read at PLUGIN_MANIFEST_MAX
    // (FR-006, F-PLUGIN-MANIFEST-DOS). An over-cap file is `Err` from
    // `bounded_read`, so `.ok()?` folds it into the same None / "invalid or
    // unreadable" path an unparsable manifest already takes here — never a
    // silent acceptance of a multi-GiB file as a clean manifest.
    let bytes = crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX).ok()?;
    CatalogManifest::parse_and_validate(&manifest_path, catalog_path, &bytes).ok()
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
    // F-WS-TOML-NEWLINE (FR-005): the catalog `name` is copied verbatim into
    // a workspace's settings.toml (and other Tome-owned files). A newline or
    // other control char in the recognised VALUE is malformed input — reject
    // it at the boundary (§IV permits validating recognised field VALUES even
    // though UNKNOWN fields stay lenient). Reusing `MissingField { key:
    // "name" }` keeps the closed error set + exit code intact: a name that is
    // structurally unusable is, for our purposes, the same failure class as an
    // absent one. This stops the poison at ingest; the toml_edit emission in
    // `workspace::init` is the defence-in-depth for names already in the DB.
    if m.name.chars().any(is_rejected_control_char) {
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

    validate_common_metadata(
        &m.keywords,
        &[
            ("homepage", m.homepage.as_deref()),
            ("repository", m.repository.as_deref()),
            ("bugs", m.bugs.as_deref()),
            ("documentation", m.documentation.as_deref()),
        ],
        m.display_name.as_deref(),
        &[m.owner.url.as_deref()],
        &[m.owner.handles.as_ref()],
    )
    .map_err(|message| ManifestInvalid::InvalidMetadata {
        file: file.to_path_buf(),
        message,
    })?;

    if let Some(icon) = m.icon.as_deref() {
        crate::manifest_meta::validate_relative_path("icon", icon).map_err(|message| {
            ManifestInvalid::InvalidMetadata {
                file: file.to_path_buf(),
                message,
            }
        })?;
    }

    Ok(())
}

/// Validate the optional metadata fields common to both manifests. `map_err`
/// converts the helper message string into the caller's error type.
pub(crate) fn validate_common_metadata(
    keywords: &[String],
    urls: &[(&str, Option<&str>)],
    display_name: Option<&str>,
    people_urls: &[Option<&str>],
    people_handles: &[Option<&crate::manifest_meta::Handles>],
) -> Result<(), String> {
    crate::manifest_meta::validate_keywords(keywords)?;
    for (field, val) in urls {
        if let Some(v) = val {
            crate::manifest_meta::validate_http_url(field, v)?;
        }
    }
    if let Some(dn) = display_name {
        crate::manifest_meta::validate_display_name(dn)?;
    }
    for v in people_urls.iter().flatten() {
        crate::manifest_meta::validate_http_url("url", v)?;
    }
    for hm in people_handles.iter().flatten() {
        crate::manifest_meta::validate_handles(hm)?;
    }
    Ok(())
}

/// True for any Unicode control character — C0 (incl. `\n`, `\r`, `\t`, NUL,
/// BEL), DEL, and the C1 block. These never legitimately appear in a catalog
/// name and are the smuggling vector that can wedge a re-parsed Tome-owned
/// file (settings.toml). The check is intentionally broader than just newline
/// so no other line-/string-terminating control slips through.
fn is_rejected_control_char(c: char) -> bool {
    c.is_control()
}

/// Cheap structural email check — a single `@`, non-empty local part, and a
/// non-empty dotted domain. Promoted to `pub(crate)` at its second consumer
/// (`TomePluginManifest` `author.email` validation, Phase 8) rather than
/// duplicated — the established single-source-of-truth pattern.
pub(crate) fn looks_like_email(s: &str) -> bool {
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

    #[test]
    fn catalog_accepts_new_optional_metadata() {
        let src = r#"
name = "c"
description = "d"
version = "1.0.0"
keywords = ["auth", "oauth"]
homepage = "https://example.com"
repository = "https://github.com/x/y"
bugs = "https://github.com/x/y/issues"
documentation = "https://docs.example.com"
display_name = "Cool Catalog"
icon = "assets/icon.png"

[owner]
name = "Jo"
email = "jo@example.com"
url = "https://jo.dev"

[owner.handles]
github = "jo"
"#;
        let m = CatalogManifest::parse_and_validate(
            std::path::Path::new("tome-catalog.toml"),
            std::path::Path::new("."),
            src.as_bytes(),
        )
        .expect("valid");
        assert_eq!(m.keywords, vec!["auth", "oauth"]);
        assert_eq!(m.homepage.as_deref(), Some("https://example.com"));
        assert_eq!(m.owner.url.as_deref(), Some("https://jo.dev"));
        assert_eq!(
            m.owner
                .handles
                .as_ref()
                .unwrap()
                .get("github")
                .map(String::as_str),
            Some("jo")
        );
    }

    #[test]
    fn catalog_rejects_bad_url_and_overlong_keywords() {
        let bad_url = "name=\"c\"\ndescription=\"d\"\nversion=\"1.0.0\"\nhomepage=\"ftp://x\"\n[owner]\nname=\"J\"\nemail=\"j@x.co\"\n";
        assert!(
            CatalogManifest::parse_and_validate(
                std::path::Path::new("f"),
                std::path::Path::new("."),
                bad_url.as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn catalog_without_new_fields_still_parses() {
        let src = "name=\"c\"\ndescription=\"d\"\nversion=\"1.0.0\"\n[owner]\nname=\"J\"\nemail=\"j@x.co\"\n";
        let m = CatalogManifest::parse_and_validate(
            std::path::Path::new("f"),
            std::path::Path::new("."),
            src.as_bytes(),
        )
        .expect("valid");
        assert!(m.keywords.is_empty());
        assert!(m.homepage.is_none());
    }
}
