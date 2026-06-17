//! Per-workspace skill-routing directive: turns the workspace's tiered entries
//! plus its cached summary into the agent-facing instructions that tell an
//! agent when to call the Tome MCP `get_skill` / `search_skills` tools.

use crate::error::TomeError;
use crate::index::skills::TieredEntry;
use crate::paths::Paths;
use crate::workspace::WorkspaceName;

/// First line of a description, trimmed — keeps the directive scannable when a
/// description is multi-line.
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

/// Build the routing directive for a workspace from its tiered entries and an
/// optional cached long summary (used for the Tier 3 prose). Pure and
/// deterministic: identical inputs → byte-identical output. An empty entry set
/// yields an empty string (no directive is injected for an empty workspace).
pub fn build_directive(entries: &[TieredEntry], long_summary: Option<&str>) -> String {
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
    s.push_str(
        "# Tome — Skill Routing\n\nYou have the Tome MCP server. Load skill \
         instructions on demand via its tools.\n",
    );

    if !tier1.is_empty() {
        s.push_str("\n## Load now (Tier 1)\nAt session start, immediately call for each:\n");
        for e in &tier1 {
            s.push_str(&format!(
                "- get_skill(catalog=\"{}\", plugin=\"{}\", name=\"{}\")  — {}\n",
                e.catalog,
                e.plugin,
                e.name,
                first_line(&e.description),
            ));
        }
    }

    if !tier2.is_empty() {
        s.push_str(
            "\n## Load before matching work (Tier 2)\nBefore a task matching a \
             description below, first call its get_skill:\n",
        );
        for e in &tier2 {
            s.push_str(&format!(
                "- \"{}\" → get_skill(catalog=\"{}\", plugin=\"{}\", name=\"{}\")\n",
                first_line(&e.description),
                e.catalog,
                e.plugin,
                e.name,
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
         get_skill the top hit.\n",
    );

    s
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
    let entries = if paths.index_db.exists() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::skills::tiered_entries_for_workspace(&conn, name.as_str())?
    } else {
        Vec::new()
    };
    let summary = read_cached_long_summary(paths, name);
    let body = build_directive(&entries, summary.as_deref());

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

    #[test]
    fn empty_entries_produce_empty_directive() {
        assert!(build_directive(&[], None).is_empty());
    }

    #[test]
    fn tier1_emits_get_skill_call_with_exact_args() {
        let e = vec![entry("migrations", true, 1, "Safe schema migrations")];
        let out = build_directive(&e, None);
        assert!(out.contains("## Load now (Tier 1)"));
        assert!(out.contains(r#"get_skill(catalog="acme", plugin="db", name="migrations")"#));
        assert!(out.contains("Safe schema migrations"));
    }

    #[test]
    fn tier3_uses_summary_when_present_else_enumerates() {
        let e = vec![entry("notes", true, 3, "Release notes")];
        let with = build_directive(&e, Some("This workspace covers DB + release ops."));
        assert!(with.contains("This workspace covers DB + release ops."));
        assert!(with.contains("search_skills(query="));

        let without = build_directive(&e, None);
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
}
