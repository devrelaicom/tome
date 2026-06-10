//! Source-format detection for `convert` (FR-009, R19).
//!
//! Structural signals decide the source **harness** and the artifact **level**:
//!
//! | Signal (at the source root) | Harness | Level |
//! |---|---|---|
//! | `.claude-plugin/marketplace.json` | Claude Code | Catalog |
//! | `.claude-plugin/plugin.json` | Claude Code | Plugin |
//! | BOTH `.claude-plugin/marketplace.json` + `plugin.json` | Claude Code | The command's expected level (tie-break) |
//! | `.agents/skills/` directory | Codex | Plugin (synthesized) |
//! | `SKILL.md` | Agent Skills (generic) | Skill |
//!
//! A `--from <harness>` flag overrides the *harness* interpretation (e.g. a
//! bare `SKILL.md` is generic Agent Skills unless `--from cursor` marks it a
//! Cursor skill); the *level* still comes from structure. The three commands
//! each pass the level they expect (`catalog`/`plugin`/`skill convert`):
//!
//! * undetectable + no `--from` → [`SourceFormatUnrecognized`](crate::error::TomeError::SourceFormatUnrecognized) (83);
//! * detected level ≠ the command's expected level → [`Usage`](crate::error::TomeError::Usage) (2).
//!
//! All probing goes through [`UntrustedRoot`] so a symlinked marker cannot
//! redirect detection outside the source tree.

use std::path::Path;

use crate::authoring::untrusted::UntrustedRoot;
use crate::error::TomeError;

/// The source harness a `convert` reads from. The label doubles as the IR
/// provenance string surfaced in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceHarness {
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
    Cline,
    AgentSkills,
}

impl SourceHarness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::OpenCode => "opencode",
            Self::Cline => "cline",
            Self::AgentSkills => "agent-skills",
        }
    }

    /// Parse a `--from <harness>` value. Unknown values are a usage error.
    pub fn parse(value: &str) -> Result<Self, TomeError> {
        match value {
            "claude-code" | "claude" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "cursor" => Ok(Self::Cursor),
            "opencode" => Ok(Self::OpenCode),
            "cline" => Ok(Self::Cline),
            "agent-skills" | "agent" => Ok(Self::AgentSkills),
            other => Err(TomeError::Usage(format!(
                "unknown --from harness `{other}` (expected one of: claude-code, codex, cursor, opencode, cline, agent-skills)"
            ))),
        }
    }
}

/// The artifact level a source maps to (and the level each `convert` command
/// expects).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactLevel {
    Catalog,
    Plugin,
    Skill,
}

impl ArtifactLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Plugin => "plugin",
            Self::Skill => "skill",
        }
    }
}

/// The detection outcome: which harness, at which level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    pub harness: SourceHarness,
    pub level: ArtifactLevel,
}

/// Pure structural detection: probe the root for the known markers. Returns
/// `None` when no signal matches. `expected` breaks the tie when a repo
/// carries BOTH Claude Code manifests (the self-marketplace pattern).
fn detect_structural(root: &UntrustedRoot, expected: ArtifactLevel) -> Option<Detected> {
    let has_marketplace = root.is_file(Path::new(".claude-plugin/marketplace.json"));
    let has_plugin = root.is_file(Path::new(".claude-plugin/plugin.json"));
    if has_marketplace && has_plugin {
        // Self-marketplace repos (e.g. obra/superpowers) carry both manifests.
        // The invoking command's expected level wins: `plugin convert` reads
        // the plugin, anything else keeps the original marketplace-first
        // precedence.
        let level = if expected == ArtifactLevel::Plugin {
            ArtifactLevel::Plugin
        } else {
            ArtifactLevel::Catalog
        };
        return Some(Detected {
            harness: SourceHarness::ClaudeCode,
            level,
        });
    }
    if has_marketplace {
        return Some(Detected {
            harness: SourceHarness::ClaudeCode,
            level: ArtifactLevel::Catalog,
        });
    }
    if has_plugin {
        return Some(Detected {
            harness: SourceHarness::ClaudeCode,
            level: ArtifactLevel::Plugin,
        });
    }
    // A Codex "project" has no plugin concept — it's synthesized from its
    // `.agents/skills/` tree (+ `config.toml [mcp_servers]`). The skills dir is
    // the defining structural signal.
    if root.is_dir(Path::new(".agents/skills")) {
        return Some(Detected {
            harness: SourceHarness::Codex,
            level: ArtifactLevel::Plugin,
        });
    }
    if root.is_file(Path::new("SKILL.md")) {
        return Some(Detected {
            harness: SourceHarness::AgentSkills,
            level: ArtifactLevel::Skill,
        });
    }
    None
}

/// Detect a `convert` source's harness + level, honouring a `--from` override
/// and checking the result against the level the command expects.
///
/// * `from` — the optional `--from <harness>` override (overrides the harness
///   interpretation; the level still comes from structure).
/// * `expected` — the level the invoking command (`catalog`/`plugin`/`skill
///   convert`) requires.
///
/// # Errors
/// * [`TomeError::Usage`] (2) — an invalid `--from`, or a detected level that
///   does not match `expected`.
/// * [`TomeError::SourceFormatUnrecognized`] (83) — no structural signal and no
///   `--from` to fall back on.
pub fn detect(
    root: &UntrustedRoot,
    from: Option<&str>,
    expected: ArtifactLevel,
) -> Result<Detected, TomeError> {
    let structural = detect_structural(root, expected);

    let detected = match from {
        Some(value) => {
            let harness = SourceHarness::parse(value)?;
            // The override fixes the harness; the level still comes from
            // structure when a signal is present, else we trust the command's
            // expected level (the user asserted the format via `--from`).
            let level = structural.map(|d| d.level).unwrap_or(expected);
            Detected { harness, level }
        }
        None => structural.ok_or_else(|| TomeError::SourceFormatUnrecognized {
            path: root.root().to_path_buf(),
        })?,
    };

    if detected.level != expected {
        return Err(TomeError::Usage(format!(
            "`{expected} convert` expected a {expected} source but detected a {detected_level} ({harness}); use the matching command or `--from`",
            expected = expected.as_str(),
            detected_level = detected.level.as_str(),
            harness = detected.harness.as_str(),
        )));
    }

    Ok(detected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root_with(setup: impl FnOnce(&Path)) -> (tempfile::TempDir, UntrustedRoot) {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        setup(&base);
        let root = UntrustedRoot::open(&base).unwrap();
        (tmp, root)
    }

    #[test]
    fn detects_cc_marketplace_as_catalog() {
        let (_t, root) = root_with(|base| {
            fs::create_dir(base.join(".claude-plugin")).unwrap();
            fs::write(base.join(".claude-plugin/marketplace.json"), b"{}").unwrap();
        });
        let d = detect(&root, None, ArtifactLevel::Catalog).unwrap();
        assert_eq!(d.harness, SourceHarness::ClaudeCode);
        assert_eq!(d.level, ArtifactLevel::Catalog);
    }

    #[test]
    fn detects_cc_plugin() {
        let (_t, root) = root_with(|base| {
            fs::create_dir(base.join(".claude-plugin")).unwrap();
            fs::write(base.join(".claude-plugin/plugin.json"), b"{}").unwrap();
        });
        let d = detect(&root, None, ArtifactLevel::Plugin).unwrap();
        assert_eq!(d.harness, SourceHarness::ClaudeCode);
        assert_eq!(d.level, ArtifactLevel::Plugin);
    }

    #[test]
    fn detects_codex_project_as_plugin() {
        let (_t, root) = root_with(|base| {
            fs::create_dir_all(base.join(".agents/skills")).unwrap();
            fs::write(base.join("config.toml"), b"[mcp_servers]\n").unwrap();
        });
        let d = detect(&root, None, ArtifactLevel::Plugin).unwrap();
        assert_eq!(d.harness, SourceHarness::Codex);
        assert_eq!(d.level, ArtifactLevel::Plugin);
    }

    #[test]
    fn detects_native_skill() {
        let (_t, root) = root_with(|base| {
            fs::write(base.join("SKILL.md"), b"---\nname: foo\n---\nbody").unwrap();
        });
        let d = detect(&root, None, ArtifactLevel::Skill).unwrap();
        assert_eq!(d.harness, SourceHarness::AgentSkills);
        assert_eq!(d.level, ArtifactLevel::Skill);
    }

    #[test]
    fn from_overrides_skill_harness() {
        let (_t, root) = root_with(|base| {
            fs::write(base.join("SKILL.md"), b"body").unwrap();
        });
        let d = detect(&root, Some("cursor"), ArtifactLevel::Skill).unwrap();
        assert_eq!(d.harness, SourceHarness::Cursor);
        assert_eq!(d.level, ArtifactLevel::Skill);
    }

    #[test]
    fn unrecognized_without_from_is_83() {
        let (_t, root) = root_with(|base| {
            fs::write(base.join("README.md"), b"hi").unwrap();
        });
        let err = detect(&root, None, ArtifactLevel::Skill).unwrap_err();
        assert_eq!(err.exit_code(), 83);
    }

    #[test]
    fn level_mismatch_is_usage_2() {
        // A plugin source asked to be converted as a catalog.
        let (_t, root) = root_with(|base| {
            fs::create_dir(base.join(".claude-plugin")).unwrap();
            fs::write(base.join(".claude-plugin/plugin.json"), b"{}").unwrap();
        });
        let err = detect(&root, None, ArtifactLevel::Catalog).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn invalid_from_is_usage_2() {
        let (_t, root) = root_with(|base| {
            fs::write(base.join("SKILL.md"), b"body").unwrap();
        });
        let err = detect(&root, Some("nope"), ArtifactLevel::Skill).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn from_without_structural_signal_trusts_expected_level() {
        let (_t, root) = root_with(|base| {
            // No recognized marker, but the user asserts cursor (a skill).
            fs::write(base.join("notes.md"), b"hi").unwrap();
        });
        let d = detect(&root, Some("cursor"), ArtifactLevel::Skill).unwrap();
        assert_eq!(d.harness, SourceHarness::Cursor);
        assert_eq!(d.level, ArtifactLevel::Skill);
    }

    #[test]
    fn both_manifests_tie_break_to_the_commands_expected_level() {
        // Self-marketplace repos (obra/superpowers) carry BOTH manifests.
        let (_t, root) = root_with(|base| {
            fs::create_dir(base.join(".claude-plugin")).unwrap();
            fs::write(base.join(".claude-plugin/marketplace.json"), b"{}").unwrap();
            fs::write(base.join(".claude-plugin/plugin.json"), b"{}").unwrap();
        });
        // `plugin convert` wins the tie toward Plugin…
        let p = detect(&root, None, ArtifactLevel::Plugin).unwrap();
        assert_eq!(p.harness, SourceHarness::ClaudeCode);
        assert_eq!(p.level, ArtifactLevel::Plugin);
        // …`catalog convert` keeps the marketplace.
        let c = detect(&root, None, ArtifactLevel::Catalog).unwrap();
        assert_eq!(c.level, ArtifactLevel::Catalog);
        // A skill expectation keeps marketplace-first precedence → level mismatch.
        let err = detect(&root, None, ArtifactLevel::Skill).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }
}
