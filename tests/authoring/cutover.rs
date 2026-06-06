//! US1 — the plugin manifest cutover. `read_plugin_manifest` reads ONLY the
//! native `tome-plugin.toml`; an unconverted legacy plugin errors with
//! `PluginNotConverted` (80); a plugin carrying both files reads the native
//! one. (FR-001..FR-004, contracts/manifest-cutover.md)

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tome::error::TomeError;
use tome::plugin::read_plugin_manifest;

/// Build a plugin dir under a fresh tempdir, optionally writing the native
/// and/or legacy manifest. Returns `(TempDir guard, plugin_dir)`.
fn plugin_dir(tome: Option<&str>, legacy: Option<&str>) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("plugin-x");
    fs::create_dir_all(&dir).unwrap();
    if let Some(t) = tome {
        fs::write(dir.join("tome-plugin.toml"), t).unwrap();
    }
    if let Some(l) = legacy {
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::write(dir.join(".claude-plugin").join("plugin.json"), l).unwrap();
    }
    (tmp, dir)
}

const LEGACY_JSON: &str = r#"{"name": "plugin-x", "version": "1.0.0"}"#;
const TOME_TOML: &str = "name = \"plugin-x\"\nversion = \"2.0.0\"\n";

#[test]
fn legacy_only_plugin_is_not_converted() {
    let (_g, dir) = plugin_dir(None, Some(LEGACY_JSON));
    let err = read_plugin_manifest(&dir).expect_err("legacy-only must error");
    match &err {
        TomeError::PluginNotConverted { path } => {
            assert_eq!(path, &dir, "error must name the plugin dir");
        }
        other => panic!("expected PluginNotConverted, got {other:?}"),
    }
    // The cutover nudge: exit 80 and a message pointing at `convert`.
    assert_eq!(err.exit_code(), 80);
    assert!(
        err.to_string().contains("convert"),
        "message must hint at conversion: {err}"
    );
}

#[test]
fn tome_manifest_wins_when_both_present() {
    // Both files present, with DIFFERENT versions → the native one is read.
    let (_g, dir) = plugin_dir(Some(TOME_TOML), Some(LEGACY_JSON));
    let m = read_plugin_manifest(&dir).expect("native manifest must load");
    assert_eq!(m.name, "plugin-x");
    assert_eq!(
        m.version, "2.0.0",
        "the native tome-plugin.toml wins over plugin.json"
    );
}

#[test]
fn native_only_plugin_loads() {
    let (_g, dir) = plugin_dir(Some(TOME_TOML), None);
    let m = read_plugin_manifest(&dir).expect("native-only must load");
    assert_eq!(m.version, "2.0.0");
}

#[test]
fn unknown_field_in_native_manifest_is_parse_error() {
    let (_g, dir) = plugin_dir(
        Some("name = \"plugin-x\"\nversion = \"1.0.0\"\nhomepage = \"x\"\n"),
        None,
    );
    let err = read_plugin_manifest(&dir).expect_err("unknown field must error");
    assert!(
        matches!(err, TomeError::PluginManifestParseError { .. }),
        "got {err:?}"
    );
    assert_eq!(err.exit_code(), 22);
}

#[test]
fn no_manifest_at_all_is_parse_error() {
    let (_g, dir) = plugin_dir(None, None);
    let err = read_plugin_manifest(&dir).expect_err("no manifest must error");
    // No legacy plugin.json either → a plain parse error (22), not the
    // conversion nudge (80).
    assert!(
        matches!(err, TomeError::PluginManifestParseError { .. }),
        "got {err:?}"
    );
}

#[test]
fn bad_semver_in_native_manifest_is_parse_error() {
    let (_g, dir) = plugin_dir(Some("name = \"plugin-x\"\nversion = \"nope\"\n"), None);
    let err = read_plugin_manifest(&dir).expect_err("bad semver must error");
    assert!(matches!(err, TomeError::PluginManifestParseError { .. }));
}

/// The conversion nudge must reference the plugin directory by path so a user
/// knows exactly which plugin to convert.
#[test]
fn not_converted_message_names_the_directory() {
    let (_g, dir) = plugin_dir(None, Some(LEGACY_JSON));
    let err = read_plugin_manifest(&dir).unwrap_err();
    let msg = err.to_string();
    let dir_str = Path::new(&dir).display().to_string();
    assert!(
        msg.contains(&dir_str),
        "message `{msg}` must name `{dir_str}`"
    );
}
