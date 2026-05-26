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
