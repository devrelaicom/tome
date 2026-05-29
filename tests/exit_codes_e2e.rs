//! Phase 10 / T195 — end-to-end CLI exit-code coverage for Phase 2 codes.
//!
//! `tests/exit_codes.rs` is unit-level: it constructs each `TomeError` and
//! confirms the `exit_code()` mapping. This file verifies that the CLI
//! binary actually emits the documented exit code under real
//! reproduction conditions — i.e. that the failure path is reachable
//! from a `tome <subcommand>` invocation and propagates correctly.
//!
//! ## Coverage matrix
//!
//! Codes reachable cheaply (no embedder load, no network, no fake ONNX):
//!
//! | Code | Variant                       | Tested via                            |
//! |------|-------------------------------|---------------------------------------|
//! | 22   | PluginManifestParseError      | `tome plugin show` with bad plugin.json |
//! | 33   | ModelRegistrationParseError   | `tome models list` with bad manifest.json |
//! | 50   | IndexBusy                     | covered in `tests/concurrency.rs`     |
//! | 51   | IndexIntegrityCheckFailure    | `tome status` against a corrupt DB    |
//! | 52   | SchemaTooNew                  | covered in `tests/schema_migrations.rs` |
//! | 53   | CatalogHasEnabledPlugins      | covered in `tests/catalog_remove_cascade.rs` |
//! | 54   | NotATerminal                  | covered in `tests/plugin_disable.rs` etc. |
//!
//! Phase 4 US1.d-1 / T164 additions (CLI binary coverage for the
//! `tome workspace use` surface — bind/sync doesn't load ONNX):
//!
//! | Code | Variant                       | Tested via                            |
//! |------|-------------------------------|---------------------------------------|
//! | 2    | Usage (cwd is $HOME)          | `workspace_use_cwd_is_home_exits_2`   |
//! | 13   | WorkspaceNotFound             | `workspace_use_missing_workspace_exits_13` |
//! | 19   | HarnessClash                  | `workspace_use_harness_clash_exits_19_without_force` |
//!
//! Phase 4 US4.c / T338 addition (CLI binary coverage for the
//! `tome workspace regen-summary` summariser-missing path):
//!
//! | Code | Variant                                       | Tested via                                            |
//! |------|-----------------------------------------------|-------------------------------------------------------|
//! | 24   | SummariserFailure { kind: ModelMissing }      | `workspace_regen_summary_with_missing_model_exits_24` |
//!
//! Phase 4 / Polish PR-D / T-M7 additions (CLI binary coverage for
//! Phase 4 codes that were library-only-covered until this PR):
//!
//! | Code | Variant                       | Tested via                                                |
//! |------|-------------------------------|-----------------------------------------------------------|
//! | 14   | WorkspaceAlreadyExists        | `workspace_init_duplicate_exits_14`                       |
//! | 16   | WorkspaceHasBoundProjects     | `workspace_remove_with_bound_projects_exits_16`           |
//! | 17   | CompositionError              | `harness_list_with_composition_cycle_exits_17`            |
//! | 18   | HarnessNotSupported           | `harness_list_with_unsupported_harness_exits_18`          |
//! | 70   | WorkspaceMalformed            | `workspace_info_with_malformed_global_settings_exits_70`  |
//! | 7    | Io                            | `workspace_init_with_unwritable_parent_dir_exits_7` (unix)|
//!
//! Codes reachable only through embedder/inference paths (deferred to
//! library-level tests for CI cost reasons — running these end-to-end
//! requires loading `FastembedEmbedder` which pulls ~345 MB of ONNX):
//!
//! | Code | Variant                       | Library-level coverage                |
//! |------|-------------------------------|---------------------------------------|
//! | 23   | SkillFrontmatterParseError    | `tests/frontmatter.rs` (parser) + `lifecycle` propagation |
//! | 30   | ModelMissing                  | `tests/plugin_enable.rs` (lifecycle path) + `tests/models_remove.rs` (CLI) |
//! | 31   | ModelCorrupt                  | `tests/model_download.rs::placeholder_checksum_refused` |
//! | 32   | ModelChecksumMismatch         | `tests/model_download.rs::checksum_mismatch_returns_tight_error` |
//! | 34   | InferenceRuntimeInitFailure   | exit-code unit + manual repro (missing libonnxruntime) |
//! | 35   | VectorExtensionInitFailure    | exit-code unit + manual repro (broken vec extension) |
//! | 36   | EmbeddingGenerationFailure    | `tests/atomicity_enable.rs` (StubEmbedder force-fail) |
//! | 37   | RerankingFailure              | exit-code unit + manual repro (StubReranker injected) |
//! | 40   | QueryNoResultsStrict          | requires query handler library entry point (Phase 10 follow-up) |
//! | 41   | EmbedderNameDrift             | `tests/status.rs::report_flags_embedder_drift_*` (lib via meta mutation) |
//! | 42   | EmbedderVersionDrift          | `tests/status.rs::report_flags_embedder_drift_*` (lib via meta mutation) |
//!
//! The "deferred" codes are documented here rather than hidden so the
//! pre-flight against `contracts/exit-codes.md` is auditable in one
//! place. Phase 11 / post-v0.2.0 can revisit any of them as the
//! library-API surface for `tome query` matures.
//!
//! Phase 6 / US2 addition (CLI binary coverage for the hooks sink — bind +
//! sync against a malformed `hooks/hooks.json`):
//!
//! | Code | Variant                       | Tested via                                   |
//! |------|-------------------------------|----------------------------------------------|
//! | 43   | HookSpecParseError            | `workspace_use_malformed_hooks_exits_43`     |
//!
//! Code 44 (`HookSettingsWriteFailed`) stays library-API-only — an IO
//! failure on `.claude/settings.local.json` is not cheaply forced through
//! the binary; the merge/remove tests in `tests/hooks_merge.rs` and the
//! exit-code unit in `tests/exit_codes.rs` cover it, per the established
//! e2e split (cf. codes 44/46 in `contracts/exit-codes-p6.md` §"Discipline").
//!
//! Phase 6 / US3 addition (guardrails sink). Code 46
//! (`GuardrailsWriteFailed`) stays library-API-only, per the same split — an
//! IO/render failure on a rules-file target or the Cursor sibling is not
//! cheaply forced through the binary. It is covered here via the
//! `guardrails`-module symlink-refusal path (a symlinked in-file target
//! surfaces exit 46) and by the exit-code unit in `tests/exit_codes.rs`:
//!
//! | Code | Variant                       | Tested via                                  |
//! |------|-------------------------------|---------------------------------------------|
//! | 46   | GuardrailsWriteFailed         | `guardrails_write_through_symlink_exits_46` (lib, unix) |

mod common;

use std::fs;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_all_registry_models,
    paths_for, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};
use tempfile::TempDir;
use tome::embedding::registry::MODEL_REGISTRY;
use tome::index::{OpenOptions, open};

fn options() -> OpenOptions {
    OpenOptions {
        embedder: stub_embedder_seed(),
        reranker: stub_reranker_seed(),
        summariser: stub_summariser_seed(),
    }
}

#[test]
fn plugin_show_with_malformed_plugin_json_exits_22() {
    // Setup: copy the sample-plugin-catalog fixture (which has real
    // plugin.json files), register it via a hand-written config, then
    // corrupt plugin-alpha's plugin.json. `tome plugin show` must
    // surface the parse error as exit 22 (`PluginManifestParseError`).
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    let pj = catalog_root
        .join("plugin-alpha")
        .join(".claude-plugin")
        .join("plugin.json");
    assert!(
        pj.is_file(),
        "expected fixture plugin.json at {}",
        pj.display()
    );
    fs::write(&pj, b"{ this is not valid json }").expect("write bad plugin.json");

    let out = env
        .cmd()
        .args(["plugin", "show", "sample-plugin-catalog/plugin-alpha"])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(22),
        "expected exit 22 PluginManifestParseError, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn models_list_with_corrupt_manifest_exits_33() {
    // Setup: stage a fabricated model directory, then corrupt its
    // manifest.json. `tome models list` reads the manifest and surfaces
    // a parse failure as exit 33 (`ModelRegistrationParseError`).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");
    fabricate_all_registry_models(&paths);

    let entry = MODEL_REGISTRY.first().expect("at least one model");
    let manifest_path = paths.models_dir.join(entry.name).join("manifest.json");
    fs::write(&manifest_path, b"{ broken json").expect("corrupt manifest");

    let out = env.cmd().args(["models", "list"]).output().expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(33),
        "expected exit 33 ModelRegistrationParseError, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn status_against_corrupt_index_exits_51() {
    // Setup: bootstrap a real index, then overwrite its file with random
    // bytes (header smashed). `tome status` opens the index for the
    // integrity check; the SQLite open or pragma_update should fail with
    // a controlled `IndexIntegrityCheckFailure` — exit 51.
    let _tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    {
        // Bootstrap so the path exists and is a real DB.
        let _ = open(&paths.index_db, &options()).expect("bootstrap");
    }
    // Stomp the file header. SQLite expects a magic number at offset 0;
    // overwriting the first 100 bytes is a reliable corruption.
    fs::write(&paths.index_db, b"not a sqlite database\n").expect("corrupt DB");

    let out = env.cmd().args(["status"]).output().expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(51),
        "expected exit 51 IndexIntegrityCheckFailure, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn unknown_subcommand_exits_2() {
    // Sanity check that the Phase 1 usage path still surfaces correctly
    // through the CLI binary. (Also exercised by individual command
    // tests, but a one-shot here makes the E2E coverage matrix complete.)
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["definitely-not-a-real-subcommand"])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 Usage, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// Phase 4 / US1.d-1 / T164 — `tome workspace use` exit-code surface via
// the CLI binary. The bind+sync path doesn't load real ONNX models (it
// only opens the index DB with seed metadata), so these CLI tests stay
// cheap.
// ---------------------------------------------------------------------------

#[test]
fn workspace_use_missing_workspace_exits_13() {
    // Run `tome workspace use missing-ws` from a project subdir inside
    // the isolated HOME. The workspace isn't seeded → exit 13
    // (`WorkspaceNotFound`).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let project = env.home_path().join("project");
    fs::create_dir_all(&project).expect("create project");

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["workspace", "use", "missing-ws"])
        .output()
        .expect("spawn");

    assert_eq!(
        out.status.code(),
        Some(13),
        "expected exit 13 WorkspaceNotFound, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn workspace_use_cwd_is_home_exits_2() {
    // Running `tome workspace use` from $HOME (without --force) must
    // refuse with exit 2.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // Seed the workspace so the failure is *only* the dangerous-cwd
    // check, not a missing workspace.
    {
        use common::seed_workspace;
        seed_workspace(&paths, "test-ws");
    }

    let out = env
        .cmd()
        .current_dir(env.home_path())
        .args(["workspace", "use", "test-ws"])
        .output()
        .expect("spawn");

    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 Usage, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn workspace_use_invalid_name_exits_15() {
    // `WorkspaceName::parse` rejects names containing characters outside
    // `[a-z0-9-]` per FR-347. The CLI must surface that as exit 15
    // (`WorkspaceNameInvalid`) rather than collapsing into a generic
    // usage error.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let project = env.home_path().join("project");
    fs::create_dir_all(&project).expect("create project");

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["workspace", "use", "Bad_Name!"])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(15),
        "expected exit 15 WorkspaceNameInvalid, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn workspace_use_harness_clash_exits_19_without_force() {
    // Pre-populate `.claude/settings.json` with a user-owned `tome`
    // entry; configure the global settings to declare `claude-code` as
    // the only effective harness. The bind step writes the marker + row,
    // then the sync step hits the clash and exits 19.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    {
        use common::seed_workspace;
        seed_workspace(&paths, "test-ws");
    }

    // Global settings declare claude-code as the only effective harness.
    fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .expect("write global settings");

    let project = env.home_path().join("project");
    fs::create_dir_all(&project).expect("create project");

    // User-owned `tome` entry in .claude/settings.json.
    let claude_dir = project.join(".claude");
    fs::create_dir_all(&claude_dir).expect("create .claude");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&conflict).unwrap(),
    )
    .expect("write conflict");

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["workspace", "use", "test-ws"])
        .output()
        .expect("spawn");

    assert_eq!(
        out.status.code(),
        Some(19),
        "expected exit 19 HarnessClash, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn workspace_regen_summary_with_missing_model_exits_24() {
    // Phase 4 / US4.c / T338 — `tome workspace regen-summary <name>`
    // must surface `SummariserFailure { kind: ModelMissing }` from
    // `LlamaSummariser::new` (no GGUF on disk → exit 24).
    //
    // Note on the exit-code number: tasks.md / `contracts/summariser.md`
    // refer to "exit 20" historically, but the closed-set
    // `TomeError::exit_code()` lands `SummariserFailure { .. }` at 24
    // per the F3 collision-avoidance fix — 20 is owned by
    // `PluginNotFound`. The data-model file documents this.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // The workspace must exist or regen-summary exits 13 (`WorkspaceNotFound`)
    // before reaching the summariser. Seed directly via the helper so we
    // don't depend on `tome workspace init` (which would also work but
    // is a heavier-weight invocation that boots the index machinery
    // twice).
    common::seed_workspace(&paths, "test-ws");

    // ToolEnv's $HOME is a fresh tempdir → models_dir is empty. The
    // summariser GGUF is therefore absent; `LlamaSummariser::new`
    // returns `ModelMissing` → exit 24.
    let out = env
        .cmd()
        .args(["workspace", "regen-summary", "test-ws"])
        .output()
        .expect("spawn regen-summary");
    assert_eq!(
        out.status.code(),
        Some(24),
        "expected exit 24 SummariserFailure {{ ModelMissing }}, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// Phase 4 / US5.c-1 / T-M1 — `tome doctor --fix` exit-code surface via
// the CLI binary, specifically the user-owned-MCP case (exit 75 without
// `--force`, exit 0 with `--fix --force`).
//
// These tests seed the central DB with the production registry seeds
// (not stub seeds) so the doctor flow doesn't surface embedder drift as
// an additional non-auto-fixable suggestion that would inflate the
// residual fix list and confound the exit-code assertion.
// ---------------------------------------------------------------------------

/// Seed a workspace row using the production MODEL_REGISTRY seeds, so
/// `tome doctor` (which opens with the registry seeds) doesn't report
/// embedder drift. Returns once the workspace + DB exist.
fn seed_workspace_with_registry_seeds(paths: &tome::paths::Paths, name: &str) {
    let (embedder, reranker, summariser) = tome::commands::plugin::registry_seeds();
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder,
            reranker,
            summariser,
        },
    )
    .expect("open index for seeding workspace");
    let now_unix = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspaces (name, created_at, last_used_at) VALUES (?1, ?2, ?2)",
        rusqlite::params![name, now_unix],
    )
    .expect("seed workspace row");
}

#[test]
fn doctor_fix_user_owned_mcp_exits_75() {
    // Pre-condition: a project marker bound to `test-ws`, the global
    // settings declare `claude-code` as the only effective harness, and
    // `.claude/settings.json` carries a user-owned `tome` entry. Running
    // `tome doctor --fix` (without --force) MUST leave the user-owned
    // entry alone and surface exit 75 — work was attempted but a manual
    // fix remains.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");
    fabricate_all_registry_models(&paths);
    seed_workspace_with_registry_seeds(&paths, "test-ws");

    // Global settings declare claude-code.
    fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .expect("write global settings");

    let project = env.home_path().join("project");
    fs::create_dir_all(project.join(".tome")).expect("create project marker dir");
    fs::write(
        project.join(".tome/config.toml"),
        "workspace = \"test-ws\"\n",
    )
    .expect("write project marker");

    // Insert a workspace_projects row so the project is bound in the
    // central DB (otherwise the bound-workspace-exists check works but
    // the rules-copy sync would have nothing to walk).
    {
        let conn = tome::index::open_read_only(&paths.index_db).expect("open ro");
        let ws_id: i64 = conn
            .query_row(
                "SELECT id FROM workspaces WHERE name = ?1",
                rusqlite::params!["test-ws"],
                |r| r.get(0),
            )
            .expect("workspace id");
        drop(conn);
        // Re-open for write.
        let conn = tome::index::open_read_only(&paths.index_db).expect("open ro 2");
        drop(conn);
        // Use raw rusqlite to insert without going through the schema
        // bootstrap a second time.
        let conn = rusqlite::Connection::open(&paths.index_db).expect("rusqlite open");
        conn.execute(
            "INSERT INTO workspace_projects (workspace_id, project_path, bound_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![
                ws_id,
                project.to_str().unwrap(),
                time::OffsetDateTime::now_utc().unix_timestamp(),
            ],
        )
        .expect("insert workspace_projects row");
    }

    // Pre-populate a user-owned `tome` entry in .claude/settings.json.
    let claude_dir = project.join(".claude");
    fs::create_dir_all(&claude_dir).expect("create .claude");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&conflict).unwrap(),
    )
    .expect("write conflict");

    // Run `tome doctor --fix` from inside the project directory so the
    // scope resolves via project marker walk.
    let out = env
        .cmd()
        .current_dir(&project)
        .args(["doctor", "--fix"])
        .output()
        .expect("spawn doctor --fix");
    assert_eq!(
        out.status.code(),
        Some(75),
        "expected exit 75 DoctorFixNotSafe (user-owned MCP), got {:?}, \
         stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // The user-owned entry MUST survive a non-forced --fix.
    let after = fs::read_to_string(claude_dir.join("settings.json")).expect("read settings.json");
    assert!(
        after.contains("\"evil\""),
        "user-owned `evil` command must survive non-forced --fix; got: {after}",
    );
}

#[test]
fn doctor_fix_force_user_owned_mcp_exits_0() {
    // Same setup as above, but invoke `tome doctor --fix --force`. The
    // override rewrites the user-owned entry to the Tome-owned shape
    // and exits 0.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");
    fabricate_all_registry_models(&paths);
    seed_workspace_with_registry_seeds(&paths, "test-ws");

    fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .expect("write global settings");

    let project = env.home_path().join("project");
    fs::create_dir_all(project.join(".tome")).expect("create project marker dir");
    fs::write(
        project.join(".tome/config.toml"),
        "workspace = \"test-ws\"\n",
    )
    .expect("write project marker");
    // Pre-create the project's RULES.md AND the workspace's RULES.md so
    // the binding-rules-copy check passes; the only outstanding fix is
    // the user-owned MCP one.
    let ws = tome::workspace::WorkspaceName::parse("test-ws").unwrap();
    let src = paths.workspace_rules_file(&ws);
    fs::create_dir_all(src.parent().unwrap()).expect("create workspace dir");
    fs::write(&src, b"canonical\n").expect("write workspace RULES.md");
    fs::write(project.join(".tome/RULES.md"), b"canonical\n").expect("write project RULES.md");

    {
        let conn = tome::index::open_read_only(&paths.index_db).expect("open ro");
        let ws_id: i64 = conn
            .query_row(
                "SELECT id FROM workspaces WHERE name = ?1",
                rusqlite::params!["test-ws"],
                |r| r.get(0),
            )
            .expect("workspace id");
        drop(conn);
        let conn = rusqlite::Connection::open(&paths.index_db).expect("rusqlite open");
        conn.execute(
            "INSERT INTO workspace_projects (workspace_id, project_path, bound_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![
                ws_id,
                project.to_str().unwrap(),
                time::OffsetDateTime::now_utc().unix_timestamp(),
            ],
        )
        .expect("insert workspace_projects row");
    }

    let claude_dir = project.join(".claude");
    fs::create_dir_all(&claude_dir).expect("create .claude");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&conflict).unwrap(),
    )
    .expect("write conflict");

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["doctor", "--fix", "--force"])
        .output()
        .expect("spawn doctor --fix --force");
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected exit 0 after --fix --force rewrite, got {:?}, \
         stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let after = fs::read_to_string(claude_dir.join("settings.json")).expect("read settings.json");
    assert!(
        !after.contains("\"evil\""),
        "user-owned `evil` command must be replaced; got: {after}",
    );
    assert!(
        after.contains("\"tome\""),
        "rewrite must install the Tome-owned `command = tome`; got: {after}",
    );
}

#[test]
fn doctor_force_without_fix_exits_2() {
    // R-M1: `--force` without `--fix` is a usage error (exit 2), not an
    // Io error (exit 7).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let out = env
        .cmd()
        .args(["doctor", "--force"])
        .output()
        .expect("spawn doctor --force");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 Usage, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn reindex_unknown_plugin_in_known_catalog_exits_20() {
    // Filling in the gap flagged by `review/contract-audit.md` §reindex
    // minor #2: the `Reindex unknown plugin` → exit 20 path was not
    // covered. We register a catalog (no plugins enabled), then ask to
    // reindex a non-existent plugin scope.
    let _tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let tmpdir = TempDir::new().unwrap();
    let catalog_root = copy_sample_plugin_catalog(&tmpdir, "sample-plugin-catalog");
    let cfg = format!(
        "[catalogs.sample-plugin-catalog]\n\
         name = \"sample-plugin-catalog\"\n\
         url = \"file://{path}\"\n\
         ref = \"main\"\n\
         path = \"{path}\"\n\
         last_synced = \"2026-05-13T00:00:00Z\"\n",
        path = catalog_root.display(),
    );
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("config dir");
    fs::write(&paths.global_config_file, cfg).expect("write config");

    let out = env
        .cmd()
        .args(["reindex", "sample-plugin-catalog/no-such-plugin"])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(20),
        "expected exit 20 PluginNotFound, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// Phase 4 / Polish PR-D / T-M7 — CLI binary coverage for Phase 4 codes
// that were library-only-covered until this PR. Each test exercises the
// failure path through `tome <subcommand>` and asserts the documented
// exit code from `contracts/exit-codes-p4.md`.
// ---------------------------------------------------------------------------

#[test]
fn workspace_init_duplicate_exits_14() {
    // `tome workspace init foo` twice — second invocation hits
    // `WorkspaceAlreadyExists` (exit 14).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // First init must succeed.
    let first = env
        .cmd()
        .args(["workspace", "init", "foo"])
        .output()
        .expect("spawn first init");
    assert_eq!(
        first.status.code(),
        Some(0),
        "first init must succeed, got {:?}, stderr:\n{}",
        first.status.code(),
        String::from_utf8_lossy(&first.stderr),
    );

    // Second init with the same name must exit 14.
    let second = env
        .cmd()
        .args(["workspace", "init", "foo"])
        .output()
        .expect("spawn duplicate init");
    assert_eq!(
        second.status.code(),
        Some(14),
        "expected exit 14 WorkspaceAlreadyExists, got {:?}, stderr:\n{}",
        second.status.code(),
        String::from_utf8_lossy(&second.stderr),
    );
}

#[test]
fn workspace_remove_with_bound_projects_exits_16() {
    // Init `foo`, bind a project to it, then `workspace remove foo`
    // without `--force` → exit 16 (`WorkspaceHasBoundProjects`).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let init_out = env
        .cmd()
        .args(["workspace", "init", "foo"])
        .output()
        .expect("spawn init");
    assert!(
        init_out.status.success(),
        "init must succeed, stderr:\n{}",
        String::from_utf8_lossy(&init_out.stderr),
    );

    // Bind a project under HOME (avoids the cwd-is-home refusal).
    let project = env.home_path().join("proj");
    fs::create_dir_all(&project).expect("create project");
    let bind_out = env
        .cmd()
        .current_dir(&project)
        .args(["workspace", "use", "foo"])
        .output()
        .expect("spawn use");
    assert!(
        bind_out.status.success(),
        "bind must succeed, exit={:?} stderr:\n{}",
        bind_out.status.code(),
        String::from_utf8_lossy(&bind_out.stderr),
    );

    let remove_out = env
        .cmd()
        .args(["workspace", "remove", "foo"])
        .output()
        .expect("spawn remove");
    assert_eq!(
        remove_out.status.code(),
        Some(16),
        "expected exit 16 WorkspaceHasBoundProjects, got {:?}, stderr:\n{}",
        remove_out.status.code(),
        String::from_utf8_lossy(&remove_out.stderr),
    );
}

#[test]
fn harness_list_with_unsupported_harness_exits_18() {
    // Write global settings with an unsupported harness name; running
    // `tome harness list` triggers composition resolution and surfaces
    // `HarnessNotSupported` (exit 18).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    fs::write(&paths.global_settings_file, "harnesses = [\"bogus\"]\n")
        .expect("write global settings");

    let out = env
        .cmd()
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list");
    assert_eq!(
        out.status.code(),
        Some(18),
        "expected exit 18 HarnessNotSupported, got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn harness_list_with_composition_cycle_exits_17() {
    // Seed two workspaces (a, b), then write settings.toml files that
    // form a composition cycle: a → [workspaces.b], b → [workspaces.a].
    // Resolving the project's effective list (which pulls in workspace
    // `a` via the project marker) must hit the cycle and exit 17.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // Init both workspaces via the CLI so the central DB rows are
    // populated and the workspace dirs exist.
    for ws in ["a", "b"] {
        let out = env
            .cmd()
            .args(["workspace", "init", ws])
            .output()
            .expect("spawn init");
        assert!(
            out.status.success(),
            "init {ws} stderr:\n{}",
            String::from_utf8_lossy(&out.stderr),
        );
    }

    // Write the cyclic settings. Workspace settings require a `name`
    // field at the top level per `WorkspaceSettings::deny_unknown_fields`;
    // omitting it would surface as exit 70 (`WorkspaceMalformed`) before
    // the cycle is reached.
    let ws_a = tome::workspace::WorkspaceName::parse("a").unwrap();
    let ws_b = tome::workspace::WorkspaceName::parse("b").unwrap();
    fs::write(
        paths.workspace_settings_file(&ws_a),
        "name = \"a\"\nharnesses = [\"[workspaces.b]\"]\n",
    )
    .expect("write a settings");
    fs::write(
        paths.workspace_settings_file(&ws_b),
        "name = \"b\"\nharnesses = [\"[workspaces.a]\"]\n",
    )
    .expect("write b settings");

    // Project bound to workspace `a` triggers the cycle on resolution.
    let project = env.home_path().join("cyclic-project");
    fs::create_dir_all(project.join(".tome")).expect("create marker dir");
    fs::write(project.join(".tome/config.toml"), "workspace = \"a\"\n")
        .expect("write project marker");

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list");
    assert_eq!(
        out.status.code(),
        Some(17),
        "expected exit 17 CompositionError (cycle), got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn workspace_info_with_malformed_global_settings_exits_70() {
    // Write malformed TOML in the global settings file, then call any
    // command that parses it. `tome harness list` calls
    // `parse_global` → `WorkspaceMalformed` (exit 70).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    fs::write(
        &paths.global_settings_file,
        "this is = not = valid = toml\n",
    )
    .expect("write malformed settings");

    let out = env
        .cmd()
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list");
    assert_eq!(
        out.status.code(),
        Some(70),
        "expected exit 70 WorkspaceMalformed, got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[cfg(unix)]
#[test]
fn workspace_init_with_unwritable_parent_dir_exits_7() {
    // chmod 0o500 the workspaces parent dir (read+execute, no write),
    // then attempt `tome workspace init foo`. The init code tries to
    // create the workspace subdirectory and fails with a write
    // permission denied — `Io` (exit 7).
    use std::os::unix::fs::PermissionsExt;

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");
    // Ensure the workspaces parent dir exists and is empty, then chmod
    // it so the init's mkdir fails. (The dir is `<root>/workspaces`.)
    let workspaces_dir = paths.root.join("workspaces");
    fs::create_dir_all(&workspaces_dir).expect("create workspaces dir");
    let mut perms = fs::metadata(&workspaces_dir).unwrap().permissions();
    perms.set_mode(0o500);
    fs::set_permissions(&workspaces_dir, perms).expect("chmod workspaces dir");

    let out = env
        .cmd()
        .args(["workspace", "init", "foo"])
        .output()
        .expect("spawn init");

    // Restore permissions before asserting so a test failure doesn't
    // leak a chmod'd dir into the TempDir cleanup.
    let mut restore = fs::metadata(&workspaces_dir).unwrap().permissions();
    restore.set_mode(0o700);
    let _ = fs::set_permissions(&workspaces_dir, restore);

    assert_eq!(
        out.status.code(),
        Some(7),
        "expected exit 7 Io, got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// Phase 6 / US2 / T071: a malformed plugin `hooks/hooks.json` surfaces exit 43
// through the binary. `tome workspace use` binds the project and runs sync;
// the hooks pass reads the malformed source for an enabled plugin and fails
// with `HookSpecParseError` (43). The committed settings.json is never written.
// ---------------------------------------------------------------------------

#[test]
fn workspace_use_malformed_hooks_exits_43() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");
    seed_workspace_with_registry_seeds(&paths, "test-ws");

    // claude-code is the only effective harness (the sole RealJson harness).
    fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .expect("write global settings");

    // Seed a catalog enrolment + an enabled `skill` row for `plugin-a`, and
    // plant a MALFORMED `hooks/hooks.json` under the plugin's on-disk root
    // (manifest-less fallback: `<cache_dir_for(url)>/<plugin>/...`).
    let url = "https://example.test/plugin-a.git";
    let cache = paths.cache_dir_for(url);
    let hooks_dir = cache.join("plugin-a").join("hooks");
    fs::create_dir_all(&hooks_dir).expect("create hooks dir");
    fs::write(hooks_dir.join("hooks.json"), "{ not valid json").expect("write malformed hooks");

    {
        // Raw read-write connection — `open_read_only` cannot INSERT, and the
        // schema is already bootstrapped by `seed_workspace_with_registry_seeds`.
        let conn = rusqlite::Connection::open(&paths.index_db).expect("rusqlite open rw");
        tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-a", url, "main")
            .expect("enrol catalog");
        // An enabled `skill`-kind row so the plugin shows up in the
        // enabled-plugin enumeration the hooks pass walks.
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES ('cat-a', 'plugin-a', 'demo', 'skill', 'd', '0.0.0',
                     'skills/demo/SKILL.md', 'h', 1, 0, NULL, '1970-01-01T00:00:00Z')",
            [],
        )
        .expect("insert skill row");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog='cat-a' AND plugin='plugin-a'",
                [],
                |r| r.get(0),
            )
            .expect("skill id");
        let ws_id: i64 = conn
            .query_row(
                "SELECT id FROM workspaces WHERE name = 'test-ws'",
                [],
                |r| r.get(0),
            )
            .expect("ws id");
        conn.execute(
            "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
            rusqlite::params![ws_id, skill_id],
        )
        .expect("enrol skill");
    }

    let project = env.home_path().join("project");
    fs::create_dir_all(&project).expect("create project");

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["workspace", "use", "test-ws"])
        .output()
        .expect("spawn workspace use");

    assert_eq!(
        out.status.code(),
        Some(43),
        "expected exit 43 HookSpecParseError, got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // The hooks pass must never have written the local settings file — the
    // malformed source fails before any merge. (`.claude/settings.json` is the
    // separate MCP-config sink and may legitimately exist from the MCP write.)
    assert!(
        !project.join(".claude/settings.local.json").exists(),
        "settings.local.json must not be written when the hook source is malformed",
    );
}

// =====================================================================
// Phase 6 / US3 — guardrails write failure → exit 46 (library-API).
// =====================================================================

/// A symlinked in-file guardrails target is refused before any write; the
/// failure surfaces `GuardrailsWriteFailed` (exit 46), naming the file.
#[test]
#[cfg(unix)]
fn guardrails_write_through_symlink_exits_46() {
    use std::collections::BTreeMap;
    use tome::harness::guardrails;

    let dir = TempDir::new().expect("tempdir");
    let decoy = dir.path().join("decoy.md");
    fs::write(&decoy, "ORIGINAL\n").expect("write decoy");
    let target = dir.path().join("CLAUDE.md");
    std::os::unix::fs::symlink(&decoy, &target).expect("plant symlink");

    let mut desired = BTreeMap::new();
    desired.insert("cat:plug".to_string(), "be careful\n".to_string());

    let err = guardrails::reconcile_in_file_region(&target, &desired)
        .expect_err("symlinked guardrails target must be refused");
    assert_eq!(
        err.exit_code(),
        46,
        "guardrails write through a symlink → exit 46; got {err:?}"
    );

    // The decoy the symlink pointed at is untouched.
    assert_eq!(
        fs::read_to_string(&decoy).unwrap(),
        "ORIGINAL\n",
        "the symlink target must NOT be overwritten"
    );
}
