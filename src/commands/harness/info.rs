//! `tome harness info <name>` — per-harness details for the current
//! project.
//!
//! Reports: identity, detection (with the per-user dir probed),
//! rules-file + MCP config targets, currently-integrated state
//! (rules block + MCP entry presence), and which settings scopes
//! reference this harness.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::HarnessInfoArgs;
use crate::error::TomeError;
use crate::harness::{
    BlockBodyStyle, McpDialect, RulesFileStrategy, mcp_config, rules_file, with_effective_modules,
};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::settings::resolver::resolve_effective_list;
use crate::workspace::ResolvedScope;
use crate::{index, index::skills::enabled_agents_for_workspace};

use super::home_root;

/// One reason this harness appears in the effective list. Each entry
/// names a settings scope plus, optionally, the composition reference
/// in another scope that pulled this harness in.
///
/// Examples:
///
/// * `{ scope: "project", via: None }` — declared directly in the
///   project marker.
/// * `{ scope: "project", via: Some("[global]") }` — declared in global
///   settings, pulled into the effective list by the project marker's
///   `[global]` reference.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HarnessReference {
    pub scope: String,
    pub via: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessInfoOutcome {
    pub name: String,
    pub description: String,
    pub detected: bool,
    pub detected_path: PathBuf,
    pub rules_target: Option<PathBuf>,
    pub mcp_target: Option<PathBuf>,
    pub rules_block_present: Option<bool>,
    pub mcp_entry_present: Option<bool>,
    pub mcp_tome_owned: Option<bool>,
    /// Why this harness appears in the effective list.
    ///
    /// Populated from the resolver's `source_chain` for the named
    /// harness when a project is resolved: the first chain element is
    /// the entry-point scope (e.g. `"project"`), and any subsequent
    /// element is the composition reference that pulled this harness
    /// into the effective list (e.g. `"[global]"`). When `project_root`
    /// is `None`, the resolver cannot run with project + workspace
    /// context, so this falls back to direct-declaration scanning of
    /// workspace + global settings (C-B3 from US3 review).
    pub references: Vec<HarnessReference>,
    /// Phase 11 / US5 (T063): the paste-able MCP-server snippet — the EXACT
    /// bytes Tome would write for this harness's [`McpDialect`], built with
    /// the canonical `["mcp", "--workspace", "<ws>", "--harness", "<name>"]`
    /// args. For a manual-only harness (jetbrains-ai) this is the primary
    /// recovery artifact; for every harness it is the self-heal paste target.
    ///
    /// Appended LAST + `skip_serializing_if`-gated so the byte-stable `--json`
    /// pins for the pre-Phase-11 fields don't move. Always populated in
    /// practice (every dialect renders), but `Option` keeps the wire shape
    /// additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_snippet: Option<String>,
    /// Phase 2 (native-agent expansion) / Task 14: advisory notice when this
    /// harness is rules-only (does not support native agents AND is not an
    /// opt-in target) AND there are ≥1 enabled agents in scope. The notice
    /// explains how to surface the agents as MCP prompts via
    /// `expose_agents_as_personas`. `None` for native-supporting harnesses
    /// (claude-code/codex/cursor/opencode/gemini), opt-in targets
    /// (generic/generic-op), or when zero agents are enabled. Rules-only
    /// harnesses such as jetbrains-ai, cline, junie, crush, and antigravity
    /// WILL show the notice when agents are enabled.
    ///
    /// Appended after `mcp_snippet` + `skip_serializing_if`-gated (absent ⇒
    /// key omitted) to keep the byte-stable pin additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unrepresented_agents_notice: Option<String>,
    /// US11 (native plugin-hook translation): advisory notice for harnesses
    /// that support hook translation (`hook_support().is_some()`). Shows the
    /// on/off state, registered event count, dropped-to-GUARDRAILS count, and
    /// prompt-model availability. `None` for harnesses without hook translation.
    ///
    /// Appended after `unrepresented_agents_notice` + `skip_serializing_if`-gated
    /// so byte-stable pins are additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_translation_notice: Option<String>,
    /// Issue #292 (translation-fidelity loss): advisory notice when this harness
    /// is rules-only for hooks (no `RealJson` sink, no `#318` dispatcher, not an
    /// opt-in target) AND ≥1 enabled plugin ships hooks in scope. The notice
    /// explains that those hooks are rendered as `GUARDRAILS.md` prose only, not
    /// enforced natively — the hooks analogue of `unrepresented_agents_notice`.
    /// `None` for hook-capable harnesses (claude-code / the five `#318`
    /// harnesses), for the opt-in targets `generic` / `generic-op`, or when no
    /// enabled plugin ships hooks. `goose` DOES show the notice — it is a
    /// detectable harness with no native hook path (rules-only for hooks).
    ///
    /// Appended after `hook_translation_notice` + `skip_serializing_if`-gated so
    /// byte-stable pins are additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks_notice: Option<String>,
}

/// Per-harness snapshot captured outside the registry's read guard.
struct ModuleSnapshot {
    name: String,
    description: String,
    rules_strategy: RulesFileStrategy,
    mcp_dialect: McpDialect,
    detected: bool,
    detected_path: PathBuf,
    rules_target: Option<PathBuf>,
    mcp_target: Option<PathBuf>,
    #[allow(dead_code)]
    block_body_style: BlockBodyStyle,
    /// Whether this harness supports native translated agents. Captured from
    /// [`HarnessModule::supports_native_agents`] so callers outside the read
    /// guard can test it.
    module_supports_native_agents: bool,
    /// Whether this harness is an opt-in target (generic/generic-op). Captured
    /// from [`HarnessModule::is_opt_in_target`] so the notice gate outside the
    /// read guard can exclude opt-in targets (which have no native-agent
    /// directory either way, making the notice misleading).
    module_is_opt_in_target: bool,
    /// US11: the portable events supported by this harness's hook translation,
    /// captured from `hook_support().map(|hs| hs.events)`. `None` when the
    /// harness has no hook translation support.
    module_hook_events: Option<Vec<crate::harness::hooks_ir::PortableEvent>>,
    /// Issue #292: whether this harness is rules-only for hooks (no `RealJson`
    /// sink, no `#318` dispatcher, not an opt-in target). Captured from the SSOT
    /// [`HarnessModule::is_rules_only_for_hooks`] so the hooks notice gate outside
    /// the read guard mirrors the doctor/status definition exactly.
    module_is_rules_only_for_hooks: bool,
}

pub fn run(
    args: HarnessInfoArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    match args.name.as_deref() {
        // Single-harness form — BYTE-IDENTICAL to the pre-#327 behaviour
        // (human section AND a single `--json` object).
        Some(name) => {
            let outcome = build_info(name, scope, paths)?;
            match mode {
                Mode::Human => emit_human(&outcome),
                Mode::Json => write_json(&outcome),
            }
        }
        // No name — report one section per harness in the effective list (the
        // same set `harness list` reports). Human: a section per harness with a
        // blank-line separator. Json: an ARRAY of the per-harness outcomes.
        None => run_effective(scope, paths, mode),
    }
}

/// No-name `tome harness info` — build a `HarnessInfoOutcome` for every harness
/// in the effective list and emit them as sections (human) / an array (json).
///
/// The effective NAME set is resolved via [`resolve_effective_list`], mirroring
/// `harness list`. An empty effective set degrades gracefully: a one-line hint
/// (human) / `[]` (json), exit 0 — never an error.
fn run_effective(scope: &ResolvedScope, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let outcomes = build_effective_outcomes(scope, paths)?;
    match mode {
        Mode::Human => emit_human_sections(&outcomes),
        Mode::Json => write_json(&outcomes),
    }
}

/// Build a [`HarnessInfoOutcome`] for every harness in the effective list (the
/// same set `harness list` reports). Emit-free, so a caller/test can serialise
/// the returned vec to inspect the real no-name `--json` ARRAY wire shape. An
/// empty effective set yields an empty vec (serialises as `[]`), never an error.
pub fn build_effective_outcomes(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Vec<HarnessInfoOutcome>, TomeError> {
    let names = effective_harness_names(scope, paths)?;
    let mut outcomes = Vec::with_capacity(names.len());
    for name in &names {
        outcomes.push(build_info(name, scope, paths)?);
    }
    Ok(outcomes)
}

/// Resolve the effective harness NAMES for the current project, mirroring
/// `harness list`'s `list_effective`. Returns an empty vec (not an error) when
/// nothing is configured.
fn effective_harness_names(scope: &ResolvedScope, paths: &Paths) -> Result<Vec<String>, TomeError> {
    let marker = super::list::load_project_marker_for_use(scope)?;
    let workspace_settings = super::list::load_workspace_settings_for_use(scope, paths)?;
    let global_settings = super::list::load_global_settings_for_use(paths)?;
    let provider = super::CentralDbScopeProvider::new(paths);

    let resolved = resolve_effective_list(
        marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        &provider,
    )
    .map_err(TomeError::from)?;

    Ok(resolved.harnesses.into_iter().map(|h| h.name).collect())
}

/// Build the [`HarnessInfoOutcome`] for a single harness `name`. Extracted from
/// `run` so both the `Some(name)` single form and the `None` effective-list
/// form share one resolution + probe + assembly path. An unknown explicit name
/// resolves to `HarnessNotSupported` here (unchanged).
fn build_info(
    name: &str,
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<HarnessInfoOutcome, TomeError> {
    let home = home_root()?;
    let project_root = scope.project_root.clone();
    // Snapshot the named module's fields. Build the `ModuleSnapshot` from a
    // `&dyn HarnessModule` so the same closure serves both the override-aware
    // `with_effective_modules` path and the opt-in `lookup` fallback below.
    let snapshot_of = |m: &dyn crate::harness::HarnessModule| ModuleSnapshot {
        name: m.name().to_string(),
        description: m.description().to_string(),
        rules_strategy: m.rules_file_strategy(),
        mcp_dialect: m.mcp_dialect(),
        detected: m.detect(&home),
        detected_path: m.detect_path(&home),
        rules_target: project_root.as_deref().map(|p| m.rules_file_target(p)),
        mcp_target: project_root.as_deref().map(|p| m.mcp_config_path(p, &home)),
        block_body_style: m.block_body_style(),
        module_supports_native_agents: m.supports_native_agents(),
        module_is_opt_in_target: m.is_opt_in_target(),
        module_hook_events: m.hook_support().map(|hs| hs.events.to_vec()),
        module_is_rules_only_for_hooks: m.is_rules_only_for_hooks(),
    };
    // Phase 11 / US4 (M1): resolve via the effective registry FIRST (so a test
    // override and the supported harnesses both work), then fall back to the
    // alias+opt-in-aware `lookup` so `tome harness info generic` / `generic-op`
    // resolve their opt-in modules rather than erroring `HarnessNotSupported`.
    // `lookup` does not consult the override slot, but the override branch has
    // already matched when one is installed.
    let snap = with_effective_modules(|mods| {
        mods.iter()
            .find(|m| m.name() == name)
            .map(|m| snapshot_of(*m))
    })
    .or_else(|| crate::harness::lookup(name).map(snapshot_of))
    .ok_or_else(|| TomeError::HarnessNotSupported {
        name: name.to_string(),
    })?;

    let (rules_block_present, mcp_entry_present, mcp_tome_owned) =
        match (&snap.rules_target, &snap.mcp_target) {
            (Some(rules_path), Some(mcp_path)) => {
                let block_present = probe_rules_block(rules_path, snap.rules_strategy)?;
                let entry = mcp_config::read_entry(mcp_path, &snap.mcp_dialect)?;
                let entry_present = entry.is_some();
                let entry_tome_owned = entry.as_ref().map(mcp_config::is_tome_owned);
                (Some(block_present), Some(entry_present), entry_tome_owned)
            }
            _ => (None, None, None),
        };

    let references = collect_references(scope, paths, &snap.name)?;

    // Phase 2 / Task 14: compute the enabled-agent count for this workspace,
    // used to gate the unrepresented-agents notice. Read-only; guards on the
    // DB existing and the workspace being resolvable. Zero when the DB is
    // absent or the query fails (notice omitted — prefer silence over noise).
    let enabled_agent_count: u32 = if paths.index_db.is_file() {
        index::open_read_only(&paths.index_db)
            .ok()
            .and_then(|conn| {
                let ws_name = scope.scope.name();
                enabled_agents_for_workspace(&conn, ws_name.as_str())
                    .ok()
                    .map(|v| u32::try_from(v.len()).unwrap_or(u32::MAX))
            })
            .unwrap_or(0)
    } else {
        0
    };

    // "Rules-only" means not a native-agent-capable harness AND not an opt-in
    // target (generic/generic-op). This matches the definition used by
    // `fill_unrepresented_agents` in `status.rs` and the doctor drop-report.
    let unrepresented_agents_notice = if !snap.module_supports_native_agents
        && !snap.module_is_opt_in_target
        && enabled_agent_count > 0
    {
        Some(if enabled_agent_count == 1 {
            "1 enabled agent has no native agent form on this harness; \
                 enable `expose_agents_as_personas` to surface it as an MCP prompt."
                .to_string()
        } else {
            format!(
                "{enabled_agent_count} enabled agents have no native agent form on this \
                     harness; enable `expose_agents_as_personas` to surface them as MCP prompts."
            )
        })
    } else {
        None
    };

    // US11: hook-translation notice for harnesses that support it. Read-only.
    let hook_translation_notice = snap.module_hook_events.as_ref().map(|supported_events| {
        let cfg = crate::config::load_or_default(paths);
        let enabled = cfg.hooks.translate_plugin_hooks.unwrap_or(true);
        let has_prompt = cfg.hooks.prompt_provider.is_some() || cfg.hooks.prompt_model.is_some();

        let manifest_path = paths.hooks_manifest(scope.scope.name(), &snap.name);
        let manifest = crate::harness::hooks_ir::read_manifest(&manifest_path).ok();
        let registered = manifest.as_ref().map(|m| m.events.len()).unwrap_or(0);

        let dropped = crate::harness::hooks_ir::PortableEvent::ALL
            .iter()
            .filter(|e| !supported_events.contains(e))
            .count();

        let state = if enabled { "on" } else { "off" };
        let mut parts = vec![
            format!("hook translation {state}"),
            format!("{registered} event(s) registered"),
        ];
        if dropped > 0 {
            parts.push(format!("{dropped} event(s) dropped to GUARDRAILS"));
        }
        if has_prompt {
            parts.push("prompt-model configured (first execution may request trust)".to_string());
        }
        parts.join("; ")
    });

    // Issue #292: hooks notice for a rules-only-for-hooks harness — the hooks
    // analogue of `unrepresented_agents_notice`. Shown only when THIS harness
    // cannot deliver hooks natively (SSOT `is_rules_only_for_hooks`) AND ≥1
    // enabled plugin ships hooks in scope. Counted via the SAME SSOT the doctor
    // report + status count use (`build_unrepresented_hooks_report`), scoped to
    // this single harness — so all three surfaces resolve the set identically.
    let hooks_notice = if snap.module_is_rules_only_for_hooks {
        let single = crate::settings::resolver::EffectiveHarnessList {
            harnesses: vec![crate::settings::resolver::EffectiveHarness {
                name: snap.name.clone(),
                source_chain: Vec::new(),
            }],
            excluded: Vec::new(),
        };
        let unrepresented = crate::doctor::checks::build_unrepresented_hooks_report(
            paths,
            scope.scope.name(),
            &home,
            Some(&single),
        )
        .map(|r| r.hooks.len())
        .unwrap_or(0);
        if unrepresented > 0 {
            Some(if unrepresented == 1 {
                "1 enabled plugin hook has no native form on this harness; \
                 it is rendered as GUARDRAILS.md prose only, not enforced."
                    .to_string()
            } else {
                format!(
                    "{unrepresented} enabled plugin hooks have no native form on this \
                     harness; they are rendered as GUARDRAILS.md prose only, not enforced."
                )
            })
        } else {
            None
        }
    } else {
        None
    };

    // Phase 11 / US5 (T063): render the paste-able MCP snippet from the
    // harness's dialect, built with the canonical args the sync writer uses
    // (`mcp --workspace <ws> --harness <name>`, the `--harness` trailing so
    // the ownership marker survives). Keyed off the resolved workspace name +
    // the harness name.
    //
    // #337 Phase B: the snippet's `command` is the RESOLVED launcher
    // (`tome_command()`), CONSISTENT with what `sync` writes — so a snippet
    // pasted into a PATH-less / sandboxed host (the whole point of #337) starts
    // the server, and the snippet matches the writer's `command` byte-for-byte
    // (modulo surrounding developer content `write_entry` preserves). The
    // resolved launcher is an absolute path that copy-pastes across machines as
    // a literal; on a normal install it is `…/tome`, recognised as Tome-owned by
    // the basename arm of `looks_like_tome_launcher`.
    let snippet_entry = mcp_config::TomeEntry::new(
        crate::harness::launcher::tome_command(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            scope.scope.name().as_str().to_string(),
            "--harness".to_string(),
            snap.name.clone(),
        ],
    );
    let mcp_snippet = Some(mcp_config::render_entry_snippet(
        &snap.mcp_dialect,
        &snippet_entry,
    ));

    Ok(HarnessInfoOutcome {
        name: snap.name,
        description: snap.description,
        detected: snap.detected,
        detected_path: snap.detected_path,
        rules_target: snap.rules_target,
        mcp_target: snap.mcp_target,
        rules_block_present,
        mcp_entry_present,
        mcp_tome_owned,
        references,
        mcp_snippet,
        unrepresented_agents_notice,
        hook_translation_notice,
        hooks_notice,
    })
}

fn probe_rules_block(
    target: &std::path::Path,
    strategy: RulesFileStrategy,
) -> Result<bool, TomeError> {
    match strategy {
        RulesFileStrategy::BlockInExistingFile => {
            let body =
                match crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX) {
                    Ok(s) => s,
                    Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(false);
                    }
                    Err(e) => return Err(e),
                };
            Ok(rules_file::parse_block(&body)?.is_some())
        }
        RulesFileStrategy::StandaloneFile => Ok(target.exists()),
    }
}

/// Collect references that contribute the named harness to the
/// effective list (C-B3 from US3 review).
///
/// Compute the effective list via the production resolver and find
/// every entry with `name == name`. For each match, translate its
/// `source_chain` into a [`HarnessReference`]:
///
/// * Chain of length 1 (e.g. `["project"]`) → `{ scope: "project",
///   via: None }` — direct declaration.
/// * Longer chain (e.g. `["project", "[global]"]`) → `{ scope:
///   "project", via: Some("[global]") }` — pulled in via the last
///   composition reference. The intermediate steps narrate the path but
///   the user-actionable signal is "disabling the last reference would
///   remove this harness".
///
/// When the resolver fails (e.g. malformed settings or an unknown
/// `[workspaces.<name>]` reference) we fall back to direct-declaration
/// scanning rather than propagating the error — `tome harness info` is
/// a diagnostic; a partial report beats no report.
fn collect_references(
    scope: &ResolvedScope,
    paths: &Paths,
    name: &str,
) -> Result<Vec<HarnessReference>, TomeError> {
    // Load the three settings layers.
    let marker = super::list::load_project_marker_for_use(scope)?;
    let workspace_settings = super::list::load_workspace_settings_for_use(scope, paths)?;
    let global_settings = super::list::load_global_settings_for_use(paths)?;

    let provider = super::CentralDbScopeProvider::new(paths);
    let resolved = resolve_effective_list(
        marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        &provider,
    );

    match resolved {
        Ok(list) => {
            let mut references: Vec<HarnessReference> = list
                .harnesses
                .into_iter()
                .filter(|h| h.name == name)
                .map(|h| {
                    let scope = h.source_chain.first().cloned().unwrap_or_default();
                    let via = if h.source_chain.len() > 1 {
                        h.source_chain.last().cloned()
                    } else {
                        None
                    };
                    HarnessReference { scope, via }
                })
                .collect();
            references.dedup();
            Ok(references)
        }
        Err(_) => {
            // Fall back to direct-declaration scanning when the
            // resolver can't run cleanly. This is the pre-C-B3 behaviour
            // generalised into the new wire shape.
            let mut found = Vec::new();
            if let Some(m) = marker.as_ref()
                && let Some(list) = m.harnesses.as_ref()
                && list.iter().any(|n| n == name)
            {
                found.push(HarnessReference {
                    scope: "project".to_string(),
                    via: None,
                });
            }
            if let Some(ws) = workspace_settings.as_ref()
                && let Some(list) = ws.harnesses.as_ref()
                && list.iter().any(|n| n == name)
            {
                found.push(HarnessReference {
                    scope: "workspace".to_string(),
                    via: None,
                });
            }
            if let Some(list) = global_settings.enabled.as_ref()
                && list.iter().any(|n| n == name)
            {
                found.push(HarnessReference {
                    scope: "global".to_string(),
                    via: None,
                });
            }
            Ok(found)
        }
    }
}

fn emit_human(outcome: &HarnessInfoOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "Harness: {}", outcome.name)?;
    writeln!(out, "  Description:     {}", outcome.description)?;
    writeln!(
        out,
        "  Detected:        {} (probed {})",
        if outcome.detected { "yes" } else { "no" },
        outcome.detected_path.display(),
    )?;
    match &outcome.rules_target {
        Some(p) => writeln!(out, "  Rules-file:      {}", p.display())?,
        None => writeln!(out, "  Rules-file:      — (no project resolved)")?,
    }
    match &outcome.mcp_target {
        Some(p) => writeln!(out, "  MCP config:      {}", p.display())?,
        None => writeln!(out, "  MCP config:      — (no project resolved)")?,
    }
    match outcome.rules_block_present {
        Some(true) => writeln!(out, "  Rules block:     present")?,
        Some(false) => writeln!(out, "  Rules block:     absent")?,
        None => {}
    }
    match (outcome.mcp_entry_present, outcome.mcp_tome_owned) {
        (Some(true), Some(true)) => writeln!(out, "  MCP entry:       present (Tome-owned)")?,
        (Some(true), Some(false)) => writeln!(out, "  MCP entry:       present (user-owned)")?,
        (Some(false), _) => writeln!(out, "  MCP entry:       absent")?,
        _ => {}
    }
    if outcome.references.is_empty() {
        writeln!(out, "  References:      (none)")?;
    } else {
        for r in &outcome.references {
            match &r.via {
                None => writeln!(out, "  References:      {} (direct)", r.scope)?,
                Some(via) => writeln!(out, "  References:      {} via {}", r.scope, via)?,
            }
        }
    }
    // Phase 11 / US5 (T063): the paste-able MCP-config snippet. For a
    // manual-only harness (jetbrains-ai) this is how a user adds the Tome
    // server by hand; for every harness it is the self-heal paste target the
    // rules preamble points at.
    if let Some(snippet) = &outcome.mcp_snippet {
        writeln!(out)?;
        writeln!(out, "  MCP config — paste into {}:", outcome.name)?;
        writeln!(out)?;
        // Emit verbatim (already carries its own trailing newline).
        write!(out, "{snippet}")?;
    }
    // Phase 2 / Task 14: advisory notice for rules-only harnesses with
    // enabled agents that have no native form.
    if let Some(notice) = &outcome.unrepresented_agents_notice {
        writeln!(out)?;
        writeln!(out, "  Note: {notice}")?;
    }
    // US11: hook-translation advisory notice (for harnesses with hook support).
    if let Some(notice) = &outcome.hook_translation_notice {
        writeln!(out)?;
        writeln!(out, "  Hook translation: {notice}")?;
        // Issue #439: translated hooks FAIL OPEN by design, so a misfiring
        // hook is silent — point at the debugging tools while translation is
        // active. Human renderer only: the notice STRING rides `--json`
        // byte-for-byte, so the hint must not be folded into it.
        if let Some(hint) = hook_debug_hint(notice, &outcome.name) {
            writeln!(out, "  {hint}")?;
        }
    }
    // Issue #292: hooks advisory notice (for rules-only-for-hooks harnesses with
    // enabled plugin hooks that fall back to GUARDRAILS prose).
    if let Some(notice) = &outcome.hooks_notice {
        writeln!(out)?;
        writeln!(out, "  Note: {notice}")?;
    }
    Ok(())
}

/// The hook-debugging pointer rendered under the hook-translation notice
/// when translation is active (issue #439); `None` when it is off — there is
/// nothing to debug then. Gated on the notice's own "hook translation on"
/// prefix (the Tome-owned string built in `build_info`) rather than a new
/// outcome field, so the `--json` wire shape stays byte-identical: the hint
/// is a human-output affordance, not part of the notice.
fn hook_debug_hint(notice: &str, harness: &str) -> Option<String> {
    if !notice.starts_with("hook translation on") {
        return None;
    }
    Some(format!(
        "debug: TOME_HOOK_DEBUG=1 or `tome harness run-hook --explain --event <event> --harness {harness}`"
    ))
}

/// #327: render one `emit_human` section per outcome for the no-name
/// `tome harness info` form, separated by a blank line. An empty set prints a
/// helpful one-liner (never an error) — the effective list is empty when no
/// harness is configured for this scope.
fn emit_human_sections(outcomes: &[HarnessInfoOutcome]) -> Result<(), TomeError> {
    if outcomes.is_empty() {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "No harnesses configured for this scope; run `tome harness use <name>`."
        )?;
        return Ok(());
    }
    // Reuse the single-harness renderer per section. `emit_human` locks stdout
    // per call, so separate the sections with a blank line printed between (not
    // after) them for clean, greppable output. The separator goes through a
    // short-lived locked-handle `writeln!` that PROPAGATES `io::Error` — matching
    // every other write in this file — rather than a bare `println!`, which
    // panics on a broken pipe (`tome harness info | head`) and, under
    // `panic = "abort"`, aborts.
    for (i, outcome) in outcomes.iter().enumerate() {
        if i > 0 {
            writeln!(std::io::stdout().lock())?;
        }
        emit_human(outcome)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome_with_snippet(snippet: Option<String>) -> HarnessInfoOutcome {
        HarnessInfoOutcome {
            name: "jetbrains-ai".to_string(),
            description: "JetBrains AI Assistant".to_string(),
            detected: true,
            detected_path: PathBuf::from("/h/.aiassistant"),
            rules_target: None,
            mcp_target: None,
            rules_block_present: None,
            mcp_entry_present: None,
            mcp_tome_owned: None,
            references: Vec::new(),
            mcp_snippet: snippet,
            unrepresented_agents_notice: None,
            hook_translation_notice: None,
            hooks_notice: None,
        }
    }

    fn outcome_with_notice(notice: Option<String>) -> HarnessInfoOutcome {
        HarnessInfoOutcome {
            name: "gemini".to_string(),
            description: "Gemini CLI".to_string(),
            detected: false,
            detected_path: PathBuf::from("/h/.gemini"),
            rules_target: None,
            mcp_target: None,
            rules_block_present: None,
            mcp_entry_present: None,
            mcp_tome_owned: None,
            references: Vec::new(),
            mcp_snippet: None,
            unrepresented_agents_notice: notice,
            hook_translation_notice: None,
            hooks_notice: None,
        }
    }

    /// T063: `mcp_snippet` serialises LAST + is `skip_serializing_if`-gated so
    /// the byte-stable pre-Phase-11 `--json` pins don't move.
    #[test]
    fn mcp_snippet_is_appended_last_and_gated() {
        let with = serde_json::to_string(&outcome_with_snippet(Some("SNIP".to_string()))).unwrap();
        // When notice is None, mcp_snippet is last.
        assert!(
            with.ends_with("\"mcp_snippet\":\"SNIP\"}"),
            "mcp_snippet must be the LAST field when notice absent; got: {with}",
        );

        // Absent → the key is omitted entirely (skip_serializing_if).
        let without = serde_json::to_string(&outcome_with_snippet(None)).unwrap();
        assert!(
            !without.contains("mcp_snippet"),
            "absent snippet must omit the key; got: {without}",
        );
        // The prior-last field (`references`) remains last when both snippet
        // and notice are absent.
        assert!(
            without.ends_with("\"references\":[]}"),
            "references must be last when both optional fields absent; got: {without}",
        );
    }

    /// Task 14: `unrepresented_agents_notice` is `Some` only for a rules-only
    /// harness with ≥1 enabled agents; absent (and key omitted) otherwise.
    ///
    /// The notice field is appended after `mcp_snippet` in the struct; when
    /// both are present it appears last. When only the notice is set (mcp_snippet
    /// absent), the notice is the last key.
    #[test]
    fn unrepresented_notice_present_only_for_rules_only_harness_with_agents() {
        // A rules-only harness (Gemini) with a notice → Some, and it is the
        // last key in the JSON.
        let notice_text = "3 enabled agents have no native agent form on this harness; \
                           enable `expose_agents_as_personas` to surface them as MCP prompts.";
        let with_notice =
            serde_json::to_string(&outcome_with_notice(Some(notice_text.to_string()))).unwrap();
        assert!(
            with_notice.contains("unrepresented_agents_notice"),
            "notice key must be present when Some; got: {with_notice}",
        );
        assert!(
            with_notice.ends_with(&format!(
                "\"unrepresented_agents_notice\":\"{notice_text}\"}}"
            )),
            "notice must be the last key; got: {with_notice}",
        );

        // Native-supporting harness or zero agents → None → key omitted.
        let without_notice = serde_json::to_string(&outcome_with_notice(None)).unwrap();
        assert!(
            !without_notice.contains("unrepresented_agents_notice"),
            "absent notice must omit the key; got: {without_notice}",
        );
        // When both mcp_snippet and notice are absent, references is last.
        assert!(
            without_notice.ends_with("\"references\":[]}"),
            "references must be last when both optional fields absent; got: {without_notice}",
        );

        // Both present: notice is after mcp_snippet (last field).
        let both = HarnessInfoOutcome {
            mcp_snippet: Some("SNIP".to_string()),
            unrepresented_agents_notice: Some(notice_text.to_string()),
            ..outcome_with_notice(None)
        };
        let both_json = serde_json::to_string(&both).unwrap();
        // mcp_snippet appears before notice in the serialized output.
        let snippet_pos = both_json.find("mcp_snippet").unwrap();
        let notice_pos = both_json.find("unrepresented_agents_notice").unwrap();
        assert!(
            snippet_pos < notice_pos,
            "mcp_snippet must precede unrepresented_agents_notice; got: {both_json}",
        );
        assert!(
            both_json.ends_with(&format!(
                "\"unrepresented_agents_notice\":\"{notice_text}\"}}"
            )),
            "notice must be the last key when both present; got: {both_json}",
        );
    }

    /// Task 14 (Change 2): singular grammar is correct for count == 1.
    ///
    /// "1 enabled agent has ... surface it as an MCP prompt." (singular verb +
    /// pronoun). Plural form is already covered by
    /// `unrepresented_notice_present_only_for_rules_only_harness_with_agents`.
    #[test]
    fn notice_singular_grammar_for_count_one() {
        let singular_notice = "1 enabled agent has no native agent form on this harness; \
                               enable `expose_agents_as_personas` to surface it as an MCP prompt.";
        let outcome = outcome_with_notice(Some(singular_notice.to_string()));
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(
            json.contains("\"1 enabled agent has"),
            "singular notice must use 'has'; got: {json}",
        );
        assert!(
            json.contains("surface it as an MCP prompt"),
            "singular notice must use 'it' (not 'them'); got: {json}",
        );
        // Verify it does NOT accidentally use 'have' or 'them'.
        assert!(
            !json.contains("agents have"),
            "singular notice must not contain 'agents have'; got: {json}",
        );
        assert!(
            !json.contains("surface them"),
            "singular notice must not contain 'them'; got: {json}",
        );
    }

    /// Issue #439: the hook-debug hint renders only while translation is
    /// active, names the harness for the `run-hook` invocation, and lives
    /// OUTSIDE the notice string — so the `--json` wire shape (which carries
    /// the notice verbatim) stays byte-identical.
    #[test]
    fn hook_debug_hint_gates_on_active_translation() {
        let on = "hook translation on; 2 event(s) registered";
        let hint = hook_debug_hint(on, "devin").expect("active translation must yield the hint");
        assert!(hint.contains("TOME_HOOK_DEBUG=1"), "hint names the env var");
        assert!(
            hint.contains("tome harness run-hook --explain --event <event> --harness devin"),
            "hint names the dry-run command with the harness filled in; got: {hint}",
        );

        assert!(
            hook_debug_hint("hook translation off; 0 event(s) registered", "devin").is_none(),
            "translation off → nothing to debug → no hint",
        );

        // The JSON envelope carries the notice UNCHANGED — the hint is not
        // folded into it.
        let mut outcome = outcome_with_notice(None);
        outcome.hook_translation_notice = Some(on.to_string());
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(
            json.contains(
                "\"hook_translation_notice\":\"hook translation on; 2 event(s) registered\""
            ),
            "notice string must ride --json verbatim; got: {json}",
        );
        assert!(
            !json.contains("TOME_HOOK_DEBUG"),
            "the debug hint must never reach the JSON envelope; got: {json}",
        );
    }
}
