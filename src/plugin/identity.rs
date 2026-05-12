//! Canonical plugin address: `<catalog>/<plugin>`.
//!
//! Lookup against the catalog registry happens at the command boundary; this
//! module only enforces shape and on-disk-safety invariants (no embedded
//! slashes, no parent traversal, no leading dot, no absolute paths).
//!
//! Spec: data-model.md §1, plugin-commands.md.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PluginId {
    pub catalog: String,
    pub plugin: String,
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.catalog, self.plugin)
    }
}

impl FromStr for PluginId {
    type Err = PluginIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (catalog, plugin) = s
            .split_once('/')
            .ok_or_else(|| PluginIdParseError::Format(s.to_owned()))?;

        validate_segment(catalog).map_err(|kind| PluginIdParseError::Catalog {
            value: catalog.to_owned(),
            kind,
        })?;
        validate_segment(plugin).map_err(|kind| PluginIdParseError::Plugin {
            value: plugin.to_owned(),
            kind,
        })?;

        Ok(Self {
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
        })
    }
}

fn validate_segment(segment: &str) -> Result<(), SegmentRejection> {
    if segment.is_empty() {
        return Err(SegmentRejection::Empty);
    }
    if segment.contains('/') {
        return Err(SegmentRejection::EmbeddedSlash);
    }
    if segment == ".." || segment == "." {
        return Err(SegmentRejection::Traversal);
    }
    if segment.starts_with('.') {
        return Err(SegmentRejection::LeadingDot);
    }
    // Reject anything that would resolve as absolute on either platform.
    if segment.starts_with('/') || segment.starts_with('\\') {
        return Err(SegmentRejection::Absolute);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PluginIdParseError {
    #[error("plugin id `{0}` must be `<catalog>/<plugin>`")]
    Format(String),

    #[error("catalog segment `{value}` is invalid: {kind}")]
    Catalog {
        value: String,
        kind: SegmentRejection,
    },

    #[error("plugin segment `{value}` is invalid: {kind}")]
    Plugin {
        value: String,
        kind: SegmentRejection,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SegmentRejection {
    #[error("must not be empty")]
    Empty,
    #[error("must not contain `/`")]
    EmbeddedSlash,
    #[error("must not be `.` or `..`")]
    Traversal,
    #[error("must not start with `.`")]
    LeadingDot,
    #[error("must not be an absolute path")]
    Absolute,
}
