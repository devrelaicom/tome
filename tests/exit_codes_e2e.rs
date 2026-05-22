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
