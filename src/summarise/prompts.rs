//! Prompt templates and length-window constants for the workspace
//! summariser.
//!
//! The summariser runs two sequential inference passes:
//!
//! 1. **Short pass** — compress every enabled skill's description into a
//!    single comma-separated topic list (≤ 800 chars). The output feeds
//!    the MCP server's tool description (FR-425).
//! 2. **Long pass** — using the short output as input, write a 4–6
//!    sentence rules section for the harness's `RULES.md` file
//!    (≤ 2400 chars). The output feeds the workspace's composed
//!    `RULES.md`.
//!
//! Both prompts are pinned `&'static str` constants — research §R-15
//! treats prompt wording as code, not configuration, so a wording change
//! ships in a Tome release rather than as a runtime tweakable.
//!
//! The length windows below are advisory: outputs at or below `MAX`
//! pass; outputs strictly above `MAX` emit a `tracing::warn!` and
//! continue (per `contracts/summariser.md` §"Length windows" — a
//! too-long short summary that's been embedded into the MCP tool
//! description is a warning, not a hard error). Empty outputs are a
//! hard failure (`SummariserFailureKind::OutputEmpty`, exit 24).
//!
//! Rust 1.93's stable `const` evaluator does not support construction of
//! `RangeInclusive<usize>` in a `const` initialiser, so the target
//! range is exposed as two `MIN` / `MAX` constants and the call site
//! composes the range as needed (e.g. `SHORT_TARGET_MIN..=SHORT_TARGET_MAX`).
//! Contract: `contracts/summariser.md` §"Prompts" + §"Length windows".

/// First-pass prompt. `{descriptions}` is substituted via `String::replace`
/// with one line per skill of the form `<plugin>: <skill-name> — <skill-description>`.
pub const SHORT_PROMPT: &str =
    "You are summarising a developer's skill library. Given the descriptions below,
produce a single comma-separated phrase listing the topics these skills cover.
No prose, no lead-in, no bullet points. Maximum 700 characters.

Skill descriptions:
{descriptions}";

/// Second-pass prompt. `{topics}` is substituted with the output of the
/// short pass (cascading from short to long; the long prompt benefits
/// from the short summary's already-compressed topic list).
pub const LONG_PROMPT: &str =
    "You are writing a short rules section for an AI coding agent. The agent has access
to a search tool that retrieves skills relevant to a task. Below are the topics the
user's skill library covers. Write a 4–6 sentence rules section that
(1) tells the agent which topics the skill library covers,
(2) instructs the agent to call the search_skills tool when working on tasks
   involving those topics,
(3) is written for the agent to read at session start.
Plain prose, no headings, no bullet points. Maximum 2400 characters.

Topics:
{topics}";

/// Hard upper bound for the short summary. Outputs strictly above this
/// emit a tracing warning; outputs at or below are cached without
/// comment. Per `contracts/summariser.md` §"Length windows" the short
/// summary is also subject to a soft target window of 400–800 chars.
pub const SHORT_MAX_CHARS: usize = 800;

/// Target lower bound for the short summary (advisory, no warning emitted).
pub const SHORT_TARGET_MIN: usize = 400;

/// Target upper bound for the short summary (advisory, no warning emitted).
pub const SHORT_TARGET_MAX: usize = 800;

/// Hard upper bound for the long summary. Outputs strictly above this
/// emit a tracing warning. Per the contract the long summary's target
/// window is 1500–2500 chars, slightly wider than the hard cap so a
/// model that slightly overshoots target still passes the strict gate.
pub const LONG_MAX_CHARS: usize = 2400;

/// Target lower bound for the long summary (advisory, no warning emitted).
pub const LONG_TARGET_MIN: usize = 1500;

/// Target upper bound for the long summary (advisory, no warning emitted).
pub const LONG_TARGET_MAX: usize = 2500;

// Compile-time guards on the length-window constants. A future tweak
// that flips the inequality wakes the build, not the test suite.
const _: () = {
    assert!(SHORT_TARGET_MIN < SHORT_TARGET_MAX);
    assert!(SHORT_TARGET_MAX <= SHORT_MAX_CHARS);
    assert!(LONG_TARGET_MIN < LONG_TARGET_MAX);
    // The long target band intentionally extends past LONG_MAX_CHARS
    // — see the contract's "Length windows" table.
    assert!(LONG_TARGET_MIN < LONG_MAX_CHARS);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_prompt_includes_substitution_marker() {
        assert!(SHORT_PROMPT.contains("{descriptions}"));
    }

    #[test]
    fn long_prompt_includes_substitution_marker() {
        assert!(LONG_PROMPT.contains("{topics}"));
    }
}
