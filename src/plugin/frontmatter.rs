//! Lenient parser for the YAML metadata header at the top of a `SKILL.md`.
//!
//! Two distinct failure modes per spec:
//!
//! * **Delimiter failure** ("header could not be located") — caller maps to
//!   [`TomeError::SkillFrontmatterParseError`] (exit 23) and aborts the enable.
//!   See FR-013b's analogue for plugin manifests and the plugin-commands.md
//!   step 7 carve-out.
//! * **YAML body failure** ("header located but invalid YAML inside") — caller
//!   logs a warning naming the file and skips that single skill per FR-013c;
//!   the rest of the plugin still enables.
//!
//! Fallbacks per FR-011 / FR-012:
//!
//! * Missing or empty `name` → skill directory name.
//! * Missing or empty `description` → first 500 chars of the body text.
//!
//! The caller is expected to log a warning whenever a fallback is applied —
//! [`ParsedSkill::resolved_name`] / [`resolved_description`] return a
//! `fallback_applied` flag for that purpose.
//!
//! Spec: data-model.md §4, plugin-commands.md, FR-011, FR-012, FR-013c.

use std::path::{Path, PathBuf};

const DESCRIPTION_FALLBACK_LIMIT_CHARS: usize = 500;

/// Subset of the YAML header Tome consumes. Other fields (`when_to_use`,
/// `allowed-tools`, etc.) are deliberately omitted — serde_yaml will skip
/// them under the lenient parse policy.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Parsed `SKILL.md`: the YAML header plus the body text that follows it.
/// The body is preserved verbatim so the description-fallback can quote it
/// faithfully (FR-012).
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
}

impl ParsedSkill {
    /// Resolve `name` per FR-011: trimmed frontmatter value if non-empty,
    /// otherwise the supplied directory name. The returned `bool` is `true`
    /// when the directory-name fallback was applied (so the caller can warn).
    pub fn resolved_name(&self, dir_name: &str) -> (String, bool) {
        match self.frontmatter.name.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => (s.to_owned(), false),
            _ => (dir_name.to_owned(), true),
        }
    }

    /// Resolve `description` per FR-012: trimmed frontmatter value if
    /// non-empty, otherwise the first 500 characters of the body. The bool
    /// is `true` when the body-prefix fallback was applied.
    pub fn resolved_description(&self) -> (String, bool) {
        match self.frontmatter.description.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => (s.to_owned(), false),
            _ => {
                let prefix: String = self
                    .body
                    .chars()
                    .take(DESCRIPTION_FALLBACK_LIMIT_CHARS)
                    .collect();
                (prefix, true)
            }
        }
    }
}

/// Errors returned by [`parse_skill_frontmatter`]. The caller decides which
/// failure mode is fatal — see the module docs.
#[derive(Debug, thiserror::Error)]
pub enum FrontmatterError {
    /// The opening `---` delimiter is missing, or the closing one is not
    /// found before EOF. Per the contract this is an exit-23 condition.
    #[error("frontmatter delimiters not located in {}: {message}", file.display())]
    MissingDelimiters { file: PathBuf, message: String },

    /// The delimiters were located but the YAML between them does not parse.
    /// Per FR-013c the caller logs and skips this single skill.
    #[error("frontmatter YAML body invalid in {}: {message}", file.display())]
    InvalidYaml { file: PathBuf, message: String },

    /// Bubbled-up I/O failure. Maps to [`TomeError::Io`] at the boundary.
    #[error("could not read {}: {source}", file.display())]
    Io {
        file: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Read and parse a `SKILL.md` from disk.
pub fn parse_skill_frontmatter(path: &Path) -> Result<ParsedSkill, FrontmatterError> {
    let contents = std::fs::read_to_string(path).map_err(|source| FrontmatterError::Io {
        file: path.to_path_buf(),
        source,
    })?;
    parse_skill_frontmatter_str(path, &contents)
}

/// Parse a `SKILL.md` from an in-memory string. The `file` path is recorded
/// in any error for diagnostics.
pub fn parse_skill_frontmatter_str(
    file: &Path,
    contents: &str,
) -> Result<ParsedSkill, FrontmatterError> {
    // Strip an optional UTF-8 BOM. The delimiters are line-anchored, so we
    // work over the post-BOM tail directly.
    let stripped = contents.strip_prefix('\u{FEFF}').unwrap_or(contents);

    let (yaml_block, body) =
        split_frontmatter(stripped).ok_or_else(|| FrontmatterError::MissingDelimiters {
            file: file.to_path_buf(),
            message: "expected `---` header at file start with a matching closing `---`".to_owned(),
        })?;

    // An entirely empty header block is treated as "all fields missing" rather
    // than a parse failure. The serde defaults will leave both fields as None.
    let frontmatter: SkillFrontmatter = if yaml_block.trim().is_empty() {
        SkillFrontmatter::default()
    } else {
        serde_yaml::from_str(yaml_block).map_err(|cause| FrontmatterError::InvalidYaml {
            file: file.to_path_buf(),
            message: cause.to_string(),
        })?
    };

    Ok(ParsedSkill {
        frontmatter,
        body: body.to_owned(),
    })
}

/// Split `contents` into `(yaml_block, body)`. Returns `None` if either the
/// opening or the closing `---` delimiter is missing.
///
/// Delimiter lines accept both `\n` and `\r\n` terminators and may have
/// trailing whitespace, but must consist solely of three dashes.
fn split_frontmatter(contents: &str) -> Option<(&str, &str)> {
    let after_open = strip_delimiter_line(contents)?;
    let close_at = find_closing_delimiter(after_open)?;
    let yaml = &after_open[..close_at.start];
    let body = &after_open[close_at.end..];
    Some((yaml, body))
}

/// If `s` begins with a `---` delimiter line, return the remainder after that
/// line (its newline included). Otherwise return `None`.
fn strip_delimiter_line(s: &str) -> Option<&str> {
    let (first_line, rest) = match s.find('\n') {
        Some(idx) => (&s[..idx], &s[idx + 1..]),
        None => (s, ""),
    };
    if is_delimiter(first_line) {
        Some(rest)
    } else {
        None
    }
}

struct DelimiterSpan {
    /// Byte offset of the first character of the delimiter line.
    start: usize,
    /// Byte offset just past the trailing newline (or EOF).
    end: usize,
}

fn find_closing_delimiter(s: &str) -> Option<DelimiterSpan> {
    let bytes = s.as_bytes();
    let mut line_start = 0;
    while line_start <= bytes.len() {
        let next_newline = bytes[line_start..].iter().position(|b| *b == b'\n');
        let line_end = match next_newline {
            Some(off) => line_start + off,
            None => bytes.len(),
        };
        // SAFETY: line_start..line_end are byte offsets inside a &str at
        // ASCII-only boundaries (we only advance past `\n`).
        let line = &s[line_start..line_end];
        if is_delimiter(line) {
            let end = if next_newline.is_some() {
                line_end + 1
            } else {
                line_end
            };
            return Some(DelimiterSpan {
                start: line_start,
                end,
            });
        }
        match next_newline {
            Some(_) => line_start = line_end + 1,
            None => break,
        }
    }
    None
}

/// Returns true iff `line` (with its trailing CR/whitespace stripped) is
/// exactly `---`. Anything else — including `----`, `--- foo`, or `... `
/// (the YAML end-of-document marker) — is not a delimiter.
fn is_delimiter(line: &str) -> bool {
    let trimmed = line.trim_end_matches(['\r', ' ', '\t']);
    trimmed == "---"
}
