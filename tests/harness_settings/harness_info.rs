//! Library-API tests for `tome harness info [<name>]`.

use crate::common::{HarnessModulesGuard, HomeGuard, NamedStubHarness, ToolEnv, paths_for};
use tome::cli::HarnessInfoArgs;
use tome::commands::harness::info;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn fallback_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
    }
}

#[test]
fn info_for_unknown_harness_returns_exit_18() {
    // `info::run` reads the process-global harness-modules override slot;
    // serialise against the override-installing tests that share this binary.
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessInfoArgs {
        name: Some("not-a-real-harness".to_string()),
    };
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    let err = info::run(args, &scope, &paths, Mode::Json).expect_err("unknown");
    assert_eq!(err.exit_code(), 18);
}

#[test]
fn info_for_real_harness_runs_without_project() {
    // `info::run` reads the process-global harness-modules override slot;
    // serialise against the override-installing tests that share this binary.
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessInfoArgs {
        name: Some("claude-code".to_string()),
    };
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    let result = info::run(args, &scope, &paths, Mode::Json);
    assert!(result.is_ok(), "info run: {result:?}");
}

/// Phase 11 / US4 (M1): `tome harness info generic` / `generic-op` must resolve
/// the opt-in target via `lookup` and print its snippet, NOT error
/// `HarnessNotSupported` (exit 18). The opt-in targets live in `OPT_IN_TARGETS`,
/// not `SUPPORTED_HARNESSES` / the override slot, so `info::run`'s `lookup`
/// fallback is exercised against the REAL registry.
#[test]
fn info_for_opt_in_targets_resolves_via_lookup() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let _home = HomeGuard::install(env.home_path());

    for name in ["generic", "generic-op"] {
        let args = HarnessInfoArgs {
            name: Some(name.to_string()),
        };
        let scope = fallback_scope();
        let result = info::run(args, &scope, &paths, Mode::Json);
        assert!(
            result.is_ok(),
            "info {name} must resolve via lookup (not exit 18); got {result:?}",
        );
    }
}

#[test]
fn info_reports_direct_scope_when_global_declares() {
    // `info::run` reads the process-global harness-modules override slot;
    // serialise against the override-installing tests that share this binary.
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .unwrap();

    let args = HarnessInfoArgs {
        name: Some("claude-code".to_string()),
    };
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    let result = info::run(args, &scope, &paths, Mode::Human);
    assert!(result.is_ok(), "info run: {result:?}");
}

/// T063: `tome harness info jetbrains-ai` (a manual-only MCP harness) renders
/// the paste-able snippet path without error — for jetbrains-ai the snippet is
/// the primary recovery artifact. (Exact-byte snippet pins live in the
/// `mcp_config` unit tests; this exercises the `info::run` wiring end-to-end.)
#[test]
fn info_for_manual_only_harness_renders_snippet_path() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    // Both modes exercise the snippet branch (Human prints it; Json serialises
    // the `mcp_snippet` field).
    assert!(
        info::run(
            HarnessInfoArgs {
                name: Some("jetbrains-ai".to_string()),
            },
            &scope,
            &paths,
            Mode::Human,
        )
        .is_ok()
    );
    assert!(
        info::run(
            HarnessInfoArgs {
                name: Some("jetbrains-ai".to_string()),
            },
            &scope,
            &paths,
            Mode::Json,
        )
        .is_ok()
    );
}

/// MINOR (US5 closeout): capture the Human-mode STDOUT bytes of
/// `tome harness info jetbrains-ai` (via the real CLI) and assert the
/// `MCP config — paste into …:` heading + the exact paste-able snippet (now
/// carrying `"env": {}` per M1). `info` is read-only — no project, no models.
#[test]
fn info_human_stdout_contains_paste_heading_and_snippet() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["harness", "info", "jetbrains-ai"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("MCP config — paste into jetbrains-ai:"),
        "paste heading present; got:\n{s}",
    );
    // The snippet body — mcpServers shape with env:{} (the M1 fix).
    assert!(s.contains("\"mcpServers\""), "snippet present; got:\n{s}");
    assert!(
        s.contains("\"env\": {}"),
        "snippet carries env:{{}}; got:\n{s}"
    );
}

// --- #327: no-name `tome harness info` — one section/entry per effective harness

/// A `GlobalFallback` scope whose effective set comes from the global
/// `[harness].enabled` list. Two stub harnesses ⇒ an effective set of ≥2.
fn seed_two_harness_global(env: &ToolEnv, paths: &tome::paths::Paths) {
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; declare two harnesses so the
    // effective list has ≥2 entries.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"alpha\", \"beta\"]\n",
    )
    .unwrap();
    let _ = env; // env kept for symmetry / future use.
}

/// #327 (json): `tome harness info` with NO name emits a JSON ARRAY with one
/// entry per effective harness. Asserted on REAL serialized JSON (the emit-free
/// `build_effective_outcomes` path `run` uses for `Mode::Json`), not the
/// in-memory struct.
#[test]
fn info_no_name_json_is_array_with_one_entry_per_effective_harness() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha", "beta"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    seed_two_harness_global(&env, &paths);
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());

    let outcomes =
        info::build_effective_outcomes(&scope, &paths).expect("build effective outcomes");
    // Serialize the REAL vec — this is the exact array `write_json` emits for the
    // no-name `--json` form.
    let json = serde_json::to_string(&outcomes).expect("serialise");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");

    let arr = parsed.as_array().expect("no-name json must be an array");
    assert_eq!(arr.len(), 2, "one entry per effective harness; got: {json}");
    let names: Vec<&str> = arr
        .iter()
        .map(|o| o["name"].as_str().expect("name field"))
        .collect();
    assert!(names.contains(&"alpha"), "alpha present; got: {names:?}");
    assert!(names.contains(&"beta"), "beta present; got: {names:?}");
    // Each array element is a full HarnessInfoOutcome object (has `description`).
    assert!(
        arr.iter().all(|o| o["description"].is_string()),
        "each entry is a full outcome object; got: {json}",
    );
}

/// #327 (human): `tome harness info` with NO name renders one `Harness: <name>`
/// section per effective harness. Driven through the real `info::run` path.
#[test]
fn info_no_name_human_renders_one_section_per_effective_harness() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha", "beta"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    seed_two_harness_global(&env, &paths);
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());

    // No-name form: `name: None`.
    let result = info::run(HarnessInfoArgs { name: None }, &scope, &paths, Mode::Human);
    assert!(result.is_ok(), "no-name human info run: {result:?}");

    // Also confirm the section renderer produces a `Harness:` header per outcome
    // by inspecting the real outcomes vec (the same vec the human path renders).
    let outcomes =
        info::build_effective_outcomes(&scope, &paths).expect("build effective outcomes");
    assert_eq!(outcomes.len(), 2, "two sections expected");
}

/// #327 (empty set): `tome harness info` with NO name and NOTHING configured
/// degrades gracefully — `--json` emits `[]`, human prints a hint, exit 0.
/// Never an error.
#[test]
fn info_no_name_empty_effective_set_is_graceful() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Registry has a stub, but NOTHING is declared in any settings layer ⇒ empty
    // effective set.
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());

    // Real serialized JSON is `[]`.
    let outcomes =
        info::build_effective_outcomes(&scope, &paths).expect("build effective outcomes");
    assert!(outcomes.is_empty(), "empty effective set ⇒ empty vec");
    assert_eq!(
        serde_json::to_string(&outcomes).expect("serialise"),
        "[]",
        "empty no-name json must be []",
    );

    // Both modes exit 0 (no error) on the empty set.
    let json = info::run(HarnessInfoArgs { name: None }, &scope, &paths, Mode::Json);
    assert!(
        json.is_ok(),
        "empty-set json must be Ok(exit 0); got {json:?}"
    );
    let human = info::run(HarnessInfoArgs { name: None }, &scope, &paths, Mode::Human);
    assert!(
        human.is_ok(),
        "empty-set human must be Ok(exit 0); got {human:?}",
    );
}

/// #327 (single-name unchanged): an explicit unknown name still errors
/// `HarnessNotSupported` (exit 18) — the no-name path must NOT swallow it.
#[test]
fn info_explicit_unknown_name_still_errors_18() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());

    let err = info::run(
        HarnessInfoArgs {
            name: Some("not-a-real-harness".to_string()),
        },
        &scope,
        &paths,
        Mode::Json,
    )
    .expect_err("unknown explicit name must error");
    assert_eq!(err.exit_code(), 18);
}
