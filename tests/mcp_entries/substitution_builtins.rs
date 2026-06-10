//! Phase 5 / US2.a — built-ins stage tests.
//!
//! Covers every recognised `${TOME_*}` name, the unknown-pass-through
//! semantics (FR-023), the default-syntax non-trigger for built-ins
//! (FR-022), the path-component sanitisation rule (FR-024), the clock
//! injection seam (`SUBSTITUTION_CLOCK_OVERRIDE` via `ClockOverrideGuard`),
//! lazy data-dir creation via the override seam, idempotence, and the
//! error path on `create_dir_all` failure.

use std::path::PathBuf;
use std::sync::MutexGuard;

use crate::common::{
    ClockOverrideGuard, PluginDataDirGuard, WorkspaceDataDirGuard, lifecycle_paths,
};
use time::OffsetDateTime;
use tome::substitution::{self, SubstitutionContext, SubstitutionContextBuilder};

/// Serialise every test in this binary that installs a substitution override
/// slot (`SUBSTITUTION_CLOCK_OVERRIDE`, `PLUGIN_DATA_DIR_OVERRIDE`,
/// `WORKSPACE_DATA_DIR_OVERRIDE`) or reads one back via `render()`. Those slots
/// are process-global, so co-resident tests in the consolidated binary race
/// otherwise. Backed by the shared [`crate::common::SUBSTITUTION_OVERRIDE_MUTEX`]
/// so serialisation spans every `substitution_*` file in the binary, not just
/// this one (the former per-file mutex only covered a single process).
fn lock_overrides() -> MutexGuard<'static, ()> {
    crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Build a fully-populated context against `home` with all clock
/// fields set deterministically. The caller can override specific
/// fields via the returned builder before `.build()`.
fn ctx_builder(home: &std::path::Path) -> SubstitutionContextBuilder {
    let paths = lifecycle_paths(home);
    SubstitutionContext::builder()
        .catalog_name("test-catalog")
        .plugin_name("test-plugin")
        .plugin_version("1.2.3")
        .entry_name("hello")
        .entry_path(PathBuf::from("/plugins/x/skills/hello/SKILL.md"))
        .entry_dir(PathBuf::from("/plugins/x/skills/hello"))
        .plugin_root_dir(PathBuf::from("/plugins/x"))
        .workspace_name("global")
        .clock(OffsetDateTime::UNIX_EPOCH)
        .paths(paths)
}

fn ctx(home: &std::path::Path) -> SubstitutionContext {
    ctx_builder(home).build().expect("builder")
}

// --- Per-built-in resolution (12 tests) -----------------------------------

#[test]
fn skill_dir_resolves_to_entry_dir() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("dir=${TOME_SKILL_DIR}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "dir=/plugins/x/skills/hello");
}

// --- ${TOME_PROJECT_DIR} (Phase 8 / US1, contracts/substitution-project-dir.md)

#[test]
fn project_dir_resolves_when_set() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_builder(tmp.path())
        .project_dir(Some(PathBuf::from("/work/my-project")))
        .build()
        .expect("builder");
    let out = substitution::render("root=${TOME_PROJECT_DIR}/run.sh", &ctx).unwrap();
    assert_eq!(out, "root=/work/my-project/run.sh");
}

#[test]
fn project_dir_passes_through_verbatim_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    // No project_dir set (default None) → the token passes through VERBATIM,
    // never empty-string, so `${TOME_PROJECT_DIR}/run.sh` cannot collapse to
    // the absolute root `/run.sh`.
    let out = substitution::render("root=${TOME_PROJECT_DIR}/run.sh", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "root=${TOME_PROJECT_DIR}/run.sh");
}

#[test]
fn project_dir_single_sweep_not_rescanned() {
    let tmp = tempfile::tempdir().unwrap();
    // A resolved project_dir that itself contains a `${TOME_*}` token must NOT
    // re-enter the scanner (NFR-005 single-sweep): the resolved value emits in
    // the single pass and the literal token survives.
    let ctx = ctx_builder(tmp.path())
        .project_dir(Some(PathBuf::from("/p/${TOME_PLUGIN_DIR}")))
        .build()
        .expect("builder");
    let out = substitution::render("x=${TOME_PROJECT_DIR}", &ctx).unwrap();
    assert_eq!(out, "x=/p/${TOME_PLUGIN_DIR}");
}

#[test]
fn skill_path_resolves_to_entry_path() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("path=${TOME_SKILL_PATH}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "path=/plugins/x/skills/hello/SKILL.md");
}

#[test]
fn skill_name_resolves_to_entry_name() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("name=${TOME_SKILL_NAME}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "name=hello");
}

#[test]
fn plugin_dir_resolves_to_plugin_root_dir() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("dir=${TOME_PLUGIN_DIR}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "dir=/plugins/x");
}

#[test]
fn plugin_name_returns_unsanitised_name() {
    // FR-024: PLUGIN_NAME passes through verbatim (sanitisation is path-only).
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let ctx = ctx_builder(tmp.path())
        .plugin_name("weird name!")
        .build()
        .unwrap();
    let out = substitution::render("name=${TOME_PLUGIN_NAME}", &ctx).unwrap();
    assert_eq!(out, "name=weird name!");
}

#[test]
fn plugin_version_resolves_to_plugin_version() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("v=${TOME_PLUGIN_VERSION}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "v=1.2.3");
}

#[test]
fn plugin_data_resolves_to_override_path() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let pd = tmp.path().join("pd-explicit");
    let _g = PluginDataDirGuard::install(pd.clone());
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("p=${TOME_PLUGIN_DATA}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, format!("p={}", pd.display()));
}

#[test]
fn catalog_name_returns_unsanitised_name() {
    // FR-024: CATALOG_NAME passes through verbatim.
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let ctx = ctx_builder(tmp.path())
        .catalog_name("my-catalog!")
        .build()
        .unwrap();
    let out = substitution::render("c=${TOME_CATALOG_NAME}", &ctx).unwrap();
    assert_eq!(out, "c=my-catalog!");
}

#[test]
fn workspace_name_resolves_to_workspace_name() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("w=${TOME_WORKSPACE_NAME}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "w=global");
}

#[test]
fn workspace_data_resolves_to_override_path() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let wd = tmp.path().join("wd-explicit");
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(wd.clone());
    let out = substitution::render("p=${TOME_WORKSPACE_DATA}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, format!("p={}", wd.display()));
}

#[test]
fn date_renders_yyyy_mm_dd_against_fixed_clock() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    // 2025-03-14 00:00:00 UTC = 1741910400
    let when = OffsetDateTime::from_unix_timestamp(1_741_910_400).unwrap();
    let ctx = ctx_builder(tmp.path()).clock(when).build().unwrap();
    let out = substitution::render("d=${TOME_DATE}", &ctx).unwrap();
    assert_eq!(out, "d=2025-03-14");
}

#[test]
fn timestamp_renders_rfc3339_against_fixed_clock() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    // 1970-01-01T00:00:00Z = UNIX_EPOCH.
    let ctx = ctx_builder(tmp.path())
        .clock(OffsetDateTime::UNIX_EPOCH)
        .build()
        .unwrap();
    let out = substitution::render("t=${TOME_TIMESTAMP}", &ctx).unwrap();
    assert_eq!(out, "t=1970-01-01T00:00:00Z");
}

// --- Pass-through semantics ----------------------------------------------

#[test]
fn unknown_tome_namespace_reference_passes_through_verbatim() {
    // FR-023: unrecognised TOME_ name → match left in place verbatim.
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("x=${TOME_NOT_REAL}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "x=${TOME_NOT_REAL}");
}

#[test]
fn default_syntax_for_known_name_returns_value_not_default() {
    // FR-022: built-ins are always set, so the `:-default` form returns
    // the resolved value, never the default.
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("n=${TOME_SKILL_NAME:-fallback}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "n=hello");
}

#[test]
fn non_tome_namespace_reference_passes_through_unchanged() {
    // FR-052: references outside the Tome namespace are not matched
    // by stage 1's regex.
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render(
        "tok=${GITHUB_TOKEN} path=${PATH} claude=${CLAUDE_SESSION_ID}",
        &ctx(tmp.path()),
    )
    .unwrap();
    assert_eq!(
        out,
        "tok=${GITHUB_TOKEN} path=${PATH} claude=${CLAUDE_SESSION_ID}",
    );
}

// --- Path sanitisation (FR-024) ------------------------------------------

#[test]
fn plugin_data_path_sanitises_catalog_and_plugin_components() {
    // No override — the real ensure_plugin_data runs and substitutes
    // non-[A-Za-z0-9._-] characters with underscores.
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let ctx = ctx_builder(tmp.path())
        .catalog_name("my catalog!")
        .plugin_name("plug/in")
        .build()
        .unwrap();
    let out = substitution::render("p=${TOME_PLUGIN_DATA}", &ctx).unwrap();
    // The path is anchored under `<root>/plugin-data/<catalog>/<plugin>`.
    let expected_suffix = std::path::PathBuf::from("plugin-data")
        .join("my_catalog_")
        .join("plug_in");
    let prefix = "p=";
    let rendered_path = std::path::Path::new(&out[prefix.len()..]);
    assert!(
        rendered_path.ends_with(&expected_suffix),
        "rendered path {} did not end with {}",
        rendered_path.display(),
        expected_suffix.display(),
    );
    assert!(
        rendered_path.is_dir(),
        "lazy create_dir_all should have created {}",
        rendered_path.display(),
    );
}

// --- Clock injection (T209) ----------------------------------------------

#[test]
fn date_honours_substitution_clock_override() {
    // 2030-12-25 00:00:00 UTC = 1924387200
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let when = OffsetDateTime::from_unix_timestamp(1_924_387_200).unwrap();
    let _clock = ClockOverrideGuard::install(when);
    // The context's clock is what `${TOME_DATE}` reads; the override
    // affects `substitution::current_clock()`. Confirm both halves
    // independently.
    assert_eq!(substitution::current_clock(), when);
    let ctx = ctx_builder(tmp.path()).clock(when).build().unwrap();
    let out = substitution::render("d=${TOME_DATE}", &ctx).unwrap();
    assert_eq!(out, "d=2030-12-25");
}

#[test]
fn timestamp_honours_substitution_clock_override() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    // 2026-01-01T00:00:00Z = 1767225600
    let when = OffsetDateTime::from_unix_timestamp(1_767_225_600).unwrap();
    let _clock = ClockOverrideGuard::install(when);
    assert_eq!(substitution::current_clock(), when);
    let ctx = ctx_builder(tmp.path()).clock(when).build().unwrap();
    let out = substitution::render("t=${TOME_TIMESTAMP}", &ctx).unwrap();
    assert_eq!(out, "t=2026-01-01T00:00:00Z");
}

// --- Lazy directory creation ---------------------------------------------

#[test]
fn plugin_data_directory_exists_on_disk_after_substitution() {
    // No override — the real ensure_plugin_data runs and create_dir_all's
    // the path. Confirms FR-024's lazy-creation contract.
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let ctx = ctx(tmp.path());
    let out = substitution::render("p=${TOME_PLUGIN_DATA}", &ctx).unwrap();
    let real_path = std::path::Path::new(&out[2..]);
    assert!(
        real_path.is_dir(),
        "expected {} to exist after substitution",
        real_path.display(),
    );
}

// --- Idempotence ---------------------------------------------------------

#[test]
fn render_is_idempotent_against_same_inputs() {
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let _g = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let body = "name=${TOME_SKILL_NAME} v=${TOME_PLUGIN_VERSION} d=${TOME_DATE}";
    let when = OffsetDateTime::from_unix_timestamp(1_741_910_400).unwrap();
    let ctx = ctx_builder(tmp.path()).clock(when).build().unwrap();
    let first = substitution::render(body, &ctx).unwrap();
    let second = substitution::render(body, &ctx).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, "name=hello v=1.2.3 d=2025-03-14");
}

// --- Error path ----------------------------------------------------------

#[cfg(unix)]
#[test]
fn create_dir_all_failure_surfaces_plugin_data_dir_creation_failed() {
    use std::os::unix::fs::PermissionsExt;

    // Make the home root read-only so plugin-data/ creation under it
    // fails with EACCES. Root bypasses DAC checks — skip when running
    // as uid 0 (detected via the canonical $USER env var; CI runs as a
    // non-root user so this is sufficient).
    if std::env::var("USER").as_deref() == Ok("root") {
        eprintln!("skipping: cannot test EACCES as root");
        return;
    }
    let _lock = lock_overrides();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("ro-root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500)).unwrap();

    // Build a Paths rooted at the read-only dir. ensure_plugin_data
    // will attempt `create_dir_all(<root>/plugin-data/...)` and EACCES.
    let paths = lifecycle_paths(&root);
    let ctx = SubstitutionContext::builder()
        .catalog_name("c")
        .plugin_name("p")
        .plugin_version("1.0.0")
        .entry_name("e")
        .entry_path(PathBuf::from("/x/e.md"))
        .entry_dir(PathBuf::from("/x"))
        .plugin_root_dir(PathBuf::from("/x"))
        .workspace_name("global")
        .clock(OffsetDateTime::UNIX_EPOCH)
        .paths(paths)
        .build()
        .unwrap();

    let err = substitution::render("p=${TOME_PLUGIN_DATA}", &ctx).expect_err("should fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("failed to create plugin data dir"),
        "unexpected error message: {msg}",
    );

    // Restore perms so TempDir Drop can clean up.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
}
