//! Phase 10 / US1 — end-to-end identity + consent behaviour, driven through the
//! REAL `tome` binary over an isolated `$HOME` (`ToolEnv`).
//!
//! Every test here drives the spawned binary, NOT the library API, so it
//! exercises the actual CLI dispatch (`main.rs` notice gate + `commands::telemetry`).
//! The lib-level mint/race/reset/purge unit tests live in
//! `src/telemetry/identity.rs`; this suite asserts the observable CLI surface:
//! exit codes, stdout JSON, the on-disk `telemetry/id`, and the first-run notice
//! on stderr.
//!
//! **Env hygiene is mandatory.** `ToolEnv::cmd()` inherits the parent env, and
//! the suite itself may run under CI — so unless every CI var (and the two
//! `TOME_TELEMETRY*` overrides) is cleared per-`Command`, every test would see
//! `enabled=false` (CI auto-off) and the assertions would be meaningless. Each
//! test calls [`clean_cmd`] which `env_remove`s all of them, then sets only what
//! the case needs.

use std::process::Command;

use serde_json::Value;

use crate::common::ToolEnv;
use crate::queue_util::TELEMETRY_ENV_VARS;

/// Build a `tome` command over the isolated `$HOME` with EVERY telemetry/CI env
/// var removed. The caller then `.env(...)`s only the vars the case needs.
fn clean_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd
}

/// Run `clean_cmd` with the given args, asserting the process spawned, and
/// return the captured output.
fn run(env: &ToolEnv, args: &[&str]) -> std::process::Output {
    clean_cmd(env)
        .args(args)
        .output()
        .expect("spawn tome binary")
}

/// Parse `telemetry status --json` stdout into a JSON object, asserting exit 0.
fn status_json(env: &ToolEnv) -> Value {
    let out = run(env, &["telemetry", "status", "--json"]);
    assert!(
        out.status.success(),
        "telemetry status --json exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "status --json is not valid JSON: {e}; stdout: {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The `telemetry/id` file path under the isolated home.
fn id_path(env: &ToolEnv) -> std::path::PathBuf {
    env.tome_root().join("telemetry").join("id")
}

/// The `telemetry/queue.jsonl` file path under the isolated home.
fn queue_path(env: &ToolEnv) -> std::path::PathBuf {
    env.tome_root().join("telemetry").join("queue.jsonl")
}

/// Seed the queue file with `body`, creating `telemetry/` first.
fn seed_queue(env: &ToolEnv, body: &str) {
    let q = queue_path(env);
    std::fs::create_dir_all(q.parent().unwrap()).expect("create telemetry dir");
    std::fs::write(&q, body).expect("seed queue");
}

/// A canonical `8-4-4-4-12` UUID shape check (hex groups). Mirrors the v4 shape
/// the binary mints without re-deriving the version-nibble logic here.
fn looks_like_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let lens = [8usize, 4, 4, 4, 12];
    parts.len() == 5
        && parts
            .iter()
            .zip(lens)
            .all(|(p, n)| p.len() == n && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

// ---------------------------------------------------------------------------
// status — read-only, never mints
// ---------------------------------------------------------------------------

#[test]
fn status_on_fresh_install_is_default_on() {
    let env = ToolEnv::new();
    let v = status_json(&env);

    assert_eq!(
        v["enabled"],
        Value::Bool(true),
        "fresh install is opt-out on"
    );
    assert_eq!(v["source"], "default");
    assert_eq!(v["pending"], 0);
    // Kernel migration: `init` builds an ENABLED handle for every command on a
    // default-on install, and the kernel mints the install id at build time — so
    // a default-on `status` surfaces a freshly-minted `install_uuid`. (The
    // read-only-never-mint guarantee now holds only when telemetry is DISABLED —
    // see `ci_auto_disables_with_no_mint_and_no_notice`.)
    let uuid = v["install_uuid"]
        .as_str()
        .expect("a default-on status surfaces the minted install_uuid");
    assert!(looks_like_uuid(uuid), "install_uuid shape: {uuid}");
    assert!(id_path(&env).exists(), "default-on init mints telemetry/id");
}

// ---------------------------------------------------------------------------
// on / off — mint + persist, then keep the id across a disable
// ---------------------------------------------------------------------------

#[test]
fn on_mints_and_persists_the_id() {
    let env = ToolEnv::new();
    let out = run(&env, &["telemetry", "on"]);
    assert!(
        out.status.success(),
        "telemetry on exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // The id now exists on disk.
    assert!(id_path(&env).exists(), "on must mint telemetry/id");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(id_path(&env))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "id file must be 0600");
    }

    // Status now reads the config-sourced enabled state and a present uuid.
    let v = status_json(&env);
    assert_eq!(v["enabled"], Value::Bool(true));
    assert_eq!(v["source"], "config");
    let uuid = v["install_uuid"]
        .as_str()
        .expect("install_uuid present after on")
        .to_string();
    assert!(looks_like_uuid(&uuid), "install_uuid shape: {uuid}");
}

#[test]
fn off_keeps_the_same_install_uuid() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());
    let before = status_json(&env)["install_uuid"]
        .as_str()
        .expect("uuid after on")
        .to_string();

    let out = run(&env, &["telemetry", "off"]);
    assert!(out.status.success(), "telemetry off should exit 0");

    let v = status_json(&env);
    assert_eq!(v["enabled"], Value::Bool(false), "off disables telemetry");
    assert_eq!(
        v["install_uuid"].as_str(),
        Some(before.as_str()),
        "off must preserve the install UUID (only purge deletes it)"
    );
}

// ---------------------------------------------------------------------------
// reset — sever continuity (new UUID) and require confirmation on a non-TTY
// ---------------------------------------------------------------------------

#[test]
fn reset_yes_changes_the_install_uuid() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());
    let before = status_json(&env)["install_uuid"]
        .as_str()
        .expect("uuid after on")
        .to_string();

    let out = run(&env, &["telemetry", "reset", "--yes"]);
    assert!(
        out.status.success(),
        "telemetry reset --yes exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let after = status_json(&env)["install_uuid"]
        .as_str()
        .expect("uuid after reset")
        .to_string();
    assert_ne!(before, after, "reset must mint a fresh install UUID");
    assert!(looks_like_uuid(&after), "fresh uuid shape: {after}");
}

#[test]
fn reset_without_yes_on_non_tty_is_exit_54() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());

    // Spawned ⇒ stdin is not a TTY ⇒ `prompt::confirm` refuses up front with
    // NotATerminal (exit 54), the same pattern as `models remove`.
    let out = run(&env, &["telemetry", "reset"]);
    assert_eq!(
        out.status.code(),
        Some(54),
        "non-TTY reset without --yes must be NotATerminal (54); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Issue #305 — the global `--non-interactive` flag suppresses the reset
/// confirmation prompt just like the per-command `--yes` (baseline:
/// `reset_without_yes_on_non_tty_is_exit_54`). `telemetry reset` is a second,
/// distinct prompt-bearing command exercising the `--yes` flag family.
#[test]
fn reset_non_interactive_flag_suppresses_prompt() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());

    let out = run(&env, &["telemetry", "reset", "--non-interactive"]);
    assert!(
        out.status.success(),
        "--non-interactive should skip the reset prompt; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Issue #305 — `TOME_NONINTERACTIVE=1` suppresses the reset prompt too. Built
/// from `clean_cmd` (which strips telemetry/CI vars) plus the one var under
/// test; each spawned process carries its own env map.
#[test]
fn reset_tome_noninteractive_env_var_suppresses_prompt() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());

    let out = clean_cmd(&env)
        .env("TOME_NONINTERACTIVE", "1")
        .args(["telemetry", "reset"])
        .output()
        .expect("spawn tome binary");
    assert!(
        out.status.success(),
        "TOME_NONINTERACTIVE=1 should skip the reset prompt; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Issue #305 — `--force` is a hidden alias on `telemetry reset` (which only
/// documented `--yes`), so both non-interactive spellings work everywhere.
#[test]
fn reset_force_alias_is_accepted() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());

    let out = run(&env, &["telemetry", "reset", "--force"]);
    assert!(
        out.status.success(),
        "--force alias should behave like --yes on reset; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// purge — delete the id and switch telemetry off
// ---------------------------------------------------------------------------

#[test]
fn purge_removes_the_id_and_disables() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());
    assert!(id_path(&env).exists(), "id present before purge");

    let out = run(&env, &["telemetry", "purge"]);
    assert!(
        out.status.success(),
        "telemetry purge exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    assert!(!id_path(&env).exists(), "purge must delete telemetry/id");

    let v = status_json(&env);
    assert_eq!(v["enabled"], Value::Bool(false), "purge disables telemetry");
    assert!(
        v.get("install_uuid").is_none(),
        "no install_uuid after purge: {v}"
    );
}

// ---------------------------------------------------------------------------
// status pending counter — counts non-empty queue lines (US2-independent pin)
// ---------------------------------------------------------------------------

#[test]
fn status_pending_counts_queue_lines() {
    let env = ToolEnv::new();
    // N non-empty JSON-ish lines + one blank line. The blank line must NOT be
    // counted. This pins the line-counter independent of US2's queue producer.
    seed_queue(&env, "{\"e\":\"a\"}\n{\"e\":\"b\"}\n{\"e\":\"c\"}\n\n");

    let v = status_json(&env);
    assert_eq!(
        v["pending"], 3,
        "pending counts the 3 non-empty lines, not the blank: {v}"
    );
}

// ---------------------------------------------------------------------------
// on → off → on reuses the SAME install UUID (re-enable does not re-mint)
// ---------------------------------------------------------------------------

#[test]
fn on_off_on_reuses_same_uuid() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());
    let original = status_json(&env)["install_uuid"]
        .as_str()
        .expect("uuid after first on")
        .to_string();

    assert!(run(&env, &["telemetry", "off"]).status.success());
    assert!(run(&env, &["telemetry", "on"]).status.success());

    let v = status_json(&env);
    assert_eq!(v["enabled"], Value::Bool(true), "re-enabled");
    assert_eq!(v["source"], "config");
    assert_eq!(
        v["install_uuid"].as_str(),
        Some(original.as_str()),
        "re-enable reuses the original install UUID (no re-mint)"
    );
}

// ---------------------------------------------------------------------------
// reset / purge clear the queue
// ---------------------------------------------------------------------------

#[test]
fn reset_clears_the_queue() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());
    seed_queue(&env, "{\"e\":\"a\"}\n{\"e\":\"b\"}\n");
    assert_eq!(status_json(&env)["pending"], 2, "queue seeded");

    let out = run(&env, &["telemetry", "reset", "--yes"]);
    assert!(
        out.status.success(),
        "reset --yes exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    assert_eq!(
        status_json(&env)["pending"],
        0,
        "reset must clear the queue"
    );
}

#[test]
fn purge_clears_the_queue() {
    let env = ToolEnv::new();
    assert!(run(&env, &["telemetry", "on"]).status.success());
    seed_queue(&env, "{\"e\":\"a\"}\n{\"e\":\"b\"}\n{\"e\":\"c\"}\n");

    let out = run(&env, &["telemetry", "purge"]);
    assert!(
        out.status.success(),
        "purge exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // Purge also disables telemetry, but the queue must be gone regardless.
    assert!(
        !queue_path(&env).exists(),
        "purge must clear the queue file"
    );
    assert_eq!(status_json(&env)["pending"], 0, "no pending after purge");
}

// ---------------------------------------------------------------------------
// CI auto-disable — no mint, no notice
// ---------------------------------------------------------------------------

#[test]
fn ci_auto_disables_with_no_mint_and_no_notice() {
    let env = ToolEnv::new();
    // A normal (non-telemetry, non-mcp) command under CI. `catalog list` exits
    // cleanly in an empty home.
    let out = clean_cmd(&env)
        .env("CI", "true")
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome binary");
    assert!(
        out.status.success(),
        "catalog list should exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // CI auto-off ⇒ the notice gate self-skips: no id minted, no notice on stderr.
    assert!(
        !id_path(&env).exists(),
        "CI auto-off must mint no install id"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.to_lowercase().contains("telemetry"),
        "CI run must print no telemetry notice; stderr: {stderr}"
    );

    // And status reports the CI source.
    let v = clean_cmd(&env)
        .env("CI", "true")
        .args(["telemetry", "status", "--json"])
        .output()
        .map(|o| serde_json::from_slice::<Value>(&o.stdout).expect("status json"))
        .expect("spawn tome");
    assert_eq!(v["enabled"], Value::Bool(false));
    assert_eq!(v["source"], "ci");
}

#[test]
fn force_on_overrides_ci() {
    let env = ToolEnv::new();
    let v = clean_cmd(&env)
        .env("CI", "true")
        .env("TOME_TELEMETRY", "1")
        .args(["telemetry", "status", "--json"])
        .output()
        .map(|o| serde_json::from_slice::<Value>(&o.stdout).expect("status json"))
        .expect("spawn tome");
    assert_eq!(
        v["enabled"],
        Value::Bool(true),
        "TOME_TELEMETRY=1 overrides CI"
    );
    assert_eq!(v["source"], "env_on");
}

// ---------------------------------------------------------------------------
// first-run opt-out notice — once per minted id, on stderr, excludes `telemetry`
// ---------------------------------------------------------------------------

#[test]
fn first_run_notice_fires_once_on_stderr() {
    let env = ToolEnv::new();

    // Force-on so the notice gate is reliably enabled even under CI. Run a
    // non-telemetry command twice in the SAME isolated home.
    let run1 = clean_cmd(&env)
        .env("TOME_TELEMETRY", "1")
        .args(["catalog", "list"])
        .output()
        .expect("spawn run1");
    let run2 = clean_cmd(&env)
        .env("TOME_TELEMETRY", "1")
        .args(["catalog", "list"])
        .output()
        .expect("spawn run2");

    let err1 = String::from_utf8_lossy(&run1.stderr).to_lowercase();
    let err2 = String::from_utf8_lossy(&run2.stderr).to_lowercase();

    // Run 1 minted the id ⇒ the opt-out notice prints. Match stable substrings
    // from `src/telemetry/notice.rs` ("telemetry" + "off") rather than the full
    // sentence, so a reword that keeps the meaning doesn't brittle-break this.
    assert!(
        err1.contains("telemetry") && err1.contains("off"),
        "first run must print the opt-out notice on stderr; stderr: {err1}"
    );
    // The full FR-013 clause set is pinned by
    // `first_run_notice_discloses_all_required_clauses` below.
    // Run 2 saw the id already present ⇒ no notice.
    assert!(
        !err2.contains("telemetry"),
        "second run must NOT re-print the notice; stderr: {err2}"
    );
}

#[test]
fn first_run_notice_discloses_all_required_clauses() {
    // FR-013: the single first-run line must disclose ALL of — anonymous usage
    // data, named usage of plugins from allowlisted catalogs (Midnight), the
    // opt-out mechanism, and a pointer to `tome telemetry --help`.
    let env = ToolEnv::new();
    let out = clean_cmd(&env)
        .env("TOME_TELEMETRY", "1")
        .args(["catalog", "list"])
        .output()
        .expect("spawn run1");
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();

    assert!(
        err.contains("anonymous"),
        "notice must disclose anonymous collection; stderr: {err}"
    );
    assert!(
        err.contains("midnight"),
        "notice must name the allowlisted catalog (Midnight); stderr: {err}"
    );
    assert!(
        err.contains("tome telemetry --help"),
        "notice must point to `tome telemetry --help`; stderr: {err}"
    );
    assert!(
        err.contains("off"),
        "notice must state the opt-out (`tome telemetry off`); stderr: {err}"
    );
}

#[test]
fn telemetry_command_prints_no_notice() {
    let env = ToolEnv::new();
    // Even force-on against a fresh home, `tome telemetry status` is excluded
    // from the notice gate (the telemetry group manages its own state) and must
    // not first mint an id + print a notice.
    let out = clean_cmd(&env)
        .env("TOME_TELEMETRY", "1")
        .args(["telemetry", "status"])
        .output()
        .expect("spawn tome");
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        !stderr.contains("collects anonymous usage telemetry") && !stderr.contains("opt-out"),
        "the telemetry group must print no first-run notice; stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// AC#7 — MCP server mints the install UUID SILENTLY on first-ever run.
//
// Not assertable in US1: the MCP cold-start mint is realized in US2 via the
// `tome.cold_start` enqueue→mint at server startup (the MCP surface passes
// `surface_is_cli = false`, so it never prints the first-run notice). US2 MUST
// add an integration test asserting the MCP path mints a 0600 `telemetry/id`
// with NO notice on any stream.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// upgrade detection — SKIPPED in US1 (tracked US2 obligation).
//
// `detect_and_record_version` (the `telemetry/last-version` rewrite mechanism)
// exists and is unit-tested in `src/telemetry/identity.rs`, but it is NOT yet
// called from ANY CLI path — `main.rs` only invokes the first-run notice. Wiring
// the version-record call site (and the `tome.upgrade` enqueue) is US2's startup
// path. There is therefore no observable CLI behaviour to assert here, so the
// upgrade sub-test is intentionally omitted until US2 lands the call site.
//
// US2 MUST add: an integration assertion that a version change emits
// `tome.upgrade { from_version }` and rewrites `telemetry/last-version`.
