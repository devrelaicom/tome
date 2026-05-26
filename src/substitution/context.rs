//! `SubstitutionContext` + builder + `ArgumentValues`.
//!
//! Per data-model.md §3.2. The builder pattern is the sole public
//! construction surface; required fields fail at `.build()` with a
//! descriptive error.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::paths::Paths;

use super::SubstitutionError;

/// Fully-resolved context for one substitution pass over an entry body.
///
/// Built via [`SubstitutionContext::builder`]; all fields except `args`
/// are required. `declared_args` is the entry's declared argument names
/// in declaration order — used by [`super::arguments::apply_arguments`]
/// to map positional `$N` references and to emit the `ARGUMENTS:` tail
/// when caller-supplied args were not consumed by the body.
pub struct SubstitutionContext {
    // Built-in values (R-9 paths anchored under <home>/.tome/)
    pub catalog_name: String,
    pub plugin_name: String,
    pub plugin_version: String,
    pub entry_name: String,
    pub entry_path: PathBuf,
    pub entry_dir: PathBuf,
    pub plugin_root_dir: PathBuf,
    pub plugin_data_dir: PathBuf,
    pub workspace_name: String,
    pub workspace_data_dir: PathBuf,
    pub clock: time::OffsetDateTime,

    // Argument values + declarations
    pub args: Option<ArgumentValues>,
    pub declared_args: Vec<String>,

    // Lazy-init handle for data dir resolution
    pub paths: Paths,
}

/// Caller-supplied argument shape per R-10.
///
/// `Single` is the default form when the entry declares no named
/// arguments and the caller passes a raw string. `Object` is the
/// structured form: `named` carries name → value pairs;
/// `declared_order` preserves the entry's frontmatter order so
/// positional `$1` / `$2` references can be resolved deterministically.
pub enum ArgumentValues {
    Single(String),
    Object {
        named: HashMap<String, String>,
        declared_order: Vec<String>,
    },
}

impl SubstitutionContext {
    /// Begin building a [`SubstitutionContext`]. See
    /// [`SubstitutionContextBuilder`] for the field set.
    pub fn builder() -> SubstitutionContextBuilder {
        SubstitutionContextBuilder::default()
    }
}

/// Builder for [`SubstitutionContext`].
///
/// Every setter consumes and returns `self` so calls can be chained.
/// `.build()` returns `Err(SubstitutionError::InvalidArgumentFrontmatter)`
/// when a required field is missing — the error variant is reused for
/// builder-stage failures in F3; US1 may promote builder errors to a
/// dedicated variant if a real consumer surfaces a distinct need.
#[derive(Default)]
pub struct SubstitutionContextBuilder {
    catalog_name: Option<String>,
    plugin_name: Option<String>,
    plugin_version: Option<String>,
    entry_name: Option<String>,
    entry_path: Option<PathBuf>,
    entry_dir: Option<PathBuf>,
    plugin_root_dir: Option<PathBuf>,
    plugin_data_dir: Option<PathBuf>,
    workspace_name: Option<String>,
    workspace_data_dir: Option<PathBuf>,
    clock: Option<time::OffsetDateTime>,
    args: Option<ArgumentValues>,
    declared_args: Vec<String>,
    paths: Option<Paths>,
}

impl SubstitutionContextBuilder {
    pub fn catalog_name(mut self, v: impl Into<String>) -> Self {
        self.catalog_name = Some(v.into());
        self
    }

    pub fn plugin_name(mut self, v: impl Into<String>) -> Self {
        self.plugin_name = Some(v.into());
        self
    }

    pub fn plugin_version(mut self, v: impl Into<String>) -> Self {
        self.plugin_version = Some(v.into());
        self
    }

    pub fn entry_name(mut self, v: impl Into<String>) -> Self {
        self.entry_name = Some(v.into());
        self
    }

    pub fn entry_path(mut self, v: impl Into<PathBuf>) -> Self {
        self.entry_path = Some(v.into());
        self
    }

    pub fn entry_dir(mut self, v: impl Into<PathBuf>) -> Self {
        self.entry_dir = Some(v.into());
        self
    }

    pub fn plugin_root_dir(mut self, v: impl Into<PathBuf>) -> Self {
        self.plugin_root_dir = Some(v.into());
        self
    }

    pub fn plugin_data_dir(mut self, v: impl Into<PathBuf>) -> Self {
        self.plugin_data_dir = Some(v.into());
        self
    }

    pub fn workspace_name(mut self, v: impl Into<String>) -> Self {
        self.workspace_name = Some(v.into());
        self
    }

    pub fn workspace_data_dir(mut self, v: impl Into<PathBuf>) -> Self {
        self.workspace_data_dir = Some(v.into());
        self
    }

    pub fn clock(mut self, v: time::OffsetDateTime) -> Self {
        self.clock = Some(v);
        self
    }

    pub fn args(mut self, v: Option<ArgumentValues>) -> Self {
        self.args = v;
        self
    }

    pub fn declared_args(mut self, v: Vec<String>) -> Self {
        self.declared_args = v;
        self
    }

    pub fn paths(mut self, v: Paths) -> Self {
        self.paths = Some(v);
        self
    }

    /// Assemble the [`SubstitutionContext`].
    ///
    /// Returns [`SubstitutionError::InvalidArgumentFrontmatter`] with
    /// `reason = "builder missing required field: <name>"` and an empty
    /// `file` when a required setter was not called. The error variant
    /// is reused for builder-stage failures in F3; a dedicated variant
    /// can be added in US1 if a real consumer surfaces a distinct need.
    pub fn build(self) -> Result<SubstitutionContext, SubstitutionError> {
        fn missing<T>(name: &str) -> Result<T, SubstitutionError> {
            Err(SubstitutionError::InvalidArgumentFrontmatter {
                reason: format!("builder missing required field: {name}"),
                file: PathBuf::new(),
            })
        }

        Ok(SubstitutionContext {
            catalog_name: self
                .catalog_name
                .map_or_else(|| missing("catalog_name"), Ok)?,
            plugin_name: self
                .plugin_name
                .map_or_else(|| missing("plugin_name"), Ok)?,
            plugin_version: self
                .plugin_version
                .map_or_else(|| missing("plugin_version"), Ok)?,
            entry_name: self.entry_name.map_or_else(|| missing("entry_name"), Ok)?,
            entry_path: self.entry_path.map_or_else(|| missing("entry_path"), Ok)?,
            entry_dir: self.entry_dir.map_or_else(|| missing("entry_dir"), Ok)?,
            plugin_root_dir: self
                .plugin_root_dir
                .map_or_else(|| missing("plugin_root_dir"), Ok)?,
            plugin_data_dir: self
                .plugin_data_dir
                .map_or_else(|| missing("plugin_data_dir"), Ok)?,
            workspace_name: self
                .workspace_name
                .map_or_else(|| missing("workspace_name"), Ok)?,
            workspace_data_dir: self
                .workspace_data_dir
                .map_or_else(|| missing("workspace_data_dir"), Ok)?,
            clock: self.clock.map_or_else(|| missing("clock"), Ok)?,
            args: self.args,
            declared_args: self.declared_args,
            paths: self.paths.map_or_else(|| missing("paths"), Ok)?,
        })
    }
}
