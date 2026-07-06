//! Per-harness fidelity preview (issue #288) — the read-only "what would
//! `harness sync` produce for harness X" report.
//!
//! A cross-harness plugin author has no way to see what their plugin looks like
//! in a specific target harness before shipping. `convert` reports only
//! importer-side drops; nothing reports what `harness sync` will drop for a
//! given harness (native vs persona agents, dropped agent model/tools, whether
//! plugin hooks reach the harness at all, native vs MCP-routed skills). This
//! module computes exactly that, per enabled entry, in a report-only pass.
//!
//! ## Accuracy via REUSE (the whole point)
//!
//! The preview MUST match what `sync` actually produces. It therefore routes
//! every verdict through the SAME translation SSOTs the reconcilers use — it
//! never reimplements or approximates:
//!
//! * Agents — [`crate::harness::reconcile::agents::prepare_agent`] parses each
//!   enabled agent into a [`CanonicalAgent`] (identical to the agents sink),
//!   then the harness module's own [`HarnessModule::translate_agent`] produces
//!   the [`TranslatedAgent`] whose `dropped_fields` IS the authoritative
//!   per-field (model / tools) drop list. Whether a harness gets native agents
//!   vs personas vs nothing is decided by the SAME
//!   [`HarnessModule::supports_native_agents`] /
//!   [`HarnessModule::is_opt_in_target`] predicates the info notice + doctor
//!   drop-report use.
//! * Hooks — [`crate::harness::reconcile::hooks::resolve_enabled_canonical_hooks`]
//!   enumerates every enabled plugin's translatable hooks into the
//!   [`CanonicalHook`] IR (parse → rewrite → `parse_canonical_hooks`), the same
//!   function the dispatch reconciler feeds its manifest from. Whether they
//!   reach the harness natively is decided by [`HarnessModule::hook_support`]
//!   (+ its `events` set) — the very thing sync's used-event computation reads.
//!   The prose fallback is decided by the same [`read_guardrails_source`] the
//!   guardrails sink calls.
//! * Skills / commands — plugin skills + commands are NEVER written as native
//!   files by `harness sync`; they always reach the harness through the Tome
//!   MCP server + the session-start routing directive
//!   ([`crate::index::skills::tiered_entries_for_workspace`] →
//!   `routing::build_directive`). The preview reports them as MCP-routed
//!   accordingly (commands via their MCP prompt, skills via `get_skill`),
//!   which is exactly how sync delivers them.
//! * Rules directive + MCP registration target — the same `rules_file_target` /
//!   `mcp_dialect` / `mcp_manual_only` the `harness info` command surfaces.
//!
//! ## Scope of the "matches sync" claim
//!
//! The preview reports each entry's DELIVERY ROUTING (native / persona /
//! unrepresented / MCP-routed / native-hook / GUARDRAILS) plus the agent
//! `model` / `tools` drops (from `translate_agent`'s `dropped_fields`). It does
//! NOT render the translated agent body, and it does NOT surface the Claude-Code
//! privileged passthrough fields (`hooks` / `mcp_servers` / `permission_mode`).
//! Those fields are the only thing the `strip_plugin_agent_privileges` setting
//! affects at sync time (a Claude-Code-only emission-clone that clears them),
//! and since the preview never reports them, its output is unaffected by that
//! setting: the preview translates the un-stripped `CanonicalAgent`, but the
//! divergence is invisible because the stripped fields are not part of the
//! preview's surface. `dropped_fields` (model / tools) is independent of the
//! strip and stays exact. In short: the preview matches sync for routing +
//! model/tools drops, and deliberately does not model the privileged-passthrough
//! strip.
//!
//! ## Read-only
//!
//! No writes, no harness files touched. The DB is opened read-only; the harness
//! is resolved via [`with_effective_modules`] + [`lookup`] (override- and
//! alias-aware, exactly like `harness info`). Sync-only — no async.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::TomeError;
use crate::harness::agents::{CanonicalAgent, TranslatedAgent};
use crate::harness::hooks_ir::CanonicalHook;
use crate::harness::sync::SyncDeps;
use crate::harness::{HarnessModule, lookup, with_effective_modules};
use crate::index::skills::{EnabledAgent, TieredEntry};
use crate::paths::Paths;
use crate::plugin::identity::EntryKind;
use crate::workspace::ResolvedScope;

/// The verdict for one native-agent-capable harness's agent, or the
/// persona/unrepresented outcome for a rules-only harness.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "delivery")]
pub enum AgentDelivery {
    /// Translated into a native agent file under the harness's `agent_dir`.
    /// `dropped_fields` is the harness's own `translate_agent` drop list
    /// (`model` when the model has no same-vendor target, `tools` when the
    /// harness drops per-agent tool posture, …).
    Native {
        /// The on-disk filename Tome would write (`<plugin>__<name>.<ext>`).
        filename: String,
        /// The displayed / registered name (clash-prefixed when it collides).
        displayed_name: String,
        /// Frontmatter fields dropped during translation (verbatim from
        /// `TranslatedAgent::dropped_fields`).
        dropped_fields: Vec<String>,
    },
    /// The harness has no native agent form but personas are enabled
    /// (`expose_agents_as_personas`): the agent is surfaced as an MCP prompt.
    Persona,
    /// The harness has no native agent form and personas are off: the agent is
    /// unrepresented on this harness (nothing is emitted for it).
    Unrepresented,
}

/// One enabled agent's preview verdict.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentPreview {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    #[serde(flatten)]
    pub delivery: AgentDelivery,
    /// A per-agent parse failure (post-enable source corruption): the agent
    /// enabled cleanly but its source could not be parsed now. Recorded so the
    /// preview is honest rather than silently omitting the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How a skill / command reaches the harness. Plugin skills + commands are
/// always MCP-routed (never native files at `harness sync` time).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryDelivery {
    /// A command — invoked via its MCP prompt.
    McpPrompt,
    /// A skill — loaded via the `get_skill` MCP tool.
    McpGetSkill,
}

/// One enabled skill / command's preview verdict.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EntryPreview {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// `"skill"` or `"command"`.
    pub kind: String,
    pub delivery: EntryDelivery,
}

/// How a plugin's hooks reach the harness. One verdict per enabled plugin that
/// ships `hooks/hooks.json` and/or `GUARDRAILS.md`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookPreview {
    pub catalog: String,
    pub plugin: String,
    /// Portable hook events (CC names) that reach this harness NATIVELY via the
    /// #318 translation (`hook_support().events` ∩ the plugin's used events).
    pub native_events: Vec<String>,
    /// Portable hook events the plugin declares that this harness does NOT
    /// translate natively (fall back to `GUARDRAILS.md`).
    pub guardrails_events: Vec<String>,
    /// `true` when the plugin ships a `GUARDRAILS.md` prose file (the prose
    /// fallback is rendered into the harness's guardrails region).
    pub has_guardrails_prose: bool,
}

/// The full per-harness preview report (issue #288). `--json` serialises this
/// verbatim; the human renderer groups it by entry kind.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewReport {
    /// The resolved (alias-expanded) harness name.
    pub harness: String,
    pub description: String,
    /// The workspace whose enabled entries were previewed.
    pub workspace: String,
    /// The single plugin scope, when `--plugin` was passed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_filter: Option<String>,
    /// `true` when this harness emits native translated agents.
    pub supports_native_agents: bool,
    /// `true` when personas are enabled (`expose_agents_as_personas`), which
    /// governs the fallback for a rules-only harness's agents.
    pub personas_enabled: bool,
    /// The rules-directive sink target for this harness (`None` when no project
    /// is resolved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules_target: Option<PathBuf>,
    /// The MCP registration target file (`None` when the harness is manual-only
    /// or no project is resolved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_target: Option<PathBuf>,
    /// `true` when the harness has NO writable MCP config file (jetbrains-ai):
    /// MCP is configured via a paste-in snippet (see `tome harness info`).
    pub mcp_manual_only: bool,
    /// `true` when this harness supports native plugin-hook translation (#318).
    pub supports_native_hooks: bool,
    pub agents: Vec<AgentPreview>,
    pub entries: Vec<EntryPreview>,
    pub hooks: Vec<HookPreview>,
    /// The first hook-enumeration error encountered (e.g. a malformed
    /// `hooks/hooks.json`). `resolve_enabled_canonical_hooks` records but does
    /// not propagate it (forward-progress, like sync); the preview surfaces it
    /// here so a malformed source is reported rather than silently omitted.
    /// Appended LAST + `skip_serializing_if`-gated so the byte-stable pins for
    /// the other fields don't move.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks_error: Option<String>,
}

/// A snapshot of the resolved harness module's report-relevant fields, captured
/// inside the registry read guard so downstream computation runs outside it.
struct ModuleSnapshot {
    name: String,
    description: String,
    supports_native_agents: bool,
    is_opt_in_target: bool,
    supports_native_hooks: bool,
    /// The portable events this harness translates natively (`hook_support().events`).
    native_hook_events: Vec<crate::harness::hooks_ir::PortableEvent>,
    mcp_manual_only: bool,
    rules_target: Option<PathBuf>,
    mcp_target: Option<PathBuf>,
}

/// Compute the per-harness preview (pure — no I/O beyond a read-only DB open).
///
/// `harness_name` is resolved through the alias + override-aware registry (like
/// `harness info`); an unknown name → [`TomeError::HarnessNotSupported`] (exit
/// 18). `plugin_filter` scopes the report to one plugin id (matched on the
/// `skills.plugin` column). An empty enabled set is not an error — the caller
/// renders a "nothing enabled" message.
pub fn pipeline(
    harness_name: &str,
    plugin_filter: Option<&str>,
    scope: &ResolvedScope,
    paths: &Paths,
    home: &std::path::Path,
) -> Result<PreviewReport, TomeError> {
    let project_root = scope.project_root.clone();

    // Snapshot the resolved module's fields — reuse the SAME override-aware
    // resolution `harness info` uses (effective registry first for a test
    // override + supported harnesses, then the alias/opt-in `lookup` fallback so
    // `generic` / `generic-op` / `antigravity-cli` resolve).
    let snapshot_of = |m: &dyn HarnessModule| {
        let hook_support = m.hook_support();
        ModuleSnapshot {
            name: m.name().to_string(),
            description: m.description().to_string(),
            supports_native_agents: m.supports_native_agents(),
            is_opt_in_target: m.is_opt_in_target(),
            supports_native_hooks: hook_support.is_some(),
            native_hook_events: hook_support
                .map(|hs| hs.events.to_vec())
                .unwrap_or_default(),
            mcp_manual_only: m.mcp_manual_only(),
            rules_target: project_root.as_deref().map(|p| m.rules_file_target(p)),
            mcp_target: project_root.as_deref().map(|p| m.mcp_config_path(p, home)),
        }
    };
    let snap = with_effective_modules(|mods| {
        mods.iter()
            .find(|m| m.name() == harness_name)
            .map(|m| snapshot_of(*m))
    })
    .or_else(|| lookup(harness_name).map(snapshot_of))
    .ok_or_else(|| TomeError::HarnessNotSupported {
        name: harness_name.to_string(),
    })?;

    let workspace = scope.scope.name();
    let cfg = crate::config::load_or_default(paths);
    let personas_enabled = cfg.harness.expose_agents_as_personas.unwrap_or(false);
    // Mirror the dispatch reconciler's prompt gate (default off): a prompt-only
    // hook is dropped to GUARDRAILS by sync unless a prompt provider/model is
    // configured. The preview must apply the same gate so it doesn't report a
    // prompt-only event as native under the default config.
    let prompt_enabled = cfg.hooks.prompt_provider.is_some() || cfg.hooks.prompt_model.is_some();

    // No DB → nothing enabled. Every enumeration returns empty (not an error).
    if !paths.index_db.is_file() {
        return Ok(build_report(
            &snap,
            workspace.as_str(),
            plugin_filter,
            personas_enabled,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
    }
    let conn = crate::index::open_read_only(&paths.index_db)?;
    let ws = workspace.as_str();

    // ----- Agents (reuse prepare_agent + translate_agent, the sync SSOTs) -----
    let clash_set = crate::index::skills::agent_name_clash_set(&conn, ws)?;
    let enabled_agents = crate::index::skills::enabled_agents_for_workspace(&conn, ws)?;
    let model_registry = crate::model_registry::ModelRegistry::load(paths);
    let agents = preview_agents(
        &conn,
        paths,
        ws,
        harness_name,
        plugin_filter,
        &snap,
        personas_enabled,
        &clash_set,
        &enabled_agents,
        &model_registry,
    );

    // ----- Skills / commands (always MCP-routed at sync time) -----
    let tiered = crate::index::skills::tiered_entries_for_workspace(&conn, ws)?;
    let entries = preview_entries(&tiered, plugin_filter);

    // ----- Hooks (reuse resolve_enabled_canonical_hooks, the dispatch SSOT) ---
    // Build the same `SyncDeps` shape the dispatch reconciler consumes so the
    // canonical hook enumeration is byte-identical.
    let deps = SyncDeps {
        paths,
        home_root: home,
        workspace_name: workspace,
        force: false,
        only_harness: None,
        dry_run: false,
    };
    // The read-only enumeration records a first parse error but never halts; a
    // hook whose source is malformed/unreadable is omitted from `canonical`
    // (matches sync's forward-progress). Unlike sync — which only records the
    // error internally — the preview SURFACES it as a report-level note so a
    // malformed `hooks/hooks.json` is honest, not silently dropped (consistent
    // with the agent path surfacing per-entry parse errors).
    let mut first_error: Option<TomeError> = None;
    let canonical =
        crate::harness::reconcile::hooks::resolve_enabled_canonical_hooks(&deps, &mut first_error)?;
    let hooks = preview_hooks(
        &conn,
        paths,
        ws,
        plugin_filter,
        &snap,
        &canonical,
        prompt_enabled,
    )?;
    let hooks_error = first_error.map(|e| e.to_string());

    let mut report = build_report(
        &snap,
        ws,
        plugin_filter,
        personas_enabled,
        agents,
        entries,
        hooks,
    );
    report.hooks_error = hooks_error;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    snap: &ModuleSnapshot,
    workspace: &str,
    plugin_filter: Option<&str>,
    personas_enabled: bool,
    agents: Vec<AgentPreview>,
    entries: Vec<EntryPreview>,
    hooks: Vec<HookPreview>,
) -> PreviewReport {
    // A manual-only harness (jetbrains-ai) has no writable MCP file → no target.
    let mcp_target = if snap.mcp_manual_only {
        None
    } else {
        snap.mcp_target.clone()
    };
    PreviewReport {
        harness: snap.name.clone(),
        description: snap.description.clone(),
        workspace: workspace.to_string(),
        plugin_filter: plugin_filter.map(str::to_string),
        supports_native_agents: snap.supports_native_agents,
        personas_enabled,
        rules_target: snap.rules_target.clone(),
        mcp_target,
        mcp_manual_only: snap.mcp_manual_only,
        supports_native_hooks: snap.supports_native_hooks,
        agents,
        entries,
        hooks,
        // Set by `pipeline` after `resolve_enabled_canonical_hooks` runs; the
        // no-DB early-return path has no hook enumeration, so `None` is correct.
        hooks_error: None,
    }
}

/// Preview every enabled agent. Native-supporting harnesses translate each via
/// the module's own `translate_agent` (the SSOT that owns `dropped_fields`);
/// rules-only harnesses report persona vs unrepresented.
#[allow(clippy::too_many_arguments)]
fn preview_agents(
    conn: &rusqlite::Connection,
    paths: &Paths,
    workspace: &str,
    harness_name: &str,
    plugin_filter: Option<&str>,
    snap: &ModuleSnapshot,
    personas_enabled: bool,
    clash_set: &BTreeSet<String>,
    enabled: &[EnabledAgent],
    model_registry: &crate::model_registry::ModelRegistry,
) -> Vec<AgentPreview> {
    let mut out = Vec::new();
    for row in enabled {
        if let Some(want) = plugin_filter
            && row.plugin != want
        {
            continue;
        }
        // Parse via the SAME helper the agents sink uses so preview + sync agree
        // on the CanonicalAgent input.
        let canonical =
            match crate::harness::reconcile::agents::prepare_agent(conn, paths, workspace, row) {
                Ok(c) => c,
                Err(e) => {
                    out.push(AgentPreview {
                        catalog: row.catalog.clone(),
                        plugin: row.plugin.clone(),
                        name: row.name.clone(),
                        // A parse failure is reported honestly. Even on a
                        // rules-only harness the delivery is the harness's
                        // rules-only fallback; the error annotates it.
                        delivery: rules_only_delivery(snap, personas_enabled),
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            };

        let delivery = if snap.supports_native_agents {
            match translate_for_preview(harness_name, &canonical, clash_set, model_registry) {
                Ok(t) => AgentDelivery::Native {
                    filename: t.filename,
                    displayed_name: t.displayed_name,
                    dropped_fields: t.dropped_fields,
                },
                Err(e) => {
                    out.push(AgentPreview {
                        catalog: canonical.catalog.clone(),
                        plugin: canonical.plugin.clone(),
                        name: canonical.name.clone(),
                        delivery: AgentDelivery::Unrepresented,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            }
        } else {
            rules_only_delivery(snap, personas_enabled)
        };

        out.push(AgentPreview {
            catalog: canonical.catalog,
            plugin: canonical.plugin,
            name: canonical.name,
            delivery,
            error: None,
        });
    }
    out
}

/// The delivery for an agent on a harness with no native agent form. An opt-in
/// target (generic/generic-op) never surfaces personas either (it has no MCP
/// prompt surface of its own), so it is Unrepresented; a real rules-only harness
/// surfaces personas iff `expose_agents_as_personas` is on. This mirrors the
/// `!supports_native_agents() && !is_opt_in_target()` gate the info notice +
/// doctor drop-report use.
fn rules_only_delivery(snap: &ModuleSnapshot, personas_enabled: bool) -> AgentDelivery {
    if !snap.is_opt_in_target && personas_enabled {
        AgentDelivery::Persona
    } else {
        AgentDelivery::Unrepresented
    }
}

/// Translate one canonical agent through the resolved harness module's OWN
/// `translate_agent`, under the registry guard (so a test override + the alias
/// resolution both apply). This is the exact call the agents sink makes.
fn translate_for_preview(
    harness_name: &str,
    canonical: &CanonicalAgent,
    clash_set: &BTreeSet<String>,
    model_registry: &crate::model_registry::ModelRegistry,
) -> Result<TranslatedAgent, TomeError> {
    let clashes = clash_set.contains(&canonical.name);
    let resolved = with_effective_modules(|mods| {
        mods.iter()
            .find(|m| m.name() == harness_name)
            .map(|m| m.translate_agent(canonical, clashes, model_registry))
    });
    match resolved {
        Some(r) => r,
        // Unreachable in practice: this fn is only called for a harness the
        // caller ALREADY resolved with `supports_native_agents == true`, and
        // every native-supporting harness lives in `SUPPORTED_HARNESSES` (so
        // `with_effective_modules` finds it). If this arm ever goes live, a
        // future refactor broke the "resolved-supporting" invariant — surface
        // it as an internal integrity failure (NOT a misleading exit-18
        // "harness not supported", which the caller already ruled out).
        None => {
            debug_assert!(
                false,
                "translate_for_preview: harness `{harness_name}` reported \
                 supports_native_agents but is absent from the effective registry",
            );
            Err(TomeError::IndexIntegrityCheckFailure(format!(
                "preview: native-agent harness `{harness_name}` not resolvable in the effective registry"
            )))
        }
    }
}

/// Preview enabled skills + commands: always MCP-routed at sync time (commands
/// via their MCP prompt, skills via `get_skill`). Reuses the SAME tiered-entry
/// query the routing directive builder consumes.
fn preview_entries(tiered: &[TieredEntry], plugin_filter: Option<&str>) -> Vec<EntryPreview> {
    tiered
        .iter()
        .filter(|e| plugin_filter.is_none_or(|want| e.plugin == want))
        .map(|e| EntryPreview {
            catalog: e.catalog.clone(),
            plugin: e.plugin.clone(),
            name: e.name.clone(),
            kind: e.kind.as_str().to_string(),
            delivery: match e.kind {
                EntryKind::Command => EntryDelivery::McpPrompt,
                // Skills (and any non-command kind that reaches here) load via
                // get_skill. Agents never appear in `tiered_entries_for_workspace`.
                _ => EntryDelivery::McpGetSkill,
            },
        })
        .collect()
}

/// Preview hooks per enabled plugin: which portable events reach the harness
/// natively (#318), which fall back to GUARDRAILS, and whether the plugin ships
/// `GUARDRAILS.md` prose. Reuses the canonical hook enumeration (`canonical`)
/// the dispatch reconciler consumes.
///
/// `prompt_enabled` mirrors the dispatch reconciler's per-harness/config gate
/// (`reconcile_one_harness_dispatch`: `cfg.hooks.prompt_provider.is_some() ||
/// cfg.hooks.prompt_model.is_some()`). `resolve_enabled_canonical_hooks`
/// deliberately does NOT apply this gate (it is config-dependent, applied
/// downstream), so the preview must apply it here: when prompts are disabled,
/// `Handler::Prompt` hooks are filtered out BEFORE computing each plugin's
/// declared-event set — exactly like sync builds `effective_canonical` → `used`.
/// A prompt-only event with prompts off therefore lands in `guardrails_events`,
/// matching what sync actually delivers under the default config.
fn preview_hooks(
    conn: &rusqlite::Connection,
    paths: &Paths,
    workspace: &str,
    plugin_filter: Option<&str>,
    snap: &ModuleSnapshot,
    canonical: &[CanonicalHook],
    prompt_enabled: bool,
) -> Result<Vec<HookPreview>, TomeError> {
    use std::collections::{BTreeMap, HashSet};

    use crate::harness::hooks_ir::{Handler, PortableEvent};

    // The set of portable events this harness translates natively. `PortableEvent`
    // is `Hash` but not `Ord`, so use a `HashSet` for membership and iterate the
    // fixed `PortableEvent::ALL` order for deterministic output.
    let native_set: HashSet<PortableEvent> = snap.native_hook_events.iter().copied().collect();

    // Per plugin, split its declared events into two sets, mirroring sync:
    //
    //   * `effective` — events with ≥1 hook that SURVIVES the prompt gate (sync's
    //     `effective_canonical`). These are candidates for native translation:
    //     `used = effective ∩ hook_support().events`; the rest fall back to
    //     GUARDRAILS (declared-but-unsupported).
    //   * `prompt_dropped` — events ALL of whose hooks were prompt-filtered (a
    //     prompt-only event with prompts disabled). Sync drops these to the
    //     GUARDRAILS floor, so they belong in `guardrails_events` — NOT omitted.
    //
    // Tracking both is what fixes the "preview says survives when sync drops"
    // failure: a prompt-only event with prompts off must appear as a GUARDRAILS
    // fallback, not vanish.
    struct PluginEvents {
        effective: HashSet<PortableEvent>,
        prompt_dropped: HashSet<PortableEvent>,
    }
    let mut by_plugin: BTreeMap<(String, String), PluginEvents> = BTreeMap::new();
    for h in canonical {
        if plugin_filter.is_some_and(|want| h.plugin != want) {
            continue;
        }
        let entry = by_plugin
            .entry((h.catalog.clone(), h.plugin.clone()))
            .or_insert_with(|| PluginEvents {
                effective: HashSet::new(),
                prompt_dropped: HashSet::new(),
            });
        if !prompt_enabled && matches!(h.handler, Handler::Prompt { .. }) {
            entry.prompt_dropped.insert(h.event);
        } else {
            entry.effective.insert(h.event);
        }
    }

    // Every enabled plugin (so a plugin that ships ONLY GUARDRAILS.md, with no
    // translatable hooks, still appears). Read-only.
    let enabled = crate::index::skills::enabled_plugins_for_workspace(conn, workspace)?;
    let mut out = Vec::new();
    for (catalog, plugin) in &enabled {
        if plugin_filter.is_some_and(|want| plugin != want) {
            continue;
        }
        let key = (catalog.clone(), plugin.clone());
        let declared = by_plugin.get(&key);

        // GUARDRAILS.md presence via the SAME reader the guardrails sink uses.
        let has_guardrails_prose =
            match crate::index::skills::plugin_root_dir(conn, paths, workspace, catalog, plugin) {
                Ok(root) => crate::harness::guardrails::read_guardrails_source(&root)
                    .map(|o| o.is_some())
                    .unwrap_or(false),
                // Catalog cache evicted / unreadable: treat as no prose (matches
                // the guardrails sink skipping it).
                Err(_) => false,
            };

        // A plugin's declared-event footprint = effective ∪ prompt_dropped.
        let declared_empty =
            declared.is_none_or(|d| d.effective.is_empty() && d.prompt_dropped.is_empty());
        // Skip plugins with no hooks AND no guardrails prose — they contribute
        // nothing to this sink.
        if declared_empty && !has_guardrails_prose {
            continue;
        }

        // Iterate the fixed PortableEvent::ALL order so native/guardrails lists
        // are deterministic (byte-stable) regardless of HashSet iteration order.
        let mut native_events = Vec::new();
        let mut guardrails_events = Vec::new();
        if let Some(d) = declared {
            for ev in PortableEvent::ALL {
                // A prompt-dropped-only event → GUARDRAILS floor (sync drops it).
                if d.effective.contains(&ev) {
                    if native_set.contains(&ev) {
                        native_events.push(ev.cc_name().to_string());
                    } else {
                        // Declared + effective but unsupported by this harness.
                        guardrails_events.push(ev.cc_name().to_string());
                    }
                } else if d.prompt_dropped.contains(&ev) {
                    // Every hook for this event was prompt-filtered → GUARDRAILS.
                    guardrails_events.push(ev.cc_name().to_string());
                }
            }
        }

        out.push(HookPreview {
            catalog: catalog.clone(),
            plugin: plugin.clone(),
            native_events,
            guardrails_events,
            has_guardrails_prose,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_snapshot() -> ModuleSnapshot {
        ModuleSnapshot {
            name: "opencode".into(),
            description: "OpenCode".into(),
            supports_native_agents: true,
            is_opt_in_target: false,
            supports_native_hooks: false,
            native_hook_events: Vec::new(),
            mcp_manual_only: false,
            rules_target: Some(PathBuf::from("/p/AGENTS.md")),
            mcp_target: Some(PathBuf::from("/p/opencode.json")),
        }
    }

    fn rules_only_snapshot() -> ModuleSnapshot {
        ModuleSnapshot {
            name: "cline".into(),
            description: "Cline".into(),
            supports_native_agents: false,
            is_opt_in_target: false,
            supports_native_hooks: false,
            native_hook_events: Vec::new(),
            mcp_manual_only: false,
            rules_target: Some(PathBuf::from("/p/.clinerules/tome.md")),
            mcp_target: Some(PathBuf::from("/p/.cline/mcp.json")),
        }
    }

    #[test]
    fn rules_only_delivery_respects_personas_and_opt_in() {
        let snap = rules_only_snapshot();
        // Personas on → Persona.
        assert_eq!(rules_only_delivery(&snap, true), AgentDelivery::Persona);
        // Personas off → Unrepresented.
        assert_eq!(
            rules_only_delivery(&snap, false),
            AgentDelivery::Unrepresented
        );

        // An opt-in target never surfaces personas, even with personas on.
        let mut opt_in = rules_only_snapshot();
        opt_in.name = "generic".into();
        opt_in.is_opt_in_target = true;
        assert_eq!(
            rules_only_delivery(&opt_in, true),
            AgentDelivery::Unrepresented
        );
    }

    #[test]
    fn build_report_manual_only_omits_mcp_target() {
        let mut snap = native_snapshot();
        snap.mcp_manual_only = true;
        snap.name = "jetbrains-ai".into();
        let report = build_report(&snap, "global", None, false, vec![], vec![], vec![]);
        assert!(report.mcp_manual_only);
        assert!(
            report.mcp_target.is_none(),
            "manual-only harness must not report an MCP target"
        );
    }

    #[test]
    fn build_report_carries_rules_and_mcp_targets_for_writable_harness() {
        let snap = native_snapshot();
        let report = build_report(&snap, "global", None, false, vec![], vec![], vec![]);
        assert_eq!(report.rules_target, Some(PathBuf::from("/p/AGENTS.md")));
        assert_eq!(report.mcp_target, Some(PathBuf::from("/p/opencode.json")));
        assert!(!report.mcp_manual_only);
        assert!(report.supports_native_agents);
    }

    #[test]
    fn preview_entries_routes_commands_to_prompt_and_skills_to_get_skill() {
        let tiered = vec![
            TieredEntry {
                catalog: "cat".into(),
                plugin: "p".into(),
                name: "do-thing".into(),
                kind: EntryKind::Command,
                description: "d".into(),
                when_to_use: None,
                tier: 1,
            },
            TieredEntry {
                catalog: "cat".into(),
                plugin: "p".into(),
                name: "some-skill".into(),
                kind: EntryKind::Skill,
                description: "d".into(),
                when_to_use: None,
                tier: 2,
            },
        ];
        let out = preview_entries(&tiered, None);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, "command");
        assert_eq!(out[0].delivery, EntryDelivery::McpPrompt);
        assert_eq!(out[1].kind, "skill");
        assert_eq!(out[1].delivery, EntryDelivery::McpGetSkill);
    }

    #[test]
    fn preview_entries_honours_plugin_filter() {
        let tiered = vec![
            TieredEntry {
                catalog: "cat".into(),
                plugin: "keep".into(),
                name: "a".into(),
                kind: EntryKind::Skill,
                description: "d".into(),
                when_to_use: None,
                tier: 1,
            },
            TieredEntry {
                catalog: "cat".into(),
                plugin: "drop".into(),
                name: "b".into(),
                kind: EntryKind::Skill,
                description: "d".into(),
                when_to_use: None,
                tier: 1,
            },
        ];
        let out = preview_entries(&tiered, Some("keep"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].plugin, "keep");
    }

    /// The `--json` shape: `delivery` is flattened onto the agent record via the
    /// internally-tagged enum, so a native agent serialises
    /// `{ …, "delivery":"native", "filename":…, "dropped_fields":[…] }`.
    #[test]
    fn agent_preview_json_flattens_delivery_tag() {
        let ap = AgentPreview {
            catalog: "cat".into(),
            plugin: "p".into(),
            name: "reviewer".into(),
            delivery: AgentDelivery::Native {
                filename: "p__reviewer.md".into(),
                displayed_name: "reviewer".into(),
                dropped_fields: vec!["model".into()],
            },
            error: None,
        };
        let json = serde_json::to_string(&ap).unwrap();
        assert!(json.contains("\"delivery\":\"native\""), "got: {json}");
        assert!(
            json.contains("\"filename\":\"p__reviewer.md\""),
            "got: {json}"
        );
        assert!(
            json.contains("\"dropped_fields\":[\"model\"]"),
            "got: {json}"
        );
        // No error key when None.
        assert!(
            !json.contains("\"error\""),
            "error omitted when None: {json}"
        );
    }

    #[test]
    fn agent_preview_persona_and_unrepresented_serialise_tag_only() {
        let persona = AgentPreview {
            catalog: "c".into(),
            plugin: "p".into(),
            name: "a".into(),
            delivery: AgentDelivery::Persona,
            error: None,
        };
        assert!(
            serde_json::to_string(&persona)
                .unwrap()
                .contains("\"delivery\":\"persona\"")
        );
        let unrep = AgentPreview {
            catalog: "c".into(),
            plugin: "p".into(),
            name: "a".into(),
            delivery: AgentDelivery::Unrepresented,
            error: None,
        };
        assert!(
            serde_json::to_string(&unrep)
                .unwrap()
                .contains("\"delivery\":\"unrepresented\"")
        );
    }
}
