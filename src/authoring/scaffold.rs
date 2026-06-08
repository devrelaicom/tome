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

use minijinja::{Environment, context};

use crate::authoring::detect::ArtifactLevel;
use crate::authoring::ir::{Artifact, CatalogIr, EntryIr, MappedFrontmatter, PluginIr, Provenance};
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
            let entry = scaffold_skill_entry(&name, params)?;
            Ok((
                Artifact::Plugin(plugin_ir(&name, vec![entry], params)),
                name,
            ))
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

fn plugin_ir(name: &str, entries: Vec<EntryIr>, params: &CreateParams) -> PluginIr {
    PluginIr {
        name: name.to_owned(),
        version: DEFAULT_VERSION.to_owned(),
        description: Some(description_for(params)),
        author: params.author_name.as_ref().map(|n| TomeAuthor {
            name: Some(n.clone()),
            email: None,
        }),
        license: None,
        entries,
        mcp_servers: Vec::new(),
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
        body,
        supporting_files: Vec::new(),
        source_path: std::path::PathBuf::new(),
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
}
