//! Built-in `${TOME_*}` placeholder substitution.
//!
//! Stage 1 of the substitution pipeline per
//! `contracts/substitution-engine.md` § Stage 1. The regex scans the
//! body once; each match is dispatched to [`resolve_builtin`] which
//! walks a whitelist of 12 recognised names. Unknown names pass through
//! verbatim with `tracing::debug!` per FR-023.

use time::format_description::well_known::Rfc3339;

use super::{SubstitutionContext, SubstitutionError, data_dir, regex_sets};
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

        // Workspace scalar + path.
        "WORKSPACE_NAME" => ctx.workspace_name.clone(),
        "WORKSPACE_DATA" => {
            // The substitution context carries the workspace name as a
            // plain String (built via the builder for ergonomic reasons).
            // The Paths accessor wants a `WorkspaceName` newtype, which
            // enforces FR-347 validation. Workspaces in the index are
            // always built through this newtype upstream, so re-parsing
            // here cannot fail in practice; on the off chance a synthetic
            // test context smuggles in an invalid name, surface it as a
            // data-dir creation failure rather than panicking.
            let ws = WorkspaceName::parse(&ctx.workspace_name).map_err(|err| {
                SubstitutionError::PluginDataDirCreationFailed {
                    path: ctx.workspace_data_dir.clone(),
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

/// Apply built-in `${TOME_*}` placeholder substitution.
///
/// Scans `body` once via [`regex_sets::builtin_regex`] and concatenates
/// the rendered output into a fresh `String`. Each match is resolved via
/// [`resolve_builtin`]; on `Ok(None)` the match is emitted verbatim and
/// a debug-level event records the unknown reference.
///
/// Substituted values are NOT re-scanned by later stages (FR-051 — the
/// pipeline is single-pass).
pub(super) fn apply_builtins(
    body: &str,
    ctx: &SubstitutionContext,
) -> Result<String, SubstitutionError> {
    let re = regex_sets::builtin_regex();
    let mut out = String::with_capacity(body.len());
    let mut last_end = 0;
    for caps in re.captures_iter(body) {
        // Group 0 is guaranteed by `captures_iter` to exist.
        let m = caps.get(0).expect("regex group 0 always present");
        out.push_str(&body[last_end..m.start()]);
        let name = caps.get(1).map(|c| c.as_str()).unwrap_or("");
        let default = caps.get(2).map(|c| c.as_str());
        match resolve_builtin(name, ctx, default)? {
            Some(value) => out.push_str(&value),
            None => {
                tracing::debug!(
                    target: "tome::substitution",
                    builtin = name,
                    "unknown TOME_ built-in; leaving verbatim",
                );
                out.push_str(m.as_str());
            }
        }
        last_end = m.end();
    }
    out.push_str(&body[last_end..]);
    Ok(out)
}
