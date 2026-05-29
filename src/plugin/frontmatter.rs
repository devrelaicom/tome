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

use crate::plugin::identity::EntryKind;

const DESCRIPTION_FALLBACK_LIMIT_CHARS: usize = 500;

/// Argument-name pattern enforced by [`validate_argument_names`].
/// Names must match `^[a-z_][a-z0-9_]*$` per
/// `contracts/frontmatter-p5.md` § Recognised fields.
const ARGUMENT_NAME_PATTERN: &str = r"^[a-z_][a-z0-9_]*$";

/// S-M1 (US1.d reviewer pass): hard cap on the number of arguments
/// surfaced by a single entry's frontmatter. Defends against DoS at
/// plugin-enable time — a hostile catalog could otherwise ship a 1 GiB
/// YAML list of single-character argument names and force the parser
/// to allocate proportionally. Enforced in both the YAML sequence
/// visitor and the space-separated string visitor in
/// [`deserialize_arguments`]. 256 is intentionally generous: every
/// real-world prompt declares fewer than 10 named arguments; the cap
/// exists to bound pathological input, not to constrain legitimate
/// authoring.
const MAX_ARGUMENTS: usize = 256;

/// Lenient subset of the YAML header Tome consumes.
///
/// Phase 5 widens this struct with the new fields documented in
/// `contracts/frontmatter-p5.md`. The struct-level `kebab-case` rename
/// covers `disable-model-invocation`, `user-invocable`, and
/// `argument-hint`; the two snake_case fields (`when_to_use` and
/// `prompt_name`) carry explicit `#[serde(rename = "...")]` attributes
/// that override the struct-level rule. Other fields (`allowed-tools`,
/// `agent`, etc.) are tolerated silently by `serde_yaml`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Phase 5 — guidance text indexed alongside `description` to widen the
    /// embedding-text composition (see `entry-schema-p5.md`).
    #[serde(default, rename = "when_to_use")]
    pub when_to_use: Option<String>,
    /// Phase 5 — argument names. Accepts either a space-separated string
    /// (`arguments: a b c`) or a YAML list. Both forms produce a `Vec<String>`.
    #[serde(default, deserialize_with = "deserialize_arguments")]
    pub arguments: Vec<String>,
    /// Phase 5 — human-facing argument-hint text shown to invocation
    /// surfaces. Tome ingests but does not interpret.
    #[serde(default)]
    pub argument_hint: Option<String>,
    /// Phase 5 — when `true`, the entry is excluded from `search_skills`
    /// (i.e. resolved `searchable = false`).
    #[serde(default)]
    pub disable_model_invocation: Option<bool>,
    /// Phase 5 — explicit override for the `user-invocable` resolved
    /// default. Skills default to `false`, commands default to `true`.
    #[serde(default)]
    pub user_invocable: Option<bool>,
    /// Phase 5 — explicit prompt-name override surfaced through MCP
    /// `prompts/list`. Tome ingests but does not interpret in US1.a.
    #[serde(default, rename = "prompt_name")]
    pub prompt_name: Option<String>,
}

/// Custom deserialiser for the `arguments` field per
/// `contracts/frontmatter-p5.md` § "arguments accepts both string and list
/// forms".
///
/// Accepted shapes:
/// * Absent → `Vec::new()` (handled by `#[serde(default)]`).
/// * Empty string or string of only whitespace → `Vec::new()`.
/// * Space-separated string `"a b c"` → `vec!["a", "b", "c"]`.
/// * YAML list `[a, b, c]` → `vec!["a", "b", "c"]`.
/// * Any other shape (integer, mapping, list-of-non-strings) → deser error;
///   the caller maps this to `TomeError::InvalidArgumentFrontmatter`
///   (exit 29).
///
/// S-M1 (US1.d reviewer pass): the parser enforces a 256-entry hard cap
/// on the resulting `Vec<String>` to defend against DoS at plugin-enable
/// time. See [`MAX_ARGUMENTS`].
fn deserialize_arguments<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, Visitor};
    use std::fmt;

    struct ArgsVisitor;

    impl<'de> Visitor<'de> for ArgsVisitor {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a space-separated string or a list of strings")
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
            let mut out: Vec<String> = Vec::new();
            for tok in v.split_whitespace() {
                if out.len() >= MAX_ARGUMENTS {
                    return Err(E::custom(format!(
                        "arguments list exceeds {MAX_ARGUMENTS} entries"
                    )));
                }
                out.push(tok.to_owned());
            }
            Ok(out)
        }

        fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
            self.visit_str(&v)
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
        where
            S: serde::de::SeqAccess<'de>,
        {
            let mut out: Vec<String> = match seq.size_hint() {
                Some(n) => Vec::with_capacity(n.min(MAX_ARGUMENTS)),
                None => Vec::new(),
            };
            while let Some(elem) = seq.next_element::<String>()? {
                if out.len() >= MAX_ARGUMENTS {
                    return Err(<S::Error as Error>::custom(format!(
                        "arguments list exceeds {MAX_ARGUMENTS} entries"
                    )));
                }
                out.push(elem);
            }
            Ok(out)
        }

        fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_any(self)
        }
    }

    deserializer.deserialize_any(ArgsVisitor)
}

impl SkillFrontmatter {
    /// Resolved `searchable` value per `contracts/frontmatter-p5.md`
    /// § Resolved defaults. Equivalent to
    /// `!disable_model_invocation.unwrap_or(false)` — i.e. searchable by
    /// default unless the author explicitly opts out.
    pub fn resolved_searchable(&self) -> bool {
        !self.disable_model_invocation.unwrap_or(false)
    }

    /// Resolved `user_invocable` value per `contracts/frontmatter-p5.md`
    /// § Resolved defaults. Defaults depend on `kind`: skills default to
    /// `false`, commands default to `true`; an explicit frontmatter value
    /// overrides those two. Agents are special-cased (entry-schema-p6.md):
    /// they are NEVER user-invocable — there is no frontmatter flag to
    /// flip an agent into a prompt, so we ignore any author override and
    /// return `false` before consulting `self.user_invocable`.
    pub fn resolved_user_invocable(&self, kind: EntryKind) -> bool {
        match kind {
            EntryKind::Agent => false,
            EntryKind::Skill => self.user_invocable.unwrap_or(false),
            EntryKind::Command => self.user_invocable.unwrap_or(true),
        }
    }
}

/// Validate every argument name against `^[a-z_][a-z0-9_]*$`. Returns the
/// first illegal name on failure. The caller is responsible for converting
/// this into a `TomeError::InvalidArgumentFrontmatter { file, reason }`
/// (exit 29).
pub fn validate_argument_names(names: &[String]) -> Result<(), String> {
    // Hand-rolled validation to avoid promoting `regex` to a direct dep
    // before Phase 5 / F2 (which is the dedicated promotion slice).
    for name in names {
        if !is_valid_argument_name(name) {
            return Err(format!(
                "argument name `{name}` must match {ARGUMENT_NAME_PATTERN}"
            ));
        }
    }
    Ok(())
}

fn is_valid_argument_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
    let contents = crate::util::bounded_read_to_string(path, crate::util::PLUGIN_MANIFEST_MAX)
        .map_err(|err| match err {
            crate::error::TomeError::Io(source) => FrontmatterError::Io {
                file: path.to_path_buf(),
                source,
            },
            other => FrontmatterError::Io {
                file: path.to_path_buf(),
                source: std::io::Error::other(other.to_string()),
            },
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
