//! Dispatcher for `tome harness <subcommand>` plus shared helpers.
//!
//! Phase 4 / US3.c-2 promotes this module from a single-function shim
//! to the full subcommand surface. The pre-existing
//! [`sync_for_project_root`] entry point (used by `tome workspace use`'s
//! binding flow) is preserved verbatim — `BindDeps`-flavoured callers
//! still go through it. The new public API is [`run`], which dispatches
//! the clap subcommand surface, and the per-subcommand modules
//! ([`bare`], [`list`], [`use_`], [`remove`], [`info`], [`sync`]).
//!
//! ## Resolving the project root
//!
//! Every subcommand other than `list <workspace>` / `info` may consult
//! the resolved scope's `project_root`. When the scope was resolved via
//! a project marker, this is the project dir; otherwise it is `None`
//! and the subcommand decides whether absence is fatal (sync / use
//! --scope project → error) or merely informational (bare / info →
//! `—` placeholder).
//!
//! ## ScopeProvider for `tome harness list`
//!
//! `harness list` (no arg) resolves the effective harness list which
//! may chase `[workspaces.<name>]` references. The production
//! [`ScopeProvider`] [`CentralDbScopeProvider`] consults the central
//! SQLite registry (`workspaces` table) to confirm workspace membership
//! and then reads the workspace's on-disk `settings.toml` (when present)
//! for the directly-declared harnesses list:
//!
//! * **Workspace exists, settings file present + parses** → `Ok(Some(list))`
//! * **Workspace exists, settings file absent** → `Ok(None)` (legal — no
//!   harnesses declared)
//! * **Workspace exists, file unreadable or unparsable** →
//!   `Err(SettingsReadFailure)` which maps to exit 70
//!   (`WorkspaceMalformed`) — distinct from "workspace doesn't exist"
//! * **Workspace not in central registry** → `Err(UnknownWorkspace)` which
//!   maps to exit 13 (`WorkspaceNotFound`).
//!
//! When the central DB has not yet been bootstrapped (no `index.db` file),
//! only the privileged `global` workspace is considered to exist. Any
//! other reference resolves to `UnknownWorkspace`.

pub mod bare;
pub mod info;
pub mod list;
pub mod remove;
pub mod session_start;
pub mod use_;

use std::path::Path;

use crate::cli::{HarnessArgs, HarnessCommand, HarnessScopeArg};
use crate::error::{CompositionErrorKind, TomeError};
use crate::output::Mode;
use crate::paths::Paths;
use crate::settings::parser::parse_workspace;
use crate::settings::resolver::ScopeProvider;
use crate::workspace::binding::BindDeps;
use crate::workspace::{ResolvedScope, WorkspaceName};

pub use crate::harness::sync::SyncOutcome;

/// Sync every effective harness for `project_root` against the freshly-
/// bound `workspace_name`. Computes the effective harness list from
/// `<project_root>/.tome/config.toml` + the workspace's `settings.toml`
/// + the global `settings.toml`, then dispatches per-harness writes.
///
/// `force` is forwarded to the orchestrator's clash-override path
/// (FR-501).
pub fn sync_for_project_root(
    project_root: &Path,
    workspace_name: &WorkspaceName,
    deps: &BindDeps<'_>,
    force: bool,
) -> Result<SyncOutcome, TomeError> {
    let sync_deps =
        crate::harness::sync::build_deps(deps.paths, deps.home_root, workspace_name, force);
    crate::harness::sync::sync_project(project_root, &sync_deps)
}

/// Resolve the effective harness `--scope` argument.
///
/// Precedence: explicit CLI `--scope` → `[harness] default_scope` in
/// `~/.tome/config.toml` → `HarnessScopeArg::Project`.
///
/// Config is loaded strictly (`config::load`) — aligned with T8/T10 and every
/// other foreground config read — so a malformed `config.toml` surfaces exit 5
/// rather than silently ignoring the user's configured default scope.
pub(crate) fn effective_harness_scope(
    arg: Option<HarnessScopeArg>,
    paths: &Paths,
) -> Result<HarnessScopeArg, crate::error::TomeError> {
    if let Some(explicit) = arg {
        return Ok(explicit);
    }
    let cfg = crate::config::load(paths)?;
    Ok(match cfg.harness.default_scope {
        Some(crate::config::HarnessScope::Global) => HarnessScopeArg::Global,
        Some(crate::config::HarnessScope::Project) | None => HarnessScopeArg::Project,
        // `Workspace` is not a valid `HarnessScope` in config (only Project/Global
        // are the two persisted options), so no third arm is needed.
    })
}

/// Subcommand dispatcher invoked by `main.rs`.
pub fn run(args: HarnessArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    match args.command {
        None => bare::run(scope, &paths, mode),
        Some(HarnessCommand::List(a)) => list::run(a, scope, &paths, mode),
        Some(HarnessCommand::Use(a)) => {
            // Phase 11 / US6: `use` is now multi-harness — telemetry is emitted
            // per successfully-configured harness inside `use_::run` (it knows
            // the resolved selection + which harnesses succeeded), so the
            // dispatcher no longer references a single name.
            use_::run(a, scope, &paths, mode)
        }
        Some(HarnessCommand::Remove(a)) => {
            let name = a.name.clone();
            let r = remove::run(a, scope, &paths, mode);
            if r.is_ok() {
                emit_harness_action(&name, crate::telemetry::event::HarnessAction::Remove);
            }
            r
        }
        Some(HarnessCommand::Info(a)) => info::run(a, scope, &paths, mode),
        Some(HarnessCommand::SessionStart(a)) => session_start::run(a, scope, &paths, mode),
    }
}

/// Map a harness id string (as used by the `HarnessModule::name()` registry —
/// `claude-code` / `cursor` / `codex` / `opencode` / `gemini`) to the closed
/// telemetry [`Harness`](crate::telemetry::event::Harness) enum.
///
/// Note the one rename: the harness module names itself `gemini` while the
/// telemetry enum's wire token is `gemini-cli`; this is the single place the
/// two vocabularies are bridged. Returns `None` for any unknown id so the
/// caller can SKIP the emit rather than invent a value (closed-by-construction).
pub(crate) fn harness_name_to_enum(name: &str) -> Option<crate::telemetry::event::Harness> {
    use crate::telemetry::event::Harness;
    match name {
        "claude-code" => Some(Harness::ClaudeCode),
        "cursor" => Some(Harness::Cursor),
        "codex" => Some(Harness::Codex),
        "opencode" => Some(Harness::Opencode),
        "gemini" => Some(Harness::GeminiCli),
        // Phase 11 — additional harnesses. For these the wire token equals the
        // id, so this is a flat name→variant bridge — but NOT for `gemini`
        // (id `gemini` → wire token `gemini-cli`, the one rename handled above).
        // `antigravity-cli` is an alias of `gemini` and is resolved upstream,
        // never reaching this function.
        "copilot-cli" => Some(Harness::CopilotCli),
        "copilot" => Some(Harness::Copilot),
        "devin" => Some(Harness::Devin),
        "cline" => Some(Harness::Cline),
        "junie" => Some(Harness::Junie),
        "jetbrains-ai" => Some(Harness::JetbrainsAi),
        "antigravity" => Some(Harness::Antigravity),
        "pi" => Some(Harness::Pi),
        "crush" => Some(Harness::Crush),
        "zed" => Some(Harness::Zed),
        "kiro" => Some(Harness::Kiro),
        "generic" => Some(Harness::Generic),
        "generic-op" => Some(Harness::GenericOp),
        "goose" => Some(Harness::Goose),
        // SKIP: unmapped — never guess a closed-enum value.
        _ => None,
    }
}

/// Emit one `tome.harness_action` event for a single harness on the success
/// path. Infallible best-effort `enqueue`; SKIPs silently when the name does
/// not map to the closed [`Harness`](crate::telemetry::event::Harness) enum.
pub(crate) fn emit_harness_action(name: &str, action: crate::telemetry::event::HarnessAction) {
    if let Some(harness) = harness_name_to_enum(name) {
        crate::telemetry::enqueue(crate::telemetry::event::HarnessActionEvent { action, harness });
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Production [`ScopeProvider`] backed by the central SQLite registry
/// (the source of truth for workspace membership) and the on-disk
/// `<root>/workspaces/<name>/settings.toml` files (the source of truth
/// for the directly-declared harness list).
///
/// Three-way classification per the trait contract:
///
/// 1. **Workspace not in the registry** → `Err(UnknownWorkspace)`
///    (exit 13).
/// 2. **Workspace in the registry, settings file absent** →
///    `Ok(None)` — legal: the workspace exists but doesn't declare a
///    `harnesses` list. The resolver treats this as "no recursion shape
///    from this scope" per FR-449.
/// 3. **Workspace in the registry, settings file present** →
///    `Ok(Some(list))` with whatever the file declares. IO / parse
///    failures surface as `Err(SettingsReadFailure)` (exit 70) — distinct
///    from "unknown" so the user sees the malformed-state hint rather
///    than a misleading "workspace not found" message.
///
/// When the central DB has not been bootstrapped (no `index.db`), only
/// `WorkspaceName::global()` is considered to exist. The `global`
/// workspace is the bootstrap-seeded row in every DB; treating it as
/// always-present aligns with that invariant.
pub(crate) struct CentralDbScopeProvider<'a> {
    paths: &'a Paths,
}

impl<'a> CentralDbScopeProvider<'a> {
    pub(crate) fn new(paths: &'a Paths) -> Self {
        Self { paths }
    }

    /// Confirm `name` exists in the central `workspaces` table. Falls
    /// back to "only global is known" when the DB file is absent so a
    /// freshly-installed Tome still resolves `[global]` cleanly without
    /// requiring an initial bootstrap pass.
    fn workspace_is_registered(&self, name: &WorkspaceName) -> bool {
        // Bootstrap-not-yet shortcut: privileged `global` is always
        // considered present; everything else is unknown.
        if !self.paths.index_db.exists() {
            return name.as_str() == WorkspaceName::global().as_str();
        }
        let conn = match crate::index::open_read_only(&self.paths.index_db) {
            Ok(c) => c,
            // If we can't open read-only, the DB is in a broken state.
            // Treat the workspace as unknown — surfaces as exit 13 with
            // a hint pointing at `tome doctor`.
            Err(_) => return name.as_str() == WorkspaceName::global().as_str(),
        };
        conn.query_row(
            "SELECT 1 FROM workspaces WHERE name = ?1",
            rusqlite::params![name.as_str()],
            |_| Ok(()),
        )
        .is_ok()
    }
}

impl ScopeProvider for CentralDbScopeProvider<'_> {
    fn directly_declared_harnesses(
        &self,
        name: &WorkspaceName,
    ) -> Result<Option<Vec<String>>, CompositionErrorKind> {
        // 1. Membership check against the central registry.
        if !self.workspace_is_registered(name) {
            return Err(CompositionErrorKind::UnknownWorkspace(
                name.as_str().to_owned(),
            ));
        }

        // 2. Read the workspace's settings.toml. Absent = legal "no
        //    harnesses declared" → Ok(None). IO + parse failures =
        //    SettingsReadFailure → exit 70 via the From boundary.
        let path = self.paths.workspace_settings_file(name);
        let body = match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
            Ok(b) => b,
            Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(CompositionErrorKind::SettingsReadFailure(
                    name.as_str().to_owned(),
                    format!("read {}: {e}", path.display()),
                ));
            }
        };
        let ws = parse_workspace(&body).map_err(|e| {
            CompositionErrorKind::SettingsReadFailure(
                name.as_str().to_owned(),
                format!("parse {}: {e}", path.display()),
            )
        })?;
        Ok(ws.harnesses)
    }
}

/// Resolve `$HOME` for harness-detect calls. Centralised so subcommands
/// don't sprinkle `std::env::var_os("HOME")` calls.
///
/// PR-E S-M7 mirrors the validation discipline in
/// [`crate::paths::home_root`]: refuse empty / non-absolute values up
/// front. Harness detection probes well-known dirs like
/// `<home>/.claude/` — a relative `$HOME` would resolve into the cwd,
/// surfacing spurious "claude-code detected" verdicts on the user's
/// current project directory rather than against their per-user state.
pub(crate) fn home_root() -> Result<std::path::PathBuf, TomeError> {
    let home_os = std::env::var_os("HOME").ok_or_else(|| {
        TomeError::Usage("$HOME is not set — cannot probe harness detection paths".to_string())
    })?;
    if home_os.is_empty() {
        return Err(TomeError::Usage(
            "$HOME is set to an empty string — cannot probe harness detection paths".to_string(),
        ));
    }
    let home = std::path::PathBuf::from(home_os);
    if !home.is_absolute() {
        return Err(TomeError::Usage(format!(
            "$HOME is not an absolute path: {}",
            home.display()
        )));
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::harness_name_to_enum;
    use crate::telemetry::event::Harness;

    #[test]
    fn harness_name_to_enum_maps_every_known_id() {
        // The five harness module ids → their closed telemetry enum tokens.
        // `gemini` is the one renamed bridge (module id `gemini` → `GeminiCli`).
        assert_eq!(
            harness_name_to_enum("claude-code"),
            Some(Harness::ClaudeCode)
        );
        assert_eq!(harness_name_to_enum("cursor"), Some(Harness::Cursor));
        assert_eq!(harness_name_to_enum("codex"), Some(Harness::Codex));
        assert_eq!(harness_name_to_enum("opencode"), Some(Harness::Opencode));
        assert_eq!(harness_name_to_enum("gemini"), Some(Harness::GeminiCli));
        // Phase 11 additions — id == wire token (no rename beyond `gemini`).
        assert_eq!(
            harness_name_to_enum("copilot-cli"),
            Some(Harness::CopilotCli)
        );
        assert_eq!(
            harness_name_to_enum("jetbrains-ai"),
            Some(Harness::JetbrainsAi)
        );
        assert_eq!(harness_name_to_enum("generic-op"), Some(Harness::GenericOp));
        assert_eq!(harness_name_to_enum("pi"), Some(Harness::Pi));
        assert_eq!(harness_name_to_enum("goose"), Some(Harness::Goose));
    }

    #[test]
    fn harness_name_to_enum_unmapped_is_none() {
        // SSOT mapper used by BOTH the CLI harness_action emit and the MCP
        // `calling_harness` resolver: an unknown / unstamped id must yield
        // `None` so the caller omits the field rather than guessing.
        assert_eq!(harness_name_to_enum(""), None);
        assert_eq!(harness_name_to_enum("gemini-cli"), None);
        assert_eq!(harness_name_to_enum("not-a-real-harness"), None);
    }
}
