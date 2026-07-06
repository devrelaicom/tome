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

    // `verify_by_default` in config.toml: effective verify = flag OR config.
    // DEFENSIVE load (issue #287): doctor is the command you run to diagnose a
    // broken setup — a malformed `~/.tome/config.toml` must not exit 5 before
    // the report renders. The parse problem is surfaced as a `SuggestedFix`
    // below (and flips `overall` to Unhealthy), so it is reported loudly, not
    // swallowed; a malformed config simply means `verify_by_default` reads as
    // the default (false). Every NON-diagnostic command still loads strictly.
    let cfg = crate::config::load_or_default(&paths);
    let verify = args.verify || cfg.doctor.verify_by_default.unwrap_or(false);

    // Read-only probe for a malformed config (issue #287). `None` when absent or
    // well-formed; `Some(message)` carries the legible diagnostic (offending
    // key/section + line/column) — identical to the strict exit-5 every other
    // command emits. Captured before `assemble_report` so the finding is folded
    // into both the report and `--fix`'s re-assembly path below.
    let config_error = crate::config::probe_error(&paths);

    let mut report = doctor::assemble_report(scope, &paths, &home, verify)?;
    apply_config_finding(&mut report, config_error.as_deref());

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
        crate::telemetry::emit(crate::telemetry::event::MetaActionEvent {
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

        // Phase 12 / US4 (FR-017): `re_assemble` rebuilds `suggested_fixes` via
        // the 8-arg SSOT, which doesn't know about the corrupt-remote-index
        // fix. Re-check it read-only and re-append when it still holds. A
        // BUNDLED-local repair (`reindex --force`) clears
        // `meta.embedder_dimension`, so `check_corrupt_index` then reads
        // "meta absent → N/A → no finding": nothing is re-appended, the
        // re-assembled `index.integrity_ok` is back to true, and `overall`
        // returns to Ok → exit 0 (self-healed). A REMOTE mismatch is never
        // auto-fixed (paid API cost), so it re-appears as an
        // `auto_fixable: false` fix while `index.integrity_ok` stays false from
        // assembly — `overall` remains non-Ok and `has_remaining_manual_fixes`
        // is true → `DoctorFixNotSafe` (exit 75).
        doctor::reappend_corrupt_index_fix(&mut report, &paths, &cfg);

        // Issue #291: `re_assemble` rebuilds `suggested_fixes` + `overall` via
        // the 8-arg SSOT, which doesn't know about the provider-credential
        // findings. `doctor --fix` NEVER sets a user's env var, so a
        // still-missing `TOME_<NAME>_API_KEY` is never auto-fixed — re-apply the
        // findings read-only so they persist through `--fix` as non-auto-fixable
        // manual findings (→ exit 75 via `has_remaining_manual_fixes`) and keep
        // escalating `overall`, exactly like the corrupt-index / config cases.
        // Uses the strictly-loaded `cfg` (defensive-loaded in the CLI wrapper);
        // a provider referenced with a resolve error is reported by the
        // exit-93 path, not here.
        doctor::apply_provider_credential_findings(
            &mut report.suggested_fixes,
            &mut report.overall,
            &cfg,
        );

        // Issue #283: `re_assemble` rebuilds `suggested_fixes` via the 8-arg
        // SSOT, which doesn't know about the fresh-install onboarding nudges.
        // `doctor --fix` never enrols a catalog / enables a plugin / configures
        // a harness (those are user product decisions), so a still-not-set-up
        // install keeps its onboarding guidance through `--fix`. `Onboarding`
        // fixes are informational (excluded from the exit-75 gate), so they
        // never change the exit code.
        doctor::reappend_onboarding_fixes(&mut report);

        // `re_assemble` rebuilds `suggested_fixes` + `overall` via the SSOT,
        // which doesn't know about the config-parse finding. `doctor --fix`
        // NEVER rewrites the user-authored `config.toml`, so a malformed config
        // is never auto-fixed — re-probe read-only and re-apply so it persists
        // through `--fix` as a non-auto-fixable manual finding (→ exit 75 via
        // `has_remaining_manual_fixes`), exactly like the corrupt-remote-index
        // case above.
        apply_config_finding(&mut report, crate::config::probe_error(&paths).as_deref());
    }

    emit(&report, mode)?;

    // `tome.doctor_run`: emit AFTER the report renders but BEFORE any of the
    // exit paths below (one of which is a hard `std::process::exit` with a
    // health code that would otherwise skip the emit). `findings` is the raw number of
    // suggested-fix issues the report surfaced (the kernel buckets it).
    crate::telemetry::emit(crate::telemetry::event::DoctorRun {
        fix: args.fix,
        findings: report.suggested_fixes.len() as u32,
    });

    // Exit-code semantics per `contracts/doctor.md` (issue #282 — three
    // distinct health codes, matching `tome status`):
    // - Overall Ok → exit 0.
    // - Overall Degraded → exit `EXIT_HEALTH_DEGRADED` (10): the report found
    //   a non-fatal issue (queries still serve). Distinct from Unhealthy so a
    //   CI gate can fail-on-unhealthy-only; still non-zero.
    // - Overall Unhealthy → exit `EXIT_HEALTH_UNHEALTHY` (1).
    // - `--fix` ran but un-fixable issues remain → exit 75
    //   (`DoctorFixNotSafe`) INSTEAD of the health code (communicates "fix did
    //   something, but the work isn't done"). This takes precedence over the
    //   Degraded/Unhealthy split — it is the more actionable verdict after a
    //   repair attempt, and it routes through the closed `TomeError` map.
    let remaining_manual = doctor::fixes::has_remaining_manual_fixes(&report);
    let health_code = match report.overall {
        DoctorClassification::Ok => return Ok(()),
        DoctorClassification::Degraded => crate::error::EXIT_HEALTH_DEGRADED,
        DoctorClassification::Unhealthy => crate::error::EXIT_HEALTH_UNHEALTHY,
    };
    if args.fix && remaining_manual {
        return Err(TomeError::DoctorFixNotSafe {
            // Issue #283: mirror `has_remaining_manual_fixes`'s exclusion of
            // `Subsystem::Onboarding` exactly. Onboarding nudges sort BEFORE the
            // genuine subsystem fixes (e.g. Config), so a bare `.find(|f|
            // !f.auto_fixable)` would label the error with the one subsystem
            // that is explicitly not supposed to block. The two scans must use
            // the SAME predicate so the label names the fix that actually
            // triggered the exit-75 gate.
            subsystem: report
                .suggested_fixes
                .iter()
                .find(|f| !f.auto_fixable && f.subsystem != doctor::Subsystem::Onboarding)
                .map(|f| f.subsystem.to_wire_string())
                .unwrap_or_else(|| "unknown".to_owned()),
        });
    }
    // Raw exit (not a `TomeError`) so the report always renders first.
    std::process::exit(health_code);
}

/// Fold a malformed-`config.toml` parse error (issue #287) into the report: push
/// a non-auto-fixable `Config` `SuggestedFix` carrying the legible diagnosis, and
/// flip `overall` to `Unhealthy` so `tome doctor` exits non-zero while it holds.
///
/// A no-op when `config_error` is `None` (absent or well-formed config), so the
/// byte-stable minimal-report JSON pin is unchanged for a clean system. The
/// finding is `auto_fixable: false` — Tome never rewrites the user-authored
/// `config.toml`; the user fixes the named key by hand (the diagnosis quotes the
/// `toml` error, which names the offending key/section + line/column).
fn apply_config_finding(report: &mut DoctorReport, config_error: Option<&str>) {
    let Some(message) = config_error else {
        return;
    };
    report.suggested_fixes.push(doctor::SuggestedFix {
        subsystem: doctor::Subsystem::Config,
        // "could not be loaded" rather than "is malformed": `probe_error`'s
        // fallback arm also covers a pure I/O / unreadable / over-cap failure
        // (not only a TOML parse error), so the wording must not mislabel those.
        diagnosis: format!("`~/.tome/config.toml` could not be loaded: {message}"),
        // Pointer, not a runnable repair — Tome must not rewrite a user-owned
        // config; `auto_fixable: false` is what keeps `--fix` from "fixing" it.
        command: "edit ~/.tome/config.toml to correct the reported problem".to_owned(),
        auto_fixable: false,
    });
    report.overall = DoctorClassification::Unhealthy;
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
            crate::workspace::ScopeSource::Config => "config.toml [workspace] default",
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

    // Issue #432: the Phase 3 opt-in workspace registry file is gone —
    // `check_workspace_registry` is a permanent `present: false` stub kept
    // only for the byte-stable JSON envelope (see its doc comment). The
    // human section it fed read like something was missing on every healthy
    // install, so it is no longer rendered; the JSON field is untouched.

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

    // Phase 2 (native-agent expansion): unrepresented agents drop-report.
    if let Some(u) = &report.unrepresented_agents {
        let count = u.agents.len();
        let harness_list = u.rules_only_harnesses.join(", ");
        writeln!(
            out,
            "Agents without native form on rules-only harnesses ({harness_list}):"
        )?;
        writeln!(
            out,
            "  {warn} {count} agent{s} — reachable via MCP persona when expose_agents_as_personas is enabled",
            s = if count == 1 { "" } else { "s" },
        )?;
        for a in &u.agents {
            writeln!(out, "    {}:{}/{}", a.catalog, a.plugin, a.name)?;
        }
        writeln!(out)?;
    }

    // US11: plugin-hook translation surface (per-harness dispatch state).
    if let Some(ht) = &report.hook_translation {
        writeln!(out, "Hook translation:")?;
        for h in &ht.per_harness {
            let state = if h.enabled { "on" } else { "off" };
            let events = if h.registered_events.is_empty() {
                "—".to_string()
            } else {
                h.registered_events.join(", ")
            };
            writeln!(out, "  {} [{}]  registered: {}", h.harness, state, events)?;
            if !h.dropped_to_guardrails.is_empty() {
                let dropped = h.dropped_to_guardrails.join(", ");
                writeln!(out, "    {info} dropped to GUARDRAILS: {dropped}")?;
            }
            if h.manifest_stale {
                writeln!(
                    out,
                    "    {warn} manifest stale — run `tome sync` to reconcile",
                )?;
            }
            if h.trust_prompt_note {
                writeln!(
                    out,
                    "    {info} prompt-model configured (first execution may request trust)",
                )?;
            }
        }
        writeln!(out)?;
    }

    // Issue #292 (translation-fidelity loss): unrepresented hooks drop-report.
    if let Some(u) = &report.unrepresented_hooks {
        let count = u.hooks.len();
        let harness_list = u.rules_only_harnesses.join(", ");
        writeln!(
            out,
            "Hooks without native form on rules-only harnesses ({harness_list}):"
        )?;
        writeln!(
            out,
            "  {warn} {count} plugin hook{s} — rendered as GUARDRAILS.md prose only (not enforced)",
            s = if count == 1 { "" } else { "s" },
        )?;
        for h in &u.hooks {
            writeln!(out, "    {}:{}  {}", h.catalog, h.plugin, h.event)?;
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

    // Phase 12 / US4: remote provider report (only when providers configured).
    if !report.providers.is_empty() {
        writeln!(out, "Providers:")?;
        for p in &report.providers {
            // Issue #291: when no credential resolves, NAME the exact expected
            // env var (`TOME_<NAME>_API_KEY`, derived via the shared SSOT — never
            // hardcoded) so the fix is obvious inline. The credential VALUE is
            // never printed — only the env-var NAME (Principle XIII). The
            // derivation is gated inside the no-credential branch so it isn't
            // computed-and-discarded for every provider row that resolves fine.
            let cred = if p.credential_resolvable {
                "credential resolved".to_owned()
            } else {
                let env_var = crate::provider::config::derive_env_var_name(&p.name);
                format!("no credential (set {env_var})")
            };
            let reach = match p.reachable {
                Some(true) => format!("  {ok} reachable"),
                Some(false) => format!("  {fail} unreachable"),
                None => String::new(),
            };
            writeln!(
                out,
                "  {} ({}) [{}] — {}{}",
                p.name,
                p.kind,
                p.capabilities.join(", "),
                cred,
                reach,
            )?;
        }
        writeln!(out)?;
    }

    // Phase 13 (native-agent model-registry): model-registry source line.
    {
        let mr = &report.model_registry;
        writeln!(
            out,
            "Model registry:  {} ({} models, fetched {})",
            mr.source, mr.model_count, mr.fetched_at,
        )?;
        if mr.override_corrupt {
            writeln!(
                out,
                "  {warn} override file is corrupt (active registry fell back to baked)",
            )?;
            writeln!(
                out,
                "    run `tome models update --include-registry` to refresh",
            )?;
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
