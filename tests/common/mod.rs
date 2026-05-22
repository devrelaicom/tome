//! Shared test harness for the catalog command integration suites. Each test
//! builds a fresh fixture catalog inside a `tempfile::TempDir`, runs
//! `git init && git add -A && git commit -m init` against it (so it has a
//! HEAD), and constructs `Command` invocations of the `tome` binary with
//! isolated `HOME`/`XDG_*` so the host's real config is never touched.
//!
//! All paths are absolute. No mocking of git or the filesystem.

#![allow(dead_code)] // each test file uses a subset of these helpers

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use time::OffsetDateTime;
use tome::config::{CatalogEntry, Config};
use tome::embedding::registry::{MODEL_REGISTRY, ModelManifest};
use tome::index::MetaSeed;
use tome::paths::Paths;

/// Build a self-contained Git fixture catalog from the on-disk
/// `tests/fixtures/sample-catalog/` skeleton. Returns the temp dir handle
/// (must stay alive for the lifetime of the test) and a `file://` URL the
/// `tome` binary can clone from.
pub struct Fixture {
    pub tempdir: TempDir,
    pub repo_path: PathBuf,
    pub url: String,
}

impl Fixture {
    pub fn build_sample() -> Self {
        Self::build_from(fixture_path("sample-catalog"))
    }

    pub fn build_from(skeleton: PathBuf) -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let repo_path = tempdir.path().join("catalog");
        copy_dir(&skeleton, &repo_path).expect("copy skeleton");
        // We need real plugin directories — git won't track empty ones, and
        // `.keep` files are inside them so they materialise as soon as
        // they're copied.
        git_init_and_commit(&repo_path);
        let url = format!("file://{}", repo_path.display());
        Self {
            tempdir,
            repo_path,
            url,
        }
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn git_init_and_commit(repo: &Path) {
    let run = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            // Suppress identity prompts in CI.
            .env("GIT_AUTHOR_NAME", "Tome Test")
            .env("GIT_AUTHOR_EMAIL", "tests@tome.invalid")
            .env("GIT_COMMITTER_NAME", "Tome Test")
            .env("GIT_COMMITTER_EMAIL", "tests@tome.invalid")
            .status()
            .unwrap_or_else(|e| panic!("git {:?}: {}", args, e));
        assert!(status.success(), "git {:?} exited {}", args, status);
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

/// Isolated environment for invoking the `tome` binary. Each test gets a
/// fresh XDG layout so the host config is never touched.
pub struct ToolEnv {
    pub home: TempDir,
}

impl ToolEnv {
    pub fn new() -> Self {
        Self {
            home: TempDir::new().expect("tool env home"),
        }
    }

    pub fn home_path(&self) -> &Path {
        self.home.path()
    }

    /// Phase 4: the on-disk root for every Tome path inside the isolated
    /// `$HOME`. Tests previously named this `config_dir` / `data_dir`
    /// before the F2a collapse; the single accessor matches the new layout.
    pub fn tome_root(&self) -> PathBuf {
        self.home.path().join(".tome")
    }

    pub fn config_file(&self) -> PathBuf {
        self.tome_root().join("config.toml")
    }

    pub fn catalogs_dir(&self) -> PathBuf {
        self.tome_root().join("catalogs")
    }

    /// Build a `Command` for the compiled `tome` binary, pre-populated with
    /// the isolated env.
    pub fn cmd(&self) -> Command {
        let mut cmd = Command::new(tome_bin());
        cmd.env("HOME", self.home.path())
            .env_remove("TOME_LOG")
            .env_remove("RUST_LOG");
        cmd
    }
}

fn tome_bin() -> PathBuf {
    // Cargo points `CARGO_BIN_EXE_<name>` at the freshly-built binary for
    // the package; integration tests get this for free.
    PathBuf::from(env!("CARGO_BIN_EXE_tome"))
}

// ---------------------------------------------------------------------------
// Phase 3 (US1) lifecycle helpers.
//
// These mirror the in-module test scaffolding from `src/plugin/lifecycle.rs`
// so integration tests can drive the lifecycle library API directly without
// spawning the CLI binary (which loads the real ONNX models).
// ---------------------------------------------------------------------------

/// Path to the `tests/fixtures/sample-plugin-catalog/` skeleton on disk.
/// Tests that need a catalog of plugins copy this into a temp dir.
pub fn sample_plugin_catalog_fixture() -> PathBuf {
    fixture_path("sample-plugin-catalog")
}

/// Copy `sample-plugin-catalog` into the supplied TempDir and return the
/// catalog root path (the directory containing `tome-catalog.toml`).
pub fn copy_sample_plugin_catalog(into: &TempDir, name: &str) -> PathBuf {
    let dst = into.path().join(name);
    copy_dir(&sample_plugin_catalog_fixture(), &dst).expect("copy sample-plugin-catalog");
    dst
}

/// Build a `Paths` rooted entirely under `root`. Mirrors the helper used by
/// `lifecycle::tests::test_paths` so integration tests never have to touch
/// `$HOME` or environment variables. F2a collapses everything under one
/// root, so this is now a thin wrapper over [`Paths::from_root`].
pub fn lifecycle_paths(root: &Path) -> Paths {
    Paths::from_root(root.to_path_buf())
}

/// `MetaSeed` matching the deterministic stub embedder.
pub fn stub_embedder_seed() -> MetaSeed {
    MetaSeed {
        name: "stub-embedder".into(),
        version: "0".into(),
    }
}

/// `MetaSeed` matching the deterministic stub reranker.
pub fn stub_reranker_seed() -> MetaSeed {
    MetaSeed {
        name: "stub-reranker".into(),
        version: "0".into(),
    }
}

/// `MetaSeed` matching the deterministic stub summariser. Phase 4 / F9
/// added a third identity row to `meta` + a third field to
/// `OpenOptions`; the stub seed mirrors the embedder/reranker shape so
/// integration tests don't have to know about the registry's
/// placeholder values.
pub fn stub_summariser_seed() -> MetaSeed {
    MetaSeed {
        name: "stub-summariser".into(),
        version: "0".into(),
    }
}

/// Fabricate `manifest.json` for every entry in `MODEL_REGISTRY` so the
/// model-presence gate in `lifecycle::enable` is satisfied without a real
/// download. Mirrors `src/plugin/lifecycle.rs::tests::fabricate_models`.
pub fn fabricate_models(paths: &Paths) {
    for entry in MODEL_REGISTRY {
        let dir = paths.models_dir.join(entry.name);
        std::fs::create_dir_all(&dir).expect("create model dir");
        let manifest = ModelManifest {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            kind: entry.kind,
            source_url: entry.source_url.to_owned(),
            sha256: entry.sha256.to_owned(),
            size_bytes: entry.size_bytes,
            licence: entry.licence.to_owned(),
            files: entry.files.iter().map(|s| (*s).to_owned()).collect(),
            installed_at: OffsetDateTime::now_utc(),
        };
        let body = serde_json::to_vec_pretty(&manifest).expect("serialise manifest");
        std::fs::write(dir.join("manifest.json"), body).expect("write manifest");
    }
}

/// Fabricate fully-installed registered models on disk: writes each
/// entry's `manifest.json` AND a sparse artefact file sized to
/// `entry.size_bytes`. Sparse files (`File::set_len`) take ~no disk
/// space on Linux and macOS, so even the ~400 MB summariser fixture
/// is essentially free. Auxiliary files (tokenizer.json etc.) get a
/// 1-byte sparse file — present + non-empty satisfies the existence
/// half of `models::cheap_state`. The bytes are all-zero, so the
/// SHA-256 does NOT match each registry pin — `models list --verify`
/// uses this to flip the state to `checksum_mismatched`.
///
/// Pass `MODEL_REGISTRY` (or any sub-slice) to fabricate every entry,
/// or build a one-element slice for a single-model fabrication. The
/// consolidation from the older `fabricate_installed_model` /
/// `fabricate_all_registry_models` pair into this single plural
/// surface is the P6-deferred rename from Phase 3 — research §R-17.
pub fn fabricate_installed_models(
    paths: &Paths,
    entries: &[&tome::embedding::registry::ModelEntry],
) {
    for entry in entries {
        let dir = paths.models_dir.join(entry.name);
        std::fs::create_dir_all(&dir).expect("create model dir");
        let manifest = ModelManifest {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            kind: entry.kind,
            source_url: entry.source_url.to_owned(),
            sha256: entry.sha256.to_owned(),
            size_bytes: entry.size_bytes,
            licence: entry.licence.to_owned(),
            files: entry.files.iter().map(|s| (*s).to_owned()).collect(),
            installed_at: OffsetDateTime::now_utc(),
        };
        let body = serde_json::to_vec_pretty(&manifest).expect("serialise manifest");
        std::fs::write(dir.join("manifest.json"), body).expect("write manifest");

        for (i, file) in entry.files.iter().enumerate() {
            let path = dir.join(file);
            let f = std::fs::File::create(&path).expect("create artefact file");
            let len = if i == 0 { entry.size_bytes } else { 1 };
            f.set_len(len).expect("set_len for sparse artefact");
        }
    }
}

/// Convenience: fabricate every entry in `MODEL_REGISTRY`. Mirrors the
/// most common call pattern of the old `fabricate_all_registry_models`.
pub fn fabricate_all_registry_models(paths: &Paths) {
    let entries: Vec<&tome::embedding::registry::ModelEntry> = MODEL_REGISTRY.iter().collect();
    fabricate_installed_models(paths, &entries);
}

/// Construct a minimal `Config` containing one catalog whose on-disk cache
/// lives at `catalog_root`. The catalog `name` is recorded both as the
/// `BTreeMap` key and the inner `CatalogEntry.name` so lookups via the CLI
/// surface match the lifecycle library API.
pub fn config_with_catalog(catalog_name: &str, catalog_root: &Path) -> Config {
    use std::collections::BTreeMap;
    let mut catalogs = BTreeMap::new();
    catalogs.insert(
        catalog_name.to_owned(),
        CatalogEntry {
            name: catalog_name.to_owned(),
            url: format!("file://{}", catalog_root.display()),
            ref_: "main".into(),
            path: catalog_root.to_path_buf(),
            last_synced: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        },
    );
    Config { catalogs }
}

/// Write the supplied [`Config`] to `paths.global_config_file` as TOML so a
/// child `tome` binary process can read it. Used by `plugin list` /
/// `plugin show` integration tests that bypass `catalog add` (no git fixture
/// needed).
pub fn write_config_for_cli(paths: &Paths, config: &Config) {
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    let body = toml::to_string_pretty(config).expect("serialise config");
    std::fs::write(&paths.global_config_file, body).expect("write config.toml");
}

/// Build a minimal index database on disk with `meta.schema_version` stamped
/// at the supplied value. The DB has *only* the `meta` table (no `skills`,
/// no `skill_embeddings`) so the schema-migration e2e tests can register
/// synthetic migrations that create whatever tables they need without
/// conflicting with the production v1 schema.
///
/// `meta` is created with the same shape as production (STRICT, `key TEXT
/// PRIMARY KEY`). One row is inserted: `('schema_version', '<version>')`.
/// The connection is dropped before returning so the on-disk file is fully
/// flushed.
pub fn write_index_db_with_schema_version(path: &Path, version: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    let conn = rusqlite::Connection::open(path).expect("open synthetic index db");
    conn.execute_batch(
        "CREATE TABLE meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        ) STRICT",
    )
    .expect("create meta table");
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
        rusqlite::params![version.to_string()],
    )
    .expect("stamp schema_version");
}

/// Mirror of [`Paths::resolve`] that derives the layout from a [`ToolEnv`]'s
/// isolated `$HOME` instead of touching real env vars. Lets the lifecycle
/// library API and the spawned CLI binary share an on-disk layout without
/// `Command::env` mutating process state.
pub fn paths_for(env: &ToolEnv) -> Paths {
    Paths::from_root(env.tome_root())
}

/// Construct a `Scope(global)` value for tests that need to thread a
/// scope through the lifecycle / query APIs without caring which
/// workspace they're under. Centralises the (now-tuple-struct)
/// constructor expression so future Scope reshapes touch one place.
pub fn global_scope() -> tome::workspace::Scope {
    tome::workspace::Scope(tome::workspace::WorkspaceName::global())
}
