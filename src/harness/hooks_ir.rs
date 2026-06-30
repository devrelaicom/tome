//! The plugin-hook translation IR (the `CanonicalHook` analogue of
//! `CanonicalAgent`), the parse from a plugin's `hooks/hooks.json`, and the
//! resolved per-(workspace, harness) dispatch manifest the runtime dispatcher
//! reads. Sync only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::harness::hooks::RewrittenHooks;

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

/// Why a hook entry was not promoted to the typed IR (dropped to GUARDRAILS).
// Future USs wire this into the sync orchestrator; defined here to complete the
// IR vocabulary. Allowed dead until the orchestrator call sites land (US2+).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookDropReason {
    NonPortableEvent,
    UnsupportedHandler,
    /// Gate applied by the caller (US6); defined here for the full drop vocabulary.
    PromptDisabled,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookDrop {
    pub(crate) reason: HookDropReason,
    pub(crate) event: String,
}

/// Promote the CC-verbatim `RewrittenHooks` into the typed IR. Non-portable
/// events and non-{command,http,prompt} handlers are dropped into `drops`
/// (the plugin keeps its `GUARDRAILS.md` floor for them). The `prompt` gate
/// (US6) is applied by the caller, not here.
#[allow(dead_code)]
pub(crate) fn parse_canonical_hooks(
    catalog: &str,
    plugin: &str,
    rewritten: &RewrittenHooks,
    drops: &mut Vec<HookDrop>,
) -> Vec<CanonicalHook> {
    let mut out = Vec::new();
    for (event_name, groups) in &rewritten.events {
        let Some(event) = PortableEvent::from_cc_name(event_name) else {
            drops.push(HookDrop {
                reason: HookDropReason::NonPortableEvent,
                event: event_name.clone(),
            });
            continue;
        };
        for group in groups {
            let matcher = group
                .get("matcher")
                .and_then(|m| m.as_str())
                .map(str::to_owned);
            let if_pred = group.get("if").and_then(|m| m.as_str()).map(str::to_owned);
            let Some(handlers) = group.get("hooks").and_then(|h| h.as_array()) else {
                continue;
            };
            for h in handlers {
                let timeout_secs = h.get("timeout").and_then(|t| t.as_u64());
                let kind = h.get("type").and_then(|t| t.as_str()).unwrap_or("command");
                let handler = match kind {
                    "command" => match h.get("command").and_then(|c| c.as_str()) {
                        Some(cmd) => Handler::Command {
                            command: cmd.to_owned(),
                        },
                        None => {
                            drops.push(HookDrop {
                                reason: HookDropReason::UnsupportedHandler,
                                event: event_name.clone(),
                            });
                            continue;
                        }
                    },
                    "http" => match h.get("url").and_then(|u| u.as_str()) {
                        Some(url) => Handler::Http {
                            url: url.to_owned(),
                            headers: h
                                .get("headers")
                                .and_then(json_string_map)
                                .unwrap_or_default(),
                            allowed_env_vars: h
                                .get("allowedEnvVars")
                                .and_then(json_string_vec)
                                .unwrap_or_default(),
                        },
                        None => {
                            drops.push(HookDrop {
                                reason: HookDropReason::UnsupportedHandler,
                                event: event_name.clone(),
                            });
                            continue;
                        }
                    },
                    "prompt" => match h.get("prompt").and_then(|p| p.as_str()) {
                        Some(p) => Handler::Prompt {
                            prompt: p.to_owned(),
                        },
                        None => {
                            drops.push(HookDrop {
                                reason: HookDropReason::UnsupportedHandler,
                                event: event_name.clone(),
                            });
                            continue;
                        }
                    },
                    _ => {
                        drops.push(HookDrop {
                            reason: HookDropReason::UnsupportedHandler,
                            event: event_name.clone(),
                        });
                        continue;
                    }
                };
                out.push(CanonicalHook {
                    catalog: catalog.to_owned(),
                    plugin: plugin.to_owned(),
                    event,
                    matcher: matcher.clone(),
                    if_pred: if_pred.clone(),
                    handler,
                    timeout_secs,
                });
            }
        }
    }
    out
}

#[allow(dead_code)]
fn json_string_map(v: &serde_json::Value) -> Option<BTreeMap<String, String>> {
    let obj = v.as_object()?;
    Some(
        obj.iter()
            .filter_map(|(k, val)| Some((k.clone(), val.as_str()?.to_owned())))
            .collect(),
    )
}

#[allow(dead_code)]
fn json_string_vec(v: &serde_json::Value) -> Option<Vec<String>> {
    Some(
        v.as_array()?
            .iter()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::hooks::RewrittenHooks;

    #[test]
    fn portable_event_cc_name_roundtrips() {
        for ev in PortableEvent::ALL {
            assert_eq!(PortableEvent::from_cc_name(ev.cc_name()), Some(ev));
        }
        assert_eq!(PortableEvent::from_cc_name("Notification"), None);
        assert_eq!(PortableEvent::from_cc_name("SubagentStop"), None);
        assert_eq!(PortableEvent::PreToolUse.cc_name(), "PreToolUse");
    }

    fn rewritten(event: &str, entries: serde_json::Value) -> RewrittenHooks {
        RewrittenHooks {
            events: vec![(event.to_owned(), entries.as_array().unwrap().clone())],
        }
    }

    #[test]
    fn parse_extracts_command_http_prompt_and_records_drops() {
        // A PreToolUse command hook with a matcher + timeout.
        let rw = rewritten(
            "PreToolUse",
            serde_json::json!([
                { "matcher": "Bash", "hooks": [
                    { "type": "command", "command": "/p/guard.sh", "timeout": 30 }
                ]}
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].event, PortableEvent::PreToolUse);
        assert_eq!(hooks[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(hooks[0].timeout_secs, Some(30));
        assert_eq!(
            hooks[0].handler,
            Handler::Command {
                command: "/p/guard.sh".into()
            }
        );
        assert!(drops.is_empty());

        // A non-portable event → dropped.
        let rw = rewritten(
            "Notification",
            serde_json::json!([
                { "hooks": [ { "type": "command", "command": "x" } ] }
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert!(hooks.is_empty());
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].reason, HookDropReason::NonPortableEvent);

        // An mcp_tool handler → dropped (unsupported kind).
        let rw = rewritten(
            "PreToolUse",
            serde_json::json!([
                { "hooks": [ { "type": "mcp_tool", "tool": "x" } ] }
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert!(hooks.is_empty());
        assert_eq!(drops[0].reason, HookDropReason::UnsupportedHandler);
    }
}
