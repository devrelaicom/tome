use clap::Parser;

use tome::catalog::git;
use tome::cli::{Cli, Command};
use tome::paths;
use tome::presentation::{colour, progress, prompt};
use tome::workspace;
use tome::{commands, logging, output};

fn main() {
    // `--version` is handled before clap dispatch so the output can include
    // embedder + reranker identities (per `contracts/version-output.md`) and
    // honour the global `--json` flag. Clap's auto `--version` is disabled on
    // the `Cli` derive to avoid intercepting first.
    let raw: Vec<String> = std::env::args().collect();
    if raw.iter().skip(1).any(|a| a == "--version" || a == "-V") {
        let json = raw.iter().any(|a| a == "--json");
        // colour is not initialized yet — print_version must use plain output (no colour helpers).
        commands::status::print_version(json);
        std::process::exit(0);
    }

    let cli = Cli::parse();

    // `tome completions <shell>` is intercepted here — after the `--version`
    // hook, after `Cli::parse()`, but BEFORE `Paths::resolve()`, scope
    // resolution, logging/colour init, and telemetry. Generating a completion
    // script is a pure static operation over the derived `Cli` command tree; it
    // reads no HOME, index, config, or workspace. A user runs it during shell
    // setup (possibly before Tome is configured), so it must never require valid
    // state. `--json` is irrelevant for a shell script (ignored). The borrow of
    // `cli.command` ends before the later `match cli.command`, so there is no
    // move conflict. On error, emit via the same code/mode path as any command.
    if let Command::Completions(args) = &cli.command {
        if let Err(err) = commands::completions::run(args) {
            output::write_error(cli.mode(), &err);
            std::process::exit(err.exit_code());
        }
        std::process::exit(0);
    }

    // Skip the stderr-based CLI tracing subscriber on the MCP path —
    // `mcp::run` installs its own file-backed JSON subscriber, and the
    // global `tracing` registry only accepts one. Also skip the
    // Ctrl-C signal handler: the MCP server uses tokio's async
    // `signal::ctrl_c()` instead, and the CLI handler would race.
    if !matches!(cli.command, Command::Mcp(_)) {
        // Load config once defensively for logging + output knobs. A malformed
        // config.toml falls back to defaults here; the strict error is surfaced
        // by the command itself via `config::load`.
        let output_cfg = tome::paths::Paths::resolve().ok().map(|p| {
            let cfg = tome::config::load_or_default(&p);
            (cfg.logging.level, cfg.output)
        });
        let cfg_level = output_cfg.as_ref().and_then(|(lvl, _)| *lvl);
        logging::init(cli.verbosity(), cfg_level);
        git::install_signal_handler();
        // Forward the --no-color flag BEFORE init() so the OnceLock in
        // `colour::init` sees it.
        colour::set_disabled(cli.no_color);
        // Resolve the colour-enabled decision once, before any human output.
        // Pass the config value from the single `output_cfg` load above so
        // colour, progress, and logging all derive from the same snapshot.
        // Precedence: --no-color flag > NO_COLOR env > config [output] color >
        // auto (TTY). The MCP path emits only JSON-RPC, so it needs no colour.
        let cfg_color = output_cfg.as_ref().and_then(|(_, out)| out.color);
        colour::init(cfg_color);
        // Resolve progress visibility: config `[output] progress = false`
        // suppresses bars/spinners even on a TTY; otherwise auto (TTY check).
        // The MCP server never shows progress — do not init on that path.
        let cfg_progress = output_cfg.and_then(|(_, out)| out.progress);
        progress::init_progress(cfg_progress);
    }

    let mode = cli.mode();

    // Forward the global `--non-interactive` flag to the single confirmation
    // SSOT before any command dispatch, mirroring `colour::set_disabled`. Every
    // prompt-bearing command reads `prompt::non_interactive()` (which also
    // honours `TOME_NONINTERACTIVE`) alongside its per-command `--force`/`--yes`.
    prompt::set_non_interactive(cli.non_interactive);

    // Phase 4 / F10: Paths::resolve runs first so the workspace
    // resolver can consult the central index for membership. Both can
    // fail (HOME unset, central DB malformed, workspace not found,
    // workspace name invalid); errors flow through the same exit-code
    // path as command errors.
    let paths = match paths::Paths::resolve() {
        Ok(p) => p,
        Err(err) => {
            let code = err.exit_code();
            output::write_error(mode, &err);
            std::process::exit(code);
        }
    };
    // `tome doctor`, `tome status`, and `tome config` are the read-only
    // diagnostics a user reaches for when their setup is broken — including a
    // malformed `~/.tome/config.toml`. The pre-dispatch scope resolution is the
    // "universal gate" that runs strict `config::load` for every command (so a
    // typo fails loudly with exit 5 uniformly); but that same gate would brick
    // the very commands meant to diagnose the typo. Resolve those leniently
    // (a malformed config degrades step 3 — `[workspace] default` — to defaults,
    // never aborting); their command bodies then surface the parse problem
    // themselves — `config validate` REPORTS it (still exit 5), `config show`
    // re-runs the strict `load` and fails loudly (exit 5). Every OTHER command
    // keeps the strict gate, so "fail loud on a malformed config" stays intact
    // everywhere it matters.
    let diagnostic = matches!(
        cli.command,
        Command::Doctor(_) | Command::Status(_) | Command::Config(_)
    );
    let resolved = if diagnostic {
        workspace::resolution::resolve_lenient(&cli.scope, &paths)
    } else {
        workspace::resolution::resolve(&cli.scope, &paths)
    };
    let scope = match resolved {
        Ok(r) => r,
        Err(err) => {
            let code = err.exit_code();
            output::write_error(mode, &err);
            std::process::exit(code);
        }
    };

    // Issue #302: when a `[workspace] default` shadows a per-project marker that
    // exists in the CWD ancestry, the resolver records the shadowed marker on
    // `overridden_project_marker` (see `workspace::resolution`). Emit a one-line
    // stderr note here — the CLI foreground boundary, resolved ONCE before
    // dispatch — so the user learns why their per-project binding stopped
    // applying. This is a `note:` (never an error): the exit status is unchanged.
    //
    // Skip the MCP path: `tome mcp` speaks JSON-RPC and its stderr is the
    // harness's log channel, not a human's terminal — the resolver still
    // populates the field, but the server never surfaces it to a client. Skip
    // `--json` mode too so structured-stdout consumers aren't handed an
    // unstructured stderr line they didn't ask for.
    if !matches!(cli.command, Command::Mcp(_))
        && mode != output::Mode::Json
        && let Some(marker_dir) = scope.overridden_project_marker.as_deref()
    {
        eprintln!(
            "note: [workspace] default '{}' is overriding the project binding at {} \
             — per-project workspace/harness sync is inactive; unset [workspace] default \
             or run `tome workspace use` here.",
            scope.scope.name().as_str(),
            marker_dir.display(),
        );
    }

    // Build the process-global telemetry handle ONCE, unconditionally — every
    // command needs it: the CLI emit/teardown paths, the spawned `flush --quiet`
    // child, and the MCP server (its `commands::mcp::run` path emits + flushes
    // through the same global handle). A disabled handle (consent off / build
    // error) is a pure no-op, so this is safe on every path.
    tome::telemetry::init(&paths);

    // CLI process-start telemetry (FR-013/014/015 first-run notice + FR-026
    // `tome.install`/`tome.upgrade` lifecycle emits). Skip the MCP path (no
    // human stderr; it mints silently on its first enqueue) AND the `telemetry`
    // path (its subcommands manage telemetry themselves — a `telemetry off` must
    // not first mint an id + print a notice). `cli_startup` self-gates on the
    // enabled resolver (CI/disabled ⇒ no mint, no notice, no emit) and is
    // best-effort throughout — it never errors out the command.
    if !matches!(cli.command, Command::Mcp(_) | Command::Telemetry(_)) {
        tome::telemetry::cli_startup(&paths, mode);
    }

    // Capture whether this is a `tome telemetry` control-surface command BEFORE
    // `cli.command` is moved into the dispatch `match` below. Telemetry's own
    // control commands (`inspect`/`status`/`reset`/`purge`/…) must be INVISIBLE
    // to the `tome.error` boundary emit: appending a self-referential queue line
    // would (a) make telemetry self-instrument its own subsystem failures and
    // (b) violate `inspect`'s byte-identical / read-only guarantee (an exit-92
    // corrupt-queue report would otherwise grow the very file it just reported).
    let is_telemetry_cmd = matches!(cli.command, Command::Telemetry(_));

    // Whether the single-exit-path flusher teardown should run. It must NOT run
    // for `Command::Mcp` (the MCP server runs its OWN `tokio` interval flusher)
    // NOR `Command::Telemetry` — the detached `tome telemetry flush --quiet`
    // child IS a `Telemetry` command, so gating it OFF here is precisely what
    // stops a fork-bomb: the child never forks another flusher. Other telemetry
    // subcommands (`status`/`reset`/…) likewise shouldn't fork a flusher.
    let is_mcp_or_telemetry = matches!(cli.command, Command::Mcp(_) | Command::Telemetry(_));

    let result = match cli.command {
        Command::Init(_) => commands::init::run(&scope, mode),
        Command::Catalog(cmd) => commands::catalog::run(cmd, &scope, mode),
        Command::Plugin(args) => match args.command {
            Some(cmd) => commands::plugin::run(cmd, &scope, mode),
            None => commands::plugin::run_interactive(&scope, mode),
        },
        Command::Models(cmd) => commands::models::run(cmd, &scope, mode),
        Command::Query(args) => commands::query::run(args, &scope, mode),
        Command::Reindex(args) => commands::reindex::run(args, &scope, mode),
        Command::Status(args) => commands::status::run(args, &scope, mode),
        Command::Mcp(args) => commands::mcp::run(args, &scope, mode),
        Command::Workspace(args) => {
            commands::workspace::run(args.command, cli.scope.workspace.as_deref(), &scope, mode)
        }
        Command::Doctor(args) => commands::doctor::run(args, &scope, mode),
        Command::Harness(args) => commands::harness::run(args, &scope, mode),
        Command::Skill(cmd) => commands::skill::run(cmd, &scope, mode),
        Command::Meta(cmd) => commands::meta::run(cmd, &scope, mode),
        Command::Telemetry(cmd) => commands::telemetry::run(cmd, &scope, mode),
        Command::Tier(cmd) => commands::tier::run(cmd, &scope, mode),
        Command::Sync(args) => commands::sync::run(args, &scope, &paths, mode),
        Command::Config(cmd) => commands::config::run(cmd, &scope, mode),
        // Intercepted pre-dispatch above (before `Paths::resolve`); that arm
        // exits the process, so this is unreachable. Kept for exhaustiveness.
        Command::Completions(_) => unreachable!("completions is handled pre-dispatch"),
    };

    // Single exit-path teardown (FR-047b). `teardown_at_exit` is THE one call
    // site that spawns the detached telemetry flusher (a no-op stub today; US3
    // fills it). It runs in BOTH arms — after the exit code is computed and
    // after `write_error` on the error arm — but BEFORE `process::exit`, because
    // the release profile is `panic = "abort"` and runs no destructors, so a
    // `Drop`/`atexit` hook would never fire. The early `paths`/`scope`
    // resolution-failure exits above intentionally skip it (best-effort absence
    // is fine — there is nothing queued before a command runs).
    match result {
        Ok(()) => {
            if !is_mcp_or_telemetry {
                tome::telemetry::teardown_at_exit();
            }
            std::process::exit(0);
        }
        Err(err) => {
            let code = err.exit_code();
            output::write_error(mode, &err);
            // FR-029/029a: emit `tome.error` at the application boundary, carrying
            // ONLY the closed `ErrorCategory` (never the raw message) plus the CLI
            // surface. The command fns return `TomeError` directly here (the same
            // `&TomeError` `write_error` consumed), so we read `category()` rather
            // than downcasting an `anyhow::Error`. Best-effort: this enqueue must
            // not alter the exit code, produce user output, or block — `enqueue`
            // is the same infallible append. Placed AFTER `write_error` and BEFORE
            // teardown/exit. Only the error arm emits — a successful run does not.
            //
            // EXCEPT the `tome telemetry` control surface: those commands must be
            // invisible to the boundary (no self-instrumentation, and `inspect`'s
            // read-only / byte-identical guarantee stays intact — see the
            // `is_telemetry_cmd` capture above).
            if !is_telemetry_cmd {
                tome::telemetry::emit(tome::telemetry::event::ErrorEvent {
                    error_class: err.category(),
                    surface: tome::telemetry::event::Surface::Cli,
                    calling_harness: None,
                });
            }
            if !is_mcp_or_telemetry {
                tome::telemetry::teardown_at_exit();
            }
            std::process::exit(code);
        }
    }
}
