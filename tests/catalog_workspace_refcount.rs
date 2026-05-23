//! Phase 4 / F11c-1 — `workspace_catalogs` refcount + advisory-lock
//! semantics for `tome catalog add | remove` across two workspaces.
//!
//! Subsumes the Phase 3 `catalog_cache_refcount.rs` coverage that
//! depended on the now-gone `tome workspace init <path>` + `--global`
//! flag. The on-disk content-addressed cache is shared when two
//! workspaces enrol the same URL, and only torn down when the LAST
//! enrolment goes away. Concurrent removes serialise on the advisory
//! lock per FR-366 / FR-367.
//!
//! Spec: FR-361, FR-366, FR-367.
//! Contract: `contracts/catalog-and-plugin-extensions-p4.md`.

mod common;

use common::{Fixture, ToolEnv, cache_dir_for, paths_for, seed_workspace};

/// Convenience: spawn `tome catalog add --workspace <ws> <url>` against
/// the test env. We invoke the CLI binary so the production write path
/// (`workspace_catalogs::insert` under the advisory lock) is exercised.
fn catalog_add(env: &ToolEnv, workspace: &str, url: &str) -> std::process::Output {
    env.cmd()
        .args(["--workspace", workspace, "catalog", "add", url])
        .output()
        .expect("spawn catalog add")
}

/// Convenience: spawn `tome catalog remove --workspace <ws> <name> --force`.
fn catalog_remove_force(env: &ToolEnv, workspace: &str, name: &str) -> std::process::Output {
    env.cmd()
        .args([
            "--workspace",
            workspace,
            "catalog",
            "remove",
            name,
            "--force",
        ])
        .output()
        .expect("spawn catalog remove")
}

#[test]
fn two_workspaces_enroll_same_url_share_one_clone() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let paths = paths_for(&env);

    // First add via CLI — this also stamps meta + bootstraps the
    // privileged `global` workspace row. Use --workspace global so the
    // resolver doesn't pick up the cwd-walk on the harness's project.
    let add_g = catalog_add(&env, "global", &fix.url);
    assert!(
        add_g.status.success(),
        "first add failed: stderr={}",
        String::from_utf8_lossy(&add_g.stderr),
    );

    // Seed the second workspace. Meta is already stamped by the first
    // add (with registry seeds), so seed_workspace's stub-seed open is
    // a no-op on meta (`open` is first-writer-wins). The workspace row
    // itself goes through cleanly.
    seed_workspace(&paths, "second");

    let add_s = catalog_add(&env, "second", &fix.url);
    assert!(
        add_s.status.success(),
        "second-workspace add failed: stderr={}",
        String::from_utf8_lossy(&add_s.stderr),
    );

    // Exactly one on-disk clone for this URL.
    let cache = cache_dir_for(&env, &fix.url);
    assert!(
        cache.is_dir(),
        "shared cache should exist at {}",
        cache.display(),
    );

    // Two enrolment rows pointing at the same URL.
    let conn = tome::index::open_read_only(&paths.index_db).expect("open ro");
    let refs =
        tome::index::workspace_catalogs::refcount_by_url(&conn, &fix.url).expect("refcount_by_url");
    assert_eq!(refs, 2, "expected refcount=2 across (global, second)");
}

#[test]
fn remove_from_one_of_two_keeps_clone() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let paths = paths_for(&env);

    assert!(catalog_add(&env, "global", &fix.url).status.success());
    seed_workspace(&paths, "second");
    assert!(catalog_add(&env, "second", &fix.url).status.success());

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // Remove from `global` only.
    let rm = catalog_remove_force(&env, "global", "sample-experts");
    assert!(
        rm.status.success(),
        "global remove failed: stderr={}",
        String::from_utf8_lossy(&rm.stderr),
    );

    // Clone survives because `second` still references it.
    assert!(
        cache.is_dir(),
        "cache directory was removed despite `second` still referencing it",
    );

    // One enrolment left.
    let conn = tome::index::open_read_only(&paths.index_db).expect("open ro");
    let refs = tome::index::workspace_catalogs::refcount_by_url(&conn, &fix.url).unwrap();
    assert_eq!(refs, 1, "expected refcount=1 after one workspace removed");
}

#[test]
fn remove_from_last_referencing_workspace_removes_clone() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let paths = paths_for(&env);

    assert!(catalog_add(&env, "global", &fix.url).status.success());
    seed_workspace(&paths, "second");
    assert!(catalog_add(&env, "second", &fix.url).status.success());

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // Remove from `global`. Clone should still exist.
    assert!(
        catalog_remove_force(&env, "global", "sample-experts")
            .status
            .success(),
    );
    assert!(cache.is_dir(), "cache still referenced by `second`");

    // Remove from `second`. Now the clone goes away.
    assert!(
        catalog_remove_force(&env, "second", "sample-experts")
            .status
            .success(),
    );
    assert!(
        !cache.exists(),
        "cache directory should be gone after the last referencing workspace removed it",
    );

    let conn = tome::index::open_read_only(&paths.index_db).expect("open ro");
    let refs = tome::index::workspace_catalogs::refcount_by_url(&conn, &fix.url).unwrap();
    assert_eq!(refs, 0, "expected refcount=0 after both removes");
}

#[test]
fn sequential_remove_of_same_catalog_from_two_workspaces_is_idempotent() {
    // Sequential removes of the SAME `(workspace, catalog)` pair: first
    // wins, second observes the row gone and returns CatalogNotFound
    // (exit 3) per FR-367 (sequential-emulation of the concurrent race
    // outcome). The OTHER workspace's enrolment then removes cleanly.
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let paths = paths_for(&env);

    assert!(catalog_add(&env, "global", &fix.url).status.success());
    seed_workspace(&paths, "second");
    assert!(catalog_add(&env, "second", &fix.url).status.success());

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // First remove from `global` — succeeds.
    let rm1 = catalog_remove_force(&env, "global", "sample-experts");
    assert!(
        rm1.status.success(),
        "first remove failed: stderr={}",
        String::from_utf8_lossy(&rm1.stderr),
    );

    // Cache still alive — `second` still references it.
    assert!(cache.is_dir());

    // Re-attempt remove from `global` — row gone, CatalogNotFound (exit 3).
    let rm1_again = catalog_remove_force(&env, "global", "sample-experts");
    assert_eq!(
        rm1_again.status.code(),
        Some(3),
        "expected exit 3 (CatalogNotFound), got {:?}; stderr={}",
        rm1_again.status.code(),
        String::from_utf8_lossy(&rm1_again.stderr),
    );

    // Remove from `second` — succeeds; clone goes away.
    assert!(
        catalog_remove_force(&env, "second", "sample-experts")
            .status
            .success(),
    );
    assert!(!cache.exists(), "cache should be gone after final remove");
}

/// FR-367 concurrent-remove serialisation. The advisory lock is
/// non-blocking (`try_lock` → `IndexBusy` exit 50 on contention), so two
/// truly concurrent removes can either both succeed in series OR one
/// hits `IndexBusy`. Both outcomes are documented as benign.
///
/// We drive this via library calls from two threads on a `Barrier(2)`
/// rather than spawning two CLI binaries — spawning is too coarse to
/// reliably overlap the lock-hold window. The library call IS the same
/// `workspace_catalogs::delete` the CLI invokes under the lock.
///
/// Post-condition: both rows are gone, refcount = 0, cache is removed
/// exactly once.
#[test]
fn concurrent_remove_from_two_workspaces_is_serialised() {
    use std::sync::{Arc, Barrier};

    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let paths = paths_for(&env);

    assert!(catalog_add(&env, "global", &fix.url).status.success());
    seed_workspace(&paths, "second");
    assert!(catalog_add(&env, "second", &fix.url).status.success());

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // Two threads, each acquires the advisory lock and deletes ITS
    // workspace's enrolment + (if it's the last) tears down the cache.
    // Mirrors the production `catalog remove` body in compressed form.
    let barrier = Arc::new(Barrier::new(2));
    let paths_arc = Arc::new(paths.clone());
    let url = fix.url.clone();

    let mut handles = Vec::new();
    for workspace in ["global", "second"] {
        let barrier = Arc::clone(&barrier);
        let paths = Arc::clone(&paths_arc);
        let url = url.clone();
        handles.push(std::thread::spawn(move || -> Result<(), String> {
            barrier.wait();

            // Retry the advisory-lock acquisition on `IndexBusy`. The
            // non-blocking try_lock means a concurrent holder can
            // surface `IndexBusy` (exit 50); the test acceptance is
            // that the operation eventually completes — either in
            // first-try serial order or after a short backoff.
            //
            // Production callers don't retry (single-shot CLI), but
            // for the test we want to assert the post-state.
            let cache = paths.cache_dir_for(&url);
            for attempt in 0..40 {
                let lock = match tome::index::acquire_lock(&paths.index_lock) {
                    Ok(l) => l,
                    Err(tome::error::TomeError::IndexBusy) => {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        if attempt == 39 {
                            return Err(format!("{workspace}: IndexBusy after retries"));
                        }
                        continue;
                    }
                    Err(e) => return Err(format!("{workspace}: lock error {e}")),
                };

                let conn = tome::index::open(
                    &paths.index_db,
                    &tome::index::OpenOptions {
                        embedder: common::stub_embedder_seed(),
                        reranker: common::stub_reranker_seed(),
                        summariser: common::stub_summariser_seed(),
                    },
                )
                .map_err(|e| format!("{workspace}: open {e}"))?;

                let removed =
                    tome::index::workspace_catalogs::delete(&conn, workspace, "sample-experts")
                        .map_err(|e| format!("{workspace}: delete {e}"))?;
                if !removed {
                    return Err(format!("{workspace}: delete returned false"));
                }

                let refs = tome::index::workspace_catalogs::refcount_by_url(&conn, &url)
                    .map_err(|e| format!("{workspace}: refcount {e}"))?;
                if refs == 0
                    && let Err(e) = std::fs::remove_dir_all(&cache)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(format!("{workspace}: remove_dir_all {e}"));
                }
                drop(lock);
                return Ok(());
            }
            unreachable!("loop exited without return")
        }));
    }

    for h in handles {
        h.join().expect("thread panicked").expect("thread result");
    }

    // Post-conditions: both rows gone, cache removed exactly once.
    let conn = tome::index::open_read_only(&paths.index_db).expect("open ro");
    let refs = tome::index::workspace_catalogs::refcount_by_url(&conn, &fix.url).unwrap();
    assert_eq!(refs, 0, "expected refcount=0 after both removes");
    assert!(
        !cache.exists(),
        "cache directory should be gone after both removes",
    );
}
