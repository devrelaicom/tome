//! Codex project → Tome IR importer (Tier 2 synthesis, FR-010 / R16).
//!
//! Codex has no plugin concept; a "Codex plugin" is **synthesized** from a
//! project's `.agents/skills/*` (each a skill) plus `config.toml [mcp_servers]`
//! (a Tome `.mcp.json`, transport inferred from `command` vs `url`). The
//! third-party `config.toml` is parsed **leniently** (`toml::Value`, no
//! `deny_unknown_fields`): an unrecognized field is a warning, never a parse
//! abort (the strictness boundary, principle IV).

use std::collections::BTreeMap;
use std::path::Path;

use super::claude_code::import_skill;
use super::rule;
use crate::authoring::ir::{Diagnostic, McpServerIr, McpTransport, PluginIr, Provenance};
use crate::authoring::untrusted::UntrustedRoot;
use crate::error::TomeError;
use crate::util::TOME_CONFIG_MAX;

/// Synthesize a [`PluginIr`] from a Codex project rooted at `root`.
pub fn import_project(root: &UntrustedRoot, source_path: &Path) -> Result<PluginIr, TomeError> {
    let mut diagnostics = Vec::new();

    let name = source_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("project")
        .to_owned();
    diagnostics.push(Diagnostic::info(
        rule::CODEX_SYNTHESIZED_VERSION,
        "Codex projects carry no version; synthesizing `0.0.0`",
    ));

    // --- skills from .agents/skills/* --------------------------------------
    let mut entries = Vec::new();
    let skills_dir = Path::new(".agents/skills");
    if root.is_dir(skills_dir) {
        for child in root.list_dir(skills_dir)? {
            if !child.is_dir {
                continue;
            }
            // Only directories that actually carry a SKILL.md are skills.
            if !root.is_file(&child.rel.join("SKILL.md")) {
                continue;
            }
            match import_skill(root, &child.rel, &child.name) {
                Ok(entry) => entries.push(entry),
                Err(e) => diagnostics.push(Diagnostic::warning(
                    rule::SKIPPED_ENTRY,
                    format!("skipped skill `{}`: {e}", child.name),
                )),
            }
        }
    }

    // --- MCP servers from config.toml [mcp_servers] ------------------------
    let mcp_servers = import_mcp(root, &mut diagnostics)?;

    // --- unsupported Codex extras ------------------------------------------
    for rel in ["agents/openai.yaml", ".agents/openai.yaml"] {
        if root.is_file(Path::new(rel)) {
            diagnostics.push(Diagnostic::warning(
                rule::CODEX_UNSUPPORTED,
                format!("`{rel}` (Codex agent extensions) is not representable in Tome; dropped"),
            ));
        }
    }

    Ok(PluginIr {
        name,
        version: "0.0.0".to_owned(),
        description: None,
        author: None,
        license: None,
        entries,
        mcp_servers,
        hooks_files: Vec::new(),
        hooks_json: None,
        mcp_json: None,
        provenance: Provenance {
            source_harness: "codex".to_owned(),
            source_path: source_path.to_path_buf(),
        },
        diagnostics,
    })
}

/// Parse `config.toml [mcp_servers]` into [`McpServerIr`]s, lenient.
fn import_mcp(
    root: &UntrustedRoot,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<McpServerIr>, TomeError> {
    if !root.is_file(Path::new("config.toml")) {
        return Ok(Vec::new());
    }
    let content = root.read_text(Path::new("config.toml"), TOME_CONFIG_MAX)?;
    let value: toml::Value = toml::from_str(&content)
        .map_err(|e| TomeError::Usage(format!("source config.toml is not valid TOML: {e}")))?;
    let Some(servers) = value.get("mcp_servers").and_then(|v| v.as_table()) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for (name, cfg) in servers {
        let Some(cfg) = cfg.as_table() else {
            diagnostics.push(Diagnostic::warning(
                rule::MALFORMED_MCP,
                format!("`[mcp_servers.{name}]` is not a table; skipping it"),
            ));
            continue;
        };
        // HTTP-MCP-only extras have no Tome equivalent.
        for extra in ["bearer_token_env_var", "env_http_headers"] {
            if cfg.contains_key(extra) {
                diagnostics.push(Diagnostic::warning(
                    rule::CODEX_UNSUPPORTED,
                    format!("MCP server `{name}` field `{extra}` (HTTP auth) has no Tome equivalent; dropped"),
                ));
            }
        }
        if let Some(command) = cfg.get("command").and_then(|v| v.as_str()) {
            let args = cfg
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let env = cfg
                .get("env")
                .and_then(|v| v.as_table())
                .map(|t| {
                    t.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            out.push(McpServerIr {
                name: name.clone(),
                transport: McpTransport::Stdio {
                    command: command.to_owned(),
                    args,
                    env,
                },
            });
        } else if let Some(url) = cfg.get("url").and_then(|v| v.as_str()) {
            out.push(McpServerIr {
                name: name.clone(),
                transport: McpTransport::Http {
                    url: url.to_owned(),
                },
            });
        } else {
            diagnostics.push(Diagnostic::warning(
                rule::MALFORMED_MCP,
                format!("Codex MCP server `{name}` has neither `command` nor `url`; skipping it"),
            ));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn codex_project(setup: impl FnOnce(&Path)) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let src = base.join("proj");
        fs::create_dir(&src).unwrap();
        setup(&src);
        (tmp, src)
    }

    #[test]
    fn synthesizes_a_plugin_from_skills_and_mcp() {
        let (_t, src) = codex_project(|src| {
            fs::create_dir_all(src.join(".agents/skills/helper")).unwrap();
            fs::write(
                src.join(".agents/skills/helper/SKILL.md"),
                "---\nname: helper\ndescription: helps\n---\nbody\n",
            )
            .unwrap();
            fs::write(
                src.join("config.toml"),
                "[mcp_servers.local]\ncommand = \"node\"\nargs = [\"s.js\"]\n\n[mcp_servers.remote]\nurl = \"https://x/mcp\"\n",
            )
            .unwrap();
        });
        let root = UntrustedRoot::open(&src).unwrap();
        let p = import_project(&root, &src).unwrap();
        assert_eq!(p.name, "proj");
        assert_eq!(p.version, "0.0.0");
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].name, "helper");
        assert_eq!(p.mcp_servers.len(), 2);
        assert_eq!(p.mcp_servers[0].name, "local");
        assert!(matches!(
            p.mcp_servers[0].transport,
            McpTransport::Stdio { .. }
        ));
        assert!(matches!(
            p.mcp_servers[1].transport,
            McpTransport::Http { .. }
        ));
    }

    #[test]
    fn unknown_mcp_field_warns_not_aborts() {
        // An unrecognized HTTP-auth field must be a warning, not a parse abort.
        let (_t, src) = codex_project(|src| {
            fs::create_dir_all(src.join(".agents/skills")).unwrap();
            fs::write(
                src.join("config.toml"),
                "[mcp_servers.remote]\nurl = \"https://x/mcp\"\nbearer_token_env_var = \"TOK\"\n",
            )
            .unwrap();
        });
        let root = UntrustedRoot::open(&src).unwrap();
        let p = import_project(&root, &src).unwrap();
        assert_eq!(p.mcp_servers.len(), 1);
        assert!(
            p.diagnostics
                .iter()
                .any(|d| d.rule_id == rule::CODEX_UNSUPPORTED),
            "{:?}",
            p.diagnostics
        );
    }

    #[test]
    fn invalid_config_toml_is_a_usage_error() {
        let (_t, src) = codex_project(|src| {
            fs::create_dir_all(src.join(".agents/skills")).unwrap();
            fs::write(src.join("config.toml"), "this = = bad toml").unwrap();
        });
        let root = UntrustedRoot::open(&src).unwrap();
        let err = import_project(&root, &src).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn no_config_no_skills_yields_an_empty_synthesized_plugin() {
        let (_t, src) = codex_project(|src| {
            fs::create_dir_all(src.join(".agents/skills")).unwrap();
        });
        let root = UntrustedRoot::open(&src).unwrap();
        let p = import_project(&root, &src).unwrap();
        assert!(p.entries.is_empty());
        assert!(p.mcp_servers.is_empty());
    }
}
