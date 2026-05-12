//! Library surface for the `tome` binary. Integration tests under `tests/`
//! consume this; the binary entry point in `src/main.rs` is a thin shell that
//! delegates here.

pub mod catalog;
pub mod cli;
pub mod commands;
pub mod config;
pub mod error;
pub mod index;
pub mod logging;
pub mod output;
pub mod paths;
pub mod plugin;
pub mod presentation;
