use clap::Parser;

use tome::catalog::git;
use tome::cli::{Cli, Command};
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
    logging::init(cli.verbosity());
    git::install_signal_handler();

    let mode = cli.mode();

    // Phase 3 / Foundational F3: resolve the active scope before any
    // command runs. Errors here (workspace conflict, workspace not
    // found) flow through the same exit-code path as command errors.
    let scope = match workspace::resolution::resolve(&cli.scope) {
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
