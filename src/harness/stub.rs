//! Deterministic test-only `HarnessModule`.
//!
//! This module ships in the library so integration tests under
//! `tests/` can import it (they consume the library as an external
//! crate and therefore cannot reach `#[cfg(test)]`-gated hooks). It is
//! NOT registered in [`super::SUPPORTED_HARNESSES`]; production code
//! never references it. The first consumer is US1.b-3's harness sync
//! tests, which install it via [`super::HARNESS_MODULES_OVERRIDE`] to
//! exercise the dispatch path without binding to any of the five real
//! harnesses.
//!
//! Behaviour:
//!
//! - `name()`            → `"stub"`
//! - `description()`     → `"deterministic test-only harness"`
//! - `detect()`          → always `true`
//! - `rules_file_target` → `<project>/STUB_RULES.md`
//! - `rules_file_strategy` → `BlockInExistingFile`
//! - `block_body_style`  → `Inline`
//! - `mcp_config_path`   → `<project>/stub.mcp.json`
//! - `mcp_config_format` → `Json`
//! - `mcp_parent_key`    → `"mcpServers"`

use std::path::{Path, PathBuf};

use crate::harness::{BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for tests.
#[doc(hidden)]
pub struct StubHarness;

impl HarnessModule for StubHarness {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn description(&self) -> &'static str {
        "deterministic test-only harness"
    }

    fn detect(&self, _home: &Path) -> bool {
        true
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("STUB_RULES.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("stub.mcp.json")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}
