//! Issue #321 — `workspace` command arg ergonomics. CLI-driven tests over
//! the compiled binary (a non-TTY subprocess), exercising:
//!
//! - `workspace use --create <name>`  — creates + binds in one step.
//! - `workspace use <name>`           — unchanged (still binds; still exit 0).
//! - `workspace use` (no name)        — refuses on a non-terminal (picker path).
//! - `workspace init --bind`          — creates + binds `$CWD`.
//! - `workspace init <name>`          — unchanged (no bind).
//! - `workspace regen-summary` (no name, non-TTY) — refuses; never regenerates.
//!
//! The Cargo test harness runs the child with piped (non-terminal) stdio, so
//! every prompt-bearing path lands on the non-interactive branch — exactly the
//! contract we want to pin for CI/scripts. The interactive picker/confirm
//! *selection* logic itself is unit-tested against `prompt::select`/`confirm`
//! in `presentation::prompt` (they short-circuit `NotATerminal` under the
//! harness).

use crate::common::ToolEnv;
use tempfile::TempDir;

/// `workspace use --create <name>` creates the workspace then binds `$CWD`.
/// The `--create` JSON carries `"created":true`; the workspace is now bound.
#[test]
fn use_create_creates_and_binds() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["--json", "workspace", "use", "--create", "proj-ws"])
        .output()
        .expect("spawn tome");
    assert!(
        output.status.success(),
        "use --create must succeed; exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("json parse: {e}; {stdout}"));
    assert_eq!(parsed["workspace"], "proj-ws", "bound workspace name");
    assert_eq!(parsed["created"], true, "created flag set; got {stdout}");

    // The marker now resolves the project to `proj-ws`.
    let current = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["workspace", "current"])
        .output()
        .expect("spawn tome current");
    assert!(current.status.success(), "current after bind must succeed");
    assert_eq!(
        String::from_utf8(current.stdout).unwrap(),
        "proj-ws\n",
        "current directory now resolves to the created workspace",
    );
}

/// `--create` is idempotent: creating over an existing workspace is not an
/// error — it falls through to the bind. `created` is absent/false because no
/// creation happened this time.
#[test]
fn use_create_is_idempotent_over_existing() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();

    // Pre-create the workspace with a plain init.
    let init = env
        .cmd()
        .args(["workspace", "init", "already-there"])
        .output()
        .expect("spawn init");
    assert!(init.status.success(), "pre-init must succeed");

    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["--json", "workspace", "use", "--create", "already-there"])
        .output()
        .expect("spawn tome");
    assert!(
        output.status.success(),
        "use --create over existing must succeed (idempotent); exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");
    assert_eq!(parsed["workspace"], "already-there");
    // No creation happened → `created` elided (defaults false).
    assert!(
        parsed.get("created").is_none() || parsed["created"] == false,
        "created must be absent/false on idempotent path; got {stdout}",
    );
}

/// `workspace use <name>` (no `--create`) still binds an existing workspace,
/// exit 0, with NO `created` field — byte-back-compat.
#[test]
fn use_named_without_create_is_unchanged() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();

    let init = env
        .cmd()
        .args(["workspace", "init", "plain-ws"])
        .output()
        .expect("spawn init");
    assert!(init.status.success(), "pre-init must succeed");

    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["--json", "workspace", "use", "plain-ws"])
        .output()
        .expect("spawn tome");
    assert!(output.status.success(), "use <name> must succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\"created\""),
        "pre-#321 use <name> JSON must not carry created; got {stdout}",
    );
}

/// `workspace use` with NO name on a non-terminal refuses (the picker path is
/// terminal-only) — exit 54 (`NotATerminal`), NOT a silent bind of a default.
#[test]
fn use_no_name_non_tty_refuses() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["workspace", "use"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        output.status.code(),
        Some(54),
        "no-name use on a non-terminal must refuse with NotATerminal (54); stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// `workspace use --create` WITHOUT a name is a usage error (exit 2): you
/// can't create an unnamed workspace.
#[test]
fn use_create_without_name_is_usage_error() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["workspace", "use", "--create"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        output.status.code(),
        Some(2),
        "use --create with no name must be a usage error (2); stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// `workspace init --bind` creates the workspace AND binds `$CWD` — the mirror
/// of `use --create`. The `--bind` JSON carries `"bound":true`; the project is
/// bound afterwards.
#[test]
fn init_bind_creates_and_binds() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["--json", "workspace", "init", "--bind", "bound-ws"])
        .output()
        .expect("spawn tome");
    assert!(
        output.status.success(),
        "init --bind must succeed; exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");
    assert_eq!(parsed["name"], "bound-ws");
    assert_eq!(parsed["bound"], true, "bound flag set; got {stdout}");

    // The project now resolves to the created workspace.
    let current = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["workspace", "current"])
        .output()
        .expect("spawn current");
    assert!(current.status.success());
    assert_eq!(
        String::from_utf8(current.stdout).unwrap(),
        "bound-ws\n",
        "init --bind binds the current directory",
    );
}

/// `workspace init <name>` (no `--bind`) is unchanged: creates the workspace,
/// does NOT bind `$CWD`, and the JSON carries no `bound` field.
#[test]
fn init_without_bind_is_unchanged() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["--json", "workspace", "init", "unbound-ws"])
        .output()
        .expect("spawn tome");
    assert!(output.status.success(), "init <name> must succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("\"bound\""),
        "pre-#321 init <name> JSON must not carry bound; got {stdout}",
    );

    // `$CWD` is NOT bound → resolving current there falls back (exit 12).
    let current = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["workspace", "current"])
        .output()
        .expect("spawn current");
    assert_eq!(
        current.status.code(),
        Some(12),
        "init without --bind must NOT bind the current directory",
    );
}

/// `workspace regen-summary` with NO name on a non-terminal refuses — it never
/// silently regenerates the resolved (often `global`) scope. The refusal is a
/// loud non-zero exit with an actionable message; it does NOT run the
/// summariser (so no ONNX/model download is triggered).
#[test]
fn regen_summary_no_name_non_tty_refuses() {
    let env = ToolEnv::new();
    let proj = TempDir::new().unwrap();
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(proj.path())
        .args(["workspace", "regen-summary"])
        .output()
        .expect("spawn tome");
    let code = output.status.code();
    assert!(
        code == Some(54) || code == Some(2),
        "no-name regen-summary on a non-terminal must refuse (54 or 2); got {code:?} stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );
    // It must NOT have emitted a success "Regenerated summary" line.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Regenerated summary"),
        "refusal must not regenerate; got stdout {stdout}",
    );
}

/// True if a workspace named `name` appears in `tome workspace list --json`
/// (a bare array of `{name, …}` rows). Used by the orphan-prevention tests.
fn workspace_exists(env: &ToolEnv, name: &str) -> bool {
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .args(["--json", "workspace", "list"])
        .output()
        .expect("spawn tome list");
    assert!(
        output.status.success(),
        "workspace list must succeed; exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let rows: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("list json: {e}; {stdout}"));
    rows.as_array()
        .expect("list is a JSON array")
        .iter()
        .any(|r| r["name"] == name)
}

/// All-or-nothing regression: `use --create <name>` run from a DANGEROUS CWD
/// (the test's `$HOME`) WITHOUT `--force` must refuse (exit 2) and create
/// NOTHING — the guard runs before the create step, so no orphan
/// created-but-unbound workspace is left behind.
#[test]
fn use_create_at_dangerous_cwd_refuses_and_creates_nothing() {
    let env = ToolEnv::new();
    // `$HOME` is the dangerous CWD. `ToolEnv::cmd()` sets HOME to this path,
    // and the guard canonicalises both sides — running from `$HOME` trips the
    // "refusing to bind … home directory" refusal (exit 2).
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(env.home_path())
        .args(["workspace", "use", "--create", "orphan-ws"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        output.status.code(),
        Some(2),
        "use --create at $HOME without --force must refuse (2); stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        !workspace_exists(&env, "orphan-ws"),
        "refused use --create must NOT have created the workspace (no orphan)",
    );
}

/// Mirror of the above for `init --bind`: a dangerous CWD without `--force`
/// must refuse before `init` creates the workspace, leaving no orphan. (`init`
/// has no `--force`; the refusal is intentional — rerun `workspace use --force`
/// to bind a genuinely-unusual root.)
#[test]
fn init_bind_at_dangerous_cwd_refuses_and_creates_nothing() {
    let env = ToolEnv::new();
    let output = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(env.home_path())
        .args(["workspace", "init", "--bind", "orphan-init-ws"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        output.status.code(),
        Some(2),
        "init --bind at $HOME must refuse (2); stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        !workspace_exists(&env, "orphan-init-ws"),
        "refused init --bind must NOT have created the workspace (no orphan)",
    );

    // And plain `init <name>` at the SAME dangerous CWD is UNCHANGED — it is
    // never guarded, so it succeeds and creates the workspace.
    let plain = env
        .cmd()
        .env_remove("TOME_WORKSPACE")
        .current_dir(env.home_path())
        .args(["workspace", "init", "plain-at-home"])
        .output()
        .expect("spawn tome");
    assert!(
        plain.status.success(),
        "plain init (no --bind) must be unchanged / never guarded; exit={:?} stderr={}",
        plain.status.code(),
        String::from_utf8_lossy(&plain.stderr),
    );
    assert!(
        workspace_exists(&env, "plain-at-home"),
        "plain init must have created the workspace",
    );
}
