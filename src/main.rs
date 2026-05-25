use clap::Parser;

use tome::catalog::git;
use tome::cli::{Cli, Command};
use tome::paths;
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
        commands::status::print_version(json);
        std::process::exit(0);
    }

    let cli = Cli::parse();
    // Skip the stderr-based CLI tracing subscriber on the MCP path —
    // `mcp::run` installs its own file-backed JSON subscriber, and the
    // global `tracing` registry only accepts one. Also skip the
    // Ctrl-C signal handler: the MCP server uses tokio's async
    // `signal::ctrl_c()` instead, and the CLI handler would race.
    if !matches!(cli.command, Command::Mcp(_)) {
        logging::init(cli.verbosity());
        git::install_signal_handler();
    }

    let mode = cli.mode();

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
    let scope = match workspace::resolution::resolve(&cli.scope, &paths) {
        Ok(r) => r,
        Err(err) => {
            let code = err.exit_code();
            output::write_error(mode, &err);
            std::process::exit(code);
        }
    };

    let result = match cli.command {
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
    };

    match result {
        Ok(()) => std::process::exit(0),
        Err(err) => {
            let code = err.exit_code();
            output::write_error(mode, &err);
            std::process::exit(code);
        }
    }
}
