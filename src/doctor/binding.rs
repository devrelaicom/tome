//! `tome doctor` binding subsystem check (T366).
//!
//! Reports the per-project binding state when the resolved scope was
//! determined via a `.tome/config.toml` project marker. Returns `None`
//! for any other resolution source (FR-564 — doctor outside any project
//! has no binding to report on).
//!
//! ## What we check
//!
//! 1. **Marker well-formedness**: parse `<project>/.tome/config.toml`
//!    via `ProjectMarkerConfig` strict deserialise. Failures collapse to
//!    `config_well_formed = false` and the suggested-fix dispatcher
//!    surfaces a developer-actionable hint.
//! 2. **Workspace registry membership**: the marker may parse fine but
//!    still name a workspace that no longer exists in the central
//!    `workspaces` table (e.g. the workspace was `tome workspace remove`d
//!    while a project still has its marker on disk). This is the
//!    "orphan binding" case from `contracts/doctor-extensions-p4.md`
//!    §New subsystems → `Binding`. We collapse it into
//!    `config_well_formed = false` so both shapes share the existing
//!    Unhealthy classification + non-auto-fixable suggestion path —
//!    the diagnosis text + suggested commands are the same ("rebind via
//!    `tome workspace use <existing-name>` or recreate the named
//!    workspace via `tome workspace init <name>`").
//! 3. **Rules-copy currency**: byte-compare `<project>/.tome/RULES.md`
//!    against `<root>/workspaces/<name>/RULES.md`:
//!    - Source missing OR project copy missing → `Missing`.
//!    - Bytes equal → `Match`.
//!    - Bytes differ → `Drift`.
//!
//! The check is pure FS / read-only — never mutates anything, never
//! takes the advisory lock. Errors at the filesystem layer that are
//! NOT NotFound (permissions, unreadable directories) collapse to the
//! most-pessimistic classification rather than propagating: doctor is
//! the diagnostic that *surfaces* failures, it must not itself error
//! out on them.

use crate::paths::Paths;
use crate::settings::ProjectMarkerConfig;
use crate::workspace::{ResolvedScope, ScopeSource};

use super::report::{ProjectBindingState, RulesCopyState};

/// Public entry point. See module docs.
pub fn check_binding(scope: &ResolvedScope, paths: &Paths) -> Option<ProjectBindingState> {
    if !matches!(scope.source, ScopeSource::ProjectMarker) {
        return None;
    }
    let project_root = scope.project_root.as_deref()?;
    let marker_path = Paths::project_marker_config(project_root);

    // Parse the marker. Failures (NotFound, permission denied, malformed
    // TOML, deny_unknown_fields trip) all collapse to
    // `config_well_formed = false`. A well-formed marker is additionally
    // gated on the bound workspace's presence in the central registry
    // (US5.b orphan-binding case): a marker that parses but names a
    // missing workspace is functionally as broken as a malformed marker
    // — the same Unhealthy classification + non-auto-fixable suggestion
    // path covers both.
    let marker_parses = std::fs::read_to_string(&marker_path)
        .ok()
        .and_then(|body| toml::from_str::<ProjectMarkerConfig>(&body).ok())
        .is_some();
    let workspace_registered = bound_workspace_is_registered(scope, paths);
    let config_well_formed = marker_parses && workspace_registered;

    // RULES.md drift comparison. The resolved scope already carries the
    // bound workspace name; we read the source-of-truth file at
    // `<root>/workspaces/<name>/RULES.md` and compare bytes against the
    // project's copy.
    let rules_file_drift = compare_rules(scope, paths, project_root);

    Some(ProjectBindingState {
        project_root: project_root.to_path_buf(),
        bound_workspace: scope.scope.name().clone(),
        config_well_formed,
        rules_file_drift,
    })
}

/// True iff the scope's bound workspace name has a row in the central
/// `workspaces` table. Bootstrap-not-yet (no `index.db` on disk) is
/// treated as "only `global` is known" — same shortcut the production
/// `CentralDbScopeProvider` uses.
///
/// Read-only; never propagates errors. Any DB-side failure (file
/// unreadable, schema-too-new, query error) collapses to `false` so
/// doctor surfaces the binding as broken rather than itself crashing.
fn bound_workspace_is_registered(scope: &ResolvedScope, paths: &Paths) -> bool {
    let name = scope.scope.name();
    // Bootstrap-not-yet: no central DB on disk, so there is no
    // authoritative registry to consult. Treat this as permissively
    // registered — the user's next bootstrap-creating command (catalog
    // add, plugin enable, workspace init, …) will seed the registry and
    // a subsequent doctor pass will re-evaluate. This avoids flagging
    // every fresh-install project as broken.
    if !paths.index_db.exists() {
        return true;
    }
    let conn = match crate::index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(_) => return false,
    };
    conn.query_row(
        "SELECT 1 FROM workspaces WHERE name = ?1",
        rusqlite::params![name.as_str()],
        |_| Ok(()),
    )
    .is_ok()
}

fn compare_rules(
    scope: &ResolvedScope,
    paths: &Paths,
    project_root: &std::path::Path,
) -> RulesCopyState {
    let source_path = paths.workspace_rules_file(scope.scope.name());
    let project_copy = Paths::project_marker_rules(project_root);

    let source = std::fs::read(&source_path);
    let copy = std::fs::read(&project_copy);
    match (source, copy) {
        (Ok(s), Ok(c)) => {
            if s == c {
                RulesCopyState::Match
            } else {
                RulesCopyState::Drift
            }
        }
        // Any missing side collapses to Missing — the auto-fix is the
        // same (re-copy from workspace). A read error on either side is
        // also Missing for the same reason; doctor's recommendation is
        // identical to the missing case.
        _ => RulesCopyState::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Scope, WorkspaceName};
    use tempfile::TempDir;

    fn project_scope(project_root: std::path::PathBuf, ws: &str) -> ResolvedScope {
        ResolvedScope {
            scope: Scope(WorkspaceName::parse(ws).unwrap()),
            source: ScopeSource::ProjectMarker,
            project_root: Some(project_root),
        }
    }

    #[test]
    fn returns_none_when_scope_source_is_not_project_marker() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let scope = ResolvedScope::global_fallback();
        assert!(check_binding(&scope, &paths).is_none());
    }

    #[test]
    fn reports_missing_when_neither_source_nor_copy_exist() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let project_dir = tmp.path().join("project");
        std::fs::create_dir_all(project_dir.join(".tome")).unwrap();
        std::fs::write(
            project_dir.join(".tome/config.toml"),
            "workspace = \"global\"\n",
        )
        .unwrap();

        let scope = project_scope(project_dir, "global");
        let state = check_binding(&scope, &paths).unwrap();
        assert!(state.config_well_formed);
        assert_eq!(state.rules_file_drift, RulesCopyState::Missing);
    }

    #[test]
    fn reports_match_when_bytes_align() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let name = WorkspaceName::parse("alpha").unwrap();
        let src = paths.workspace_rules_file(&name);
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, b"hello rules\n").unwrap();

        let project_dir = tmp.path().join("project");
        std::fs::create_dir_all(project_dir.join(".tome")).unwrap();
        std::fs::write(
            project_dir.join(".tome/config.toml"),
            "workspace = \"alpha\"\n",
        )
        .unwrap();
        std::fs::write(project_dir.join(".tome/RULES.md"), b"hello rules\n").unwrap();

        let scope = project_scope(project_dir, "alpha");
        let state = check_binding(&scope, &paths).unwrap();
        assert_eq!(state.rules_file_drift, RulesCopyState::Match);
    }

    #[test]
    fn reports_drift_when_bytes_differ() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let name = WorkspaceName::parse("beta").unwrap();
        let src = paths.workspace_rules_file(&name);
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, b"v1\n").unwrap();

        let project_dir = tmp.path().join("project");
        std::fs::create_dir_all(project_dir.join(".tome")).unwrap();
        std::fs::write(
            project_dir.join(".tome/config.toml"),
            "workspace = \"beta\"\n",
        )
        .unwrap();
        std::fs::write(project_dir.join(".tome/RULES.md"), b"hand-edited\n").unwrap();

        let scope = project_scope(project_dir, "beta");
        let state = check_binding(&scope, &paths).unwrap();
        assert_eq!(state.rules_file_drift, RulesCopyState::Drift);
    }

    #[test]
    fn reports_config_malformed_when_marker_unparsable() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let project_dir = tmp.path().join("project");
        std::fs::create_dir_all(project_dir.join(".tome")).unwrap();
        // Missing required `workspace` field.
        std::fs::write(project_dir.join(".tome/config.toml"), "garbage = true\n").unwrap();

        let scope = project_scope(project_dir, "global");
        let state = check_binding(&scope, &paths).unwrap();
        assert!(!state.config_well_formed);
    }
}
