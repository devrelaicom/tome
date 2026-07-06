//! `clap` derive definitions. Globals (`--json`, `-v`/`-vv`, plus the
//! Phase 4 `--workspace <name>`) live on the top-level `Cli`; `--force`
//! is per-subcommand but keeps the same name everywhere (FR-021).
//! `--help` is auto-supplied by clap; `--version` is intercepted by a
//! pre-parse hook in `main.rs` so the output can include embedder +
//! reranker identities (FR-021a).
//!
//! Phase 4 / F10 deleted the `--global` flag. Workspace identity is a
//! validated [`crate::workspace::WorkspaceName`] checked against the
//! central registry; the privileged `global` workspace is the silent
//! default when no `--workspace` flag, `TOME_WORKSPACE` env var, or
//! project marker is found.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// One step of the canonical first-run path (see [`SETUP_STEPS`]).
pub(crate) struct SetupStep {
    /// The command the user runs, with placeholder arguments.
    pub(crate) command: &'static str,
    /// One-line summary rendered beside the command in `--help`.
    pub(crate) summary: &'static str,
    /// Optional caveat rendered as an indented continuation line in `--help`
    /// (`tome status`'s panel renders commands only and skips it).
    pub(crate) note: Option<&'static str>,
}

/// #293 / #422: the canonical four-step first-run path. This is the SINGLE
/// source that both `tome --help`'s getting-started block ([`quickstart`],
/// below) and `tome status`'s "Getting started" panel render — the two
/// hand-maintained copies drifted once already (#422: help stopped at `query`
/// with no `harness use` step and no model-download warning, while status
/// stopped at `harness use` with no `query`), so the step list now lives in
/// exactly one place.
pub(crate) const SETUP_STEPS: [SetupStep; 4] = [
    SetupStep {
        command: "tome catalog add <source>",
        summary: "Register a catalog (a git URL or local path)",
        note: None,
    },
    SetupStep {
        command: "tome plugin enable <catalog>/<plugin>",
        summary: "Enable a plugin and index its skills",
        note: Some(
            "(the first enable offers a model download; size depends on `[models] profile`)",
        ),
    },
    SetupStep {
        command: "tome harness use <name>",
        summary: "Wire Tome into a detected coding harness",
        note: None,
    },
    SetupStep {
        command: "tome query \"<what you need>\"",
        summary: "Search enabled skills by intent",
        note: None,
    },
];

/// Issue #423: select the OUTSTANDING steps of [`SETUP_STEPS`] for a
/// partially-set-up install. Lives beside the step list so step selection and
/// step content share one home (the #422 lesson: two surfaces hand-listing
/// steps drift). `tome status`'s "Getting started" panel renders exactly this
/// subset; `tome --help`'s quickstart always renders the full list.
///
/// Semantics — a step is outstanding when its precondition is unmet:
///
/// * `catalog add` — no catalog enrolled;
/// * `plugin enable` — nothing enabled (with or without catalogs: it is the
///   next thing on the path either way);
/// * `harness use` — no harness configured in the resolved scope;
/// * `query` — nothing enabled yet, so a search cannot return results (once
///   plugins are enabled the user can already query; nudging it again would
///   be noise).
///
/// Deliberately NOT `init::plan` (detection-based `ConfigureHarnesses` gate +
/// no query step) nor doctor's `push_onboarding_fixes` (else-if semantics +
/// prose nudges, not steps): both encode their own surface's needs; this one
/// encodes the panel's. All three read the same three signals.
pub(crate) fn outstanding_setup_steps(
    catalogs_enrolled: u32,
    plugins_enabled: u32,
    harness_configured: bool,
) -> Vec<&'static SetupStep> {
    let mut out = Vec::new();
    if catalogs_enrolled == 0 {
        out.push(&SETUP_STEPS[0]);
    }
    if plugins_enabled == 0 {
        out.push(&SETUP_STEPS[1]);
    }
    if !harness_configured {
        out.push(&SETUP_STEPS[2]);
    }
    if plugins_enabled == 0 {
        out.push(&SETUP_STEPS[3]);
    }
    out
}

/// #293: the concise getting-started block appended to clap's help text. The
/// flat command list is a dead end for a first-time user; the [`SETUP_STEPS`]
/// are the actual happy path (add a catalog → enable a plugin → wire a
/// harness → query).
///
/// clap renders this on BOTH surfaces that show help, but they differ in the
/// clap convention this respects: `tome --help` prints help to STDOUT and exits
/// 0, while bare `tome` (missing the required subcommand) prints help to STDERR
/// and exits 2 (a usage error, per constitution principle II). Either way the
/// user sees the quickstart.
fn quickstart() -> String {
    let width = SETUP_STEPS
        .iter()
        .map(|s| s.command.len())
        .max()
        .unwrap_or(0);
    let mut out = String::from("Getting started:\n");
    for (i, step) in SETUP_STEPS.iter().enumerate() {
        out.push_str(&format!(
            "  {}. {:<width$}  {}\n",
            i + 1,
            step.command,
            step.summary,
        ));
        if let Some(note) = step.note {
            out.push_str(&format!("     {:<width$}  {}\n", "", note));
        }
    }
    out.push_str(
        "\nOr run `tome init` for a guided, step-by-step setup of the same flow.\n\
         Run `tome <command> --help` for details on any command.",
    );
    out
}

#[derive(Debug, Parser)]
#[command(
    name = "tome",
    about,
    long_about = None,
    after_help = quickstart(),
    // `--version` is intercepted by a pre-parse hook in `main.rs` so the
    // output can include embedder + reranker identities and honour
    // `--json`. clap's auto handler can't do either, hence the override.
    disable_version_flag = true,
)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout instead of human text.
    /// Env: `TOME_JSON` (any truthy value — set, non-empty, and not
    /// `0`/`false`/`no`/`off`) forces JSON when the flag is absent.
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable ANSI colour in human output. Overrides `[output] color` in
    /// `~/.tome/config.toml` and the `NO_COLOR` environment variable.
    /// Env: `TOME_JSON`-style truthy `TOME_NO_COLOR` also forces colour off
    /// (a Tome-specific override layered on top of the existing `NO_COLOR`
    /// precedence). The MCP path never emits colour regardless of this flag.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Increase log verbosity. `-v` = info, `-vv` = debug. Env: TOME_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Auto-confirm every prompt-bearing command (equivalent to passing that
    /// command's `--force` / `--yes`). Also enabled by `TOME_NONINTERACTIVE=1`.
    /// A non-`global` per-command `--yes` still exists on `plugin enable` and
    /// `telemetry reset`; this global switch works after any subcommand.
    #[arg(long = "non-interactive", global = true)]
    pub non_interactive: bool,

    #[command(flatten)]
    pub scope: GlobalScopeArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Workspace selection. Flattened into `Cli` so the flag appears at the
/// top level **and** on every subcommand (clap's `global = true`).
///
/// Phase 4 / F10 collapses the Phase 3 `--workspace <path>` /
/// `--global` pair into a single name-keyed `--workspace <name>` flag.
/// The privileged `global` workspace is the silent default — no flag
/// needed.
#[derive(Debug, Default, clap::Args)]
pub struct GlobalScopeArgs {
    /// Use the named workspace from the central registry. Must already
    /// exist; create via `tome workspace init <name>`. When omitted, the
    /// resolver consults the `TOME_WORKSPACE` environment variable (an empty
    /// value is ignored) and the project-marker walk before falling back to
    /// the privileged `global` workspace. `-w` is the short form.
    #[arg(short = 'w', long, global = true, value_name = "NAME")]
    pub workspace: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Guided first-run setup wizard: bind a workspace, configure detected
    /// harnesses, add a catalog, and enable plugins — composing the same
    /// commands you would run by hand. Idempotent (a re-run offers only the
    /// outstanding steps) and every step is skippable. Interactive only:
    /// requires a terminal and does not support `--json` (script the
    /// individual commands instead).
    Init(InitArgs),
    /// Manage registered catalogs.
    #[command(subcommand)]
    Catalog(CatalogCommand),
    /// Manage plugins from registered catalogs. Run with no subcommand to
    /// drop into the interactive catalog → plugin → action browse flow
    /// (refused on a non-TTY).
    Plugin(PluginArgs),
    /// Manage on-disk embedding / reranker model artefacts.
    #[command(subcommand)]
    Models(ModelsCommand),
    /// Search enabled skills across every catalog.
    Query(QueryArgs),
    /// Force re-embedding of one or many skills outside the
    /// `tome catalog update` schedule. Use for embedder upgrades or
    /// integrity recovery.
    Reindex(ReindexArgs),
    /// Report the health of every Phase 2 subsystem (models, index, drift).
    /// Exit 0 when everything is healthy; exit 1 on degraded or unhealthy.
    Status(StatusArgs),
    /// Run as a stdio MCP server backed by the resolved scope's index.
    /// stdin / stdout carry the MCP protocol exclusively; diagnostic
    /// logs go to `${XDG_STATE_HOME}/tome/mcp.log`. Designed to be
    /// launched by an MCP-compliant harness (Claude Code, Codex, Cursor,
    /// Gemini CLI, OpenCode, …) as a child process. The global `--json`
    /// flag is intentionally ignored — the protocol IS the structured
    /// output.
    Mcp(McpArgs),
    /// Inspect or create per-project workspaces.
    Workspace(WorkspaceArgs),
    /// Comprehensive diagnostic. Reports every subsystem (workspace,
    /// models, index, drift, catalog caches, harnesses), classifies
    /// overall health, and lists suggested fixes. With `--fix`,
    /// applies the three safe repair classes (re-download models,
    /// re-clone broken catalog caches, forward-migrate the schema).
    Doctor(DoctorArgs),
    /// Inspect and manage harness integrations across ~16 coding harnesses
    /// (Claude Code, Codex, Cursor, Gemini, OpenCode, Copilot, Cline, Zed,
    /// and more). Run with no subcommand to enumerate every supported harness.
    Harness(HarnessArgs),
    /// Author, convert, and validate standalone skills. `create` scaffolds a
    /// new skill (wrapped in a minimal plugin by default; `--bare` for a
    /// naked one), `convert` turns a foreign skill into the native format,
    /// and `lint` validates a Tome skill for CI.
    #[command(subcommand)]
    Skill(SkillCommand),
    /// Install Tome's own bundled "meta skills" — native `SKILL.md` guides
    /// that teach an agent how to use Tome — into your detected harnesses.
    #[command(subcommand)]
    Meta(MetaCommand),
    /// Inspect and control local-first usage telemetry. Telemetry is opt-out
    /// (CI auto-disabled); `status` reports the current state, `on`/`off`
    /// toggle it, and `reset`/`purge` manage the local install identity.
    #[command(subcommand)]
    Telemetry(TelemetryCommand),
    /// Set, list, or clear the per-workspace routing tier of enabled skills and
    /// commands. Tiers drive what instructions Tome injects so an agent knows
    /// when to fetch a skill (Tier 1/2 via get_skill) or search (Tier 3, the
    /// default). Operates on the resolved workspace (use --workspace to target
    /// another).
    #[command(subcommand)]
    Tier(TierCommand),
    /// Propagate workspace state to bound projects: write `.tome/RULES.md` and
    /// reconcile harness files. Defaults to the current project; `--all` fans
    /// out to every bound project in the resolved workspace.
    Sync(SyncArgs),
    /// Inspect and validate the unified global config (`~/.tome/config.toml`).
    /// `show` prints every curated knob's effective value plus a
    /// `(default)`/`(config)`/`(env)` provenance annotation; `validate` runs
    /// the strict parse and reports success or the legible error (exit 5).
    /// Both are read-only. Setting values (`config set`) is a fast-follow.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Print the exit-code reference: every code with its `--json` error
    /// `category` and a one-line meaning, or one code's row
    /// (`tome exit-codes 50`). A pure static lookup over the closed error
    /// set — it needs no HOME, index, config, or lock. Honours the global
    /// `--json` flag.
    ExitCodes(ExitCodesArgs),
    /// Generate a shell completion script (bash/zsh/fish/powershell/elvish) on
    /// stdout. This is a pure static operation over the command tree — it needs
    /// no HOME, index, or config, so it works during shell setup before Tome is
    /// configured. Example: `tome completions zsh > ~/.zfunc/_tome`.
    Completions(CompletionsArgs),
}

/// `tome exit-codes [<code>]` — the exit-code reference table.
#[derive(Debug, clap::Args)]
pub struct ExitCodesArgs {
    /// Show only this code's row. An unknown code is a usage error (exit 2)
    /// pointing at the full table.
    pub code: Option<i32>,
}

/// `tome init` — the guided setup wizard. Deliberately flag-free: state
/// detection replaces flags (a re-run offers only the outstanding steps),
/// and the non-interactive equivalents are the individual commands.
#[derive(Debug, clap::Args)]
pub struct InitArgs {}

/// `tome completions <shell>` — emit a completion script for the named shell.
#[derive(Debug, clap::Args)]
pub struct CompletionsArgs {
    /// The shell to generate completions for. `clap_complete::Shell` is itself a
    /// `ValueEnum` (bash/zsh/fish/powershell/elvish), so an unknown shell is a
    /// parse-time usage error (exit 2) that lists the valid values.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

/// `tome config <subcommand>` — inspect and validate `~/.tome/config.toml`.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print every curated config knob with its effective value and a
    /// `(default)`/`(config)`/`(env)` provenance annotation. Read-only.
    /// Output honours the global `--json` flag (`tome config show --json`
    /// emits the same knobs as a machine-readable document).
    Show(ConfigShowArgs),
    /// Run the strict config parse. Prints "config is valid" and exits 0 on a
    /// good (or absent) config; on a malformed config, prints the legible
    /// key-naming error to stderr and exits 5 (`manifest_invalid`). Read-only.
    Validate,
}

#[derive(Debug, clap::Args)]
pub struct ConfigShowArgs {
    // No subcommand-specific flags yet — `--json` is the global flag.
}

/// `tome sync` — unified propagation of workspace state to bound projects.
/// Composes the per-project RULES.md write with the harness-file reconcile.
/// Replaces the former `tome workspace sync` / `tome harness sync`
/// subcommands, which were removed pre-launch.
#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    /// Sync every bound project in the resolved workspace, not just the current project.
    #[arg(long)]
    pub all: bool,
    /// Only write `.tome/RULES.md` (skip the harness reconcile).
    #[arg(long, conflicts_with = "harness_only")]
    pub rules_only: bool,
    /// Only reconcile harness files (skip the RULES.md write).
    #[arg(long)]
    pub harness_only: bool,
    /// Restrict the harness reconcile to one or more harnesses (repeatable:
    /// `--harness a --harness b`). Ignored with --rules-only. Errors on an
    /// unknown name. Aliases resolve to their canonical module; empty (the
    /// default) reconciles the full effective set.
    #[arg(long, value_name = "NAME", action = clap::ArgAction::Append)]
    pub harness: Vec<String>,
    /// Preview: compute and print exactly what this sync would change, write
    /// nothing, and exit 0. Combines with every other flag.
    #[arg(long)]
    pub dry_run: bool,
}

/// `tome tier <subcommand>` — manage per-workspace skill/command routing tiers.
#[derive(Debug, Subcommand)]
pub enum TierCommand {
    /// Set an entry's routing tier (1, 2, or 3) in the resolved workspace.
    Set(TierSetArgs),
    /// List every enabled skill/command grouped by routing tier.
    List(TierListArgs),
    /// Reset an entry's routing tier to the default (3).
    Clear(TierClearArgs),
}

#[derive(Debug, clap::Args)]
pub struct TierSetArgs {
    /// The entry to retier, as `<plugin>/<name>`, followed by the tier (1, 2,
    /// or 3): `tome tier set my-plugin/my-skill 1`. The name segment may be a
    /// `*` glob (`<plugin>/*` retiers every entry of the plugin;
    /// `<plugin>/foo-*` retiers a subset). Mutually exclusive with `--plugin`;
    /// pick exactly one of the two. With `--plugin`, the tier is the only
    /// positional.
    // #441: the paragraph above is USER-VISIBLE `--help` text — keep it
    // usage-oriented. The engineering rationale: clap sees these as two
    // optional trailing positionals so the tier can be the only one when
    // `--plugin` is present; the id/tier split, the XOR with `--plugin`, and
    // the tier's 1..=3 range are all validated at runtime
    // (missing/both/out-of-range → usage exit 2), preserving the pre-#317
    // exit codes for the single-id form.
    #[arg(value_name = "ID", num_args = 0..=2)]
    pub positionals: Vec<String>,
    /// Retier every enabled tierable entry of a plugin, selected as
    /// `<catalog>/<plugin>` (or a bare `<plugin>` disambiguated by `--catalog`).
    /// Repeatable (`--plugin a --plugin b`). A `*` glob in either segment fans
    /// out across matching catalogs/plugins. Mutually exclusive with the
    /// positional entry id.
    #[arg(long, action = clap::ArgAction::Append)]
    pub plugin: Vec<String>,
    /// Disambiguate when the same plugin name exists across catalogs.
    #[arg(long)]
    pub catalog: Option<String>,
    /// Disambiguate a skill vs command with the same name.
    #[arg(long, value_enum)]
    pub kind: Option<TierKindArg>,
}

#[derive(Debug, clap::Args)]
pub struct TierListArgs {}

#[derive(Debug, clap::Args)]
pub struct TierClearArgs {
    /// The entry to reset, as `<plugin>/<name>`. The name segment may be a `*`
    /// glob (`<plugin>/*`, `<plugin>/foo-*`). Mutually exclusive with `--plugin`
    /// and `--all`; exactly one selection source is required.
    #[arg(conflicts_with_all = ["plugin", "all"], required_unless_present_any = ["plugin", "all"])]
    pub id: Option<String>,
    /// Reset every enabled tierable entry of a plugin, selected as
    /// `<catalog>/<plugin>` (or a bare `<plugin>` disambiguated by `--catalog`).
    /// Repeatable; a `*` glob in either segment fans out. Mutually exclusive with
    /// the positional entry id and `--all`.
    #[arg(long, action = clap::ArgAction::Append, conflicts_with = "all")]
    pub plugin: Vec<String>,
    /// Reset EVERY enabled tierable entry in the resolved workspace to the
    /// default tier (3). Mutually exclusive with the positional id and `--plugin`.
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub catalog: Option<String>,
    #[arg(long, value_enum)]
    pub kind: Option<TierKindArg>,
}

/// CLI-facing entry-kind selector.
///
/// Shared between the tier commands (`tome tier set`/`clear`, where it
/// disambiguates a `<plugin>/<name>` collision) and `tome query --kind`. Tiers
/// never apply to agents — `tiered_entries_for_workspace` hard-filters to
/// `('skill', 'command')` — so passing `--kind agent` to a tier command simply
/// resolves to zero matches (`EntryNotFound`); the variant exists for the query
/// surface, which does filter on `agent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TierKindArg {
    Skill,
    Command,
    Agent,
}

/// `tome telemetry <subcommand>` — control the local-first telemetry subsystem.
#[derive(Debug, Subcommand)]
pub enum TelemetryCommand {
    /// Report telemetry state: enabled + why, install UUID (if any), the
    /// delivery endpoint, queued-event count, and last-flush stamp. Read-only —
    /// never mints an install id.
    Status,
    /// Pretty-print the pending event queue WITHOUT sending it. Read-only — the
    /// queue file is byte-identical after (the flusher self-heals; inspect never
    /// repairs). Reports any corrupt/unparsable lines; exits 92 if any exist.
    Inspect,
    /// Enable telemetry (sets the opt-out switch on) and ensure an install
    /// identity exists.
    On,
    /// Disable telemetry. The install UUID is left intact; a later `on` resumes
    /// it. Use `purge` to also delete the identity.
    Off,
    /// Sever telemetry continuity: mint a fresh install UUID and clear the
    /// queue. Prompts for confirmation unless `--yes`.
    Reset(TelemetryResetArgs),
    /// Delete all telemetry state (install UUID + queue) and switch telemetry
    /// off until explicitly re-enabled.
    Purge,
    /// Drain the pending event queue to the collector in the FOREGROUND and
    /// report the outcome. Exits 90 (`TelemetryEndpointUnreachable`) if the
    /// endpoint is unreachable. The detached background flusher invokes this with
    /// `--quiet` (no output, always exit 0).
    Flush(TelemetryFlushArgs),
}

#[derive(Debug, clap::Args)]
pub struct TelemetryResetArgs {
    /// Skip the confirmation prompt. `--force` is accepted as a hidden alias
    /// so the non-interactive spelling is consistent across commands (FR-021).
    #[arg(long, alias = "force")]
    pub yes: bool,
}

#[derive(Debug, clap::Args)]
pub struct TelemetryFlushArgs {
    /// Suppress all output and always exit 0 (used by the detached child).
    #[arg(long)]
    pub quiet: bool,
}

/// `tome meta <subcommand>` — manage Tome's bundled meta skills.
#[derive(Debug, Subcommand)]
pub enum MetaCommand {
    /// List the bundled meta skills and their per-harness install status.
    List(MetaListArgs),
    /// Install a bundled meta skill into detected (or `--harness`-named)
    /// harnesses at project (default) or `--global` scope.
    Add(MetaAddArgs),
    /// Remove an installed meta skill from the selected harnesses.
    Remove(MetaRemoveArgs),
}

#[derive(Debug, clap::Args)]
pub struct MetaListArgs {}

#[derive(Debug, clap::Args)]
pub struct MetaAddArgs {
    /// The bundled skill ids (variadic, e.g. `convert-marketplace`).
    /// Mutually exclusive with `--all`.
    #[arg(conflicts_with = "all")]
    pub skill_ids: Vec<String>,
    /// Install EVERY bundled meta skill. Mutually exclusive with explicit ids.
    #[arg(long)]
    pub all: bool,
    /// Target a specific harness (repeatable: `--harness a --harness b`).
    /// Default: every detected harness that consumes native skills. The field
    /// name matches `sync`'s `--harness` selector; the CLI flag stays
    /// `--harness`.
    #[arg(long = "harness")]
    pub harness: Vec<String>,
    /// Install into the user-level skills dir instead of the project.
    #[arg(long)]
    pub global: bool,
    /// Re-write even when the on-disk copy is already at the current revision.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct MetaRemoveArgs {
    /// The bundled skill ids (variadic). Mutually exclusive with `--all`.
    #[arg(conflicts_with = "all")]
    pub skill_ids: Vec<String>,
    /// Remove EVERY installed meta skill. Mutually exclusive with explicit ids.
    #[arg(long)]
    pub all: bool,
    /// Target a specific harness (repeatable: `--harness a --harness b`).
    /// Default: every detected harness that consumes native skills. The field
    /// name matches `sync`'s `--harness` selector; the CLI flag stays
    /// `--harness`.
    #[arg(long = "harness")]
    pub harness: Vec<String>,
    /// Remove from the user-level skills dir instead of the project.
    #[arg(long)]
    pub global: bool,
}

/// `tome skill <subcommand>` — the third artifact level (skills have no other
/// top-level command; `plugin show` still surfaces them read-only).
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Scaffold a new skill from a template. Wraps the skill in a minimal
    /// plugin (`<P>:<NAME>`) by default; `--bare` emits a naked
    /// `<NAME>/SKILL.md`.
    Create(SkillCreateArgs),
    /// Convert a foreign skill (native `SKILL.md` from Claude Code, Cursor,
    /// OpenCode, Cline, or a generic Agent Skill) into a native Tome skill.
    Convert(ConvertArgs),
    /// Validate a Tome skill: manifest/structure correctness and residual
    /// harness-specific leftovers. CI-ready exit codes.
    Lint(LintArgs),
}

/// Wraps the `harness` subcommand so the `command` field can be `None` —
/// allowing bare `tome harness` to list every supported harness in
/// tabular form.
#[derive(Debug, clap::Args)]
pub struct HarnessArgs {
    #[command(subcommand)]
    pub command: Option<HarnessCommand>,
}

#[derive(Debug, Subcommand)]
pub enum HarnessCommand {
    /// List the effective harness list for the resolved project, or the
    /// directly-declared list for a named workspace. With no `<workspace>`
    /// argument: computes the effective list via the layered settings
    /// walk + composition expansion. With a `<workspace>` argument:
    /// reports that workspace's directly-declared list verbatim.
    List(HarnessListArgs),
    /// Append a harness to the chosen scope's settings file. Default
    /// scope is `project`. Runs the sync algorithm when the effective
    /// list changes.
    Use(HarnessUseArgs),
    /// Remove a harness from the chosen scope's settings file. Runs
    /// the cleanup pass when the effective list changes.
    Remove(HarnessRemoveArgs),
    /// Report per-harness details for the current project: detection,
    /// targets, integration state, and source-of-scope. Also prints the
    /// paste-able Tome MCP-config snippet for harnesses with a manual MCP
    /// setup (e.g. JetBrains AI, Pi).
    Info(HarnessInfoArgs),
    /// Preview what `harness sync` would deliver vs drop for one harness,
    /// per enabled entry (agents native/persona/unrepresented + dropped
    /// model/tools, skills/commands MCP-routing, hooks native vs GUARDRAILS).
    /// Read-only; no files are touched.
    Preview(HarnessPreviewArgs),
    /// Reconcile the project, then print the workspace's skill-routing directive
    /// to stdout, generated fresh from live state. Intended as a SessionStart
    /// hook target; not usually run by hand.
    // #437: hidden from `tome harness --help` — this is an internal dispatch
    // target the sync writes into harness hook files, not a user-facing verb.
    // `hide` only removes it from the listing; it stays invocable by name.
    #[command(hide = true)]
    SessionStart(HarnessSessionStartArgs),
    /// Translate a plugin hook event from the target harness's native format,
    /// run the enabled plugins' matching hooks, and emit the harness's wire
    /// decision. A hook-dispatch target; not run by hand. Fails open.
    // #437: hidden for the same reason as `SessionStart` above.
    #[command(hide = true)]
    RunHook(HarnessRunHookArgs),
}

#[derive(Debug, clap::Args)]
pub struct HarnessListArgs {
    /// Optional workspace name. When omitted, reports the effective list
    /// for the current project. When present, reports the workspace's
    /// directly-declared list verbatim.
    pub workspace: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct HarnessUseArgs {
    /// Harness names (variadic). With NO names and without `--all`, every
    /// auto-detected harness is configured. With names, exactly those are
    /// configured (aliases + opt-in targets resolve by name). Mutually
    /// exclusive with `--all`.
    ///
    /// NOTE: `use`/`remove` take the multi-select as a bare POSITIONAL, the
    /// deliberate exception (#315) to the `--harness` flag used by `sync` and
    /// `meta add`/`remove`. A positional reads naturally for the "configure
    /// these harnesses" verb; the flag suits commands where the harness is a
    /// filter on some other subject.
    #[arg(conflicts_with = "all")]
    pub names: Vec<String>,
    /// Configure every supported (auto-detectable) harness, regardless of
    /// detection. Excludes the opt-in `generic` / `generic-op` targets unless
    /// `--include-opt-in` is also given. Mutually exclusive with explicit names.
    #[arg(long)]
    pub all: bool,
    /// Together with `--all`, ALSO configure the opt-in write targets
    /// (`generic` / `generic-op`) that `--all` skips by default. Only
    /// meaningful with `--all` (requires it); to configure a single opt-in
    /// target, name it explicitly instead.
    ///
    /// `conflicts_with = "names"` (alongside `requires = "all"`) makes
    /// `--include-opt-in <name>` a LOUD usage error rather than a silent
    /// no-op: with explicit names present, `names` already `conflicts_with`
    /// `all`, so clap treats this flag's `requires = "all"` as
    /// unsatisfiable-and-skipped instead of an error — the flag would then do
    /// nothing with no diagnostic (the exact anti-pattern #306 is about).
    #[arg(long, requires = "all", conflicts_with = "names")]
    pub include_opt_in: bool,
    /// Settings scope to edit. When omitted, falls back to
    /// `[harness] default_scope` in `~/.tome/config.toml`, then to
    /// `project` (requires a project marker above CWD; use `workspace`
    /// or `global` outside a project).
    #[arg(long, value_enum)]
    pub scope: Option<HarnessScopeArg>,
    /// Override a harness-clash on the MCP config write (without it, a
    /// clash exits 19).
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct HarnessRemoveArgs {
    /// Harness names (variadic). Remove each from the chosen scope's settings.
    /// Names need NOT be supported harnesses (a stale/typo'd entry is dropped
    /// too). Mutually exclusive with `--all`.
    #[arg(conflicts_with = "all")]
    pub names: Vec<String>,
    /// Remove EVERY harness configured in the resolved scope (clear the scope's
    /// list). Mutually exclusive with explicit names.
    #[arg(long)]
    pub all: bool,
    /// Settings scope to edit. When omitted, falls back to
    /// `[harness] default_scope` in `~/.tome/config.toml`, then to `project`.
    #[arg(long, value_enum)]
    pub scope: Option<HarnessScopeArg>,
}

#[derive(Debug, clap::Args)]
pub struct HarnessInfoArgs {
    /// Harness name. When omitted, reports one section per harness in the
    /// effective list (the same set `harness list` reports), like
    /// `workspace info`. An unknown explicit name errors `HarnessNotSupported`.
    pub name: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct HarnessPreviewArgs {
    /// Harness name (aliases + opt-in targets resolve by name).
    pub harness: String,
    /// Scope the preview to a single enabled plugin id. When omitted, every
    /// enabled plugin in the resolved workspace is previewed.
    #[arg(long)]
    pub plugin: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct HarnessSessionStartArgs {
    /// Workspace name. Defaults to the resolved scope.
    #[arg(long)]
    pub workspace: Option<String>,
    /// Host harness whose stdout envelope wraps the directive. Absent → emit
    /// the raw directive (the Phase ≤10 claude-code / codex path, unchanged).
    /// An unknown name fails closed (no output). A `CommandHook` harness wraps
    /// the directive in its closed JSON envelope; a `TsPlugin`/`None` harness
    /// receives the raw directive (its shim wraps it).
    #[arg(long)]
    pub harness: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct HarnessRunHookArgs {
    /// The CC event name (PreToolUse, PostToolUse, …).
    #[arg(long)]
    pub event: String,
    /// The host harness (devin, codex, cursor, gemini, copilot-cli).
    #[arg(long)]
    pub harness: String,
    /// Workspace name. Defaults to the resolved scope.
    #[arg(long)]
    pub workspace: Option<String>,
    /// Dry-run: print what WOULD fire (US10), run nothing.
    #[arg(long)]
    pub explain: bool,
}

/// Scope argument for `harness use` and `harness remove`. Distinct from
/// [`crate::workspace::ScopeKind`] (the workspace-info two-state
/// classifier) — adds the project layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum HarnessScopeArg {
    Project,
    Workspace,
    Global,
}

impl std::fmt::Display for HarnessScopeArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Project => "project",
            Self::Workspace => "workspace",
            Self::Global => "global",
        })
    }
}

#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    /// Apply the safe automatic repairs (re-download missing or
    /// corrupt models including the summariser, re-clone broken catalog
    /// caches, forward-migrate the index schema, re-copy the
    /// `<project>/.tome/RULES.md` from the bound workspace, re-run
    /// harness sync for every harness whose rules or MCP config has
    /// drifted). Destructive repairs are never automatic — see
    /// `--force` for the user-owned-MCP override.
    #[arg(long)]
    pub fix: bool,
    /// Override safe-by-default repair gates. Currently rewrites
    /// developer-authored harness MCP `tome` entries on `--fix` (the
    /// clash-overriding harness reconcile). Other
    /// manually-classified issues — notably a binding whose marker
    /// names a missing workspace — are NOT affected by `--force`:
    /// choosing the target workspace is a developer decision.
    #[arg(long)]
    pub force: bool,
    /// Rehash each installed model's primary file against its pinned
    /// SHA-256, and probe each effective harness's registered `tome mcp`
    /// server end-to-end (initialize + tools/list over stdio, bounded by a
    /// timeout). Slower than the default; still network-free. Also enabled
    /// by `[doctor] verify_by_default` in `~/.tome/config.toml`.
    #[arg(long)]
    pub verify: bool,
    /// With --fix: list the automatic repairs `--fix` would apply and exit
    /// without applying anything (the run behaves like the read-only report
    /// on disk). An error without --fix.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceArgs {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// Print the workspace bound to the current directory on one line, with
    /// no decoration — the lightweight companion to `info`/`status` for
    /// shell prompts and scripting (`$(tome workspace current 2>/dev/null)`).
    /// `--json` emits `{"workspace","scope","source"}`. Read-only. Exits
    /// non-zero (12, `WorkspaceNotBound`) with a clear, actionable stderr
    /// message and no stdout when nothing is bound to the current directory.
    Current,
    /// Report one workspace's details (catalogs, enabled plugins, bound
    /// projects, cached summary state). Read-only; never acquires the
    /// advisory lock. `<name>` defaults to the resolved workspace.
    Info(WorkspaceInfoArgs),
    /// Create a new workspace in the central registry. Lands
    /// `<root>/workspaces/<name>/` atomically (settings.toml + RULES.md)
    /// and inserts a row into the `workspaces` table. `--inherit-global`
    /// seeds the new workspace's catalogs from the global workspace's
    /// enrolments at the moment of creation.
    Init(WorkspaceInitArgs),
    /// List every workspace in the central registry with catalog,
    /// enabled-plugin, indexed-skill, bound-project counts plus
    /// `last_used_at`. The workspace resolved for the current directory
    /// is marked in the `Cur` column (`*`). `Last used` renders as a
    /// relative time by default; `--absolute` forces the RFC 3339
    /// timestamp. `--json` carries a per-row `current` bool and always
    /// emits the absolute timestamp.
    List(WorkspaceListArgs),
    /// Rename a workspace. Updates every bound project's marker
    /// `config.toml` atomically, renames `<root>/workspaces/<old>/` to
    /// `<root>/workspaces/<new>/`, and updates the `workspaces.name`
    /// row. Refuses either side of `global`.
    Rename(WorkspaceRenameArgs),
    /// Force regeneration of a workspace's cached short + long
    /// summaries. Writes the result into the workspace's
    /// `settings.toml` `[summaries]` section, rewrites
    /// `<root>/workspaces/<name>/RULES.md`, and copies the new RULES.md
    /// to every bound project's marker copy.
    RegenSummary(WorkspaceRegenSummaryArgs),
    /// Remove a workspace from the central registry. The cascade
    /// removes integration in every bound project, deletes per-workspace
    /// DB rows (`workspace_skills`, `workspace_catalogs`,
    /// `workspace_projects`, `workspaces`) inside one transaction,
    /// deletes the central `<root>/workspaces/<name>/` directory, and
    /// refcount-cleans any catalog clone no longer referenced. Refuses
    /// to remove the reserved `global` workspace (exit 15). Refuses
    /// without `--force` when ≥ 1 project is bound (exit 16).
    Remove(WorkspaceRemoveArgs),
    /// Bind the current project directory to a workspace. Creates /
    /// overwrites `<cwd>/.tome/config.toml` so subsequent Tome invocations
    /// from this tree resolve to `<name>` via the project-marker walk. The
    /// atomic-directory landing means a SIGINT mid-bind never leaves a
    /// partial `.tome/`.
    ///
    /// `--create` creates the workspace (create-if-absent) before binding —
    /// `init` + `use` in one step. Omit `<name>` on a terminal to pick from
    /// a list of existing workspaces; on a non-terminal an omitted name is a
    /// usage error (exit 54).
    ///
    /// Note: the `<name>` argument always takes precedence; the global
    /// `--workspace` flag is ignored for this subcommand.
    Use(WorkspaceUseArgs),
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceRemoveArgs {
    /// Workspace to remove. Refuses the reserved `global` workspace
    /// (exit 15).
    pub name: String,
    /// Cascade removal even when projects are bound to the workspace.
    /// Without `--force`, a non-empty bind list refuses with exit 16
    /// (`WorkspaceHasBoundProjects`) carrying the names of every bound
    /// project path so the user knows what would be torn down.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceUseArgs {
    /// Workspace name. Must already exist in the central registry unless
    /// `--create` is passed (then it is created-if-absent first). Omit the
    /// name on a terminal to pick from a list of existing workspaces.
    pub name: Option<String>,
    /// Create the workspace (create-if-absent) before binding, so `init` +
    /// `use` happen in one step. Requires an explicit `<name>`.
    #[arg(long)]
    pub create: bool,
    /// Bypass the refusal when CWD is the user's home directory or the
    /// filesystem root. Required only for genuinely unusual project roots
    /// (e.g. binding `/` for a system-management workflow).
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceInfoArgs {
    /// Workspace name. Defaults to the resolved scope. Missing names
    /// surface as exit 13 (`WorkspaceNotFound`).
    pub name: Option<String>,
    /// Expand the enabled-plugins section into a per-plugin breakdown of
    /// skills / commands / agents, showing each skill's and command's routing
    /// tier.
    #[arg(long)]
    pub details: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceInitArgs {
    /// Workspace name: 1–64 alphanumeric characters plus hyphen /
    /// underscore, with no leading or trailing punctuation. The privileged
    /// `global` workspace name is reserved.
    pub name: String,
    /// Seed the new workspace's catalogs from the global workspace's
    /// enrolments at the moment of creation. If global has no enrolled
    /// catalogs, the flag is a documented no-op.
    #[arg(long = "inherit-global")]
    pub inherit_global: bool,
    /// Bind the current directory to the freshly-created workspace (the
    /// mirror of `tome workspace use --create`), running harness sync in
    /// the same step.
    #[arg(long)]
    pub bind: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceListArgs {
    /// Render `Last used` as an absolute RFC 3339 timestamp
    /// (e.g. `2026-06-28T10:23:11Z`) instead of the default relative
    /// form (e.g. `2 days ago`). Human output only — `--json` always
    /// emits the absolute unix-second timestamp regardless of this flag.
    #[arg(long)]
    pub absolute: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceRenameArgs {
    /// Existing workspace name. Refuses to rename the reserved `global`
    /// workspace (exit 15).
    pub old: String,
    /// New workspace name. Must satisfy the workspace naming rule; must not
    /// collide with an existing workspace (exit 14); cannot be the reserved
    /// `global` (exit 15).
    pub new: String,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceRegenSummaryArgs {
    /// Workspace to regenerate summaries for. Defaults to the resolved
    /// scope when omitted, but only after an interactive confirmation — so
    /// the user can't accidentally regenerate the resolved scope (often
    /// `global`) by forgetting an argument. On a non-terminal, the name is
    /// required (exit 54): we never silently regenerate the resolved scope.
    pub name: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct McpArgs {
    /// The harness hosting this MCP server (claude-code, cursor, codex,
    /// opencode). Conveys host identity to the built-in `meta` tool so it
    /// can install skills into the right harness. Normally stamped
    /// automatically by `tome sync`; absent for a legacy config.
    #[arg(long)]
    pub harness: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct StatusArgs {
    /// Report on this workspace instead of the resolved scope (defaults to
    /// the resolved scope). Mirrors `workspace info [<name>]`. Must already
    /// exist in the central registry (missing → exit 13).
    // The Rust field is `name` (not `workspace`) deliberately: clap keys an arg
    // on its field name, so naming it `workspace` would collide with the global
    // `-w`/`--workspace` flag's id and shadow it on `tome status`. This matches
    // `WorkspaceInfoArgs.name`, keeping `tome status -w <name>` working.
    #[arg(value_name = "WORKSPACE")]
    pub name: Option<String>,

    /// Rehash each installed model's primary file against its pinned
    /// SHA-256. Slower (several seconds for the reranker), but catches
    /// silent on-disk corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct ReindexArgs {
    /// Scopes. Omit ALL of `<scope>`, `--catalog`, and `--plugin` to reindex
    /// every enabled plugin (the whole-index form — the only form that restamps
    /// the global embedder identity). Each positional token is one of:
    /// `<catalog>` (every enabled plugin in that catalog), `<catalog>/<plugin>`
    /// (one plugin), or a `*` glob (`<catalog>/*`, `<catalog>/compact-*`, or a
    /// bare `mycat-*` matching enrolled catalog names). Multiple tokens are
    /// unioned. Issue #316.
    #[arg(num_args = 0..)]
    pub scopes: Vec<String>,
    /// Reindex every enabled plugin in the named catalog(s). Repeatable; a `*`
    /// glob matches enrolled catalog names. Cannot be combined with positional
    /// `<scope>` tokens (use one form or the other), but MAY be combined with
    /// `--plugin` (their targets union).
    #[arg(long, action = clap::ArgAction::Append, conflicts_with = "scopes")]
    pub catalog: Vec<String>,
    /// Reindex the named `<catalog>/<plugin>`. Repeatable; a `*` glob is allowed
    /// in the plugin segment (`<catalog>/compact-*`). Cannot be combined with
    /// positional `<scope>` tokens, but MAY be combined with `--catalog`.
    #[arg(long, action = clap::ArgAction::Append, conflicts_with = "scopes")]
    pub plugin: Vec<String>,
    /// Re-embed every in-scope skill regardless of `content_hash`. Used for
    /// embedder upgrades and integrity recovery.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// Download the active profile's models if missing. `--all` fetches every
    /// registered model; `--force` re-downloads even when the on-disk manifest
    /// already records a complete install.
    Download(ModelsDownloadArgs),
    /// List every registered model with its on-disk state, the profile(s) that
    /// reference it, and which set the active profile selects. `--verify`
    /// rehashes each installed model against its pinned SHA-256.
    List(ModelsListArgs),
    /// Remove an installed model directory and its manifest.
    Remove(ModelsRemoveArgs),
    /// Show or set the active model profile (small/medium/large). The profile
    /// selects which embedder + reranker Tome uses; the summariser is shared
    /// across every profile. Omit `<tier>` to show the current profile.
    Profile(ModelsProfileArgs),
    /// Run ONE real round-trip against the active model for a capability
    /// (the configured remote provider, else the bundled local model) and
    /// report whether it succeeded. Read-only — writes no stored state.
    /// `tome models test embedding` embeds a fixed string and validates the
    /// vector; `summariser` summarises a tiny input; `reranker` reranks a
    /// small candidate set. Honours `--json`.
    Test(ModelsTestArgs),
    /// Bring local model assets up to date. Ensures the active profile's
    /// models are present (re-downloading any missing). `--include-registry`
    /// also refreshes the harness model-id registry override from models.dev.
    Update(ModelsUpdateArgs),
}

/// Which model capability `tome models test` exercises. Each value drives a
/// distinct round-trip + success assertion (see `commands::models::test`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TestCapability {
    /// Summarise a tiny fixed input; success = non-empty short AND long.
    Summariser,
    /// Embed a fixed string; success = non-empty, finite, matching dimension.
    Embedding,
    /// Rerank a small fixed candidate set; success = a scored ordering.
    Reranker,
}

#[derive(Debug, clap::Args)]
pub struct ModelsTestArgs {
    /// The capability to test: `summariser`, `embedding`, or `reranker`.
    #[arg(value_enum)]
    pub capability: TestCapability,
    /// After the live round-trip, ALSO rehash the active bundled model's
    /// on-disk primary artefact against its pinned SHA-256 (the same check
    /// `status`/`doctor`/`models list` perform under `--verify`). A no-op for a
    /// capability configured to use a remote provider (there is no on-disk
    /// artefact to verify). Slower (several seconds for a large reranker) but
    /// catches silent on-disk corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct ModelsDownloadArgs {
    /// Re-download even when the on-disk manifest records a complete install.
    #[arg(long)]
    pub force: bool,
    /// Download every registered model, not just the active profile's set.
    #[arg(long)]
    pub all: bool,
    /// Download the models for a SPECIFIC profile tier (small/medium/large)
    /// instead of the active one — WITHOUT changing the stored active profile.
    /// Mutually exclusive with `--all` (which already spans every tier). Useful
    /// to pre-fetch another tier's weights before switching to it.
    #[arg(long, value_enum, conflicts_with = "all")]
    pub profile: Option<crate::embedding::Profile>,
}

#[derive(Debug, clap::Args)]
pub struct ModelsProfileArgs {
    /// Set the active model profile. Omit to show the current profile.
    #[arg(value_enum)]
    pub tier: Option<crate::embedding::Profile>,
}

#[derive(Debug, clap::Args)]
pub struct ModelsListArgs {
    /// Rehash each installed file's contents against its pinned SHA-256.
    /// Slower (several seconds for the reranker) but catches silent
    /// on-disk corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct ModelsRemoveArgs {
    /// The registered model names (variadic, e.g. `bge-small-en-v1.5`).
    /// Mutually exclusive with `--all`.
    #[arg(conflicts_with = "all")]
    pub names: Vec<String>,
    /// Evict EVERY installed model. Mutually exclusive with explicit names.
    #[arg(long)]
    pub all: bool,
    /// Skip the confirmation prompt. Required when stdin is not a TTY.
    /// `--force` is accepted as a hidden alias for back-compat.
    // #438: removal is prompted, not gated — the flag only skips the
    // confirmation (models re-download on demand), so `--yes` is the primary
    // spelling; `--force` is reserved for safety-behaviour overrides.
    #[arg(long, alias = "force")]
    pub yes: bool,
}

#[derive(Debug, clap::Args)]
pub struct ModelsUpdateArgs {
    /// Also refresh the model-id registry override (~/.tome/cache/model-registry.json)
    /// by fetching the latest data from models.dev.
    #[arg(long)]
    pub include_registry: bool,
}

#[derive(Debug, Subcommand)]
pub enum CatalogCommand {
    /// Register a remote catalog.
    Add(CatalogAddArgs),
    /// Remove a registered catalog.
    Remove(CatalogRemoveArgs),
    /// List registered catalogs.
    List(CatalogListArgs),
    /// Refresh one or every registered catalog.
    Update(CatalogUpdateArgs),
    /// Show the manifest and registration metadata for a catalog.
    Show(CatalogShowArgs),
    /// Scaffold a new catalog from a template.
    Create(CatalogCreateArgs),
    /// Convert a Claude Code marketplace into a native Tome catalog (a copy;
    /// the source is never modified).
    Convert(CatalogConvertArgs),
    /// Validate a Tome catalog (and every plugin/skill it nests). CI-ready.
    Lint(LintArgs),
}

#[derive(Debug, clap::Args)]
pub struct CatalogAddArgs {
    /// The catalog source: an owner/repo shorthand (optionally prefixed
    /// `gh:`/`gl:`/`bb:` for GitHub/GitLab/Bitbucket), a Git URL, or a local
    /// path (interpreted as `file://`).
    pub source: String,
    /// Override the display name (defaults to the manifest's `name`).
    #[arg(short = 'n', long)]
    pub name: Option<String>,
    /// Branch, tag, or SHA to track (aliases: `--branch`, `--tag`). Defaults
    /// to `main`.
    #[arg(long = "ref", visible_alias = "branch", visible_alias = "tag")]
    pub ref_: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct CatalogRemoveArgs {
    /// The catalog display name to remove.
    pub name: String,
    /// Skip the confirmation prompt. Required when stdin is not a TTY.
    /// `--yes` is accepted as a hidden alias (FR-021).
    #[arg(long, alias = "yes")]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct CatalogListArgs {
    // No flags yet — `--json` is global.
}

#[derive(Debug, clap::Args)]
pub struct CatalogUpdateArgs {
    /// The catalog to refresh. Omit to refresh every registered catalog.
    pub name: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct CatalogShowArgs {
    /// The catalog display name to inspect.
    pub name: String,
}

/// Wraps the `plugin` subcommand so the `command` field can be `None` —
/// allowing bare `tome plugin` to drop into the interactive flow.
#[derive(Debug, clap::Args)]
pub struct PluginArgs {
    #[command(subcommand)]
    pub command: Option<PluginCommand>,
}

#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Enable a plugin: index its skills and start surfacing them in queries.
    Enable(PluginEnableArgs),
    /// Disable a plugin: hide its skills from queries while retaining the
    /// embeddings on disk so re-enable is cheap.
    Disable(PluginDisableArgs),
    /// List plugins discoverable across every registered catalog.
    List(PluginListArgs),
    /// Show one plugin's metadata, component counts, and index status.
    Show(PluginShowArgs),
    /// Scaffold a new plugin from a template.
    Create(PluginCreateArgs),
    /// Convert a Claude Code plugin (or a Codex project) into a native Tome
    /// plugin (a copy; the source is never modified).
    Convert(ConvertArgs),
    /// Validate a Tome plugin (and every skill it nests). CI-ready.
    Lint(LintArgs),
}

#[derive(Debug, clap::Args)]
pub struct PluginEnableArgs {
    /// One or more plugins as `<catalog>/<plugin>`. The plugin segment (or a
    /// bare name with `--catalog`) may contain `*` wildcards, e.g.
    /// `midnight-expert/compact-*` or `midnight-expert/*`.
    #[arg(required = true, num_args = 1..)]
    pub ids: Vec<String>,
    /// Scope bare or wildcard names to one catalog. A slash-qualified
    /// `<catalog>/<plugin>` id ignores this (the explicit catalog wins).
    #[arg(long)]
    pub catalog: Option<String>,
    /// Skip the model-download confirmation prompt. Required to enable a
    /// plugin from a non-interactive context (e.g. CI) when models are
    /// not yet installed. `--force` is accepted as a hidden alias so the
    /// non-interactive spelling is consistent across commands (FR-021).
    #[arg(long, alias = "force")]
    pub yes: bool,
    /// Routing tier (1|2|3) to apply to ALL of this plugin's skills and
    /// commands at enable time. Omitted → the default tier 3. Refine
    /// per-entry later with `tome tier set`.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=3))]
    pub tier: Option<u8>,
    /// Apply the change to your harnesses immediately: after enabling, run the
    /// same propagation `tome sync` performs over every project bound to the
    /// resolved workspace (write `.tome/RULES.md` and reconcile harness files).
    /// Without it, enable only updates the index and prints a reminder to run
    /// `tome sync`.
    #[arg(long)]
    pub sync: bool,
}

#[derive(Debug, clap::Args)]
pub struct PluginDisableArgs {
    /// One or more plugins as `<catalog>/<plugin>`. The plugin segment (or a
    /// bare name with `--catalog`) may contain `*` wildcards, e.g.
    /// `midnight-expert/compact-*` or `midnight-expert/*`.
    #[arg(required = true, num_args = 1..)]
    pub ids: Vec<String>,
    /// Scope bare or wildcard names to one catalog. A slash-qualified
    /// `<catalog>/<plugin>` id ignores this (the explicit catalog wins).
    #[arg(long)]
    pub catalog: Option<String>,
    /// Skip the confirmation prompt. Required to disable a plugin from a
    /// non-interactive context (e.g. CI). `--force` is accepted as a hidden
    /// alias for back-compat.
    // #438: disabling only skips a confirmation (nothing destructive is
    // overridden — embeddings stay on disk), so `--yes` is the primary
    // spelling; `--force` is reserved for safety-behaviour overrides
    // (`catalog remove`'s cascade, `doctor`'s repair gates, …).
    #[arg(long, alias = "force")]
    pub yes: bool,
    /// Apply the change to your harnesses immediately: after disabling, run the
    /// same propagation `tome sync` performs over every project bound to the
    /// resolved workspace (write `.tome/RULES.md` and reconcile harness files).
    /// Without it, disable only updates the index and prints a reminder to run
    /// `tome sync`.
    #[arg(long)]
    pub sync: bool,
}

#[derive(Debug, clap::Args)]
pub struct PluginListArgs {
    /// Restrict the listing to one catalog.
    #[arg(long)]
    pub catalog: Option<String>,
    /// Hide disabled and unindexable plugins.
    #[arg(long = "enabled-only")]
    pub enabled_only: bool,
    /// Keep only plugins whose name OR description contains this substring
    /// (case-insensitive). Composes with `--catalog`, `--enabled-only`, and
    /// `--tier`.
    #[arg(long)]
    pub filter: Option<String>,
    /// Keep only plugins with at least one enabled entry (skill / command /
    /// agent) routed at this tier (1, 2, or 3). Composes with `--filter`,
    /// `--catalog`, and `--enabled-only`.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=3))]
    pub tier: Option<u8>,
}

#[derive(Debug, clap::Args)]
pub struct PluginShowArgs {
    /// The plugin to inspect, as `<catalog>/<plugin>`.
    pub id: String,
    /// Annotate each per-entry line (skills / commands / agents) with its
    /// routing tier. Without it the output — human and `--json` — is
    /// unchanged.
    #[arg(long)]
    pub details: bool,
}

#[derive(Debug, clap::Args)]
pub struct QueryArgs {
    /// The query text to search for, as one or more positional words. Multiple
    /// words are joined with a single space, so `tome query reset a counter`
    /// works unquoted. Embedded as-is — no name/description composition is
    /// applied. Mutually exclusive with `-q`/`--query`; when neither is given
    /// the command exits with a usage error.
    #[arg(value_name = "QUERY", num_args = 0..)]
    pub text: Vec<String>,

    /// The query text as a single (already-quoted) string — an alternative to
    /// the positional words for when the query itself contains flag-like or
    /// shell-significant tokens. Mutually exclusive with the positional form.
    #[arg(
        short = 'q',
        long = "query",
        value_name = "QUERY",
        conflicts_with = "text"
    )]
    pub query: Option<String>,

    /// Cap on returned results (post-rerank when reranking).
    /// When absent, falls back to `[query] top_k` in `~/.tome/config.toml`,
    /// then to the built-in default of 10.
    #[arg(long = "top-k")]
    pub top_k: Option<u32>,

    /// Restrict the search to one or more catalogs (repeatable). Results are
    /// limited to entries whose catalog is any of the given names. A single
    /// `--catalog x` behaves exactly as before.
    #[arg(long, action = clap::ArgAction::Append)]
    pub catalog: Vec<String>,

    /// Restrict the search to one or more plugins (repeatable, across all
    /// enabled catalogs unless `--catalog` is also set). Results are limited to
    /// entries whose plugin is any of the given names.
    #[arg(long, action = clap::ArgAction::Append)]
    pub plugin: Vec<String>,

    /// Restrict the search to one or more entry kinds (`skill`, `command`, or
    /// `agent`; repeatable). Note that `query` only ever searches indexed,
    /// searchable entries, so `--kind agent` typically returns nothing (agents
    /// are not searchable).
    #[arg(long, value_enum, action = clap::ArgAction::Append)]
    pub kind: Vec<TierKindArg>,

    /// Skip the reranker stage; scores are cosine similarity.
    #[arg(long = "no-rerank")]
    pub no_rerank: bool,

    /// Apply the score threshold and exit non-zero on empty result.
    #[arg(long)]
    pub strict: bool,

    /// Minimum score to retain a result (only enforced with `--strict`).
    /// Default is 0.0 with the reranker on, 0.5 with `--no-rerank`.
    #[arg(long = "min-score")]
    pub min_score: Option<f32>,
}

// ---------------------------------------------------------------------------
// Phase 8 — authoring & conversion (`create` / `convert` / `lint`).
//
// `--json` is the global flag (on `Cli`), so it is intentionally NOT redefined
// per command. The three verbs share `ConvertArgs` / `LintArgs` across all
// three artifact levels; only `create` differs per level (catalog has no
// `--into`; skill adds `--bare` + `--plugin-name`). Mutually-exclusive flags
// (`--output`/`--into`, `--template`/`--bare`) are enforced by clap
// `conflicts_with` so the usage error is caught at parse time (exit 2).
// ---------------------------------------------------------------------------

/// `tome catalog create <NAME>`. No `--into` (a catalog is a top-level tree).
#[derive(Debug, clap::Args)]
pub struct CatalogCreateArgs {
    /// Name of the new catalog; also the created directory name.
    pub name: String,
    /// Template to scaffold from: a reserved built-in name, a local directory,
    /// a git URL, or an `owner/repo` shorthand. Defaults to the built-in.
    #[arg(long)]
    pub template: Option<String>,
    /// Parent directory the new artifact lands under (as `<output>/<NAME>/`).
    /// Defaults to the current directory.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Description for the scaffolded artifact. Omitted → a name-derived
    /// placeholder.
    #[arg(long)]
    pub description: Option<String>,
    /// Author name for the scaffolded catalog owner. Omitted → the `Your Name`
    /// placeholder (edit it after scaffolding).
    #[arg(long)]
    pub author: Option<String>,
    /// Preview the files that would be written without touching the filesystem.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Overwrite colliding files (only those files; never a directory wipe).
    #[arg(long)]
    pub force: bool,
}

/// `tome plugin create <NAME>`.
#[derive(Debug, clap::Args)]
pub struct PluginCreateArgs {
    /// Name of the new plugin; also the created directory name.
    pub name: String,
    /// Template to scaffold from (built-in name, local dir, git URL, or
    /// `owner/repo`). Defaults to the built-in.
    #[arg(long)]
    pub template: Option<String>,
    /// Parent directory the new artifact lands under. Mutually exclusive with
    /// `--into`. Defaults to the current directory.
    #[arg(long, conflicts_with = "into")]
    pub output: Option<PathBuf>,
    /// Inject the new plugin into an existing Tome catalog, registering it in
    /// that catalog's `tome-catalog.toml`. Mutually exclusive with `--output`.
    #[arg(long)]
    pub into: Option<PathBuf>,
    /// Description for the scaffolded artifact. Omitted → a name-derived
    /// placeholder.
    #[arg(long)]
    pub description: Option<String>,
    /// Author name for the scaffolded plugin's `[author]` table. Omitted → no
    /// author is recorded.
    #[arg(long)]
    pub author: Option<String>,
    /// Preview the files that would be written without touching the filesystem.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Overwrite colliding files (only those files; never a directory wipe).
    #[arg(long)]
    pub force: bool,
}

/// `tome skill create <NAME>`. Wraps the skill in a minimal plugin by default.
#[derive(Debug, clap::Args)]
pub struct SkillCreateArgs {
    /// Name of the new skill; also the created skill directory name.
    pub name: String,
    /// Template to scaffold from (built-in name, local dir, git URL, or
    /// `owner/repo`). Defaults to the built-in. Errors with `--bare`.
    #[arg(long, conflicts_with = "bare")]
    pub template: Option<String>,
    /// Emit a naked skill (`<NAME>/SKILL.md`) instead of wrapping it in a
    /// minimal plugin. Alias for `--template bare-skill`.
    #[arg(long)]
    pub bare: bool,
    /// Name of the wrapping plugin (default: `<NAME>`), giving the full skill
    /// name `<plugin-name>:<NAME>`. Meaningless with `--bare` (no wrapping
    /// plugin) or `--into` (the wrapping plugin already exists), so it is a
    /// usage error to combine them rather than silently ignored.
    #[arg(long = "plugin-name", conflicts_with_all = ["bare", "into"])]
    pub plugin_name: Option<String>,
    /// Parent directory the new artifact lands under. Mutually exclusive with
    /// `--into`. Defaults to the current directory.
    #[arg(long, conflicts_with = "into")]
    pub output: Option<PathBuf>,
    /// Inject the new skill into an existing Tome plugin (drops it into the
    /// plugin's `skills/`). Mutually exclusive with `--output`.
    #[arg(long)]
    pub into: Option<PathBuf>,
    /// Description for the scaffolded skill. Omitted → a name-derived
    /// placeholder.
    #[arg(long)]
    pub description: Option<String>,
    /// Author name for the wrapping plugin's `[author]` table. Omitted → no
    /// author is recorded. Meaningless with `--bare` (no wrapping plugin) and
    /// with `--into` (the wrapping plugin already exists).
    #[arg(long)]
    pub author: Option<String>,
    /// Preview the files that would be written without touching the filesystem.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Overwrite colliding files (only those files; never a directory wipe).
    #[arg(long)]
    pub force: bool,
}

/// Shared `convert` arguments across all three artifact levels.
///
/// `--no-fetch` is intentionally NOT here — it only applies to `catalog
/// convert` (marketplace remote-plugin recursion), so it lives on
/// [`CatalogConvertArgs`] alone. `plugin`/`skill convert` therefore reject it at
/// parse time (exit 2) rather than silently accepting an inert flag.
#[derive(Debug, clap::Args)]
pub struct ConvertArgs {
    /// Source to convert: a local path, an `owner/repo` shorthand, or a git
    /// URL. Remote sources are fetched into a temp clone (cleaned up on every
    /// exit path). The source is never modified. Typed as a path (an
    /// `owner/repo`/URL source resolves through the same value); a non-UTF-8
    /// path is rejected with a clear usage error.
    #[arg(value_name = "PATH|REPO|URL", value_hint = clap::ValueHint::DirPath)]
    pub source: PathBuf,
    /// New name for the converted artifact. Defaults to `<source-name>-tome`.
    pub name: Option<String>,
    /// Override source-format detection (closed set): claude-code | codex |
    /// cursor | opencode | cline | agent-skills (aliases: `claude`, `agent`).
    #[arg(long = "from", value_enum)]
    pub from: Option<crate::authoring::detect::SourceHarness>,
    /// Parent directory the converted copy lands under. Mutually exclusive
    /// with `--into`. Defaults to the current directory.
    #[arg(long, conflicts_with = "into")]
    pub output: Option<PathBuf>,
    /// Inject the converted artifact into an existing Tome artifact (type
    /// auto-detected from its manifest). Mutually exclusive with `--output`.
    #[arg(long)]
    pub into: Option<PathBuf>,
    /// Overwrite colliding files (only those files).
    #[arg(long)]
    pub force: bool,
    /// Print the plan; create or modify zero files.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Abort (writing nothing) on anything Tome cannot represent.
    #[arg(long)]
    pub strict: bool,
    /// Demote a rule id out of the `--strict` blocking set (repeatable:
    /// `--allow convert/unsupported-component --allow convert/agent-lossy`). An
    /// allowed rule still emits its normal warning; it just no longer aborts
    /// `--strict`. Naming a non-blocking or unknown rule id is a harmless no-op.
    /// Only meaningful together with `--strict`.
    #[arg(long = "allow", value_name = "RULE-ID")]
    pub allow: Vec<String>,
}

/// `catalog convert` arguments: the shared [`ConvertArgs`] plus `--no-fetch`,
/// which is meaningful only for a marketplace's remote-source plugin recursion.
#[derive(Debug, clap::Args)]
pub struct CatalogConvertArgs {
    #[command(flatten)]
    pub common: ConvertArgs,
    /// Do not fetch the marketplace's remote-source plugins; they are
    /// warned-and-skipped instead. The SOURCE argument itself may still be a
    /// remote clone. `--local-only` is an accepted alias (the same flag under a
    /// non-double-negative name).
    #[arg(long = "no-fetch", visible_alias = "local-only")]
    pub no_fetch: bool,
}

/// Shared `lint` arguments across all three artifact levels.
#[derive(Debug, clap::Args)]
pub struct LintArgs {
    /// One or more Tome artifacts to validate (local paths). At least one is
    /// required; the shell expands a glob (`plugins/*`) into multiple args.
    /// Each source is linted independently (never-halt); the exit code is the
    /// worst verdict across all of them.
    #[arg(value_name = "PATH", value_hint = clap::ValueHint::DirPath, num_args = 1.., required = true)]
    pub sources: Vec<PathBuf>,
    /// Apply mechanically-safe fixes (rewritable harness-isms, `name == dir`);
    /// report fixed vs. still-manual.
    #[arg(long)]
    pub autofix: bool,
    /// Report would-be fixes but change nothing on disk. Requires `--autofix`
    /// (it only qualifies that pass); a bare `--dry-run` is a usage error.
    #[arg(long = "dry-run", requires = "autofix")]
    pub dry_run: bool,
    /// Warnings also cause a non-zero exit (CI-strict).
    #[arg(long)]
    pub strict: bool,
}

impl Cli {
    pub fn verbosity(&self) -> crate::logging::Verbosity {
        crate::logging::Verbosity::from_count(self.verbose)
    }

    pub fn mode(&self) -> crate::output::Mode {
        // Precedence: the `--json` flag wins; when absent, a truthy `TOME_JSON`
        // env var forces JSON. `env_truthy` (the shared SSOT, also used by
        // `--non-interactive`/`TOME_NONINTERACTIVE`) never hard-errors on an
        // unparsable value, unlike clap's boolish `env=` parser — the reason we
        // gate here rather than annotate the flag with `env = "TOME_JSON"`.
        crate::output::Mode::from_flag(self.json || crate::util::env_truthy("TOME_JSON"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Mode;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Issue #423: the outstanding-step selector over the three setup signals.
    /// Each partial state selects exactly its outstanding steps; a fully-set-up
    /// install selects nothing.
    #[test]
    fn outstanding_setup_steps_selects_per_state() {
        let commands = |steps: Vec<&'static SetupStep>| -> Vec<&'static str> {
            steps.into_iter().map(|s| s.command).collect()
        };

        // Fresh install: every step, in SETUP_STEPS order.
        assert_eq!(
            commands(outstanding_setup_steps(0, 0, false)),
            SETUP_STEPS.iter().map(|s| s.command).collect::<Vec<_>>(),
        );
        // Catalogs enrolled, nothing enabled, no harness: enable → harness → query.
        assert_eq!(
            commands(outstanding_setup_steps(2, 0, false)),
            vec![
                SETUP_STEPS[1].command,
                SETUP_STEPS[2].command,
                SETUP_STEPS[3].command,
            ],
        );
        // Everything but the harness: exactly `harness use` (query already works).
        assert_eq!(
            commands(outstanding_setup_steps(2, 3, false)),
            vec![SETUP_STEPS[2].command],
        );
        // Harness configured, nothing enabled: enable → query.
        assert_eq!(
            commands(outstanding_setup_steps(1, 0, true)),
            vec![SETUP_STEPS[1].command, SETUP_STEPS[3].command],
        );
        // Fully set up: nothing.
        assert!(outstanding_setup_steps(1, 1, true).is_empty());
    }

    // Env is process-global. `mode()` reads `TOME_JSON`, so any test that sets
    // it must serialise against every other env-mutating test in this binary.
    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Set/unset `TOME_JSON` for the guard's lifetime, restoring the prior value
    /// on drop. Caller MUST hold `ENV_MUTEX`.
    struct JsonEnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl JsonEnvGuard {
        fn set(value: Option<&str>) -> Self {
            let previous = std::env::var_os("TOME_JSON");
            // SAFETY: caller holds ENV_MUTEX; no other test mutates env.
            unsafe {
                match value {
                    Some(v) => std::env::set_var("TOME_JSON", v),
                    None => std::env::remove_var("TOME_JSON"),
                }
            }
            Self { previous }
        }
    }

    impl Drop for JsonEnvGuard {
        fn drop(&mut self) {
            // SAFETY: ENV_MUTEX is held for the guard's lifetime.
            unsafe {
                match &self.previous {
                    Some(v) => std::env::set_var("TOME_JSON", v),
                    None => std::env::remove_var("TOME_JSON"),
                }
            }
        }
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("cli parse")
    }

    #[test]
    fn tome_json_env_forces_json_when_flag_absent() {
        let _lock = lock_env();
        let _env = JsonEnvGuard::set(Some("1"));
        // No `--json` flag, but TOME_JSON=1 → Json.
        assert_eq!(parse(&["tome", "status"]).mode(), Mode::Json);
    }

    #[test]
    fn tome_json_unset_is_human_without_flag() {
        let _lock = lock_env();
        let _env = JsonEnvGuard::set(None);
        assert_eq!(parse(&["tome", "status"]).mode(), Mode::Human);
    }

    #[test]
    fn tome_json_flag_wins_even_without_env() {
        let _lock = lock_env();
        let _env = JsonEnvGuard::set(None);
        // `--json` flag alone → Json, no env needed.
        assert_eq!(parse(&["tome", "--json", "status"]).mode(), Mode::Json);
    }

    #[test]
    fn tome_json_garbage_value_is_truthy() {
        let _lock = lock_env();
        // `env_truthy` semantics: any set, non-empty, non-falsey token is truthy.
        let _env = JsonEnvGuard::set(Some("xyz"));
        assert_eq!(parse(&["tome", "status"]).mode(), Mode::Json);
    }

    #[test]
    fn tome_json_falsey_and_empty_are_human() {
        let _lock = lock_env();
        for falsey in ["0", "false", "no", "off", ""] {
            let _env = JsonEnvGuard::set(Some(falsey));
            assert_eq!(
                parse(&["tome", "status"]).mode(),
                Mode::Human,
                "TOME_JSON={falsey:?} must not force JSON",
            );
        }
    }

    #[test]
    fn short_w_parses_as_workspace() {
        // `-w <name>` is the short form of `--workspace`.
        let cli = parse(&["tome", "-w", "demo", "status"]);
        assert_eq!(cli.scope.workspace.as_deref(), Some("demo"));
        // The long form is unchanged.
        let cli_long = parse(&["tome", "--workspace", "demo", "status"]);
        assert_eq!(cli_long.scope.workspace.as_deref(), Some("demo"));
    }

    #[test]
    fn status_positional_workspace_parses() {
        // `status <workspace>` is a bare positional (not `--flag`).
        let cli = parse(&["tome", "status", "other"]);
        let Command::Status(args) = cli.command else {
            panic!("expected Status");
        };
        assert_eq!(args.name.as_deref(), Some("other"));
        assert!(!args.verify);
        // No positional → None.
        let cli_none = parse(&["tome", "status"]);
        let Command::Status(args_none) = cli_none.command else {
            panic!("expected Status");
        };
        assert_eq!(args_none.name, None);
    }

    #[test]
    fn status_positional_does_not_shadow_global_workspace_flag() {
        // Naming the positional field `name` (not `workspace`) keeps the global
        // `-w`/`--workspace` flag usable on `tome status` in trailing position —
        // the positional and the global flag no longer share a clap arg id.
        let cli = parse(&["tome", "status", "-w", "ws", "pos"]);
        assert_eq!(cli.scope.workspace.as_deref(), Some("ws"));
        let Command::Status(args) = cli.command else {
            panic!("expected Status");
        };
        assert_eq!(args.name.as_deref(), Some("pos"));
    }

    // ---- issue #437: hide internal harness dispatch targets ---------------

    /// `session-start` and `run-hook` are internal dispatch targets (hook files
    /// invoke them), so they must NOT appear in the `tome harness --help`
    /// listing — but hiding must not break invocation by name.
    #[test]
    fn harness_plumbing_is_hidden_from_help_but_still_invocable() {
        use clap::CommandFactory;

        let mut cmd = Cli::command();
        let harness = cmd
            .find_subcommand_mut("harness")
            .expect("harness subcommand");
        let help = harness.render_long_help().to_string();
        for hidden in ["session-start", "run-hook"] {
            assert!(
                !help.contains(hidden),
                "`tome harness --help` must not list the `{hidden}` plumbing:\n{help}",
            );
        }

        // Still invocable by name: `--help` on the hidden subcommand resolves
        // to clap's DisplayHelp (what a real invocation prints), not an
        // unknown-subcommand usage error.
        for hidden in ["session-start", "run-hook"] {
            let err = Cli::try_parse_from(["tome", "harness", hidden, "--help"])
                .expect_err("--help short-circuits parsing");
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelp,
                "`tome harness {hidden} --help` must resolve, not error",
            );
        }

        // And a normal parse still lands on the right variant.
        let cli = parse(&["tome", "harness", "session-start"]);
        let Command::Harness(HarnessArgs {
            command: Some(HarnessCommand::SessionStart(_)),
        }) = cli.command
        else {
            panic!("expected `harness session-start` to parse");
        };
        let cli = parse(&[
            "tome",
            "harness",
            "run-hook",
            "--event",
            "PreToolUse",
            "--harness",
            "codex",
        ]);
        let Command::Harness(HarnessArgs {
            command: Some(HarnessCommand::RunHook(_)),
        }) = cli.command
        else {
            panic!("expected `harness run-hook` to parse");
        };
    }

    // ---- issue #438: --yes is the primary confirmation-skip spelling -------

    /// `plugin disable` and `models remove` only skip a confirmation prompt
    /// (no safety behaviour is overridden), so `--yes` is the visible flag
    /// and the historical `--force` stays as a hidden back-compat alias —
    /// both spellings set the same field.
    #[test]
    fn confirmation_skips_take_yes_with_hidden_force_alias() {
        use clap::CommandFactory;

        for spelling in ["--yes", "--force"] {
            let cli = parse(&["tome", "plugin", "disable", "cat/p", spelling]);
            let Command::Plugin(PluginArgs {
                command: Some(PluginCommand::Disable(a)),
            }) = cli.command
            else {
                panic!("expected `plugin disable`");
            };
            assert!(a.yes, "`plugin disable {spelling}` must set `yes`");

            let cli = parse(&["tome", "models", "remove", "m", spelling]);
            let Command::Models(ModelsCommand::Remove(a)) = cli.command else {
                panic!("expected `models remove`");
            };
            assert!(a.yes, "`models remove {spelling}` must set `yes`");
        }

        // The alias is hidden: the rendered options list `--yes`, and no
        // option ENTRY is spelled `--force` (the flag description may still
        // mention the alias in prose, so scan option heads, not the whole
        // text).
        let mut cmd = Cli::command();
        for path in [["plugin", "disable"], ["models", "remove"]] {
            let sub = cmd
                .find_subcommand_mut(path[0])
                .and_then(|c| c.find_subcommand_mut(path[1]))
                .unwrap_or_else(|| panic!("subcommand {path:?}"));
            let help = sub.render_long_help().to_string();
            assert!(
                help.contains("--yes"),
                "help for {path:?} must list --yes:\n{help}",
            );
            assert!(
                !help.lines().any(|l| l.trim_start().starts_with("--force")),
                "help for {path:?} must not list a --force option entry:\n{help}",
            );
        }
    }

    // ---- issue #324: authoring flag consistency --------------------------

    /// Parse a `skill convert` and pluck out its [`ConvertArgs`], or panic.
    fn skill_convert(args: &[&str]) -> super::ConvertArgs {
        let cli = parse(args);
        let Command::Skill(super::SkillCommand::Convert(a)) = cli.command else {
            panic!("expected `skill convert`");
        };
        a
    }

    /// Parse a `catalog convert` and pluck out its [`CatalogConvertArgs`].
    fn catalog_convert(args: &[&str]) -> super::CatalogConvertArgs {
        let cli = parse(args);
        let Command::Catalog(super::CatalogCommand::Convert(a)) = cli.command else {
            panic!("expected `catalog convert`");
        };
        a
    }

    /// The exit code clap surfaces for a rejected parse (usage → 2).
    fn parse_exit(args: &[&str]) -> i32 {
        Cli::try_parse_from(args)
            .expect_err("parse should be rejected")
            .exit_code()
    }

    #[test]
    fn convert_from_is_a_closed_value_enum_with_aliases() {
        use crate::authoring::detect::SourceHarness;
        // Canonical kebab names resolve to the matching variant.
        assert_eq!(
            skill_convert(&["tome", "skill", "convert", "src", "--from", "claude-code"]).from,
            Some(SourceHarness::ClaudeCode)
        );
        assert_eq!(
            skill_convert(&["tome", "skill", "convert", "src", "--from", "cline"]).from,
            Some(SourceHarness::Cline)
        );
        // The historical `claude`/`agent` aliases still parse (back-compat).
        assert_eq!(
            skill_convert(&["tome", "skill", "convert", "src", "--from", "claude"]).from,
            Some(SourceHarness::ClaudeCode)
        );
        assert_eq!(
            skill_convert(&["tome", "skill", "convert", "src", "--from", "agent"]).from,
            Some(SourceHarness::AgentSkills)
        );
        // Omitted → None.
        assert_eq!(
            skill_convert(&["tome", "skill", "convert", "src"]).from,
            None
        );
    }

    #[test]
    fn convert_from_rejects_an_unknown_value_at_parse_time() {
        // A bogus `--from` is a clap usage error (exit 2), not a runtime failure.
        assert_eq!(
            parse_exit(&["tome", "skill", "convert", "src", "--from", "bogus"]),
            2
        );
    }

    #[test]
    fn convert_name_is_a_positional_and_the_name_flag_is_gone() {
        // The positional `<NAME>` still works.
        assert_eq!(
            skill_convert(&["tome", "skill", "convert", "src", "renamed"])
                .name
                .as_deref(),
            Some("renamed")
        );
        // `--name` was dropped: clap now rejects it as an unexpected argument.
        assert_eq!(
            parse_exit(&["tome", "skill", "convert", "src", "--name", "renamed"]),
            2
        );
    }

    #[test]
    fn no_fetch_is_catalog_convert_only() {
        // `catalog convert --no-fetch` sets the flag on the catalog-only struct.
        assert!(catalog_convert(&["tome", "catalog", "convert", "src", "--no-fetch"]).no_fetch);
        // `--local-only` is an accepted alias for the same flag.
        assert!(catalog_convert(&["tome", "catalog", "convert", "src", "--local-only"]).no_fetch);
        // Bare (no flag) → false.
        assert!(!catalog_convert(&["tome", "catalog", "convert", "src"]).no_fetch);
        // `skill`/`plugin convert --no-fetch` are rejected (unexpected argument).
        assert_eq!(
            parse_exit(&["tome", "skill", "convert", "src", "--no-fetch"]),
            2
        );
        assert_eq!(
            parse_exit(&["tome", "plugin", "convert", "src", "--no-fetch"]),
            2
        );
    }

    #[test]
    fn lint_dry_run_requires_autofix() {
        // A bare `lint --dry-run` (no `--autofix`) is a clap usage error.
        assert_eq!(
            parse_exit(&["tome", "skill", "lint", "src", "--dry-run"]),
            2
        );
        // `lint --autofix --dry-run` still parses.
        let cli = parse(&["tome", "skill", "lint", "src", "--autofix", "--dry-run"]);
        let Command::Skill(super::SkillCommand::Lint(a)) = cli.command else {
            panic!("expected `skill lint`");
        };
        assert!(a.autofix && a.dry_run);
    }

    // ---- issue #326: lint variadic sources + convert typed source ----------

    /// Parse a `plugin lint` and pluck out its [`LintArgs`], or panic.
    fn plugin_lint(args: &[&str]) -> super::LintArgs {
        let cli = parse(args);
        let Command::Plugin(super::PluginArgs {
            command: Some(super::PluginCommand::Lint(a)),
        }) = cli.command
        else {
            panic!("expected `plugin lint`");
        };
        a
    }

    #[test]
    fn lint_accepts_a_single_source_as_a_pathbuf() {
        // Back-compat: one positional still parses; `sources` holds exactly it.
        let a = plugin_lint(&["tome", "plugin", "lint", "plugins/a"]);
        assert_eq!(a.sources, vec![PathBuf::from("plugins/a")]);
    }

    #[test]
    fn lint_accepts_multiple_sources() {
        // The shell expands `plugins/*` into multiple args; the variadic
        // positional collects them all (in order).
        let a = plugin_lint(&[
            "tome",
            "plugin",
            "lint",
            "plugins/a",
            "plugins/b",
            "plugins/c",
        ]);
        assert_eq!(
            a.sources,
            vec![
                PathBuf::from("plugins/a"),
                PathBuf::from("plugins/b"),
                PathBuf::from("plugins/c"),
            ]
        );
    }

    #[test]
    fn lint_with_zero_sources_is_a_usage_error() {
        // `num_args = 1..` ⇒ at least one source is required (clap usage → 2).
        assert_eq!(parse_exit(&["tome", "plugin", "lint"]), 2);
    }

    #[test]
    fn lint_multiple_sources_still_take_the_shared_flags() {
        // Variadic positionals + trailing flags coexist: the flags bind, and all
        // preceding positionals collect into `sources`.
        let a = plugin_lint(&["tome", "plugin", "lint", "a", "b", "--strict", "--autofix"]);
        assert_eq!(a.sources, vec![PathBuf::from("a"), PathBuf::from("b")]);
        assert!(a.strict && a.autofix);
    }

    #[test]
    fn convert_source_is_a_pathbuf_and_normal_sources_still_parse() {
        // A plain path, an `owner/repo` shorthand, and a URL all parse into the
        // typed `source: PathBuf` (the value_name/value_hint change parsing of
        // none of them).
        for src in ["./local/dir", "owner/repo", "https://example.com/x.git"] {
            let a = skill_convert(&["tome", "skill", "convert", src]);
            assert_eq!(a.source, PathBuf::from(src));
        }
    }

    // ---- issue #314: plugin enable/disable variadic ids + --catalog -------

    #[test]
    fn plugin_enable_accepts_variadic_ids_and_catalog() {
        let cli = parse(&[
            "tome",
            "plugin",
            "enable",
            "cat/a",
            "cat/b*",
            "bare",
            "--catalog",
            "cat",
        ]);
        let Command::Plugin(super::PluginArgs {
            command: Some(super::PluginCommand::Enable(a)),
        }) = cli.command
        else {
            panic!("expected `plugin enable`");
        };
        assert_eq!(a.ids, vec!["cat/a", "cat/b*", "bare"]);
        assert_eq!(a.catalog.as_deref(), Some("cat"));
    }

    #[test]
    fn plugin_disable_accepts_variadic_ids_and_catalog() {
        let cli = parse(&[
            "tome",
            "plugin",
            "disable",
            "cat/a",
            "cat/*",
            "--catalog",
            "cat",
        ]);
        let Command::Plugin(super::PluginArgs {
            command: Some(super::PluginCommand::Disable(a)),
        }) = cli.command
        else {
            panic!("expected `plugin disable`");
        };
        assert_eq!(a.ids, vec!["cat/a", "cat/*"]);
        assert_eq!(a.catalog.as_deref(), Some("cat"));
    }

    #[test]
    fn plugin_enable_single_id_still_parses() {
        // Back-compat: exactly one positional id is the common case.
        let cli = parse(&["tome", "plugin", "enable", "cat/plug"]);
        let Command::Plugin(super::PluginArgs {
            command: Some(super::PluginCommand::Enable(a)),
        }) = cli.command
        else {
            panic!("expected `plugin enable`");
        };
        assert_eq!(a.ids, vec!["cat/plug"]);
        assert_eq!(a.catalog, None);
    }

    #[test]
    fn plugin_enable_requires_at_least_one_id() {
        // Zero positionals is a clap usage error (exit 2).
        assert_eq!(parse_exit(&["tome", "plugin", "enable"]), 2);
        assert_eq!(parse_exit(&["tome", "plugin", "disable"]), 2);
    }
}
