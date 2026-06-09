//! US1 — `tome meta {list,add,remove}` integration tests.
//!
//! Two layers:
//! 1. **In-process** (`tome::commands::meta::run`) over a **synthetic harness
//!    registry** (`HARNESS_MODULES_OVERRIDE` only affects in-process calls — a
//!    spawned binary uses the real `SUPPORTED_HARNESSES`). These exercise
//!    selection (all-detected vs `--harness`), scope, idempotency,
//!    partial-failure forward-progress, and symlink-component refusal (88).
//! 2. **Spawned binary** with the REAL `claude-code` harness, for the
//!    byte-stable `--json` wire-shape pins.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{HarnessModulesGuard, HomeGuard, ToolEnv};
use tempfile::TempDir;

use tome::cli::{MetaAddArgs, MetaCommand, MetaListArgs, MetaRemoveArgs};
use tome::commands::meta;
use tome::error::TomeError;
use tome::harness::{BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy};
use tome::output::Mode;
use tome::workspace::ResolvedScope;

const SKILL: &str = "convert-marketplace";

// --- synthetic native-skill harness -----------------------------------------

/// A harness stub that DOES consume native skills, landing them under
/// `<root>/.<name>/skills/`. `detected` drives the all-detected probe.
struct SkillStub {
    name: &'static str,
    detected: bool,
}

impl HarnessModule for SkillStub {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        "native-skill stub"
    }
    fn detect(&self, _home: &Path) -> bool {
        self.detected
    }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("RULES.md")
    }
    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("mcp.json")
    }
    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }
    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
    fn supports_native_skills(&self) -> bool {
        true
    }
    fn skill_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(format!(".{}/skills", self.name)))
    }
    fn skill_dir_global(&self, home: &Path) -> Option<PathBuf> {
        Some(home.join(format!(".{}/skills", self.name)))
    }
}

fn stub(name: &'static str, detected: bool) -> Box<dyn HarnessModule> {
    Box::new(SkillStub { name, detected })
}

fn project_scope(project_root: &Path) -> ResolvedScope {
    let mut s = ResolvedScope::global_fallback();
    s.project_root = Some(project_root.to_path_buf());
    s
}

fn add_args(harnesses: Vec<String>, global: bool, force: bool) -> MetaAddArgs {
    MetaAddArgs {
        skill_id: SKILL.into(),
        harnesses,
        global,
        force,
    }
}

// --- in-process functional tests --------------------------------------------

#[test]
fn add_installs_into_detected_harnesses_project_scope() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![
        stub("alpha", true),
        stub("beta", true),
        stub("gamma", false), // not detected → skipped by all-detected
    ]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    meta::run(
        MetaCommand::Add(add_args(vec![], false, false)),
        &scope,
        Mode::Human,
    )
    .expect("add");

    let alpha = project
        .path()
        .join(".alpha/skills/convert-marketplace/SKILL.md");
    assert!(alpha.is_file(), "alpha got the skill");
    assert!(
        project
            .path()
            .join(".beta/skills/convert-marketplace/SKILL.md")
            .is_file(),
        "beta got the skill"
    );
    assert!(
        !project
            .path()
            .join(".gamma/skills/convert-marketplace")
            .exists(),
        "undetected gamma is skipped"
    );
    // Revision stamp landed.
    let body = fs::read_to_string(&alpha).unwrap();
    assert!(body.contains("tome_skill_revision"), "revision stamped");
    assert!(body.contains("name: convert-marketplace"));
}

#[test]
fn add_global_scope_writes_under_home() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    meta::run(
        MetaCommand::Add(add_args(vec![], true, false)),
        &scope,
        Mode::Human,
    )
    .expect("add --global");

    assert!(
        home.path()
            .join(".alpha/skills/convert-marketplace/SKILL.md")
            .is_file(),
        "global install under home"
    );
    assert!(
        !project.path().join(".alpha").exists(),
        "project dir untouched under --global"
    );
}

#[test]
fn add_explicit_harness_selects_only_named() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true), stub("beta", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    meta::run(
        MetaCommand::Add(add_args(vec!["alpha".into()], false, false)),
        &scope,
        Mode::Human,
    )
    .expect("add --harness alpha");

    assert!(
        project
            .path()
            .join(".alpha/skills/convert-marketplace")
            .is_dir()
    );
    assert!(
        !project
            .path()
            .join(".beta/skills/convert-marketplace")
            .exists(),
        "beta not targeted despite being detected"
    );
}

#[test]
fn add_is_idempotent() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    for _ in 0..2 {
        meta::run(
            MetaCommand::Add(add_args(vec![], false, false)),
            &scope,
            Mode::Human,
        )
        .expect("idempotent add");
    }
    assert!(
        project
            .path()
            .join(".alpha/skills/convert-marketplace/SKILL.md")
            .is_file()
    );
}

#[test]
fn add_no_detected_harness_is_89() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", false), stub("beta", false)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    let err = meta::run(
        MetaCommand::Add(add_args(vec![], false, false)),
        &scope,
        Mode::Human,
    )
    .expect_err("no detected harness");
    assert_eq!(err.exit_code(), 89);
    assert!(matches!(err, TomeError::NoHarnessDetected));
}

#[test]
fn add_unknown_skill_is_87() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    let err = meta::run(
        MetaCommand::Add(MetaAddArgs {
            skill_id: "no-such-skill".into(),
            harnesses: vec![],
            global: false,
            force: false,
        }),
        &scope,
        Mode::Human,
    )
    .expect_err("unknown skill");
    assert_eq!(err.exit_code(), 87);
}

#[test]
fn add_explicit_unknown_harness_is_usage_2() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    let err = meta::run(
        MetaCommand::Add(add_args(vec!["nope".into()], false, false)),
        &scope,
        Mode::Human,
    )
    .expect_err("unknown harness");
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn remove_deletes_then_is_not_present() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    meta::run(
        MetaCommand::Add(add_args(vec![], false, false)),
        &scope,
        Mode::Human,
    )
    .unwrap();
    assert!(
        project
            .path()
            .join(".alpha/skills/convert-marketplace")
            .is_dir()
    );

    meta::run(
        MetaCommand::Remove(MetaRemoveArgs {
            skill_id: SKILL.into(),
            harnesses: vec![],
            global: false,
        }),
        &scope,
        Mode::Human,
    )
    .expect("remove");
    assert!(
        !project
            .path()
            .join(".alpha/skills/convert-marketplace")
            .exists()
    );

    // Second remove is a no-op (not an error).
    meta::run(
        MetaCommand::Remove(MetaRemoveArgs {
            skill_id: SKILL.into(),
            harnesses: vec![],
            global: false,
        }),
        &scope,
        Mode::Human,
    )
    .expect("remove no-op");
}

#[cfg(unix)]
#[test]
fn add_symlinked_component_is_88_forward_progress_no_escape() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true), stub("beta", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    // Plant a symlinked component at `<project>/.alpha` → an out-of-tree dir.
    let outside = TempDir::new().unwrap();
    symlink(outside.path(), project.path().join(".alpha")).unwrap();

    let err = meta::run(
        MetaCommand::Add(add_args(vec![], false, false)),
        &scope,
        Mode::Human,
    )
    .expect_err("symlinked alpha must fail");
    // Highest-precedence failure code surfaced.
    assert_eq!(err.exit_code(), 88);
    // Forward-progress: beta (clean) still got installed.
    assert!(
        project
            .path()
            .join(".beta/skills/convert-marketplace/SKILL.md")
            .is_file(),
        "beta installed despite alpha's failure"
    );
    // No escape: nothing was written through the symlink.
    assert!(
        !outside.path().join("skills/convert-marketplace").exists(),
        "no write escaped via the symlink"
    );
}

// --- spawned-binary JSON wire-shape pins (real claude-code harness) ----------

/// Set up an isolated `$HOME` with `~/.claude/` so the real `claude-code`
/// harness is detected, plus a project dir to run from. Returns the env and
/// project tempdir.
fn real_harness_env() -> (ToolEnv, TempDir) {
    let env = ToolEnv::new();
    fs::create_dir_all(env.home_path().join(".claude")).expect("seed ~/.claude");
    let project = TempDir::new().expect("project");
    (env, project)
}

#[test]
fn add_json_wire_shape_pin() {
    let (env, project) = real_harness_env();
    let out = env
        .cmd()
        .current_dir(project.path())
        .args(["--json", "meta", "add", SKILL])
        .output()
        .expect("spawn tome meta add --json");
    assert!(
        out.status.success(),
        "exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is one JSON object");
    assert_eq!(v["skill_id"], SKILL);
    let locs = v["locations"].as_array().expect("locations array");
    assert!(!locs.is_empty(), "at least one location");
    let cc = locs
        .iter()
        .find(|l| l["harness"] == "claude-code")
        .expect("claude-code location present");
    assert_eq!(cc["scope"], "project");
    assert_eq!(cc["result"], "installed");
    assert!(cc["revision"].is_string(), "revision string present");
    assert!(
        cc["dir"].as_str().unwrap().ends_with(".claude/skills"),
        "dir is the claude skills root"
    );
    // The skill actually landed.
    assert!(
        project
            .path()
            .join(".claude/skills/convert-marketplace/SKILL.md")
            .is_file()
    );
}

#[test]
fn list_json_wire_shape_pin() {
    let (env, project) = real_harness_env();
    let out = env
        .cmd()
        .current_dir(project.path())
        .args(["--json", "meta", "list"])
        .output()
        .expect("spawn tome meta list --json");
    assert!(out.status.success());

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON object");
    let skills = v["skills"].as_array().expect("skills array");
    let cm = skills
        .iter()
        .find(|s| s["id"] == SKILL)
        .expect("convert-marketplace listed");
    assert!(cm["summary"].is_string());
    assert!(cm["revision"].is_string());
    // status: { "claude-code": { "project": "...", "global": "..." }, ... }
    let cc = &cm["status"]["claude-code"];
    assert!(cc["project"].is_string() || cc["global"].is_string());
}

// --- review-driven additions (US1 4-reviewer closeout) ----------------------

/// Run `tome --json meta <args>` from `project` and parse the stdout object.
fn meta_json(env: &ToolEnv, project: &TempDir, args: &[&str]) -> serde_json::Value {
    let mut full = vec!["--json", "meta"];
    full.extend_from_slice(args);
    let out = env
        .cmd()
        .current_dir(project.path())
        .args(&full)
        .output()
        .expect("spawn tome");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The claude-code location row in an add/remove `--json` report.
fn loc_cc(v: &serde_json::Value) -> &serde_json::Value {
    v["locations"]
        .as_array()
        .expect("locations")
        .iter()
        .find(|l| l["harness"] == "claude-code")
        .expect("claude-code location")
}

/// The claude-code project drift status in a `meta list --json` report.
fn list_status_cc_project(v: &serde_json::Value) -> String {
    v["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == SKILL)
        .unwrap()["status"]["claude-code"]["project"]
        .as_str()
        .unwrap()
        .to_string()
}

/// T-1: `--force` re-writes; an up-to-date no-`--force` add is a no-op that does
/// NOT churn the file (NFR-010 / FR-011a).
#[test]
fn add_force_rewrites_but_no_force_is_a_no_op() {
    let (env, project) = real_harness_env();
    let skill_md = project
        .path()
        .join(".claude/skills/convert-marketplace/SKILL.md");

    // 1. fresh install
    let v1 = meta_json(&env, &project, &["add", SKILL]);
    assert_eq!(loc_cc(&v1)["result"], "installed");
    let m1 = fs::metadata(&skill_md).unwrap().modified().unwrap();

    // 2. re-add without --force → already-current, file untouched (no churn).
    let v2 = meta_json(&env, &project, &["add", SKILL]);
    assert_eq!(loc_cc(&v2)["result"], "already-current");
    let m2 = fs::metadata(&skill_md).unwrap().modified().unwrap();
    assert_eq!(
        m1, m2,
        "no-force re-add must not rewrite the file (NFR-010)"
    );

    // 3. --force → re-installed (re-write).
    let v3 = meta_json(&env, &project, &["add", SKILL, "--force"]);
    assert_eq!(loc_cc(&v3)["result"], "installed");
}

/// T-5: `--global` lands under `$HOME`, reports `scope:"global"`, project untouched.
#[test]
fn add_global_scope_via_json_lands_under_home() {
    let (env, project) = real_harness_env();
    let v = meta_json(&env, &project, &["add", SKILL, "--global"]);
    let cc = loc_cc(&v);
    assert_eq!(cc["scope"], "global");
    assert!(
        cc["dir"]
            .as_str()
            .unwrap()
            .starts_with(env.home_path().to_str().unwrap()),
        "global dir is under $HOME"
    );
    assert!(
        env.home_path()
            .join(".claude/skills/convert-marketplace/SKILL.md")
            .is_file()
    );
    assert!(
        !project
            .path()
            .join(".claude/skills/convert-marketplace")
            .exists(),
        "project dir untouched under --global"
    );
}

/// T-2: a failed location is reported in the `--json` `locations` array with a
/// populated `error`, exit 88, no escape — via the real harness + a symlinked
/// `.claude` skills root.
#[cfg(unix)]
#[test]
fn add_failed_location_reported_in_json_exit_88() {
    use std::os::unix::fs::symlink;
    let (env, project) = real_harness_env();
    let outside = TempDir::new().unwrap();
    symlink(outside.path(), project.path().join(".claude")).unwrap();

    let out = env
        .cmd()
        .current_dir(project.path())
        .args(["--json", "meta", "add", SKILL])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(88), "symlinked skills root → 88");

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report on stdout");
    let cc = loc_cc(&v);
    assert_eq!(cc["result"], "failed");
    assert!(
        cc["error"].as_str().unwrap().contains("symlink"),
        "failed location carries the symlink-refusal error: {}",
        cc["error"]
    );
    assert!(
        !outside.path().join("skills/convert-marketplace").exists(),
        "no write escaped through the symlink"
    );
}

/// T-3: `meta list` surfaces drift transitions not-installed → up-to-date → stale.
#[test]
fn list_drift_transitions_not_installed_up_to_date_stale() {
    let (env, project) = real_harness_env();

    assert_eq!(
        list_status_cc_project(&meta_json(&env, &project, &["list"])),
        "not-installed"
    );

    meta_json(&env, &project, &["add", SKILL]);
    assert_eq!(
        list_status_cc_project(&meta_json(&env, &project, &["list"])),
        "up-to-date"
    );

    // Corrupt the stamped revision → stale (refreshable).
    let skill_md = project
        .path()
        .join(".claude/skills/convert-marketplace/SKILL.md");
    fs::write(
        &skill_md,
        "---\nname: convert-marketplace\ndescription: d\nmetadata:\n  tome_skill_revision: deadbeefdeadbeef\n---\nbody\n",
    )
    .unwrap();
    assert_eq!(
        list_status_cc_project(&meta_json(&env, &project, &["list"])),
        "stale"
    );
}

/// T-4: byte-stable wire-shape pin for `meta add --json` — the volatile `dir`
/// and `revision` values are normalised, then the WHOLE object (key set + order)
/// is asserted exactly, catching a field rename/reorder/add a structural check
/// would miss.
#[test]
fn add_json_byte_stable_pin_after_normalisation() {
    let (env, project) = real_harness_env();
    let mut v = meta_json(&env, &project, &["add", SKILL]);
    for loc in v["locations"].as_array_mut().unwrap() {
        loc["dir"] = serde_json::Value::String("<DIR>".into());
        loc["revision"] = serde_json::Value::String("<REV>".into());
    }
    let normalised = serde_json::to_string(&v).unwrap();
    assert_eq!(
        normalised,
        r#"{"skill_id":"convert-marketplace","locations":[{"harness":"claude-code","scope":"project","dir":"<DIR>","result":"installed","revision":"<REV>"}]}"#
    );
}

/// T-6: `meta remove` with an unknown id is 87 at the orchestration layer
/// (a separate `find()` guard from `add`).
#[test]
fn remove_unknown_skill_is_87() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    let err = meta::run(
        MetaCommand::Remove(MetaRemoveArgs {
            skill_id: "no-such-skill".into(),
            harnesses: vec![],
            global: false,
        }),
        &scope,
        Mode::Human,
    )
    .expect_err("unknown skill on remove");
    assert_eq!(err.exit_code(), 87);
}

/// T-7: the human-output emit path renders without panicking (guards the
/// `writeln!` shapes in `emit_action`/`emit_list_human`).
#[test]
fn human_output_paths_do_not_panic() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let _reg = HarnessModulesGuard::install(vec![stub("alpha", true)]);
    let project = TempDir::new().unwrap();
    let scope = project_scope(project.path());

    // add (Human), remove (Human), list (Human) all return Ok and don't panic.
    meta::run(
        MetaCommand::Add(add_args(vec![], false, false)),
        &scope,
        Mode::Human,
    )
    .unwrap();
    meta::run(
        MetaCommand::Remove(MetaRemoveArgs {
            skill_id: SKILL.into(),
            harnesses: vec![],
            global: false,
        }),
        &scope,
        Mode::Human,
    )
    .unwrap();
    meta::run(MetaCommand::List(MetaListArgs {}), &scope, Mode::Human).unwrap();
}
