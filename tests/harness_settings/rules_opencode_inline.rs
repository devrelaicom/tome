//! Regression test for F-RULES-OPENCODE (FR-008): when several harnesses
//! SHARE one rules file and at least one of them is NOT include-capable
//! (OpenCode, `BlockBodyStyle::Inline`), the body written into that shared
//! file must be the INLINE verbatim `<project>/.tome/RULES.md` contents — not
//! a literal `@.tome/RULES.md` include directive that OpenCode cannot resolve.
//!
//! ## The bug
//!
//! `compute_rules_body` switched on a SINGLE snapshot's `block_body_style`.
//! Codex / Gemini / OpenCode all target `<project>/AGENTS.md`; the first
//! snapshot to claim the path "owns" the write. In lexical registry order
//! Codex (`AtInclude`) is processed before OpenCode (`Inline`), so the shared
//! `AGENTS.md` received `@.tome/RULES.md` as the block body. OpenCode reads
//! that line as PROSE and never sees Tome's actual rules.
//!
//! ## The fix (mirrors `reconcile/guardrails.rs`'s union-across-sharers)
//!
//! The body style for a shared rules path is the lowest common denominator
//! across every LIVE sharer of that path: if ANY live sharer requires
//! `Inline`, the inline body is written. Include-capable harnesses resolve an
//! inline body correctly (inline is the LCD), so they stay green.
//!
//! These tests drive the REAL harness modules (Codex, Gemini, OpenCode),
//! because those three genuinely share `AGENTS.md` and exercise the production
//! grouping. `block_body_style()` is the source of truth — no harness name is
//! hard-coded here. The writer-side cases scope the registry to two modules via
//! the `HARNESS_MODULES_OVERRIDE` seam; the doctor-side regression cases at the
//! bottom run against the unmodified `SUPPORTED_HARNESSES` (the effective list
//! from the project marker — not the override — drives which sharers are live).

use std::path::PathBuf;

use crate::common::{
    HarnessModulesGuard, ToolEnv, fabricate_all_registry_models, paths_for, seed_workspace,
};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

/// The verbatim rules content seeded into `<project>/.tome/RULES.md`. A
/// multi-line body distinguishes the inline write (full content) from the
/// one-line `@.tome/RULES.md` include directive unambiguously.
const RULES_BODY: &str = "# Project rules\n\nAlways write tests first.\nNever use `--no-verify`.\n";

/// The include directive an include-capable harness would emit as the block
/// body. Its presence as a standalone block line in OpenCode's file is the
/// bug signature.
const INCLUDE_LINE: &str = "@.tome/RULES.md";

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    /// Build a bound project whose marker enrols `harnesses` and seed a real
    /// `<project>/.tome/RULES.md` so the inline body is non-empty.
    fn build(workspace_name: &str, harnesses_toml: &str) -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");

        let marker_dir = project.join(".tome");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        std::fs::write(
            marker_dir.join("config.toml"),
            format!("workspace = \"{workspace_name}\"\n{harnesses_toml}\n"),
        )
        .expect("write marker config");

        // Seed the project-marker RULES.md the inline body is copied from.
        std::fs::write(marker_dir.join("RULES.md"), RULES_BODY).expect("write RULES.md");

        Fixture {
            _home: env.home,
            paths,
            project,
            workspace,
        }
    }

    fn deps(&self) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: self._home.path(),
            workspace_name: &self.workspace,
            force: false,
            only_harness: None,
        }
    }
}

/// Read the body of the single Tome block in `path`, panicking with the file
/// contents on any malformed/missing-block condition. Uses the production
/// parser so the assertion sees exactly what a harness would parse back.
fn block_body(path: &std::path::Path) -> String {
    let contents =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let block = tome::harness::rules_file::parse_block(&contents)
        .unwrap_or_else(|e| panic!("parse block in {}: {e:?}", path.display()))
        .unwrap_or_else(|| panic!("no Tome block in {}:\n{contents}", path.display()));
    block.body
}

/// Core assertion shared by both cases: OpenCode shares `AGENTS.md` with an
/// include-capable harness; after sync the shared block must hold the INLINE
/// rules — never the bare include directive.
fn assert_shared_agents_md_is_inline(harnesses_toml: &str) {
    let fx = Fixture::build("test-workspace", harnesses_toml);
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let agents_md = fx.project.join("AGENTS.md");
    assert!(
        agents_md.is_file(),
        "the shared AGENTS.md must exist after sync"
    );
    let body = block_body(&agents_md);

    // The block must be the verbatim inline rules — what EVERY sharer,
    // including OpenCode, can read directly.
    assert_eq!(
        body.trim_end(),
        RULES_BODY.trim_end(),
        "shared AGENTS.md must carry the INLINE rules body so OpenCode receives \
         Tome's rules; got:\n{body}",
    );
    // And it must NOT be the include directive OpenCode cannot resolve. A
    // standalone `@.tome/RULES.md` line is the bug signature.
    assert!(
        !body.lines().any(|l| l.trim() == INCLUDE_LINE),
        "shared AGENTS.md must NOT carry the literal `{INCLUDE_LINE}` include line \
         (OpenCode reads it as prose); got:\n{body}",
    );
}

// ---------------------------------------------------------------------------
// Case 1: OpenCode + Codex share AGENTS.md → inline body for both.
// ---------------------------------------------------------------------------

#[test]
fn opencode_codex_shared_agents_md_is_inline() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::codex::CODEX),
        Box::new(tome::harness::opencode::OPENCODE),
    ]);

    // Codex is include-capable (AtInclude); OpenCode requires Inline. They
    // share <project>/AGENTS.md.
    assert_shared_agents_md_is_inline("harnesses = [\"codex\", \"opencode\"]");
}

// ---------------------------------------------------------------------------
// Case 2: OpenCode + Gemini share AGENTS.md → inline body for both.
// ---------------------------------------------------------------------------

#[test]
fn opencode_gemini_shared_agents_md_is_inline() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::gemini::GEMINI),
        Box::new(tome::harness::opencode::OPENCODE),
    ]);

    // Gemini falls back to AGENTS.md (none of its candidates pre-exist), so it
    // shares the file with OpenCode.
    assert_shared_agents_md_is_inline("harnesses = [\"gemini\", \"opencode\"]");
}

// ---------------------------------------------------------------------------
// Control: a rules file used ONLY by include-capable harness(es) keeps the
// `@.tome/RULES.md` AtInclude body — the LCD is unchanged when no sharer
// needs Inline.
// ---------------------------------------------------------------------------

#[test]
fn include_only_group_keeps_at_include() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::codex::CODEX),
        Box::new(tome::harness::gemini::GEMINI),
    ]);

    let fx = Fixture::build("test-workspace", "harnesses = [\"codex\", \"gemini\"]");
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let agents_md = fx.project.join("AGENTS.md");
    let body = block_body(&agents_md);
    assert_eq!(
        body.trim_end(),
        INCLUDE_LINE,
        "an include-only group must keep the `@.tome/RULES.md` directive; got:\n{body}",
    );
}

// ---------------------------------------------------------------------------
// Cross-module regression: `tome doctor` must mirror the writer's
// union-across-sharers when projecting the expected rules body.
//
// The writer makes the body style a GROUP decision (Inline if ANY live sharer
// needs it), so a shared `AGENTS.md` between an include-capable harness (Codex)
// and OpenCode receives the INLINE body. If doctor's projection still computed
// the expected body from each module's OWN `block_body_style()`, the
// include-capable sharer would expect `@.tome/RULES.md`, read the inline body
// on disk, and report a PERMANENT false-positive `Drift` (`--fix` re-runs the
// same writer, so it never converges). These tests drive the REAL production
// registry (no `HARNESS_MODULES_OVERRIDE`): both sharers must read back `Ok`.
// ---------------------------------------------------------------------------

/// Run `sync_project` then `doctor::assemble_report` against the same bound
/// project, and assert every per-harness rules subsystem reports `Ok` (never
/// `Drift`). `expect_harnesses` is asserted to be exactly the effective rules
/// set doctor reports on.
fn assert_doctor_sees_no_drift_after_sync(harnesses_toml: &str, expect_harnesses: &[&str]) {
    use tome::doctor::{self, SubsystemHealth};
    use tome::workspace::{ResolvedScope, Scope, ScopeSource};

    // No override is installed here — doctor + sync resolve against the real
    // `SUPPORTED_HARNESSES`. We still hold `crate::common::HARNESS_OVERRIDE_MUTEX` so an
    // override-installing test in this file can't clobber the registry view
    // mid-run (the slot is process-global).
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let fx = Fixture::build("test-workspace", harnesses_toml);
    // `assemble_report` reads model state; fabricate so the report is clean and
    // the only signal under test is the per-harness rules health.
    fabricate_all_registry_models(&fx.paths);

    // Writer side: lands the (inline, because OpenCode shares the path) body.
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // Doctor side: read-only projection over the same on-disk state.
    let scope = ResolvedScope {
        scope: Scope(fx.workspace.clone()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(fx.project.clone()),
        overridden_project_marker: None,
    };
    let report = doctor::assemble_report(&scope, &fx.paths, fx._home.path(), false)
        .expect("assemble doctor report");

    let mut names: Vec<&str> = report
        .harness_rules
        .iter()
        .map(|h| h.harness.as_str())
        .collect();
    names.sort_unstable();
    let mut expected: Vec<&str> = expect_harnesses.to_vec();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "doctor should report rules health for exactly the effective harnesses; got {:#?}",
        report.harness_rules,
    );

    for entry in &report.harness_rules {
        assert_eq!(
            entry.health,
            SubsystemHealth::Ok,
            "doctor rules health for `{}` must be Ok — the writer landed the GROUP's \
             inline body into the shared file, so doctor's projection must expect inline \
             too (no false-positive Drift). Full rules report:\n{:#?}",
            entry.harness,
            report.harness_rules,
        );
    }
}

#[test]
fn doctor_no_drift_for_opencode_codex_shared_agents_md() {
    // Codex (AtInclude) shares AGENTS.md with OpenCode (Inline). Before the
    // doctor-side fix, Codex's check expected `@.tome/RULES.md` and saw the
    // inline body → Drift. After the fix it resolves the group style (Inline).
    assert_doctor_sees_no_drift_after_sync(
        "harnesses = [\"codex\", \"opencode\"]",
        &["codex", "opencode"],
    );
}

#[test]
fn doctor_no_drift_for_opencode_gemini_shared_agents_md() {
    // Gemini (AtInclude, AGENTS.md fallback) shares with OpenCode (Inline).
    assert_doctor_sees_no_drift_after_sync(
        "harnesses = [\"gemini\", \"opencode\"]",
        &["gemini", "opencode"],
    );
}

#[test]
fn doctor_include_only_group_stays_ok() {
    // Control: an include-only group (Codex + Gemini) keeps the AtInclude body
    // on both sides — writer and doctor agree on `@.tome/RULES.md`, no Drift.
    assert_doctor_sees_no_drift_after_sync(
        "harnesses = [\"codex\", \"gemini\"]",
        &["codex", "gemini"],
    );
}
