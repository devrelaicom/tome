//! Phase 11 / US6 (T081): multi-harness selection ergonomics for
//! `tome harness use` — variadic names, all-detected default, `--all`,
//! alias+dedupe collapse, and forward-progress.
//!
//! The selection-shape tests (explicit / `--all` / alias / dedupe) drive the
//! REAL registry at GLOBAL scope (no project marker → no sync runs, only the
//! name resolution + settings write), so they assert the EMITTED report's
//! per-harness result set directly. The detected-default and forward-progress
//! tests install a synthetic stub registry (so detection + a per-harness
//! failure are deterministic) and drive `run_inner` against a project marker.

use crate::common::{HarnessModulesGuard, HomeGuard, ToolEnv, paths_for, seed_workspace};
use tome::cli::{HarnessScopeArg, HarnessUseArgs};
use tome::commands::harness::use_;
use tome::commands::harness::use_::{HarnessUseOutcome, HarnessUseReport, HarnessUseResult};
use tome::harness::StubHarness;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse("global").unwrap()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
    }
}

fn project_scope(workspace: &str, project_root: std::path::PathBuf) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(workspace).unwrap()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project_root),
        overridden_project_marker: None,
    }
}

/// Collect the canonical names of the successful results, in report order.
fn ok_names(report: &HarnessUseReport) -> Vec<String> {
    report
        .results
        .iter()
        .filter_map(|r| match r {
            HarnessUseResult::Ok(o) => Some(o.name.clone()),
            HarnessUseResult::Failed { .. } => None,
        })
        .collect()
}

/// Explicit `a b c` → exactly those three, in order, all Ok. Driven against the
/// REAL registry at global scope (no sync), so this exercises the real
/// `lookup` validation + selection ordering.
#[test]
fn explicit_names_select_exactly_those() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec![
            "claude-code".to_string(),
            "codex".to_string(),
            "cursor".to_string(),
        ],
        all: false,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let report_ri = use_::run_inner(args, &global_scope(), &paths).expect("use ok");
    let (report, err) = (report_ri.report, report_ri.first_error);
    assert!(err.is_none());
    assert_eq!(report.selection, "explicit");
    assert_eq!(ok_names(&report), vec!["claude-code", "codex", "cursor"]);

    // Task 2: global scope now writes to config.toml [harness].enabled.
    let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
    for h in ["claude-code", "codex", "cursor"] {
        assert!(body.contains(h), "config must include {h}: {body}");
    }
}

/// `--all` → every SUPPORTED harness, NEVER the opt-in `generic` / `generic-op`
/// targets. Driven against the real registry at global scope.
#[test]
fn all_flag_selects_every_supported_excluding_generics() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec![],
        all: true,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let report_ri = use_::run_inner(args, &global_scope(), &paths).expect("use --all ok");
    let (report, err) = (report_ri.report, report_ri.first_error);
    assert!(err.is_none());
    assert_eq!(report.selection, "all");

    let names = ok_names(&report);
    // Every SUPPORTED_HARNESSES module is present.
    for m in tome::harness::SUPPORTED_HARNESSES {
        assert!(
            names.contains(&m.name().to_string()),
            "--all must include {}",
            m.name(),
        );
    }
    // The opt-in generics are NEVER in --all.
    assert!(
        !names.contains(&"generic".to_string()),
        "generic must be excluded from --all",
    );
    assert!(
        !names.contains(&"generic-op".to_string()),
        "generic-op must be excluded from --all",
    );
    // goose IS supported (detectable), so --all includes it.
    assert!(names.contains(&"goose".to_string()), "goose is in --all");

    // F3a: the report is not the only surface — the config file must have been
    // written too. Every SUPPORTED harness name is persisted, and NEITHER opt-in
    // generic appears (the write side mirrors the selection).
    // Task 2: global scope now writes to config.toml [harness].enabled.
    let body = std::fs::read_to_string(&paths.global_config_file)
        .expect("--all must write the global config file");
    for m in tome::harness::SUPPORTED_HARNESSES {
        assert!(
            body.contains(m.name()),
            "config file must persist {}: {body}",
            m.name(),
        );
    }
    // Word-boundary check: `generic` is a substring of `generic-op`, so assert on
    // the quoted TOML array element form (`"generic"`) to avoid a false positive
    // from a (non-existent) entry, and on `"generic-op"` separately.
    assert!(
        !body.contains("\"generic\""),
        "generic must NOT be persisted by --all: {body}",
    );
    assert!(
        !body.contains("\"generic-op\""),
        "generic-op must NOT be persisted by --all: {body}",
    );
}

/// M5 — `use antigravity-cli gemini` collapses to a SINGLE `gemini`
/// configuration pass (the alias resolves BEFORE dedupe), and the settings
/// file gains `gemini` exactly once (not double-written).
#[test]
fn alias_and_canonical_collapse_to_single_pass() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        // antigravity-cli is an alias of gemini; naming both must NOT configure
        // gemini twice.
        names: vec!["antigravity-cli".to_string(), "gemini".to_string()],
        all: false,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let report_ri = use_::run_inner(args, &global_scope(), &paths).expect("use ok");
    let (report, err) = (report_ri.report, report_ri.first_error);
    assert!(err.is_none());
    // ONE result, named by the canonical `gemini`.
    assert_eq!(
        ok_names(&report),
        vec!["gemini"],
        "antigravity-cli + gemini must collapse to one gemini pass",
    );

    // The config array contains exactly one `gemini` entry (no double-write).
    // Task 2: global scope now writes to config.toml [harness].enabled.
    let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert_eq!(
        body.matches("gemini").count(),
        1,
        "gemini must appear exactly once in config: {body}",
    );
}

/// A repeated/duplicate name collapses to ONE pass.
#[test]
fn duplicate_name_collapses_to_single_pass() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec!["cursor".to_string(), "cursor".to_string()],
        all: false,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let report_ri = use_::run_inner(args, &global_scope(), &paths).expect("use ok");
    let (report, err) = (report_ri.report, report_ri.first_error);
    assert!(err.is_none());
    assert_eq!(
        ok_names(&report),
        vec!["cursor"],
        "cursor cursor → one pass"
    );
}

/// No names + no `--all` → the DETECTED set. Install two stubs, one detecting
/// (`det`) and one not (`undet`); the default selection must be exactly `det`.
#[test]
fn no_args_selects_detected_set_only() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(StubHarness::default().with_name("det").with_detect(true)),
        Box::new(StubHarness::default().with_name("undet").with_detect(false)),
    ]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec![],
        all: false,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let report_ri = use_::run_inner(args, &global_scope(), &paths).expect("use ok");
    let (report, err) = (report_ri.report, report_ri.first_error);
    assert!(err.is_none());
    assert_eq!(report.selection, "detected");
    assert_eq!(
        ok_names(&report),
        vec!["det"],
        "only the detecting stub is in the default selection",
    );
}

/// No names + no `--all` in a project with NO detected harness → a clear
/// "nothing detected" outcome: an empty report (selection `detected`, zero
/// results), NOT a crash or a silent error.
#[test]
fn no_args_no_detected_harness_yields_empty_detected_report() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // A single stub that NEVER detects.
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_name("undet").with_detect(false),
    )]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec![],
        all: false,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let report_ri = use_::run_inner(args, &global_scope(), &paths).expect("use ok");
    let (report, err) = (report_ri.report, report_ri.first_error);
    assert!(err.is_none(), "no-detected is not an error");
    assert_eq!(report.selection, "detected");
    assert!(
        report.results.is_empty(),
        "no harness detected → empty result set",
    );
}

/// Forward-progress: the per-harness loop does NOT abort on the first failure —
/// EVERY selected harness is attempted (its result recorded), and the FIRST
/// failure's exit code is surfaced at the end. A healthy harness configured
/// ALONE still succeeds (proving the failure is the harness's, not the loop's).
///
/// Realism note: in `tome harness use`, configuring one harness re-runs a FULL
/// reconcile of the whole effective set, so a harness with a broken sink
/// (`stub_fail`, refused MCP write) makes BOTH passes error. That is exactly
/// the property under test — the loop keeps going past `stub_fail`'s first
/// failure to attempt the next harness (two results, not an early abort) and
/// surfaces the first error. `stub_ok` configured ALONE in a clean project
/// demonstrates the healthy path succeeds.
#[cfg(unix)]
#[test]
fn forward_progress_attempts_all_and_surfaces_first_error() {
    use std::os::unix::fs::symlink;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(StubHarness::default().with_name("stub_ok")),
        Box::new(
            StubHarness::default()
                .with_name("stub_fail")
                .with_failing_mcp(),
        ),
    ]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    // ---- (a) A healthy harness configured ALONE succeeds. ----
    let ok_project = env.home_path().join("ok_project");
    let ok_marker = ok_project.join(".tome");
    std::fs::create_dir_all(&ok_marker).unwrap();
    std::fs::write(ok_marker.join("config.toml"), "workspace = \"demo\"\n").unwrap();
    std::fs::write(ok_marker.join("RULES.md"), "# rules\n").unwrap();
    let _home = HomeGuard::install(env.home_path());

    let ok_args = HarnessUseArgs {
        names: vec!["stub_ok".to_string()],
        all: false,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let ok_report_ri = use_::run_inner(ok_args, &project_scope("demo", ok_project.clone()), &paths)
        .expect("selection resolves");
    let (ok_report, ok_err) = (ok_report_ri.report, ok_report_ri.first_error);
    assert!(ok_err.is_none(), "a healthy harness alone must succeed");
    assert_eq!(ok_names(&ok_report), vec!["stub_ok"]);

    // ---- (b) Selecting a healthy + a broken harness: BOTH attempted, the
    //          loop does not abort, and the first error surfaces. ----
    let project = env.home_path().join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"demo\"\n").unwrap();
    std::fs::write(marker_dir.join("RULES.md"), "# rules\n").unwrap();
    // Make `stub_fail`'s MCP PARENT a SYMLINK so the read+write symlink guard
    // refuses it → the reconcile errors (exit 7) whenever `stub_fail` is in the
    // effective set.
    let real = env.home_path().join("elsewhere");
    std::fs::create_dir_all(&real).unwrap();
    symlink(&real, project.join("stub_fail_MCP_BROKEN")).unwrap();

    let args = HarnessUseArgs {
        names: vec!["stub_ok".to_string(), "stub_fail".to_string()],
        all: false,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let report_ri = use_::run_inner(args, &project_scope("demo", project.clone()), &paths)
        .expect("selection resolves");
    let (report, err) = (report_ri.report, report_ri.first_error);

    // Forward-progress: BOTH harnesses were attempted — the loop did not abort
    // after the first failure (it produced a result for each selected harness).
    assert_eq!(
        report.results.len(),
        2,
        "both harnesses must be attempted (no early abort); report: {report:?}",
    );
    // `stub_fail` is recorded as a Failed result with its exit code.
    let stub_fail_code = report.results.iter().find_map(|r| match r {
        HarnessUseResult::Failed {
            name, exit_code, ..
        } if name == "stub_fail" => Some(*exit_code),
        _ => None,
    });
    assert_eq!(stub_fail_code, Some(7), "stub_fail recorded with exit 7");
    // The FIRST failure's exit code is surfaced for the process exit.
    let err = err.expect("a failure must surface an error");
    assert_eq!(err.exit_code(), 7, "first failure's exit code; got {err:?}");

    // F3b: assert the HEALTHY co-selected harness's result variant HONESTLY.
    //
    // OBSERVED behaviour (verified, NOT assumed): the healthy `stub_ok` pass
    // ALSO FAILS with exit 7. `configure_one` builds its `SyncDeps` with
    // `only_harness = None`, so EACH harness's `sync_project` walks the WHOLE
    // registered module set and runs the write-OR-cleanup decision for every
    // module — INCLUDING the broken `stub_fail`. For `stub_ok`'s pass `stub_fail`
    // is not in the effective list, so it takes the CLEANUP branch
    // (`clean_mcp_for_harness` → `mcp_config::read_entry`), and that read hits
    // the symlinked `stub_fail_MCP_BROKEN/mcp.json` parent → the symlink guard
    // refuses → exit 7. So a broken PEER anywhere in the registry poisons the
    // healthy harness's full-reconcile pass too; the per-`use` reconcile is NOT
    // scoped to the harness being configured (that scoping only exists for
    // `tome sync --harness`, via `only_harness`).
    //
    // This is consistent with the documented guarantee — "all attempted + first
    // error surfaced" — but NOT with a stronger "only the broken harness fails".
    // Pinning the real `Failed` variant here keeps the test honest and catches a
    // future change that would scope `use`'s reconcile per-harness.
    let stub_ok_result = report
        .results
        .iter()
        .find(|r| match r {
            HarnessUseResult::Ok(o) => o.name == "stub_ok",
            HarnessUseResult::Failed { name, .. } => name == "stub_ok",
        })
        .expect("stub_ok must have a result");
    assert!(
        matches!(stub_ok_result, HarnessUseResult::Failed { name, exit_code, .. } if name == "stub_ok" && *exit_code == 7),
        "the full-reconcile-per-harness coupling makes the healthy stub_ok pass \
         ALSO fail (exit 7) — its reconcile walks stub_fail's broken MCP cleanup \
         read; got: {stub_ok_result:?}",
    );
}

// ---------------------------------------------------------------------------
// F3c (US6 closeout): the no-arg empty-detected HUMAN message. `run_inner`
// returns the report but BYPASSES `emit_human`, so the distinct "No harness
// detected…" string was never exercised. Drive the REAL `tome` binary in a
// fresh isolated `$HOME` (no well-known harness dir exists → nothing detected)
// and assert the human stdout carries the actionable message — covering the
// `run → emit_human → "detected" empty` branch.
// ---------------------------------------------------------------------------

#[test]
fn no_arg_empty_detected_prints_no_harness_message() {
    // No HARNESS_OVERRIDE_MUTEX: the spawned binary uses the REAL registry, not
    // the lib-local process override. `ToolEnv::cmd()` isolates `$HOME` to a
    // fresh temp dir, so NO harness is detected.
    let env = ToolEnv::new();

    let output = env
        .cmd()
        .args(["harness", "use", "--scope", "global"])
        .output()
        .expect("run tome harness use");

    assert!(
        output.status.success(),
        "no-detected is a clean exit, not an error; stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No harness detected"),
        "the human output must carry the distinct no-harness message; got: {stdout}",
    );
}

// ---------------------------------------------------------------------------
// F2 (US6 closeout): byte-stable `--json` pin for the `tome harness use`
// envelope. The single→multi widening (`HarnessUseReport{selection,results[]}`)
// landed without a wire-shape pin; this locks the exact JSON bytes for a
// representative one-ok + one-failed report so the envelope cannot drift
// silently. Built directly from the public types (no I/O), serialised the same
// way `run`'s `--json` path does (`serde_json::to_string`).
// ---------------------------------------------------------------------------

#[test]
fn harness_use_report_json_wire_shape_is_byte_stable() {
    let report = HarnessUseReport {
        selection: "explicit",
        results: vec![
            HarnessUseResult::Ok(HarnessUseOutcome {
                scope: "global".to_string(),
                name: "cursor".to_string(),
                settings_path: std::path::PathBuf::from("/home/u/.tome/settings.toml"),
                list_changed: true,
                sync_ran: false,
                // None → omitted (skip_serializing_if), keeping the common
                // fully-automatic-MCP shape pinned.
                mcp_notice: None,
            }),
            HarnessUseResult::Failed {
                name: "codex".to_string(),
                error: "boom".to_string(),
                exit_code: 7,
            },
        ],
    };

    let json = serde_json::to_string(&report).expect("serialize report");
    assert_eq!(
        json,
        r#"{"selection":"explicit","results":[{"status":"ok","scope":"global","name":"cursor","settings_path":"/home/u/.tome/settings.toml","list_changed":true,"sync_ran":false},{"status":"failed","name":"codex","error":"boom","exit_code":7}]}"#,
        "the `tome harness use --json` envelope wire shape must stay byte-stable; got: {json}",
    );
}

// ---------------------------------------------------------------------------
// clap-layer surface: `use --all foo` is a conflict; variadic names + repeated
// `sync --harness` parse into the expected vecs.
// ---------------------------------------------------------------------------

/// `tome harness use --all foo` is a clap conflict (`--all` conflicts with the
/// variadic names) — it must fail to parse.
#[test]
fn use_all_with_explicit_name_is_a_clap_conflict() {
    use clap::Parser;
    use tome::cli::Cli;

    let res = Cli::try_parse_from(["tome", "harness", "use", "--all", "cursor"]);
    assert!(
        res.is_err(),
        "`use --all <name>` must be rejected as a conflict",
    );
}

/// `tome harness use a b c` parses into the variadic `names` vec; `--all`
/// defaults false.
#[test]
fn use_parses_variadic_names() {
    use clap::Parser;
    use tome::cli::{Cli, Command, HarnessCommand};

    let cli =
        Cli::try_parse_from(["tome", "harness", "use", "claude-code", "codex"]).expect("parse");
    let Command::Harness(h) = cli.command else {
        panic!("expected the harness subcommand");
    };
    let Some(HarnessCommand::Use(args)) = h.command else {
        panic!("expected the use subcommand");
    };
    assert_eq!(args.names, vec!["claude-code", "codex"]);
    assert!(!args.all);
}

/// `tome sync --harness a --harness b` parses into a two-element `harness` vec.
#[test]
fn sync_parses_repeated_harness_flag() {
    use clap::Parser;
    use tome::cli::{Cli, Command};

    let cli = Cli::try_parse_from(["tome", "sync", "--harness", "cursor", "--harness", "codex"])
        .expect("parse");
    let Command::Sync(args) = cli.command else {
        panic!("expected the sync subcommand");
    };
    assert_eq!(args.harness, vec!["cursor", "codex"]);
}

// ---------------------------------------------------------------------------
// Issue #306: `--include-opt-in` behaviour + the opt-in-skip notice.
// ---------------------------------------------------------------------------

/// clap wiring: `--include-opt-in` REQUIRES `--all` AND conflicts with explicit
/// names. Passing it alone OR with names is a LOUD usage error — never a silent
/// no-op (the review MINOR: `requires = "all"` alone is skipped-not-enforced when
/// names are present because `names` already conflicts with `--all`, so the flag
/// would parse and do nothing without `conflicts_with = "names"`).
#[test]
fn include_opt_in_requires_all() {
    use clap::Parser;
    use tome::cli::Cli;

    // (a) Alone → error (requires --all).
    assert!(
        Cli::try_parse_from(["tome", "harness", "use", "--include-opt-in"]).is_err(),
        "`--include-opt-in` without `--all` must be a usage error",
    );
    // (b) With an explicit name but no --all → error (conflicts_with = names).
    // Without this rule the flag would be a silent no-op (parses, ignored).
    assert!(
        Cli::try_parse_from(["tome", "harness", "use", "--include-opt-in", "cursor"]).is_err(),
        "`--include-opt-in <name>` (no --all) must be a usage error, not a silent no-op",
    );
    assert!(
        Cli::try_parse_from(["tome", "harness", "use", "cursor", "--include-opt-in"]).is_err(),
        "name-then-flag ordering must also error (conflicts_with = names)",
    );
    // (c) With --all → parses, and both flags land.
    let cli = Cli::try_parse_from(["tome", "harness", "use", "--all", "--include-opt-in"])
        .expect("--all --include-opt-in must parse");
    let tome::cli::Command::Harness(h) = cli.command else {
        panic!("expected the harness subcommand");
    };
    let Some(tome::cli::HarnessCommand::Use(args)) = h.command else {
        panic!("expected the use subcommand");
    };
    assert!(args.all && args.include_opt_in);
    assert!(args.names.is_empty(), "no names in the --all path");
}

/// `--all --include-opt-in` selects the `--all` set PLUS the opt-in targets
/// (`generic` / `generic-op`), and the settings file DURABLY records them. The
/// skipped-opt-in note list is empty (so `run` emits no note). Driven against
/// the REAL registry at global scope (no sync).
#[test]
fn all_include_opt_in_configures_generics() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec![],
        all: true,
        include_opt_in: true,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let ri = use_::run_inner(args, &global_scope(), &paths).expect("use --all --include-opt-in ok");
    assert!(ri.first_error.is_none());
    assert_eq!(ri.report.selection, "all");
    // With --include-opt-in nothing is skipped → no note.
    assert!(
        ri.skipped_opt_in.is_empty(),
        "no opt-in target is skipped when --include-opt-in is given; got {:?}",
        ri.skipped_opt_in,
    );

    let names = ok_names(&ri.report);
    // Both opt-in targets ARE selected now.
    for m in tome::harness::OPT_IN_TARGETS {
        assert!(
            names.contains(&m.name().to_string()),
            "--all --include-opt-in must include the opt-in target {}",
            m.name(),
        );
    }

    // DURABLE effect: the opt-in targets are persisted to the global config's
    // `[harness].enabled` array (quoted TOML element form, since `generic` is a
    // substring of `generic-op`).
    let body = std::fs::read_to_string(&paths.global_config_file)
        .expect("--all --include-opt-in must write the global config file");
    assert!(
        body.contains("\"generic\""),
        "generic must be persisted by --all --include-opt-in: {body}",
    );
    assert!(
        body.contains("\"generic-op\""),
        "generic-op must be persisted by --all --include-opt-in: {body}",
    );
}

/// Plain `--all` (no `--include-opt-in`) does NOT select the opt-in targets, and
/// `run_inner` reports them in `skipped_opt_in` (the note's source). This is the
/// SSOT view; the binary test below proves the stderr note itself.
#[test]
fn all_without_include_opt_in_reports_skipped_targets() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec![],
        all: true,
        include_opt_in: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let ri = use_::run_inner(args, &global_scope(), &paths).expect("use --all ok");
    assert!(ri.first_error.is_none());

    // The opt-in targets are reported as skipped (drives the #306 note).
    let expected: Vec<String> = tome::harness::OPT_IN_TARGETS
        .iter()
        .map(|m| m.name().to_string())
        .collect();
    assert_eq!(
        ri.skipped_opt_in, expected,
        "plain --all must report every opt-in target as skipped",
    );

    // And they are NOT configured.
    let names = ok_names(&ri.report);
    for m in tome::harness::OPT_IN_TARGETS {
        assert!(
            !names.contains(&m.name().to_string()),
            "plain --all must NOT configure the opt-in target {}",
            m.name(),
        );
    }
}

/// End-to-end (binary): `tome harness use --all` prints the human-only opt-in
/// skip note to STDERR naming the opt-in targets, and does NOT write them to the
/// config. `--all --include-opt-in` writes them and prints NO note. Driven
/// through the real binary in an isolated `$HOME` so the stderr signal is real.
#[test]
fn all_emits_opt_in_skip_note_on_stderr() {
    let env = ToolEnv::new();

    // (a) plain --all → note on stderr, generics absent from config.
    let out = env
        .cmd()
        .args(["harness", "use", "--all", "--scope", "global"])
        .output()
        .expect("run tome harness use --all");
    assert!(
        out.status.success(),
        "harness use --all is a clean exit; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--all skipped opt-in targets")
            && stderr.contains("generic")
            && stderr.contains("generic-op")
            && stderr.contains("--include-opt-in"),
        "plain --all must note the skipped opt-in targets on stderr; got: {stderr}",
    );
    let cfg = env.config_file();
    let body = std::fs::read_to_string(&cfg).expect("config written by --all");
    assert!(
        !body.contains("\"generic\"") && !body.contains("\"generic-op\""),
        "plain --all must NOT persist the opt-in targets: {body}",
    );

    // (b) --all --include-opt-in → NO note, generics present.
    let env2 = ToolEnv::new();
    let out2 = env2
        .cmd()
        .args([
            "harness",
            "use",
            "--all",
            "--include-opt-in",
            "--scope",
            "global",
        ])
        .output()
        .expect("run tome harness use --all --include-opt-in");
    assert!(out2.status.success());
    let stderr2 = String::from_utf8_lossy(&out2.stderr);
    assert!(
        !stderr2.contains("skipped opt-in targets"),
        "--include-opt-in must suppress the skip note; got: {stderr2}",
    );
    let body2 = std::fs::read_to_string(env2.config_file()).expect("config written");
    assert!(
        body2.contains("\"generic\"") && body2.contains("\"generic-op\""),
        "--include-opt-in must persist the opt-in targets: {body2}",
    );
}

/// The opt-in skip note is HUMAN-ONLY: under `--json` it is suppressed (stderr
/// carries no `note:` line). The `--json` stdout envelope stays the machine
/// surface; the note is informational and must not leak into scripted runs.
#[test]
fn all_opt_in_skip_note_suppressed_under_json() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["harness", "use", "--all", "--scope", "global", "--json"])
        .output()
        .expect("run tome harness use --all --json");
    assert!(
        out.status.success(),
        "harness use --all --json is a clean exit; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("skipped opt-in targets"),
        "the opt-in skip note must be suppressed under --json; got stderr: {stderr}",
    );
    // The JSON envelope is still on stdout (selection == "all").
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"selection\":\"all\""),
        "the --json envelope must still be emitted on stdout; got: {stdout}",
    );
}
