//! F-CACHE-KEY regression: the on-disk cache directory and the reuse
//! refcount MUST be keyed by the *scrubbed* URL — the same key every
//! reader (`show` / `update` / `remove` / the reuse path) resolves by —
//! while the clone itself still uses the *raw* URL for auth.
//!
//! The bug: `add` keyed `cache_dir_for` + `refcount_by_url` by the raw
//! resolved URL, but stored the scrubbed URL into `workspace_catalogs`.
//! For any source whose URL changes under scrubbing (plain SSH
//! `git@host:owner/repo`, `ssh://…`, `https://user:token@…`, or — as used
//! here against a local fixture — `file://user:token@/path`), the
//! add-time key differed from the read-time key: the clone landed under
//! `sha256(raw)` while every reader looked under `sha256(scrubbed)`,
//! orphaning the clone on disk and breaking reuse/refcount.
//!
//! We exercise the round-trip with a credential-bearing `file://` URL.
//! `git` silently ignores userinfo for local transports (so the clone
//! still works with no network), but `git::scrub_credentials` strips the
//! `user:token@` userinfo via its URL-login rule — making
//! `scrubbed != raw`, which is exactly the condition that surfaces the
//! bug. A plain-`https` case (where `raw == scrubbed`) is the regression
//! guard that must stay green.

use std::path::Path;

use crate::common::{Fixture, ToolEnv, global_enrolment_url, paths_for};
use serde_json::Value;

/// Count the catalog clone directories under `catalogs_dir`. After a
/// successful `add` there should be exactly one; after a `remove` whose
/// refcount hit zero there should be none. A leftover here is an orphaned
/// clone — the precise failure F-CACHE-KEY guards against.
fn clone_dir_count(catalogs_dir: &Path) -> usize {
    match std::fs::read_dir(catalogs_dir) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            // Ignore transient staging dirs (`.tome-incoming-*`); only the
            // landed content-addressed clones count.
            .filter(|e| {
                e.path().is_dir()
                    && !e
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".tome-incoming-")
            })
            .count(),
        // Dir absent => no clones at all.
        Err(_) => 0,
    }
}

/// Build a credential-bearing `file://user:token@/path` URL for the
/// fixture. The userinfo is ignored by `git` for local transports but is
/// stripped by `scrub_credentials`, so `scrubbed != raw`.
fn creds_url(fix: &Fixture) -> String {
    fix.url.replacen("file://", "file://alice:supersecret@", 1)
}

#[test]
fn ssh_form_source_round_trips_through_show_update_remove_with_zero_orphans() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let raw_url = creds_url(&fix);

    // Sanity: this source is one where scrubbing changes the URL. If this
    // ever stops holding the test would be a no-op, so assert it up front.
    let scrubbed_url = global_scrub(&raw_url);
    assert_ne!(
        raw_url, scrubbed_url,
        "test premise broken: scrub must change this URL (raw={raw_url}, scrubbed={scrubbed_url})",
    );

    // --- add ---------------------------------------------------------------
    let out = env
        .cmd()
        .args(["catalog", "add", &raw_url])
        .output()
        .expect("spawn add");
    assert!(
        out.status.success(),
        "add exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let paths = paths_for(&env);

    // The stored URL is the scrubbed one (existing scrubbing invariant).
    let stored = global_enrolment_url(&paths, "sample-experts").expect("enrolment present");
    assert_eq!(
        stored, scrubbed_url,
        "stored URL should be the scrubbed URL"
    );

    // INVARIANT: the clone must land under the SCRUBBED key — the key every
    // reader resolves by. With the bug it lands under the raw key instead.
    let scrubbed_cache = paths.cache_dir_for(&scrubbed_url);
    assert!(
        scrubbed_cache.join("tome-catalog.toml").is_file(),
        "clone is not under the scrubbed-URL cache dir ({}); add keyed the cache by the raw URL",
        scrubbed_cache.display(),
    );
    // And it must NOT be sitting under the raw key (the orphan location).
    let raw_cache = paths.cache_dir_for(&raw_url);
    assert!(
        raw_cache == scrubbed_cache || !raw_cache.exists(),
        "clone landed under the raw-URL cache dir ({}) — that is the orphan the fix prevents",
        raw_cache.display(),
    );

    // Exactly one clone on disk after add.
    assert_eq!(
        clone_dir_count(&paths.catalogs_dir),
        1,
        "expected exactly one clone dir after add",
    );

    // --- show --------------------------------------------------------------
    // `show` resolves the cache dir from the stored (scrubbed) URL. With the
    // bug the clone is under the raw key, so the manifest read 404s -> exit 7.
    let out = env
        .cmd()
        .args(["catalog", "show", "sample-experts", "--json"])
        .output()
        .expect("spawn show");
    assert!(
        out.status.success(),
        "show exit={:?} stderr={} -- cache dir not resolved by the stored scrubbed URL",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("show json");
    assert_eq!(v["name"], "sample-experts");
    assert_eq!(v["plugins"].as_array().unwrap().len(), 2);
    // The credentials must never appear in show output.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("supersecret") && !stdout.contains("alice:"),
        "credentials leaked in show output: {stdout}",
    );

    // --- update ------------------------------------------------------------
    // `update` also resolves the cache dir from the stored (scrubbed) URL and
    // runs git inside it. With the bug it points at an empty/absent dir and
    // git fails (exit 6). Targeted to the single workspace's enrolment.
    let out = env
        .cmd()
        .args(["catalog", "update", "sample-experts"])
        .output()
        .expect("spawn update");
    assert!(
        out.status.success(),
        "update exit={:?} stderr={} -- cache dir not resolved by the stored scrubbed URL",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // --- remove ------------------------------------------------------------
    // `remove` computes the cache path from the stored (scrubbed) URL and,
    // at refcount 0, removes it. With the bug it removes the (empty)
    // scrubbed-keyed dir and leaves the raw-keyed clone orphaned on disk.
    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts", "--force"])
        .output()
        .expect("spawn remove");
    assert!(
        out.status.success(),
        "remove exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // INVARIANT: zero orphaned clones after remove.
    assert_eq!(
        clone_dir_count(&paths.catalogs_dir),
        0,
        "orphaned clone(s) left under {} after remove -- cache dir keyed by raw URL, removed by scrubbed",
        paths.catalogs_dir.display(),
    );
}

#[test]
fn plain_https_raw_equals_scrubbed_still_round_trips() {
    // Regression guard: the common case where the resolved URL is unchanged
    // by scrubbing must keep working unchanged. A plain `file://` fixture URL
    // (no userinfo, no secrets) is the local stand-in for plain https.
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    assert_eq!(
        fix.url,
        global_scrub(&fix.url),
        "premise: this URL is unchanged by scrubbing",
    );

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn add");
    assert!(
        out.status.success(),
        "add exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let paths = paths_for(&env);
    let stored = global_enrolment_url(&paths, "sample-experts").expect("enrolment present");
    assert_eq!(stored, fix.url, "stored URL should equal the raw URL here");
    assert!(
        paths
            .cache_dir_for(&stored)
            .join("tome-catalog.toml")
            .is_file(),
        "clone not resolvable by the stored URL in the raw==scrubbed case",
    );
    assert_eq!(
        clone_dir_count(&paths.catalogs_dir),
        1,
        "one clone after add"
    );

    // show resolves it.
    let out = env
        .cmd()
        .args(["catalog", "show", "sample-experts", "--json"])
        .output()
        .expect("spawn show");
    assert!(
        out.status.success(),
        "show should succeed in raw==scrubbed case"
    );

    // remove leaves zero orphans.
    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts", "--force"])
        .output()
        .expect("spawn remove");
    assert!(out.status.success(), "remove should succeed");
    assert_eq!(
        clone_dir_count(&paths.catalogs_dir),
        0,
        "no orphans after remove in the raw==scrubbed case",
    );
}

/// Mirror of the production scrub transform so the test can compute the
/// expected scrubbed URL without reimplementing the regex rules.
/// `tome::catalog::git::scrub_credentials` is the single source of truth.
fn global_scrub(url: &str) -> String {
    String::from_utf8_lossy(&tome::catalog::git::scrub_credentials(url.as_bytes())).into_owned()
}
