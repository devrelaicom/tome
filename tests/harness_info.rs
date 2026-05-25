//! Library-API tests for `tome harness info <name>`.

mod common;

use common::{ToolEnv, paths_for};
use tome::cli::HarnessInfoArgs;
use tome::commands::harness::info;
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
fn info_for_unknown_harness_returns_exit_18() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessInfoArgs {
        name: "not-a-real-harness".to_string(),
    };
    let scope = fallback_scope();
    let prev_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", env.home_path());
    }
    let err = info::run(args, &scope, &paths, Mode::Json).expect_err("unknown");
    unsafe {
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    assert_eq!(err.exit_code(), 18);
}

#[test]
fn info_for_real_harness_runs_without_project() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessInfoArgs {
        name: "claude-code".to_string(),
    };
    let scope = fallback_scope();
    let prev_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", env.home_path());
    }
    let result = info::run(args, &scope, &paths, Mode::Json);
    unsafe {
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    assert!(result.is_ok(), "info run: {result:?}");
}

#[test]
fn info_reports_direct_scope_when_global_declares() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.
    std::fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .unwrap();

    let args = HarnessInfoArgs {
        name: "claude-code".to_string(),
    };
    let scope = fallback_scope();
    let prev_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", env.home_path());
    }
    let result = info::run(args, &scope, &paths, Mode::Human);
    unsafe {
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    assert!(result.is_ok(), "info run: {result:?}");
}
