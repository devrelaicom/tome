//! Shared test harness for the catalog command integration suites. Each test
//! builds a fresh fixture catalog inside a `tempfile::TempDir`, runs
//! `git init && git add -A && git commit -m init` against it (so it has a
//! HEAD), and constructs `Command` invocations of the `tome` binary with
//! isolated `HOME`/`XDG_*` so the host's real config is never touched.
//!
//! All paths are absolute. No mocking of git or the filesystem.

#![allow(dead_code)] // each test file uses a subset of these helpers

/// In-process MCP test harness (Phase 7 / FR-012). Constructs + drives a
/// real `mcp::server::Server` over a staged workspace using the
/// StubEmbedder — see [`mcp_harness::McpHarness`].
pub mod mcp_harness;

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

/// The deterministic `file://` enrolment URL a test uses for `catalog_name`.
/// Mirrors what `tome catalog add` would record; `cache_dir_for(url)` is the
/// on-disk clone root.
pub fn test_catalog_url(catalog_name: &str) -> String {
    format!("file://example.invalid/{catalog_name}")
}

/// Enrol `(workspace_name, catalog_name)` in the central DB and symlink the
/// URL-hashed clone dir `cache_dir_for(url)` onto an EXISTING on-disk catalog
/// tree at `catalog_root` (the production layout is a real clone; tests
/// symlink so in-place mutations — `rewrite_skill`, `remove_skill`, refresh —
/// are seen by `resolve_plugin_dir`, which reads `cache_dir_for(url)`).
///
/// The enrolment URL is `file://<catalog_root>`, so it is unique per test
/// tree and independent of any in-memory `Config` the test still keeps for a
/// not-yet-migrated command. Opens the DB with the **registry** seeds (via
/// [`enrol_catalog_row`]) — a test that drives the stub-seed query path
/// (`run_with_deps`, whose drift check would reject a registry-stamped `meta`)
/// must bootstrap `meta` with the stub seeds *before* calling this, so this
/// helper's open is a no-op reopen. Idempotent on the symlink. Unix-only
/// symlink (the suite runs on macOS / Linux).
pub fn enrol_catalog_symlinked(
    paths: &Paths,
    workspace_name: &str,
    catalog_name: &str,
    catalog_root: &Path,
) -> String {
    let url = format!("file://{}", catalog_root.display());
    enrol_catalog_row(paths, workspace_name, catalog_name, &url);
    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        std::fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        #[cfg(not(unix))]
        copy_dir(catalog_root, &cache_dir).expect("copy catalog cache (non-unix)");
    }
    url
}

/// Insert one `(workspace_name, catalog_name) -> url` enrolment row, opening
/// (and bootstrapping) the DB with the **registry** seeds.
///
/// Registry seeds (not stub) are stamped on the first open so this helper is
/// safe to call *before* a test's own `lifecycle::enable`: `index::open`
/// ignores `OpenOptions` on a reopen, so a stub-embedder enable still works
/// against a registry-stamped `meta`, while drift-checking tests (`status`,
/// `doctor`) — which expect the registry identity — see a consistent baseline.
/// Mirrors the seed discipline already used by [`write_config_for_cli`].
fn enrol_catalog_row(paths: &Paths, workspace_name: &str, catalog_name: &str, url: &str) {
    let (e, r, s) = registry_seeds_for_test();
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .expect("open index for catalog enrolment");
    match tome::index::workspace_catalogs::insert(&conn, workspace_name, catalog_name, url, "main")
    {
        Ok(()) => {}
        Err(tome::error::TomeError::CatalogAlreadyExists(_)) => {}
        Err(e) => panic!("enrol catalog in workspace_catalogs failed: {e}"),
    }
}

/// Stage the `sample-plugin-catalog` fixture the way production does (FF1):
/// copy it into the content-addressed clone dir `paths.cache_dir_for(url)`
/// and enrol `(workspace_name, catalog_name) -> url` into the
/// `workspace_catalogs` table — never `config.toml`. This is the real
/// `tome catalog add` shape, so `lifecycle::resolve_plugin_dir` (which reads
/// the enrolment URL → cache dir) finds the plugin tree.
///
/// Returns the staged clone root (`cache_dir_for(url)`) so callers can do
/// their on-disk work — `write_plugin`, `rewrite_skill`, `remove_skill` —
/// against the directory resolution will actually read. The fixture copy is
/// skipped if the clone already exists, so a second workspace enrolling the
/// same catalog reuses it. Stamps `meta` with the **registry** seeds (see
/// [`enrol_catalog_row`]).
pub fn stage_sample_catalog_in_db(
    paths: &Paths,
    workspace_name: &str,
    catalog_name: &str,
) -> PathBuf {
    stage_catalog_dir_in_db(
        paths,
        workspace_name,
        catalog_name,
        &sample_plugin_catalog_fixture(),
    )
}

/// Like [`stage_sample_catalog_in_db`] but stages an arbitrary `source`
/// directory (e.g. a per-test fixture skeleton) into the clone dir. When
/// `source` is empty/absent the clone dir is created bare — callers that lay
/// the tree out themselves via `write_plugin` pass a non-existent path.
pub fn stage_catalog_dir_in_db(
    paths: &Paths,
    workspace_name: &str,
    catalog_name: &str,
    source: &Path,
) -> PathBuf {
    let url = test_catalog_url(catalog_name);
    let cache_root = paths.cache_dir_for(&url);
    if !cache_root.exists() {
        if source.is_dir() {
            copy_dir(source, &cache_root).expect("stage catalog clone");
        } else {
            std::fs::create_dir_all(&cache_root).expect("create bare catalog clone");
        }
    }
    enrol_catalog_row(paths, workspace_name, catalog_name, &url);
    cache_root
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

/// Write the supplied [`Config`] to `paths.global_config_file` as TOML
/// (legacy, F11b-deprecated) AND seed each enrolment into the central
/// DB's `workspace_catalogs` table for the privileged `global` workspace.
///
/// **Seed discipline**: this helper opens the DB if it does not yet
/// exist, stamping `meta` with the **registry** seeds (BGE) so the
/// CLI binary's later opens see matching identities. Test files that
/// then call `lifecycle::enable` via the StubEmbedder must use
/// `stub_*_seed()` consistently from the test side; the central DB
/// becomes the source of truth for "what identities were stamped".
///
/// If the DB already exists at call time, the seed step is a no-op
/// (subsequent `index::open` calls ignore `opts`).
pub fn write_config_for_cli(paths: &Paths, config: &Config) {
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    // Legacy: still written so the (PR-2/PR-3) commands that continue to read
    // `config.toml [catalogs]` keep finding the catalog while their migration
    // is pending. `resolve_plugin_dir` no longer consults it (FF1).
    #[allow(deprecated)]
    let body = toml::to_string_pretty(config).expect("serialise config");
    std::fs::write(&paths.global_config_file, body).expect("write config.toml");

    // F11b: seed enrolments in the central DB so CLI catalog commands
    // see them. Use registry seeds so the CLI binary (which opens with
    // registry seeds) doesn't drift on subsequent opens.
    let (e, r, s) = registry_seeds_for_test();
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .expect("open index db for catalog seed");
    #[allow(deprecated)]
    for entry in config.catalogs.values() {
        // FF1: `resolve_plugin_dir` now resolves the catalog root from the
        // enrolment URL via `cache_dir_for(url)`. The fixture tree lives at
        // `entry.path`; SYMLINK (not copy) the content-addressed clone dir
        // onto it so the DB-resolved path and the fixture tree are the SAME
        // inode tree. A copy snapshots at stage time and goes stale: a test
        // that corrupts a plugin.json AFTER setup (exit_codes_e2e) or compares
        // on-disk mtimes against `indexed_at` (doctor_p5) would otherwise see
        // the pristine copy, not its own mutation. `remove_dir_all` does not
        // follow symlinks, so each TempDir's cleanup unlinks the symlink
        // without touching the shared fixture. Unix-only — matches the
        // macOS + Linux test matrix.
        let cache_root = paths.cache_dir_for(&entry.url);
        if entry.path.is_dir() && !cache_root.exists() {
            if let Some(parent) = cache_root.parent() {
                std::fs::create_dir_all(parent).expect("create catalogs cache parent");
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&entry.path, &cache_root)
                .expect("symlink catalog clone for cli tests");
            #[cfg(not(unix))]
            copy_dir(&entry.path, &cache_root).expect("stage catalog clone for cli tests");
        }
        match tome::index::workspace_catalogs::insert(
            &conn,
            "global",
            &entry.name,
            &entry.url,
            &entry.ref_,
        ) {
            Ok(()) => {}
            Err(tome::error::TomeError::CatalogAlreadyExists(_)) => {}
            Err(e) => panic!("seed workspace_catalogs failed: {e}"),
        }
    }
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

/// Build a `MetaSeed` triple from the registry — matches whatever the
/// CLI binary opens with. Used by test helpers that open the central
/// DB AFTER the CLI has stamped meta (re-opens ignore `opts`).
fn registry_seeds_for_test() -> (
    tome::index::MetaSeed,
    tome::index::MetaSeed,
    tome::index::MetaSeed,
) {
    let pick = |kind| {
        let entry = tome::embedding::registry::MODEL_REGISTRY
            .iter()
            .find(|m| std::mem::discriminant(&m.kind) == std::mem::discriminant(&kind))
            .unwrap();
        tome::index::MetaSeed {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
        }
    };
    (
        pick(tome::embedding::registry::ModelKind::Embedder),
        pick(tome::embedding::registry::ModelKind::Reranker),
        pick(tome::embedding::registry::ModelKind::Summariser),
    )
}

/// Read every catalog enrolment for the privileged `global` workspace
/// from the central DB. Used by tests that assert on F11b enrolment
/// state without parsing config.toml.
pub fn read_global_enrolments(paths: &Paths) -> Vec<tome::index::CatalogEnrolment> {
    if !paths.index_db.is_file() {
        return Vec::new();
    }
    let (e, r, s) = registry_seeds_for_test();
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .expect("open index db for enrolment read");
    tome::index::workspace_catalogs::list_for_workspace(&conn, "global").unwrap_or_default()
}

/// Convenience: true iff a `(global, name)` enrolment exists in the
/// central DB.
pub fn has_global_enrolment(paths: &Paths, catalog_name: &str) -> bool {
    read_global_enrolments(paths)
        .iter()
        .any(|e| e.catalog_name == catalog_name)
}

/// Look up the URL of one `(global, name)` enrolment.
pub fn global_enrolment_url(paths: &Paths, catalog_name: &str) -> Option<String> {
    read_global_enrolments(paths)
        .into_iter()
        .find(|e| e.catalog_name == catalog_name)
        .map(|e| e.url)
}

/// Seed a named workspace row directly into the central DB. Mirrors the
/// shape of `schema::bootstrap`'s seed of the privileged `global`
/// workspace; this is the seam US2 (`tome workspace add`) will own when
/// it ships. Until then, tests that need a non-`global` workspace
/// present in the `workspaces` table call this helper.
///
/// The seed step opens (and creates) the DB if absent, stamping `meta`
/// with stub seeds. Tests that subsequently invoke the CLI binary
/// against the same `Paths` must use [`write_config_for_cli`] BEFORE
/// this call so the CLI's registry-seed open isn't shadowed by a
/// stub-seeded one (`open` is "first writer wins" on `meta`).
pub fn seed_workspace(paths: &Paths, name: &str) {
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index for seeding workspace");
    let now_unix = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspaces (name, created_at, last_used_at) VALUES (?1, ?2, ?2)",
        rusqlite::params![name, now_unix],
    )
    .expect("seed workspace row");
}

/// Compute the on-disk cache directory for a given catalog URL using the
/// same content-addressing as the `tome` binary (sha256 hex of the URL).
pub fn cache_dir_for(env: &ToolEnv, url: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    env.catalogs_dir().join(hex::encode(h.finalize()))
}

/// RAII guard installing a synthetic harness module set in
/// [`tome::harness::HARNESS_MODULES_OVERRIDE`]. The slot is restored
/// to `None` on drop, surviving panics so a failing assertion can't
/// leak the override into the next test binary entry.
///
/// The override slot is a process-global `RwLock`; integration tests
/// that install one must serialise via the `OVERRIDE_MUTEX` pattern
/// documented in `tests/harness_sync_stub.rs` to avoid clobbering one
/// another when cargo runs them in parallel.
pub struct HarnessModulesGuard;

impl HarnessModulesGuard {
    pub fn install(modules: Vec<Box<dyn tome::harness::HarnessModule>>) -> Self {
        *tome::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("HARNESS_MODULES_OVERRIDE poisoned") = Some(modules);
        Self
    }
}

impl Drop for HarnessModulesGuard {
    fn drop(&mut self) {
        *tome::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("HARNESS_MODULES_OVERRIDE poisoned") = None;
    }
}

/// `HarnessModule` whose `name()` is determined at construction. The
/// name string is leaked via [`Box::leak`] so we can satisfy the
/// `&'static str` return shape the trait requires. Used by tests that
/// drive composition resolution against synthetic harness names not in
/// the production registry (e.g. `"a"`, `"x"`, `"alpha"`).
///
/// Cost: each `new()` call leaks one `String`. Tests that build a
/// fixed-size set up front (the common case) leak O(n) once, which is
/// trivial against cargo's per-test-binary memory budget.
pub struct NamedStubHarness {
    name: &'static str,
}

impl NamedStubHarness {
    pub fn new(name: &str) -> Self {
        let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
        Self { name: leaked }
    }

    /// Construct a `Vec<Box<dyn HarnessModule>>` from any iterable of
    /// names. Helper for the common `HarnessModulesGuard::install(...)`
    /// call site in composition-resolver tests.
    pub fn boxed_set<I, S>(names: I) -> Vec<Box<dyn tome::harness::HarnessModule>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        names
            .into_iter()
            .map(|s| {
                let owned: Box<dyn tome::harness::HarnessModule> =
                    Box::new(NamedStubHarness::new(s.as_ref()));
                owned
            })
            .collect()
    }
}

impl tome::harness::HarnessModule for NamedStubHarness {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        "test-only named stub"
    }
    fn detect(&self, _home: &Path) -> bool {
        true
    }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("STUB_RULES.md")
    }
    fn rules_file_strategy(&self) -> tome::harness::RulesFileStrategy {
        tome::harness::RulesFileStrategy::BlockInExistingFile
    }
    fn block_body_style(&self) -> tome::harness::BlockBodyStyle {
        tome::harness::BlockBodyStyle::Inline
    }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("stub.mcp.json")
    }
    fn mcp_config_format(&self) -> tome::harness::McpConfigFormat {
        tome::harness::McpConfigFormat::Json
    }
    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

/// Test helper: overwrite the `pinned_ref` for one `(global, name)`
/// enrolment. Used by SHA-pinning tests that previously hand-edited
/// `config.toml`.
pub fn set_global_enrolment_ref(paths: &Paths, catalog_name: &str, new_ref: &str) {
    let (e, r, s) = registry_seeds_for_test();
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .unwrap();
    let affected = conn
        .execute(
            "UPDATE workspace_catalogs SET pinned_ref = ?1
             WHERE workspace_id = (SELECT id FROM workspaces WHERE name = 'global')
               AND catalog_name = ?2",
            rusqlite::params![new_ref, catalog_name],
        )
        .unwrap();
    assert!(affected > 0, "no enrolment matched for `{catalog_name}`");
}

// ---------------------------------------------------------------------------
// HOME env serialisation (T-B1 from US3 review)
//
// Process-global `$HOME` mutations across parallel integration tests race
// otherwise. `HomeGuard::install(new)` snapshots the current value, sets
// the new one, and restores on Drop — while holding `HOME_MUTEX` so no
// other test mutates the env concurrently.
// ---------------------------------------------------------------------------

/// Process-global serialisation lock for tests that mutate `$HOME`.
///
/// Exposed `pub` so test files can lock it directly if they need to read
/// `$HOME` without mutating it (rare); the typical entry point is
/// [`HomeGuard::install`].
pub static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that installs a new value for `$HOME` and restores the
/// previous value on drop. Holds `HOME_MUTEX` for its lifetime so
/// parallel tests are serialised.
///
/// **Field-drop order discipline**: `_previous` is declared before
/// `_lock`. Rust drops struct fields in declaration order, so
/// `_previous` (whose Drop restores `$HOME`) runs BEFORE `_lock`'s Drop
/// (which releases `HOME_MUTEX`). That ordering matters — the restore
/// must complete before another test acquires the mutex and reads the
/// env. Do not reorder these fields.
pub struct HomeGuard {
    _previous: PrevHome,
    _lock: std::sync::MutexGuard<'static, ()>,
}

struct PrevHome(Option<std::ffi::OsString>);

impl Drop for PrevHome {
    fn drop(&mut self) {
        // SAFETY: the surrounding `_lock` field is dropped AFTER us
        // (declared-order drop), so we still hold the mutex while
        // restoring. No other thread is racing the env.
        match &self.0 {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }
}

impl HomeGuard {
    /// Acquire `HOME_MUTEX`, snapshot the current `$HOME`, and set it
    /// to `new_home`. The previous value is restored when the returned
    /// guard is dropped.
    ///
    /// On poisoned mutex we recover the inner guard and proceed —
    /// poisoning from another test's panic shouldn't cascade into this
    /// one's setup failure.
    pub fn install(new_home: &Path) -> Self {
        let lock = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("HOME");
        // SAFETY: we hold `HOME_MUTEX` for the lifetime of `Self`.
        unsafe {
            std::env::set_var("HOME", new_home);
        }
        Self {
            _previous: PrevHome(previous),
            _lock: lock,
        }
    }
}

// ---------------------------------------------------------------------------
// Substitution layer override guards (Phase 5 / F3)
//
// Each guard installs a value into the matching
// `tome::substitution::*_OVERRIDE` slot on construction and clears the
// slot's value on Drop. Mirrors the `HarnessModulesGuard` pattern above.
//
// Poisoned-mutex recovery via `PoisonError::into_inner` per the Phase 4
// P5 retro lesson — a panic in one test must not cascade into setup
// failures for the next.
//
// Unlike `HomeGuard`, these guards do not serialise against a process-
// global lock: the override slots are per-substitution-layer and the
// production code path consults them as read-only inputs once per
// `render()` call. Tests that drive `render()` concurrently against
// different override values would need to introduce their own
// synchronisation; F3 ships no consumer that does this.
// ---------------------------------------------------------------------------

/// Install a fixed clock value into
/// [`tome::substitution::SUBSTITUTION_CLOCK_OVERRIDE`]. The slot is
/// restored to `None` on drop.
pub struct ClockOverrideGuard;

impl ClockOverrideGuard {
    pub fn install(when: time::OffsetDateTime) -> Self {
        *tome::substitution::SUBSTITUTION_CLOCK_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(when);
        Self
    }
}

impl Drop for ClockOverrideGuard {
    fn drop(&mut self) {
        *tome::substitution::SUBSTITUTION_CLOCK_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Install a path into
/// [`tome::substitution::PLUGIN_DATA_DIR_OVERRIDE`]. The slot is
/// restored to `None` on drop.
pub struct PluginDataDirGuard;

impl PluginDataDirGuard {
    pub fn install(path: PathBuf) -> Self {
        *tome::substitution::PLUGIN_DATA_DIR_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(path);
        Self
    }
}

impl Drop for PluginDataDirGuard {
    fn drop(&mut self) {
        *tome::substitution::PLUGIN_DATA_DIR_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Install a path into
/// [`tome::substitution::WORKSPACE_DATA_DIR_OVERRIDE`]. The slot is
/// restored to `None` on drop.
pub struct WorkspaceDataDirGuard;

impl WorkspaceDataDirGuard {
    pub fn install(path: PathBuf) -> Self {
        *tome::substitution::WORKSPACE_DATA_DIR_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(path);
        Self
    }
}

impl Drop for WorkspaceDataDirGuard {
    fn drop(&mut self) {
        *tome::substitution::WORKSPACE_DATA_DIR_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
}
