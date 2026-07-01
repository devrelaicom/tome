//! `tome config {show,validate}` integration tests (issue #286).
//!
//! Drives the compiled `tome` binary against an isolated `$HOME` so provenance
//! resolution is exercised end-to-end — including the env-override knobs, which
//! are set on the spawned process's environment. `config show`/`validate` read
//! only `~/.tome/config.toml` (no index, no models), so these are cheap spawns.

mod common;

use std::collections::BTreeMap;

use common::ToolEnv;
use serde::Deserialize;

/// Write `body` to `<home>/.tome/config.toml`.
fn write_config(env: &ToolEnv, body: &str) {
    let root = env.tome_root();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("config.toml"), body).unwrap();
}

/// One `--json` knob record: `{ "value", "source" }`.
#[derive(Debug, Deserialize)]
struct KnobJson {
    value: String,
    source: String,
}

/// Parse `tome config show --json` stdout (one JSON object of key → record).
fn show_json(env: &ToolEnv) -> BTreeMap<String, KnobJson> {
    let out = env
        .cmd()
        .args(["--json", "config", "show"])
        .output()
        .expect("spawn tome config show --json");
    assert!(
        out.status.success(),
        "config show --json exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The record is a single JSON object on one line (NDJSON with a single row).
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("a JSON object on stdout");
    serde_json::from_str(line).expect("stdout is one JSON object")
}

/// Spawn `config show --json` with extra env vars applied.
fn show_json_with_env(env: &ToolEnv, vars: &[(&str, &str)]) -> BTreeMap<String, KnobJson> {
    let mut cmd = env.cmd();
    cmd.args(["--json", "config", "show"]);
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn tome config show --json (env)");
    assert!(
        out.status.success(),
        "config show --json (env) exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("a JSON object on stdout");
    serde_json::from_str(line).expect("stdout is one JSON object")
}

// ---------------------------------------------------------------------------
// show — provenance
// ---------------------------------------------------------------------------

#[test]
fn show_default_config_is_all_default() {
    let env = ToolEnv::new();
    // No config.toml written → every knob defaults. The base `cmd()` clears
    // TOME_LOG / RUST_LOG, so the env-sensitive logging knob is also default.
    //
    // NOTE: `telemetry.enabled` is intentionally excluded — the shared test
    // harness (`ToolEnv::cmd`) force-sets `TOME_TELEMETRY=0` to prevent a
    // detached flusher storm across the suite, so that ONE knob is genuinely
    // `(env)` in tests (which is itself correct provenance, exercised by the
    // `show_env_override_telemetry_enabled` test).
    let knobs = show_json(&env);

    // A representative spread across sections — every one with no env influence.
    for key in [
        "query.top_k",
        "query.rerank",
        "summariser.enabled",
        "logging.level",
        "output.color",
        "output.progress",
        "workspace.default",
        "mcp.description_max_chars",
        "models.profile",
        "doctor.verify_by_default",
        "harness.default_scope",
        "telemetry.endpoint",
    ] {
        let k = knobs
            .get(key)
            .unwrap_or_else(|| panic!("knob {key} present"));
        assert_eq!(k.source, "default", "{key} should be (default): {k:?}");
    }
    // Spot-check a couple of default values.
    assert_eq!(knobs["query.top_k"].value, "10");
    assert_eq!(knobs["models.profile"].value, "medium");
    assert_eq!(knobs["output.color"].value, "auto");
    // And confirm the knob set is complete (all 15 curated knobs present).
    assert!(
        knobs.contains_key("telemetry.enabled"),
        "the full curated knob set is rendered"
    );
    assert_eq!(knobs.len(), 15, "exactly the 15 curated knobs: {knobs:?}");
}

#[test]
fn show_config_override_marks_config() {
    let env = ToolEnv::new();
    write_config(
        &env,
        "[query]\ntop_k = 25\n[models]\nprofile = \"large\"\n[doctor]\nverify_by_default = true\n",
    );
    let knobs = show_json(&env);

    assert_eq!(knobs["query.top_k"].value, "25");
    assert_eq!(knobs["query.top_k"].source, "config");
    assert_eq!(knobs["models.profile"].value, "large");
    assert_eq!(knobs["models.profile"].source, "config");
    assert_eq!(knobs["doctor.verify_by_default"].value, "true");
    assert_eq!(knobs["doctor.verify_by_default"].source, "config");
    // A key NOT in the file stays default.
    assert_eq!(knobs["query.rerank"].source, "default");
}

#[test]
fn show_config_present_at_default_value_is_still_config() {
    // A key set to the built-in default value must still read `(config)` —
    // presence is detected from the raw document, not the final struct.
    let env = ToolEnv::new();
    write_config(&env, "[query]\ntop_k = 10\n");
    let knobs = show_json(&env);
    assert_eq!(knobs["query.top_k"].value, "10");
    assert_eq!(knobs["query.top_k"].source, "config");
}

#[test]
fn show_env_override_logging_level() {
    let env = ToolEnv::new();
    write_config(&env, "[logging]\nlevel = \"warn\"\n");
    // TOME_LOG wins over the config value AND marks the knob (env).
    let knobs = show_json_with_env(&env, &[("TOME_LOG", "trace")]);
    assert_eq!(knobs["logging.level"].value, "trace");
    assert_eq!(knobs["logging.level"].source, "env");
}

#[test]
fn show_env_override_rust_log_also_counts() {
    let env = ToolEnv::new();
    // RUST_LOG is the second env source for logging level.
    let knobs = show_json_with_env(&env, &[("RUST_LOG", "debug")]);
    assert_eq!(knobs["logging.level"].value, "debug");
    assert_eq!(knobs["logging.level"].source, "env");
}

#[test]
fn show_env_override_no_color() {
    let env = ToolEnv::new();
    write_config(&env, "[output]\ncolor = \"always\"\n");
    // NO_COLOR forces colour off and marks the knob (env), overriding the file.
    let knobs = show_json_with_env(&env, &[("NO_COLOR", "1")]);
    assert_eq!(knobs["output.color"].value, "never");
    assert_eq!(knobs["output.color"].source, "env");
}

#[test]
fn show_env_override_telemetry_enabled() {
    let env = ToolEnv::new();
    // The base `cmd()` sets TOME_TELEMETRY=0 already, but we set it explicitly
    // to assert the (env) provenance on the exact token the resolver honours.
    let knobs = show_json_with_env(&env, &[("TOME_TELEMETRY", "1")]);
    assert_eq!(knobs["telemetry.enabled"].value, "true");
    assert_eq!(knobs["telemetry.enabled"].source, "env");
}

#[test]
fn show_env_override_telemetry_endpoint() {
    let env = ToolEnv::new();
    let knobs = show_json_with_env(&env, &[("TOME_GAUGE_ENDPOINT", "https://collector.test/")]);
    assert_eq!(knobs["telemetry.endpoint"].value, "https://collector.test/");
    assert_eq!(knobs["telemetry.endpoint"].source, "env");
}

#[test]
fn show_no_false_env_for_non_env_knobs() {
    // A knob without an env override must NEVER be `(env)`, even when a similarly
    // named env var is set. `models.profile` has no env override; set a bogus
    // env and assert it stays config/default.
    let env = ToolEnv::new();
    write_config(&env, "[models]\nprofile = \"small\"\n");
    let knobs = show_json_with_env(
        &env,
        &[
            ("TOME_MODELS_PROFILE", "large"),
            ("MODELS_PROFILE", "large"),
        ],
    );
    assert_eq!(knobs["models.profile"].value, "small");
    assert_eq!(knobs["models.profile"].source, "config");
}

#[test]
fn show_human_output_has_annotations() {
    let env = ToolEnv::new();
    write_config(&env, "[query]\ntop_k = 7\n");
    let out = env
        .cmd()
        .args(["config", "show"])
        .output()
        .expect("spawn tome config show");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The overridden knob shows its value + (config); a defaulted one shows
    // (default).
    assert!(
        stdout.contains("query.top_k") && stdout.contains("7 (config)"),
        "human output must annotate the config knob:\n{stdout}"
    );
    assert!(
        stdout.contains("(default)"),
        "human output must annotate defaulted knobs:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------

#[test]
fn validate_good_config_exits_zero() {
    let env = ToolEnv::new();
    write_config(&env, "[query]\ntop_k = 5\n");
    let out = env
        .cmd()
        .args(["config", "validate"])
        .output()
        .expect("spawn tome config validate");
    assert!(out.status.success(), "exit {:?}", out.status.code());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("config is valid"),
        "expected a 'config is valid' line on stdout"
    );
}

#[test]
fn validate_absent_config_is_valid() {
    let env = ToolEnv::new();
    // No config.toml → all defaults → valid.
    let out = env
        .cmd()
        .args(["config", "validate"])
        .output()
        .expect("spawn tome config validate");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("config is valid"));
}

#[test]
fn validate_good_config_json() {
    let env = ToolEnv::new();
    write_config(&env, "[query]\ntop_k = 5\n");
    let out = env
        .cmd()
        .args(["--json", "config", "validate"])
        .output()
        .expect("spawn tome config validate --json");
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("json");
    assert_eq!(v["valid"], serde_json::json!(true));
    // On success, `error` is omitted.
    assert!(v.get("error").is_none());
}

#[test]
fn validate_malformed_config_exits_5_and_names_key() {
    let env = ToolEnv::new();
    write_config(&env, "[query]\nnope = 1\n");
    let out = env
        .cmd()
        .args(["config", "validate"])
        .output()
        .expect("spawn tome config validate");
    assert_eq!(
        out.status.code(),
        Some(5),
        "malformed config must exit 5; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nope"),
        "the error must name the offending key: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("unknown"),
        "the error should describe the unknown field: {stderr}"
    );
}

#[test]
fn validate_malformed_config_json_reports_on_stdout() {
    let env = ToolEnv::new();
    write_config(&env, "[query]\nnope = 1\n");
    let out = env
        .cmd()
        .args(["--json", "config", "validate"])
        .output()
        .expect("spawn tome config validate --json");
    assert_eq!(out.status.code(), Some(5));
    // The structured report lands on stdout for scriptability.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout json: {e}: {stdout}"));
    assert_eq!(v["valid"], serde_json::json!(false));
    assert!(
        v["error"].as_str().unwrap_or("").contains("nope"),
        "the JSON error must name the offending key: {stdout}"
    );
}

/// The command surface never writes to the tome root (read-only). Assert no
/// `config.toml` is created by `show`/`validate` when the file is absent.
#[test]
fn show_and_validate_are_read_only() {
    let env = ToolEnv::new();
    let cfg = env.tome_root().join("config.toml");
    assert!(!cfg.exists());
    for args in [&["config", "show"][..], &["config", "validate"][..]] {
        let out = env.cmd().args(args).output().expect("spawn tome config");
        assert!(
            out.status.success(),
            "{args:?} exit {:?}",
            out.status.code()
        );
    }
    assert!(
        !cfg.exists(),
        "config show/validate must not create the config file"
    );
}
