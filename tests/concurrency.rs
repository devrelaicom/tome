//! Phase 10 / T193 — two-process index lock contention.
//!
//! `tests/index_lock.rs` covers intra-process two-fd contention against the
//! `tome::index::acquire_lock` library. This file adds the cross-process
//! regression: hold the lock from the test process, run the `tome` CLI
//! binary against the same lockfile, and confirm `IndexBusy` (exit 50)
//! surfaces all the way through.
//!
//! The test uses `tome catalog remove --force <name>` as the cheap lock
//! path: when the catalog has enabled plugins, the cascade-disable path
//! acquires the advisory lock for pure-deletion work — no `FastembedEmbedder`
//! load, so the test runs in CI without ONNX models.

mod common;

use std::fs::OpenOptions;

use common::{
    Fixture, ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, write_config_for_cli,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

fn enable_alpha(
    paths: &tome::paths::Paths,
    config: &tome::config::Config,
    embedder: &StubEmbedder,
) {
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let deps = LifecycleDeps {
        paths,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable alpha");
}

#[test]
fn second_writer_against_held_lock_exits_50() {
    // Setup: register a catalog with one enabled plugin, then take the
    // advisory lock from the test process before invoking the CLI. The
    // lock is held for the lifetime of `_held` (until end of function).
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    // Hold the lockfile from the test process. We open with the same flags
    // as `index::lock::acquire` and call `try_lock` — once it succeeds, any
    // other open of the same inode trying to `try_lock` will get
    // `WouldBlock`, which `tome` translates into exit 50.
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&paths.index_lock)
        .expect("open lockfile");
    held.try_lock().expect("hold lock");

    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-plugin-catalog", "--force"])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(50),
        "expected exit 50 IndexBusy, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // Releasing the lock allows the next attempt to succeed.
    held.unlock().expect("release lock");
    drop(held);
    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-plugin-catalog", "--force"])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "expected exit 0 after lock released, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn reader_during_writer_does_not_block() {
    // The Phase 2 contract distinguishes write commands (lock-taking) from
    // read commands (lock-free). With the advisory lock held by the test
    // process, `tome catalog list` (Phase 1 catalog read; doesn't open the
    // index) and `tome status` (read-only DB open; never takes the lock)
    // must both succeed without contention.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("add");

    // Take the lock from the test process. `status` and `catalog list`
    // must still complete.
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&paths.index_lock)
        .expect("open lockfile");
    held.try_lock().expect("hold lock");

    let list = env.cmd().args(["catalog", "list"]).output().expect("list");
    assert!(
        list.status.success(),
        "catalog list should not block on the index lock, got {:?}",
        list.status.code(),
    );

    // `tome status` opens the index but never takes the lock; it should
    // also return quickly. The exit code may be 0 or 1 depending on
    // whether the cheap state probe reports models as installed; either
    // way it should not be 50 (IndexBusy).
    let status = env.cmd().args(["status"]).output().expect("status");
    assert_ne!(
        status.status.code(),
        Some(50),
        "status should not block on the index lock, stderr:\n{}",
        String::from_utf8_lossy(&status.stderr),
    );

    held.unlock().expect("release lock");
}
