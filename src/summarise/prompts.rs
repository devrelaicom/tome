//! Prompt templates and length-window constants for the workspace
//! summariser.
//!
//! The summariser runs two sequential inference passes:
//!
//! 1. **Short pass** — compress every enabled skill's description into a
//!    single comma-separated list of TOPICS + example TASKS (≤ 800
//!    chars). The output feeds the MCP server's `search_skills` tool
//!    description, which wraps it in an imperative routing template
//!    (FR-425).
//! 2. **Long pass** — using the short output as input, write a 4–6
//!    sentence rules section for the harness's `RULES.md` file
//!    (≤ 2500 chars). The output feeds the workspace's composed
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
///
/// The instruction asks for TOPICS + example TASKS (not a prose paragraph)
/// because the sole consumer of `short` is the MCP `search_skills` tool
/// description, which wraps it in an imperative routing template
/// ("Before working on tasks related to {short}, … call this tool …").
/// That template reads naturally only when `{short}` is a comma-separated
/// list of topics and example tasks, so the model is steered toward that
/// shape here rather than at the wrapping site.
pub const SHORT_PROMPT: &str =
    "You are summarising a developer's skill library. Given the descriptions below,
write a single line naming the concrete TOPICS this workspace's skills cover and
2-3 example TASKS a user might ask for, as a comma-separated list. No preamble,
no sentences — just the comma-separated topics and example tasks. Example:
\"database migrations, release notes drafting, OAuth login flows, writing a deploy script\".

Skill descriptions:
{descriptions}";

/// Second-pass prompt template. Use [`long_prompt`] to build the actual
/// prompt string with the configured character cap substituted in.
/// `{topics}` is substituted with the output of the short pass; `{max_chars}`
/// is substituted with the effective `long_max_chars` value.
const LONG_PROMPT_TEMPLATE: &str =
    "You are writing a short rules section for an AI coding agent. The agent has access
to a search tool that retrieves skills relevant to a task. Below are the topics the
user's skill library covers. Write a 4–6 sentence rules section that
(1) tells the agent which topics the skill library covers,
(2) instructs the agent to call the search_skills tool when working on tasks
   involving those topics,
(3) is written for the agent to read at session start.
Plain prose, no headings, no bullet points. Maximum {max_chars} characters.

Topics:
{topics}";

/// Build the second-pass prompt with `max_chars` substituted for the
/// character-limit instruction. Callers pass `effective_long_max` (resolved
/// from `config.summariser.long_max_chars.unwrap_or(LONG_MAX_CHARS)`) so
/// the model targets the user-configured budget rather than the hardcoded
/// 2500 default.
///
/// The `{topics}` placeholder is left in the returned string for the
/// caller to substitute with the short-pass output via `str::replace`.
pub fn long_prompt(max_chars: usize) -> String {
    LONG_PROMPT_TEMPLATE.replace("{max_chars}", &max_chars.to_string())
}

// The hard upper bounds [`SHORT_MAX_CHARS`] and [`LONG_MAX_CHARS`] live
// in [`crate::summarise`] as the single source of truth across the
// inference loop (`llama.rs`) and the regen-summary command
// (`workspace::regen_summary`). US4.d-1 consolidation. Re-exported here
// for backwards-compatibility with the existing `prompts::*` import
// paths.
pub use super::{LONG_MAX_CHARS, SHORT_MAX_CHARS};

/// Target lower bound for the short summary (advisory, no warning emitted).
pub const SHORT_TARGET_MIN: usize = 400;

/// Target upper bound for the short summary (advisory, no warning emitted).
pub const SHORT_TARGET_MAX: usize = 800;

/// Target lower bound for the long summary (advisory, no warning emitted).
pub const LONG_TARGET_MIN: usize = 1500;

/// Target upper bound for the long summary (advisory, no warning emitted).
pub const LONG_TARGET_MAX: usize = 2500;

// Compile-time guards on the length-window constants that DON'T reference
// the runtime `LONG_MAX_CHARS` (which can now be overridden per config).
// Guards for LONG_TARGET_MIN < LONG_MAX_CHARS are checked at runtime via
// `effective_long_max` validation in `regen_summary.rs`.
const _: () = {
    assert!(SHORT_TARGET_MIN < SHORT_TARGET_MAX);
    assert!(SHORT_TARGET_MAX <= SHORT_MAX_CHARS);
    assert!(LONG_TARGET_MIN < LONG_TARGET_MAX);
    // After US4.d-1's consolidation `LONG_MAX_CHARS == LONG_TARGET_MAX`
    // — the long target band aligns with the hard cap rather than
    // extending past it. The assert below stays valid (strict <) and
    // validates the DEFAULT constant; runtime values are validated
    // separately via `validate_long_max_chars`.
    assert!(LONG_TARGET_MIN < LONG_MAX_CHARS);
};

/// Validate (and optionally clamp) a user-configured `long_max_chars` value.
///
/// Returns the effective cap to use. If the supplied value is 0 or below
/// `LONG_TARGET_MIN` (1500), it is clamped to `LONG_MAX_CHARS` (the default)
/// because a value that small produces a degenerate prompt — either the
/// character limit is smaller than the minimum target window, or it is zero
/// which makes no sense at all. A `tracing::warn!` is emitted so the user
/// can correct their configuration.
///
/// Values equal to or above `LONG_TARGET_MIN` are accepted unchanged.
pub fn validate_long_max_chars(configured: usize) -> usize {
    if configured < LONG_TARGET_MIN {
        tracing::warn!(
            configured,
            min = LONG_TARGET_MIN,
            default = LONG_MAX_CHARS,
            "config.summariser.long_max_chars is below minimum ({}); \
             clamping to default {}",
            LONG_TARGET_MIN,
            LONG_MAX_CHARS,
        );
        LONG_MAX_CHARS
    } else {
        configured
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_prompt_includes_substitution_marker() {
        assert!(SHORT_PROMPT.contains("{descriptions}"));
    }

    #[test]
    fn long_prompt_includes_topics_substitution_marker() {
        assert!(long_prompt(LONG_MAX_CHARS).contains("{topics}"));
    }

    #[test]
    fn long_prompt_uses_configured_cap() {
        // TDD: the long prompt builder with a custom cap embeds that cap,
        // not the hardcoded 2500 default.
        let prompt = long_prompt(4000);
        assert!(
            prompt.contains("4000"),
            "long_prompt(4000) should embed '4000' in the instruction, got:\n{prompt}",
        );
        assert!(
            !prompt.contains("2500"),
            "long_prompt(4000) must NOT embed the default '2500', got:\n{prompt}",
        );
    }

    #[test]
    fn long_prompt_default_cap_embeds_2500() {
        let prompt = long_prompt(LONG_MAX_CHARS);
        assert!(
            prompt.contains("2500"),
            "long_prompt(LONG_MAX_CHARS=2500) should embed '2500', got:\n{prompt}",
        );
    }

    #[test]
    fn validate_long_max_chars_accepts_at_or_above_min() {
        // LONG_TARGET_MIN = 1500: anything >= 1500 is returned unchanged.
        assert_eq!(validate_long_max_chars(LONG_TARGET_MIN), LONG_TARGET_MIN);
        assert_eq!(validate_long_max_chars(2500), 2500);
        assert_eq!(validate_long_max_chars(4000), 4000);
    }

    #[test]
    fn validate_long_max_chars_clamps_below_min_to_default() {
        // 0 and values below LONG_TARGET_MIN (1500) clamp to LONG_MAX_CHARS (2500).
        assert_eq!(validate_long_max_chars(0), LONG_MAX_CHARS);
        assert_eq!(validate_long_max_chars(100), LONG_MAX_CHARS);
        assert_eq!(validate_long_max_chars(LONG_TARGET_MIN - 1), LONG_MAX_CHARS);
    }
}
