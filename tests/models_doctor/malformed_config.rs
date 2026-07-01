//! Issue #287 — a malformed `~/.tome/config.toml` must NOT brick `tome doctor`
//! and `tome status` (the diagnostics you reach for to fix it), while every
//! ordinary command keeps failing loudly with exit 5.
//!
//! These are SPAWNED-binary tests: only the assembled dispatch path exercises
//! the pre-dispatch "universal gate" (strict `config::load` inside workspace
//! resolution) that the fix softens for the two diagnostic commands. A unit
//! test on the command body alone would miss the gate (it runs before the body).

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use serde_json::Value;

/// Plant a malformed global config under the isolated HOME and fabricate the
/// registry models so the ONLY thing wrong with the install is the config — any
/// non-Ok classification is then attributable to the config finding alone.
fn env_with_malformed_config(body: &str) -> ToolEnv {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    std::fs::write(&paths.global_config_file, body).unwrap();
    env
}

#[test]
fn doctor_json_reports_malformed_config_instead_of_exit_5() {
    // Unknown key inside a known section — the most common typo.
    let env = env_with_malformed_config("[query]\nnope = 1\n");

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();

    // It must NOT be the loud strict-parse exit (5) that other commands emit:
    // doctor RAN and produced a report.
    assert_ne!(
        out.status.code(),
        Some(5),
        "doctor must not exit 5 on a malformed config; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Non-zero overall (Unhealthy) because the config is broken. Issue #282:
    // a malformed config is Unhealthy → the unhealthy code (1), not Degraded.
    assert_eq!(
        out.status.code(),
        Some(tome::error::EXIT_HEALTH_UNHEALTHY),
        "doctor should classify Unhealthy"
    );

    let v: Value = serde_json::from_slice(&out.stdout)
        .expect("doctor --json must still emit a parseable report");

    // The config parse problem appears as a `config` suggested-fix naming the key.
    let fixes = v["suggested_fixes"]
        .as_array()
        .expect("suggested_fixes array");
    let cfg_fix = fixes
        .iter()
        .find(|f| f["subsystem"] == "config")
        .expect("a `config` suggested-fix must be present");
    let diagnosis = cfg_fix["diagnosis"].as_str().unwrap();
    assert!(
        diagnosis.contains("nope"),
        "diagnosis must name the offending key: {diagnosis}",
    );
    assert!(
        diagnosis.contains("config.toml"),
        "diagnosis must point at the file: {diagnosis}",
    );
    // It is a manual finding — Tome never rewrites a user-authored config.
    assert_eq!(cfg_fix["auto_fixable"], false);
    assert_eq!(v["overall"], "unhealthy");
}

#[test]
fn doctor_human_reports_malformed_config_instead_of_exit_5() {
    // Unknown TOP-LEVEL key (forward-incompatible / stray section name).
    let env = env_with_malformed_config("totally_unknown = 5\n");

    let out = env.cmd().arg("doctor").output().unwrap();
    assert_ne!(out.status.code(), Some(5), "doctor must not exit 5");

    let stdout = String::from_utf8_lossy(&out.stdout);
    // The human report rendered AND surfaced the malformed-config finding.
    assert!(
        stdout.contains("config"),
        "human doctor output should mention the config finding; got:\n{stdout}",
    );
    assert!(
        stdout.contains("totally_unknown"),
        "human doctor output should name the offending key; got:\n{stdout}",
    );
}

#[test]
fn doctor_fix_keeps_malformed_config_as_manual_finding() {
    // `--fix` must never silently "repair" a user-authored config; the finding
    // persists through the re-assembly path and becomes the exit-75
    // (DoctorFixNotSafe) "fix ran, manual work remains" signal.
    let env = env_with_malformed_config("[query]\nbad_key = true\n");

    let out = env
        .cmd()
        .args(["--json", "doctor", "--fix"])
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(5), "doctor --fix must not exit 5");

    let v: Value = serde_json::from_slice(&out.stdout)
        .expect("doctor --fix --json must still emit a parseable report");
    let fixes = v["suggested_fixes"]
        .as_array()
        .expect("suggested_fixes array");
    assert!(
        fixes.iter().any(|f| f["subsystem"] == "config"),
        "the config finding must survive `--fix`'s re-assembly: {v}",
    );
    // The config remains on disk untouched by --fix (no rewrite).
    let paths = paths_for(&env);
    let after = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert!(
        after.contains("bad_key"),
        "doctor --fix must not rewrite the user's config",
    );
    // Exit 75 = "fix did something but a manual issue remains" (the config).
    assert_eq!(
        out.status.code(),
        Some(75),
        "an unfixable config finding under --fix should exit 75",
    );
}

#[test]
fn status_json_reports_malformed_config_instead_of_exit_5() {
    let env = env_with_malformed_config("[output]\nbogus = \"x\"\n");

    let out = env.cmd().args(["--json", "status"]).output().unwrap();
    assert_ne!(
        out.status.code(),
        Some(5),
        "status must not exit 5 on a malformed config; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    let v: Value = serde_json::from_slice(&out.stdout)
        .expect("status --json must still emit a parseable report");

    // The `config` health block appears (omitted when the config is clean) and
    // names the offending key. Overall flips to unhealthy.
    let cfg = v
        .get("config")
        .expect("status --json must carry a `config` block on a malformed config");
    assert_eq!(cfg["ok"], false);
    assert!(
        cfg["message"].as_str().unwrap().contains("bogus"),
        "status config message must name the offending key: {cfg}",
    );
    assert_eq!(v["overall"], "unhealthy");
    assert_eq!(
        out.status.code(),
        Some(tome::error::EXIT_HEALTH_UNHEALTHY),
        "status should classify unhealthy"
    );
}

#[test]
fn status_json_omits_config_block_when_well_formed() {
    // Control: a clean (well-formed) config keeps the byte-stable shape — the
    // `config` key is absent (skip_serializing_if).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    std::fs::write(&paths.global_config_file, "[query]\ntop_k = 5\n").unwrap();

    let out = env.cmd().args(["--json", "status"]).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).expect("status --json parses");
    assert!(
        v.get("config").is_none(),
        "a well-formed config must omit the `config` block (byte-stable pin): {v}",
    );
}

#[test]
fn ordinary_command_still_exits_5_on_malformed_config() {
    // The strict/defensive split is intentional: a NON-diagnostic command must
    // keep failing loudly so a typo in the one global config is never silently
    // swallowed everywhere.
    let env = env_with_malformed_config("[query]\nnope = 1\n");

    let out = env.cmd().args(["catalog", "list"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(5),
        "catalog list must still exit 5 on a malformed config; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // And the loud error names the offending key.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nope"),
        "the strict error must name the offending key: {stderr}",
    );
}

/// Issue #287 (review item 3): a config that is OTHERWISE malformed but ALSO
/// carries `[workspace] default = "..."`. Because the whole file fails to parse,
/// the `default` knob is silently ignored under lenient resolution — the
/// resolver falls through to the global fallback, doctor/status still run, and
/// the Config finding is reported. (Were leniency to somehow consult the
/// half-parsed `default` and require that workspace's membership, the command
/// would error before reporting; this confirms it does not.)
#[test]
fn doctor_and_status_run_when_malformed_config_also_sets_workspace_default() {
    // `[workspace] default` is valid in isolation, but the stray top-level key
    // makes the whole document fail the strict (deny-unknown-fields) parse. The
    // named workspace ("ghost") is deliberately NOT seeded — it must never be
    // looked up, because the malformed file means `default` is never honoured.
    let body = "stray_top_level = 1\n[workspace]\ndefault = \"ghost\"\n";

    // doctor.
    let env = env_with_malformed_config(body);
    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    assert_ne!(
        out.status.code(),
        Some(5),
        "doctor must not exit 5; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // It also must NOT be the WorkspaceNotFound exit (13) — i.e. leniency did not
    // try to honour the ignored `default = "ghost"`.
    assert_ne!(
        out.status.code(),
        Some(13),
        "the ignored `[workspace] default` must not be resolved (no exit 13); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        out.status.code(),
        Some(tome::error::EXIT_HEALTH_UNHEALTHY),
        "doctor classifies unhealthy"
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");
    let fixes = v["suggested_fixes"]
        .as_array()
        .expect("suggested_fixes array");
    assert!(
        fixes.iter().any(|f| f["subsystem"] == "config"),
        "the Config finding must be reported: {v}",
    );

    // status — same config, same expectations.
    let env = env_with_malformed_config(body);
    let out = env.cmd().args(["--json", "status"]).output().unwrap();
    assert_ne!(out.status.code(), Some(5), "status must not exit 5");
    assert_ne!(
        out.status.code(),
        Some(13),
        "status must not resolve the ignored `[workspace] default`",
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("status --json parses");
    assert!(
        v.get("config").is_some(),
        "status must carry the `config` block: {v}",
    );
}
