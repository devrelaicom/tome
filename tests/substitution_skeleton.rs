//! Phase 5 / F3 — substitution module skeleton smoke tests.
//!
//! Confirms the module compiles, the public API surface is reachable,
//! the override seams are visible to integration tests, and the three
//! RAII guards install + clear correctly.
//!
//! Production behaviour (the four-stage pipeline) lands in US1+US2+US3;
//! at F3 every stage returns the body unchanged.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use common::{ClockOverrideGuard, PluginDataDirGuard, WorkspaceDataDirGuard, lifecycle_paths};
use time::OffsetDateTime;
use tome::substitution::{self, ArgumentValues, SubstitutionContext};

fn dummy_context(home: &std::path::Path) -> SubstitutionContext {
    let paths = lifecycle_paths(home);
    SubstitutionContext::builder()
        .catalog_name("test-catalog")
        .plugin_name("test-plugin")
        .plugin_version("1.0.0")
        .entry_name("hello")
        .entry_path(PathBuf::from("/tmp/x/hello.md"))
        .entry_dir(PathBuf::from("/tmp/x"))
        .plugin_root_dir(PathBuf::from("/tmp/x"))
        .workspace_name("global")
        .clock(OffsetDateTime::UNIX_EPOCH)
        .paths(paths)
        .build()
        .expect("builder")
}

#[test]
fn render_returns_body_unchanged_in_skeleton() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = dummy_context(tmp.path());
    let body = "Hello, world! Static body with no placeholders.";
    let out = substitution::render(body, &ctx).expect("skeleton render");
    assert_eq!(out, body);
}

#[test]
fn render_with_args_returns_body_unchanged_in_skeleton() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ctx = dummy_context(tmp.path());
    ctx.args = Some(ArgumentValues::Single("hello world".to_string()));
    let body = "Body that the skeleton will not transform.";
    let out = substitution::render(body, &ctx).expect("skeleton render");
    assert_eq!(out, body);
}

#[test]
fn argument_values_object_constructs_without_panic() {
    let mut named = HashMap::new();
    named.insert("first".to_string(), "alice".to_string());
    let _av = ArgumentValues::Object {
        named,
        declared_order: vec!["first".to_string()],
    };
}

#[test]
fn builder_reports_missing_required_field() {
    // Build with no setters at all — the first missing field is reported.
    // `SubstitutionContext` deliberately does not implement Debug (it
    // carries `Paths`, which historically does, but the substitution
    // context grew without a Debug derive), so we can't use
    // `expect_err`; match the Result instead.
    match SubstitutionContext::builder().build() {
        Ok(_) => panic!("expected builder to fail with missing required fields"),
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("builder missing required field"),
                "unexpected error message: {msg}"
            );
        }
    }
}

#[test]
fn clock_override_guard_installs_and_clears() {
    let now = OffsetDateTime::UNIX_EPOCH;
    {
        let _g = ClockOverrideGuard::install(now);
        let cell = tome::substitution::SUBSTITUTION_CLOCK_OVERRIDE
            .get()
            .expect("initialised by guard");
        let v = cell.lock().unwrap_or_else(|e| e.into_inner());
        assert!(v.is_some(), "guard installs value");
    }
    let cell = tome::substitution::SUBSTITUTION_CLOCK_OVERRIDE
        .get()
        .expect("cell exists after first install");
    let v = cell.lock().unwrap_or_else(|e| e.into_inner());
    assert!(v.is_none(), "guard clears value on Drop");
}

#[test]
fn plugin_data_dir_override_guard_installs_and_clears() {
    let p = PathBuf::from("/tmp/override-pd");
    {
        let _g = PluginDataDirGuard::install(p.clone());
        let cell = tome::substitution::PLUGIN_DATA_DIR_OVERRIDE.get().unwrap();
        assert_eq!(
            cell.lock().unwrap_or_else(|e| e.into_inner()).as_ref(),
            Some(&p)
        );
    }
    let cell = tome::substitution::PLUGIN_DATA_DIR_OVERRIDE.get().unwrap();
    assert!(cell.lock().unwrap_or_else(|e| e.into_inner()).is_none());
}

#[test]
fn workspace_data_dir_override_guard_installs_and_clears() {
    let p = PathBuf::from("/tmp/override-wd");
    {
        let _g = WorkspaceDataDirGuard::install(p.clone());
        let cell = tome::substitution::WORKSPACE_DATA_DIR_OVERRIDE
            .get()
            .unwrap();
        assert_eq!(
            cell.lock().unwrap_or_else(|e| e.into_inner()).as_ref(),
            Some(&p)
        );
    }
    let cell = tome::substitution::WORKSPACE_DATA_DIR_OVERRIDE
        .get()
        .unwrap();
    assert!(cell.lock().unwrap_or_else(|e| e.into_inner()).is_none());
}
