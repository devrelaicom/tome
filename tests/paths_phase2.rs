//! Integration tests for `Paths::resolve()` — the Phase 4 single-root
//! layout. Phase 1–3's XDG-separated fields are gone; everything lives
//! under `<home>/.tome/`.
//!
//! `Paths::resolve()` reads `$HOME`, so this suite runs single-threaded
//! against a `Mutex` to avoid env interference between tests. The same
//! pattern was used in Phase 1–3.

use std::path::PathBuf;
use std::sync::Mutex;

use tome::paths::Paths;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    keys: Vec<&'static str>,
    prior: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvGuard {
    fn set(keys_values: &[(&'static str, &str)]) -> Self {
        let prior = keys_values
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in keys_values {
            // SAFETY: Tests guard env mutation behind ENV_LOCK; no other
            // threads observe the transient state.
            unsafe {
                std::env::set_var(k, v);
            }
        }
        Self {
            keys: keys_values.iter().map(|(k, _)| *k).collect(),
            prior,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (i, key) in self.keys.iter().enumerate() {
            // SAFETY: under ENV_LOCK.
            unsafe {
                match self.prior.get(i).and_then(|(_, v)| v.clone()) {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

#[test]
fn resolve_places_every_path_under_home_dot_tome() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/fake-home")]);

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
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h")]);

    let p = Paths::resolve().expect("resolve");
    let got = p.model_path("bge-small-en-v1.5").expect("ok");
    assert_eq!(got, PathBuf::from("/tmp/h/.tome/models/bge-small-en-v1.5"));
}

#[test]
fn model_path_rejects_separators_and_traversal() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h")]);

    let p = Paths::resolve().expect("resolve");
    for bad in ["", ".", "..", "a/b", "a\\b", "/abs"] {
        assert!(
            p.model_path(bad).is_err(),
            "model_path({bad:?}) should error",
        );
    }
}
