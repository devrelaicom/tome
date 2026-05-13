//! Enable / disable orchestrator for a single plugin.
//!
//! This module composes the plugin-metadata parsers (`plugin::manifest`,
//! `plugin::frontmatter`), the index layer (`index::open`, `index::acquire_lock`,
//! `index::enable_plugin_atomic`, `index::mark_all_disabled_for_plugin`), and
//! the embedding model presence check (`embedding::registry`,
//! `embedding::download`) into the contract described in
//! `specs/002-phase-2-plugins-index/contracts/plugin-commands.md` (lines 9–97).
//!
//! No CLI / IO / prompt code lives here — slice 1b wires `tome plugin
//! {enable,disable}` on top of this surface. The TTY-versus-non-TTY decision
//! for "is it OK to download a missing model" is reduced to the
//! [`LifecycleDeps::allow_model_download`] boolean so this module remains
//! testable without a terminal.
//!
//! Spec: FR-004, FR-005, FR-006, FR-013a/b/c, FR-024, FR-025, FR-053;
//! contracts/plugin-commands.md §1–§2.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use crate::catalog::git::was_cancelled;
use crate::config::Config;
use crate::embedding::download::download_model;
use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelManifest};
use crate::embedding::{Embedder, ModelKind};
use crate::error::{PluginState, TomeError};
use crate::index::skills::{EnableSummary, PendingSkill};
use crate::index::{
    self, MetaSeed, OpenOptions, acquire_lock, enable_plugin_atomic, mark_all_disabled_for_plugin,
};
use crate::paths::Paths;
use crate::plugin::frontmatter::{FrontmatterError, parse_skill_frontmatter};
use crate::plugin::identity::PluginId;
use crate::plugin::manifest::{manifest_path_for, parse_plugin_manifest};

/// Result of a successful enable.
#[derive(Debug, Clone)]
pub struct EnableOutcome {
    pub plugin: PluginId,
    pub summary: EnableSummary,
    pub duration: Duration,
    /// Human-readable warnings collected during the walk. Each entry is a
    /// stable diagnostic the CLI layer surfaces on stderr — FR-011 / FR-012
    /// fallback notices and FR-013c skipped-skill notices.
    pub warnings: Vec<String>,
}

/// Result of a successful disable.
#[derive(Debug, Clone)]
pub struct DisableOutcome {
    pub plugin: PluginId,
    pub skills_retained: u32,
    pub duration: Duration,
}

/// Inputs to [`enable`]. Kept as a single struct so the CLI wrapper that
/// constructs `embedder`, `embedder_seed`, `reranker_seed`, and the TTY
/// decision can pass them through unchanged.
pub struct LifecycleDeps<'a> {
    pub paths: &'a Paths,
    pub config: &'a Config,
    pub embedder: &'a dyn Embedder,
    pub embedder_seed: MetaSeed,
    pub reranker_seed: MetaSeed,
    /// `true` when the CLI has confirmed (via TTY prompt) that Tome may
    /// download missing models. `false` is the non-TTY refusal contract —
    /// the function returns `ModelMissing` (exit 30) per plugin-commands.md
    /// step 4.
    pub allow_model_download: bool,
}

// -------------------------------------------------------------------------
// Public API
// -------------------------------------------------------------------------

/// Enable a plugin: walk its skills, embed-and-insert under one SQLite
/// transaction, and surface fallback / skipped-skill warnings.
///
/// The full contract is captured by `plugin-commands.md` §1. Atomic guarantee
/// (FR-004): on any failure after the lock is acquired, the on-disk index is
/// indistinguishable from its pre-call state.
pub fn enable(id: &PluginId, deps: &LifecycleDeps<'_>) -> Result<EnableOutcome, TomeError> {
    let started = Instant::now();
    let plugin_dir = resolve_plugin_dir(id, deps.config)?;

    // Step 2 — manifest parse. We don't *use* the parsed fields below (the
    // `plugin_version` we record per-skill is sourced from this manifest's
    // `version` field), but reading it early gives us the right exit code
    // (22) before we touch the index.
    let manifest_path = manifest_path_for(&plugin_dir);
    let manifest = parse_plugin_manifest(&manifest_path)?;
    let plugin_version = manifest
        .version
        .clone()
        .unwrap_or_else(|| "0.0.0".to_string());

    // Step 3 — already-enabled check. We open the DB read-only-ish (the
    // bootstrap is idempotent on re-open) and look for any enabled row.
    // Doing this before the lock is acquired keeps the contention surface
    // small: a quick check and bail.
    if any_skill_enabled(deps, id)? {
        return Err(TomeError::PluginAlreadyInState {
            plugin: id.to_string(),
            state: PluginState::Enabled,
        });
    }

    // Step 4 — model presence (T074).
    ensure_models_present(deps)?;

    // Step 5 — advisory lock. Held until step 10.
    let lock = acquire_lock(&deps.paths.index_lock)?;

    // Run the rest under the lock; release explicitly on success, drop on
    // failure (Drop releases best-effort, matching the lock module's docs).
    let result = enable_locked(id, &plugin_dir, &plugin_version, deps);

    match result {
        Ok((summary, warnings)) => {
            lock.release()?;
            Ok(EnableOutcome {
                plugin: id.clone(),
                summary,
                duration: started.elapsed(),
                warnings,
            })
        }
        Err(e) => {
            drop(lock);
            Err(e)
        }
    }
}

/// Disable a plugin: flip every `(catalog, plugin)` row's `enabled` column
/// to 0. Embeddings are retained for cheap re-enable (FR-005, FR-006).
///
/// The CLI layer is responsible for the confirmation prompt (and `--force`).
/// This function performs no prompting; it only mutates state.
pub fn disable(
    id: &PluginId,
    paths: &Paths,
    config: &Config,
    embedder_seed: MetaSeed,
    reranker_seed: MetaSeed,
) -> Result<DisableOutcome, TomeError> {
    let started = Instant::now();
    // We still resolve the plugin directory to reject typos before touching
    // the index — same exit-code surface as enable.
    let _plugin_dir = resolve_plugin_dir(id, config)?;

    let lock = acquire_lock(&paths.index_lock)?;
    let outcome = disable_locked(id, paths, embedder_seed, reranker_seed);

    match outcome {
        Ok(skills_retained) => {
            lock.release()?;
            Ok(DisableOutcome {
                plugin: id.clone(),
                skills_retained,
                duration: started.elapsed(),
            })
        }
        Err(e) => {
            drop(lock);
            Err(e)
        }
    }
}

// -------------------------------------------------------------------------
// Private helpers
// -------------------------------------------------------------------------

/// Resolve `<catalog>/<plugin>` against the registry and on-disk cache.
///
/// Authoritative source is the catalog's `tome-catalog.toml`:
/// `entry.path.join(&plugins[].source)` for the entry whose `name` matches
/// `id.plugin`. The lookup is intentionally manifest-first so that catalogs
/// declaring nested layouts (e.g. `source = "./plugins/foo"`) work uniformly
/// across `enable`, `show`, and `list` (see also FR-008 and `query.md`).
///
/// When `tome-catalog.toml` is absent or unparsable the resolver falls back
/// to the flat layout `entry.path.join(&id.plugin)` — this preserves
/// back-compat for library callers that construct catalog roots without a
/// manifest (the `lifecycle.rs` in-module tests, hand-rolled fixtures, and
/// the "I cloned a plugin into a bare directory" recovery path).
pub fn resolve_plugin_dir(id: &PluginId, config: &Config) -> Result<PathBuf, TomeError> {
    let entry = config
        .catalogs
        .get(&id.catalog)
        .ok_or_else(|| TomeError::CatalogNotFound(id.catalog.clone()))?;

    let plugin_dir = match crate::catalog::manifest::read_catalog_manifest(&entry.path) {
        Some(manifest) => {
            let decl = manifest
                .plugins
                .iter()
                .find(|p| p.name == id.plugin)
                .ok_or_else(|| TomeError::PluginNotFound(id.to_string()))?;
            entry.path.join(&decl.source)
        }
        None => entry.path.join(&id.plugin),
    };

    if !plugin_dir.is_dir() {
        return Err(TomeError::PluginNotFound(id.to_string()));
    }
    Ok(plugin_dir)
}

/// Returns `true` when the index already contains at least one row for
/// `(catalog, plugin)` with `enabled = 1`.
fn any_skill_enabled(deps: &LifecycleDeps<'_>, id: &PluginId) -> Result<bool, TomeError> {
    let conn = index::open(
        &deps.paths.index_db,
        &OpenOptions {
            embedder: deps.embedder_seed.clone(),
            reranker: deps.reranker_seed.clone(),
        },
    )?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skills
             WHERE catalog = ?1 AND plugin = ?2 AND enabled = 1",
            rusqlite::params![id.catalog, id.plugin],
            |row| row.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("check enabled skills: {e}")))?;
    Ok(count > 0)
}

/// Steps 6–9 of the enable contract — held under the advisory lock.
fn enable_locked(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    deps: &LifecycleDeps<'_>,
) -> Result<(EnableSummary, Vec<String>), TomeError> {
    if was_cancelled() {
        return Err(TomeError::Interrupted);
    }

    let mut conn = index::open(
        &deps.paths.index_db,
        &OpenOptions {
            embedder: deps.embedder_seed.clone(),
            reranker: deps.reranker_seed.clone(),
        },
    )?;

    let mut warnings: Vec<String> = Vec::new();
    let pending = collect_pending_skills(id, plugin_dir, plugin_version, &mut warnings)?;

    let embedder = deps.embedder;
    let summary = enable_plugin_atomic(&mut conn, &pending, |text| {
        // Cancellation is observed inside the embed loop too (handover
        // gotcha #3): each embed call peeks the SIGINT flag. The closure
        // returns `Err(TomeError::Interrupted)` which `enable_plugin_atomic`
        // propagates and the surrounding transaction rolls back.
        if was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        embedder.embed(text)
    })?;

    info!(
        plugin = %id,
        total = summary.total_skills,
        newly = summary.newly_embedded,
        skipped = warnings.len(),
        "plugin enable completed",
    );

    Ok((summary, warnings))
}

/// Disable-locked branch — runs under the advisory lock.
fn disable_locked(
    id: &PluginId,
    paths: &Paths,
    embedder_seed: MetaSeed,
    reranker_seed: MetaSeed,
) -> Result<u32, TomeError> {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
        },
    )?;

    // The contract requires "already-disabled" detection. Two cases collapse
    // into one PluginAlreadyInState: zero rows for the plugin OR every row
    // already has `enabled = 0`.
    let (total, enabled_count): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(enabled), 0)
             FROM skills
             WHERE catalog = ?1 AND plugin = ?2",
            rusqlite::params![id.catalog, id.plugin],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("count skills: {e}")))?;
    if total == 0 || enabled_count == 0 {
        return Err(TomeError::PluginAlreadyInState {
            plugin: id.to_string(),
            state: PluginState::Disabled,
        });
    }

    let affected = mark_all_disabled_for_plugin(&conn, &id.catalog, &id.plugin)?;
    Ok(affected)
}

/// Walk `<plugin_dir>/skills/*/SKILL.md`. Errors per-file are funnelled:
///
/// * Delimiter failure → `SkillFrontmatterParseError` (whole-plugin abort).
/// * YAML body failure → warning + skip (FR-013c).
/// * IO failure → bubble as `TomeError::Io`.
fn collect_pending_skills(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<PendingSkill>, TomeError> {
    let skills_root = plugin_dir.join("skills");
    if !skills_root.is_dir() {
        debug!(
            plugin = %id,
            skills_dir = %skills_root.display(),
            "no skills directory; enabling zero rows",
        );
        return Ok(Vec::new());
    }

    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&skills_root) {
        Ok(it) => it
            .filter_map(|res| res.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) => return Err(TomeError::Io(e)),
    };
    // Deterministic ordering across platforms / filesystems.
    entries.sort();

    let mut pending: Vec<PendingSkill> = Vec::with_capacity(entries.len());

    for skill_dir in entries {
        if was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        let skill_file = skill_dir.join("SKILL.md");
        if !skill_file.is_file() {
            continue;
        }

        let parsed = match parse_skill_frontmatter(&skill_file) {
            Ok(p) => p,
            Err(FrontmatterError::MissingDelimiters { file, message }) => {
                return Err(TomeError::SkillFrontmatterParseError { file, message });
            }
            Err(FrontmatterError::InvalidYaml { file, message }) => {
                let warning = format!(
                    "skipped {}: frontmatter YAML invalid: {}",
                    file.display(),
                    message
                );
                warn!(file = %file.display(), reason = %message, "skipping skill: invalid YAML body");
                warnings.push(warning);
                continue;
            }
            Err(FrontmatterError::Io { file: _, source }) => return Err(TomeError::Io(source)),
        };

        let dir_name = skill_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let (name, name_fallback) = parsed.resolved_name(&dir_name);
        let (description, desc_fallback) = parsed.resolved_description();
        if name_fallback {
            warnings.push(format!(
                "name fallback applied for {}: using directory name `{}`",
                skill_file.display(),
                name
            ));
        }
        if desc_fallback {
            warnings.push(format!(
                "description fallback applied for {}: using leading body text",
                skill_file.display()
            ));
        }

        let rel_path = skill_file
            .strip_prefix(plugin_dir)
            .unwrap_or(&skill_file)
            .to_string_lossy()
            .into_owned();

        pending.push(PendingSkill {
            catalog: id.catalog.clone(),
            plugin: id.plugin.clone(),
            name,
            description,
            plugin_version: plugin_version.to_owned(),
            path: rel_path,
        });
    }

    Ok(pending)
}

/// Step 4 — confirm the embedder and reranker entries in `MODEL_REGISTRY`
/// each have a readable `manifest.json` on disk. Missing models prompt a
/// download iff `allow_model_download` is set; otherwise we error with
/// `ModelMissing` (exit 30).
fn ensure_models_present(deps: &LifecycleDeps<'_>) -> Result<(), TomeError> {
    for entry in MODEL_REGISTRY {
        // Only enforce embedder and reranker — other kinds, if added later,
        // are not strict requirements of the enable path.
        match entry.kind {
            ModelKind::Embedder | ModelKind::Reranker => {}
        }
        if model_manifest_ok(deps.paths, entry)? {
            continue;
        }
        if !deps.allow_model_download {
            return Err(TomeError::ModelMissing {
                model: entry.name.to_owned(),
            });
        }
        info!(model = entry.name, "downloading model artefact");
        download_model(entry, &deps.paths.models_dir)?;
    }
    Ok(())
}

/// Returns `Ok(true)` iff a parseable `manifest.json` for `entry` exists
/// under `paths.models_dir`. A read or parse failure is treated as "model
/// not installed" — the contract redirects the user to download.
fn model_manifest_ok(paths: &Paths, entry: &ModelEntry) -> Result<bool, TomeError> {
    let manifest_path = paths.model_manifest(entry.name)?;
    if !manifest_path.is_file() {
        return Ok(false);
    }
    let bytes = match std::fs::read(&manifest_path) {
        Ok(b) => b,
        Err(_) => return Ok(false),
    };
    match serde_json::from_slice::<ModelManifest>(&bytes) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

// -------------------------------------------------------------------------
// Unit tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CatalogEntry;
    use crate::embedding::stub::StubEmbedder;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    // ---- test scaffolding --------------------------------------------------

    /// Build a `Paths` rooted entirely under `root` so tests never touch
    /// `$HOME` or env vars (foundational retro: gotcha #5).
    fn test_paths(root: &Path) -> Paths {
        Paths {
            config_dir: root.join("config"),
            config_file: root.join("config/config.toml"),
            data_dir: root.join("data"),
            catalogs_dir: root.join("data/catalogs"),
            index_db: root.join("data/index.db"),
            index_lock: root.join("data/index.lock"),
            models_dir: root.join("data/models"),
        }
    }

    fn stub_seed() -> MetaSeed {
        MetaSeed {
            name: "stub-embedder".into(),
            version: "0".into(),
        }
    }

    fn stub_reranker_seed() -> MetaSeed {
        MetaSeed {
            name: "stub-reranker".into(),
            version: "0".into(),
        }
    }

    /// Fabricate model dirs + manifest.json for every entry in
    /// `MODEL_REGISTRY`. We do NOT touch the network.
    fn fabricate_models(paths: &Paths) {
        for entry in MODEL_REGISTRY {
            let dir = paths.models_dir.join(entry.name);
            fs::create_dir_all(&dir).expect("create model dir");
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
            fs::write(dir.join("manifest.json"), body).expect("write manifest");
        }
    }

    /// Build a minimal `Config` with one catalog whose cache lives at
    /// `catalog_root`.
    fn config_with_catalog(catalog_name: &str, catalog_root: &Path) -> Config {
        let mut catalogs = BTreeMap::new();
        catalogs.insert(
            catalog_name.to_owned(),
            CatalogEntry {
                name: catalog_name.to_owned(),
                url: "https://example.invalid/repo".into(),
                ref_: "main".into(),
                path: catalog_root.to_path_buf(),
                last_synced: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            },
        );
        Config { catalogs }
    }

    /// Lay out a plugin on disk: `<catalog>/<plugin>/.claude-plugin/plugin.json`
    /// + zero or more skills (each `(dir_name, skill_md_contents)`).
    fn write_plugin(
        catalog_root: &Path,
        plugin_name: &str,
        plugin_version: Option<&str>,
        skills: &[(&str, &str)],
    ) -> PathBuf {
        let plugin_dir = catalog_root.join(plugin_name);
        fs::create_dir_all(plugin_dir.join(".claude-plugin")).expect("plugin dir");
        let version_line = plugin_version
            .map(|v| format!(", \"version\": \"{v}\""))
            .unwrap_or_default();
        let manifest = format!(r#"{{"name": "{plugin_name}"{version_line}}}"#);
        fs::write(
            plugin_dir.join(".claude-plugin").join("plugin.json"),
            manifest,
        )
        .expect("write manifest");

        for (dir_name, contents) in skills {
            let skill_dir = plugin_dir.join("skills").join(dir_name);
            fs::create_dir_all(&skill_dir).expect("skill dir");
            fs::write(skill_dir.join("SKILL.md"), contents).expect("write SKILL.md");
        }

        plugin_dir
    }

    /// Construct a `LifecycleDeps` against the supplied stub. We have to
    /// thread it through carefully because `LifecycleDeps` borrows.
    fn make_deps<'a>(
        paths: &'a Paths,
        config: &'a Config,
        embedder: &'a StubEmbedder,
        allow_model_download: bool,
    ) -> LifecycleDeps<'a> {
        LifecycleDeps {
            paths,
            config,
            embedder,
            embedder_seed: stub_seed(),
            reranker_seed: stub_reranker_seed(),
            allow_model_download,
        }
    }

    fn count_rows(paths: &Paths, catalog: &str, plugin: &str) -> (i64, i64) {
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: stub_seed(),
                reranker: stub_reranker_seed(),
            },
        )
        .expect("open index");
        conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(enabled), 0) FROM skills
             WHERE catalog = ?1 AND plugin = ?2",
            rusqlite::params![catalog, plugin],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("count")
    }

    fn good_skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n\nbody text\n")
    }

    // ---- enable: happy path ------------------------------------------------

    #[test]
    fn enable_happy_path_inserts_skills() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first skill")),
                ("beta", &good_skill_md("beta", "second skill")),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let outcome = enable(&id, &deps).expect("enable should succeed");
        assert_eq!(outcome.summary.total_skills, 2);
        assert_eq!(outcome.summary.newly_embedded, 2);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);

        let (total, enabled_sum) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 2);
        assert_eq!(enabled_sum, 2);
    }

    // ---- enable: idempotency rejected --------------------------------------

    #[test]
    fn enable_when_already_enabled_returns_error() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[("alpha", &good_skill_md("alpha", "first"))],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        enable(&id, &deps).expect("first enable");
        let err = enable(&id, &deps).expect_err("re-enable rejected");
        match err {
            TomeError::PluginAlreadyInState { state, .. } => {
                assert_eq!(state, PluginState::Enabled);
            }
            other => panic!("expected PluginAlreadyInState, got {other:?}"),
        }
    }

    // ---- enable: unknown catalog / plugin ----------------------------------

    #[test]
    fn enable_unknown_catalog() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fabricate_models(&paths);
        let config = Config::default();
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "ghost/plug".parse().unwrap();
        let err = enable(&id, &deps).expect_err("unknown catalog");
        assert!(matches!(err, TomeError::CatalogNotFound(c) if c == "ghost"));
    }

    #[test]
    fn enable_unknown_plugin_directory() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fabricate_models(&paths);
        let catalog_root = tmp.path().join("catalog");
        fs::create_dir_all(&catalog_root).unwrap();
        let config = config_with_catalog("acme", &catalog_root);
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/ghost".parse().unwrap();
        let err = enable(&id, &deps).expect_err("unknown plugin dir");
        assert!(matches!(err, TomeError::PluginNotFound(s) if s == "acme/ghost"));
    }

    // ---- enable: delimiter failure aborts the plugin -----------------------

    #[test]
    fn enable_aborts_on_missing_frontmatter_delimiters() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        // A SKILL.md with no `---` at all — delimiter failure, the whole
        // enable aborts and nothing is inserted.
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("broken", "no frontmatter here at all\n"),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let err = enable(&id, &deps).expect_err("delimiter failure aborts");
        assert!(matches!(err, TomeError::SkillFrontmatterParseError { .. }));

        // Transaction rolled back: zero rows for this plugin.
        let (total, _) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 0);
    }

    // ---- enable: YAML body failure skips one skill -------------------------

    #[test]
    fn enable_skips_skill_with_invalid_yaml_body_but_keeps_going() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        // Bad skill: delimiters present, but the YAML body is `:` which is
        // syntactically invalid YAML. Good skill: well-formed.
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("good", &good_skill_md("good", "a fine skill")),
                ("bad", "---\n: not valid yaml here\n---\nbody\n"),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let outcome = enable(&id, &deps).expect("enable continues past bad skill");
        assert_eq!(outcome.summary.total_skills, 1);
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.contains("frontmatter YAML invalid")),
            "expected skip warning, got {:?}",
            outcome.warnings,
        );

        // Only one row inserted; the bad skill's row is absent.
        let (total, _) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 1);
    }

    // ---- enable: fallback warnings -----------------------------------------

    #[test]
    fn enable_emits_fallback_warning_for_missing_name() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        // Empty name → directory-name fallback triggers a warning. The
        // description is present so only one fallback fires.
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[(
                "mydir",
                "---\nname: \"\"\ndescription: a description\n---\nbody\n",
            )],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let outcome = enable(&id, &deps).expect("enable");
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.contains("name fallback applied") && w.contains("mydir")),
            "expected fallback warning, got {:?}",
            outcome.warnings,
        );
    }

    // ---- disable -----------------------------------------------------------

    #[test]
    fn disable_flips_all_rows_to_disabled() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("beta", &good_skill_md("beta", "second")),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();
        enable(&id, &deps).expect("enable");

        let outcome =
            disable(&id, &paths, &config, stub_seed(), stub_reranker_seed()).expect("disable");
        assert_eq!(outcome.skills_retained, 2);

        let (total, enabled_sum) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 2);
        assert_eq!(enabled_sum, 0);
    }

    #[test]
    fn disable_when_already_disabled_returns_error() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[("alpha", &good_skill_md("alpha", "first"))],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();
        enable(&id, &deps).expect("enable");
        disable(&id, &paths, &config, stub_seed(), stub_reranker_seed()).expect("disable");

        let err = disable(&id, &paths, &config, stub_seed(), stub_reranker_seed())
            .expect_err("second disable rejected");
        match err {
            TomeError::PluginAlreadyInState { state, .. } => {
                assert_eq!(state, PluginState::Disabled);
            }
            other => panic!("expected PluginAlreadyInState, got {other:?}"),
        }
    }

    // ---- model presence ----------------------------------------------------

    #[test]
    fn enable_returns_model_missing_when_models_absent_and_download_disallowed() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.data_dir).unwrap();
        // Note: we deliberately do NOT fabricate_models(&paths).

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[("alpha", &good_skill_md("alpha", "first"))],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false); // <-- false
        let id: PluginId = "acme/plug".parse().unwrap();

        let err = enable(&id, &deps).expect_err("model-missing");
        assert!(matches!(err, TomeError::ModelMissing { .. }));
    }

    // Cancellation: covered end-to-end by the slice-3 atomicity test (T084).
    // Unit-testing it here would require flipping `catalog::git::CANCELLED`
    // and remembering to flip it back across every other test, which is
    // racy under cargo's parallel runner. Skipped intentionally.
}
