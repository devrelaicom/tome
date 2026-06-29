//! Canonical + translated agent types and the harness-agnostic translation
//! machinery (data-model §4, `contracts/agent-translation.md`).
//!
//! This module owns the SHARED building blocks every per-harness
//! `translate_agent` impl reuses:
//!
//! * [`CanonicalAgent::parse`] — parse a plugin's `agents/<name>.md` (YAML
//!   frontmatter + Markdown body) into a [`CanonicalAgent`].
//! * [`agent_filename`] — the sole provenance mechanism, `<plugin>__<name>.<ext>`
//!   (R-19 single source of truth; no provenance frontmatter key).
//! * [`render_markdown_yaml`] / [`render_codex_toml`] — the two render
//!   primitives (Markdown+YAML body, and a triple-quoted
//!   `developer_instructions` Codex-TOML string built via `toml_edit`).
//! * [`map_model`] — the same-vendor-only model alias table (FR-034/037).
//! * [`infer_read_only`] — read-only intent inference (FR-036).
//! * [`displayed_name`] — clean vs clash-prefixed displayed name (FR-041).
//! * [`is_safe_agent_name`] — the S-1 single-safe-path-segment gate applied
//!   at index time before a `name` is stored.
//! * [`plugin_of_owned_file`] — the inverse of [`agent_filename`]; the SSOT
//!   `<plugin>__*.<ext>` ownership split consumed by the sync reconciliation
//!   for both the per-plugin removal and orphan-cleanup passes (FR-043).
//!
//! The per-harness `translate_agent` impls (which directory, which format,
//! which fields survive the field map) and the sync reconciliation consume
//! these helpers; the public surface here is the harness-agnostic core.

use std::path::{Path, PathBuf};

use super::AgentFormat;
use crate::error::TomeError;

/// A plugin's source agent, parsed from `<plugin>/agents/<name>.md`
/// (data-model §4). The privileged fields (`hooks`, `mcp_servers`,
/// `permission_mode`) are passed through to Claude Code by default and
/// stripped under the `strip_plugin_agent_privileges` setting (FR-050 /
/// FR-052). `serde_json::Value` keeps the privileged blobs opaque — Tome
/// neither interprets nor validates their internal shape, it only forwards
/// or drops them wholesale.
///
/// `plugin` is carried on the canonical so the per-harness
/// [`super::HarnessModule::translate_agent`] impls can build the
/// `<plugin>__<name>` filename and the clash-prefixed displayed name
/// without threading plugin context through a separate parameter. `catalog`
/// is retained because the clash-set SSOT keys identity on
/// `(catalog, plugin)`; chunk C needs it when it computes per-agent display
/// names from the workspace clash set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalAgent {
    /// Owning catalog (clash-set identity is keyed on `(catalog, plugin)`).
    pub catalog: String,
    /// Owning plugin — the `<plugin>` half of `<plugin>__<name>`.
    pub plugin: String,
    /// Frontmatter `name`, else the filename stem.
    pub name: String,
    /// Frontmatter `description`, if present.
    pub description: Option<String>,
    /// System-prompt Markdown (the body below the frontmatter).
    pub body: String,
    /// Canonical model value (`opus`, `inherit`, …), if declared. Mapped
    /// per-harness via the same-vendor-only model alias table (FR-037).
    pub model: Option<String>,
    /// Allowed tools posture (drives read-only inference, FR-036).
    pub tools: Option<Vec<String>>,
    /// Disallowed tools posture.
    pub disallowed_tools: Option<Vec<String>>,
    /// Privileged: hook spec passed through to Claude Code (FR-050).
    pub hooks: Option<serde_json::Value>,
    /// Privileged: MCP server spec passed through to Claude Code (FR-050).
    pub mcp_servers: Option<serde_json::Value>,
    /// Privileged: permission mode passed through to Claude Code (FR-050).
    pub permission_mode: Option<String>,
}

/// The per-harness emission result for one agent (data-model §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslatedAgent {
    /// Target directory — the harness's `agent_dir(project)`.
    pub dir: PathBuf,
    /// Always `<plugin>__<name>.<ext>` (FR-040).
    pub filename: String,
    /// Clean `<name>`, or a clash-prefixed `<plugin>-<name>` (FR-041);
    /// OpenCode always uses `<plugin>__<name>` (FR-042).
    pub displayed_name: String,
    /// MarkdownYaml or Toml, per the harness's `agent_format()`.
    pub format: AgentFormat,
    /// The rendered file content (body in the file body, or in a
    /// triple-quoted `developer_instructions` TOML string — FR-033).
    pub rendered: String,
    /// Frontmatter fields dropped during translation, recorded for
    /// diagnostics (FR-032 / FR-034 / FR-036).
    pub dropped_fields: Vec<String>,
}

/// The frontmatter subset Tome reads off a source agent `.md`.
///
/// Parses leniently — third-party plugin input, so unknown keys are
/// tolerated (the strictness boundary, FR-013a, applies only to Tome-owned
/// inputs). Recognised keys cover Claude Code's canonical agent frontmatter
/// vocabulary; everything else is silently dropped by `serde_yaml` and (per
/// FR-032) never forwarded on the assumption a harness tolerates it.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct AgentFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// Claude Code spells this `disallowedTools` (camelCase); accept that and
    /// the snake_case alias for robustness.
    #[serde(default, rename = "disallowedTools", alias = "disallowed_tools")]
    disallowed_tools: Option<Vec<String>>,
    #[serde(default)]
    hooks: Option<serde_json::Value>,
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    mcp_servers: Option<serde_json::Value>,
    #[serde(default, rename = "permissionMode", alias = "permission_mode")]
    permission_mode: Option<String>,
}

impl CanonicalAgent {
    /// Parse a source agent `.md` into a [`CanonicalAgent`].
    ///
    /// `contents` is the raw file text; `catalog`, `plugin`, and
    /// `filename_stem` supply the provenance context the body cannot. `name`
    /// resolves to the frontmatter `name` when present and non-empty, else
    /// the filename stem (FR-040 / data-model §4).
    ///
    /// Reuses the same frontmatter/body split as `SKILL.md`
    /// ([`crate::plugin::frontmatter`]) — agents differ only in *which*
    /// frontmatter fields they carry, not in the delimiter grammar.
    ///
    /// Malformed frontmatter (missing delimiters, or invalid YAML between
    /// them) maps to [`TomeError::AgentTranslationFailed`] (exit 45): unlike
    /// `SKILL.md`'s two-mode handling, a malformed agent is always a hard
    /// failure for that agent — there is no partial-skip fallback because
    /// the translated file would be meaningless without its frontmatter.
    pub fn parse(
        catalog: &str,
        plugin: &str,
        filename_stem: &str,
        contents: &str,
    ) -> Result<Self, TomeError> {
        // `agent_label` is the diagnostic identity carried on the error so
        // the doctor / sync surfaces can name the offending agent.
        let agent_label = format!("{catalog}/{plugin}/{filename_stem}");

        // Reuse the SKILL.md delimiter/body split. We parse the YAML
        // ourselves into `AgentFrontmatter` (the agent vocabulary), so we
        // only borrow the splitter, not the skill struct.
        let path = Path::new(filename_stem);
        let parsed = crate::plugin::frontmatter::parse_skill_frontmatter_str(path, contents)
            .map_err(|_| TomeError::AgentTranslationFailed {
                agent: agent_label.clone(),
            })?;

        // Re-extract the raw YAML block to deserialize the agent vocabulary.
        // `parse_skill_frontmatter_str` already validated the delimiters and
        // that the block is valid YAML for the skill struct; re-parsing into
        // the agent struct can still fail if a recognised agent key carries
        // the wrong YAML type (e.g. `tools: 7`).
        let stripped = contents.strip_prefix('\u{FEFF}').unwrap_or(contents);
        let yaml_block =
            split_frontmatter_block(stripped).ok_or_else(|| TomeError::AgentTranslationFailed {
                agent: agent_label.clone(),
            })?;

        let fm: AgentFrontmatter = if yaml_block.trim().is_empty() {
            AgentFrontmatter::default()
        } else {
            serde_yaml::from_str(yaml_block).map_err(|_| TomeError::AgentTranslationFailed {
                agent: agent_label.clone(),
            })?
        };

        let name = match fm.name.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => s.to_owned(),
            _ => filename_stem.to_owned(),
        };

        Ok(Self {
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
            name,
            description: fm
                .description
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty()),
            body: parsed.body,
            model: fm
                .model
                .map(|m| m.trim().to_owned())
                .filter(|m| !m.is_empty()),
            tools: fm.tools,
            disallowed_tools: fm.disallowed_tools,
            hooks: fm.hooks,
            mcp_servers: fm.mcp_servers,
            permission_mode: fm
                .permission_mode
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty()),
        })
    }
}

/// Re-extract just the YAML block from a frontmatter document, reusing the
/// SKILL.md splitter's grammar. Returns `None` when delimiters are absent.
///
/// `parse_skill_frontmatter_str` owns the splitter but only exposes the
/// parsed skill struct + body, not the raw YAML; this private helper mirrors
/// its split so the agent parser can deserialize the agent vocabulary off
/// the same byte range. The duplication is minimal and avoids widening the
/// frontmatter module's public surface for one extra consumer.
fn split_frontmatter_block(contents: &str) -> Option<&str> {
    let after_open = {
        let (first_line, rest) = match contents.find('\n') {
            Some(idx) => (&contents[..idx], &contents[idx + 1..]),
            None => (contents, ""),
        };
        let trimmed = first_line.trim_end_matches(['\r', ' ', '\t']);
        if trimmed == "---" {
            rest
        } else {
            return None;
        }
    };
    // Find the closing `---` line.
    let bytes = after_open.as_bytes();
    let mut line_start = 0usize;
    while line_start <= bytes.len() {
        let nl = bytes[line_start..].iter().position(|b| *b == b'\n');
        let line_end = match nl {
            Some(off) => line_start + off,
            None => bytes.len(),
        };
        let line = &after_open[line_start..line_end];
        if line.trim_end_matches(['\r', ' ', '\t']) == "---" {
            return Some(&after_open[..line_start]);
        }
        match nl {
            Some(_) => line_start = line_end + 1,
            None => break,
        }
    }
    None
}

/// Validate that an agent `name` is a single safe path segment (S-1).
///
/// The emitted filename is `<plugin>__<name>.<ext>` and sync joins it onto
/// the harness agent dir, so an attacker-controlled `name` such as
/// `../../../../tmp/evil` would escape the directory. This is the index-time
/// gate: a `name` must resolve to exactly one `Component::Normal` and carry
/// no `/`, `\`, NUL, leading `.`, or `.`/`..` traversal token. On rejection
/// the caller maps to [`TomeError::AgentTranslationFailed`] (exit 45).
///
/// Mirrors the `identity::validate_segment` discipline but is stricter on
/// two fronts the plugin-id path does not face: an embedded NUL (invalid in
/// any POSIX/Windows path component) and an embedded backslash (a Windows
/// separator that `identity::validate_segment` only rejects as a *leading*
/// char). The single-`Component::Normal` check is the robust backstop —
/// anything that decomposes into more than one component, or into a
/// `ParentDir`/`CurDir`/`RootDir`/`Prefix`, is rejected regardless of how it
/// is spelled on the host platform.
pub(crate) fn is_safe_agent_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // NUL can never appear in a valid path component and would truncate the
    // path at the syscall boundary on Unix.
    if name.contains('\u{0}') {
        return false;
    }
    // Reject separators on either platform up front. `Path::components`
    // already splits on `/` (and on `\` on Windows), but checking here keeps
    // the rejection platform-independent so a `\`-bearing name is refused on
    // Unix too.
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    // Explicit traversal / dotfile rejection (matches identity::validate_segment).
    if name == "." || name == ".." || name.starts_with('.') {
        return false;
    }
    // The robust backstop: the name must decompose into exactly one
    // `Component::Normal` equal to itself.
    let mut comps = Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(seg)), None) => seg == std::ffi::OsStr::new(name),
        _ => false,
    }
}

/// File extension for a harness [`AgentFormat`].
pub(crate) fn agent_extension(format: AgentFormat) -> &'static str {
    match format {
        AgentFormat::MarkdownYaml => "md",
        AgentFormat::Toml => "toml",
    }
}

/// Build the agent filename — `<plugin>__<name>.<ext>` (FR-040, R-19).
///
/// This is the SOLE provenance mechanism: Tome adds no provenance
/// frontmatter key (an unknown key risks breaking a harness parser). The
/// double underscore separator distinguishes the Tome-owned prefix from a
/// single-underscore name. Every harness's removal glob and emission path
/// route through this one builder.
pub(crate) fn agent_filename(plugin: &str, name: &str, ext: &str) -> String {
    format!("{plugin}__{name}.{ext}")
}

/// Render a Markdown-with-YAML-frontmatter agent file.
///
/// `frontmatter` is an ordered slice of `(key, value)` pairs — Tome does not
/// take a direct dependency on an insertion-ordered map type, so the
/// per-harness caller expresses key order positionally and this writer
/// preserves it verbatim. An empty slice renders an empty `---\n---\n`
/// header followed by the body — callers that want no frontmatter at all
/// should special-case that upstream.
///
/// The body is appended verbatim after the closing delimiter. A single
/// newline separates the header from the body; the body's own leading
/// whitespace is preserved.
///
/// Trust assumption (companion to the `render_codex_toml` escaping note):
/// the body is copied verbatim and is NOT a frontmatter-field injection
/// vector. (a) The destination Markdown-YAML harnesses parse only the
/// LEADING `---…---` block as frontmatter; a `---` later in the body is a
/// thematic break, not a re-opened frontmatter block. (b) The frontmatter
/// values Tome emits are YAML-escaped by `serde_yaml`, so a hostile value
/// can't break out of the leading block either. A body line such as `---`
/// followed by `tools: [Bash]` therefore lands as body prose, never as
/// parsed frontmatter — verified by
/// `body_with_frontmatter_delimiter_does_not_inject_fields`.
pub(crate) fn render_markdown_yaml(
    frontmatter: &[(String, serde_yaml::Value)],
    body: &str,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    if !frontmatter.is_empty() {
        // Build a YAML mapping preserving the slice order, then serialise.
        // `serde_yaml::Mapping` keeps key order as inserted.
        let mut map = serde_yaml::Mapping::new();
        for (k, v) in frontmatter {
            map.insert(serde_yaml::Value::String(k.clone()), v.clone());
        }
        // `serde_yaml::to_string` of a mapping never fails for owned Values.
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(map)).unwrap_or_default();
        out.push_str(&yaml);
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// Render a Codex agent TOML document with the body in a triple-quoted
/// `developer_instructions` string (FR-033, R-14).
///
/// Built via `toml_edit` (the existing dep) so quoting and escaping are the
/// library's, never hand-rolled. `toml_edit`'s default string
/// representation promotes any value containing a newline to a multiline
/// basic string (`"""…"""`) — see `toml_write::TomlStringBuilder::as_default`
/// — which is exactly the triple-quoted form the contract mandates. Agent
/// bodies are multi-line Markdown, so the promotion is reliable; a body that
/// happens to be a single line would render as a regular basic string, which
/// is still valid TOML for the same value.
///
/// `scalars` carries the additional top-level keys (e.g. `name`,
/// `description`, `model`) as an ordered `(key, value)` slice; they are
/// written before `developer_instructions` so the prose block lands last.
pub(crate) fn render_codex_toml(scalars: &[(String, String)], body: &str) -> String {
    use toml_edit::{DocumentMut, value};

    let mut doc = DocumentMut::new();
    for (k, v) in scalars {
        doc[k.as_str()] = value(v.as_str());
    }
    doc["developer_instructions"] = value(body);
    doc.to_string()
}

/// Per-harness model alias table — SAME-VENDOR ONLY (FR-034/037, R-8).
///
/// `map_model(registry, harness, source)` returns the harness-native identifier
/// for a canonical model value, or `None` to DROP the field (harness default
/// inherited). Cross-vendor mapping is FORBIDDEN: `opus → codex` is `None`,
/// never an OpenAI id. `inherit` drops everywhere. Any source value with no
/// same-vendor target for the harness drops.
///
/// The `registry` is used by OpenCode, Goose, and Pi to resolve tier aliases
/// (`opus` / `sonnet` / `haiku`) to the newest non-preview same-vendor id at
/// call time (registry-driven, not a static string). For Cursor, any pinned
/// model maps to `inherit` (Cursor's proprietary model ids are not in
/// models.dev; the `inherit` value preserves the intent of the original pin).
///
/// This is the named artefact SC-002 verifies against; the table is pinned
/// in `contracts/agent-translation.md`.
///
/// Ecosystem caveat: the exact harness-native identifiers are confirmed
/// against current harness docs at implementation time; the *policy*
/// (same-vendor-only, drop-on-no-target) is fixed.
pub(crate) fn map_model(
    registry: &crate::model_registry::ModelRegistry,
    harness: &str,
    source: &str,
) -> Option<String> {
    // `inherit` is always dropped — there is no native "inherit the caller's
    // model" value that ports across harnesses.
    if source == "inherit" {
        return None;
    }
    match harness {
        // Claude Code is the canonical vendor: aliases ARE its native ids.
        "claude-code" => match source {
            "opus" | "sonnet" | "haiku" => Some(source.to_owned()),
            other => Some(other.to_owned()),
        },
        // Codex is OpenAI-vendored: no Anthropic alias maps. DROP.
        "codex" => None,
        // Cursor's model ids are proprietary and not in models.dev; a pinned
        // model becomes `inherit` (Cursor accepts it), preserving intent.
        "cursor" => Some("inherit".to_owned()),
        // OpenCode needs a concrete `<vendor>/<id>` resolved from the registry.
        "opencode" => match source {
            "opus" | "sonnet" | "haiku" => registry
                .resolve_tier("anthropic", source)
                .map(|id| format!("anthropic/{id}")),
            // An already-namespaced concrete id passes through; a bare one
            // can't be safely namespaced → drop.
            other if other.contains('/') => Some(other.to_owned()),
            _ => None,
        },
        // Gemini (Google vendor): no Anthropic id maps; `inherit` is the
        // literal default Gemini accepts, so a pinned model becomes `inherit`.
        "gemini" => Some("inherit".to_owned()),
        // Kiro: ids are dotted (`claude-sonnet-4.5`) so the registry's
        // hyphenated ids are rejected, and the field is ignored in programmatic
        // subagent dispatch (issue #6637) — DROP, Kiro uses the current chat LLM.
        "kiro" => None,
        // Goose + Pi pass a bare provider model id verbatim to the session's
        // provider: a tier alias resolves to the registry's bare anthropic id;
        // a concrete id passes through.
        "goose" | "pi" => match source {
            "opus" | "sonnet" | "haiku" => registry.resolve_tier("anthropic", source),
            other => Some(other.to_owned()),
        },
        // Devin uses its own short aliases (`opus`/`sonnet`); `haiku` has no
        // Devin alias and concrete/registry ids are unconfirmed → DROP those.
        "devin" => match source {
            "opus" | "sonnet" => Some(source.to_owned()),
            _ => None,
        },
        // Unknown harness: drop conservatively.
        _ => None,
    }
}

/// Tools classified as write / edit / execute for read-only inference
/// (FR-036). Matched case-insensitively against the agent's tool posture.
///
/// The classification covers Claude Code's built-in mutating tools plus the
/// shell/execution surface. A tool not in this set is treated as read-only
/// (e.g. `Read`, `Grep`, `Glob`, `WebFetch`, `WebSearch`). The set is
/// intentionally conservative: anything that writes a file, edits a file, or
/// runs arbitrary commands counts as a write/edit/execute-class tool.
const WRITE_EDIT_EXECUTE_TOOLS: &[&str] = &[
    "write",     // create/overwrite a file
    "edit",      // surgical file edit
    "multiedit", // batched file edits
    "notebookedit",
    "bash",    // arbitrary shell execution
    "execute", // generic execution alias
    "run",     // generic run alias
];

/// Returns true when `tool` is a write/edit/execute-class tool (FR-036).
fn is_write_edit_execute(tool: &str) -> bool {
    let lower = tool.trim().to_ascii_lowercase();
    WRITE_EDIT_EXECUTE_TOOLS.contains(&lower.as_str())
}

/// Infer read-only intent from an agent's tool posture (FR-036).
///
/// **Rule**: an agent is read-only when its effective tool set contains no
/// write/edit/execute-class tool — i.e. the allowlist (if present) excludes
/// every such tool, OR the disallowed list denies all of them.
///
/// Return semantics:
/// * `Some(true)`  — provably read-only.
/// * `Some(false)` — provably NOT read-only (a write/edit/execute tool is
///   present in the allowlist).
/// * `None`        — indeterminate (no allowlist and the disallowed list
///   does not deny the full write/edit/execute set, or both are absent).
///   The caller DROPS the field and inherits the harness default.
///
/// The allowlist is authoritative when present: a `tools` allowlist fully
/// describes the agent's posture, so we decide purely from it. With no
/// allowlist, we can only conclude read-only when the disallowed list denies
/// *every* write/edit/execute tool we classify; a partial deny is an
/// indeterminate (mixed) posture → `None`.
pub(crate) fn infer_read_only(
    tools: Option<&[String]>,
    disallowed: Option<&[String]>,
) -> Option<bool> {
    if let Some(allow) = tools {
        // Allowlist present: read-only iff it grants no write/edit/execute
        // tool.
        let grants_mutating = allow.iter().any(|t| is_write_edit_execute(t));
        return Some(!grants_mutating);
    }

    // No allowlist. We can only conclude read-only if the disallowed list
    // denies the entire write/edit/execute class; otherwise the posture is
    // indeterminate (the agent may use a mutating tool we did not see
    // denied).
    if let Some(deny) = disallowed {
        let denied: std::collections::HashSet<String> =
            deny.iter().map(|t| t.trim().to_ascii_lowercase()).collect();
        let denies_all = WRITE_EDIT_EXECUTE_TOOLS.iter().all(|t| denied.contains(*t));
        if denies_all {
            return Some(true);
        }
    }
    None
}

/// Resolve a harness-required `description` (FR-035): the canonical
/// `description` if present; else the first non-empty trimmed body line; else
/// a documented placeholder. The single source every required-`description`
/// harness (OpenCode, Gemini, Copilot, Pi) routes through.
pub(crate) fn synthesize_description(canonical: &CanonicalAgent) -> String {
    if let Some(desc) = &canonical.description {
        return desc.clone();
    }
    if let Some(line) = canonical
        .body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
    {
        return line.to_owned();
    }
    format!("Agent {} (no description provided).", canonical.name)
}

/// Normalise a displayed agent name to a harness's slug rules. `allow_underscore`
/// keeps `_` (Gemini: `^[a-z0-9-_]+$`); when false, `_` becomes `-` (Kiro:
/// lowercase + hyphens only). Lowercases; maps any other char to `-`; collapses
/// runs of `-`; trims leading/trailing `-`; empties fall back to `"agent"`.
// Consumed by the Gemini and Kiro harness modules (later tasks). The allow
// silences the dead-code lint until those callers land.
#[allow(dead_code)]
pub(crate) fn slugify_agent_name(name: &str, allow_underscore: bool) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for ch in name.chars().flat_map(|c| c.to_lowercase()) {
        let keep = ch.is_ascii_alphanumeric() || ch == '-' || (allow_underscore && ch == '_');
        if keep {
            // Track whether the emitted char was a dash so runs of LITERAL
            // input hyphens collapse too (not only replacement dashes).
            if ch == '-' {
                if !last_dash {
                    out.push(ch);
                    last_dash = true;
                }
            } else {
                out.push(ch);
                last_dash = false;
            }
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "agent".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Resolve the displayed / registered agent name (FR-041).
///
/// Uses the clean `<name>` normally, and the plugin-prefixed
/// `<plugin>-<name>` form ONLY when `clashes` is true (two or more enabled
/// plugins in the workspace hold the same `<name>`). The on-disk filename
/// stays `<plugin>__<name>` regardless of clash; this governs only the
/// human-facing / harness-registered display name.
///
/// OpenCode derives its agent name from the filename and so always shows
/// `<plugin>__<name>` — that override is a chunk-C concern; this helper just
/// exposes the clean-vs-clash distinction every other harness uses.
pub(crate) fn displayed_name(plugin: &str, name: &str, clashes: bool) -> String {
    if clashes {
        format!("{plugin}-{name}")
    } else {
        name.to_owned()
    }
}

/// Recover the `<plugin>` prefix from a Tome-owned agent filename
/// `<plugin>__<name>.<ext>`, or `None` when the filename is not Tome-owned
/// (no `__` separator, an empty plugin prefix, or an empty `<name>` stem).
///
/// This is the inverse of [`agent_filename`] and the single source of truth
/// for the `<plugin>__` ownership split — the sync reconciliation consumes
/// it for both the per-plugin removal pass and the orphan-cleanup pass so
/// the split rule is never re-rolled (FR-043).
pub(crate) fn plugin_of_owned_file(filename: &str) -> Option<&str> {
    let (plugin, rest) = filename.split_once("__")?;
    if plugin.is_empty() {
        return None;
    }
    // Require a non-empty `<name>` before the extension dot.
    let stem = rest.rsplit_once('.').map(|(s, _)| s).unwrap_or(rest);
    if stem.is_empty() {
        return None;
    }
    Some(plugin)
}

/// Public re-export of [`plugin_of_owned_file`] for the US5 doctor
/// read-only surface (`crate::doctor::checks`), which lives outside the
/// `harness` module and so cannot reach the `pub(crate)` original. The
/// ownership split rule stays single-sourced — this is a thin forwarder.
pub fn plugin_of_owned_file_pub(filename: &str) -> Option<&str> {
    plugin_of_owned_file(filename)
}

/// Canonical Claude-Code tool kinds Tome can translate. Unknown CC tools
/// classify to `None` and are dropped by every renderer.
// Consumed by the per-harness translate_agent impls (Tasks 5–12). The allow
// silences the dead-code lint until those callers land.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CcToolKind {
    Read,
    Grep,
    Glob,
    Ls,
    Edit,
    Write,
    MultiEdit,
    NotebookEdit,
    Bash,
    WebFetch,
    WebSearch,
}

/// Classify a Claude-Code tool name (case-insensitive) into a canonical kind.
/// The single source of truth for the CC tool vocabulary; each harness renderer
/// maps `CcToolKind` to its own target string.
// Consumed by the per-harness translate_agent impls (Tasks 5–12). The allow
// silences the dead-code lint until those callers land.
#[allow(dead_code)]
pub(crate) fn classify_cc_tool(cc_tool: &str) -> Option<CcToolKind> {
    match cc_tool.trim().to_ascii_lowercase().as_str() {
        "read" => Some(CcToolKind::Read),
        "grep" => Some(CcToolKind::Grep),
        "glob" => Some(CcToolKind::Glob),
        "ls" => Some(CcToolKind::Ls),
        "edit" => Some(CcToolKind::Edit),
        "write" => Some(CcToolKind::Write),
        "multiedit" => Some(CcToolKind::MultiEdit),
        "notebookedit" => Some(CcToolKind::NotebookEdit),
        "bash" => Some(CcToolKind::Bash),
        "webfetch" => Some(CcToolKind::WebFetch),
        "websearch" => Some(CcToolKind::WebSearch),
        _ => None,
    }
}

/// Gemini: 1:1 native tool names (verified 2026-06-29 — write-side is
/// `write_file`/`replace`, NOT `edit_file`). Drops unknown (Gemini's `.strict()`
/// schema forbids unknown values). Preserves order; de-dupes.
// Consumed by the Gemini harness module (Task 5). The allow silences the
// dead-code lint until that caller lands.
#[allow(dead_code)]
pub(crate) fn gemini_tools(tools: &[String]) -> Vec<String> {
    map_tools_dedup(tools, |k| {
        Some(match k {
            CcToolKind::Read => "read_file",
            CcToolKind::Grep => "grep_search",
            CcToolKind::Glob => "glob",
            CcToolKind::Ls => "list_directory",
            CcToolKind::Edit | CcToolKind::MultiEdit | CcToolKind::NotebookEdit => "replace",
            CcToolKind::Write => "write_file",
            CcToolKind::Bash => "run_shell_command",
            CcToolKind::WebFetch => "web_fetch",
            CcToolKind::WebSearch => "google_web_search",
        })
    })
}

/// Kiro: category tags (`read`/`write`/`shell`/`web`), deduped, first-appearance order.
// Consumed by the Kiro harness module (Task 6). The allow silences the
// dead-code lint until that caller lands.
#[allow(dead_code)]
pub(crate) fn kiro_tools(tools: &[String]) -> Vec<String> {
    map_tools_dedup(tools, |k| {
        Some(match k {
            CcToolKind::Read | CcToolKind::Grep | CcToolKind::Glob | CcToolKind::Ls => "read",
            CcToolKind::Edit
            | CcToolKind::Write
            | CcToolKind::MultiEdit
            | CcToolKind::NotebookEdit => "write",
            CcToolKind::Bash => "shell",
            CcToolKind::WebFetch | CcToolKind::WebSearch => "web",
        })
    })
}

/// Devin: `allowed-tools` Devin-lowercase names (`Bash`→`exec`). Deduped.
// Consumed by the Devin harness module (Task 7). The allow silences the
// dead-code lint until that caller lands.
#[allow(dead_code)]
pub(crate) fn devin_tools(tools: &[String]) -> Vec<String> {
    map_tools_dedup(tools, |k| match k {
        CcToolKind::Read => Some("read"),
        CcToolKind::Write => Some("write"),
        CcToolKind::Edit | CcToolKind::MultiEdit | CcToolKind::NotebookEdit => Some("edit"),
        CcToolKind::Grep => Some("grep"),
        CcToolKind::Glob => Some("glob"),
        CcToolKind::Bash => Some("exec"),
        // No documented Devin name for these → drop.
        CcToolKind::Ls | CcToolKind::WebFetch | CcToolKind::WebSearch => None,
    })
}

/// Pi: comma-separated string (`read, grep, bash`). `None` when nothing maps.
// Consumed by the Pi harness module (Task 8). The allow silences the
// dead-code lint until that caller lands.
#[allow(dead_code)]
pub(crate) fn pi_tools(tools: &[String]) -> Option<String> {
    let mapped = map_tools_dedup(tools, |k| match k {
        CcToolKind::Read => Some("read"),
        CcToolKind::Grep => Some("grep"),
        CcToolKind::Glob => Some("find"),
        CcToolKind::Ls => Some("ls"),
        CcToolKind::Edit | CcToolKind::MultiEdit | CcToolKind::NotebookEdit => Some("edit"),
        CcToolKind::Write => Some("write"),
        CcToolKind::Bash => Some("bash"),
        // No Pi name for web tools → drop.
        CcToolKind::WebFetch | CcToolKind::WebSearch => None,
    });
    if mapped.is_empty() {
        None
    } else {
        Some(mapped.join(", "))
    }
}

/// Classify each CC tool, map it via `f`, drop `None`s, de-dupe preserving order.
fn map_tools_dedup(
    tools: &[String],
    f: impl Fn(CcToolKind) -> Option<&'static str>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tools {
        let Some(kind) = classify_cc_tool(t) else {
            continue;
        };
        let Some(target) = f(kind) else { continue };
        if !out.iter().any(|x| x == target) {
            out.push(target.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_md(name: &str, extra: &str, body: &str) -> String {
        format!("---\nname: {name}\n{extra}---\n{body}")
    }

    #[test]
    fn agent_filename_is_double_underscore_provenance() {
        assert_eq!(
            agent_filename("midnight-expert", "reviewer", "md"),
            "midnight-expert__reviewer.md"
        );
        assert_eq!(
            agent_filename("midnight-expert", "reviewer", "toml"),
            "midnight-expert__reviewer.toml"
        );
    }

    #[test]
    fn map_model_same_vendor_only() {
        let reg = crate::model_registry::test_registry();
        // opus → opencode resolves the newest same-vendor Anthropic id from
        // the registry (registry-driven, not a static string).
        assert_eq!(
            map_model(&reg, "opencode", "opus").as_deref(),
            Some("anthropic/claude-opus-4-5")
        );
        assert_eq!(
            map_model(&reg, "opencode", "sonnet").as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        assert_eq!(
            map_model(&reg, "opencode", "haiku").as_deref(),
            Some("anthropic/claude-haiku-4-5")
        );
        // opus → codex is DROP, never an OpenAI id.
        assert_eq!(map_model(&reg, "codex", "opus"), None);
        // claude-code passes the alias through verbatim.
        assert_eq!(
            map_model(&reg, "claude-code", "opus").as_deref(),
            Some("opus")
        );
        // cursor: any pinned model → `inherit` (proprietary ids, not in registry).
        assert_eq!(
            map_model(&reg, "cursor", "opus").as_deref(),
            Some("inherit")
        );
    }

    #[test]
    fn map_model_inherit_drops_everywhere() {
        let reg = crate::model_registry::test_registry();
        for harness in [
            "claude-code",
            "codex",
            "cursor",
            "opencode",
            "gemini",
            "kiro",
            "goose",
            "pi",
            "devin",
        ] {
            assert_eq!(
                map_model(&reg, harness, "inherit"),
                None,
                "inherit must drop for {harness}"
            );
        }
    }

    #[test]
    fn never_cross_vendor_model() {
        let reg = crate::model_registry::test_registry();
        // SC-002: no emitted file ever carries a cross-vendor id. codex is
        // OpenAI-vendored — every Anthropic source must drop.
        for source in ["opus", "sonnet", "haiku", "inherit", "something-else"] {
            assert_eq!(
                map_model(&reg, "codex", source),
                None,
                "codex must never carry an Anthropic-sourced model ({source})"
            );
        }
    }

    #[test]
    fn infer_read_only_allowlist_no_mutating_is_read_only() {
        let tools = vec!["Read".to_owned(), "Grep".to_owned(), "Glob".to_owned()];
        assert_eq!(infer_read_only(Some(&tools), None), Some(true));
    }

    #[test]
    fn infer_read_only_allowlist_with_write_is_not_read_only() {
        let tools = vec!["Read".to_owned(), "Edit".to_owned()];
        assert_eq!(infer_read_only(Some(&tools), None), Some(false));
        let tools = vec!["Bash".to_owned()];
        assert_eq!(infer_read_only(Some(&tools), None), Some(false));
    }

    #[test]
    fn infer_read_only_no_allowlist_is_indeterminate() {
        // Neither posture present → indeterminate (drop).
        assert_eq!(infer_read_only(None, None), None);
        // Partial deny → still indeterminate.
        let deny = vec!["Bash".to_owned()];
        assert_eq!(infer_read_only(None, Some(&deny)), None);
    }

    #[test]
    fn infer_read_only_full_deny_is_read_only() {
        // Deny every write/edit/execute tool → provably read-only.
        let deny: Vec<String> = WRITE_EDIT_EXECUTE_TOOLS
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(infer_read_only(None, Some(&deny)), Some(true));
    }

    #[test]
    fn displayed_name_clean_vs_clash() {
        assert_eq!(displayed_name("myplugin", "reviewer", false), "reviewer");
        assert_eq!(
            displayed_name("myplugin", "reviewer", true),
            "myplugin-reviewer"
        );
    }

    #[test]
    fn parse_round_trip_full_frontmatter() {
        let src = agent_md(
            "reviewer",
            "description: Reviews code\nmodel: opus\ntools:\n  - Read\n  - Grep\ndisallowedTools:\n  - Bash\npermissionMode: ask\n",
            "You are a careful reviewer.\nBe thorough.\n",
        );
        let agent = CanonicalAgent::parse("cat", "myplugin", "reviewer", &src)
            .expect("well-formed agent parses");
        assert_eq!(agent.catalog, "cat");
        assert_eq!(agent.plugin, "myplugin");
        assert_eq!(agent.name, "reviewer");
        assert_eq!(agent.description.as_deref(), Some("Reviews code"));
        assert_eq!(agent.model.as_deref(), Some("opus"));
        assert_eq!(
            agent.tools.as_deref(),
            Some(&["Read".to_owned(), "Grep".to_owned()][..])
        );
        assert_eq!(
            agent.disallowed_tools.as_deref(),
            Some(&["Bash".to_owned()][..])
        );
        assert_eq!(agent.permission_mode.as_deref(), Some("ask"));
        assert!(agent.body.contains("careful reviewer"));
    }

    #[test]
    fn parse_name_falls_back_to_filename_stem() {
        // No `name` key → filename stem is used.
        let src = "---\ndescription: x\n---\nbody\n";
        let agent = CanonicalAgent::parse("cat", "myplugin", "my-agent", src).expect("parses");
        assert_eq!(agent.name, "my-agent");
    }

    #[test]
    fn parse_malformed_frontmatter_is_exit_45() {
        // No closing delimiter → malformed → AgentTranslationFailed (45).
        let src = "---\nname: oops\nno closing delimiter here\n";
        let err = CanonicalAgent::parse("cat", "myplugin", "oops", src)
            .expect_err("malformed frontmatter must fail");
        assert!(matches!(err, TomeError::AgentTranslationFailed { .. }));
        assert_eq!(err.exit_code(), 45);
    }

    #[test]
    fn parse_wrong_typed_field_is_exit_45() {
        // `tools` declared as a scalar instead of a list → YAML type error
        // when deserialised into the agent struct → exit 45.
        let src = "---\nname: x\ntools: 7\n---\nbody\n";
        let err = CanonicalAgent::parse("cat", "myplugin", "x", src)
            .expect_err("wrong-typed field must fail");
        assert!(matches!(err, TomeError::AgentTranslationFailed { .. }));
    }

    #[test]
    fn codex_toml_puts_body_in_triple_quoted_developer_instructions() {
        let scalars = vec![
            ("name".to_owned(), "reviewer".to_owned()),
            ("description".to_owned(), "Reviews code".to_owned()),
        ];
        let body = "You are a careful reviewer.\nBe thorough.\n";
        let rendered = render_codex_toml(&scalars, body);
        assert!(
            rendered.contains("developer_instructions = \"\"\""),
            "body must land in a triple-quoted developer_instructions string:\n{rendered}"
        );
        assert!(rendered.contains("name = \"reviewer\""));
        assert!(rendered.contains("careful reviewer"));
        // Round-trips back to a parseable TOML document with the body intact.
        let doc: toml_edit::DocumentMut = rendered.parse().expect("valid TOML");
        assert_eq!(
            doc["developer_instructions"].as_str(),
            Some("You are a careful reviewer.\nBe thorough.\n")
        );
    }

    #[test]
    fn markdown_yaml_preserves_key_order_and_body() {
        let fm = vec![
            (
                "name".to_owned(),
                serde_yaml::Value::String("reviewer".to_owned()),
            ),
            (
                "description".to_owned(),
                serde_yaml::Value::String("Reviews code".to_owned()),
            ),
        ];
        let rendered = render_markdown_yaml(&fm, "Body text here.\n");
        assert!(rendered.starts_with("---\n"));
        // `name` is emitted before `description` (insertion order).
        let name_at = rendered.find("name:").expect("name present");
        let desc_at = rendered.find("description:").expect("description present");
        assert!(name_at < desc_at, "key order must be preserved");
        assert!(rendered.ends_with("Body text here.\n"));
    }

    /// SEC-1 regression: a hostile agent BODY that embeds its own `---`
    /// fence followed by privileged keys must NOT inject those keys into the
    /// parsed frontmatter. Only the LEADING `---…---` block is frontmatter;
    /// the embedded fence is a thematic break inside the body.
    #[test]
    fn body_with_frontmatter_delimiter_does_not_inject_fields() {
        let fm = vec![(
            "name".to_owned(),
            serde_yaml::Value::String("reviewer".to_owned()),
        )];
        // The body tries to re-open a frontmatter block and smuggle in
        // privilege-granting keys.
        let body = "intro line\n---\ntools: [Bash]\npermissionMode: bypassPermissions\n";
        let rendered = render_markdown_yaml(&fm, body);

        // Split on the SECOND `---\n` (the close of the leading block), then
        // parse ONLY that leading block as YAML — exactly what the destination
        // harness does.
        let after_open = rendered
            .strip_prefix("---\n")
            .expect("rendered file opens with a frontmatter delimiter");
        let close = after_open
            .find("\n---\n")
            .expect("rendered file has a closing frontmatter delimiter");
        let leading_yaml = &after_open[..close + 1];
        let parsed: serde_yaml::Mapping =
            serde_yaml::from_str(leading_yaml).expect("leading block parses as a YAML mapping");

        // The only frontmatter is Tome's own `name`; the injected keys are absent.
        assert!(
            parsed.contains_key(serde_yaml::Value::String("name".to_owned())),
            "Tome's own frontmatter survives: {parsed:?}"
        );
        assert!(
            !parsed.contains_key(serde_yaml::Value::String("tools".to_owned())),
            "the body's `tools` did NOT become frontmatter: {parsed:?}"
        );
        assert!(
            !parsed.contains_key(serde_yaml::Value::String("permissionMode".to_owned())),
            "the body's `permissionMode` did NOT become frontmatter: {parsed:?}"
        );

        // The injected text is still present — it just sits in the body, after
        // the first `---`-delimited block.
        let file_body = &after_open[close + "\n---\n".len()..];
        assert!(
            file_body.contains("permissionMode: bypassPermissions"),
            "the injected text lands verbatim in the body: {file_body}"
        );
    }

    #[test]
    fn plugin_of_owned_file_recovers_prefix() {
        assert_eq!(
            plugin_of_owned_file("myplugin__reviewer.md"),
            Some("myplugin")
        );
        assert_eq!(
            plugin_of_owned_file("myplugin__reviewer.toml"),
            Some("myplugin")
        );
        // Single underscore is not the provenance separator.
        assert_eq!(plugin_of_owned_file("myplugin_reviewer.md"), None);
        // Empty plugin prefix.
        assert_eq!(plugin_of_owned_file("__reviewer.md"), None);
        // Empty stem.
        assert_eq!(plugin_of_owned_file("myplugin__.md"), None);
    }

    #[test]
    fn synthesize_description_prefers_frontmatter_then_body_then_placeholder() {
        let mut a = CanonicalAgent {
            catalog: "c".into(),
            plugin: "p".into(),
            name: "solo".into(),
            description: Some("  Reviews code  ".into()),
            body: "Body line.\n".into(),
            model: None,
            tools: None,
            disallowed_tools: None,
            hooks: None,
            mcp_servers: None,
            permission_mode: None,
        };
        // Frontmatter wins (already trimmed at parse time; pass through verbatim).
        assert_eq!(synthesize_description(&a), "  Reviews code  ");
        // No description → first non-empty trimmed body line.
        a.description = None;
        a.body = "\n  First real line.  \nSecond.\n".into();
        assert_eq!(synthesize_description(&a), "First real line.");
        // Empty body → placeholder.
        a.body = "   \n\n".into();
        assert_eq!(
            synthesize_description(&a),
            "Agent solo (no description provided)."
        );
    }

    #[test]
    fn gemini_tools_maps_names_and_drops_unknown() {
        let t = vec![
            "Read".into(),
            "Bash".into(),
            "Edit".into(),
            "Frobnicate".into(),
        ];
        assert_eq!(
            gemini_tools(&t),
            vec!["read_file", "run_shell_command", "replace"]
        );
    }
    #[test]
    fn kiro_tools_collapses_to_categories_deduped() {
        let t = vec![
            "Read".into(),
            "Grep".into(),
            "Glob".into(),
            "Edit".into(),
            "Bash".into(),
            "WebFetch".into(),
        ];
        // read (x3 collapse), write (Edit), shell (Bash), web (WebFetch) — order = first appearance.
        assert_eq!(kiro_tools(&t), vec!["read", "write", "shell", "web"]);
    }
    #[test]
    fn pi_tools_comma_joins_and_devin_lowercases() {
        let t = vec!["Read".into(), "Grep".into(), "Bash".into()];
        assert_eq!(pi_tools(&t).as_deref(), Some("read, grep, bash"));
        let d = vec!["Bash".into(), "Write".into(), "Read".into()];
        assert_eq!(devin_tools(&d), vec!["exec", "write", "read"]);
        // Nothing maps → Pi returns None (omit the field).
        assert_eq!(pi_tools(&["Nope".to_owned()]), None);
    }

    #[test]
    fn devin_tools_drops_ls_and_web() {
        let t = vec![
            "Ls".into(),
            "WebFetch".into(),
            "WebSearch".into(),
            "Bash".into(),
        ];
        assert_eq!(devin_tools(&t), vec!["exec"]); // Ls/Web dropped, Bash maps
    }
    #[test]
    fn pi_tools_drops_web_but_keeps_others() {
        let t = vec!["WebFetch".into(), "Read".into(), "WebSearch".into()];
        assert_eq!(pi_tools(&t).as_deref(), Some("read")); // web dropped, Read maps
    }

    #[test]
    fn slugify_agent_name_handles_both_modes() {
        // allow_underscore = true (Gemini: [a-z0-9-_]).
        assert_eq!(
            slugify_agent_name("MyPlugin__Reviewer", true),
            "myplugin__reviewer"
        );
        assert_eq!(slugify_agent_name("Code Review!", true), "code-review");
        // allow_underscore = false (Kiro: lowercase + hyphens only).
        assert_eq!(
            slugify_agent_name("MyPlugin__Reviewer", false),
            "myplugin-reviewer"
        );
        assert_eq!(slugify_agent_name("a__b", false), "a-b");
        // Collapse repeats / trim edges; empty → fallback.
        assert_eq!(slugify_agent_name("--a  b--", true), "a-b");
        assert_eq!(slugify_agent_name("!!!", false), "agent");
        // Literal input hyphens collapse too (not only replacement dashes).
        assert_eq!(slugify_agent_name("a--b", false), "a-b");
        assert_eq!(slugify_agent_name("a--b", true), "a-b");
    }

    #[test]
    fn map_model_phase2_arms() {
        let reg = crate::model_registry::test_registry(); // opus→claude-opus-4-5, etc.
        // Gemini: any pinned model → inherit literal.
        assert_eq!(
            map_model(&reg, "gemini", "opus").as_deref(),
            Some("inherit")
        );
        assert_eq!(
            map_model(&reg, "gemini", "claude-x").as_deref(),
            Some("inherit")
        );
        assert_eq!(map_model(&reg, "gemini", "inherit"), None);
        // Kiro: always drop (dotted-only ids; ignored in dispatch).
        assert_eq!(map_model(&reg, "kiro", "opus"), None);
        // Goose + Pi: tier → registry bare id; concrete id passes through.
        assert_eq!(
            map_model(&reg, "goose", "opus").as_deref(),
            Some("claude-opus-4-5")
        );
        assert_eq!(
            map_model(&reg, "pi", "sonnet").as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(
            map_model(&reg, "pi", "claude-custom-1").as_deref(),
            Some("claude-custom-1")
        );
        // Third tier alias (haiku) resolves via the registry too.
        assert_eq!(
            map_model(&reg, "goose", "haiku").as_deref(),
            Some("claude-haiku-4-5")
        );
        // Devin: opus/sonnet pass through; haiku + concrete drop.
        assert_eq!(map_model(&reg, "devin", "opus").as_deref(), Some("opus"));
        assert_eq!(
            map_model(&reg, "devin", "sonnet").as_deref(),
            Some("sonnet")
        );
        assert_eq!(map_model(&reg, "devin", "haiku"), None);
        assert_eq!(map_model(&reg, "devin", "claude-opus-4-5"), None);
    }

    #[test]
    fn is_safe_agent_name_rejects_traversal_and_separators() {
        // Well-formed single segments are accepted.
        assert!(is_safe_agent_name("reviewer"));
        assert!(is_safe_agent_name("my-agent_v2"));
        // Path traversal in every spelling is rejected.
        assert!(!is_safe_agent_name("../../../../tmp/evil"));
        assert!(!is_safe_agent_name(".."));
        assert!(!is_safe_agent_name("."));
        assert!(!is_safe_agent_name("a/b"));
        assert!(!is_safe_agent_name("a\\b"));
        // Absolute, leading-dot, NUL, and empty are rejected.
        assert!(!is_safe_agent_name("/etc/passwd"));
        assert!(!is_safe_agent_name(".hidden"));
        assert!(!is_safe_agent_name("evil\u{0}name"));
        assert!(!is_safe_agent_name(""));
    }
}
