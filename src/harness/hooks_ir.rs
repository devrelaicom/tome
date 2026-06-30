//! The plugin-hook translation IR (the `CanonicalHook` analogue of
//! `CanonicalAgent`), the parse from a plugin's `hooks/hooks.json`, and the
//! resolved per-(workspace, harness) dispatch manifest the runtime dispatcher
//! reads. Sync only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The portable-core hook events Tome translates across harnesses. Every other
/// CC event (Notification, SubagentStop, Setup, PermissionRequest, …) has no
/// cross-harness target and falls back to `GUARDRAILS.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortableEvent {
    PreToolUse,
    PostToolUse,
    UserPromptSubmit,
    Stop,
    SessionStart,
    SessionEnd,
    PreCompact,
}

impl PortableEvent {
    /// All seven, for iteration in tests + the used-event computation.
    pub const ALL: [PortableEvent; 7] = [
        PortableEvent::PreToolUse,
        PortableEvent::PostToolUse,
        PortableEvent::UserPromptSubmit,
        PortableEvent::Stop,
        PortableEvent::SessionStart,
        PortableEvent::SessionEnd,
        PortableEvent::PreCompact,
    ];

    /// The Claude Code event name — the IR's canonical vocabulary and the
    /// manifest event-map key.
    pub fn cc_name(self) -> &'static str {
        match self {
            PortableEvent::PreToolUse => "PreToolUse",
            PortableEvent::PostToolUse => "PostToolUse",
            PortableEvent::UserPromptSubmit => "UserPromptSubmit",
            PortableEvent::Stop => "Stop",
            PortableEvent::SessionStart => "SessionStart",
            PortableEvent::SessionEnd => "SessionEnd",
            PortableEvent::PreCompact => "PreCompact",
        }
    }

    /// Parse a CC event name; `None` for any non-portable event (→ GUARDRAILS).
    pub fn from_cc_name(s: &str) -> Option<Self> {
        PortableEvent::ALL.into_iter().find(|e| e.cc_name() == s)
    }
}

/// A single runnable hook handler. The three handler kinds Tome can execute.
/// CC's `mcp_tool`/`agent` handler kinds are NOT representable here and are
/// dropped at parse time (→ GUARDRAILS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Handler {
    /// Relocated verbatim (only the 2 path tokens rewritten by the parser).
    Command { command: String },
    /// Tome POSTs the CC JSON; allowlisted env interpolated into header values.
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_env_vars: Vec<String>,
    },
    /// Single-turn LLM eval (config-gated). The prompt text, relocated verbatim.
    Prompt { prompt: String },
}

/// One runnable hook parsed from an enabled plugin's `hooks/hooks.json`.
/// The IR — the `CanonicalAgent` analogue for hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalHook {
    pub catalog: String,
    pub plugin: String,
    pub event: PortableEvent,
    /// The CC matcher VERBATIM (`None`/`""`/`"*"` = all). Applied by the
    /// dispatcher under CC matcher semantics, in CC tool vocabulary.
    pub matcher: Option<String>,
    /// CC's `if` permission-rule predicate (additive; evaluated over tool_input).
    pub if_pred: Option<String>,
    pub handler: Handler,
    /// CC seconds. Harness-specific unit conversion happens at manifest write.
    pub timeout_secs: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_event_cc_name_roundtrips() {
        for ev in PortableEvent::ALL {
            assert_eq!(PortableEvent::from_cc_name(ev.cc_name()), Some(ev));
        }
        assert_eq!(PortableEvent::from_cc_name("Notification"), None);
        assert_eq!(PortableEvent::from_cc_name("SubagentStop"), None);
        assert_eq!(PortableEvent::PreToolUse.cc_name(), "PreToolUse");
    }
}
