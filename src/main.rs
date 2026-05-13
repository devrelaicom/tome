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
        Command::Plugin(cmd) => commands::plugin::run(cmd, mode),
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
