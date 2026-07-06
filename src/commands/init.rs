//! `tome init` — guided first-run setup wizard (issue #418).
//!
//! Composes the EXISTING building blocks — nothing here re-implements a
//! step's logic. Each step dispatches through the same public command
//! entry points the manual flow uses, so exit codes, prompts, telemetry,
//! and `next:` hints stay identical to the standalone commands:
//!
//! 1. workspace bind    → `commands::workspace::run(WorkspaceCommand::Use …)`
//!    (the `workspace use --create` path)
//! 2. harness configure → `commands::harness::run(HarnessCommand::Use …)`
//!    (the same detection `tome harness use` with no args performs)
//! 3. catalog add       → `commands::catalog::run(CatalogCommand::Add …)`
//! 4. plugin enable     → `commands::plugin::run(PluginCommand::Enable …)`,
//!    preceded by an up-front model-download warning sized from
//!    `MODEL_REGISTRY` via the shared `missing_models_for_active_profile`
//!    (the same set the enable prompt itself gates on)
//! 5. finish            → the standard `tome status` panel
//!    (`status::full_report` + `status::emit_human` — NOT `status::run`,
//!    whose health-code `process::exit` would make a half-configured
//!    fresh install exit non-zero) plus the outstanding manual commands.
//!
//! ## Shape (the `plugin/interactive.rs` discipline)
//!
//! The pure planning half — [`InitState`] + [`plan`] — computes which steps
//! are outstanding from a snapshot and is unit-testable without a TTY. The
//! thin interactive driver ([`run`]) snapshots live state, walks the plan,
//! and re-checks each step's precondition just before prompting (a step
//! can complete or become moot mid-run — e.g. binding a workspace changes
//! the effective harness list).
//!
//! ## Skip / cancel / error semantics
//!
//! - **Idempotent**: re-running offers only the outstanding steps; a fully
//!   set-up install says so and shows the status panel.
//! - **Every step skippable**: Esc (surfaced as [`TomeError::Interrupted`]
//!   by `presentation::prompt`) or an explicit Skip choice moves on — never
//!   an error. Declining the model download inside `plugin enable` (also
//!   `Interrupted`) skips that step the same way.
//! - **Ctrl-C outside a prompt** hits the global SIGINT handler (exit 8),
//!   like every other command. *Inside* a prompt, inquire cannot
//!   distinguish Ctrl-C from Esc — both surface as `Interrupted` — so an
//!   in-prompt Ctrl-C deliberately skips the step rather than exiting.
//! - **Step failures** follow the established forward-progress
//!   `first_error` pattern (`harness use`, the reconcilers): warn, keep
//!   going so later steps still land, surface the first failure's exit
//!   code after the closing panel.
//! - **Non-TTY** refuses with the existing [`TomeError::NotATerminal`]
//!   (exit 54) and points at the equivalent non-interactive commands. The
//!   global `--non-interactive` / `TOME_NONINTERACTIVE` auto-confirm
//!   contract cannot drive a multi-select wizard, so it takes the same
//!   refusal path rather than silently prompting anyway.
//! - **No `--json`**: the wizard is interactive-only (documented in the
//!   help text); structured consumers use the individual commands.

use std::fmt;
use std::io::Write;

use crate::cli::{
    CatalogAddArgs, CatalogCommand, GlobalScopeArgs, HarnessArgs, HarnessCommand, HarnessScopeArg,
    HarnessUseArgs, PluginCommand, PluginEnableArgs, WorkspaceCommand, WorkspaceUseArgs,
};
use crate::error::TomeError;
use crate::index::workspace_catalogs;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::presentation::{colour, prompt};
use crate::workspace::{ResolvedScope, ScopeSource, WorkspaceName};

// ---------------------------------------------------------------------------
// Pure planning half — unit-testable without a TTY.
// ---------------------------------------------------------------------------

/// Snapshot of the setup-relevant state, gathered once up front (and again
/// for the closing "remaining steps" list). Field per wizard decision; the
/// driver re-derives finer detail (candidate lists) live inside each step.
#[derive(Debug, Clone, Default)]
pub struct InitState {
    /// The current directory resolves to a project marker (`.tome/config.toml`
    /// in the CWD ancestry) — i.e. `ResolvedScope::project_root` is set.
    pub project_bound: bool,
    /// Auto-detected harnesses NOT already in the effective harness list —
    /// the same detection `tome harness use` (no args) runs, minus the same
    /// effective list `tome harness list` reports.
    pub unconfigured_detected: Vec<String>,
    /// Catalogs enrolled in the resolved workspace (`workspace_catalogs`).
    pub catalogs_enrolled: usize,
    /// Plugins with at least one enabled entry in the resolved workspace.
    pub plugins_enabled: u32,
}

/// One outstanding wizard step, in fixed wizard order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitStep {
    BindWorkspace,
    ConfigureHarnesses,
    AddCatalog,
    EnablePlugins,
}

impl InitStep {
    /// The equivalent manual command, for the closing "remaining steps"
    /// list and skip hints.
    fn manual_command(self) -> &'static str {
        match self {
            Self::BindWorkspace => "tome workspace use --create <name>",
            Self::ConfigureHarnesses => "tome harness use",
            Self::AddCatalog => "tome catalog add <source>",
            Self::EnablePlugins => "tome plugin enable <catalog>/<plugin>",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::BindWorkspace => "bind this directory to a workspace",
            Self::ConfigureHarnesses => "configure detected harnesses",
            Self::AddCatalog => "add a plugin catalog",
            Self::EnablePlugins => "enable plugins",
        }
    }
}

/// Compute the outstanding steps from a state snapshot. Pure — no I/O, no
/// TTY — so the idempotency contract ("offer only what is missing") is
/// directly unit-testable.
///
/// `EnablePlugins` is planned whenever nothing is enabled yet, even with
/// zero catalogs: the step runs after `AddCatalog` in wizard order, and the
/// driver re-checks live enrolment at that point (skipping gracefully when
/// the user also skipped the catalog step).
pub fn plan(state: &InitState) -> Vec<InitStep> {
    let mut steps = Vec::new();
    if !state.project_bound {
        steps.push(InitStep::BindWorkspace);
    }
    if !state.unconfigured_detected.is_empty() {
        steps.push(InitStep::ConfigureHarnesses);
    }
    if state.catalogs_enrolled == 0 {
        steps.push(InitStep::AddCatalog);
    }
    if state.plugins_enabled == 0 {
        steps.push(InitStep::EnablePlugins);
    }
    steps
}

/// Gather the live [`InitState`] for the resolved scope. Read-only: the
/// index is opened read-only when it exists and is never bootstrapped here
/// (a fresh install snapshots as all-zeros without creating `~/.tome`
/// state the user didn't ask for yet).
fn snapshot(scope: &ResolvedScope, paths: &Paths) -> Result<InitState, TomeError> {
    let project_bound = scope.project_root.is_some();
    let unconfigured_detected = unconfigured_detected_harnesses(scope, paths)?;

    let catalogs_enrolled = if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        // Mirror `check_index`'s tolerance: a missing workspace row or a
        // pre-junction schema degrades to "no catalogs" (the wizard then
        // offers the catalog step) instead of erroring where `status`
        // would have stayed graceful.
        match workspace_catalogs::list_for_workspace(&conn, scope.scope.name().as_str()) {
            Ok(list) => list.len(),
            Err(TomeError::WorkspaceNotFound { .. })
            | Err(TomeError::IndexIntegrityCheckFailure(_)) => 0,
            Err(e) => return Err(e),
        }
    } else {
        0
    };
    // `check_index` handles the missing-DB / stale-schema cases itself and
    // counts per-workspace enabled plugins — the same number `status` shows.
    let plugins_enabled = super::status::check_index(paths, &scope.scope)?.plugins_enabled;

    Ok(InitState {
        project_bound,
        unconfigured_detected,
        catalogs_enrolled,
        plugins_enabled,
    })
}

/// Detected-but-unconfigured harnesses: the auto-detected set (the same
/// `detect(home)` walk `tome harness use` with no args performs) minus the
/// effective configured list (the same resolver `tome harness list` uses).
fn unconfigured_detected_harnesses(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Vec<String>, TomeError> {
    let home = super::harness::home_root()?;
    let detected: Vec<String> = crate::harness::with_effective_modules(|mods| {
        mods.iter()
            .filter(|m| m.detect(&home))
            .map(|m| m.name().to_string())
            .collect()
    });
    let configured = super::harness::use_::compute_effective_names(scope, paths)?;
    Ok(detected
        .into_iter()
        .filter(|name| !configured.contains(name))
        .collect())
}

// ---------------------------------------------------------------------------
// Interactive driver.
// ---------------------------------------------------------------------------

/// The stderr pointer printed on a non-TTY (or forced non-interactive)
/// invocation, naming the equivalent manual commands. Kept as one constant
/// so the integration test pins the real message.
const NON_TTY_POINTER: &str = "`tome init` is interactive and requires a terminal. \
Run the equivalent steps manually:\n  \
1. tome catalog add <source>\n  \
2. tome plugin enable <catalog>/<plugin>\n  \
3. tome harness use\n  \
4. tome query \"<what you need>\"";

pub fn run(scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    if mode == Mode::Json {
        return Err(TomeError::Usage(
            "--json is not valid for `tome init` (interactive-only); run the individual \
             commands (`tome catalog add`, `tome plugin enable`, `tome harness use`) \
             with --json instead"
                .to_owned(),
        ));
    }

    // FR-051 discipline (the bare-`tome plugin` precedent): refuse without a
    // terminal on BOTH ends, writing the pointer message before returning the
    // existing NotATerminal variant (exit 54). `--non-interactive` /
    // `TOME_NONINTERACTIVE` take the same path — auto-confirm cannot answer a
    // multi-select, and silently prompting would break that flag's contract.
    if !(output::stdin_is_tty() && output::stdout_is_tty()) || prompt::non_interactive() {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "{NON_TTY_POINTER}");
        return Err(TomeError::NotATerminal);
    }

    let paths = Paths::resolve()?;
    // The bind step re-resolves this (a fresh project marker changes the
    // scope every later step should see), hence the local mutable copy.
    let mut scope = scope.clone();

    say(&format!(
        "Workspace: `{}` ({})",
        scope.scope.name().as_str(),
        describe_source(scope.source),
    ))?;

    let steps = plan(&snapshot(&scope, &paths)?);
    if steps.is_empty() {
        say("Everything is already set up — nothing to do.\n")?;
        return finish(&scope, &paths);
    }

    // Forward-progress `first_error` (the `harness use` pattern): a failed
    // step warns and continues so later steps still land; the first
    // failure's exit code surfaces after the closing panel.
    let mut first_error: Option<TomeError> = None;
    let total = steps.len();
    for (i, step) in steps.iter().enumerate() {
        say(&format!("\n[{}/{}] {}", i + 1, total, step.summary()))?;
        let result = match step {
            InitStep::BindWorkspace => step_bind_workspace(&mut scope, &paths, mode),
            InitStep::ConfigureHarnesses => step_configure_harnesses(&scope, &paths, mode),
            InitStep::AddCatalog => step_add_catalog(&scope, mode),
            InitStep::EnablePlugins => step_enable_plugins(&scope, &paths, mode),
        };
        if let Err(e) = result {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(
                err,
                "{} step failed ({e}); continuing — re-run `tome init` or `{}` later",
                colour::warning("warning:"),
                step.manual_command(),
            );
            if first_error.is_none() {
                first_error = Some(e);
            }
        }
    }

    say("")?;
    finish(&scope, &paths)?;
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Closing panel: the standard `tome status` surface (identical report +
/// human rendering) followed by whatever is STILL outstanding, as manual
/// commands. Never exits with the status health code — mid-setup a fresh
/// install is legitimately "unhealthy" until models download.
fn finish(scope: &ResolvedScope, paths: &Paths) -> Result<(), TomeError> {
    let report = super::status::full_report(scope, paths, false)?;
    super::status::emit_human(&report)?;

    let remaining = plan(&snapshot(scope, paths)?);
    let mut out = std::io::stdout().lock();
    if remaining.is_empty() {
        writeln!(out, "\nSetup complete. Try: tome query \"<what you need>\"")?;
    } else {
        writeln!(out, "\nRemaining steps:")?;
        for step in remaining {
            writeln!(out, "  - {}  ({})", step.manual_command(), step.summary())?;
        }
        writeln!(
            out,
            "Re-run `tome init` any time to pick up where you left off."
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Steps. Each re-checks its precondition live (state can change mid-run),
// maps prompt-cancel to "skipped", and delegates the real work to the
// existing command entry point.
// ---------------------------------------------------------------------------

/// A prompt outcome the wizard steps discriminate: a real answer, or
/// Esc/cancel = skip the step (never an error — the wizard contract).
enum StepChoice<T> {
    Picked(T),
    Skip,
}

fn prompt_or_skip<T>(result: Result<T, TomeError>) -> Result<StepChoice<T>, TomeError> {
    match result {
        Ok(v) => Ok(StepChoice::Picked(v)),
        Err(TomeError::Interrupted) => Ok(StepChoice::Skip),
        Err(e) => Err(e),
    }
}

/// Step 1 — bind the CWD to a workspace via the real `workspace use
/// [--create]` path (`commands::workspace::run`), then re-resolve the scope
/// so later steps see the fresh project binding.
fn step_bind_workspace(
    scope: &mut ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    enum Choice {
        Existing(WorkspaceName),
        Create,
        Skip,
    }
    impl fmt::Display for Choice {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Existing(name) => write!(f, "Bind to `{}`", name.as_str()),
                Self::Create => f.write_str("Create a new workspace and bind to it"),
                Self::Skip => f.write_str("Skip"),
            }
        }
    }

    let mut choices: Vec<Choice> = crate::workspace::sync::list_workspace_names(paths)?
        .into_iter()
        .map(Choice::Existing)
        .collect();
    choices.push(Choice::Create);
    choices.push(Choice::Skip);

    let picked = match prompt_or_skip(prompt::select(
        "This directory has no project binding. Bind it to a workspace?",
        choices,
    ))? {
        StepChoice::Picked(c) => c,
        StepChoice::Skip => return skipped(InitStep::BindWorkspace),
    };

    let (name, create) = match picked {
        Choice::Skip => return skipped(InitStep::BindWorkspace),
        Choice::Existing(name) => (name, false),
        Choice::Create => match prompt_workspace_name()? {
            StepChoice::Picked(name) => (name, true),
            StepChoice::Skip => return skipped(InitStep::BindWorkspace),
        },
    };

    // The real `workspace use [--create]` path — dangerous-cwd guard,
    // create-if-absent, atomic marker landing, harness sync, telemetry and
    // the `next:` hint all included.
    super::workspace::run(
        WorkspaceCommand::Use(WorkspaceUseArgs {
            name: Some(name.as_str().to_owned()),
            create,
            force: false,
        }),
        None,
        scope,
        mode,
    )?;

    // Re-resolve with default inputs so the fresh marker wins exactly as it
    // would on the next invocation (an explicit `-w` flag from THIS
    // invocation deliberately stops shadowing the new binding).
    *scope = crate::workspace::resolution::resolve(&GlobalScopeArgs::default(), paths)?;
    Ok(())
}

/// Prompt-and-validate loop for a new workspace name. Empty input or Esc
/// skips; an invalid name explains and re-prompts.
fn prompt_workspace_name() -> Result<StepChoice<WorkspaceName>, TomeError> {
    loop {
        let raw = match prompt_or_skip(prompt::text(
            "New workspace name (empty to skip)",
            Some("1-64 alphanumeric / hyphen / underscore characters"),
        ))? {
            StepChoice::Picked(s) => s.trim().to_owned(),
            StepChoice::Skip => return Ok(StepChoice::Skip),
        };
        if raw.is_empty() {
            return Ok(StepChoice::Skip);
        }
        match WorkspaceName::parse(&raw) {
            Ok(name) => return Ok(StepChoice::Picked(name)),
            Err(e) => {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "{} {e}", colour::warning("invalid name:"));
            }
        }
    }
}

/// Step 2 — multi-select the detected-but-unconfigured harnesses, then run
/// the real `harness use` machinery (`commands::harness::run`) for the
/// selection: settings edit, effective-list recompute, project sync,
/// MCP notices, forward progress — all the standalone command's behaviour.
fn step_configure_harnesses(
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    // Recompute live: the bind step may have changed the effective list.
    let candidates = unconfigured_detected_harnesses(scope, paths)?;
    if candidates.is_empty() {
        return say("Every detected harness is already configured — nothing to select.");
    }

    let picked = match prompt_or_skip(prompt::multiselect(
        "Select harnesses to configure (space toggles, enter confirms; none = skip)",
        candidates,
    ))? {
        StepChoice::Picked(v) => v,
        StepChoice::Skip => return skipped(InitStep::ConfigureHarnesses),
    };
    if picked.is_empty() {
        return skipped(InitStep::ConfigureHarnesses);
    }

    // Scope: with a project binding, leave the default resolution (explicit
    // none → config `[harness] default_scope` → project) — identical to a
    // manual `tome harness use`. Without one, project scope would be a
    // usage error, so fall back to global explicitly and say so.
    let scope_arg = if scope.project_root.is_some() {
        None
    } else {
        say("(no project binding — configuring at --scope global)")?;
        Some(HarnessScopeArg::Global)
    };

    super::harness::run(
        HarnessArgs {
            command: Some(HarnessCommand::Use(HarnessUseArgs {
                names: picked,
                all: false,
                include_opt_in: false,
                scope: scope_arg,
                force: false,
            })),
        },
        scope,
        mode,
    )
}

/// Step 3 — free-text catalog source → the real `catalog add` path. A
/// failed add (typo'd URL, unreachable remote) warns and re-prompts;
/// empty input or Esc skips.
fn step_add_catalog(scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    loop {
        let source = match prompt_or_skip(prompt::text(
            "Catalog source (empty to skip)",
            Some("an owner/repo GitHub shorthand, a git URL, or a file:// path"),
        ))? {
            StepChoice::Picked(s) => s.trim().to_owned(),
            StepChoice::Skip => return skipped(InitStep::AddCatalog),
        };
        if source.is_empty() {
            return skipped(InitStep::AddCatalog);
        }

        match super::catalog::run(
            CatalogCommand::Add(CatalogAddArgs {
                source,
                name: None,
                ref_: None,
            }),
            scope,
            mode,
        ) {
            Ok(()) => return Ok(()),
            Err(e) => {
                // Free-text is the one step where user error is routine —
                // report and re-prompt instead of failing the wizard run.
                let mut err = std::io::stderr().lock();
                let _ = writeln!(
                    err,
                    "{} catalog add failed: {e}\nTry another source, or leave empty to skip.",
                    colour::warning("warning:"),
                );
            }
        }
    }
}

/// Step 4 — multi-select disabled plugins across every enrolled catalog and
/// enable them via the real `plugin enable` path. The up-front download
/// warning states the active profile and the REAL total size of the models
/// still missing from disk (from `MODEL_REGISTRY` `size_bytes`, via the
/// same shared helper the enable prompt gates on) — shown only when
/// something would actually download.
fn step_enable_plugins(scope: &ResolvedScope, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    /// Multi-select row: a slash-qualified plugin id.
    struct Pick(PluginId);
    impl fmt::Display for Pick {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    const NO_CATALOGS_SKIP: &str = "No catalogs enrolled — skipping plugin enable. \
         Add one with `tome catalog add <source>`, then `tome plugin enable`.";

    let workspace_name = scope.scope.name().as_str();
    let candidates: Vec<Pick> = {
        // A missing DB is the no-catalogs case; skip before
        // `open_index_for_read` can bootstrap `~/.tome` state from a read
        // path (`snapshot` promises the wizard never does that).
        if !paths.index_db.is_file() {
            return say(NO_CATALOGS_SKIP);
        }
        let conn = super::plugin::open_index_for_read(paths, &scope.scope)?;
        if workspace_catalogs::list_for_workspace(&conn, workspace_name)?.is_empty() {
            return say(NO_CATALOGS_SKIP);
        }
        let mut out = Vec::new();
        for id in super::plugin::discoverable_plugin_ids(&conn, paths, workspace_name)? {
            let agg = super::plugin::aggregate_for_plugin(
                &conn,
                workspace_name,
                &id.catalog,
                &id.plugin,
            )?;
            let enabled = agg.total > 0 && agg.enabled > 0;
            if !enabled {
                out.push(Pick(id));
            }
        }
        out
        // Read handle dropped here — `plugin enable` below opens its own
        // write connection under the advisory lock.
    };
    if candidates.is_empty() {
        return say("Every discoverable plugin is already enabled — nothing to select.");
    }

    // The up-front warning the manual flow lacks (#418): before the user
    // commits to a selection, state the active profile and the real bytes
    // the first enable would download. Only when models are missing —
    // a warm install prompts for nothing, so it gets no warning either.
    let missing = super::plugin::missing_models_for_active_profile(paths)?;
    if !missing.is_empty() {
        let profile = active_profile_or_default(paths)?;
        let total: u64 = missing.iter().map(|e| e.size_bytes).sum();
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "{} enabling a first plugin downloads the `{}` profile's models:",
            colour::warning("Note:"),
            profile.as_str(),
        )?;
        for entry in &missing {
            writeln!(
                out,
                "  - {} (~{}, {})",
                entry.name,
                super::plugin::human_mb(entry.size_bytes),
                entry.licence,
            )?;
        }
        writeln!(
            out,
            "  Total: ~{} — you will be asked to confirm before anything downloads.",
            super::plugin::human_mb(total),
        )?;
    }

    let picked = match prompt_or_skip(prompt::multiselect(
        "Select plugins to enable (space toggles, enter confirms; none = skip)",
        candidates,
    ))? {
        StepChoice::Picked(v) => v,
        StepChoice::Skip => return skipped(InitStep::EnablePlugins),
    };
    if picked.is_empty() {
        return skipped(InitStep::EnablePlugins);
    }

    // The real `plugin enable` path: model-download confirm, spinner,
    // indexing, per-plugin forward progress, telemetry, `next:` hints.
    match super::plugin::run(
        PluginCommand::Enable(PluginEnableArgs {
            ids: picked.into_iter().map(|p| p.0.to_string()).collect(),
            catalog: None,
            yes: false,
            tier: None,
            sync: false,
        }),
        scope,
        mode,
    ) {
        Ok(()) => Ok(()),
        // Declining the model download surfaces as Interrupted (exit 8 in
        // the standalone command); in the wizard a decline is a skip.
        Err(TomeError::Interrupted) => {
            say("Model download declined — plugins were not enabled.")?;
            skipped(InitStep::EnablePlugins)
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Small shared bits.
// ---------------------------------------------------------------------------

/// The active model profile with the fresh-install fallback — the display
/// companion to `missing_models_for_active_profile` (same DB-then-default
/// resolution, so the warning names the profile whose sizes it lists).
fn active_profile_or_default(paths: &Paths) -> Result<crate::embedding::Profile, TomeError> {
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::meta::active_profile(&conn)
    } else {
        Ok(crate::embedding::Profile::DEFAULT)
    }
}

fn describe_source(source: ScopeSource) -> &'static str {
    match source {
        ScopeSource::Flag => "from --workspace",
        ScopeSource::Env => "from TOME_WORKSPACE",
        ScopeSource::Config => "from `[workspace] default` in ~/.tome/config.toml",
        ScopeSource::ProjectMarker => "bound via this project's marker",
        ScopeSource::GlobalFallback => "the default global workspace",
    }
}

fn say(message: &str) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{message}")?;
    Ok(())
}

fn skipped(step: InitStep) -> Result<(), TomeError> {
    say(&format!("Skipped — run `{}` later.", step.manual_command()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> InitState {
        InitState {
            project_bound: false,
            unconfigured_detected: vec!["claude-code".to_owned()],
            catalogs_enrolled: 0,
            plugins_enabled: 0,
        }
    }

    /// Fresh install: every step is outstanding, in wizard order.
    #[test]
    fn plan_fresh_state_offers_all_steps_in_order() {
        assert_eq!(
            plan(&fresh()),
            vec![
                InitStep::BindWorkspace,
                InitStep::ConfigureHarnesses,
                InitStep::AddCatalog,
                InitStep::EnablePlugins,
            ],
        );
    }

    /// Fully set-up install: empty plan (the idempotency contract).
    #[test]
    fn plan_complete_state_is_empty() {
        let state = InitState {
            project_bound: true,
            unconfigured_detected: vec![],
            catalogs_enrolled: 2,
            plugins_enabled: 3,
        };
        assert!(plan(&state).is_empty());
    }

    /// Each satisfied precondition drops exactly its own step.
    #[test]
    fn plan_partial_states_offer_only_outstanding_steps() {
        let mut bound = fresh();
        bound.project_bound = true;
        assert_eq!(
            plan(&bound),
            vec![
                InitStep::ConfigureHarnesses,
                InitStep::AddCatalog,
                InitStep::EnablePlugins,
            ],
        );

        let mut harnessed = fresh();
        harnessed.unconfigured_detected.clear();
        assert_eq!(
            plan(&harnessed),
            vec![
                InitStep::BindWorkspace,
                InitStep::AddCatalog,
                InitStep::EnablePlugins,
            ],
        );

        let mut catalogued = fresh();
        catalogued.catalogs_enrolled = 1;
        assert_eq!(
            plan(&catalogued),
            vec![
                InitStep::BindWorkspace,
                InitStep::ConfigureHarnesses,
                InitStep::EnablePlugins,
            ],
        );

        let mut enabled = fresh();
        enabled.plugins_enabled = 1;
        assert_eq!(
            plan(&enabled),
            vec![
                InitStep::BindWorkspace,
                InitStep::ConfigureHarnesses,
                InitStep::AddCatalog,
            ],
        );
    }

    /// No detected-but-unconfigured harness ⇒ no harness step, even on an
    /// otherwise fresh install (nothing to select).
    #[test]
    fn plan_omits_harness_step_when_nothing_detected() {
        let mut state = fresh();
        state.unconfigured_detected.clear();
        assert!(!plan(&state).contains(&InitStep::ConfigureHarnesses));
    }

    /// Catalogs enrolled but nothing enabled: only the plugin step remains
    /// of the catalog/plugin pair.
    #[test]
    fn plan_catalogs_without_plugins_offers_enable_only() {
        let state = InitState {
            project_bound: true,
            unconfigured_detected: vec![],
            catalogs_enrolled: 1,
            plugins_enabled: 0,
        };
        assert_eq!(plan(&state), vec![InitStep::EnablePlugins]);
    }

    /// The non-TTY pointer names all four equivalent manual commands
    /// (`catalog add` → `plugin enable` → `harness use` → `query`).
    #[test]
    fn non_tty_pointer_names_the_manual_commands() {
        for needle in [
            "tome catalog add",
            "tome plugin enable",
            "tome harness use",
            "tome query",
        ] {
            assert!(
                NON_TTY_POINTER.contains(needle),
                "pointer must mention `{needle}`: {NON_TTY_POINTER}",
            );
        }
    }
}
