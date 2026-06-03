//! Integration tests for `Paths::resolve()` — the Phase 4 single-root
//! layout. Phase 1–3's XDG-separated fields are gone; everything lives
//! under `<home>/.tome/`.
//!
//! `Paths::resolve()` reads `$HOME`, so this suite runs serialised
//! against the project-wide `HOME_MUTEX` via `HomeGuard`. PR-E T-M8
//! collapsed the per-file `ENV_LOCK` + `EnvGuard` (the two `unsafe`
//! `set_var` blocks they carried) onto the shared helper used by 9
//! other test files.

use std::path::PathBuf;

use tome::paths::Paths;

use crate::common::HomeGuard;

#[test]
fn resolve_places_every_path_under_home_dot_tome() {
    let _guard = HomeGuard::install(std::path::Path::new("/tmp/fake-home"));

    let p = Paths::resolve().expect("resolve");
    assert_eq!(p.root, PathBuf::from("/tmp/fake-home/.tome"));
    assert_eq!(p.index_db, PathBuf::from("/tmp/fake-home/.tome/index.db"));
    assert_eq!(
        p.index_lock,
        PathBuf::from("/tmp/fake-home/.tome/index.lock"),
    );
    assert_eq!(p.models_dir, PathBuf::from("/tmp/fake-home/.tome/models"));
    assert_eq!(
        p.global_config_file,
        PathBuf::from("/tmp/fake-home/.tome/config.toml"),
    );
}

#[test]
fn model_path_keeps_path_inside_models_dir() {
    let _guard = HomeGuard::install(std::path::Path::new("/tmp/h"));

    let p = Paths::resolve().expect("resolve");
    let got = p.model_path("bge-small-en-v1.5").expect("ok");
    assert_eq!(got, PathBuf::from("/tmp/h/.tome/models/bge-small-en-v1.5"));
}

#[test]
fn model_path_rejects_separators_and_traversal() {
    let _guard = HomeGuard::install(std::path::Path::new("/tmp/h"));

    let p = Paths::resolve().expect("resolve");
    for bad in ["", ".", "..", "a/b", "a\\b", "/abs"] {
        assert!(
            p.model_path(bad).is_err(),
            "model_path({bad:?}) should error",
        );
    }
}
