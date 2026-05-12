//! Integration tests for the Phase 2 additions to `Paths::resolve()` —
//! `index_db`, `index_lock`, `models_dir`, and the `model_path` /
//! `model_manifest` helpers.
//!
//! `Paths::resolve()` reads `HOME`, `XDG_CONFIG_HOME`, and `XDG_DATA_HOME`
//! from the environment, so this suite runs single-threaded against a
//! `Mutex` to avoid env interference between tests. The Phase 1 suite
//! already established this pattern.

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
fn resolve_places_phase2_paths_under_xdg_data_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[
        ("HOME", "/tmp/fake-home"),
        ("XDG_CONFIG_HOME", "/tmp/fake-cfg"),
        ("XDG_DATA_HOME", "/tmp/fake-data"),
    ]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(p.data_dir, PathBuf::from("/tmp/fake-data/tome"));
    assert_eq!(p.index_db, PathBuf::from("/tmp/fake-data/tome/index.db"));
    assert_eq!(
        p.index_lock,
        PathBuf::from("/tmp/fake-data/tome/index.lock")
    );
    assert_eq!(p.models_dir, PathBuf::from("/tmp/fake-data/tome/models"));
}

#[test]
fn resolve_falls_back_to_default_xdg_data_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[
        ("HOME", "/tmp/fake-home"),
        ("XDG_CONFIG_HOME", ""),
        ("XDG_DATA_HOME", ""),
    ]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(
        p.index_db,
        PathBuf::from("/tmp/fake-home/.local/share/tome/index.db"),
    );
}

#[test]
fn model_path_keeps_path_inside_models_dir() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h"), ("XDG_DATA_HOME", "/tmp/d")]);

    let p = Paths::resolve().expect("resolve");
    let got = p.model_path("bge-small-en-v1.5").expect("ok");
    assert_eq!(got, PathBuf::from("/tmp/d/tome/models/bge-small-en-v1.5"));
}

#[test]
fn model_path_rejects_separators_and_traversal() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h"), ("XDG_DATA_HOME", "/tmp/d")]);

    let p = Paths::resolve().expect("resolve");
    for bad in ["", ".", "..", "a/b", "a\\b", "/abs"] {
        assert!(
            p.model_path(bad).is_err(),
            "model_path({bad:?}) should error",
        );
    }
}
