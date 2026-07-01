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

/// Every environment variable that overrides a shown knob's provenance. The
/// default-assertion tests clear ALL of these so an ambient value (a dev's
/// `NO_COLOR`, a CI's `TOME_GAUGE_ENDPOINT`, …) can't flip a knob to `(env)`
/// and flake the test. `TOME_LOG`/`RUST_LOG`/`TOME_TELEMETRY` are already
/// handled by the base `ToolEnv::cmd()`, but we clear them here too so the
/// clean baseline is self-contained and explicit.
const ENV_OVERRIDE_VARS: &[&str] = &[
    "TOME_LOG",
    "RUST_LOG",
    "NO_COLOR",
    "TOME_WORKSPACE",
    "TOME_GAUGE_ENDPOINT",
    "TOME_TELEMETRY",
];

/// Parse a single-object JSON stdout line into the knob map.
fn parse_show_json(stdout: &str) -> BTreeMap<String, KnobJson> {
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("a JSON object on stdout");
    serde_json::from_str(line).expect("stdout is one JSON object")
}

/// `tome config show --json` with a CLEAN env: every provenance-affecting env
/// var removed, and `TOME_TELEMETRY=0` restored (so telemetry.enabled has a
/// deterministic `(env)` state matching the harness's flusher-suppression
/// contract). Use for the default-assertion tests.
fn show_json(env: &ToolEnv) -> BTreeMap<String, KnobJson> {
    let mut cmd = env.cmd();
    cmd.args(["--json", "config", "show"]);
    for v in ENV_OVERRIDE_VARS {
        cmd.env_remove(v);
    }
    // Restore the harness's telemetry-off contract deterministically.
    cmd.env("TOME_TELEMETRY", "0");
    let out = cmd.output().expect("spawn tome config show --json");
    assert!(
        out.status.success(),
        "config show --json exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    parse_show_json(&String::from_utf8_lossy(&out.stdout))
}

/// Spawn `config show --json` from the CLEAN baseline, then apply the requested
/// env overrides. Clearing first means a test asserting `(env)` for one var
/// isn't disturbed by an ambient value of another.
fn show_json_with_env(env: &ToolEnv, vars: &[(&str, &str)]) -> BTreeMap<String, KnobJson> {
    let mut cmd = env.cmd();
    cmd.args(["--json", "config", "show"]);
    for v in ENV_OVERRIDE_VARS {
        cmd.env_remove(v);
    }
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
    parse_show_json(&String::from_utf8_lossy(&out.stdout))
}

// ---------------------------------------------------------------------------
// show — provenance
// ---------------------------------------------------------------------------

#[test]
fn show_default_config_is_all_default() {
    let env = ToolEnv::new();
    // No config.toml written → every knob defaults. `show_json` clears EVERY
    // provenance-affecting env var (NO_COLOR / TOME_WORKSPACE /
    // TOME_GAUGE_ENDPOINT / TOME_LOG / RUST_LOG), so the env-sensitive knobs
    // read `(default)` deterministically regardless of the ambient environment.
    let knobs = show_json(&env);

    // Every knob EXCEPT telemetry.enabled is `(default)` on a fresh config.
    for key in [
        "query.top_k",
        "query.rerank",
        "query.strict_min_score",
        "summariser.enabled",
        "summariser.long_max_chars",
        "logging.level",
        "output.color",
        "output.progress",
        "workspace.default",
        "mcp.description_max_chars",
        "models.profile",
        "doctor.verify_by_default",
        "harness.default_scope",
        "hooks.translate_plugin_hooks",
        "telemetry.endpoint",
    ] {
        let k = knobs
            .get(key)
            .unwrap_or_else(|| panic!("knob {key} present"));
        assert_eq!(k.source, "default", "{key} should be (default): {k:?}");
    }

    // `telemetry.enabled` is `(env)` in tests: `show_json` sets TOME_TELEMETRY=0
    // (the harness flusher-suppression contract), which the telemetry SSOT
    // surfaces as an env-decided `false`. This is correct provenance, not a bug.
    assert_eq!(knobs["telemetry.enabled"].source, "env");
    assert_eq!(knobs["telemetry.enabled"].value, "false");

    // Spot-check the corrected, single-sourced default VALUES (issue #286 #2/#3).
    assert_eq!(knobs["query.top_k"].value, "10");
    assert_eq!(knobs["query.strict_min_score"].value, "none"); // #3: no floor
    assert_eq!(knobs["summariser.long_max_chars"].value, "2500"); // #2: was 4000
    assert_eq!(knobs["mcp.description_max_chars"].value, "150"); // #2: was 200
    assert_eq!(knobs["output.progress"].value, "auto"); // honest auto default
    assert_eq!(knobs["models.profile"].value, "medium");
    assert_eq!(knobs["output.color"].value, "auto");
    assert_eq!(knobs["hooks.translate_plugin_hooks"].value, "true"); // #7

    // Confirm the knob set is complete: exactly the 16 curated knobs.
    assert_eq!(knobs.len(), 16, "exactly the 16 curated knobs: {knobs:?}");
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
fn show_no_false_env_for_summariser_enabled() {
    // A second non-env knob: `summariser.enabled` has no env override. A bogus
    // `TOME_SUMMARISER_ENABLED` must NOT flip it to `(env)`.
    let env = ToolEnv::new();
    write_config(&env, "[summariser]\nenabled = false\n");
    let knobs = show_json_with_env(&env, &[("TOME_SUMMARISER_ENABLED", "true")]);
    assert_eq!(knobs["summariser.enabled"].value, "false");
    assert_eq!(knobs["summariser.enabled"].source, "config");
}

#[test]
fn show_telemetry_ci_auto_disable_is_env() {
    // Item #4: in CI the effective telemetry state is forced OFF by the CI
    // auto-disable short-circuit — the shown value/source must reflect that
    // (false + env), NOT the config/default the file would suggest. We simulate
    // CI via GITHUB_ACTIONS and clear TOME_TELEMETRY so the CI branch (not the
    // explicit env-off) is the decider.
    let env = ToolEnv::new();
    write_config(&env, "[telemetry]\nenabled = true\n");
    let mut cmd = env.cmd();
    cmd.args(["--json", "config", "show"]);
    for v in ENV_OVERRIDE_VARS {
        cmd.env_remove(v);
    }
    cmd.env("GITHUB_ACTIONS", "true"); // CI marker → resolver Source::Ci
    let out = cmd.output().expect("spawn tome config show --json (ci)");
    assert!(
        out.status.success(),
        "exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let knobs = parse_show_json(&String::from_utf8_lossy(&out.stdout));
    // Effective is false (CI forces off) and the provenance is env — even though
    // the file says `enabled = true`.
    assert_eq!(knobs["telemetry.enabled"].value, "false");
    assert_eq!(knobs["telemetry.enabled"].source, "env");
}

#[test]
fn show_json_never_leaks_provider_secret() {
    // Item #6: `show` excludes the provider registry entirely. A config with an
    // inline `api_key` must produce a `config show --json` whose FULL stdout
    // contains none of the secret, the `api_key` field name, or `providers`.
    let env = ToolEnv::new();
    write_config(
        &env,
        "[providers.p]\nkind = \"openai\"\napi_key = \"sk-leaktest0123456789abcdef\"\n",
    );
    let mut cmd = env.cmd();
    cmd.args(["--json", "config", "show"]);
    for v in ENV_OVERRIDE_VARS {
        cmd.env_remove(v);
    }
    cmd.env("TOME_TELEMETRY", "0");
    let out = cmd
        .output()
        .expect("spawn tome config show --json (secret)");
    assert!(out.status.success(), "exit {:?}", out.status.code());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("sk-leaktest0123456789abcdef"),
        "the inline api_key must not appear in show output:\n{stdout}"
    );
    assert!(
        !stdout.contains("api_key"),
        "the api_key field name must not appear:\n{stdout}"
    );
    assert!(
        !stdout.contains("providers"),
        "the providers registry must not appear:\n{stdout}"
    );
}

#[test]
fn validate_malformed_near_secret_does_not_leak() {
    // Item #1 (end-to-end): a parse error ON an inline api_key line makes toml
    // echo that whole line (secret included) in its snippet. A duplicate
    // `api_key` puts the error on that exact line deterministically. Neither the
    // scriptable `validate --json` stdout report NOR the stderr error envelope
    // may carry the credential.
    let env = ToolEnv::new();
    write_config(
        &env,
        "[providers.p]\nkind = \"openai\"\napi_key = \"sk-leaktest0123456789abcdef\"\napi_key = \"sk-second00123456789abcdef\"\n",
    );
    let out = env
        .cmd()
        .args(["--json", "config", "validate"])
        .output()
        .expect("spawn tome config validate --json (secret)");
    assert_eq!(out.status.code(), Some(5));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for secret in ["sk-leaktest0123456789abcdef", "sk-second00123456789abcdef"] {
        assert!(
            !stdout.contains(secret),
            "validate --json stdout leaked the api_key {secret}:\n{stdout}"
        );
        assert!(
            !stderr.contains(secret),
            "validate stderr leaked the api_key {secret}:\n{stderr}"
        );
    }
    // The redaction marker is present (proves the snippet DID contain the key
    // and was scrubbed, not that the key simply never appeared).
    assert!(
        stdout.contains("<scrubbed>") || stderr.contains("<scrubbed>"),
        "expected a redaction marker proving the scrub fired:\nstdout={stdout}\nstderr={stderr}"
    );
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
