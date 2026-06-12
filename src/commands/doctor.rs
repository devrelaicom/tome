//! `tome doctor [--fix] [--verify]` CLI surface. The heavy lifting
//! (`assemble_report` + per-subsystem checks + `--fix` repairs) lives
//! in `crate::doctor`; this module is the thin arg-validation + emit +
//! exit-code layer.

use std::io::Write;

use crate::cli::DoctorArgs;
use crate::doctor::{self, DoctorClassification, DoctorReport};
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::colour;
use crate::workspace::ResolvedScope;

pub fn run(args: DoctorArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    // `--force` without `--fix` is a user error: there is nothing for
    // `--force` to override during a read-only report pass. Surface as
    // exit 2 (Usage) rather than exit 7 (Io) — per US5 reviewer R-M1
    // this is a usage-shape failure, not an I/O failure, and the
    // closed-set error vocabulary already encodes it.
    if args.force && !args.fix {
        return Err(TomeError::Usage(
            "`--force` requires `--fix`; rerun as `tome doctor --fix --force`".to_owned(),
        ));
    }

    let paths = Paths::resolve()?;
    // Route HOME through the validated `home_root()` (rejects empty/non-absolute,
    // exit 2) — the SAME resolver every meta write path uses, so doctor's
    // harness-detection home and the installer's never diverge. `home_root()`
    // already errors when HOME is unset, replacing the bespoke check here.
    let home = crate::commands::harness::home_root()?;

    // Phase 8 cutover (US1 closeout MAJOR-1): migrate any legacy model
    // `manifest.json` → `manifest.toml` BEFORE classification, so a pre-cutover
    // install (model bytes present, only the legacy JSON) is seen as healthy
    // and the model `--fix` never spuriously `remove_dir_all`s + re-downloads
    // it — honouring the contract's "No re-download" guarantee. Only under
    // `--fix` (read-only doctor must not mutate). A migration failure is
    // warned, not propagated (FR-561); the legacy manifest then simply stays
    // surfaced by `assemble_report` below.
    if args.fix {
        match doctor::cutover::migrate_model_manifests(&paths) {
            Ok(migrated) if !migrated.is_empty() => tracing::info!(
                count = migrated.len(),
                "doctor --fix: migrated legacy model manifest.json → manifest.toml",
            ),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "doctor --fix: model manifest migration failed");
            }
        }
    }

    let mut report = doctor::assemble_report(scope, &paths, &home, args.verify)?;

    if args.fix {
        let ctx = doctor::fixes::FixContext {
            paths: &paths,
            scope,
            home: &home,
            force: args.force,
        };
        let _attempts = doctor::fixes::apply(&mut report, &ctx);

        // Phase 9 / US4 (FR-032): repair meta-skill drift by re-running the
        // idempotent `meta::install_skill` for every stale / missing-but-
        // expected candidate (the SAME safe atomic symlink-checked path — NOT
        // a bespoke writer). Forward-progress: a per-location failure is
        // warned inside `repair` and the loop continues; the returned first
        // error is logged but never aborts the doctor pass (FR-561). The
        // report's `meta_skills` field is then re-projected (gated on "the
        // repair ran") so it reflects post-repair on-disk state.
        let meta_repair = doctor::meta_drift::repair(&home, scope);
        match &meta_repair {
            Ok(n) if *n > 0 => {
                tracing::info!(count = n, "doctor --fix: (re)installed meta skills");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "doctor --fix: meta-skill repair had failures");
            }
        }
        // `MetaAction::Fix` is doctor's meta `--fix` repair (FR-026). Forward-
        // progress: `repair` returns the first error after continuing past
        // per-location failures → all-ok = Ok, an Err carries at least one
        // failure (some may have still landed) = Partial. One infallible emit.
        crate::telemetry::enqueue(crate::telemetry::event::MetaActionEvent {
            action: crate::telemetry::event::MetaAction::Fix,
            outcome: if meta_repair.is_ok() {
                crate::telemetry::event::Outcome::Ok
            } else {
                crate::telemetry::event::Outcome::Partial
            },
        });
        report.meta_skills = doctor::meta_drift::check(&home, scope);

        // FR-410: sweep orphan `.tome.tmp.*` staging directories. Best-
        // effort — per-fix failures are warn'd inside `cleanup_stale_
        // staging_dirs`. We don't propagate the count to the report
        // (no contract field for it) but it surfaces in the trace log.
        let cleaned = doctor::orphan_cleanup::cleanup_stale_staging_dirs(&paths).unwrap_or(0);
        if cleaned > 0 {
            tracing::info!(
                count = cleaned,
                "doctor --fix: removed stale `.tome.tmp.*` staging directories",
            );
        }
        doctor::fixes::re_assemble(&mut report);
    }

    emit(&report, mode)?;

    // `tome.doctor_run`: emit AFTER the report renders but BEFORE any of the
    // exit paths below (one of which is a hard `std::process::exit(1)` that
    // would otherwise skip the emit). `findings_bucket` buckets the number of
    // suggested-fix issues the report surfaced.
    crate::telemetry::enqueue(crate::telemetry::event::DoctorRun {
        fix: args.fix,
        findings_bucket: crate::telemetry::buckets::FindingsBucket::from(
            report.suggested_fixes.len(),
        ),
    });

    // Exit-code semantics per `contracts/doctor.md`:
    // - Overall Ok → exit 0.
    // - Overall Degraded / Unhealthy → exit 1 (the report classifies).
    // - `--fix` ran but un-fixable issues remain → exit 75 instead
    //   of 1 (communicates "fix did something, but the work isn't
    //   done").
    let remaining_manual = doctor::fixes::has_remaining_manual_fixes(&report);
    let overall_ok = matches!(report.overall, DoctorClassification::Ok);
    if overall_ok {
        return Ok(());
    }
    if args.fix && remaining_manual {
        return Err(TomeError::DoctorFixNotSafe {
            subsystem: report
                .suggested_fixes
                .iter()
                .find(|f| !f.auto_fixable)
                .map(|f| f.subsystem.to_wire_string())
                .unwrap_or_else(|| "unknown".to_owned()),
        });
    }
    std::process::exit(1);
}

fn emit(report: &DoctorReport, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(report),
        Mode::Json => write_json(report),
    }
}

fn emit_human(report: &DoctorReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    let tty = crate::output::stdout_is_tty();

    // Glyphs match status's idiom: green tick on TTY, ASCII fallback
    // otherwise. Stays cheap to render in pipes.
    let (ok, warn, fail, info) = if tty {
        (
            colour::success("✓").to_string(),
            colour::warning("⚠").to_string(),
            colour::error("✗").to_string(),
            "·".to_owned(),
        )
    } else {
        (
            "[ok]".to_owned(),
            "[warn]".to_owned(),
            "[fail]".to_owned(),
            "[—]".to_owned(),
        )
    };
    let model_glyph = |state: &str| -> String {
        if state == "ok" {
            format!("{ok} ok")
        } else {
            format!("{fail} {state}")
        }
    };

    writeln!(out, "Tome:            {}", report.tome_version)?;
    writeln!(out)?;

    writeln!(
        out,
        "Workspace:       {}",
        match (report.workspace.scope, report.workspace.path.as_deref(),) {
            (crate::workspace::ScopeKind::Workspace, Some(p)) => p.display().to_string(),
            _ => "(global)".to_owned(),
        }
    )?;
    writeln!(
        out,
        "  resolved via:  {}",
        match report.workspace.source {
            crate::workspace::ScopeSource::Flag => "--workspace flag",
            crate::workspace::ScopeSource::Env => "TOME_WORKSPACE env",
            crate::workspace::ScopeSource::ProjectMarker => "project marker walk",
            crate::workspace::ScopeSource::GlobalFallback => "global fallback",
        }
    )?;
    writeln!(out, "  catalogs:      {}", report.workspace.catalogs)?;
    writeln!(
        out,
        "  plugins:       {} total, {} enabled",
        report.workspace.plugins_total, report.workspace.plugins_enabled,
    )?;
    writeln!(out)?;

    writeln!(out, "Models:")?;
    writeln!(
        out,
        "  embedder       {} ({})  {}",
        report.embedder.name,
        report.embedder.version,
        model_glyph(&report.embedder.state),
    )?;
    writeln!(
        out,
        "  reranker       {} ({})  {}",
        report.reranker.name,
        report.reranker.version,
        model_glyph(&report.reranker.state),
    )?;
    writeln!(out)?;

    if report.index.present {
        writeln!(
            out,
            "Index database:  {} ({} plugins enabled, {} skills indexed, {})",
            if report.index.integrity_ok {
                &ok
            } else {
                &fail
            },
            report.index.plugins_enabled,
            report.index.skills_indexed,
            human_size(report.index.size_bytes),
        )?;
        if let Some(v) = report.index.schema_version {
            writeln!(out, "Schema version:  {v}")?;
        }
    } else {
        writeln!(out, "Index database:  not yet bootstrapped")?;
    }
    writeln!(out, "Drift:           {}", drift_label(&report.drift))?;
    writeln!(out)?;

    writeln!(out, "Catalog caches:")?;
    if report.catalogs.is_empty() {
        writeln!(out, "  (none registered in this scope)")?;
    } else {
        for c in &report.catalogs {
            // Orphan clones render with the info glyph (not the failure
            // glyph) since they're informational per
            // `catalog-extensions-p3.md` §"Doctor reporting".
            let glyph = match c.state {
                doctor::CatalogCacheState::Ok => ok.clone(),
                doctor::CatalogCacheState::Orphan => info.clone(),
                _ => fail.clone(),
            };
            let suffix = match c.state {
                doctor::CatalogCacheState::Ok => String::new(),
                doctor::CatalogCacheState::Missing => " missing".into(),
                doctor::CatalogCacheState::NotARepo => " not a git repo".into(),
                doctor::CatalogCacheState::ManifestInvalid => " manifest invalid".into(),
                doctor::CatalogCacheState::Orphan => {
                    format!(" orphan at {}", c.cache_path.display())
                }
            };
            writeln!(out, "  {:30}     {}{}", c.name, glyph, suffix)?;
        }
    }
    writeln!(out)?;

    // FR-M-DOC-2: workspace-registry status line.
    if report.workspace_registry.present {
        writeln!(
            out,
            "Workspace registry: opt-in (file present, {} tracked)",
            report.workspace_registry.tracked,
        )?;
    } else {
        writeln!(out, "Workspace registry: opt-in (file absent)")?;
    }
    writeln!(out)?;

    writeln!(out, "Detected harnesses:")?;
    for h in &report.harnesses {
        let glyph = if h.present { &ok } else { &info };
        let label = harness_display_name(&h.name);
        if h.present {
            writeln!(out, "  {glyph} {label:14} {}", h.path.display())?;
        } else {
            writeln!(
                out,
                "  {glyph} {label:14} {}    (not detected)",
                h.path.display()
            )?;
        }
    }
    writeln!(out)?;

    // Phase 5 / US5.b: prompts surface (skipped when absent).
    if let Some(p) = &report.prompts {
        let workspace_label = report
            .workspace
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "global".to_owned());
        writeln!(
            out,
            "Prompts surface for workspace \"{}\":",
            workspace_label
        )?;
        if p.prompts.is_empty() {
            writeln!(out, "  (no user-invocable entries enrolled)")?;
        } else {
            // Render per-plugin grouping. The PromptDescriptor.name is
            // the final prompt name (post-collision). We re-construct
            // the plugin grouping by looking at the collision records'
            // member entries — but the simpler path is to enumerate
            // descriptors directly with a single section header.
            // Per the contract's "Human-mode rendering" section we
            // qualify with the harness prefix `/mcp__tome__<name>` —
            // R-m11 (US5.c): consume the canonical constant from
            // `src/mcp/mod.rs` rather than hard-coding the literal here.
            for d in &p.prompts {
                writeln!(out, "  {}{}", crate::mcp::MCP_SLASH_PREFIX, d.name)?;
            }
        }
        if !p.collisions.is_empty() {
            writeln!(out)?;
            writeln!(out, "  Prompt name collisions:")?;
            for c in &p.collisions {
                writeln!(
                    out,
                    "    base `{}` → {} entries (winner takes the base name; losers suffixed)",
                    c.base_name,
                    c.entries.len(),
                )?;
                for entry in &c.entries {
                    writeln!(
                        out,
                        "      - {}/{} ({}) → {}",
                        entry.identity.catalog,
                        entry.identity.plugin,
                        entry.identity.name,
                        entry.final_name,
                    )?;
                }
            }
        }
        writeln!(out)?;
    }

    // Phase 5 / US5.b: orphan persistent-data directories.
    if let Some(o) = &report.orphan_data_dirs
        && (!o.plugin_data.is_empty() || !o.workspace_data.is_empty())
    {
        writeln!(out, "Orphan persistent data directories:")?;
        if !o.plugin_data.is_empty() {
            writeln!(out, "  plugin-data (no longer enabled in any workspace):")?;
            for p in &o.plugin_data {
                writeln!(out, "    {}", p.display())?;
            }
        }
        if !o.workspace_data.is_empty() {
            writeln!(out, "  workspace-data:")?;
            for p in &o.workspace_data {
                writeln!(out, "    {}", p.display())?;
            }
        }
        writeln!(out)?;
        writeln!(
            out,
            "  Cleanup: not auto-fixable in Phase 5. Manual rm -rf required; future phases will add tooling.",
        )?;
        writeln!(out)?;
    }

    // Phase 5 / US5.b: per-kind entry counts.
    if let Some(c) = &report.entry_counts {
        writeln!(out, "Entry counts:")?;
        writeln!(out, "  Skills:               {}", c.skills)?;
        writeln!(out, "  Commands:             {}", c.commands)?;
        writeln!(out, "  Agents:               {}", c.agents)?;
        writeln!(out, "  Pending re-embedding: {}", c.pending_re_embedding,)?;
        writeln!(out)?;
    }

    // Phase 6 / US5: hooks contribution + drift (Claude Code).
    if let Some(h) = &report.hooks
        && !h.plugins.is_empty()
    {
        writeln!(out, "Hooks (Claude Code):")?;
        for p in &h.plugins {
            for c in &p.contributed {
                writeln!(
                    out,
                    "  {ok} {}:{} {} x{} contributed",
                    p.catalog, p.plugin, c.event, c.count,
                )?;
            }
            for m in &p.missing {
                writeln!(
                    out,
                    "  {warn} {}:{} {} x{} expected but missing (drift; re-merges on next sync)",
                    p.catalog, p.plugin, m.event, m.count,
                )?;
            }
        }
        writeln!(out)?;
    }

    // Phase 6 / US5: guardrails regions per file.
    if let Some(g) = &report.guardrails
        && !g.files.is_empty()
    {
        writeln!(out, "Guardrails regions:")?;
        for f in &g.files {
            writeln!(out, "  {}", f.path.display())?;
            for cp in &f.present {
                let suppressed = f
                    .suppressed
                    .iter()
                    .any(|s| s.catalog == cp.catalog && s.plugin == cp.plugin);
                let orphaned = f
                    .orphaned
                    .iter()
                    .any(|o| o.catalog == cp.catalog && o.plugin == cp.plugin);
                let (glyph, suffix) = if orphaned {
                    (&info, "  (orphaned)")
                } else if suppressed {
                    (&info, "  (suppressed by hooks)")
                } else {
                    (&ok, "")
                };
                writeln!(out, "    {glyph} {}:{}{suffix}", cp.catalog, cp.plugin)?;
            }
        }
        writeln!(out)?;
    }

    // Phase 6 / US5: native agent files per harness.
    if let Some(a) = &report.agents
        && !a.harnesses.is_empty()
    {
        writeln!(out, "Native agents:")?;
        for h in &a.harnesses {
            writeln!(
                out,
                "  {} ({} present, {} orphaned)",
                harness_display_name(&h.harness),
                h.present.len(),
                h.orphaned.len(),
            )?;
            for orphan in &h.orphaned {
                writeln!(out, "    {warn} {orphan} (orphaned)")?;
            }
            for d in &h.dropped_fields {
                writeln!(out, "    {info} {} dropped: {:?}", d.agent, d.fields)?;
            }
        }
        writeln!(out)?;
    }

    // Phase 6 / US5: privilege-escalation audit (FR-051).
    if let Some(p) = &report.privilege_escalation
        && !p.plugins.is_empty()
    {
        writeln!(
            out,
            "Privileged agents (carry hooks/mcpServers/permissionMode):"
        )?;
        for plug in &p.plugins {
            for ag in &plug.agents {
                writeln!(
                    out,
                    "  {warn} {}:{}/{} → {:?}",
                    plug.catalog, plug.plugin, ag.name, ag.fields,
                )?;
            }
        }
        writeln!(out)?;
    }

    // Phase 6 / US5: persona surface (only present when personas are on).
    if let Some(p) = &report.personas {
        writeln!(out, "Agent personas (expose_agents_as_personas on):")?;
        for persona in &p.personas {
            let clash = if persona.clash_prefixed {
                "  (clash-prefixed)"
            } else {
                ""
            };
            writeln!(
                out,
                "  {}{}{}",
                crate::mcp::MCP_SLASH_PREFIX,
                persona.resolved_persona_name,
                clash,
            )?;
        }
        writeln!(out, "  {}{}", crate::mcp::MCP_SLASH_PREFIX, p.drop_persona)?;
        writeln!(out)?;
    }

    // Phase 8 cutover surfaces.
    if !report.legacy_model_manifests.is_empty() {
        writeln!(out, "Legacy model manifests (pre-cutover manifest.json):")?;
        for name in &report.legacy_model_manifests {
            writeln!(
                out,
                "  {warn} {name}  (run `tome doctor --fix` to migrate to manifest.toml)",
            )?;
        }
        writeln!(out)?;
    }
    if !report.unconverted_plugins.is_empty() {
        writeln!(
            out,
            "Unconverted plugins (legacy plugin.json, no tome-plugin.toml):"
        )?;
        for p in &report.unconverted_plugins {
            writeln!(out, "  {warn} {p}  (run `tome plugin convert <source>`)")?;
        }
        writeln!(out)?;
    }

    // Phase 9 / US4: meta-skill drift (stale only; missing is "not installed").
    if !report.meta_skills.is_empty() {
        writeln!(out, "Meta skills (drift):")?;
        for m in &report.meta_skills {
            writeln!(
                out,
                "  {warn} {} @ {} ({})  {}  (run `tome doctor --fix`)",
                m.skill_id, m.harness, m.scope, m.state,
            )?;
        }
        writeln!(out)?;
    }

    // Phase 10 / US5: read-only telemetry subsystem report (FR-064).
    if let Some(t) = &report.telemetry {
        let on_off = if t.enabled { "enabled" } else { "disabled" };
        writeln!(out, "Telemetry:")?;
        writeln!(
            out,
            "  state:     {on_off} ({})",
            telemetry_source_label(t.source),
        )?;
        if let Some(err) = &t.config_error {
            writeln!(out, "  {warn} config error: {err}")?;
        }
        if t.install_id.present {
            let mode = t
                .install_id
                .mode
                .map(|m| format!(" mode {m:04o}"))
                .unwrap_or_default();
            let age = t
                .install_id
                .age_seconds
                .map(|s| format!(" age {s}s"))
                .unwrap_or_default();
            writeln!(out, "  install:   {}{mode}{age}", t.install_id.path)?;
        } else {
            writeln!(out, "  install:   (none) {}", t.install_id.path)?;
        }
        let oldest = t
            .queue
            .oldest_age_seconds
            .map(|s| format!(", oldest {s}s"))
            .unwrap_or_default();
        writeln!(
            out,
            "  queue:     {} pending, {} unparsable{oldest}",
            t.queue.pending, t.queue.corrupt,
        )?;
        match &t.last_flush {
            Some(lf) => match lf.status {
                Some(s) => writeln!(out, "  last flush: {} (status {s})", lf.timestamp)?,
                None => writeln!(
                    out,
                    "  last flush: {} (no successful delivery)",
                    lf.timestamp
                )?,
            },
            None => writeln!(out, "  last flush: never")?,
        }
        writeln!(out, "  endpoint:  {}", t.endpoint)?;
        if t.allowlist.is_empty() {
            writeln!(out, "  allowlist: (empty)")?;
        } else {
            writeln!(out, "  allowlist:")?;
            for e in &t.allowlist {
                writeln!(out, "    {} -> {}", e.short_id, e.canonical_source)?;
            }
        }
        writeln!(out)?;
    }

    if !report.suggested_fixes.is_empty() {
        writeln!(out, "Suggested fixes:")?;
        for f in &report.suggested_fixes {
            let auto = if f.auto_fixable { " (auto)" } else { "" };
            writeln!(out, "  {}: {}{auto}", f.subsystem, f.diagnosis)?;
            writeln!(out, "    {}", f.command)?;
        }
        writeln!(out)?;
    }

    let overall_label = match report.overall {
        DoctorClassification::Ok => format!("{ok} healthy"),
        DoctorClassification::Degraded => format!("{warn} degraded"),
        DoctorClassification::Unhealthy => format!("{fail} unhealthy"),
    };
    writeln!(out, "Overall:         {overall_label}")?;
    Ok(())
}

/// Human label for the telemetry enabled-state provenance. Mirrors the
/// `commands::telemetry` status labels so the two surfaces read identically.
fn telemetry_source_label(source: crate::telemetry::config::Source) -> &'static str {
    use crate::telemetry::config::Source;
    match source {
        Source::EnvOn => "TOME_TELEMETRY=1",
        Source::EnvOff => "TOME_TELEMETRY=0",
        Source::Ci => "CI auto-off",
        Source::Config => "config file",
        Source::Default => "default",
    }
}

fn harness_display_name(machine_name: &str) -> &'static str {
    match machine_name {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "gemini" => "Gemini CLI",
        "opencode" => "OpenCode",
        "continue" => "Continue",
        _ => "unknown",
    }
}

fn drift_label(d: &crate::index::meta::DriftStatus) -> String {
    use crate::index::meta::DriftStatus;
    match d {
        DriftStatus::None => "none".to_owned(),
        DriftStatus::EmbedderNameDrift { stored, configured } => {
            format!("embedder name drift (stored: {stored}, configured: {configured})")
        }
        DriftStatus::EmbedderVersionDrift { stored, configured } => {
            format!("embedder version drift (stored: {stored}, configured: {configured})")
        }
        DriftStatus::RerankerDrift { stored, configured } => {
            format!("reranker drift (stored: {stored}, configured: {configured})")
        }
        DriftStatus::SummariserDrift { stored, configured } => {
            format!("summariser drift (stored: {stored}, configured: {configured})")
        }
    }
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kib = bytes as f64 / 1024.0;
    if kib < 1024.0 {
        return format!("{:.1} KiB", kib);
    }
    let mib = kib / 1024.0;
    format!("{:.1} MiB", mib)
}
