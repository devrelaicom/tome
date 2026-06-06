//! Built-in `${TOME_*}` placeholder substitution.
//!
//! Stage 1 of the substitution pipeline per
//! `contracts/substitution-engine.md` § Stage 1. As of US2.d B2, Stage 1
//! and Stage 2 are scanned in a SINGLE regex pass (see
//! [`super::render`]); this module exposes [`resolve_builtin`] which is
//! called from the unified per-match loop.
//!
//! Unknown names pass through verbatim with `tracing::debug!` per
//! FR-023.

use time::format_description::well_known::Rfc3339;

use super::{SubstitutionContext, SubstitutionError, data_dir};
use crate::workspace::WorkspaceName;

/// Resolve one recognised `${TOME_<NAME>}` built-in to its string value.
///
/// Returns:
/// - `Ok(Some(value))` for a recognised name — built-ins are always set,
///   so `default` is never consulted in practice (the syntax is supported
///   for stage-2 mirroring per FR-022).
/// - `Ok(None)` for an unknown name — the caller leaves the match
///   verbatim and `tracing::debug!`s.
/// - `Err(_)` if a side-effect (e.g. `create_dir_all` for `PLUGIN_DATA`
///   / `WORKSPACE_DATA`) fails.
pub(super) fn resolve_builtin(
    name: &str,
    ctx: &SubstitutionContext,
    _default: Option<&str>,
) -> Result<Option<String>, SubstitutionError> {
    let value = match name {
        // Entry-level paths.
        "SKILL_DIR" => ctx.entry_dir.to_string_lossy().into_owned(),
        "SKILL_PATH" => ctx.entry_path.to_string_lossy().into_owned(),
        "SKILL_NAME" => ctx.entry_name.clone(),

        // Plugin-level scalars + paths.
        "PLUGIN_DIR" => ctx.plugin_root_dir.to_string_lossy().into_owned(),
        "PLUGIN_NAME" => ctx.plugin_name.clone(),
        "PLUGIN_VERSION" => ctx.plugin_version.clone(),
        "PLUGIN_DATA" => {
            let dir =
                data_dir::ensure_plugin_data(&ctx.paths, &ctx.catalog_name, &ctx.plugin_name)?;
            dir.to_string_lossy().into_owned()
        }

        // Catalog scalar.
        "CATALOG_NAME" => ctx.catalog_name.clone(),

        // Project-level path (Phase 8, R6). Resolved once at context-build time
        // (`ResolvedScope.project_root` → fresh CWD marker-walk). When no
        // project exists up-tree the token passes through VERBATIM (`Ok(None)`),
        // never empty-string — so `${TOME_PROJECT_DIR}/run.sh` cannot collapse
        // to the absolute root `/run.sh`.
        "PROJECT_DIR" => match &ctx.project_dir {
            Some(dir) => dir.to_string_lossy().into_owned(),
            None => return Ok(None),
        },

        // Workspace scalar + path.
        "WORKSPACE_NAME" => ctx.workspace_name.clone(),
        "WORKSPACE_DATA" => {
            // The substitution context carries the workspace name as a
            // plain String (built via the builder for ergonomic reasons).
            // The Paths accessor wants a `WorkspaceName` newtype, which
            // enforces FR-347 validation. Workspaces in the index are
            // always built through this newtype upstream, so this branch
            // is unreachable from production; the defensive map exists
            // for synthetic test contexts only. US2.d B1 fixed the
            // error-variant misclassification — this is a workspace-data
            // failure (exit 25) not a plugin-data failure (exit 9).
            let ws = WorkspaceName::parse(&ctx.workspace_name).map_err(|err| {
                // Defensive only — unreachable from production callers
                // (workspace names flow through `WorkspaceName::parse`
                // upstream). On the off chance a synthetic test context
                // smuggles in an invalid name, surface as a
                // workspace-data creation failure (US2.d B1: previously
                // misrouted through PluginDataDirCreationFailed / exit 9).
                SubstitutionError::WorkspaceDataDirCreationFailed {
                    path: ctx.paths.root.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("invalid workspace name `{}`: {err}", ctx.workspace_name),
                    ),
                }
            })?;
            let dir = data_dir::ensure_workspace_data(
                &ctx.paths,
                &ws,
                &ctx.catalog_name,
                &ctx.plugin_name,
            )?;
            dir.to_string_lossy().into_owned()
        }

        // Clock-derived scalars per § Stage 1 table.
        "DATE" => format!(
            "{:04}-{:02}-{:02}",
            ctx.clock.year(),
            u8::from(ctx.clock.month()),
            ctx.clock.day()
        ),
        "TIMESTAMP" => ctx
            .clock
            .format(&Rfc3339)
            .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z")),

        // Unknown name in the TOME_ namespace: pass through verbatim.
        // The caller is expected to log at debug level per FR-023.
        _ => return Ok(None),
    };
    Ok(Some(value))
}
