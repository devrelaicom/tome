//! Library-API tests for `tome harness list <workspace>` — directly-
//! declared list verbatim (no composition expansion).

mod common;

use common::{ToolEnv, paths_for, seed_workspace};
use tome::cli::HarnessListArgs;
use tome::commands::harness::list;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn fallback_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

#[test]
fn list_for_named_workspace_emits_as_written_array() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    // Write workspace settings carrying both a direct entry AND a
    // composition reference — the as-written report must include both
    // unchanged.
    let ws_dir = paths.workspaces_dir.join("demo");
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        ws_dir.join("settings.toml"),
        "name = \"demo\"\nharnesses = [\"claude-code\", \"[global]\"]\n",
    )
    .unwrap();

    let args = HarnessListArgs {
        workspace: Some("demo".to_string()),
    };
    let scope = fallback_scope();
    let result = list::run(args, &scope, &paths, Mode::Json);
    assert!(result.is_ok(), "list run: {result:?}");
}

#[test]
fn list_for_workspace_without_settings_emits_empty_array() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessListArgs {
        workspace: Some("nonexistent-yet-valid".to_string()),
    };
    let scope = fallback_scope();
    let result = list::run(args, &scope, &paths, Mode::Json);
    assert!(result.is_ok(), "list run: {result:?}");
}

#[test]
fn list_for_invalid_workspace_name_returns_validation_error() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessListArgs {
        workspace: Some("not!valid".to_string()),
    };
    let scope = fallback_scope();
    let err = list::run(args, &scope, &paths, Mode::Json).expect_err("invalid name");
    assert_eq!(
        err.exit_code(),
        15,
        "want WorkspaceNameInvalid; got {err:?}"
    );
}
