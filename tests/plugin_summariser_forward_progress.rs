//! Phase 4 / F11c-2 — T098l + T098r: FR-385 workspace_skills
//! forward-progress at the summariser boundary.
//!
//! Contract: the `workspace_skills` row INSERT / DELETE that records a
//! per-workspace enable / disable MUST commit in its own transaction
//! BEFORE the summariser is invoked. A summariser failure must NOT roll
//! back the enrolment mutation, and the prior cached summary must
//! survive (no half-state on disk).
//!
//! ## Status: placeholder
//!
//! Phase 4 / US4.b is the slice that wires the summariser invocation
//! into `lifecycle::enable` / `lifecycle::disable`. Until then, the
//! invariant is structurally satisfied — the summariser isn't called,
//! so every workspace_skills mutation trivially commits "before" any
//! post-state-mutation work. There's no production code path here to
//! exercise yet.
//!
//! When US4.b lands, the placeholder below should fill in to:
//!
//!   1. Seed a workspace with an enabled plugin set.
//!   2. Configure the summariser to fail on next invocation.
//!   3. Re-enable / disable a plugin in the workspace.
//!   4. Assert: the command exits with summariser-failure (exit 24).
//!   5. Assert: the `workspace_skills` row INSERT / DELETE committed
//!      (the enrolment state reflects the requested action).
//!   6. Assert: the prior cached summary in
//!      `<root>/workspaces/<name>/settings.toml` is unchanged (no
//!      rollback of cache, no half-written `[summaries]` table).
//!
//! Pair: T098l (forward-progress invariant) + T098r (paired summariser
//! invocation surface) collapse to a single test file because they
//! describe the same observable behaviour from the workspace_skills
//! side. The two tasks share this placeholder; US4.b's wire-in
//! commit should split them into two named tests if the invariant
//! decomposes into separately-observable surfaces.

#[test]
#[ignore = "US4.b: needs summariser invocation wired into enable/disable"]
fn workspace_skills_commits_before_summariser_failure() {
    // Placeholder — body lands in US4.b. See module-level doc-comment
    // for the test shape.
}
