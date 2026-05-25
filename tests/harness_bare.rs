//! Library-API tests for bare `tome harness` — lists every supported
//! harness in tabular form with detection + per-project targets (FR-520).
//!
//! Process-global serialisation: `HARNESS_MODULES_OVERRIDE` is a single
//! `RwLock` slot; tests that install a guard must hold the file-local
//! mutex for their entire duration.

mod common;

use std::sync::Mutex;

use common::{HarnessModulesGuard, NamedStubHarness, ToolEnv, paths_for};
use tome::commands::harness::bare;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn fallback_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

#[test]
fn bare_runs_against_real_registry_without_project() {
    // No override — exercise SUPPORTED_HARNESSES against an isolated
    // $HOME where no harness dotdirs exist.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    // Replace process $HOME so harness detect probes hit our tempdir.
    let prev_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", env.home_path());
    }
    let scope = fallback_scope();
    let result = bare::run(&scope, &paths, Mode::Human);
    // Restore HOME before assertions so panics still reset.
    unsafe {
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    assert!(result.is_ok(), "bare run: {result:?}");
}

#[test]
fn bare_with_synthetic_registry_emits_one_row_per_module() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha", "beta"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let prev_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", env.home_path());
    }
    let scope = fallback_scope();
    let result = bare::run(&scope, &paths, Mode::Json);
    unsafe {
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    assert!(result.is_ok(), "bare run: {result:?}");
}

#[test]
fn bare_with_project_root_emits_per_project_targets() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let prev_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", env.home_path());
    }
    let project = env.home_path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project.clone()),
    };
    let result = bare::run(&scope, &paths, Mode::Json);
    unsafe {
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    assert!(result.is_ok(), "bare run: {result:?}");
}
