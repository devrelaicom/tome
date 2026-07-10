//! Template scaffolding for `create` — build a new native Tome artifact from a
//! built-in (or, later, fetched) template, rendered via **minijinja** with the
//! variable set `{ name, plugin_name, version, description, author_name, date }`.
//!
//! Built-in templates are embedded (offline). The manifest fields are
//! structured (built from the args); the entry **body** is the rendered
//! template. minijinja's `{{ }}` delimiters never collide with the emitted
//! runtime `${TOME_*}`/`$ARGUMENTS` tokens, so those survive verbatim.
//!
//! The `name == directory` invariant is enforced (the artifact's final name is
//! also its emitted directory), and a freshly-scaffolded artifact is lint-clean
//! by construction (kebab name, semver version, a present description).
//!
//! ## Component kinds (G9)
//!
//! Beyond skills, `create` can scaffold three additional component kinds via
//! [`ScaffoldComponent`]:
//!
//! - **Command** (`commands/<name>.md`) — a user-invocable slash command.
//! - **Agent** (`agents/<name>.md`) — a sub-agent entry.
//! - **Hooks** (`hooks/hooks.json`) — a minimal hooks manifest stub.
//! - **Mcp** (`.mcp.json`) — a `.mcp.json` MCP server stub.
//!
//! All four are plugin-level structures and therefore always emitted inside a
//! wrapping plugin (unlike `--bare` skills).

use minijinja::{Environment, context};

use crate::authoring::detect::ArtifactLevel;
use crate::authoring::ir::{
    Artifact, CatalogIr, EntryIr, MappedFrontmatter, McpServerIr, McpTransport, PluginIr,
    Provenance, SupportingFile,
};
use crate::catalog::manifest::Owner;
use crate::error::TomeError;
use crate::plugin::identity::{EntryKind, is_kebab, validate_segment};
use crate::plugin::manifest::TomeAuthor;

const DEFAULT_VERSION: &str = "0.1.0";

/// Placeholder catalog-owner name when no `--author` is given. Non-empty so the
/// scaffold is lint-clean (the `owner-missing` rule fires only when BOTH name
/// and email are blank); clearly signals the user should edit it.
const PLACEHOLDER_OWNER: &str = "Your Name";

/// The built-in default skill `SKILL.md` body template.
const SKILL_BODY_TEMPLATE: &str = "# {{ name }}\n\n{{ description }}\n\n## When to use\n\nDescribe when the `{{ name }}` skill applies.\n\n## Instructions\n\nDescribe what `{{ name }}` does. Scaffolded by `tome` on {{ date }}.\n";

/// The built-in default command body template (`commands/<name>.md`).
const COMMAND_BODY_TEMPLATE: &str = "# {{ name }}\n\n{{ description }}\n\nDescribe what the `{{ name }}` command does.\n\n## Usage\n\n```\n/{{ name }}\n```\n\nScaffolded by `tome` on {{ date }}.\n";

/// The built-in default agent body template (`agents/<name>.md`).
const AGENT_BODY_TEMPLATE: &str = "# {{ name }}\n\n{{ description }}\n\nDescribe what the `{{ name }}` agent does, when it should run, and what tools it may use.\n\nScaffolded by `tome` on {{ date }}.\n";

/// The built-in `hooks/hooks.json` stub: a single `SessionStart` hook that
/// runs an example shell script via `sh`. Using `sh` as the command avoids
/// requiring the script to be marked executable (`chmod +x`), so the scaffold
/// is immediately runnable without a post-create setup step.
/// The `${TOME_PLUGIN_ROOT}` token is preserved verbatim — it is rewritten at
/// harness-sync time.
const HOOKS_JSON_TEMPLATE: &str = r#"{
  "SessionStart": [
    {
      "hooks": [
        {
          "type": "command",
          "command": "sh",
          "args": ["${TOME_PLUGIN_ROOT}/hooks/on-start.sh"]
        }
      ]
    }
  ]
}
"#;

/// The `hooks/on-start.sh` stub emitted alongside `hooks/hooks.json`.
const HOOKS_SCRIPT_TEMPLATE: &str = "#!/bin/sh\n# {{ name }} — SessionStart hook.\n# Scaffolded by `tome` on {{ date }}.\n# Implement your session-start logic here.\n";

/// The built-in `.mcp.json` stub: one placeholder stdio MCP server entry.
/// The server name and command use the plugin name so the scaffold is
/// immediately meaningful without further edits. The `"env"` field is omitted
/// — the emitter strips empty-env objects anyway, so including it would cause
/// the template to differ from what gets written to disk.
const MCP_JSON_TEMPLATE: &str = r#"{
  "mcpServers": {
    "{{ name }}": {
      "command": "node",
      "args": ["server.js"]
    }
  }
}
"#;

/// What component to scaffold inside (or alongside) a plugin.
///
/// The default for `tome plugin create` / `tome skill create` is [`ScaffoldComponent::Skill`],
/// which matches the prior behaviour. The additional kinds are enabled via
/// `--kind` on the plugin create surface (G9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScaffoldComponent {
    /// A skill entry (`skills/<name>/SKILL.md`) — the existing default.
    #[default]
    Skill,
    /// A command entry (`commands/<name>.md`).
    Command,
    /// An agent entry (`agents/<name>.md`).
    Agent,
    /// A hooks stub (`hooks/hooks.json` + `hooks/on-start.sh`).
    Hooks,
    /// An MCP server stub (`.mcp.json`).
    Mcp,
}

impl ScaffoldComponent {
    /// Parse the `--kind` CLI value.
    pub fn parse_kind(s: &str) -> Option<Self> {
        match s {
            "skill" => Some(Self::Skill),
            "command" => Some(Self::Command),
            "agent" => Some(Self::Agent),
            "hooks" => Some(Self::Hooks),
            "mcp" => Some(Self::Mcp),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Command => "command",
            Self::Agent => "agent",
            Self::Hooks => "hooks",
            Self::Mcp => "mcp",
        }
    }
}

/// Inputs to a `create`.
#[derive(Debug, Clone)]
pub struct CreateParams {
    /// The `<NAME>` positional (the skill/plugin/catalog name).
    pub name: String,
    /// (skill) the wrapping plugin name; `None` → `name`.
    pub plugin_name: Option<String>,
    /// `--description`; `None` → a default derived from `name`.
    pub description: Option<String>,
    /// `[author] name`; `None` → empty.
    pub author_name: Option<String>,
    /// Today's date (`YYYY-MM-DD`), supplied by the caller for determinism.
    pub date: String,
    /// (skill) `--bare`: emit a naked skill instead of wrapping it in a plugin.
    pub bare: bool,
    /// What to scaffold inside the plugin. Defaults to [`ScaffoldComponent::Skill`]
    /// for backwards-compatibility. Ignored at the catalog level and when `bare`
    /// is true (bare always produces a naked skill).
    pub component: ScaffoldComponent,
}

/// Build the IR + the artifact's final name (also its emitted directory) for a
/// `create` at the given level. The result is lint-clean by construction.
pub fn create_artifact(
    level: ArtifactLevel,
    params: &CreateParams,
) -> Result<(Artifact, String), TomeError> {
    match level {
        ArtifactLevel::Catalog => {
            let name = validated_name(&params.name)?;
            let catalog = CatalogIr {
                name: name.clone(),
                version: DEFAULT_VERSION.to_owned(),
                description: description_for(params),
                owner: Owner {
                    name: params
                        .author_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| PLACEHOLDER_OWNER.to_owned()),
                    email: String::new(),
                },
                plugins: Vec::new(),
                provenance: Provenance::local("scaffold", std::path::PathBuf::from(&name)),
                diagnostics: Vec::new(),
            };
            Ok((Artifact::Catalog(catalog), name))
        }
        ArtifactLevel::Plugin => {
            let name = validated_name(&params.name)?;
            let plugin = scaffold_plugin_for_component(&name, params)?;
            Ok((Artifact::Plugin(plugin), name))
        }
        ArtifactLevel::Skill if params.bare => {
            let name = validated_name(&params.name)?;
            let entry = scaffold_skill_entry(&name, params)?;
            Ok((Artifact::Skill(entry), name))
        }
        ArtifactLevel::Skill => {
            // Plugin-wrapped: a minimal plugin (named `plugin_name`, default
            // `name`) holding the skill `name`. The artifact directory is the
            // PLUGIN name (preserving plugin name == dir); the skill lands at
            // `skills/<name>/`.
            let skill_name = validated_name(&params.name)?;
            let plugin_name =
                validated_name(params.plugin_name.as_deref().unwrap_or(&params.name))?;
            let entry = scaffold_skill_entry(&skill_name, params)?;
            Ok((
                Artifact::Plugin(plugin_ir(&plugin_name, vec![entry], params)),
                plugin_name,
            ))
        }
    }
}

/// Build a [`PluginIr`] for the requested [`ScaffoldComponent`]. For entry
/// kinds (`Skill` / `Command` / `Agent`) this creates a plugin wrapping one
/// entry of that kind; for structural kinds (`Hooks` / `Mcp`) it creates an
/// empty-entry plugin with the structure filed in the appropriate slot.
fn scaffold_plugin_for_component(
    plugin_name: &str,
    params: &CreateParams,
) -> Result<PluginIr, TomeError> {
    match params.component {
        ScaffoldComponent::Skill => {
            let entry = scaffold_skill_entry(plugin_name, params)?;
            Ok(plugin_ir(plugin_name, vec![entry], params))
        }
        ScaffoldComponent::Command => {
            let entry = scaffold_command_entry(plugin_name, params)?;
            Ok(plugin_ir(plugin_name, vec![entry], params))
        }
        ScaffoldComponent::Agent => {
            let entry = scaffold_agent_entry(plugin_name, params)?;
            Ok(plugin_ir(plugin_name, vec![entry], params))
        }
        ScaffoldComponent::Hooks => scaffold_hooks_plugin(plugin_name, params),
        ScaffoldComponent::Mcp => scaffold_mcp_plugin(plugin_name, params),
    }
}

fn plugin_ir(name: &str, entries: Vec<EntryIr>, params: &CreateParams) -> PluginIr {
    PluginIr {
        name: name.to_owned(),
        version: DEFAULT_VERSION.to_owned(),
        description: Some(description_for(params)),
        // An empty/whitespace-only `--author` is treated as absent (NO
        // `[author]` table), byte-identical to omitting the flag — the same
        // trim+empty-filter the catalog owner path uses, so a blank author can
        // never emit a lint-tripping `name = ""`.
        author: params
            .author_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| TomeAuthor {
                name: Some(n.to_owned()),
                email: None,
            }),
        license: None,
        entries,
        mcp_servers: Vec::new(),
        hooks_files: Vec::new(),
        hooks_json: None,
        mcp_json: None,
        provenance: Provenance::local("scaffold", std::path::PathBuf::from(name)),
        diagnostics: Vec::new(),
    }
}

/// Build the default skill `EntryIr` (frontmatter `name`/`description` + the
/// rendered body).
fn scaffold_skill_entry(skill_name: &str, params: &CreateParams) -> Result<EntryIr, TomeError> {
    let description = description_for(params);
    let body = render(SKILL_BODY_TEMPLATE, skill_name, &description, params)?;
    Ok(EntryIr {
        kind: EntryKind::Skill,
        name: skill_name.to_owned(),
        description: Some(description),
        frontmatter: MappedFrontmatter::default(),
        agent_meta: None,
        body,
        supporting_files: Vec::new(),
        source_path: std::path::PathBuf::new(),
        diagnostics: Vec::new(),
    })
}

/// Build a command `EntryIr` (`commands/<name>.md`).
fn scaffold_command_entry(command_name: &str, params: &CreateParams) -> Result<EntryIr, TomeError> {
    let description = description_for(params);
    let body = render(COMMAND_BODY_TEMPLATE, command_name, &description, params)?;
    Ok(EntryIr {
        kind: EntryKind::Command,
        name: command_name.to_owned(),
        description: Some(description),
        frontmatter: MappedFrontmatter::default(),
        agent_meta: None,
        body,
        supporting_files: Vec::new(),
        source_path: std::path::PathBuf::new(),
        diagnostics: Vec::new(),
    })
}

/// Build an agent `EntryIr` (`agents/<name>.md`).
fn scaffold_agent_entry(agent_name: &str, params: &CreateParams) -> Result<EntryIr, TomeError> {
    let description = description_for(params);
    let body = render(AGENT_BODY_TEMPLATE, agent_name, &description, params)?;
    Ok(EntryIr {
        kind: EntryKind::Agent,
        name: agent_name.to_owned(),
        description: Some(description),
        frontmatter: MappedFrontmatter::default(),
        agent_meta: None,
        body,
        supporting_files: Vec::new(),
        source_path: std::path::PathBuf::new(),
        diagnostics: Vec::new(),
    })
}

/// Build a plugin IR carrying a hooks stub (`hooks/hooks.json` +
/// `hooks/on-start.sh`) but no entries. The hooks files are carried as
/// `hooks_files` (verbatim pass-through) so `emit` copies them alongside the
/// manifest without touching the entry slots. The `hooks_json` body is also
/// set so that the `lint/hooks-spec` rule can validate the stub at lint time.
///
/// The generated file content is written to a temporary directory that is
/// **persisted on disk** via `TempDir::keep()`. The scaffold code path
/// runs at most once per `tome plugin create --kind hooks` invocation and the
/// generated bytes (~200 B total) are negligible; the OS temp-cleaner handles
/// them eventually. The persisted path outlives the `emit` call that reads
/// from it, so no use-after-free is possible.
fn scaffold_hooks_plugin(plugin_name: &str, params: &CreateParams) -> Result<PluginIr, TomeError> {
    let description = description_for(params);
    let hooks_json = render(HOOKS_JSON_TEMPLATE, plugin_name, &description, params)?;
    let hooks_script = render(HOOKS_SCRIPT_TEMPLATE, plugin_name, &description, params)?;

    // Persist the temp dir: `keep()` releases the handle without deleting the
    // directory, returning the `PathBuf` that outlives this function. Paths are
    // derived from the returned `PathBuf` (not from `tmp_dir.path()`, which is
    // gone after the call) — the OS temp-cleaner handles eventual cleanup.
    let tmp_dir = tempfile::Builder::new()
        .prefix("tome-hooks-scaffold-")
        .tempdir()
        .map_err(TomeError::Io)?;
    let tmp_path = tmp_dir.keep();
    let hooks_json_path = tmp_path.join("hooks.json");
    let hooks_script_path = tmp_path.join("on-start.sh");
    std::fs::write(&hooks_json_path, hooks_json.as_bytes()).map_err(TomeError::Io)?;
    std::fs::write(&hooks_script_path, hooks_script.as_bytes()).map_err(TomeError::Io)?;

    Ok(PluginIr {
        name: plugin_name.to_owned(),
        version: DEFAULT_VERSION.to_owned(),
        description: Some(description),
        author: params
            .author_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| TomeAuthor {
                name: Some(n.to_owned()),
                email: None,
            }),
        license: None,
        entries: Vec::new(),
        mcp_servers: Vec::new(),
        hooks_files: vec![
            SupportingFile {
                relative: std::path::PathBuf::from("hooks").join("hooks.json"),
                source: hooks_json_path,
            },
            SupportingFile {
                relative: std::path::PathBuf::from("hooks").join("on-start.sh"),
                source: hooks_script_path,
            },
        ],
        hooks_json: Some(hooks_json),
        mcp_json: None,
        provenance: Provenance::local("scaffold", std::path::PathBuf::from(plugin_name)),
        diagnostics: Vec::new(),
    })
}

/// Build a plugin IR with a `.mcp.json` stub (one placeholder stdio server).
/// The server name matches the plugin name so the stub is immediately meaningful.
fn scaffold_mcp_plugin(plugin_name: &str, params: &CreateParams) -> Result<PluginIr, TomeError> {
    let description = description_for(params);
    let mcp_json_body = render(MCP_JSON_TEMPLATE, plugin_name, &description, params)?;
    // Parse the rendered JSON through the MCP server IR so `emit` writes it
    // via the standard `.mcp.json` emission path (which serialises from
    // `McpServerIr`). Parse the JSON manually to extract the server name and
    // command.
    let parsed: serde_json::Value = serde_json::from_str(&mcp_json_body).map_err(|e| {
        TomeError::Internal(anyhow::anyhow!(
            "scaffold_mcp_plugin: template rendered invalid JSON: {e}"
        ))
    })?;
    let servers = parsed
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            TomeError::Internal(anyhow::anyhow!("scaffold_mcp_plugin: missing mcpServers"))
        })?;

    let mut mcp_servers = Vec::new();
    for (name, srv) in servers {
        let command = srv
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("node")
            .to_owned();
        let args = srv
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a.as_str())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        mcp_servers.push(McpServerIr {
            name: name.clone(),
            transport: McpTransport::Stdio {
                command,
                args,
                env: std::collections::BTreeMap::new(),
            },
        });
    }

    Ok(PluginIr {
        name: plugin_name.to_owned(),
        version: DEFAULT_VERSION.to_owned(),
        description: Some(description),
        author: params
            .author_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| TomeAuthor {
                name: Some(n.to_owned()),
                email: None,
            }),
        license: None,
        entries: Vec::new(),
        mcp_servers,
        hooks_files: Vec::new(),
        hooks_json: None,
        mcp_json: Some(mcp_json_body),
        provenance: Provenance::local("scaffold", std::path::PathBuf::from(plugin_name)),
        diagnostics: Vec::new(),
    })
}

/// Render a template with the scaffold variable set.
fn render(
    template: &str,
    name: &str,
    description: &str,
    params: &CreateParams,
) -> Result<String, TomeError> {
    let mut env = Environment::new();
    env.add_template("t", template).map_err(template_err)?;
    let tmpl = env.get_template("t").map_err(template_err)?;
    tmpl.render(context! {
        name => name,
        plugin_name => params.plugin_name.clone().unwrap_or_else(|| name.to_owned()),
        version => DEFAULT_VERSION,
        description => description,
        author_name => params.author_name.clone().unwrap_or_default(),
        date => params.date,
    })
    .map_err(template_err)
}

fn template_err(e: minijinja::Error) -> TomeError {
    TomeError::TemplateInvalid {
        template: "built-in".to_owned(),
        reason: e.to_string(),
    }
}

/// A non-empty description (default derived from `name`).
fn description_for(params: &CreateParams) -> String {
    params
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("The {} scaffold.", params.name))
}

/// Validate the artifact name is a safe path segment AND kebab-case, so the
/// scaffolded artifact lints clean (and `name == dir` holds). `is_kebab` is the
/// shared SSOT in `plugin::identity` — the same predicate the lint rules use,
/// so a scaffolded name can never be one lint would reject.
fn validated_name(name: &str) -> Result<String, TomeError> {
    validate_segment(name).map_err(|kind| {
        TomeError::Usage(format!("`{name}` is not a valid artifact name: {kind}"))
    })?;
    if !is_kebab(name) {
        return Err(TomeError::Usage(format!(
            "`{name}` is not kebab-case (lowercase letters/digits and single hyphens)"
        )));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(name: &str) -> CreateParams {
        CreateParams {
            name: name.to_owned(),
            plugin_name: None,
            description: None,
            author_name: None,
            date: "2026-06-08".to_owned(),
            bare: false,
            component: ScaffoldComponent::Skill,
        }
    }

    #[test]
    fn scaffolds_a_plugin_wrapped_skill_by_default() {
        let (artifact, dir) = create_artifact(ArtifactLevel::Skill, &params("review")).unwrap();
        assert_eq!(dir, "review");
        match artifact {
            Artifact::Plugin(p) => {
                assert_eq!(p.name, "review");
                assert_eq!(p.entries.len(), 1);
                assert_eq!(p.entries[0].name, "review");
                assert!(p.entries[0].body.contains("# review"));
                assert!(p.entries[0].description.is_some());
            }
            other => panic!("expected a plugin, got {other:?}"),
        }
    }

    #[test]
    fn plugin_name_overrides_the_wrapping_plugin_and_dir() {
        let mut pa = params("review");
        pa.plugin_name = Some("qa".to_owned());
        let (artifact, dir) = create_artifact(ArtifactLevel::Skill, &pa).unwrap();
        assert_eq!(dir, "qa", "the dir is the plugin name (name == dir)");
        match artifact {
            Artifact::Plugin(p) => {
                assert_eq!(p.name, "qa");
                assert_eq!(p.entries[0].name, "review"); // qa:review
            }
            other => panic!("expected a plugin, got {other:?}"),
        }
    }

    #[test]
    fn bare_skill_is_a_naked_entry() {
        let mut pa = params("review");
        pa.bare = true;
        let (artifact, dir) = create_artifact(ArtifactLevel::Skill, &pa).unwrap();
        assert_eq!(dir, "review");
        assert!(matches!(artifact, Artifact::Skill(_)));
    }

    #[test]
    fn plugin_and_catalog_scaffold() {
        let (a, _) = create_artifact(ArtifactLevel::Plugin, &params("toolkit")).unwrap();
        assert!(matches!(a, Artifact::Plugin(_)));
        let (c, _) = create_artifact(ArtifactLevel::Catalog, &params("my-catalog")).unwrap();
        assert!(matches!(c, Artifact::Catalog(_)));
    }

    #[test]
    fn rejects_a_non_kebab_name() {
        let err = create_artifact(ArtifactLevel::Skill, &params("Not_Kebab")).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn empty_or_whitespace_author_is_treated_as_absent() {
        // #325 review Minor: a blank/whitespace `--author` must NOT emit an
        // `[author]` table with `name = ""` — it is treated as absent, matching
        // the catalog owner path's trim+empty-filter (and lint-clean goal). A
        // real author still populates the table.
        for blank in ["", "   ", "\t"] {
            let mut pa = params("toolkit");
            pa.author_name = Some(blank.to_owned());
            let (artifact, _) = create_artifact(ArtifactLevel::Plugin, &pa).unwrap();
            match artifact {
                Artifact::Plugin(p) => assert!(
                    p.author.is_none(),
                    "blank author {blank:?} must yield no [author] table"
                ),
                other => panic!("expected a plugin, got {other:?}"),
            }
        }

        let mut pa = params("toolkit");
        pa.author_name = Some("Acme".to_owned());
        let (artifact, _) = create_artifact(ArtifactLevel::Plugin, &pa).unwrap();
        match artifact {
            Artifact::Plugin(p) => {
                assert_eq!(p.author.and_then(|a| a.name).as_deref(), Some("Acme"))
            }
            other => panic!("expected a plugin, got {other:?}"),
        }
    }
}
