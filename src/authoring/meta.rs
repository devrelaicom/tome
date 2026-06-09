//! Embedded meta-skill registry + the shared install / remove / drift compute.
//!
//! Tome ships its own curated, Tome-authored **meta skills** — native `SKILL.md`
//! folders embedded in the binary (authored under `assets/meta-skills/`, pulled
//! in by the `build.rs` manifest generator). This module is the single **sync**
//! compute path behind BOTH the `tome meta` CLI and the MCP `meta` tool
//! (NFR-005): install a skill folder into a harness's `skills/` dir, remove it,
//! and probe an on-disk install for drift against the embedded revision.
//!
//! The write path inherits the project's SSOTs (Principle XII): atomic
//! populated-directory landing via [`crate::util::land_directory_with_replace`]
//! and the symlink-safe pre-write guard [`crate::util::refuse_symlinked_component`]
//! — the same guards the Phase-6 native-agent sink uses. Failures map to the
//! dedicated closed-set codes 87 (`MetaSkillNotFound`) / 88 (`MetaInstallFailed`),
//! never `Io` (7), mirroring the agent-sink precedent (P6/P8 CON-1).
//!
//! Sync-only — the async island is `src/mcp/`; `tests/sync_boundary.rs` guards
//! this tree.

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::plugin::identity::validate_segment;
use crate::util::{
    ENTRY_BODY_MAX, bounded_read_to_string, land_directory_with_replace, refuse_symlinked_component,
};

/// One file embedded in the binary as part of a meta skill.
pub struct EmbeddedFile {
    /// POSIX-relative path inside the skill folder (`SKILL.md`,
    /// `references/x.md`, …). Proven `Normal`-only at build time.
    pub rel_path: &'static str,
    pub bytes: &'static [u8],
}

/// One embedded meta skill — a record in the `build.rs`-generated manifest.
pub struct EmbeddedMetaSkill {
    /// kebab-case id; equals the on-disk install folder name; a safe path
    /// segment (validated at build time).
    pub id: &'static str,
    /// One-line summary (the SKILL.md frontmatter `description`), for
    /// `tome meta list`.
    pub summary: &'static str,
    /// Content-hash revision (sha256-short over the sorted file bytes),
    /// computed at build time (R-2). Drift compares this for **inequality**
    /// only — no ordering is defined.
    pub revision: &'static str,
    /// The reserved built-in MCP prompt this skill declares, if any (US3).
    pub prompt_name: Option<&'static str>,
    pub files: &'static [EmbeddedFile],
}

// The generated `META_SKILLS: &[EmbeddedMetaSkill]` slice (see build.rs). The
// `EmbeddedMetaSkill`/`EmbeddedFile` names above are in scope at this site.
include!(concat!(env!("OUT_DIR"), "/meta_skills_manifest.rs"));

/// Frontmatter map key Tome stamps the revision under at install (R-2). Nested
/// under `metadata:` so it lives in the lenient third-party `metadata` map that
/// harnesses tolerate, alongside the native `name`/`description`.
pub const METADATA_KEY: &str = "metadata";
/// The revision sub-key under [`METADATA_KEY`].
pub const REVISION_KEY: &str = "tome_skill_revision";

/// Look up an embedded skill by id. Linear over a tiny compile-time slice
/// (O(1) in practice).
pub fn find(id: &str) -> Option<&'static EmbeddedMetaSkill> {
    META_SKILLS.iter().find(|s| s.id == id)
}

/// All embedded skills (registry order = build.rs sorted-by-id order).
pub fn all() -> &'static [EmbeddedMetaSkill] {
    META_SKILLS
}

/// Success result of [`install_skill`].
#[derive(Debug, Clone)]
pub struct InstalledAt {
    /// The skills root written under (e.g. `<project>/.claude/skills`).
    pub target_dir: PathBuf,
    /// The owned skill folder (`<target_dir>/<id>`), canonicalised.
    pub skill_dir: PathBuf,
    /// The embedded revision stamped into the landed `SKILL.md`.
    pub revision: String,
}

/// Outcome of [`remove_skill`] at one location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// The owned `<id>/` folder existed and was deleted.
    Removed,
    /// Nothing to remove at this location (idempotent no-op).
    NotPresent,
}

/// Install an embedded meta skill into `target_dir` (a harness `skills/` root).
///
/// Guarantees (contract `harness-skill-emit.md`):
/// 1. Unknown `skill_id` → [`TomeError::MetaSkillNotFound`] (87).
/// 2. The resolved id is validated as a safe single path segment;
///    failure → [`TomeError::MetaInstallFailed`] (88).
/// 3. Content is staged in a `.tome.tmp.*` sibling with the embedded `revision`
///    stamped into `SKILL.md`, then POSIX-atomically renamed into
///    `<target_dir>/<id>/` via [`land_directory_with_replace`].
/// 4. The resolved target is symlink-guarded **before** the write; any
///    symlinked component → [`TomeError::MetaInstallFailed`] (88), with **no
///    write outside `target_dir`**.
/// 5. Idempotent: replaces an existing same-id folder; an up-to-date no-`force`
///    no-op is the caller's concern (it checks the on-disk revision first).
pub fn install_skill(skill_id: &str, target_dir: &Path) -> Result<InstalledAt, TomeError> {
    let skill = find(skill_id).ok_or_else(|| not_found(skill_id))?;

    // Defence-in-depth: the embedded id was validated safe at build time, so
    // this never fires in practice — but the path join below MUST use a proven
    // segment. Use the resolved `skill.id` (not the raw caller string) so a weird
    // caller input can only ever map to a known-safe id or `MetaSkillNotFound`.
    if validate_segment(skill.id).is_err() {
        return Err(install_failed(
            skill.id,
            target_dir,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "embedded skill id `{}` is not a safe path segment",
                    skill.id
                ),
            ),
        ));
    }

    let skill_dir = target_dir.join(skill.id);

    // Symlink-safe pre-write guard on the resolved target — dedicated 88, not
    // the `Io` (7) that `land_directory` would surface for its own internal
    // check. Mirrors `reconcile/agents.rs` guarding the write sink before emit.
    if let Err(e) = refuse_symlinked_component(&skill_dir) {
        return Err(install_failed(skill.id, target_dir, e));
    }

    let revision = skill.revision.to_owned();
    let landed = land_directory_with_replace(&skill_dir, 0o755, |staged| populate(skill, staged))
        .map_err(|e| install_failed(skill.id, target_dir, into_io(e)))?;

    Ok(InstalledAt {
        target_dir: target_dir.to_owned(),
        skill_dir: landed,
        revision,
    })
}

/// Write the skill's files into the staged directory, stamping the embedded
/// revision into `SKILL.md`. Sub-paths (`references/…`) get their parent dirs
/// created. The `rel_path`s are `Normal`-only by build-time construction.
fn populate(skill: &EmbeddedMetaSkill, staged: &Path) -> Result<(), TomeError> {
    for file in skill.files {
        let dest = staged.join(file.rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if file.rel_path == "SKILL.md" {
            let stamped = stamp_revision(file.bytes, skill.revision);
            std::fs::write(&dest, stamped)?;
        } else {
            std::fs::write(&dest, file.bytes)?;
        }
    }
    Ok(())
}

/// Remove the owned `<id>/` skill folder under `target_dir`.
///
/// - Unknown `skill_id` → [`TomeError::MetaSkillNotFound`] (87).
/// - Folder absent → [`RemoveOutcome::NotPresent`] (idempotent no-op).
/// - Same symlink-safe guard as install; a refused/failed delete → 88.
pub fn remove_skill(skill_id: &str, target_dir: &Path) -> Result<RemoveOutcome, TomeError> {
    let skill = find(skill_id).ok_or_else(|| not_found(skill_id))?;
    let skill_dir = target_dir.join(skill.id);

    if let Err(e) = refuse_symlinked_component(&skill_dir) {
        return Err(install_failed(skill.id, target_dir, e));
    }
    if !skill_dir.exists() {
        return Ok(RemoveOutcome::NotPresent);
    }
    std::fs::remove_dir_all(&skill_dir).map_err(|e| install_failed(skill.id, target_dir, e))?;
    Ok(RemoveOutcome::Removed)
}

/// Drift classification for one (skill, skills-root) location (data-model §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftState {
    /// On-disk revision equals the embedded revision.
    UpToDate,
    /// Installed but the revision differs, is missing, or the file is
    /// malformed/oversized/non-UTF-8 — all refreshable (FR-031b).
    Stale {
        installed_rev: Option<String>,
        embedded_rev: String,
    },
    /// No install at this location (the `<id>/SKILL.md` is absent).
    MissingButExpected,
}

/// Probe `<dir>/<skill_id>/SKILL.md` for drift against the embedded revision.
///
/// Read is **bounded + UTF-8-fail-closed** (FR-031b): an over-cap, non-UTF-8,
/// or marker-less file is treated as [`DriftState::Stale`] (refreshable), never
/// a halt. An absent `SKILL.md` is [`DriftState::MissingButExpected`].
///
/// Read/write parity (P9 LOW-1, the P8 "route EVERY untrusted read through the
/// one guard" pattern): the read is gated behind the SAME
/// [`refuse_symlinked_component`] guard the write path runs. A symlinked
/// component in `<dir>/<id>` is classified [`DriftState::Stale`] (refreshable —
/// the subsequent install re-runs the full write guard and refuses/lands
/// safely) rather than read through the link, so this probe can never disclose
/// out-of-tree content. The honest posture mirrors the lint parser: a refusal
/// degrades to a benign result, never a panic.
pub fn drift_probe(skill_id: &str, dir: &Path) -> DriftState {
    let Some(skill) = find(skill_id) else {
        // Not an embedded skill — nothing to expect here.
        return DriftState::MissingButExpected;
    };
    let skill_md = dir.join(skill.id).join("SKILL.md");

    // Read-side symlink guard: refuse to read THROUGH a symlinked component of
    // the owned `<dir>/<id>` folder (the write path's `refuse_symlinked_component`
    // analogue). A refusal degrades to refreshable `Stale` — never a read.
    if refuse_symlinked_component(&skill_md).is_err() {
        return DriftState::Stale {
            installed_rev: None,
            embedded_rev: skill.revision.to_owned(),
        };
    }

    match bounded_read_to_string(&skill_md, ENTRY_BODY_MAX) {
        Ok(content) => match extract_revision(&content) {
            Some(rev) if rev == skill.revision => DriftState::UpToDate,
            other => DriftState::Stale {
                installed_rev: other,
                embedded_rev: skill.revision.to_owned(),
            },
        },
        Err(_) => {
            // Distinguish "absent" (→ missing) from "present but unreadable"
            // (over-cap / non-UTF-8 → refreshable). `exists()` follows the
            // final symlink, which is fine for an existence test.
            if skill_md.exists() {
                DriftState::Stale {
                    installed_rev: None,
                    embedded_rev: skill.revision.to_owned(),
                }
            } else {
                DriftState::MissingButExpected
            }
        }
    }
}

// --- revision stamp / read (frontmatter `metadata.tome_skill_revision`) ------

/// Stamp `revision` into the SKILL.md frontmatter under
/// `metadata.tome_skill_revision`, preserving the body verbatim. Operates on
/// the EMBEDDED (unstamped) bytes each install, so the stamp never accumulates
/// across re-installs. If the bytes have no parseable frontmatter (should never
/// happen for an authored, lint-gated skill), they are written through
/// unchanged — the drift probe then treats the install as refreshable.
fn stamp_revision(skill_md: &[u8], revision: &str) -> Vec<u8> {
    let Ok(content) = std::str::from_utf8(skill_md) else {
        return skill_md.to_vec();
    };
    let Some((yaml, body)) = split_frontmatter(content) else {
        return skill_md.to_vec();
    };
    let mut map: serde_yaml::Mapping = match serde_yaml::from_str(yaml) {
        Ok(m) => m,
        Err(_) => return skill_md.to_vec(),
    };
    // Get-or-create the nested `metadata` mapping, then set the revision key.
    let metadata_key = serde_yaml::Value::String(METADATA_KEY.to_owned());
    let metadata = map
        .entry(metadata_key)
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if !metadata.is_mapping() {
        // A non-map `metadata:` would be clobbered; replace it with a fresh map
        // rather than silently lose the stamp.
        *metadata = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    if let Some(meta_map) = metadata.as_mapping_mut() {
        meta_map.insert(
            serde_yaml::Value::String(REVISION_KEY.to_owned()),
            serde_yaml::Value::String(revision.to_owned()),
        );
    }
    let Ok(yaml_out) = serde_yaml::to_string(&map) else {
        return skill_md.to_vec();
    };
    let mut out = String::with_capacity(yaml_out.len() + body.len() + 8);
    out.push_str("---\n");
    out.push_str(&yaml_out);
    if !yaml_out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    // Body is preserved verbatim (its leading newline is the intentional blank
    // line after the frontmatter); only the frontmatter block was rewritten.
    out.push_str(body);
    out.into_bytes()
}

/// Read `metadata.tome_skill_revision` out of a SKILL.md's frontmatter, leniently
/// (any parse failure → `None`).
fn extract_revision(content: &str) -> Option<String> {
    let (yaml, _) = split_frontmatter(content)?;
    let map: serde_yaml::Mapping = serde_yaml::from_str(yaml).ok()?;
    map.get(serde_yaml::Value::String(METADATA_KEY.to_owned()))?
        .as_mapping()?
        .get(serde_yaml::Value::String(REVISION_KEY.to_owned()))?
        .as_str()
        .map(str::to_owned)
}

/// Split leading `---`-delimited YAML frontmatter from the body. Returns the
/// raw `(yaml, body)` slices, or `None` if the opening/closing `---` is absent.
/// A small local copy of the discipline in `plugin::frontmatter` (whose splitter
/// is private); the delimiter must be a line that is exactly `---`.
fn split_frontmatter(contents: &str) -> Option<(&str, &str)> {
    let after_open = strip_delim_line(contents)?;
    let close = find_close(after_open)?;
    Some((&after_open[..close.0], &after_open[close.1..]))
}

fn strip_delim_line(s: &str) -> Option<&str> {
    let (first, rest) = match s.find('\n') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    is_delim(first).then_some(rest)
}

/// Returns `(start, end)` byte offsets of the closing delimiter line within `s`.
fn find_close(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut line_start = 0;
    while line_start <= bytes.len() {
        let nl = bytes[line_start..].iter().position(|b| *b == b'\n');
        let line_end = nl.map_or(bytes.len(), |off| line_start + off);
        if is_delim(&s[line_start..line_end]) {
            let end = if nl.is_some() { line_end + 1 } else { line_end };
            return Some((line_start, end));
        }
        match nl {
            Some(_) => line_start = line_end + 1,
            None => break,
        }
    }
    None
}

fn is_delim(line: &str) -> bool {
    line.trim_end_matches(['\r', ' ', '\t']) == "---"
}

// --- error helpers -----------------------------------------------------------

/// Build a [`TomeError::MetaSkillNotFound`] (exit 87) whose message enumerates
/// the bundled ids (FR-033). Shared by the compute path and the CLI.
pub fn not_found(skill_id: &str) -> TomeError {
    TomeError::MetaSkillNotFound {
        id: skill_id.to_owned(),
        available: all().iter().map(|s| s.id).collect::<Vec<_>>().join(", "),
    }
}

/// Build a [`TomeError::MetaInstallFailed`] (exit 88) for a write-path failure.
fn install_failed(skill_id: &str, dir: &Path, source: std::io::Error) -> TomeError {
    TomeError::MetaInstallFailed {
        skill_id: skill_id.to_owned(),
        dir: dir.to_owned(),
        source,
    }
}

/// Coerce a landing `TomeError` into the `io::Error` that `MetaInstallFailed`
/// carries. `land_directory_with_replace` surfaces failures (including its own
/// symlink refusal) as `TomeError::Io`; anything else is wrapped by message.
fn into_io(err: TomeError) -> std::io::Error {
    match err {
        TomeError::Io(e) => e,
        other => std::io::Error::other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::lint::{Verdict, parse::parse_artifact, rules, run};
    use std::path::Path;

    /// Materialise an embedded skill's raw (unstamped) bytes into
    /// `<base>/<id>/…` so it can be parsed as a native artifact.
    fn materialise(skill: &EmbeddedMetaSkill, base: &Path) -> std::path::PathBuf {
        let root = base.join(skill.id);
        for f in skill.files {
            let dest = root.join(f.rel_path);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(&dest, f.bytes).unwrap();
        }
        root
    }

    // --- registry ---------------------------------------------------------

    #[test]
    fn registry_contains_convert_marketplace() {
        let skill = find("convert-marketplace").expect("convert-marketplace embedded");
        assert_eq!(skill.id, "convert-marketplace");
        assert!(!skill.summary.trim().is_empty(), "summary populated");
        assert_eq!(skill.revision.len(), 16, "sha256-short = 16 hex chars");
        assert!(skill.revision.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(
            skill.files.iter().any(|f| f.rel_path == "SKILL.md"),
            "has a root SKILL.md"
        );
        // The unused-until-US3 field is read here so the public API stays live.
        let _ = skill.prompt_name;
    }

    #[test]
    fn find_unknown_is_none() {
        assert!(find("does-not-exist").is_none());
    }

    /// US2 / SC-002: the `convert-marketplace` skill body encodes the five-step
    /// workflow IN ORDER and carries the **unconditional** report-and-confirm
    /// gate. A structural pin so an edit cannot silently drop the safety gate or
    /// re-order the flow.
    #[test]
    fn convert_marketplace_encodes_ordered_workflow() {
        let skill = find("convert-marketplace").expect("embedded");
        let body = std::str::from_utf8(
            skill
                .files
                .iter()
                .find(|f| f.rel_path == "SKILL.md")
                .unwrap()
                .bytes,
        )
        .unwrap();

        // The ordered workflow markers appear, in order.
        let ordered = [
            "## Step 1 — Inventory",
            "## Step 2 — Mechanical conversion",
            "## Step 3 — Judgment pass",
            "## Step 4 — Verify",
            "## Step 5 — Report, then STOP",
            "## Step 6 — Confirmed registration",
        ];
        let mut last = 0usize;
        for marker in ordered {
            let at = body
                .find(marker)
                .unwrap_or_else(|| panic!("missing workflow step `{marker}`"));
            assert!(at >= last, "step out of order: `{marker}`");
            last = at;
        }

        // It DRIVES the Phase-8 CLI, not reimplements it (FR-024). Pin the
        // CORRECT verbs (`catalog convert` / `catalog lint` — `lint` is a
        // subcommand, not a top-level verb) so a regression to a non-existent
        // command can't ship.
        assert!(body.contains("tome catalog convert"));
        assert!(
            body.contains("tome catalog lint"),
            "Step 4 uses the real `tome catalog lint` (not the non-existent `tome lint`)"
        );

        // The report-and-confirm gate is UNCONDITIONAL (SC-002): it fires even
        // with zero unsupported components, registers nothing until confirmed.
        assert!(
            body.contains("**unconditional**"),
            "gate marked unconditional"
        );
        assert!(
            body.to_lowercase().contains("zero unsupported components"),
            "gate explicitly fires on a clean conversion"
        );
        assert!(
            body.contains("Register nothing yet") || body.contains("register nothing"),
            "registers nothing before confirmation"
        );
        // T-5: pin the conjoined wait-for-confirmation phrase + the fail-closed
        // decline branch, not two scattered tokens.
        assert!(
            body.contains("wait for an explicit answer"),
            "waits for an explicit confirmation answer"
        );
        assert!(
            body.to_lowercase().contains("nothing is registered")
                && body.to_lowercase().contains("do not proceed"),
            "decline branch is fail-closed (nothing registered, do not proceed)"
        );

        // T-3 / SC-002 security boundary: every workspace-MUTATING command lives
        // AFTER the report-and-confirm gate (Step 6) — none leaks into Steps 1–5.
        let step6 = body.find("## Step 6").expect("Step 6 present");
        for cmd in [
            "tome catalog add",
            "tome plugin enable",
            "tome workspace init",
            "tome workspace use",
        ] {
            if let Some(at) = body.find(cmd) {
                assert!(
                    at > step6,
                    "registration command `{cmd}` appears before the confirm gate (offset {at} < Step 6 {step6})"
                );
            }
        }

        // T-2: the rubric the body links to must actually ship (no dangling link).
        assert!(
            body.contains("references/unsupported-component-rubric.md"),
            "Step 3 links the rubric"
        );
        assert!(
            skill
                .files
                .iter()
                .any(|f| f.rel_path == "references/unsupported-component-rubric.md"),
            "the linked rubric file is embedded (no dangling link)"
        );

        // This skill declares its reserved MCP prompt (consumed by US3).
        assert_eq!(skill.prompt_name, Some("add-tome-conversion-skill"));
    }

    /// FR-005 / R-12: every embedded skill is lint-clean by construction —
    /// parsed to native IR and run through the full rule registry; fail on any
    /// error OR strict warning. This is the CI gate.
    #[test]
    fn every_embedded_skill_is_lint_clean() {
        let tmp = tempfile::tempdir().unwrap();
        for skill in all() {
            // T-1: `parse_artifact` lints only the root `SKILL.md` (a bare-skill
            // folder has no `skills/` subtree), so `references/` files are NOT
            // run through the rule registry. Guard the shipped supporting files
            // directly: every embedded file must be valid UTF-8 and under the
            // body cap, so the install-time copy + any harness re-read is safe.
            for f in skill.files {
                assert!(
                    std::str::from_utf8(f.bytes).is_ok(),
                    "embedded file `{}` in `{}` is not valid UTF-8",
                    f.rel_path,
                    skill.id
                );
                assert!(
                    f.bytes.len() as u64 <= ENTRY_BODY_MAX,
                    "embedded file `{}` in `{}` exceeds the {ENTRY_BODY_MAX}-byte body cap",
                    f.rel_path,
                    skill.id
                );
            }

            let root = materialise(skill, tmp.path());
            let artifact = parse_artifact(&root)
                .unwrap_or_else(|e| panic!("embedded skill `{}` failed to parse: {e}", skill.id));
            let report = run(&artifact, &rules::all());
            assert_eq!(
                report.verdict(true),
                Verdict::Clean,
                "embedded skill `{}` is not lint-clean: {} error(s), {} warning(s): {:?}",
                skill.id,
                report.errors,
                report.warnings,
                report.diagnostics,
            );
        }
    }

    // --- install / remove -------------------------------------------------

    #[test]
    fn install_lands_stamped_skill_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".claude/skills");
        std::fs::create_dir_all(&root).unwrap();

        let at = install_skill("convert-marketplace", &root).expect("install");
        let skill_md = root.join("convert-marketplace/SKILL.md");
        assert!(skill_md.is_file());
        assert_eq!(at.revision, find("convert-marketplace").unwrap().revision);

        // The landed SKILL.md carries the stamped revision.
        let content = std::fs::read_to_string(&skill_md).unwrap();
        assert_eq!(
            extract_revision(&content).as_deref(),
            Some(at.revision.as_str())
        );
        // Name/description survive the stamp round-trip.
        assert!(content.contains("name: convert-marketplace"));

        // Idempotent: a second install replaces in place, same revision.
        let at2 = install_skill("convert-marketplace", &root).expect("re-install");
        assert_eq!(at2.revision, at.revision);
        assert!(skill_md.is_file());
    }

    /// FIX H (Test Minor #4): the on-disk SKILL.md that `install_skill` LANDS —
    /// after the `stamp_revision` serde_yaml frontmatter round-trip — still
    /// parses as a valid native skill and stays lint-clean. Guards against the
    /// stamp corrupting the frontmatter in a way the embedded-source lint gate
    /// (`every_embedded_skill_is_lint_clean`, which lints the UNSTAMPED bytes)
    /// would never see. Reuses the same parse + rule registry that gate uses.
    #[test]
    fn installed_stamped_skill_md_stays_lint_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".claude/skills");
        std::fs::create_dir_all(&root).unwrap();

        let at = install_skill("convert-marketplace", &root).expect("install");
        // Lint the LANDED skill folder (the stamped on-disk bytes), not the
        // embedded source — through the same parse + `rules::all()` as the CI gate.
        let landed = at.skill_dir;
        let artifact = parse_artifact(&landed)
            .unwrap_or_else(|e| panic!("landed stamped SKILL.md failed to parse: {e}"));
        let report = run(&artifact, &rules::all());
        assert_eq!(
            report.verdict(true),
            Verdict::Clean,
            "the stamped on-disk skill is not lint-clean: {} error(s), {} warning(s): {:?}",
            report.errors,
            report.warnings,
            report.diagnostics,
        );
        // And the stamp the lint just saw is the embedded revision.
        let on_disk = std::fs::read_to_string(landed.join("SKILL.md")).unwrap();
        assert_eq!(
            extract_revision(&on_disk).as_deref(),
            Some(at.revision.as_str())
        );
    }

    #[test]
    fn install_unknown_skill_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = install_skill("nope", tmp.path()).expect_err("unknown");
        assert!(matches!(err, TomeError::MetaSkillNotFound { .. }));
        assert_eq!(err.exit_code(), 87);
    }

    #[test]
    fn remove_deletes_then_is_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".cursor/skills");
        std::fs::create_dir_all(&root).unwrap();
        install_skill("convert-marketplace", &root).unwrap();

        assert_eq!(
            remove_skill("convert-marketplace", &root).unwrap(),
            RemoveOutcome::Removed
        );
        assert!(!root.join("convert-marketplace").exists());
        assert_eq!(
            remove_skill("convert-marketplace", &root).unwrap(),
            RemoveOutcome::NotPresent
        );
    }

    #[test]
    fn remove_unknown_skill_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = remove_skill("nope", tmp.path()).expect_err("unknown");
        assert_eq!(err.exit_code(), 87);
    }

    // --- drift ------------------------------------------------------------

    #[test]
    fn drift_up_to_date_after_install() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".codex/skills");
        std::fs::create_dir_all(&root).unwrap();
        install_skill("convert-marketplace", &root).unwrap();
        assert_eq!(
            drift_probe("convert-marketplace", &root),
            DriftState::UpToDate
        );
    }

    #[test]
    fn drift_missing_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            drift_probe("convert-marketplace", tmp.path()),
            DriftState::MissingButExpected
        );
    }

    #[test]
    fn drift_stale_on_revision_mismatch_and_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("convert-marketplace");
        std::fs::create_dir_all(&dir).unwrap();

        // Wrong stamped revision → stale.
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: convert-marketplace\ndescription: d\nmetadata:\n  tome_skill_revision: deadbeefdeadbeef\n---\nbody\n",
        )
        .unwrap();
        assert!(matches!(
            drift_probe("convert-marketplace", tmp.path()),
            DriftState::Stale {
                installed_rev: Some(_),
                ..
            }
        ));

        // No marker at all → stale (refreshable).
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: convert-marketplace\ndescription: d\n---\nbody\n",
        )
        .unwrap();
        assert!(matches!(
            drift_probe("convert-marketplace", tmp.path()),
            DriftState::Stale {
                installed_rev: None,
                ..
            }
        ));
    }

    /// FIX C (P9 LOW-1): a symlinked component on the way to `<dir>/<id>/SKILL.md`
    /// is NOT read through — `drift_probe` classifies it refreshable `Stale`
    /// (read/write containment parity), never disclosing the link target.
    #[cfg(unix)]
    #[test]
    fn drift_probe_refuses_symlinked_component_as_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let skills = base.join("skills");
        std::fs::create_dir_all(&skills).unwrap();

        // Plant a REAL skill outside the skills root, with a DIFFERENT revision
        // stamp, then point `<skills>/<id>` at it via a symlink. If the probe
        // followed the link it would read that file (and could disclose its
        // content / a non-`None` installed_rev); the guard must refuse first.
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("SKILL.md"),
            "---\nname: convert-marketplace\ndescription: d\nmetadata:\n  tome_skill_revision: deadbeefdeadbeef\n---\nbody\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, skills.join("convert-marketplace")).unwrap();

        // The link IS resolvable at the OS level …
        assert!(skills.join("convert-marketplace/SKILL.md").exists());
        // … but the probe refuses to read through the symlinked component and
        // reports refreshable Stale with NO installed_rev (it never read the
        // out-of-tree stamp).
        assert_eq!(
            drift_probe("convert-marketplace", &skills),
            DriftState::Stale {
                installed_rev: None,
                embedded_rev: find("convert-marketplace").unwrap().revision.to_owned(),
            },
        );
    }

    // --- symlink safety (88, no escape) -----------------------------------

    #[cfg(unix)]
    #[test]
    fn install_refuses_symlinked_target_component() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        // `skills` is a symlink to `real` — a symlinked component of the target.
        std::os::unix::fs::symlink(&real, base.join("skills")).unwrap();

        let err = install_skill("convert-marketplace", &base.join("skills"))
            .expect_err("symlinked component must be refused");
        assert!(matches!(err, TomeError::MetaInstallFailed { .. }));
        assert_eq!(err.exit_code(), 88);
        // No skill folder landed through the symlink.
        assert!(!real.join("convert-marketplace").exists());
    }

    // --- stamp round-trip -------------------------------------------------

    #[test]
    fn stamp_then_extract_round_trips() {
        let bytes = b"---\nname: x\ndescription: hello\n---\n# body\n";
        let stamped = stamp_revision(bytes, "abc123");
        let s = String::from_utf8(stamped).unwrap();
        assert_eq!(extract_revision(&s).as_deref(), Some("abc123"));
        assert!(s.contains("description: hello"));
        assert!(s.contains("# body"));
    }
}
