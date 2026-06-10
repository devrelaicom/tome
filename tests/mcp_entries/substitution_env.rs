//! Phase 5 / US2.b — env-passthrough stage tests.
//!
//! Covers the four host-env state rows from
//! `contracts/substitution-engine.md` § Stage 2 (set/unset × default-or-not),
//! the namespace boundary (FR-033 + NFR-005: refs outside `TOME_ENV_`
//! MUST NOT match), idempotence (NFR-007), and the mixed
//! built-ins + env body case (stage ordering).
//!
//! Env-var serialisation: every test in this file mutates the host
//! environment via `std::env::set_var` / `remove_var`. Both are
//! `unsafe` on Rust 2024 and unsafe for any process with threads
//! contending the env block. Tests in cargo run on the same process,
//! so we serialise via a file-local `ENV_MUTEX` (matching the
//! `OVERRIDE_MUTEX` pattern from Phase 4 / US3.c-1
//! `tests/harness_sync_stub.rs`). `EnvVarGuard` is RAII: it snapshots
//! the previous value on install and restores on drop — survives
//! panics, no manual teardown.
//!
//! The mutex deliberately lives in this file (not promoted to
//! `tests/common/mod.rs`) because no other test surface exercises the
//! `TOME_ENV_*` namespace yet. Promote at the second consumer.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::common::{PluginDataDirGuard, WorkspaceDataDirGuard, lifecycle_paths};
use time::OffsetDateTime;
use tome::substitution::{self, SubstitutionContext, SubstitutionContextBuilder};

// --- Env serialisation discipline ----------------------------------------

static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Set `key` to `value` for the scope of the returned guard. On drop
/// the previous value (or absence) is restored. Caller MUST hold the
/// `ENV_MUTEX` for the lifetime of the guard.
struct EnvVarGuard {
    key: String,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: caller holds ENV_MUTEX; no other test mutates env.
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key: key.to_owned(),
            previous,
        }
    }

    fn unset(key: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: caller holds ENV_MUTEX; no other test mutates env.
        unsafe {
            std::env::remove_var(key);
        }
        Self {
            key: key.to_owned(),
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: ENV_MUTEX is still held by the test for the
        // lifetime of this guard.
        unsafe {
            match &self.previous {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}

// --- Context plumbing ----------------------------------------------------

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

// --- Host env state matrix (contract § Stage 2) --------------------------

#[test]
fn set_no_default_resolves_to_host_value() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T214_SET_NODEFAULT", "the-value");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("k=${TOME_ENV_T214_SET_NODEFAULT}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "k=the-value");
}

#[test]
fn set_with_default_resolves_to_host_value_default_ignored() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T214_SET_DEFAULT", "host-wins");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render(
        "k=${TOME_ENV_T214_SET_DEFAULT:-the-default}",
        &ctx(tmp.path()),
    )
    .unwrap();
    assert_eq!(out, "k=host-wins");
}

#[test]
fn unset_no_default_resolves_to_empty_string() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _lock = lock_env();
    // Explicitly unset — defends against leaked vars from previous tests.
    let _guard = EnvVarGuard::unset("TOME_ENV_T214_UNSET_NODEFAULT");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render(
        "before=${TOME_ENV_T214_UNSET_NODEFAULT}=after",
        &ctx(tmp.path()),
    )
    .unwrap();
    // Empty string substituted in place — the bracketing chars stay.
    assert_eq!(out, "before==after");
}

#[test]
fn unset_with_default_resolves_to_default() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _lock = lock_env();
    let _guard = EnvVarGuard::unset("TOME_ENV_T214_UNSET_DEFAULT");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render(
        "k=${TOME_ENV_T214_UNSET_DEFAULT:-fallback-value}",
        &ctx(tmp.path()),
    )
    .unwrap();
    assert_eq!(out, "k=fallback-value");
}

// --- Namespace boundary (FR-033 + NFR-005) -------------------------------

#[test]
fn github_token_ref_passes_through_unchanged() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // ${GITHUB_TOKEN} does NOT carry the TOME_ENV_ prefix; the Stage 2
    // regex must not match it. The body must pass through verbatim
    // even when a like-named host env var is set.
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("GITHUB_TOKEN", "secret-must-not-leak");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("tok=${GITHUB_TOKEN}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "tok=${GITHUB_TOKEN}");
    assert!(
        !out.contains("secret-must-not-leak"),
        "leaked non-TOME_ENV_ value into render output: {out}",
    );
}

#[test]
fn aws_secret_ref_passes_through_unchanged() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("AWS_SECRET_ACCESS_KEY", "AKIA...redacted...");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("s=${AWS_SECRET_ACCESS_KEY}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "s=${AWS_SECRET_ACCESS_KEY}");
    assert!(
        !out.contains("AKIA"),
        "leaked non-TOME_ENV_ value into render output: {out}",
    );
}

// --- Idempotence (NFR-007) -----------------------------------------------

#[test]
fn render_twice_yields_identical_output() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T214_IDEMPOTENT", "stable");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let body = "v=${TOME_ENV_T214_IDEMPOTENT} d=${TOME_ENV_NEVER_SET:-fallback}";
    let first = substitution::render(body, &ctx(tmp.path())).unwrap();
    let second = substitution::render(body, &ctx(tmp.path())).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, "v=stable d=fallback");
}

// --- Pipeline ordering: stage 1 + stage 2 compose ------------------------

#[test]
fn mixed_builtins_and_env_render_both_stages_correctly() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T214_MIXED", "from-env");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let body = "name=${TOME_SKILL_NAME} env=${TOME_ENV_T214_MIXED} v=${TOME_PLUGIN_VERSION}";
    let out = substitution::render(body, &ctx(tmp.path())).unwrap();
    assert_eq!(out, "name=hello env=from-env v=1.2.3");
}

#[test]
fn body_with_no_env_references_is_unchanged_by_stage_2() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Sanity check on the fast-path: a body the regex doesn't match
    // round-trips through Stage 2 unmodified. The Stage 1 substitution
    // is the only transformation visible in the output.
    let _lock = lock_env();
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let out = substitution::render("plain=${TOME_SKILL_NAME}", &ctx(tmp.path())).unwrap();
    assert_eq!(out, "plain=hello");
}

// --- No-rescan invariant (NFR-007 / FR-051) ------------------------------
//
// These tests pin the structural fix from US2.d B2: a value resolved by
// the Stage 1 (built-ins) branch that HAPPENS to contain
// `${TOME_ENV_*}` syntax is emitted verbatim into the output buffer and
// is NEVER re-scanned by the Stage 2 (env) branch. This closes the
// exfiltration vector where a hostile plugin author writes
// `"version": "${TOME_ENV_GITHUB_TOKEN}"` in plugin.json (lenient
// parser; the manifest is third-party-owned) and any skill body
// referencing `${TOME_PLUGIN_VERSION}` would otherwise leak the
// operator's `TOME_ENV_GITHUB_TOKEN` host env var into the LLM context.
//
// The combined-regex single-sweep implementation (`substitution::render`
// after US2.d) makes the invariant structurally true rather than
// relying on stage-ordering documentation: there is no Stage 2 pass
// over the Stage 1 output to re-scan.

#[test]
fn stage_1_substituted_value_containing_tome_env_syntax_is_not_rescanned_by_stage_2() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Setup: ctx.plugin_version is the literal text "${TOME_ENV_LEAKED}".
    // TOME_ENV_LEAKED is set in the host env to "SECRET".
    //
    // Render: "Version: ${TOME_PLUGIN_VERSION}" — Stage 1 resolves
    // `${TOME_PLUGIN_VERSION}` to the literal "${TOME_ENV_LEAKED}".
    //
    // Expectation: the output is "Version: ${TOME_ENV_LEAKED}" verbatim
    // — NOT "Version: SECRET". If a Stage 2 pass re-scanned the
    // resolved value, "SECRET" would leak.
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_LEAKED", "SECRET");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let ctx = ctx_builder(tmp.path())
        .plugin_version("${TOME_ENV_LEAKED}")
        .build()
        .expect("builder");

    let out = substitution::render("Version: ${TOME_PLUGIN_VERSION}", &ctx).unwrap();
    assert_eq!(
        out, "Version: ${TOME_ENV_LEAKED}",
        "no-rescan invariant violated: Stage 1 output was re-scanned by Stage 2 — \
         this is the exfiltration vector closed by US2.d B2",
    );
    assert!(
        !out.contains("SECRET"),
        "TOME_ENV_LEAKED leaked into output: {out:?}",
    );
}

#[test]
fn stage_1_skill_name_containing_tome_env_syntax_is_not_rescanned() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Mirror of the previous test against a different Stage 1 built-in
    // (entry_name → ${TOME_SKILL_NAME}) — same invariant.
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_HIDDEN", "exfil-target");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let ctx = ctx_builder(tmp.path())
        .entry_name("${TOME_ENV_HIDDEN}")
        .build()
        .expect("builder");

    let out = substitution::render("Skill is ${TOME_SKILL_NAME}", &ctx).unwrap();
    assert_eq!(out, "Skill is ${TOME_ENV_HIDDEN}");
    assert!(
        !out.contains("exfil-target"),
        "TOME_ENV_HIDDEN leaked into output: {out:?}",
    );
}

#[test]
fn stage_2_default_containing_builtin_syntax_is_not_rescanned() {
    let _lock = crate::common::SUBSTITUTION_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // A `${TOME_ENV_FOO:-${TOME_SKILL_NAME}}` reference is a single
    // regex match — the default capture group is the literal text
    // `${TOME_SKILL_NAME}`. The env branch resolves the value to that
    // literal default; it is NOT re-scanned by Stage 1.
    //
    // This is the dual of the previous tests: text emitted by the Stage
    // 2 branch must not be re-scanned by Stage 1 either. The
    // single-sweep design makes both directions of the invariant true
    // by construction.
    let _lock = lock_env();
    let _guard = EnvVarGuard::unset("TOME_ENV_DEFAULT_RESCAN_PROBE");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    // NB: regex pattern uses `(?::-(.*?))?` (lazy), so the inner `}`
    // closes the OUTER reference: the default text is
    // "${TOME_SKILL_NAME" (without the closing `}`). That's a textual
    // peculiarity of the contract pattern, not a no-rescan violation —
    // what matters here is that NO part of the resolved-default text
    // gets sent back through the scanner to resolve `TOME_SKILL_NAME`
    // against the context's `entry_name` ("hello").
    let body = "X=${TOME_ENV_DEFAULT_RESCAN_PROBE:-${TOME_SKILL_NAME}}";
    let out = substitution::render(body, &ctx(tmp.path())).unwrap();
    // The trailing `}` after the lazy default match is preserved
    // verbatim; the literal `${TOME_SKILL_NAME` text is NOT resolved
    // against the context.
    assert!(
        out.contains("${TOME_SKILL_NAME"),
        "Stage 2 default text was re-scanned by Stage 1: {out:?}",
    );
    assert!(
        !out.contains("=hello"),
        "context entry_name leaked through Stage 2 default text: {out:?}",
    );
}
