//! Path resolution for the Phase 4 single-root layout.
//!
//! Every Tome-owned path lives under `<home>/.tome/`. There is no
//! XDG-style separation between config / data / cache / state — the
//! constitution v1.3.0 §Paths amendment formalised the new layout, and
//! research §R-1 documents why we walked away from XDG and the
//! `directories` crate (which Tome never actually depended on; Phase 3
//! used raw `HOME` env-var resolution behind XDG-style joins).
//!
//! `home_root()` resolves `<home>` via `std::env::var_os("HOME")`. The
//! `std::env::home_dir` API has been un-deprecated as of Rust 1.85 and
//! is a viable future fallback; until then the raw env-var pattern
//! remains sufficient on every supported platform (Linux, macOS).
//!
//! All path joins in the codebase happen here. No other module
//! constructs Tome-owned paths from string literals — the
//! `tests/no_phase3_paths.rs` structural guard enforces this for the
//! Phase 3 identifier set that's now gone.
//!
//! The Phase 3 `_for(&Scope)` accessor pattern is **gone**. Phase 4
//! has exactly one central `index.db`, one central `index.lock`, and
//! one central global `config.toml`. Per-workspace state moves to
//! either (a) the central database via the `workspace_skills` /
//! `workspace_catalogs` junction tables (F11), or (b)
//! `<root>/workspaces/<name>/{settings.toml,RULES.md}` for harness
//! composition surfaces. Project-bound `.tome/` markers (F2a uses the
//! associated functions on this struct) are thin binding pointers, not
//! databases.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::TomeError;
use crate::workspace::name::WorkspaceName;

/// Resolve `<home>/.tome/`. Inspects `$HOME` directly per research §R-1.
///
/// Validates that `$HOME` is set and (if non-empty) parses as an
/// absolute path. PR-E S-M7 hardens this past the bare-env-var pattern
/// so a developer mis-setting `HOME=~/foo` or `HOME=relative` surfaces
/// as a recognisable error rather than landing Tome state in the cwd.
///
/// We deliberately do NOT canonicalise (which would require the path
/// to exist on disk) — fresh-user setups must work, and the directory
/// is created on demand by the first write.
///
/// # Errors
///
/// - [`TomeError::Usage`] (exit 2) when `$HOME` is unset, empty, or
///   non-absolute. These are user-environment misconfigurations, not
///   filesystem failures.
pub fn home_root() -> Result<PathBuf, TomeError> {
    let home_os = std::env::var_os("HOME").ok_or_else(|| {
        TomeError::Usage("$HOME is not set — cannot resolve the Tome root directory".to_string())
    })?;
    if home_os.is_empty() {
        return Err(TomeError::Usage(
            "$HOME is set to an empty string — cannot resolve the Tome root directory".to_string(),
        ));
    }
    let home = PathBuf::from(home_os);
    if !home.is_absolute() {
        return Err(TomeError::Usage(format!(
            "$HOME is not an absolute path: {}",
            home.display()
        )));
    }
    Ok(home.join(".tome"))
}

/// All resolved Tome-owned paths, derived once at startup from
/// [`home_root`]. Every accessor returns a path strictly inside
/// [`Paths::root`]; no public method allows constructing a Tome path
/// that escapes the root.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub global_config_file: PathBuf,
    pub global_settings_file: PathBuf,
    pub index_db: PathBuf,
    pub index_lock: PathBuf,
    pub catalogs_dir: PathBuf,
    pub models_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub mcp_log: PathBuf,
    pub mcp_log_prev: PathBuf,
    pub workspaces_dir: PathBuf,
}

impl Paths {
    /// Resolve the Phase 4 layout from `$HOME`. The directories
    /// themselves are NOT created here — Tome creates each as needed
    /// (`config.toml` write triggers `<root>/` bootstrap; first
    /// catalog clone triggers `<root>/catalogs/`; etc.).
    pub fn resolve() -> Result<Self, TomeError> {
        let root = home_root()?;
        Ok(Self::from_root(root))
    }

    /// Build a `Paths` rooted at an arbitrary directory. Used by tests
    /// that point at a `TempDir`; the production resolver consumes
    /// [`home_root`] instead.
    pub fn from_root(root: PathBuf) -> Self {
        let global_config_file = root.join("config.toml");
        let global_settings_file = root.join("settings.toml");
        let index_db = root.join("index.db");
        let index_lock = root.join("index.lock");
        let catalogs_dir = root.join("catalogs");
        let models_dir = root.join("models");
        let logs_dir = root.join("logs");
        let mcp_log = logs_dir.join("mcp.log");
        let mcp_log_prev = logs_dir.join("mcp.log.1");
        let workspaces_dir = root.join("workspaces");
        Self {
            root,
            global_config_file,
            global_settings_file,
            index_db,
            index_lock,
            catalogs_dir,
            models_dir,
            logs_dir,
            mcp_log,
            mcp_log_prev,
            workspaces_dir,
        }
    }

    /// Content-addressed catalog clone directory. The sha256(url) hex
    /// digest gives every distinct catalog URL its own directory under
    /// [`Paths::catalogs_dir`]. Refcounting across workspaces lives in
    /// `catalog::store::reference_count` (Phase 3 / US3.b).
    pub fn cache_dir_for(&self, url: &str) -> PathBuf {
        let mut h = Sha256::new();
        h.update(url.as_bytes());
        self.catalogs_dir.join(hex::encode(h.finalize()))
    }

    /// On-disk root for a named model. The directory contains the
    /// model artefact(s) plus a Tome-owned `manifest.json` (see
    /// `ModelManifest`). Rejects empty / traversing / absolute names
    /// at the boundary so callers can rely on the returned path
    /// staying inside [`Paths::models_dir`].
    pub fn model_path(&self, name: &str) -> Result<PathBuf, TomeError> {
        if name.is_empty() {
            return Err(TomeError::Usage("model name is empty".into()));
        }
        if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
            return Err(TomeError::Usage(format!(
                "model name `{name}` contains a path separator or traversal",
            )));
        }
        if Path::new(name).is_absolute() {
            return Err(TomeError::Usage(format!(
                "model name `{name}` is an absolute path",
            )));
        }
        Ok(self.models_dir.join(name))
    }

    /// Per-model manifest path (the JSON file Tome writes after a
    /// verified download).
    pub fn model_manifest(&self, name: &str) -> Result<PathBuf, TomeError> {
        Ok(self.model_path(name)?.join("manifest.json"))
    }

    // --- Workspace accessors --------------------------------------------
    //
    // Workspaces live under `<root>/workspaces/<name>/`. The
    // `WorkspaceName` newtype guarantees the name component is already
    // validated; the join is a pure string operation. No filesystem
    // bootstrap happens here — `tome workspace add` (US2) is the place
    // that calls `land_directory` to create the directory atomically.

    /// `<root>/workspaces/<name>/` — the per-workspace settings root.
    pub fn workspace_dir(&self, name: &WorkspaceName) -> PathBuf {
        self.workspaces_dir.join(name.as_str())
    }

    /// `<root>/workspaces/<name>/settings.toml` — workspace-layer
    /// harness settings (strict TOML).
    pub fn workspace_settings_file(&self, name: &WorkspaceName) -> PathBuf {
        self.workspace_dir(name).join("settings.toml")
    }

    /// `<root>/workspaces/<name>/RULES.md` — summariser output target.
    pub fn workspace_rules_file(&self, name: &WorkspaceName) -> PathBuf {
        self.workspace_dir(name).join("RULES.md")
    }

    // --- Project marker accessors ---------------------------------------
    //
    // Project markers live at `<project_root>/.tome/`. They are
    // independent of `Paths::resolve()` — every accessor below is an
    // associated function rather than a method on `&self`. The contract
    // is: a project marker is a thin binding pointer; the source of
    // truth for the bound workspace's settings lives under
    // `Paths::workspace_dir(&name)`.

    /// `<project_root>/.tome/` — the project's binding marker dir.
    pub fn project_marker_dir(project_root: &Path) -> PathBuf {
        project_root.join(".tome")
    }

    /// `<project_root>/.tome/config.toml` — the project-to-workspace
    /// binding pointer (carries `workspace = "<name>"`).
    pub fn project_marker_config(project_root: &Path) -> PathBuf {
        Self::project_marker_dir(project_root).join("config.toml")
    }

    /// `<project_root>/.tome/RULES.md` — copy of the workspace-layer
    /// summary, materialised inside the project for harness pickup.
    pub fn project_marker_rules(project_root: &Path) -> PathBuf {
        Self::project_marker_dir(project_root).join("RULES.md")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Paths {
        Paths::from_root(PathBuf::from("/tmp/tome-root"))
    }

    #[test]
    fn from_root_places_every_path_under_root() {
        let p = fixture();
        assert_eq!(p.root, PathBuf::from("/tmp/tome-root"));
        assert_eq!(p.global_config_file, p.root.join("config.toml"));
        assert_eq!(p.global_settings_file, p.root.join("settings.toml"));
        assert_eq!(p.index_db, p.root.join("index.db"));
        assert_eq!(p.index_lock, p.root.join("index.lock"));
        assert_eq!(p.catalogs_dir, p.root.join("catalogs"));
        assert_eq!(p.models_dir, p.root.join("models"));
        assert_eq!(p.logs_dir, p.root.join("logs"));
        assert_eq!(p.mcp_log, p.root.join("logs/mcp.log"));
        assert_eq!(p.mcp_log_prev, p.root.join("logs/mcp.log.1"));
        assert_eq!(p.workspaces_dir, p.root.join("workspaces"));
    }

    #[test]
    fn workspace_accessors_compose_under_workspaces_dir() {
        let p = fixture();
        let name = WorkspaceName::global();
        assert_eq!(p.workspace_dir(&name), p.workspaces_dir.join("global"));
        assert_eq!(
            p.workspace_settings_file(&name),
            p.workspaces_dir.join("global/settings.toml"),
        );
        assert_eq!(
            p.workspace_rules_file(&name),
            p.workspaces_dir.join("global/RULES.md"),
        );
    }

    #[test]
    fn project_marker_accessors_are_independent_of_self() {
        let project = PathBuf::from("/abs/project");
        assert_eq!(Paths::project_marker_dir(&project), project.join(".tome"));
        assert_eq!(
            Paths::project_marker_config(&project),
            project.join(".tome/config.toml"),
        );
        assert_eq!(
            Paths::project_marker_rules(&project),
            project.join(".tome/RULES.md"),
        );
    }

    #[test]
    fn cache_dir_is_deterministic_per_url() {
        let p = fixture();
        let a = p.cache_dir_for("https://github.com/owner/repo");
        let b = p.cache_dir_for("https://github.com/owner/repo");
        assert_eq!(a, b);
        let c = p.cache_dir_for("https://github.com/owner/other");
        assert_ne!(a, c);
        assert_eq!(a.file_name().unwrap().to_str().unwrap().len(), 64);
    }

    #[test]
    fn model_path_accepts_simple_name() {
        let p = fixture();
        let got = p.model_path("bge-small-en-v1.5").unwrap();
        assert_eq!(got, p.models_dir.join("bge-small-en-v1.5"));
    }

    #[test]
    fn model_path_rejects_traversal_and_separators() {
        let p = fixture();
        for bad in ["", ".", "..", "../etc", "a/b", "a\\b", "/abs/path"] {
            assert!(
                p.model_path(bad).is_err(),
                "model_path({bad:?}) should have errored",
            );
        }
    }
}
