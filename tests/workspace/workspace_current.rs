//! `tome workspace current [--json]` (issue #301).
//!
//! The lightweight "which workspace is bound to this directory" command for
//! shell prompts / scripting. Test mix per the project convention:
//!
//! - Library API (`commands::workspace::current::run`) for the pure
//!   Ok/Err + exit-code contract over a synthetic `ResolvedScope` (bound
//!   sources vs the `GlobalFallback` "not bound" source).
//! - CLI binary (real `Some(exit_code)` + captured stdout/stderr) for the
//!   load-bearing prompt contract: bound → JUST the name on one line;
//!   unbound → non-zero exit with EMPTY stdout so
//!   `$(tome workspace current 2>/dev/null)` yields the empty string; and
//!   the `--json` wire shape.

use crate::common::ToolEnv;
use tempfile::TempDir;
use tome::commands::workspace::current;
use tome::error::TomeError;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

/// A `ResolvedScope` bound to `name` via `source` (an explicit selection or
/// project binding).
fn bound_scope(name: &str, source: ScopeSource) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(name).unwrap()),
        source,
        project_root: None,
        overridden_project_marker: None,
    }
}

// ---------------------------------------------------------------------------
// Library API — the Ok/Err + exit-code contract.
// ---------------------------------------------------------------------------

#[test]
fn unbound_global_fallback_errors_with_exit_12() {
    // The GlobalFallback source is the ONLY "not bound" case: no flag, no
    // env, no config default, no project marker.
    let scope = ResolvedScope::global_fallback();
    let err = current::run(&scope, Mode::Human).expect_err("unbound must fail");
    assert!(
        matches!(err, TomeError::WorkspaceNotBound),
        "expected WorkspaceNotBound, got {err:?}",
    );
    assert_eq!(err.exit_code(), 12, "not-bound uses WorkspaceNotBound(12)");
    // The diagnostic must be actionable — it points at how to bind/select,
    // not the registry-oriented `init` hint of WorkspaceNotFound.
    let msg = err.to_string();
    assert!(
        msg.contains("no workspace is bound to the current directory")
            && msg.contains("tome workspace use")
            && msg.contains("--workspace"),
        "message must be actionable; got {msg:?}",
    );
    // And it must NOT leak the misleading `workspace init` hint.
    assert!(
        !msg.contains("tome workspace init"),
        "message must not carry the registry `init` hint; got {msg:?}",
    );
}

#[test]
fn unbound_global_fallback_errors_in_json_too() {
    let scope = ResolvedScope::global_fallback();
    let err = current::run(&scope, Mode::Json).expect_err("unbound must fail");
    assert_eq!(err.exit_code(), 12);
    assert_eq!(err.category().as_str(), "workspace_not_bound");
}

#[test]
fn bound_via_flag_is_ok() {
    let scope = bound_scope("global", ScopeSource::Flag);
    current::run(&scope, Mode::Human).expect("bound scope prints and exits 0");
}

#[test]
fn bound_via_env_is_ok() {
    let scope = bound_scope("my-ws", ScopeSource::Env);
    current::run(&scope, Mode::Human).expect("env-bound scope is ok");
}

#[test]
fn bound_via_config_is_ok() {
    let scope = bound_scope("my-ws", ScopeSource::Config);
    current::run(&scope, Mode::Human).expect("config-bound scope is ok");
}

#[test]
fn bound_via_project_marker_is_ok() {
    let scope = bound_scope("proj-ws", ScopeSource::ProjectMarker);
    current::run(&scope, Mode::Json).expect("marker-bound scope is ok");
}

// ---------------------------------------------------------------------------
// CLI binary — the prompt/script contract (stdout content + exit codes).
// ---------------------------------------------------------------------------

/// Bound (via the always-present `global` workspace, selected with the
/// `--workspace` flag): human mode prints JUST the name on one line, no
/// decoration, exit 0.
#[test]
fn cli_bound_prints_only_the_name() {
    let env = ToolEnv::new();
    let output = env
        .cmd()
        .args(["--workspace", "global", "workspace", "current"])
        .output()
        .expect("spawn tome");
    assert!(
        output.status.success(),
        "exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // JUST the name on one line — no labels, no bookshelf, no key/value.
    assert_eq!(
        stdout, "global\n",
        "stdout must be the bare name; got {stdout:?}"
    );
}

/// Unbound (a scratch dir with no marker, no flag, no env): exit non-zero
/// (12), EMPTY stdout, and a diagnostic on stderr — the
/// `$(tome workspace current 2>/dev/null)` prompt contract.
#[test]
fn cli_unbound_exits_12_with_empty_stdout() {
    let env = ToolEnv::new();
    let scratch = TempDir::new().unwrap();
    let output = env
        .cmd()
        // Clear any ambient TOME_WORKSPACE (resolver step 2) so the resolver
        // is guaranteed to reach GlobalFallback — the test must not depend on
        // the host env. `ToolEnv::cmd()` isolates $HOME but not this var.
        .env_remove("TOME_WORKSPACE")
        .current_dir(scratch.path())
        .args(["workspace", "current"])
        .output()
        .expect("spawn tome");
    assert_eq!(output.status.code(), Some(12), "unbound must exit 12");
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty so `2>/dev/null` yields nothing; got {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    // The actionable message lands on stderr, not stdout.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no workspace is bound to the current directory")
            && stderr.contains("tome workspace use"),
        "stderr must carry the actionable diagnostic; got {stderr:?}",
    );
}

/// `--json` emits a stable single-line record with the documented fields.
#[test]
fn cli_json_shape_is_stable() {
    let env = ToolEnv::new();
    let output = env
        .cmd()
        .args(["--json", "--workspace", "global", "workspace", "current"])
        .output()
        .expect("spawn tome");
    assert!(output.status.success(), "exit={:?}", output.status.code());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Byte-stable field order pin (mirrors `workspace info --json`).
    assert_eq!(
        stdout.trim_end(),
        r#"{"workspace":"global","scope":"global","source":"flag"}"#,
        "json shape drifted; got {stdout}",
    );
    // And it parses as one object with the expected values.
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(parsed["workspace"], "global");
    assert_eq!(parsed["scope"], "global");
    assert_eq!(parsed["source"], "flag");
}

/// `--json` on the unbound case emits the structured error envelope (never a
/// success record) and exits 12.
#[test]
fn cli_json_unbound_emits_error_envelope() {
    let env = ToolEnv::new();
    let scratch = TempDir::new().unwrap();
    let output = env
        .cmd()
        // Clear ambient TOME_WORKSPACE so the resolver reaches GlobalFallback
        // regardless of the host env (see the human-mode test above).
        .env_remove("TOME_WORKSPACE")
        .current_dir(scratch.path())
        .args(["--json", "workspace", "current"])
        .output()
        .expect("spawn tome");
    assert_eq!(output.status.code(), Some(12), "unbound must exit 12");
    assert!(output.stdout.is_empty(), "no success record on stdout");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr is the json error envelope");
    assert_eq!(parsed["error"]["category"], "workspace_not_bound");
    assert_eq!(parsed["error"]["exit_code"], 12);
    // The clean, actionable message rides in the envelope for scripts.
    let msg = parsed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("no workspace is bound to the current directory"),
        "envelope message must be the actionable one; got {msg:?}",
    );
}
