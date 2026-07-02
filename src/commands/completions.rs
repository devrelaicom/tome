//! `tome completions <shell>` (issue #322).
//!
//! Generates a shell-completion script on stdout via `clap_complete::generate`
//! over the already-derived [`crate::cli::Cli`] command tree. This is a pure,
//! static operation: it reads no HOME, index, config, or workspace state, so a
//! user can run it during shell setup before Tome is otherwise configured. The
//! pre-dispatch interception in `main.rs` runs this BEFORE `Paths::resolve()` to
//! preserve that property.
//!
//! The binary name is hardcoded to `tome` to match `[[bin]] name = "tome"` — the
//! generated script's function/`#compdef` names key off it, so it must be the
//! installed command name regardless of how the crate is published.

use clap::CommandFactory;

use crate::cli::CompletionsArgs;
use crate::error::TomeError;

/// Write the completion script for `args.shell` to stdout. Infallible in
/// practice (`generate` writes to the `stdout` handle and returns `()`); the
/// `Result` keeps the handler shape uniform with every other command dispatch.
pub fn run(args: &CompletionsArgs) -> Result<(), TomeError> {
    let mut cmd = crate::cli::Cli::command();
    clap_complete::generate(args.shell, &mut cmd, "tome", &mut std::io::stdout());
    Ok(())
}
