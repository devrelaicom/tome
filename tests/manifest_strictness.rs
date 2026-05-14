//! Strictness coverage for the manifest and config parsers. Three layers:
//!
//! 1. Structural grep guard (R-7) — every `Deserialize`-deriving struct in
//!    `src/catalog/manifest.rs` and `src/config.rs` carries
//!    `#[serde(deny_unknown_fields)]`. Catches the regression at lint time.
//! 2. Exhaustive bad-manifest corpus — one test per `ManifestInvalid`
//!    variant (and matching strictness rejection). SC-005: 100% of the
//!    documented malformed shapes are refused.
//! 3. Config strictness corpus — same posture for `config.toml`. The
//!    `Config` and `CatalogEntry` `deny_unknown_fields` attributes are
//!    exercised on real inputs.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use tome::catalog::manifest::CatalogManifest;
use tome::config::Config;
use tome::error::ManifestInvalid;

fn project_file(rel: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join(rel)
}

fn assert_every_deserialize_has_deny_unknown(path: &str) {
    let contents = fs::read_to_string(project_file(path))
        .unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
    let lines: Vec<&str> = contents.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let l = line.trim();
        // A line that derives `serde::Deserialize`, or `Deserialize` imported
        // from `serde`. Tolerate any number of preceding/following derives.
        let derives_deserialize =
            l.starts_with("#[derive(") && l.contains("Deserialize") && !l.contains("// not-strict");
        if !derives_deserialize {
            continue;
        }
        // The next non-attribute, non-blank line that begins a `pub struct`
        // or `struct` must be preceded by a `#[serde(deny_unknown_fields)]`
        // attribute somewhere in the attribute cluster after `#[derive(...)]`.
        let mut saw_deny_unknown = false;
        for follow in &lines[idx + 1..] {
            let f = follow.trim();
            if f.is_empty() {
                continue;
            }
            if f.starts_with("#[serde(") && f.contains("deny_unknown_fields") {
                saw_deny_unknown = true;
                continue;
            }
            if f.starts_with("#[") {
                continue;
            }
            // First non-attribute line — must be a struct or enum
            // definition. Both can carry `#[serde(deny_unknown_fields)]`;
            // an enum with an unknown variant is the symmetric attack
            // (a future tooling version inserting a new variant that
            // older Tome silently accepts).
            assert!(
                f.starts_with("pub struct")
                    || f.starts_with("struct")
                    || f.starts_with("pub enum")
                    || f.starts_with("enum"),
                "in {}, expected a struct or enum after a #[derive(Deserialize)] cluster but found: {}",
                path,
                f
            );
            assert!(
                saw_deny_unknown,
                "in {}, item `{}` derives Deserialize without #[serde(deny_unknown_fields)]",
                path, f
            );
            break;
        }
    }
}

#[test]
fn manifest_module_structs_all_carry_deny_unknown_fields() {
    assert_every_deserialize_has_deny_unknown("src/catalog/manifest.rs");
}

#[test]
fn config_module_structs_all_carry_deny_unknown_fields() {
    assert_every_deserialize_has_deny_unknown("src/config.rs");
}

/// T219 (P10 deferred coverage): `ModelManifest` lives in
/// `src/embedding/registry.rs` and is a Tome-owned strict input —
/// every deserialise-eligible struct in that module must carry
/// `#[serde(deny_unknown_fields)]` so a stray model-installer field
/// from a future tooling version cannot silently land.
#[test]
fn embedding_registry_structs_all_carry_deny_unknown_fields() {
    assert_every_deserialize_has_deny_unknown("src/embedding/registry.rs");
}

// ---------------------------------------------------------------------------
// Manifest bad-input corpus (FR-022 / FR-023 / SC-005)
// ---------------------------------------------------------------------------

fn write_manifest(text: &str) -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let manifest = root.join("tome-catalog.toml");
    fs::write(&manifest, text).unwrap();
    (temp, root, manifest)
}

fn parse(text: &str) -> Result<CatalogManifest, ManifestInvalid> {
    let (_t, root, manifest) = write_manifest(text);
    CatalogManifest::parse_and_validate(&manifest, &root, text.as_bytes())
}

const GOOD: &str = r#"
name = "x"
description = "y"
version = "0.1.0"

[owner]
name = "n"
email = "n@e.co"
"#;

#[test]
fn good_manifest_parses() {
    parse(GOOD).expect("good manifest");
}

#[test]
fn unknown_top_level_field_rejected() {
    let bad = format!("{}\nextra = \"v\"", GOOD);
    let err = parse(&bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::UnknownField { ref key, .. } if key == "extra"),
        "got: {:?}",
        err
    );
}

#[test]
fn unknown_owner_field_rejected() {
    let bad = format!("{}\n[owner.extra]\nfoo = \"bar\"", GOOD);
    let err = parse(&bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::UnknownField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn unknown_plugin_field_rejected() {
    let bad = format!(
        "{}\n[[plugins]]\nname = \"p\"\nsource = \"./p\"\nextra = \"v\"",
        GOOD
    );
    let err = parse(&bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::UnknownField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn missing_name_rejected() {
    let bad = r#"
description = "y"
version = "0.1.0"

[owner]
name = "n"
email = "n@e.co"
"#;
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { ref key, .. } if key == "name"),
        "got: {:?}",
        err
    );
}

#[test]
fn missing_description_rejected() {
    let bad = r#"
name = "x"
version = "0.1.0"

[owner]
name = "n"
email = "n@e.co"
"#;
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn missing_version_rejected() {
    let bad = r#"
name = "x"
description = "y"

[owner]
name = "n"
email = "n@e.co"
"#;
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn missing_owner_rejected() {
    let bad = r#"
name = "x"
description = "y"
version = "0.1.0"
"#;
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn missing_owner_email_rejected() {
    let bad = r#"
name = "x"
description = "y"
version = "0.1.0"

[owner]
name = "n"
"#;
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn non_semver_version_rejected() {
    let bad = r#"
name = "x"
description = "y"
version = "not-a-version"

[owner]
name = "n"
email = "n@e.co"
"#;
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::InvalidVersion { ref got, .. } if got == "not-a-version"),
        "got: {:?}",
        err
    );
}

#[test]
fn non_email_owner_email_rejected() {
    let bad = r#"
name = "x"
description = "y"
version = "0.1.0"

[owner]
name = "n"
email = "not-an-email"
"#;
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::InvalidEmail { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn missing_plugin_name_rejected() {
    let bad = format!("{}\n[[plugins]]\nsource = \"./p\"", GOOD);
    let err = parse(&bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn missing_plugin_source_rejected() {
    let bad = format!("{}\n[[plugins]]\nname = \"p\"", GOOD);
    let err = parse(&bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::MissingField { .. }),
        "got: {:?}",
        err
    );
}

#[test]
fn duplicate_plugin_name_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    let manifest = root.join("tome-catalog.toml");
    let text = format!(
        "{}\n[[plugins]]\nname = \"dup\"\nsource = \"./a\"\n[[plugins]]\nname = \"dup\"\nsource = \"./b\"\n",
        GOOD
    );
    fs::write(&manifest, &text).unwrap();
    let err = CatalogManifest::parse_and_validate(&manifest, &root, text.as_bytes()).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::DuplicatePluginName { ref name, .. } if name == "dup"),
        "got: {:?}",
        err
    );
}

#[test]
fn malformed_toml_rejected_as_toml_parse() {
    let bad = "this is = = not valid toml [[";
    let err = parse(bad).unwrap_err();
    assert!(
        matches!(err, ManifestInvalid::TomlParse { .. }),
        "got: {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// Config (config.toml) strictness corpus (FR-016)
// ---------------------------------------------------------------------------

#[test]
fn config_unknown_top_level_field_rejected() {
    let toml = r#"
unexpected = "value"
"#;
    let err = toml::from_str::<Config>(toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("unknown"));
}

#[test]
fn config_unknown_field_inside_catalog_entry_rejected() {
    let toml = r#"
[catalogs.foo]
name = "foo"
url = "https://example/"
ref = "main"
path = "/x"
last_synced = "2026-01-01T00:00:00Z"
extra = "nope"
"#;
    let err = toml::from_str::<Config>(toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("unknown"));
}

#[test]
fn config_well_formed_round_trips() {
    let toml = r#"
[catalogs.midnight]
name = "midnight"
url = "https://github.com/midnight/x"
ref = "main"
path = "/x"
last_synced = "2026-01-01T00:00:00Z"
"#;
    let cfg: Config = toml::from_str(toml).expect("parse");
    let back = toml::to_string(&cfg).expect("serialise");
    let cfg2: Config = toml::from_str(&back).expect("re-parse");
    assert_eq!(cfg, cfg2);
}
