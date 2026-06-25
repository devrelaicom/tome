//! Library surface for the `tome` binary. Integration tests under `tests/`
//! consume this; the binary entry point in `src/main.rs` is a thin shell that
//! delegates here.

pub mod authoring;
pub mod catalog;
pub mod cli;
pub mod commands;
pub mod config;
pub mod doctor;
pub mod embedding;
pub mod error;
pub mod harness;
pub mod index;
pub mod logging;
pub mod mcp;
pub mod output;
pub mod paths;
pub mod plugin;
pub mod presentation;
pub mod provider;
pub mod settings;
pub mod substitution;
pub mod summarise;
pub mod telemetry;
pub mod util;
pub mod workspace;

pub use util::atomic_dir::{land_directory, land_directory_with_replace};
