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
//! | 5    | ManifestInvalid               | `workspace_info_with_malformed_global_config_exits_5`     |
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
//! | 45   | AgentTranslationFailed        | `workspace_use_malformed_agent_exits_45`     |
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

use std::fs;

use crate::common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_all_registry_models,
    paths_for, stage_sample_catalog_in_db, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed, write_config_for_cli,
};
use tempfile::TempDir;
use tome::embedding::registry::MODEL_REGISTRY;
use tome::index::{OpenOptions, open};

fn options() -> OpenOptions {
    OpenOptions {
        embedder: stub_embedder_seed(),
        reranker: stub_reranker_seed(),
        summariser: stub_summariser_seed(),
        profile: None,
    }
}

#[test]
fn plugin_show_with_malformed_tome_plugin_toml_exits_22() {
    // Setup: copy the sample-plugin-catalog fixture (whose plugins carry a
    // native `tome-plugin.toml`), register it via a hand-written config, then
    // corrupt plugin-alpha's `tome-plugin.toml`. Post-cutover `tome plugin
    // show` reads ONLY the native manifest, so a malformed one must surface as
    // exit 22 (`PluginManifestParseError`).
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    let manifest = catalog_root.join("plugin-alpha").join("tome-plugin.toml");
    assert!(
        manifest.is_file(),
        "expected fixture tome-plugin.toml at {}",
        manifest.display()
    );
    fs::write(&manifest, b"this is = not valid = toml").expect("write bad tome-plugin.toml");

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
fn plugin_show_on_unconverted_plugin_exits_80_with_convert_hint() {
    // US1 closeout TEST-M5: the headline cutover UX. A plugin carrying only the
    // legacy `.claude-plugin/plugin.json` (no `tome-plugin.toml`) is
    // *unconverted* → `tome plugin show` exits 80 (PluginNotConverted) with a
    // hint pointing at `tome plugin convert`.
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    // Downgrade plugin-alpha to a pre-cutover (unconverted) plugin: delete the
    // native manifest, leaving only the legacy `.claude-plugin/plugin.json`.
    let native = catalog_root.join("plugin-alpha").join("tome-plugin.toml");
    fs::remove_file(&native).expect("remove native manifest");
    assert!(
        catalog_root
            .join("plugin-alpha")
            .join(".claude-plugin")
            .join("plugin.json")
            .is_file(),
        "legacy plugin.json must remain"
    );

    let out = env
        .cmd()
        .args(["plugin", "show", "sample-plugin-catalog/plugin-alpha"])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(80),
        "expected exit 80 PluginNotConverted, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("convert"),
        "stderr must hint at conversion; got: {stderr}"
    );
}

#[test]
fn models_list_with_corrupt_manifest_exits_33() {
    // Setup: stage a fabricated model directory, then corrupt its
    // manifest.toml. `tome models list` reads the manifest and surfaces
    // a parse failure as exit 33 (`ModelRegistrationParseError`).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");
    fabricate_all_registry_models(&paths);

    let entry = MODEL_REGISTRY.first().expect("at least one model");
    let manifest_path = paths.models_dir.join(entry.name).join("manifest.toml");
    fs::write(&manifest_path, b"this is = not = valid = toml").expect("corrupt manifest");

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
        use crate::common::seed_workspace;
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
        use crate::common::seed_workspace;
        seed_workspace(&paths, "test-ws");
    }

    // Global config declares claude-code as the only effective harness.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .expect("write global config");

    // The project lives under $HOME. The walk guard (Critical fix) ensures that
    // ~/.tome/config.toml is never mistaken for a project marker, so the
    // project dir no longer needs to be in a separate TempDir outside $HOME.
    let project = env.home_path().join("project");
    fs::create_dir_all(&project).expect("create project");

    // User-owned `tome` entry in .mcp.json (claude-code's MCP config path
    // since issue #496; previously .claude/settings.json).
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    fs::write(
        project.join(".mcp.json"),
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
    crate::common::seed_workspace(&paths, "test-ws");

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
            profile: None,
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

    // Global config declares claude-code.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .expect("write global config");

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

    // Pre-populate a user-owned `tome` entry in .mcp.json (claude-code's
    // MCP config path since issue #496).
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    fs::write(
        project.join(".mcp.json"),
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
    let after = fs::read_to_string(project.join(".mcp.json")).expect("read .mcp.json");
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

    // Task 2: global harness settings now live in config.toml [harness].enabled.
    fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .expect("write global config");

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

    // Pre-populate a user-owned `tome` entry in .mcp.json (claude-code's
    // MCP config path since issue #496).
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    fs::write(
        project.join(".mcp.json"),
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

    // Issue #496: the MCP entry is now in .mcp.json, not .claude/settings.json.
    let after = fs::read_to_string(project.join(".mcp.json")).expect("read .mcp.json");
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
    //
    // FF2 (R2/#153 pattern): this test previously wrote `config.toml`
    // directly with NO DB enrolment and so only passed because `parse_scope`
    // read `config.catalogs` — it codified the bug. `reindex` now resolves
    // catalog existence from the `workspace_catalogs` DB, so enrol the
    // catalog there (the real `tome catalog add` shape). With the catalog
    // KNOWN but `no-such-plugin` absent, the exit-20 PluginNotFound path is
    // what we mean to exercise.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("config dir");
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    assert!(
        !paths.global_config_file.exists(),
        "this test must run with NO config.toml (DB enrolment only)",
    );

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

/// #303 — bare `tome sync` outside a project (no marker, no `--all`) in HUMAN
/// mode must (1) fan out to the resolved workspace's bound projects, (2) print
/// the human-only fan-out NOTE to stderr, and (3) actually reconcile the bound
/// project. Binary-driven so the real `eprintln!` from `run()` is asserted on
/// the process's stderr — the enhancement's whole UX signal. Also asserts the
/// `--json` bare-sync report is byte-identical to explicit `--all` (the
/// "note is human-only, --json untouched" claim).
#[test]
fn bare_sync_outside_project_human_prints_fanout_note_and_reconciles() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // Init workspace `foo` and bind a project to it (real central-DB binding via
    // the binary — `workspace use` runs the initial sync too).
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

    // Give the workspace a known central RULES.md and stale the project copy so
    // a genuine reconcile is observable.
    let ws = tome::workspace::WorkspaceName::parse("foo").unwrap();
    let central_rules = paths.workspace_rules_file(&ws);
    fs::create_dir_all(central_rules.parent().unwrap()).expect("workspace dir");
    fs::write(&central_rules, b"foo canonical rules\n").expect("write central RULES.md");
    let project_rules = project.join(".tome/RULES.md");
    fs::write(&project_rules, b"STALE\n").expect("stale project RULES.md");

    // Run bare `tome sync` (HUMAN mode — no --json) from a NON-marker directory
    // that is neither the project nor $HOME. `TOME_WORKSPACE=foo` resolves the
    // scope to `foo` with project_root=None → the bare-sync fan-out branch.
    let elsewhere = env.home_path().join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("create elsewhere");
    let out = env
        .cmd()
        .current_dir(&elsewhere)
        .env("TOME_WORKSPACE", "foo")
        .args(["sync", "--rules-only"])
        .output()
        .expect("spawn bare sync");

    assert_eq!(
        out.status.code(),
        Some(0),
        "bare sync must exit 0, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // (2) The human-only fan-out NOTE fired on stderr.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no project marker here")
            && stderr.contains("syncing every project bound to workspace `foo`")
            && stderr.contains("(like --all)"),
        "fan-out note missing from stderr:\n{stderr}",
    );

    // (3) The bound project was actually reconciled to the workspace body.
    assert_eq!(
        fs::read(&project_rules).expect("read project RULES.md"),
        b"foo canonical rules\n",
        "bare-sync fan-out did not reconcile the bound project",
    );

    // Re-stale, then compare bare-sync `--json` stdout vs explicit `--all`
    // `--json` stdout over the SAME fixture — they must be byte-identical
    // (the "--json untouched, note is human-only" claim).
    fs::write(&project_rules, b"STALE2\n").expect("re-stale");
    let bare_json = env
        .cmd()
        .current_dir(&elsewhere)
        .env("TOME_WORKSPACE", "foo")
        .args(["sync", "--rules-only", "--json"])
        .output()
        .expect("spawn bare sync --json");
    assert_eq!(bare_json.status.code(), Some(0));

    fs::write(&project_rules, b"STALE3\n").expect("re-stale again");
    let all_json = env
        .cmd()
        .current_dir(&elsewhere)
        .env("TOME_WORKSPACE", "foo")
        .args(["sync", "--rules-only", "--all", "--json"])
        .output()
        .expect("spawn --all --json");
    assert_eq!(all_json.status.code(), Some(0));

    assert_eq!(
        bare_json.stdout,
        all_json.stdout,
        "bare-sync --json must be byte-identical to explicit --all --json;\nbare:\n{}\nall:\n{}",
        String::from_utf8_lossy(&bare_json.stdout),
        String::from_utf8_lossy(&all_json.stdout),
    );

    // And bare-sync --json emits NO note on stdout (JSON stays clean).
    let bare_json_stdout = String::from_utf8_lossy(&bare_json.stdout);
    assert!(
        !bare_json_stdout.contains("no project marker here"),
        "the human note must NOT appear on --json stdout:\n{bare_json_stdout}",
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

    // Task 2: global harness settings now live in config.toml [harness].enabled.
    fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"bogus\"]\n",
    )
    .expect("write global config");

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
fn workspace_info_with_malformed_global_config_exits_5() {
    // Write malformed TOML in the global config file, then call any
    // command that parses it. `tome harness list` calls `config::load`
    // which surfaces `ManifestInvalid::TomlParse` → exit 5.
    // Task 2 / fix-3: config-parse failures must be exit 5 (not exit 70).
    // The plan's Global Constraints mandate this: `config.toml` is a
    // config-manifest, not a workspace settings file.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    fs::write(&paths.global_config_file, "this is = not = valid = toml\n")
        .expect("write malformed config");

    let out = env
        .cmd()
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list");
    assert_eq!(
        out.status.code(),
        Some(5),
        "expected exit 5 ManifestInvalid (config parse failure), got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Critical fix (Task 2): `~/.tome/config.toml` (written by
/// `harness use --scope global`) must NEVER be misidentified as a project
/// marker by `walk_for_project_marker`. Running any command from a directory
/// under `$HOME` (with no closer `.tome/config.toml`) must NOT exit 70
/// (`WorkspaceMalformed`) from trying to parse the global config as a
/// `ProjectMarkerConfig`. Instead it should resolve via the global fallback
/// and exit 0 (or the expected code for the subcommand).
#[test]
fn global_config_present_does_not_cause_exit_70_for_commands_under_home() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // Write a valid global config with [harness].enabled.
    fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .expect("write global config");

    // A project directory UNDER $HOME with no .tome/ marker of its own.
    // Before the walk guard fix, the walk would stop at $HOME/.tome/config.toml
    // and try to parse it as a ProjectMarkerConfig → exit 70.
    let project = env.home_path().join("unmarked-project");
    fs::create_dir_all(&project).expect("create project dir");

    // `tome harness list` resolves the effective harness list. With the walk
    // guard it should either exit 0 (global fallback works) or 18 (unsupported
    // harness in the list — but "claude-code" is valid so expect 0 or possibly
    // a non-zero code from list printing, NOT 70).
    let out = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list from unmarked project under home");

    assert_ne!(
        out.status.code(),
        Some(70),
        "global config.toml must NOT be misidentified as a project marker \
         (walk guard fix); got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Issue #302: when `[workspace] default` wins resolution AND a per-project
/// `.tome/config.toml` marker exists in the CWD ancestry, the CLI prints a
/// one-line `note:` on stderr explaining the override. The exit status is
/// unchanged (exit 0) and the resolved workspace is still the config default —
/// this is an additive notice, not an error.
#[test]
fn workspace_default_overriding_project_marker_prints_stderr_note() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // Seed the "work" workspace in the central DB so the config default passes
    // the membership check.
    let out = env
        .cmd()
        .args(["workspace", "init", "work"])
        .output()
        .expect("spawn workspace init work");
    assert!(
        out.status.success(),
        "workspace init work stderr:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Set `[workspace] default = "work"` in the global config.
    fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .expect("write global config");

    // A project dir UNDER $HOME with its OWN `.tome/config.toml` marker — the
    // per-project binding the config default will shadow.
    let project = env.home_path().join("bound-project");
    fs::create_dir_all(project.join(".tome")).expect("create marker dir");
    fs::write(project.join(".tome/config.toml"), "workspace = \"work\"\n")
        .expect("write project marker");

    // Any cheap foreground command that resolves scope before dispatch.
    let out = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list from bound project");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Exit status unchanged (the notice is additive; `harness list` succeeds).
    assert_eq!(
        out.status.code(),
        Some(0),
        "notice must NOT change the exit status; got {:?}, stdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code(),
    );

    // The one-line stderr note names the shadowing default, the marker path,
    // and the accurate remediation commands.
    assert!(
        stderr.contains("note: [workspace] default 'work' is overriding the project binding"),
        "expected the override note on stderr; stderr:\n{stderr}",
    );
    assert!(
        stderr.contains("bound-project"),
        "note must reference the shadowed project directory (`overridden_project_marker`); \
         stderr:\n{stderr}",
    );
    assert!(
        stderr.contains("tome workspace use"),
        "note must name the accurate remediation command; stderr:\n{stderr}",
    );
}

/// Issue #302 (`--json` gate): the override `note:` is a human-mode affordance
/// only — in `--json` mode the note is suppressed so structured-stdout consumers
/// aren't handed an unstructured stderr line. This exercises the SAME shadowing
/// scenario as the positive test (config `[workspace] default` set + a `.tome`
/// project marker in the CWD ancestry, so the Config-wins branch DOES populate
/// `overridden_project_marker`) — proving the `--json` gate, not an unpopulated
/// field, is what suppresses the note. Exit status is unchanged (exit 0).
#[test]
fn workspace_default_overriding_project_marker_in_json_mode_prints_no_note() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    // Seed "work" so the config default passes the membership check — the field
    // IS populated, and only the `--json` gate suppresses the note.
    let out = env
        .cmd()
        .args(["workspace", "init", "work"])
        .output()
        .expect("spawn workspace init work");
    assert!(
        out.status.success(),
        "workspace init work stderr:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );

    fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .expect("write global config");

    // A project dir UNDER $HOME with its OWN `.tome/config.toml` marker — the
    // exact shadowing scenario that populates the field.
    let project = env.home_path().join("json-bound-project");
    fs::create_dir_all(project.join(".tome")).expect("create marker dir");
    fs::write(project.join(".tome/config.toml"), "workspace = \"work\"\n")
        .expect("write project marker");

    // Same foreground command, in `--json` mode. The pre-dispatch resolve still
    // populates the field; the `main.rs` note guard skips it because `mode ==
    // Json`.
    let out = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "list", "--json"])
        .output()
        .expect("spawn harness list --json from bound project");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Exit status unchanged.
    assert_eq!(
        out.status.code(),
        Some(0),
        "the --json gate must not change the exit status; got {:?}, stdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code(),
    );

    // The note must NOT appear on stderr in `--json` mode.
    assert!(
        !stderr.contains("is overriding the project binding"),
        "the override note must be suppressed in --json mode; stderr:\n{stderr}",
    );
}

/// Issue #302 (negative): with `[workspace] default` set but NO project marker
/// in the CWD ancestry, nothing is shadowed → no `note:` is printed.
#[test]
fn workspace_default_without_project_marker_prints_no_note() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let out = env
        .cmd()
        .args(["workspace", "init", "work"])
        .output()
        .expect("spawn workspace init work");
    assert!(out.status.success());

    fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .expect("write global config");

    // A project dir UNDER $HOME with NO `.tome/config.toml` marker of its own.
    let project = env.home_path().join("unmarked-project");
    fs::create_dir_all(&project).expect("create project dir");

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list from unmarked project");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("is overriding the project binding"),
        "no marker present → no override note; stderr:\n{stderr}",
    );
}

/// Issue #302 (MCP path): the override `note:` is a CLI-foreground affordance
/// only — `tome mcp` speaks JSON-RPC and must NEVER surface it. Even in the
/// exact Config-default-shadows-a-marker scenario (which populates
/// `overridden_project_marker` on the resolved scope), spawning `tome mcp` from
/// that CWD does not print the note to the process's stderr. The scope is
/// still stamped via `--workspace` at real `harness sync`, but the resolver
/// here reaches step 3, so this exercises the same detection the CLI note uses
/// — proving the emit is gated OFF for the MCP command in `main.rs`.
#[test]
fn mcp_path_does_not_emit_override_note() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");

    let out = env
        .cmd()
        .args(["workspace", "init", "work"])
        .output()
        .expect("spawn workspace init work");
    assert!(out.status.success());

    fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .expect("write global config");

    let project = env.home_path().join("mcp-bound-project");
    fs::create_dir_all(project.join(".tome")).expect("create marker dir");
    fs::write(project.join(".tome/config.toml"), "workspace = \"work\"\n")
        .expect("write project marker");

    // Spawn `tome mcp` WITHOUT `--workspace` so resolution reaches step 3
    // (`[workspace] default`) — the branch that populates
    // `overridden_project_marker`. Closed stdin so the server (if it starts)
    // shuts down immediately; whether preflight then fails on missing models or
    // the server starts, the notice guard in `main.rs` runs BEFORE `mcp::run`
    // and skips the note for the MCP command. `TOME_MCP_LOG=off` keeps the
    // server's own file-log sink off so stderr carries only what `main.rs`
    // would print.
    let out = env
        .cmd()
        .current_dir(&project)
        .env("TOME_MCP_LOG", "off")
        .args(["mcp"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn tome mcp from bound project");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("is overriding the project binding"),
        "the MCP path must never emit the override note; stderr:\n{stderr}",
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
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .expect("write global config");

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

    // The project lives under $HOME. The walk guard (Critical fix) ensures that
    // ~/.tome/config.toml is never mistaken for a project marker.
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

    // Forward progress: the malformed THIRD-PARTY hook source contributes
    // nothing (it fails parsing → recorded as the first error → surfaced as
    // exit 43), and is never merged. Tome's own trusted SessionStart routing
    // hook is still reconciled for the live claude-code harness, so
    // settings.local.json may now exist — but if it does it must contain ONLY
    // that Tome-owned entry, proving the malformed plugin merged nothing.
    // (`.claude/settings.json` is the separate MCP-config sink and may
    // legitimately exist from the MCP write.)
    let settings_local = project.join(".claude/settings.local.json");
    if settings_local.exists() {
        let doc: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&settings_local).expect("read settings.local.json"),
        )
        .expect("settings.local.json is valid JSON");
        let hooks = doc
            .get("hooks")
            .and_then(|h| h.as_object())
            .expect("hooks object present");
        // Only the SessionStart event, holding exactly the Tome routing hook.
        assert_eq!(
            hooks.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["SessionStart"],
            "only Tome's SessionStart hook may be written; the malformed plugin \
             must contribute nothing: {doc}",
        );
        let entries = hooks["SessionStart"]
            .as_array()
            .expect("SessionStart is an array");
        assert_eq!(
            entries.len(),
            1,
            "exactly one (Tome-owned) SessionStart entry"
        );
        let cmd = entries[0]["hooks"][0]["command"].as_str().unwrap_or("");
        assert!(
            cmd.contains("harness session-start"),
            "the sole entry must be Tome's session-start hook, got: {cmd}",
        );
    }
}

// ---------------------------------------------------------------------------
// Phase 6 / US5 / T134: a malformed enabled agent surfaces exit 45
// (`AgentTranslationFailed`) through the binary. `tome workspace use` binds the
// project and runs sync; the agents pass re-translates the enabled agent's
// source `.md` for the claude-code harness and fails on the malformed
// frontmatter. This mirrors the exit-43 hooks split: the sync/translation path
// is the binary-reachable surface, and a malformed agent is forced through it
// cheaply (sync doesn't load ONNX).
// ---------------------------------------------------------------------------

#[test]
fn workspace_use_malformed_agent_exits_45() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("data dir");
    seed_workspace_with_registry_seeds(&paths, "test-ws");

    // claude-code supports native agents → the sync agents pass translates.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .expect("write global config");

    // Plant a MALFORMED agent source (no frontmatter delimiters at all) under
    // the plugin's on-disk root (manifest-less fallback:
    // `<cache_dir_for(url)>/<plugin>/agents/<name>.md`), then enrol the catalog
    // and insert an enabled `agent`-kind row pointing at it.
    let url = "https://example.test/plugin-a.git";
    let cache = paths.cache_dir_for(url);
    let agent_dir = cache.join("plugin-a").join("agents");
    fs::create_dir_all(&agent_dir).expect("create agent dir");
    fs::write(
        agent_dir.join("broken.md"),
        "this agent file has no frontmatter at all\n",
    )
    .expect("write malformed agent");

    {
        let conn = rusqlite::Connection::open(&paths.index_db).expect("rusqlite open rw");
        tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-a", url, "main")
            .expect("enrol catalog");
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES ('cat-a', 'plugin-a', 'broken', 'agent', 'd', '0.0.0',
                     'agents/broken.md', 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
            [],
        )
        .expect("insert agent row");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog='cat-a' AND plugin='plugin-a' AND kind='agent'",
                [],
                |r| r.get(0),
            )
            .expect("agent id");
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
        .expect("enrol agent");
    }

    // The project lives under $HOME. The walk guard (Critical fix) ensures that
    // ~/.tome/config.toml is never mistaken for a project marker.
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
        Some(45),
        "expected exit 45 AgentTranslationFailed, got {:?}, stdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
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

// ---------------------------------------------------------------------------
// Phase 9 / US1 — `tome meta` exit codes (87 unknown skill, 89 no harness).
//
// 88 (install/unsafe-path) is covered in-process in `tests/meta_cli.rs`
// (`add_symlinked_component_is_88_forward_progress_no_escape`) — reproducing it
// via the spawned binary needs the real harness AND a planted symlink, which the
// synthetic-registry in-process test exercises far more directly.
// ---------------------------------------------------------------------------

#[test]
fn meta_add_unknown_skill_exits_87() {
    // The skill-id lookup fails closed BEFORE target resolution, so an unknown
    // id is 87 regardless of which harnesses are present.
    let env = ToolEnv::new();
    let cwd = TempDir::new().expect("isolated cwd");
    let out = env
        .cmd()
        .current_dir(cwd.path())
        .args(["meta", "add", "no-such-skill"])
        .output()
        .expect("spawn tome meta add");
    assert_eq!(
        out.status.code(),
        Some(87),
        "unknown skill id → 87; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn meta_add_no_harness_detected_exits_89() {
    // Isolated $HOME with no harness dotdirs and a marker-less CWD → the
    // all-detected default finds nothing → 89.
    let env = ToolEnv::new();
    let cwd = TempDir::new().expect("isolated cwd");
    let out = env
        .cmd()
        .current_dir(cwd.path())
        .args(["meta", "add", "convert-marketplace"])
        .output()
        .expect("spawn tome meta add");
    assert_eq!(
        out.status.code(),
        Some(89),
        "no detected harness → 89; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---- #436: `tome exit-codes` (pre-dispatch, no HOME/index/config) ----------

/// The full table renders on a completely unconfigured machine (nothing under
/// `$HOME` — the command is intercepted before `Paths::resolve()`), and one
/// code's row filters correctly, in both human and `--json` modes.
#[test]
fn exit_codes_command_works_without_any_configured_state() {
    let env = ToolEnv::new();
    // Deliberately NO data-dir creation: the command needs no state at all.

    let full = env
        .cmd()
        .args(["exit-codes"])
        .output()
        .expect("spawn exit-codes");
    assert_eq!(
        full.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&full.stderr),
    );
    let stdout = String::from_utf8_lossy(&full.stdout);
    for needle in ["index_busy", "remote_embedding_invalid", "Success."] {
        assert!(
            stdout.contains(needle),
            "full table missing `{needle}`:\n{stdout}"
        );
    }

    // Single-code JSON: exactly one row, the right one, `category` non-null.
    let one = env
        .cmd()
        .args(["--json", "exit-codes", "50"])
        .output()
        .expect("spawn exit-codes 50");
    assert_eq!(one.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&one.stdout).expect("parse JSON");
    let rows = v["exit_codes"].as_array().expect("exit_codes array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["code"], 50);
    assert_eq!(rows[0]["category"], "index_busy");

    // The success row's `category` is JSON null (pinned — the cli-surface
    // `exitCodes` shape).
    let zero = env
        .cmd()
        .args(["--json", "exit-codes", "0"])
        .output()
        .expect("spawn exit-codes 0");
    let v: serde_json::Value = serde_json::from_slice(&zero.stdout).expect("parse JSON");
    assert!(v["exit_codes"][0]["category"].is_null());
}

/// An unknown code is a usage error (exit 2) that names the code and points
/// at the full table.
#[test]
fn exit_codes_command_unknown_code_exits_2() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["exit-codes", "11"])
        .output()
        .expect("spawn exit-codes 11");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown exit code 11"), "{stderr}");
    assert!(stderr.contains("tome exit-codes"), "{stderr}");
}
