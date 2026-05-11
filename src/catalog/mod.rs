//! The catalog module: manifest schema + parsing, Git shell-outs with
//! credential scrubbing at the process-output boundary, and the registry
//! store that persists `config.toml` atomically.

pub mod git;
pub mod manifest;
pub mod store;
