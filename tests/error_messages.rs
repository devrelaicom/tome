//! FR-023 audit: every `ManifestInvalid` variant's `Display` output names the
//! file path and (where the variant carries one) the offending value or
//! field key. SC-003 (errors are actionable) is the user-facing form of the
//! same requirement.

use std::path::PathBuf;

use tome::error::ManifestInvalid;

fn dummy_file() -> PathBuf {
    PathBuf::from("/tmp/catalog/tome-catalog.toml")
}

fn dummy_io_err() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, "no such file or directory")
}

#[test]
fn unknown_field_names_file_and_key() {
    let err = ManifestInvalid::UnknownField {
        file: dummy_file(),
        key: "mystery".into(),
        expected_schema_uri: "https://example/schema".into(),
    };
    let m = err.to_string();
    assert!(m.contains("mystery"), "no key in {}", m);
    assert!(
        m.contains("/tmp/catalog/tome-catalog.toml"),
        "no file in {}",
        m
    );
    assert!(
        m.contains("https://example/schema"),
        "no schema URI in {}",
        m
    );
}

#[test]
fn missing_field_names_file_and_key() {
    let err = ManifestInvalid::MissingField {
        file: dummy_file(),
        key: "version".into(),
    };
    let m = err.to_string();
    assert!(m.contains("version"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
}

#[test]
fn invalid_version_names_file_and_value() {
    let err = ManifestInvalid::InvalidVersion {
        file: dummy_file(),
        got: "not-a-version".into(),
    };
    let m = err.to_string();
    assert!(m.contains("not-a-version"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains("semver"));
}

#[test]
fn invalid_email_names_file_and_value() {
    let err = ManifestInvalid::InvalidEmail {
        file: dummy_file(),
        got: "not-an-email".into(),
    };
    let m = err.to_string();
    assert!(m.contains("not-an-email"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains("email"));
}

#[test]
fn duplicate_plugin_name_names_file_and_value() {
    let err = ManifestInvalid::DuplicatePluginName {
        file: dummy_file(),
        name: "dup".into(),
    };
    let m = err.to_string();
    assert!(m.contains("dup"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
}

#[test]
fn source_looks_like_url_names_file_and_value() {
    let err = ManifestInvalid::SourceLooksLikeUrl {
        file: dummy_file(),
        value: "https://example/repo".into(),
    };
    let m = err.to_string();
    assert!(m.contains("https://example/repo"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains("URL"));
}

#[test]
fn source_absolute_names_file_and_value() {
    let err = ManifestInvalid::SourceAbsolute {
        file: dummy_file(),
        value: "/etc/passwd".into(),
    };
    let m = err.to_string();
    assert!(m.contains("/etc/passwd"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains("absolute"));
}

#[test]
fn source_parent_traversal_names_file_and_value() {
    let err = ManifestInvalid::SourceParentTraversal {
        file: dummy_file(),
        value: "../escape".into(),
    };
    let m = err.to_string();
    assert!(m.contains("../escape"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains(".."));
}

#[test]
fn source_escapes_root_names_file_and_value() {
    let err = ManifestInvalid::SourceEscapesRoot {
        file: dummy_file(),
        value: "link".into(),
    };
    let m = err.to_string();
    assert!(m.contains("link"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains("catalog repo"));
}

#[test]
fn source_unresolvable_names_file_value_and_cause() {
    let err = ManifestInvalid::SourceUnresolvable {
        file: dummy_file(),
        value: "missing".into(),
        cause: dummy_io_err(),
    };
    let m = err.to_string();
    assert!(m.contains("missing"));
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains("no such file") || m.contains("not found"));
}

#[test]
fn catalog_root_unresolvable_names_root() {
    let err = ManifestInvalid::CatalogRootUnresolvable {
        root: PathBuf::from("/tmp/missing-root"),
        cause: dummy_io_err(),
    };
    let m = err.to_string();
    assert!(m.contains("/tmp/missing-root"));
}

#[test]
fn toml_parse_names_file_and_message() {
    let err = ManifestInvalid::TomlParse {
        file: dummy_file(),
        message: "expected `=` at line 3".into(),
    };
    let m = err.to_string();
    assert!(m.contains("/tmp/catalog/tome-catalog.toml"));
    assert!(m.contains("expected `=` at line 3"));
}
