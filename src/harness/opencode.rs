//! `opencode` — OpenCode CLI.
//!
//! Per research §R-8 + `contracts/harness-modules.md`:
//!
//! - Per-user dir: `~/.opencode/`.
//! - Rules-file target: `AGENTS.md`.
//! - Strategy: `BlockInExistingFile`. Body style is `Inline` — OpenCode
//!   does not document `@`-include support, so the block holds the
//!   full rules content verbatim and the sync algorithm must rewrite
//!   the block on every summary regeneration.
//! - MCP config: `<project>/opencode.json` (per-project, no dot
//!   prefix).
//! - Parent key: `"mcpServers"`.
//! - Guardrails target (Phase 6): `AGENTS.md` in-file region, no
//!   suppression — the trait default (`InFileRegion` on the rules-file
//!   target) is exactly correct, so no `guardrails_target` override is
//!   needed (FR-012). Shares the `AGENTS.md` file with codex / gemini.

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{
    self, CanonicalAgent, TranslatedAgent, agent_extension, agent_filename,
};
use crate::harness::{
    AgentFormat, BlockBodyStyle, EntryShape, ExtraField, ExtraValue, FileFormat, HarnessModule,
    McpDialect, RulesFileStrategy, ServerType, SessionSteering, ShimKind,
};

/// Unit struct implementing [`HarnessModule`] for OpenCode CLI.
pub struct OpenCode;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const OPENCODE: OpenCode = OpenCode;

impl HarnessModule for OpenCode {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn description(&self) -> &'static str {
        "OpenCode CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".opencode").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }

    /// OpenCode receives Tome's session-start directive through an embedded
    /// TypeScript plugin shim (Phase 11 / G2, US3) — it has no native
    /// session-start hook file. The dir is PROJECT-RELATIVE: `reconcile_plugins`
    /// anchors it under `project_root` via `project_root.join(dir)`. The shim
    /// lands at `<project>/.opencode/plugin/tome.ts` (singular `plugin/`, per the
    /// contract), a dedicated file inside OpenCode's own plugin dir. This is the
    /// ONLY behavioural addition for OpenCode in Phase 11 US3 — every other sink
    /// (rules / MCP dialect / native agents / native skills) is unchanged.
    fn session_steering(&self) -> SessionSteering {
        SessionSteering::TsPlugin {
            dir: PathBuf::from(".opencode/plugin"),
            kind: ShimKind::OpenCode,
        }
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("opencode.json")
    }

    /// OpenCode's MCP dialect (Phase 11 G1 fix — the canary proving the
    /// dialect generalization). The Phase ≤10 emit used the legacy
    /// `mcpServers` + `command`/`args` shape, which is SUSPECTED WRONG for
    /// OpenCode. The contract (`contracts/mcp-dialects.md`) pins the real
    /// shape:
    ///
    /// ```jsonc
    /// { "mcp": { "tome": {
    ///     "type": "local",
    ///     "command": ["tome", "mcp", "--workspace", "<ws>", "--harness", "opencode"],
    ///     "enabled": true
    /// } } }
    /// ```
    ///
    /// - `mcp` parent key (NOT `mcpServers`).
    /// - `CommandArray` body — the launcher AND args live in ONE
    ///   `command` array; there is no separate `args` key. The ownership
    ///   predicate becomes `command[0] == "tome" && command[1] == "mcp"`
    ///   by construction of the read-side normalisation.
    /// - `type: "local"` discriminator.
    /// - `enabled: true` mandated field, re-derived on every rewrite.
    /// - `Jsonc` format (OpenCode tolerates comments in `opencode.json`;
    ///   Tome still emits plain JSON).
    ///
    /// A live-probe gate (the `#[ignore]`d test below) confirms this is
    /// what OpenCode actually reads before the fix ships.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Jsonc,
            parent_key: "mcp",
            entry_shape: EntryShape::CommandArray,
            entry_type: Some(ServerType::Local),
            emit_env: false,
            extra_fields: &[ExtraField {
                key: "enabled",
                value: ExtraValue::Bool(true),
            }],
        }
    }

    // -- Native agents (FR-030–FR-036, FR-042) ------------------------------

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        // Note: singular `agent/` (not `agents/`) per the contract.
        Some(project_root.join(".opencode/agent"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::MarkdownYaml)
    }

    // -- Native skills (Phase 9, harness-skill-emit.md) ---------------------

    fn supports_native_skills(&self) -> bool {
        true
    }

    fn skill_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".opencode/skills"))
    }

    fn skill_dir_global(&self, home: &Path) -> Option<PathBuf> {
        // OpenCode's user-scope config lives under XDG `~/.config/opencode/`
        // (the per-user dir it probes is `~/.opencode/`, but global skills land
        // in the XDG config tree per the contract table).
        Some(home.join(".config/opencode/skills"))
    }

    /// OpenCode derives the agent name from the FILENAME, so the displayed
    /// name is ALWAYS `<plugin>__<name>` regardless of the workspace clash
    /// flag (FR-042 — the prefix cannot be hidden, an accepted wart).
    ///
    /// `description` is REQUIRED by OpenCode (FR-035): when the source lacks
    /// one, fall back to the first non-empty trimmed body line, else the
    /// documented placeholder. `mode` defaults to `subagent`. `model` ports
    /// via the same-vendor `anthropic/...` aliases. Read-only intent maps to
    /// per-tool `permission` entries (edit/bash → `deny`); a not-read-only or
    /// indeterminate posture drops the permission block.
    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        _clashes: bool,
    ) -> Result<TranslatedAgent, TomeError> {
        let mut frontmatter: Vec<(String, serde_yaml::Value)> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();

        // Filename-derived name: always the full `<plugin>__<name>` form.
        let displayed_name = format!("{}__{}", canonical.plugin, canonical.name);

        // `description` is required — resolve via the FR-035 fallback chain.
        let description = resolve_description(canonical);
        frontmatter.push((
            "description".to_owned(),
            serde_yaml::Value::String(description),
        ));

        // Source agents are subagents.
        frontmatter.push((
            "mode".to_owned(),
            serde_yaml::Value::String("subagent".to_owned()),
        ));

        // `model` ports via the OpenCode same-vendor table (anthropic/...).
        if let Some(src) = canonical.model.as_deref() {
            match agents::map_model("opencode", src) {
                Some(mapped) => {
                    frontmatter.push(("model".to_owned(), serde_yaml::Value::String(mapped)));
                }
                None => dropped.push("model".to_owned()),
            }
        }

        // Read-only intent → per-tool `permission` block (edit/bash → deny).
        // A not-read-only or indeterminate posture inherits OpenCode's
        // default. C-2: when the intent is NOT reconstructed, record the
        // canonical SOURCE field(s) responsible (`tools` / `disallowedTools`)
        // — NOT the harness target name `permission` — so the US5 doctor's
        // `DroppedFieldEntry` names the source. OpenCode records these source
        // fields nowhere else, so we record them here on the drop path.
        //
        // C-1: only read-only *intent* is reconstructed; a non-read-only
        // restrictive `tools` allowlist is dropped (full allowlist→per-tool
        // permission translation is deferred).
        match agents::infer_read_only(
            canonical.tools.as_deref(),
            canonical.disallowed_tools.as_deref(),
        ) {
            Some(true) => {
                let mut perm = serde_yaml::Mapping::new();
                perm.insert(
                    serde_yaml::Value::String("edit".to_owned()),
                    serde_yaml::Value::String("deny".to_owned()),
                );
                perm.insert(
                    serde_yaml::Value::String("bash".to_owned()),
                    serde_yaml::Value::String("deny".to_owned()),
                );
                frontmatter.push(("permission".to_owned(), serde_yaml::Value::Mapping(perm)));
            }
            _ => {
                if canonical.tools.is_some() {
                    dropped.push("tools".to_owned());
                }
                if canonical.disallowed_tools.is_some() {
                    dropped.push("disallowedTools".to_owned());
                }
            }
        }

        // The privileged fields have no OpenCode carrier.
        if canonical.hooks.is_some() {
            dropped.push("hooks".to_owned());
        }
        if canonical.mcp_servers.is_some() {
            dropped.push("mcpServers".to_owned());
        }
        if canonical.permission_mode.is_some() {
            dropped.push("permissionMode".to_owned());
        }

        let rendered = agents::render_markdown_yaml(&frontmatter, &canonical.body);
        let filename = agent_filename(
            &canonical.plugin,
            &canonical.name,
            agent_extension(AgentFormat::MarkdownYaml),
        );

        Ok(TranslatedAgent {
            dir: PathBuf::from(".opencode/agent"),
            filename,
            displayed_name,
            format: AgentFormat::MarkdownYaml,
            rendered,
            dropped_fields: dropped,
        })
    }
}

/// Resolve OpenCode's required `description` (FR-035).
///
/// Precedence: the canonical `description` if present; else the first
/// non-empty trimmed line of the body; else the documented placeholder.
fn resolve_description(canonical: &CanonicalAgent) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base(name: &str) -> CanonicalAgent {
        CanonicalAgent {
            catalog: "cat".into(),
            plugin: "myplugin".into(),
            name: name.into(),
            description: None,
            body: String::new(),
            model: Some("opus".into()),
            tools: Some(vec!["Read".into()]),
            disallowed_tools: None,
            hooks: None,
            mcp_servers: None,
            permission_mode: None,
        }
    }

    #[test]
    fn name_always_filename_derived_even_without_clash() {
        let agent = CanonicalAgent {
            body: "Body line.\n".into(),
            ..base("reviewer")
        };
        // `clashes = false` but OpenCode name is ALWAYS `<plugin>__<name>`.
        let t = OPENCODE.translate_agent(&agent, false).unwrap();
        assert_eq!(t.displayed_name, "myplugin__reviewer");
        assert_eq!(t.filename, "myplugin__reviewer.md");
    }

    #[test]
    fn mode_subagent_model_mapped_readonly_permission() {
        let agent = CanonicalAgent {
            body: "First body line.\n".into(),
            ..base("reviewer")
        };
        let t = OPENCODE.translate_agent(&agent, false).unwrap();
        assert!(t.rendered.contains("mode: subagent"));
        // opus → opencode same-vendor anthropic id.
        assert!(t.rendered.contains("model: anthropic/claude-opus-4.7"));
        // read-only intent → per-tool permission deny block.
        assert!(t.rendered.contains("permission:"));
        assert!(t.rendered.contains("edit: deny"));
        assert!(t.rendered.contains("bash: deny"));
    }

    #[test]
    fn description_falls_back_to_first_body_line_then_placeholder() {
        // No frontmatter description → first non-empty body line.
        let agent = CanonicalAgent {
            body: "\n  First real line.  \nSecond line.\n".into(),
            ..base("reviewer")
        };
        let t = OPENCODE.translate_agent(&agent, false).unwrap();
        assert!(
            t.rendered.contains("description: First real line."),
            "first non-empty body line is the description:\n{}",
            t.rendered
        );

        // Empty body → documented placeholder.
        let empty = base("solo");
        let t2 = OPENCODE.translate_agent(&empty, false).unwrap();
        assert!(
            t2.rendered
                .contains("Agent solo (no description provided)."),
            "empty body → placeholder:\n{}",
            t2.rendered
        );
    }

    #[test]
    fn mcp_dialect_is_the_g1_canary_shape() {
        use crate::harness::{EntryShape, ExtraValue, FileFormat, HarnessModule, ServerType};
        let d = OPENCODE.mcp_dialect();
        assert_eq!(d.file_format, FileFormat::Jsonc);
        assert_eq!(d.parent_key, "mcp");
        assert_eq!(d.entry_shape, EntryShape::CommandArray);
        assert_eq!(d.entry_type, Some(ServerType::Local));
        assert!(!d.emit_env);
        assert_eq!(d.extra_fields.len(), 1);
        assert_eq!(d.extra_fields[0].key, "enabled");
        assert_eq!(d.extra_fields[0].value, ExtraValue::Bool(true));
        // The Phase ≤10 scalar accessors derive from the dialect.
        assert_eq!(
            d.config_format(),
            crate::harness::McpConfigFormat::Json,
            "Jsonc maps to the serde_json read/write path"
        );
        assert_eq!(OPENCODE.mcp_parent_key(), "mcp");
    }

    /// Phase 11 / US3 (T057): OpenCode's session steering is the embedded
    /// `TsPlugin` shim, project-relative dir `.opencode/plugin` (singular
    /// `plugin/`), `ShimKind::OpenCode`.
    #[test]
    fn session_steering_is_opencode_ts_plugin() {
        assert_eq!(
            OPENCODE.session_steering(),
            SessionSteering::TsPlugin {
                dir: PathBuf::from(".opencode/plugin"),
                kind: ShimKind::OpenCode,
            },
        );
    }

    /// T058 — shim byte pin: OpenCode's embedded `tome.ts` is non-empty.
    #[test]
    fn embedded_shim_is_non_empty() {
        let plugin = crate::harness::plugin_assets::find("opencode")
            .expect("opencode shim must be embedded");
        let entry = plugin
            .files
            .iter()
            .find(|f| f.rel_path == "tome.ts")
            .expect("opencode shim must contain tome.ts");
        assert!(
            !entry.bytes.is_empty(),
            "opencode tome.ts must be non-empty"
        );
    }

    /// T058 — invocation + fail-closed pin: the embedded shim invokes
    /// `tome … session-start … --harness opencode` (B3) and no-ops fail-closed
    /// on a missing binary.
    #[test]
    fn embedded_shim_invokes_session_start_and_fails_closed() {
        let plugin = crate::harness::plugin_assets::find("opencode").unwrap();
        let src = std::str::from_utf8(plugin.files[0].bytes).expect("shim is UTF-8");
        assert!(src.contains("\"tome\""), "shim launches the `tome` binary");
        assert!(
            src.contains("session-start"),
            "shim runs the session-start subcommand",
        );
        assert!(
            src.contains("\"--harness\"") && src.contains("\"opencode\""),
            "shim passes --harness opencode (defers to the Rust directive source)",
        );
        assert!(
            src.contains("catch") && src.contains("return \"\""),
            "shim must fail closed (catch → empty string → no injection) on a missing binary",
        );
    }

    /// Live-probe merge gate (R14 / T087). NOT run in CI — a human must run
    /// this against a real OpenCode install before the G1 fix ships.
    ///
    /// What to verify by hand:
    ///
    /// 1. `tome harness use opencode` (or `tome sync --harness opencode`) in a
    ///    workspace-bound project, then open the generated `opencode.json`.
    /// 2. Confirm the Tome entry is under the top-level `"mcp"` key (NOT
    ///    `"mcpServers"`), with `"type": "local"`, a single `"command"` array
    ///    `["tome","mcp","--workspace","<ws>","--harness","opencode"]` (NO
    ///    separate `"args"` key), and `"enabled": true`.
    /// 3. Start OpenCode in that project and confirm it actually discovers and
    ///    connects to the Tome MCP server (tools `search_skills` / `get_skill`
    ///    appear). This is the assertion the unit pins cannot make: that
    ///    OpenCode READS this shape. The current `mcpServers`+command/args
    ///    emit is suspected wrong; this probe is what confirms the fix.
    #[test]
    #[ignore = "live-probe: confirm OpenCode reads the `mcp`+command-array+type:local shape against a real install"]
    fn opencode_reads_mcp_command_array_shape_live_probe() {
        // No automated body — see the doc comment for the manual checklist a
        // human runs against a real OpenCode install. Present so the gate is
        // discoverable via `cargo test -- --ignored`.
    }
}
