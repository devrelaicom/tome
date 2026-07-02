//! `tome plugin show <catalog>/<plugin>`.
//!
//! Renders one plugin's metadata + component breakdown + index state.
//!
//! Phase 5 / US5.b extends the output (both human + JSON) per
//! `contracts/catalog-and-plugin-extensions-p5.md` § `tome plugin show`
//! to group entries into Skills / Commands sections, each annotated
//! with the resolved `searchable=` / `user_invocable=` flags + the
//! derived `prompt_name` (when the entry is user-invocable). Entries
//! whose flags resolve to BOTH `searchable=false` AND
//! `user_invocable=false` are annotated `[dormant]`.

use std::io::Write;
use std::str::FromStr;

use comfy_table::{Cell, CellAlignment};
use serde::Serialize;

use crate::cli::PluginShowArgs;
use crate::error::TomeError;
use crate::mcp::prompt_name::derive_name;
use crate::output::Mode;
use crate::paths::Paths;
use crate::plugin::components::count_components;
use crate::plugin::frontmatter::parse_skill_frontmatter;
use crate::plugin::identity::EntryKind;
use crate::plugin::manifest::read_plugin_manifest;
use crate::plugin::{PluginId, PluginRecord, PluginStatus};
use crate::presentation::{colour, tables};
use crate::workspace::ResolvedScope;

use super::{aggregate_for_plugin, human_relative, open_index_for_read, resolve_plugin_dir};

pub fn run(args: PluginShowArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let id = PluginId::from_str(&args.id)
        .map_err(|e| TomeError::Usage(format!("invalid plugin id `{}`: {e}", args.id)))?;

    let paths = Paths::resolve()?;

    // Catalog enrolment lives in the DB (F11b), so the read-only handle is
    // opened before resolving the plugin directory; it is reused below for the
    // index aggregate + per-entry listing.
    let conn = open_index_for_read(&paths, &scope.scope)?;
    let plugin_dir = resolve_plugin_dir(&id, &conn, scope.scope.name().as_str(), &paths)?;

    // Strict failure here: exit 80 on an unconverted plugin, 22 on a malformed
    // `tome-plugin.toml` (the cutover reader).
    let manifest = read_plugin_manifest(&plugin_dir)?;
    let component_counts = count_components(&plugin_dir);

    let agg = aggregate_for_plugin(&conn, scope.scope.name().as_str(), &id.catalog, &id.plugin)?;

    let status = if agg.total == 0 {
        PluginStatus::Disabled
    } else if agg.enabled > 0 {
        PluginStatus::Enabled
    } else {
        PluginStatus::Disabled
    };

    let last_indexed_at_dt = agg.last_indexed_at.as_deref().and_then(|s| {
        use time::OffsetDateTime;
        use time::format_description::well_known::Rfc3339;
        OffsetDateTime::parse(s, &Rfc3339).ok()
    });

    // Phase 5 / US5.b: enumerate entries from the DB for the resolved
    // workspace and split by `kind`. Each entry is hydrated with its
    // frontmatter (for `argument_hint` + `prompt_name` override) so the
    // human + JSON outputs match the MCP `prompts/list` view.
    //
    // Frontmatter reads happen against `plugin_dir` (the catalog's
    // on-disk plugin tree resolved via `lifecycle::resolve_plugin_dir`)
    // rather than `resolve_entry_body_path` (which resolves via
    // `paths.cache_dir_for(url)`). The two diverge in test fixtures
    // where the catalog source lives at a custom path; in production
    // they are equal because the catalog cache IS the plugin source.
    let entries = list_entries(
        &conn,
        &plugin_dir,
        scope.scope.name().as_str(),
        &id.catalog,
        &id.plugin,
    )?;
    let mut skills: Vec<EntryView> = Vec::new();
    let mut commands: Vec<EntryView> = Vec::new();
    let mut agents: Vec<EntryView> = Vec::new();
    for e in entries {
        match e.kind {
            EntryKind::Skill => skills.push(e),
            EntryKind::Command => commands.push(e),
            EntryKind::Agent => agents.push(e),
        }
    }

    // #309: the upstream committer date for this plugin's subtree, computed at
    // display time from the catalog clone (best-effort; None when the clone has
    // no history for it / isn't a git repo). Routes through the SAME
    // `.git`-guarded helper `list` uses — resolving the `(clone_dir, source)`
    // split from the identity so `git log` never walks up to an unrelated
    // ancestor repository (which running it from the joined `plugin_dir` would
    // allow).
    let last_upstream_change =
        super::last_upstream_change_for_id(&conn, &paths, scope.scope.name().as_str(), &id);

    let record = PluginRecord {
        id: id.clone(),
        version: manifest.version.clone(),
        author: manifest.author.as_ref().and_then(|a| a.display()),
        description: manifest.description.clone(),
        last_upstream_change,
        status,
        component_counts,
        last_indexed_at: last_indexed_at_dt,
    };

    // Phase 6 / US5 (FR-083): hooks/guardrails presence booleans (read-only
    // fs existence checks against the on-disk plugin tree) + per-agent
    // resolved persona name when `expose_agents_as_personas` resolves true.
    let ships_hooks_json = plugin_dir.join("hooks").join("hooks.json").is_file();
    let ships_guardrails_md = plugin_dir.join("hooks").join("GUARDRAILS.md").is_file();

    // Persona names mirror the US4 derivation: clash-prefixed
    // `<plugin>-<name>` base for an agent name held by ≥2 enabled plugins,
    // `<name>` otherwise, then the `-persona` suffix. Computed only when the
    // flag resolves true at the scope; never invokes substitution.
    let expose_personas = crate::mcp::resolve_expose_personas(scope, &paths).unwrap_or(false);
    if expose_personas && !agents.is_empty() {
        let clash_set =
            crate::index::skills::agent_name_clash_set(&conn, scope.scope.name().as_str())
                .unwrap_or_default();
        for a in &mut agents {
            let clash_prefixed = clash_set.contains(&a.name);
            let base = if clash_prefixed {
                format!("{}-{}", id.plugin, a.name)
            } else {
                a.name.clone()
            };
            a.persona_name = Some(crate::mcp::prompt_name::derive_suffixed_name(
                &base, "persona",
            ));
        }
    }

    match mode {
        Mode::Human => emit_human(
            &record,
            &agg,
            &skills,
            &commands,
            &agents,
            ships_hooks_json,
            ships_guardrails_md,
        ),
        Mode::Json => emit_json(
            &record,
            &skills,
            &commands,
            &agents,
            ships_hooks_json,
            ships_guardrails_md,
        ),
    }
}

/// One per-entry projection emitted by [`list_entries`] — carries all
/// the fields the human + JSON renderers need without re-reading
/// frontmatter twice.
#[derive(Debug, Clone, Serialize)]
struct EntryView {
    name: String,
    description: String,
    when_to_use: Option<String>,
    #[serde(skip)]
    kind: EntryKind,
    searchable: bool,
    user_invocable: bool,
    /// Sanitised + truncated prompt name (no harness prefix). `None` for
    /// non-user-invocable entries — those never appear in
    /// `prompts/list`.
    prompt_name: Option<String>,
    /// Declared argument names. Empty when the entry takes catch-all
    /// `args` or no arguments.
    arguments: Vec<String>,
    /// Frontmatter `argument-hint` text, surfaced verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    argument_hint: Option<String>,
    /// Phase 6 / US5 (FR-083): the resolved `<name>-persona` slug for an
    /// agent entry, populated only when `expose_agents_as_personas`
    /// resolves true at the scope. `None` for non-agent entries and when
    /// personas are off — absent from the JSON wire in that case.
    #[serde(skip_serializing_if = "Option::is_none")]
    persona_name: Option<String>,
}

/// Pull every enabled-or-indexed entry for `(catalog, plugin)` from
/// the index and hydrate per-entry frontmatter from disk. The
/// frontmatter parse is the same lenient parser used by the enable
/// pipeline; unparsable entries (or entries whose source file is no
/// longer on disk) surface with the DB-stored defaults (empty
/// `arguments`, no override).
///
/// Frontmatter paths are resolved relative to `plugin_dir` using each
/// row's stored `skills.path` (a plugin-relative path produced by the
/// enable pipeline). Stored paths are already-sanitised through
/// `Path::join`; defensively skip absolute or traversal-bearing stored
/// paths via the same boundary check as `resolve_entry_body_path`.
fn list_entries(
    conn: &rusqlite::Connection,
    plugin_dir: &std::path::Path,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<Vec<EntryView>, TomeError> {
    use std::path::PathBuf;
    let records = crate::index::skills::list_for_plugin(conn, workspace_name, catalog, plugin)?;
    let mut out: Vec<EntryView> = Vec::with_capacity(records.len());
    for r in records {
        let stored = PathBuf::from(&r.path);
        // Polish M-4: SSOT S-H1 boundary check (US1.d BLOCKER). Future
        // additions to the safety predicate land in one place; this
        // site no longer maintains a parallel inline copy.
        let absolute = crate::index::skills::validate_db_stored_path(&stored)
            .ok()
            .map(|_| plugin_dir.join(&stored));
        let (arguments, argument_hint, prompt_name_override) =
            match absolute.as_deref().map(parse_skill_frontmatter) {
                Some(Ok(parsed)) => (
                    // Names only — `plugin show` surfaces the declared
                    // argument names; per-argument descriptions (issue #312)
                    // are a `prompts/list` concern and keep this wire pin
                    // byte-stable.
                    parsed.frontmatter.argument_names(),
                    parsed.frontmatter.argument_hint.clone(),
                    parsed.frontmatter.prompt_name.clone(),
                ),
                _ => (Vec::new(), None, None),
            };

        let prompt_name = if r.user_invocable {
            Some(derive_name(
                &r.plugin,
                &r.name,
                prompt_name_override.as_deref(),
            ))
        } else {
            None
        };
        // R-M4 (US5.c): mirror US4.d's MAX_DESCRIPTION_MAX_CHARS cap
        // (100 KiB) as a soft warning surface for `plugin show`. The
        // contract trust model is "catalog trusted-on-enrol, not
        // trusted-on-read" so we don't truncate the description here
        // (operators inspecting a plugin want the full text), but a
        // catalog shipping multi-megabyte descriptions is misbehaving
        // and operators should know.
        if r.description.len()
            > crate::mcp::tools::search_skills::MAX_DESCRIPTION_MAX_CHARS as usize
        {
            tracing::warn!(
                catalog = %r.catalog,
                plugin = %r.plugin,
                entry = %r.name,
                description_len = r.description.len(),
                cap = crate::mcp::tools::search_skills::MAX_DESCRIPTION_MAX_CHARS,
                "plugin show: entry description exceeds the soft size cap; \
                 likely a misbehaving catalog",
            );
        }
        out.push(EntryView {
            name: r.name,
            description: r.description,
            when_to_use: r.when_to_use,
            kind: r.kind,
            searchable: r.searchable,
            user_invocable: r.user_invocable,
            prompt_name,
            arguments,
            argument_hint,
            // Filled in by the caller when personas resolve true (FR-083).
            persona_name: None,
        });
    }
    Ok(out)
}

fn emit_human(
    record: &PluginRecord,
    agg: &super::IndexAggregate,
    skills: &[EntryView],
    commands: &[EntryView],
    agents: &[EntryView],
    ships_hooks_json: bool,
    ships_guardrails_md: bool,
) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "Plugin:       {}", record.id)?;
    writeln!(
        out,
        "Version:      {}",
        if record.version.is_empty() {
            "—".to_owned()
        } else {
            record.version.clone()
        }
    )?;

    let status_line = match record.status {
        PluginStatus::Enabled => {
            let when = agg
                .last_indexed_at
                .as_deref()
                .map(human_relative)
                .unwrap_or_else(|| "—".to_owned());
            format!("{} enabled (last indexed {})", colour::success("✓"), when)
        }
        PluginStatus::Disabled => format!("{} disabled", colour::error("✗")),
        PluginStatus::Unindexable => format!("{} unindexable", colour::warning("⚠")),
    };
    writeln!(out, "Status:       {}", status_line)?;

    // #309: honest, sourced timestamp lines (were both a bare `—` placeholder).
    //   * "Last indexed" is sourced from `indexed_at` (when Tome last built the
    //     index for this plugin), rendered as a human relative duration.
    //   * "Last upstream change" is the plugin subtree's most recent commit in
    //     the catalog clone (best-effort git-log; `—` when unavailable).
    let last_indexed = agg
        .last_indexed_at
        .as_deref()
        .map(human_relative)
        .unwrap_or_else(|| "—".to_owned());
    let last_upstream = record
        .last_upstream_change
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .map(|s| human_relative(&s))
        .unwrap_or_else(|| "—".to_owned());
    let author = record.author.clone().unwrap_or_else(|| "—".to_owned());
    writeln!(out, "Last indexed:         {} — {}", last_indexed, author)?;
    writeln!(out, "Last upstream change: {}", last_upstream)?;

    if let Some(desc) = &record.description {
        writeln!(out, "Description:  {}", desc)?;
    }

    writeln!(out)?;
    writeln!(out, "Component breakdown:")?;

    let mut table = tables::new_table();
    table.set_header(vec![
        Cell::new("Component"),
        Cell::new("Count").set_alignment(CellAlignment::Right),
    ]);
    let counts = &record.component_counts;
    let rows = [
        ("Skills", counts.skills),
        ("Agents", counts.agents),
        ("Commands", counts.commands),
        ("Hooks", counts.hooks),
        ("MCP servers", counts.mcp_servers),
    ];
    for (label, n) in rows {
        table.add_row(vec![
            Cell::new(label),
            Cell::new(n.to_string()).set_alignment(CellAlignment::Right),
        ]);
    }
    writeln!(out, "{table}")?;

    // Phase 6 / US5 (FR-083): hooks / guardrails ship-presence booleans.
    writeln!(out)?;
    writeln!(
        out,
        "Ships hooks/hooks.json:    {}",
        if ships_hooks_json { "yes" } else { "no" }
    )?;
    writeln!(
        out,
        "Ships hooks/GUARDRAILS.md: {}",
        if ships_guardrails_md { "yes" } else { "no" }
    )?;

    // Phase 5 / US5.b: per-section listing.
    if !skills.is_empty() {
        writeln!(out)?;
        writeln!(out, "Skills ({}):", skills.len())?;
        for e in skills {
            write_entry_line(&mut out, e)?;
        }
    }
    if !commands.is_empty() {
        writeln!(out)?;
        writeln!(out, "Commands ({}):", commands.len())?;
        for e in commands {
            write_entry_line(&mut out, e)?;
        }
    }
    if !agents.is_empty() {
        writeln!(out)?;
        writeln!(out, "Agents ({}):", agents.len())?;
        for e in agents {
            write_entry_line(&mut out, e)?;
        }
    }

    Ok(())
}

/// Render one entry line per the contract's "Human-mode output". The
/// fixed-width name + flag suffix matches the contract example:
///
/// ```text
///   compact-circuits     searchable=true  user_invocable=false
///     description: ...
///   fix-issue            searchable=true  user_invocable=true   prompt=midnight_expert__fix_issue
///     description: ...
///     arguments: [...]
/// ```
fn write_entry_line<W: Write>(out: &mut W, e: &EntryView) -> std::io::Result<()> {
    let dormant = !e.searchable && !e.user_invocable;
    let prompt_suffix = e
        .prompt_name
        .as_deref()
        .map(|p| format!("   prompt={p}"))
        .unwrap_or_default();
    let dormant_suffix = if dormant { "  [dormant]" } else { "" };
    writeln!(
        out,
        "  {:20} searchable={}  user_invocable={}{prompt_suffix}{dormant_suffix}",
        e.name, e.searchable, e.user_invocable,
    )?;
    if !e.description.is_empty() {
        writeln!(out, "    description: {}", e.description)?;
    }
    // Phase 6 / US5 (FR-083): resolved persona name for an agent entry,
    // present only when `expose_agents_as_personas` resolves true.
    if let Some(persona) = &e.persona_name {
        writeln!(out, "    persona: {persona}")?;
    }
    // Argument disclosure differs by entry kind:
    //   * Named arguments declared → list them.
    //   * No declared args but the entry surface allows catch-all `args`
    //     → render the documented "(none — accepts free-form 'args')"
    //     string when the entry is user-invocable (otherwise hide; a
    //     skill that doesn't accept args has nothing to show).
    if !e.arguments.is_empty() {
        writeln!(out, "    arguments: {:?}", e.arguments)?;
    } else if e.user_invocable {
        writeln!(out, "    arguments: (none — accepts free-form 'args')")?;
    }
    Ok(())
}

/// JSON-mode emission per the contract's "JSON-mode output" — the
/// `skills` + `commands` arrays carry the full per-entry projection
/// including the derived prompt_name.
fn emit_json(
    record: &PluginRecord,
    skills: &[EntryView],
    commands: &[EntryView],
    agents: &[EntryView],
    ships_hooks_json: bool,
    ships_guardrails_md: bool,
) -> Result<(), TomeError> {
    // Phase 6 / US5 (FR-083): the envelope gains two ship-presence booleans;
    // the per-agent `persona_name` rides on each `EntryView` (skip-if-none,
    // so the wire shape is unchanged when personas are off).
    #[derive(Serialize)]
    struct Envelope<'a> {
        #[serde(flatten)]
        record: &'a PluginRecord,
        skills: &'a [EntryView],
        commands: &'a [EntryView],
        agents: &'a [EntryView],
        ships_hooks_json: bool,
        ships_guardrails_md: bool,
    }
    let env = Envelope {
        record,
        skills,
        commands,
        agents,
        ships_hooks_json,
        ships_guardrails_md,
    };
    crate::output::write_json(&env)
}
