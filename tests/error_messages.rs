//! FR-023 audit: every `ManifestInvalid` variant's `Display` output names the
//! file path and (where the variant carries one) the offending value or
//! field key. SC-003 (errors are actionable) is the user-facing form of the
//! same requirement.

use std::path::PathBuf;

use tome::error::{ManifestInvalid, TomeError};

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

// ---- Phase 3 TomeError Display assertions ---------------------------------
// One per new variant, per contracts/exit-codes-p3.md §Display messages.
// We only check that the salient substrings make it into the output —
// thiserror's exact whitespace is not load-bearing.

#[test]
fn mcp_startup_failed_names_reason() {
    let err = TomeError::McpStartupFailed {
        reason: "rmcp handshake rejected".into(),
    };
    let m = err.to_string();
    assert!(m.contains("MCP server failed to start"), "{m}");
    assert!(m.contains("rmcp handshake rejected"), "{m}");
}

#[test]
fn mcp_protocol_io_names_source() {
    let err = TomeError::McpProtocolIo {
        source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe"),
    };
    let m = err.to_string();
    assert!(m.contains("MCP protocol I/O error"), "{m}");
    assert!(m.contains("broken pipe"), "{m}");
}

#[test]
fn workspace_malformed_names_path_reason_and_hint() {
    let err = TomeError::WorkspaceMalformed {
        path: PathBuf::from("/tmp/ws"),
        reason: "invalid TOML in .tome/config.toml at line 4".into(),
    };
    let m = err.to_string();
    assert!(m.contains("/tmp/ws"), "{m}");
    assert!(m.contains("invalid TOML"), "{m}");
    assert!(m.contains("`tome doctor`"), "{m}");
}

#[test]
fn workspace_not_found_names_path_and_init_hint() {
    let err = TomeError::WorkspaceNotFound {
        path: PathBuf::from("/tmp/nope"),
    };
    let m = err.to_string();
    assert!(m.contains("/tmp/nope"), "{m}");
    assert!(m.contains(".tome/"), "{m}");
    assert!(m.contains("tome workspace init"), "{m}");
}

#[test]
fn workspace_conflict_names_both_flags() {
    let err = TomeError::WorkspaceConflict;
    let m = err.to_string();
    assert!(m.contains("--workspace"), "{m}");
    assert!(m.contains("--global"), "{m}");
}

#[test]
fn schema_version_too_new_names_versions_and_hint() {
    let err = TomeError::SchemaVersionTooNew {
        on_disk: 99,
        expected: 1,
    };
    let m = err.to_string();
    assert!(m.contains("v99"), "{m}");
    assert!(m.contains("v1"), "{m}");
    assert!(m.contains("upgrade Tome"), "{m}");
}

#[test]
fn schema_migration_failed_names_versions_and_source() {
    let err = TomeError::SchemaMigrationFailed {
        from: 0,
        to: 1,
        source: anyhow::anyhow!("intermediate page header bad"),
    };
    let m = err.to_string();
    assert!(m.contains("v0"), "{m}");
    assert!(m.contains("v1"), "{m}");
    assert!(m.contains("intermediate page header bad"), "{m}");
    assert!(m.contains("hint"), "{m}");
}

#[test]
fn doctor_fix_not_safe_names_subsystem_and_hint() {
    let err = TomeError::DoctorFixNotSafe {
        subsystem: "catalog_cache".into(),
    };
    let m = err.to_string();
    assert!(m.contains("catalog_cache"), "{m}");
    assert!(m.contains("auto-fix"), "{m}");
    assert!(m.contains("suggested fixes"), "{m}");
}
