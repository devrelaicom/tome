use clap::Parser;

use tome::catalog::git;
use tome::cli::{Cli, Command};
use tome::{commands, logging, output};

fn main() {
    let cli = Cli::parse();
    logging::init(cli.verbosity());
    git::install_signal_handler();

    let mode = cli.mode();
    let result = match cli.command {
        Command::Catalog(cmd) => commands::catalog::run(cmd, mode),
        Command::Plugin(args) => match args.command {
            Some(cmd) => commands::plugin::run(cmd, mode),
            None => commands::plugin::run_interactive(mode),
        },
        Command::Models(cmd) => commands::models::run(cmd, mode),
        Command::Query(args) => commands::query::run(args, mode),
        Command::Reindex(args) => commands::reindex::run(args, mode),
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
