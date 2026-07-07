//! Issue #430 — `tome doctor` leading verdict, collapsed ok sections, and the
//! `--fix --dry-run` preview. Drives the real CLI binary in an isolated
//! `ToolEnv` (the same pattern as `doctor.rs`'s exit-code tests).
//!
//! A fresh env is deterministic here: the ONLY failing section is Models
//! (embedder/reranker missing) and no section warns, so the verdict counts
//! can be pinned as `1 failing, 0 warnings, <N> ok`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::common::ToolEnv;

/// Snapshot every file under `root` (recursive) as relative-path → contents.
/// Used to prove `--fix --dry-run` mutates nothing.
fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).expect("under root").to_path_buf(),
                    std::fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn verdict_line_leads_and_ok_sections_collapse() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let out = env.cmd().args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);

    // The verdict is the FIRST line: classification + section counts. Piped
    // output uses the ASCII glyph. On a fresh env exactly the Models section
    // fails and nothing warns.
    let first = stdout.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("[fail] unhealthy — 1 failing, 0 warnings, "),
        "verdict must lead with classification + counts; got: {first}",
    );
    assert!(
        first.ends_with(" ok"),
        "counts end with the ok tally: {first}"
    );

    // The failing section renders in full…
    assert!(stdout.contains("Models:"), "{stdout}");
    // …while the all-ok subsystems collapse to one line…
    assert!(
        stdout.contains("(run with --verbose for detail)"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("Detected harnesses:"),
        "ok sections must be collapsed without --verbose: {stdout}",
    );
    assert!(
        !stdout.contains("Catalog caches:"),
        "ok sections must be collapsed without --verbose: {stdout}",
    );
    // …and the actionable tail is never collapsed.
    assert!(stdout.contains("Suggested fixes:"), "{stdout}");
    assert!(stdout.contains("Overall:"), "{stdout}");
}

#[test]
fn verbose_restores_the_full_listing() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let out = env.cmd().args(["doctor", "-v"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Verdict still leads…
    let first = stdout.lines().next().unwrap_or_default();
    assert!(first.starts_with("[fail] unhealthy — "), "got: {first}");
    // …but every section renders and nothing is collapsed.
    assert!(stdout.contains("Detected harnesses:"), "{stdout}");
    assert!(stdout.contains("Catalog caches:"), "{stdout}");
    assert!(stdout.contains("Model registry:"), "{stdout}");
    assert!(
        !stdout.contains("(run with --verbose for detail)"),
        "--verbose must not collapse: {stdout}",
    );
}

#[test]
fn fix_dry_run_lists_fixes_and_applies_nothing() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();

    // Settle any incidental first-run state with a plain read-only doctor,
    // then snapshot: the dry run must add / change / remove NOTHING beyond it.
    let _ = env.cmd().args(["doctor"]).output().unwrap();
    let before = snapshot_tree(env.home_path());

    let out = env
        .cmd()
        .args(["doctor", "--fix", "--dry-run"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);

    // The read-only health exit code — never exit 75 (no fix ran).
    assert_eq!(
        out.status.code(),
        Some(tome::error::EXIT_HEALTH_UNHEALTHY),
        "dry run keeps the read-only health exit; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    // The preview lists the auto-applicable repairs (missing models on a
    // fresh env → `tome models download`).
    assert!(
        stdout.contains("Fix dry run: `tome doctor --fix` would apply"),
        "{stdout}"
    );
    assert!(stdout.contains("tome models download"), "{stdout}");

    // Nothing was applied: the tree is byte-identical.
    assert_eq!(
        before,
        snapshot_tree(env.home_path()),
        "--fix --dry-run must not mutate any state",
    );
}

#[test]
fn dry_run_without_fix_is_usage_error() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let out = env.cmd().args(["doctor", "--dry-run"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "bare --dry-run is a usage error (exit 2); stdout={}",
        String::from_utf8_lossy(&out.stdout),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--dry-run") && stderr.contains("--fix"),
        "the error names the missing flag pair: {stderr}",
    );
}

/// #480: a summariser-only-broken install substantiates its Degraded verdict.
/// The summariser renders in the Models section and counts toward its state as
/// a WARNING (matching the #429 Degraded classification), so the verdict line
/// reads `degraded — 0 failing, 1 warning, …` instead of the pre-#480
/// `degraded — 0 failing, 0 warnings` that no section count substantiated —
/// and the Models body renders in the non-verbose warning bucket naming the
/// summariser.
#[test]
fn summariser_only_broken_counts_as_a_warning() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    // Embedder + reranker installed; the summariser alone reads `missing`.
    {
        use tome::embedding::profile::{Profile, embedder_for, reranker_for};
        crate::common::fabricate_installed_models(
            &paths,
            &[
                embedder_for(Profile::DEFAULT),
                reranker_for(Profile::DEFAULT),
            ],
        );
    }

    let out = env.cmd().args(["doctor"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);

    let first = stdout.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("[warn] degraded — 0 failing, 1 warning,"),
        "a summariser-only-broken install must count exactly one warning; got: {first}",
    );
    // The warning section renders in full without --verbose, naming the
    // summariser state…
    assert!(stdout.contains("Models:"), "{stdout}");
    assert!(
        stdout.contains("summariser") && stdout.contains("missing"),
        "the Models body must name the missing summariser: {stdout}",
    );
    // …and the exit semantics are unchanged (Degraded = exit 10, per #429).
    assert_eq!(out.status.code(), Some(10), "degraded exit code unchanged");
}
