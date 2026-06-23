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

#[derive(Debug, Parser)]
#[command(
    name = "tome",
    about,
    long_about = None,
    // `--version` is intercepted by a pre-parse hook in `main.rs` so the
    // output can include embedder + reranker identities and honour
    // `--json`. clap's auto handler can't do either, hence the override.
    disable_version_flag = true,
)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout instead of human text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable ANSI colour in human output. Overrides `[output] color` in
    /// `~/.tome/config.toml` and the `NO_COLOR` environment variable.
    /// The MCP path never emits colour regardless of this flag.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Increase log verbosity. `-v` = info, `-vv` = debug. Env: TOME_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

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
    /// resolver consults `TOME_WORKSPACE` and the project-marker walk
    /// before falling back to the privileged `global` workspace.
    #[arg(long, global = true, value_name = "NAME")]
    pub workspace: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
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
    /// The entry to retier, as `<plugin>/<name>`.
    pub id: String,
    /// The routing tier: 1 (load at session start), 2 (load before matching
    /// tasks), or 3 (default; searchable on demand).
    #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
    pub tier: u8,
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
    /// The entry to reset, as `<plugin>/<name>`.
    pub id: String,
    #[arg(long)]
    pub catalog: Option<String>,
    #[arg(long, value_enum)]
    pub kind: Option<TierKindArg>,
}

/// CLI-facing entry-kind selector for tier disambiguation (tiers never apply to
/// agents, so only skill/command are offered).
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TierKindArg {
    Skill,
    Command,
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
    /// Skip the confirmation prompt.
    #[arg(long)]
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
    /// The bundled skill id (e.g. `convert-marketplace`).
    pub skill_id: String,
    /// Target a specific harness (repeatable). Default: every detected
    /// harness that consumes native skills.
    #[arg(long = "harness")]
    pub harnesses: Vec<String>,
    /// Install into the user-level skills dir instead of the project.
    #[arg(long)]
    pub global: bool,
    /// Re-write even when the on-disk copy is already at the current revision.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct MetaRemoveArgs {
    /// The bundled skill id.
    pub skill_id: String,
    /// Target a specific harness (repeatable). Default: every detected
    /// harness that consumes native skills.
    #[arg(long = "harness")]
    pub harnesses: Vec<String>,
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
    /// Reconcile the project, then print the workspace's skill-routing directive
    /// to stdout, generated fresh from live state. Intended as a SessionStart
    /// hook target; not usually run by hand.
    SessionStart(HarnessSessionStartArgs),
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
    #[arg(conflicts_with = "all")]
    pub names: Vec<String>,
    /// Configure every supported (auto-detectable) harness, regardless of
    /// detection. Excludes the opt-in `generic` / `generic-op` targets.
    /// Mutually exclusive with explicit names.
    #[arg(long)]
    pub all: bool,
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
    /// Harness name.
    pub name: String,
    /// Settings scope to edit. When omitted, falls back to
    /// `[harness] default_scope` in `~/.tome/config.toml`, then to `project`.
    #[arg(long, value_enum)]
    pub scope: Option<HarnessScopeArg>,
}

#[derive(Debug, clap::Args)]
pub struct HarnessInfoArgs {
    /// Harness name.
    pub name: String,
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
    /// SHA-256. Slower than the default but catches silent on-disk
    /// corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceArgs {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
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
    /// `last_used_at`.
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
    /// Bind the current project directory to the named workspace.
    /// Creates / overwrites `<cwd>/.tome/config.toml` so subsequent
    /// Tome invocations from this tree resolve to `<name>` via the
    /// project-marker walk. The atomic-directory landing means a
    /// SIGINT mid-bind never leaves a partial `.tome/`. Phase 4 / US1.a
    /// stubs the harness-sync seam; US1.b wires the real sync.
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
    /// Workspace name (must already exist in the central registry; create
    /// via `tome workspace init <name>`).
    pub name: String,
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
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceListArgs {
    // No subcommand-specific flags. `--json` is the global flag.
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
    /// Workspace to regenerate summaries for. Required — `regen-summary`
    /// is the explicit summarisation command; we don't want the user to
    /// accidentally regenerate the resolved scope (often `global`) by
    /// forgetting an argument.
    pub name: String,
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
    /// Rehash each installed model's primary file against its pinned
    /// SHA-256. Slower (several seconds for the reranker), but catches
    /// silent on-disk corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct ReindexArgs {
    /// Scope. Omit to reindex every enabled plugin across every catalog;
    /// pass `<catalog>` to scope to one catalog; pass `<catalog>/<plugin>`
    /// to scope to one plugin.
    pub scope: Option<String>,
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
}

#[derive(Debug, clap::Args)]
pub struct ModelsDownloadArgs {
    /// Re-download even when the on-disk manifest records a complete install.
    #[arg(long)]
    pub force: bool,
    /// Download every registered model, not just the active profile's set.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, clap::Args)]
pub struct ModelsProfileArgs {
    /// Set the active model profile. Omit to show the current profile.
    #[arg(value_parser = ["small", "medium", "large"])]
    pub tier: Option<String>,
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
    /// The registered model name (e.g. `bge-small-en-v1.5`).
    pub name: String,
    /// Skip the confirmation prompt. Required when stdin is not a TTY.
    #[arg(long)]
    pub force: bool,
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
    Convert(ConvertArgs),
    /// Validate a Tome catalog (and every plugin/skill it nests). CI-ready.
    Lint(LintArgs),
}

#[derive(Debug, clap::Args)]
pub struct CatalogAddArgs {
    /// The catalog source: an owner/repo shorthand, a Git URL, or a local
    /// path (interpreted as `file://`).
    pub source: String,
    /// Override the display name (defaults to the manifest's `name`).
    #[arg(long)]
    pub name: Option<String>,
    /// Branch, tag, or SHA to track. Defaults to `main`.
    #[arg(long = "ref")]
    pub ref_: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct CatalogRemoveArgs {
    /// The catalog display name to remove.
    pub name: String,
    /// Skip the confirmation prompt. Required when stdin is not a TTY.
    #[arg(long)]
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
    /// Reserved for future symmetry with the other commands. Currently no-op.
    #[arg(long)]
    pub force: bool,
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
    /// The plugin to enable, as `<catalog>/<plugin>`.
    pub id: String,
    /// Skip the model-download confirmation prompt. Required to enable a
    /// plugin from a non-interactive context (e.g. CI) when models are
    /// not yet installed.
    #[arg(long)]
    pub yes: bool,
    /// Routing tier (1|2|3) to apply to ALL of this plugin's skills and
    /// commands at enable time. Omitted → the default tier 3. Refine
    /// per-entry later with `tome tier set`.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=3))]
    pub tier: Option<u8>,
}

#[derive(Debug, clap::Args)]
pub struct PluginDisableArgs {
    /// The plugin to disable, as `<catalog>/<plugin>`.
    pub id: String,
    /// Skip the confirmation prompt. Required to disable a plugin from a
    /// non-interactive context (e.g. CI).
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct PluginListArgs {
    /// Restrict the listing to one catalog.
    #[arg(long)]
    pub catalog: Option<String>,
    /// Hide disabled and unindexable plugins.
    #[arg(long = "enabled-only")]
    pub enabled_only: bool,
}

#[derive(Debug, clap::Args)]
pub struct PluginShowArgs {
    /// The plugin to inspect, as `<catalog>/<plugin>`.
    pub id: String,
}

#[derive(Debug, clap::Args)]
pub struct QueryArgs {
    /// The query text to search for. Embedded as-is — no name/description
    /// composition is applied.
    pub text: String,

    /// Cap on returned results (post-rerank when reranking).
    /// When absent, falls back to `[query] top_k` in `~/.tome/config.toml`,
    /// then to the built-in default of 10.
    #[arg(long = "top-k")]
    pub top_k: Option<u32>,

    /// Restrict the search to a single catalog.
    #[arg(long)]
    pub catalog: Option<String>,

    /// Restrict the search to a single plugin (across all enabled catalogs
    /// unless `--catalog` is also set).
    #[arg(long)]
    pub plugin: Option<String>,

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
    /// Overwrite colliding files (only those files; never a directory wipe).
    #[arg(long)]
    pub force: bool,
}

/// Shared `convert` arguments across all three artifact levels.
#[derive(Debug, clap::Args)]
pub struct ConvertArgs {
    /// Source to convert: a local path, an `owner/repo` shorthand, or a git
    /// URL. Remote sources are fetched into a temp clone (cleaned up on every
    /// exit path). The source is never modified.
    pub source: String,
    /// New name for the converted artifact (also settable via `--name`).
    /// Defaults to `<source-name>-tome`. Supplying both the positional and
    /// `--name` with different values is a usage error.
    pub name: Option<String>,
    /// New name for the converted artifact (same as the positional `<NAME>`).
    #[arg(long = "name")]
    pub name_flag: Option<String>,
    /// Override source-format detection: claude-code | codex | cursor |
    /// opencode | cline | agent-skills.
    #[arg(long = "from")]
    pub from: Option<String>,
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
    /// Do not fetch the marketplace's remote-source plugins (`catalog convert`
    /// only); they are warned-and-skipped instead. The SOURCE argument itself
    /// may still be a remote clone.
    #[arg(long = "no-fetch")]
    pub no_fetch: bool,
}

/// Shared `lint` arguments across all three artifact levels.
#[derive(Debug, clap::Args)]
pub struct LintArgs {
    /// The Tome artifact to validate (a local path).
    pub source: String,
    /// Apply mechanically-safe fixes (rewritable harness-isms, `name == dir`);
    /// report fixed vs. still-manual.
    #[arg(long)]
    pub autofix: bool,
    /// With `--autofix`, report would-be fixes but change nothing on disk.
    #[arg(long = "dry-run")]
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
        crate::output::Mode::from_flag(self.json)
    }
}
