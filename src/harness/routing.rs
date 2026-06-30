//! Per-workspace skill-routing directive: turns the workspace's tiered entries
//! plus its cached summary into the agent-facing instructions that tell an
//! agent when to call the Tome MCP `get_skill` / `search_skills` tools.

use crate::error::TomeError;
use crate::index::skills::TieredEntry;
use crate::mcp::prompts::PromptRegistry;
use crate::paths::Paths;
use crate::plugin::identity::EntryKind;
use crate::workspace::WorkspaceName;

/// The self-heal MCP-availability preamble (FR-025, R11, contract
/// rules-delivery.md §Self-heal preamble).
///
/// Prepended VERBATIM by [`build_directive`] to a non-empty directive — and
/// only then — so it rides the single directive source into `.tome/RULES.md`,
/// every Inline rules block, every `@`-included sink, AND every session-start
/// hook / plugin envelope identically. A blank line separates it from the
/// existing `# Tome — Skill Routing` header. Keep these bytes stable: changing
/// them moves every byte-stable directive / rules-region pin.
pub const SELF_HEAL_PREAMBLE: &str = "Before doing anything else, verify the Tome MCP tools (search_skills, get_skill, get_skill_info) are available.\nIf they are NOT available, tell the user to run `tome harness use <their harness>` — or run\n`tome harness info <their harness>` to get the MCP config to paste into their harness — and then restart this session.\nDo not proceed with skill routing until the Tome tools are present.";

/// First line of a description, trimmed — keeps the directive scannable when a
/// description is multi-line.
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// The action a tiered entry's directive line points an agent at.
///
/// #289: a `skill` entry is loaded via `get_skill`; a `command` entry that is
/// user-invocable (every command by default) is INVOKED via the MCP prompt
/// surface — pointing it at `get_skill` produced a guaranteed-failing
/// instruction because `get_skill` historically rejected commands. A command
/// that opted out of the prompt surface (`user_invocable: false`) has no
/// prompt; its body is still readable via `get_skill` (which now resolves
/// commands too), so it falls back to the `GetSkill` action.
enum EntryAction {
    /// Load the entry body via `get_skill(catalog, plugin, name)`.
    GetSkill,
    /// Invoke the command via the MCP prompt named `<prompt_name>`.
    Prompt(String),
}

/// Resolve how a tiered entry should be addressed in the directive, honouring
/// the live [`PromptRegistry`] (the SSOT for the override + collision-resolved
/// prompt name). Skills always route to `get_skill`; a user-invocable command
/// routes to its prompt; a non-invocable command falls back to `get_skill`.
fn entry_action(e: &TieredEntry, registry: &PromptRegistry) -> EntryAction {
    match e.kind {
        EntryKind::Command => {
            match registry.prompt_name_for(&e.catalog, &e.plugin, EntryKind::Command, &e.name) {
                Some(prompt) => EntryAction::Prompt(prompt.to_owned()),
                None => EntryAction::GetSkill,
            }
        }
        // Skills + agents (agents never appear in `tiered_entries_for_workspace`)
        // load via get_skill.
        EntryKind::Skill | EntryKind::Agent => EntryAction::GetSkill,
    }
}

/// Render the call snippet a Tier-1 directive line ends with for an entry.
fn tier1_call(e: &TieredEntry, registry: &PromptRegistry) -> String {
    match entry_action(e, registry) {
        EntryAction::GetSkill => format!(
            "get_skill(catalog=\"{}\", plugin=\"{}\", name=\"{}\")",
            e.catalog, e.plugin, e.name,
        ),
        EntryAction::Prompt(prompt) => format!("the `{prompt}` MCP prompt"),
    }
}

/// Build the routing directive for a workspace from its tiered entries, an
/// optional cached long summary (used for the Tier 3 prose), and the live
/// [`PromptRegistry`] (the SSOT for command→prompt-name resolution). Pure and
/// deterministic: identical inputs → byte-identical output. An empty entry set
/// yields an empty string (no directive is injected for an empty workspace).
///
/// #289: command-kind entries are pointed at their MCP prompt, NOT `get_skill`
/// (which historically rejected commands → a guaranteed-failing instruction).
pub fn build_directive(
    entries: &[TieredEntry],
    long_summary: Option<&str>,
    registry: &PromptRegistry,
) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut tier1 = Vec::new();
    let mut tier2 = Vec::new();
    let mut tier3 = Vec::new();
    for e in entries {
        match e.tier {
            1 => tier1.push(e),
            2 => tier2.push(e),
            _ => tier3.push(e),
        }
    }

    let mut s = String::new();
    // Self-heal preamble FIRST (FR-025): the directive is non-empty here (the
    // empty-entries early return above already handled the empty case), so the
    // preamble + a blank-line separator always precede the routing header.
    s.push_str(SELF_HEAL_PREAMBLE);
    s.push_str("\n\n");
    s.push_str(
        "# Tome — Skill Routing\n\nYou have the Tome MCP server. Load skills with \
         get_skill and invoke commands via their MCP prompt, on demand.\n",
    );

    if !tier1.is_empty() {
        s.push_str("\n## Load now (Tier 1)\nAt session start, immediately call for each:\n");
        for e in &tier1 {
            s.push_str(&format!(
                "- {}  — {}\n",
                tier1_call(e, registry),
                first_line(&e.description),
            ));
        }
    }

    if !tier2.is_empty() {
        s.push_str(
            "\n## Load before matching work (Tier 2)\nBefore a task matching a \
             description below, first call its get_skill (or invoke its prompt):\n",
        );
        for e in &tier2 {
            s.push_str(&format!(
                "- \"{}\" → {}\n",
                first_line(&e.description),
                tier1_call(e, registry),
            ));
        }
    }

    s.push_str("\n## Search before related work (Tier 3)\nThis workspace's skills cover:\n");
    match long_summary.map(str::trim).filter(|t| !t.is_empty()) {
        Some(summary) => {
            s.push_str(summary);
            s.push('\n');
        }
        None => {
            for e in &tier3 {
                s.push_str(&format!("- {} — {}\n", e.name, first_line(&e.description)));
            }
        }
    }
    s.push_str(
        "Before any task in these areas, call search_skills(query=\"<the task>\") then \
         get_skill the top hit (or, for a command result, invoke its prompt_name).\n",
    );

    s
}

/// The byte-stable ARGS SUFFIX of the Claude/Codex SessionStart hook command —
/// everything AFTER the launcher (#337 Phase B). This is the launcher-tolerant
/// ownership discriminator: the launcher prefix varies per machine (resolved
/// via `tome_command()`), but this suffix is byte-identical, so
/// [`crate::harness::hooks::merge_tome_owned_into_settings`] /
/// [`remove_tome_owned_from_settings`](crate::harness::hooks::remove_tome_owned_from_settings)
/// recognise a previously-written entry across a launcher change. Keep these
/// bytes stable — they ARE the ownership marker for the session-start sinks.
pub fn session_start_args_suffix(workspace_name: &str) -> String {
    format!("harness session-start --workspace {workspace_name}")
}

/// The single Tome-owned Claude Code SessionStart hook entry: runs the
/// `session-start` command and lets Claude Code inject its stdout as
/// `additionalContext` each session.
///
/// Ownership is launcher-tolerant (#337 Phase B): the command's LAUNCHER prefix
/// is the resolved [`crate::harness::launcher::tome_command`] (so a PATH-less /
/// sandboxed host can start it), while the stable [`session_start_args_suffix`]
/// is the byte-identical discriminator the merge/remove paths match on. Pre-#337
/// the bare `"tome"` bytes were the marker; now the suffix is. `workspace_name`
/// pins the directive to the bound workspace, mirroring the `--workspace <ws>`
/// arg the MCP entry also carries.
pub fn session_start_hook(workspace_name: &str) -> crate::harness::hooks::RewrittenHooks {
    let command =
        crate::harness::launcher::tome_hook_command(&session_start_args_suffix(workspace_name));
    let entry = serde_json::json!({
        "hooks": [
            { "type": "command", "command": command }
        ]
    });
    crate::harness::hooks::RewrittenHooks {
        events: vec![("SessionStart".to_string(), vec![entry])],
    }
}

/// The Tome-owned Codex `SessionStart` hook entry. Codex's `hooks.json` entry
/// shape carries a `matcher` (matched against the session `source`) plus a
/// nested `hooks` array. `"startup|resume"` fires on fresh sessions and resumes
/// (not `clear`/`compact`). Ownership is launcher-tolerant (#337 Phase B): the
/// command's launcher prefix is the resolved [`crate::harness::launcher::tome_command`]
/// while the stable [`session_start_args_suffix`] is the matched discriminator.
/// The command runs the harness-agnostic `session-start` printer; its
/// plain-markdown stdout becomes Codex developer context.
pub fn codex_session_start_hook(workspace_name: &str) -> crate::harness::hooks::RewrittenHooks {
    let command =
        crate::harness::launcher::tome_hook_command(&session_start_args_suffix(workspace_name));
    let entry = serde_json::json!({
        "matcher": "startup|resume",
        "hooks": [
            { "type": "command", "command": command }
        ]
    });
    crate::harness::hooks::RewrittenHooks {
        events: vec![("SessionStart".to_string(), vec![entry])],
    }
}

/// Read the cached LONG summary text for a workspace from its `settings.toml`
/// `[summaries]` block. `None` when the file or block is absent / unparsable —
/// the Tier 3 section then falls back to enumeration. Mirrors
/// `commands::workspace::info::read_summary_cache` but returns the text.
pub fn read_cached_long_summary(paths: &Paths, name: &WorkspaceName) -> Option<String> {
    let settings_path = paths.workspace_settings_file(name);
    if !settings_path.is_file() {
        return None;
    }
    let body =
        crate::util::bounded_read_to_string(&settings_path, crate::util::TOME_CONFIG_MAX).ok()?;
    let parsed = crate::settings::parser::parse_workspace(&body).ok()?;
    parsed.summaries.map(|s| s.long)
}

/// Compose the workspace's `RULES.md` from its tiered entries + cached long
/// summary, atomic-write it, and sync the bytes to every bound project's
/// `.tome/RULES.md`. Cheap (no LLM) — safe to call on every enable / disable /
/// tier change. A genuinely absent index DB means "no enabled entries". Returns
/// the number of bound projects synced.
///
/// Calls the sibling [`crate::workspace::sync::sync_workspace_rules_to_bound_projects`]
/// — NOT `regen_summary::regen` — so there is no recursion back into this path.
pub fn write_workspace_rules(paths: &Paths, name: &WorkspaceName) -> Result<u32, TomeError> {
    let (entries, registry) = if paths.index_db.exists() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        let entries = crate::index::skills::tiered_entries_for_workspace(&conn, name.as_str())?;
        // #289: build the prompt registry so the directive can point command
        // entries at their MCP prompt (the SSOT for the override +
        // collision-resolved name). Personas off — they never appear in
        // `tiered_entries_for_workspace` (agents are excluded), so they cannot
        // be a tiered command/skill that needs prompt resolution. A registry
        // build failure degrades to an empty registry rather than failing the
        // rules write: commands then fall back to `get_skill` (which now
        // resolves them) instead of taking the whole rules sync down.
        let registry =
            PromptRegistry::build_for_workspace(name, paths, &conn, false).unwrap_or_default();
        (entries, registry)
    } else {
        (Vec::new(), PromptRegistry::default())
    };
    let summary = read_cached_long_summary(paths, name);
    let body = build_directive(&entries, summary.as_deref(), &registry);

    let rules_path = paths.workspace_rules_file(name);
    crate::catalog::store::write_atomic(&rules_path, body.as_bytes())?;

    crate::workspace::sync::sync_workspace_rules_to_bound_projects(name, paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::skills::TieredEntry;

    fn entry(name: &str, kind_skill: bool, tier: u8, desc: &str) -> TieredEntry {
        TieredEntry {
            catalog: "acme".into(),
            plugin: "db".into(),
            name: name.into(),
            kind: if kind_skill {
                crate::plugin::identity::EntryKind::Skill
            } else {
                crate::plugin::identity::EntryKind::Command
            },
            description: desc.into(),
            when_to_use: None,
            tier,
        }
    }

    /// A `PromptRegistry` with one command entry registered under the given
    /// final prompt name, so `build_directive` can resolve a tiered command to
    /// its prompt. Constructed via the public builder shape used by the MCP
    /// tests; here we hand-insert one entry to keep the unit test I/O-free.
    fn registry_with_command(prompt_name: &str) -> PromptRegistry {
        use crate::mcp::prompts::{PersonaRole, PromptEntry};
        let mut reg = PromptRegistry::default();
        reg.by_name.insert(
            prompt_name.to_owned(),
            PromptEntry {
                catalog: "acme".into(),
                plugin: "db".into(),
                name: "deploy".into(),
                kind: crate::plugin::identity::EntryKind::Command,
                description: "Deploy.".into(),
                path: std::path::PathBuf::new(),
                arguments: Vec::new(),
                argument_hint: None,
                body_uses_arguments: false,
                plugin_version: "1.0.0".into(),
                persona: PersonaRole::None,
                display_name: String::new(),
            },
        );
        reg
    }

    #[test]
    fn empty_entries_produce_empty_directive() {
        assert!(build_directive(&[], None, &PromptRegistry::default()).is_empty());
    }

    #[test]
    fn non_empty_directive_starts_with_self_heal_preamble() {
        let e = vec![entry("migrations", true, 1, "Safe schema migrations")];
        let out = build_directive(&e, None, &PromptRegistry::default());
        assert!(
            out.starts_with(SELF_HEAL_PREAMBLE),
            "directive must begin with the self-heal preamble; got:\n{out}",
        );
        // The preamble is followed by a blank line then the routing header.
        assert!(out.contains(&format!("{SELF_HEAL_PREAMBLE}\n\n# Tome — Skill Routing")));
        // The verbatim self-heal guidance is present.
        assert!(out.contains("verify the Tome MCP tools"));
        assert!(out.contains("tome harness use <their harness>"));
    }

    #[test]
    fn empty_directive_has_no_preamble() {
        // The empty-workspace floor must stay byte-empty: an empty directive
        // would otherwise inject a bare preamble with no routing content.
        assert!(
            build_directive(&[], Some("ignored summary"), &PromptRegistry::default()).is_empty()
        );
    }

    #[test]
    fn tier1_emits_get_skill_call_with_exact_args_for_skill() {
        let e = vec![entry("migrations", true, 1, "Safe schema migrations")];
        let out = build_directive(&e, None, &PromptRegistry::default());
        assert!(out.contains("## Load now (Tier 1)"));
        assert!(out.contains(r#"get_skill(catalog="acme", plugin="db", name="migrations")"#));
        assert!(out.contains("Safe schema migrations"));
    }

    #[test]
    fn tier1_command_points_at_prompt_not_get_skill() {
        // #289: a user-invocable command in the registry must be addressed via
        // its MCP prompt, NOT `get_skill` (which historically rejected
        // commands).
        let e = vec![entry("deploy", false, 1, "Deploy the app")];
        let reg = registry_with_command("db__deploy");
        let out = build_directive(&e, None, &reg);
        assert!(
            out.contains("the `db__deploy` MCP prompt"),
            "command must route to its prompt; got:\n{out}",
        );
        assert!(
            !out.contains(r#"get_skill(catalog="acme", plugin="db", name="deploy")"#),
            "command must NOT be pointed at get_skill; got:\n{out}",
        );
    }

    #[test]
    fn tier1_non_invocable_command_falls_back_to_get_skill() {
        // A command with no prompt (not in the registry) still has a readable
        // body via get_skill, so it falls back to the get_skill action rather
        // than producing a dangling prompt reference.
        let e = vec![entry("deploy", false, 1, "Deploy the app")];
        let out = build_directive(&e, None, &PromptRegistry::default());
        assert!(
            out.contains(r#"get_skill(catalog="acme", plugin="db", name="deploy")"#),
            "non-invocable command falls back to get_skill; got:\n{out}",
        );
    }

    #[test]
    fn tier2_command_points_at_prompt_not_get_skill() {
        // #289: a Tier-2 user-invocable command line must point at its MCP
        // prompt (and carry the Tier-2 "or invoke its prompt" framing), NOT
        // `get_skill`.
        let e = vec![entry("deploy", false, 2, "Deploy the app")];
        let reg = registry_with_command("db__deploy");
        let out = build_directive(&e, None, &reg);
        assert!(
            out.contains("## Load before matching work (Tier 2)"),
            "directive must carry the Tier 2 section; got:\n{out}",
        );
        assert!(
            out.contains("or invoke its prompt"),
            "Tier 2 header must frame the prompt path; got:\n{out}",
        );
        assert!(
            out.contains("the `db__deploy` MCP prompt"),
            "a Tier-2 command must route to its prompt; got:\n{out}",
        );
        assert!(
            !out.contains(r#"get_skill(catalog="acme", plugin="db", name="deploy")"#),
            "a Tier-2 command must NOT be pointed at get_skill; got:\n{out}",
        );
    }

    #[test]
    fn tier3_uses_summary_when_present_else_enumerates() {
        let e = vec![entry("notes", true, 3, "Release notes")];
        let with = build_directive(
            &e,
            Some("This workspace covers DB + release ops."),
            &PromptRegistry::default(),
        );
        assert!(with.contains("This workspace covers DB + release ops."));
        assert!(with.contains("search_skills(query="));

        let without = build_directive(&e, None, &PromptRegistry::default());
        assert!(without.contains("- notes — Release notes"));
    }

    #[test]
    fn read_cached_long_summary_round_trips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().join(".tome"));
        let name = WorkspaceName::parse("mine").unwrap();
        let settings_path = paths.workspace_settings_file(&name);
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            &settings_path,
            "name = \"mine\"\n\n[summaries]\nshort = \"s\"\nlong = \"the long body\"\n\
             generated_at = 2025-01-01T00:00:00Z\n",
        )
        .unwrap();

        assert_eq!(
            read_cached_long_summary(&paths, &name).as_deref(),
            Some("the long body"),
        );
    }

    #[test]
    fn read_cached_long_summary_none_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().join(".tome"));
        let name = WorkspaceName::parse("mine").unwrap();
        assert_eq!(read_cached_long_summary(&paths, &name), None);
    }

    #[test]
    fn codex_session_start_hook_has_matcher_and_command() {
        let h = codex_session_start_hook("my-ws");
        assert_eq!(h.events.len(), 1);
        let (event, entries) = &h.events[0];
        assert_eq!(event, "SessionStart");
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e["matcher"], "startup|resume");
        // #337 Phase B: the LAUNCHER prefix is resolved (the test binary's
        // current_exe), but the byte-stable args suffix is the ownership marker
        // and is recognised by the launcher-tolerant matcher.
        let cmd = e["hooks"][0]["command"].as_str().unwrap();
        let suffix = session_start_args_suffix("my-ws");
        assert!(
            crate::harness::launcher::looks_like_tome_hook_command(cmd, &suffix),
            "codex session command must be a recognised tome hook command: {cmd}",
        );
        assert!(cmd.ends_with(&suffix));
        assert_eq!(e["hooks"][0]["type"], "command");
    }

    #[test]
    fn session_start_hook_command_is_launcher_tolerant() {
        let h = session_start_hook("my-ws");
        let (_event, entries) = &h.events[0];
        let cmd = entries[0]["hooks"][0]["command"].as_str().unwrap();
        let suffix = session_start_args_suffix("my-ws");
        assert!(
            crate::harness::launcher::looks_like_tome_hook_command(cmd, &suffix),
            "claude session command must be a recognised tome hook command: {cmd}",
        );
    }
}
