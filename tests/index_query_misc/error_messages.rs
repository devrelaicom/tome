//! FR-023 audit: every `ManifestInvalid` variant's `Display` output names the
//! file path and (where the variant carries one) the offending value or
//! field key. SC-003 (errors are actionable) is the user-facing form of the
//! same requirement.

use std::path::PathBuf;

use tome::error::{
    CompositionErrorKind, ManifestInvalid, ShortOrLong, SummariserFailureKind, TomeError,
};
use tome::workspace::ScopeKind;

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
fn workspace_name_invalid_names_input_and_reason() {
    let err = TomeError::WorkspaceNameInvalid {
        name: "bad name".into(),
        reason: "contains invalid character ` ` at position 3".into(),
    };
    let m = err.to_string();
    assert!(m.contains("bad name"), "{m}");
    assert!(m.contains("invalid character"), "{m}");
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

// ---- Phase 4 TomeError Display assertions ---------------------------------
// One per new variant, per contracts/exit-codes-p4.md §Error message style.
// Pre-allocated by F3 — these tests pin the Display surface so production
// wiring in F6/F8/F9 and US1–US5 can't silently drift the user-facing copy.

#[test]
fn workspace_not_found_names_workspace_by_name() {
    let err = TomeError::WorkspaceNotFound {
        name: "shared".into(),
    };
    let m = err.to_string();
    assert!(m.contains("`shared`"), "{m}");
    assert!(m.contains("not found"), "{m}");
    assert!(m.contains("central registry"), "{m}");
}

#[test]
fn workspace_already_exists_names_workspace() {
    let err = TomeError::WorkspaceAlreadyExists {
        name: "shared".into(),
    };
    let m = err.to_string();
    assert!(m.contains("`shared`"), "{m}");
    assert!(m.contains("already exists"), "{m}");
}

#[test]
fn workspace_name_invalid_names_value_and_reason() {
    let err = TomeError::WorkspaceNameInvalid {
        name: "-bad".into(),
        reason: "leading hyphen".into(),
    };
    let m = err.to_string();
    assert!(m.contains("`-bad`"), "{m}");
    assert!(m.contains("leading hyphen"), "{m}");
    assert!(m.contains("invalid"), "{m}");
}

#[test]
fn workspace_has_bound_projects_names_count_and_projects() {
    let err = TomeError::WorkspaceHasBoundProjects {
        name: "shared".into(),
        count: 2,
        projects: vec!["/tmp/a".into(), "/tmp/b".into()],
    };
    let m = err.to_string();
    assert!(m.contains("`shared`"), "{m}");
    assert!(m.contains("2"), "{m}");
    assert!(m.contains("/tmp/a"), "{m}");
    assert!(m.contains("/tmp/b"), "{m}");
    assert!(m.contains("--force"), "{m}");
}

#[test]
fn composition_error_cycle_names_chain() {
    let err = TomeError::CompositionError {
        kind: CompositionErrorKind::Cycle {
            path: vec!["project".into(), "shared".into(), "shared".into()],
        },
    };
    let m = err.to_string();
    assert!(m.contains("composition"), "{m}");
    assert!(m.contains("cycle"), "{m}");
    assert!(m.contains("project"), "{m}");
    assert!(m.contains("shared"), "{m}");
}

#[test]
fn composition_error_workspace_ref_outside_project_names_scope() {
    let err = TomeError::CompositionError {
        kind: CompositionErrorKind::WorkspaceRefOutsideProject {
            found_in: ScopeKind::Global,
        },
    };
    let m = err.to_string();
    assert!(m.contains("composition"), "{m}");
    assert!(m.contains("Global"), "{m}");
}

#[test]
fn composition_error_unknown_workspace_names_workspace() {
    let err = TomeError::CompositionError {
        kind: CompositionErrorKind::UnknownWorkspace("missing".into()),
    };
    let m = err.to_string();
    assert!(m.contains("composition"), "{m}");
    assert!(m.contains("`missing`"), "{m}");
}

#[test]
fn composition_error_bad_exclusion_names_token() {
    let err = TomeError::CompositionError {
        kind: CompositionErrorKind::BadExclusion("!../bad".into()),
    };
    let m = err.to_string();
    assert!(m.contains("composition"), "{m}");
    assert!(m.contains("!../bad"), "{m}");
}

#[test]
fn harness_not_supported_names_harness() {
    let err = TomeError::HarnessNotSupported {
        name: "made-up-harness".into(),
    };
    let m = err.to_string();
    assert!(m.contains("`made-up-harness`"), "{m}");
    assert!(m.contains("not supported"), "{m}");
}

#[test]
fn harness_clash_names_path_and_command_shape() {
    let err = TomeError::HarnessClash {
        path: PathBuf::from("/tmp/.cursor/mcp.json"),
        command: "node".into(),
        first_arg: "/opt/custom-tome.js".into(),
    };
    let m = err.to_string();
    assert!(m.contains("/tmp/.cursor/mcp.json"), "{m}");
    assert!(m.contains("`node`"), "{m}");
    assert!(m.contains("/opt/custom-tome.js"), "{m}");
    assert!(m.contains("--force"), "{m}");
}

#[test]
fn summariser_failure_model_missing_names_subclass() {
    let err = TomeError::SummariserFailure {
        kind: SummariserFailureKind::ModelMissing,
    };
    let m = err.to_string();
    assert!(m.contains("summariser failure"), "{m}");
    assert!(m.contains("model missing"), "{m}");
}

#[test]
fn summariser_failure_checksum_mismatch_names_hashes() {
    let err = TomeError::SummariserFailure {
        kind: SummariserFailureKind::ModelChecksumMismatch {
            expected: "aa".into(),
            observed: "bb".into(),
        },
    };
    let m = err.to_string();
    assert!(m.contains("summariser failure"), "{m}");
    assert!(m.contains("aa"), "{m}");
    assert!(m.contains("bb"), "{m}");
}

#[test]
fn summariser_failure_backend_init_names_source() {
    let err = TomeError::SummariserFailure {
        kind: SummariserFailureKind::BackendInitFailed {
            source: "llama-cpp-2 returned -1".into(),
        },
    };
    let m = err.to_string();
    assert!(m.contains("summariser failure"), "{m}");
    assert!(m.contains("backend init"), "{m}");
    assert!(m.contains("llama-cpp-2 returned -1"), "{m}");
}

#[test]
fn summariser_failure_output_unparsable_names_which() {
    let err = TomeError::SummariserFailure {
        kind: SummariserFailureKind::OutputUnparsable {
            which: ShortOrLong::Short,
        },
    };
    let m = err.to_string();
    assert!(m.contains("summariser failure"), "{m}");
    assert!(m.contains("unparsable"), "{m}");
    assert!(m.contains("short"), "{m}");
}

#[test]
fn summariser_failure_output_empty_names_which() {
    let err = TomeError::SummariserFailure {
        kind: SummariserFailureKind::OutputEmpty {
            which: ShortOrLong::Long,
        },
    };
    let m = err.to_string();
    assert!(m.contains("summariser failure"), "{m}");
    assert!(m.contains("empty"), "{m}");
    assert!(m.contains("long"), "{m}");
}

// ---- #281 first-run recovery `hint:` lines --------------------------------
// CatalogNotFound / PluginNotFound / the invalid-plugin-id format error now
// carry a recovery `hint:` in their Display, mirroring WorkspaceNotFound's
// existing hint. We assert the salient substrings (not exact whitespace) and
// that every referenced command is a real CLI surface.

#[test]
fn catalog_not_found_names_catalog_and_recovery_hint() {
    let err = TomeError::CatalogNotFound("ghost".into());
    let m = err.to_string();
    assert!(m.contains("`ghost`"), "{m}");
    assert!(m.contains("not registered"), "{m}");
    assert!(m.contains("hint:"), "{m}");
    assert!(m.contains("tome catalog list"), "{m}");
    assert!(m.contains("tome catalog add"), "{m}");
}

#[test]
fn plugin_not_found_names_plugin_and_recovery_hint() {
    let err = TomeError::PluginNotFound("acme/widgets".into());
    let m = err.to_string();
    assert!(m.contains("`acme/widgets`"), "{m}");
    assert!(m.contains("not installed"), "{m}");
    assert!(m.contains("hint:"), "{m}");
    assert!(m.contains("tome plugin list"), "{m}");
    // #311: the invalid-id hint also surfaces the interactive browser so a
    // newcomer who hits this error learns bare `tome plugin` exists.
    assert!(m.contains("`tome plugin`"), "{m}");
    // The whole recovery lives on a single `hint:` continuation line (#310
    // dims exactly one such line).
    assert_eq!(m.matches("\nhint:").count(), 1, "{m}");
}

#[test]
fn invalid_plugin_id_format_states_expected_shape_hint() {
    use tome::plugin::identity::PluginIdParseError;
    let err = PluginIdParseError::Format("no-slash".into());
    let m = err.to_string();
    assert!(m.contains("`no-slash`"), "{m}");
    assert!(m.contains("<catalog>/<plugin>"), "{m}");
    assert!(m.contains("hint:"), "{m}");
    assert!(m.contains("tome plugin list"), "{m}");
    // #311: also point at the interactive browser.
    assert!(m.contains("`tome plugin`"), "{m}");
    assert_eq!(m.matches("\nhint:").count(), 1, "{m}");
}
