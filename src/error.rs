//! The closed `TomeError` enum is the single source of truth for exit codes.
//! Adding a variant here forces edits to `tests/exit_codes.rs`, FR-022 in the
//! spec, and the PRD's exit-code table — the compiler enforces the chain.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum TomeError {
    #[error("invalid usage: {0}")]
    Usage(String),

    #[error("catalog `{0}` is not registered")]
    CatalogNotFound(String),

    #[error("catalog `{0}` is already registered")]
    CatalogAlreadyExists(String),

    #[error("manifest invalid: {0}")]
    ManifestInvalid(#[from] ManifestInvalid),

    #[error("git failed for `{catalog}`: {detail}")]
    GitFailed { catalog: String, detail: String },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("interrupted by user")]
    Interrupted,

    /// Last-resort variant for genuine programmer-facing surprises (panics
    /// caught at top level, etc.). No named failure above may collapse into
    /// this — that would defeat the closed-set guarantee.
    #[error("internal error: {0:#}")]
    Internal(anyhow::Error),
}

impl TomeError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Internal(_) => 1,
            Self::Usage(_) => 2,
            Self::CatalogNotFound(_) => 3,
            Self::CatalogAlreadyExists(_) => 4,
            Self::ManifestInvalid(_) => 5,
            Self::GitFailed { .. } => 6,
            Self::Io(_) => 7,
            Self::Interrupted => 8,
        }
    }

    /// Snake-case identifier used in `--json` error records, mapping 1:1 to
    /// the spec's FR-022 category set.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Internal(_) => "internal",
            Self::Usage(_) => "usage",
            Self::CatalogNotFound(_) => "catalog_not_found",
            Self::CatalogAlreadyExists(_) => "catalog_already_exists",
            Self::ManifestInvalid(_) => "manifest_invalid",
            Self::GitFailed { .. } => "git_failed",
            Self::Io(_) => "io",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestInvalid {
    #[error("unknown field `{key}` in {}: see {expected_schema_uri}", file.display())]
    UnknownField {
        file: PathBuf,
        key: String,
        expected_schema_uri: String,
    },

    #[error("missing required field `{key}` in {}", file.display())]
    MissingField { file: PathBuf, key: String },

    #[error("`version` in {} is not a valid semver: {got}", file.display())]
    InvalidVersion { file: PathBuf, got: String },

    #[error("`owner.email` in {} is not a valid email: {got}", file.display())]
    InvalidEmail { file: PathBuf, got: String },

    #[error("duplicate plugin name `{name}` in {}", file.display())]
    DuplicatePluginName { file: PathBuf, name: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} looks like a URL — Phase 1 supports relative paths only",
        file.display()
    )]
    SourceLooksLikeUrl { file: PathBuf, value: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} is an absolute path — must be a relative path within the catalog repo",
        file.display()
    )]
    SourceAbsolute { file: PathBuf, value: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} contains `..` — must be a normalised relative path",
        file.display()
    )]
    SourceParentTraversal { file: PathBuf, value: String },

    #[error("`plugins[].source = \"{value}\"` in {} resolves outside the catalog repo", file.display())]
    SourceEscapesRoot { file: PathBuf, value: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} does not exist or is unreachable: {cause}",
        file.display()
    )]
    SourceUnresolvable {
        file: PathBuf,
        value: String,
        cause: std::io::Error,
    },

    #[error("could not canonicalise catalog root {}: {cause}", root.display())]
    CatalogRootUnresolvable {
        root: PathBuf,
        cause: std::io::Error,
    },

    #[error("toml parse error in {}: {message}", file.display())]
    TomlParse { file: PathBuf, message: String },
}
