//! The plugin-hook translation IR (the `CanonicalHook` analogue of
//! `CanonicalAgent`), the parse from a plugin's `hooks/hooks.json`, and the
//! resolved per-(workspace, harness) dispatch manifest the runtime dispatcher
//! reads. Sync only.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::TomeError;

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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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

/// The resolved, per-(workspace, harness) dispatch manifest the runtime
/// dispatcher reads. Tome-owned strict input. `events` is keyed by the CC
/// event name; entries are ordered (deterministic merge order).
///
/// `pub` (not `pub(crate)`) because the US4 runtime dispatcher's public
/// `dispatch_core(…, Option<&HookManifest>)` entry point names it; the fields
/// stay `pub(crate)` so only in-crate code mutates the manifest shape, while
/// the consolidated integration-test binary deserializes one from JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookManifest {
    pub(crate) harness: String,
    #[serde(default)]
    pub(crate) raw_event_passthrough: bool,
    pub(crate) events: BTreeMap<String, Vec<ManifestEntry>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestEntry {
    /// `<catalog>:<plugin>` provenance (block-reason prefix + debug).
    pub(crate) plugin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) matcher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub(crate) if_pred: Option<String>,
    pub(crate) handler: Handler,
    /// Harness-converted timeout (Gemini ms; everyone else ms too — baked once).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
}

#[allow(dead_code)]
pub(crate) fn write_manifest(path: &Path, manifest: &HookManifest) -> Result<(), TomeError> {
    // Reuse the hook-file write discipline (symlink refusal + atomic mode-preserving).
    let doc = serde_json::to_value(manifest).map_err(|e| TomeError::HookSettingsWriteFailed {
        path: path.to_path_buf(),
        source: std::io::Error::other(e),
    })?;
    crate::harness::reconcile::hooks::write_hook_file(path, &doc)
}

#[allow(dead_code)]
pub(crate) fn read_manifest(path: &Path) -> Result<HookManifest, TomeError> {
    let text = crate::util::bounded_read_to_string(path, crate::util::TOME_CONFIG_MAX)?;
    serde_json::from_str(&text).map_err(|_| TomeError::HookSpecParseError {
        path: path.to_path_buf(),
    })
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

/// Apply CC matcher semantics: `None`/`""`/`"*"` = all; a token of only
/// `[A-Za-z0-9_|, ]` = exact set (pipe/comma alternation); anything else =
/// unanchored regex. An unparsable regex returns `false` — the hook is
/// skipped (fail-closed for this matcher), not run.
#[allow(dead_code)]
pub(crate) fn matcher_matches(matcher: Option<&str>, cc_tool_name: &str) -> bool {
    let Some(m) = matcher else { return true };
    if m.is_empty() || m == "*" {
        return true;
    }
    let exact_charset = m
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '|' | ',' | ' '));
    if exact_charset {
        return m
            .split(['|', ','])
            .map(str::trim)
            .any(|tok| tok == cc_tool_name);
    }
    Regex::new(m)
        .map(|re| re.is_match(cc_tool_name))
        .unwrap_or(false)
}

/// Map a harness's native tool name to its CC canonical name, enabling a
/// plugin's CC matcher (CC vocabulary) to be tested against incoming harness
/// tool names. Returns `None` for unknown natives — the dispatcher falls back
/// to `unwrap_or(native)` so an unmapped name still matches a CC matcher that
/// references it directly.
#[allow(dead_code)]
pub(crate) fn cc_tool_name(harness: &str, native_tool: &str) -> Option<&'static str> {
    match harness {
        "gemini" => match native_tool {
            "run_shell_command" => Some("Bash"),
            "read_file" => Some("Read"),
            "write_file" => Some("Write"),
            "replace" => Some("Edit"),
            "glob" => Some("Glob"),
            "search_file_content" | "grep" => Some("Grep"),
            "web_fetch" => Some("WebFetch"),
            "google_web_search" => Some("WebSearch"),
            _ => None,
        },
        "devin" => match native_tool {
            "exec" => Some("Bash"),
            "read" => Some("Read"),
            "write" => Some("Write"),
            "edit" => Some("Edit"),
            "grep" => Some("Grep"),
            "glob" => Some("Glob"),
            _ => None,
        },
        "codex" => match native_tool {
            "Bash" => Some("Bash"),
            "Read" => Some("Read"),
            "Write" => Some("Write"),
            "Edit" => Some("Edit"),
            "MultiEdit" => Some("MultiEdit"),
            "NotebookEdit" => Some("NotebookEdit"),
            "Glob" => Some("Glob"),
            "Grep" => Some("Grep"),
            "WebFetch" => Some("WebFetch"),
            "WebSearch" => Some("WebSearch"),
            "Ls" => Some("Ls"),
            _ => None,
        },
        "copilot-cli" => match native_tool {
            "bash" => Some("Bash"),
            "view" => Some("Read"),
            "create" => Some("Write"),
            "edit" => Some("Edit"),
            "grep" | "rg" => Some("Grep"),
            "glob" => Some("Glob"),
            "web_fetch" => Some("WebFetch"),
            "web_search" => Some("WebSearch"),
            _ => None,
        },
        _ => None,
    }
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

        // An http hook: non-string header value (X-Num: 123) must be silently
        // filtered by json_string_map; allowedEnvVars (camelCase) maps to
        // allowed_env_vars (snake_case).
        let rw = rewritten(
            "Stop",
            serde_json::json!([
                { "hooks": [
                    {
                        "type": "http",
                        "url": "https://example.com/hook",
                        "headers": { "Authorization": "Bearer x", "X-Num": 123 },
                        "allowedEnvVars": ["TOKEN"]
                    }
                ]}
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert_eq!(hooks.len(), 1);
        assert!(drops.is_empty());
        let Handler::Http {
            ref url,
            ref headers,
            ref allowed_env_vars,
        } = hooks[0].handler
        else {
            panic!("expected Http handler, got {:?}", hooks[0].handler);
        };
        assert_eq!(url, "https://example.com/hook");
        assert_eq!(allowed_env_vars, &["TOKEN"]);
        // Non-string header X-Num:123 is filtered; only the string-valued
        // Authorization header survives.
        assert_eq!(headers.len(), 1, "only string headers survive: {headers:?}");
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer x")
        );

        // A prompt hook → KEPT (the parse layer does not gate prompts; US6's
        // config-gate is applied by the caller).
        let rw = rewritten(
            "PreToolUse",
            serde_json::json!([
                { "hooks": [ { "type": "prompt", "prompt": "Check this tool call" } ] }
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert_eq!(hooks.len(), 1);
        assert!(drops.is_empty());
        assert_eq!(
            hooks[0].handler,
            Handler::Prompt {
                prompt: "Check this tool call".into()
            }
        );

        // A command hook missing the required "command" field → UnsupportedHandler.
        let rw = rewritten(
            "PreToolUse",
            serde_json::json!([
                { "hooks": [ { "type": "command" } ] }
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert!(hooks.is_empty());
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].reason, HookDropReason::UnsupportedHandler);

        // An http hook missing the required "url" field → UnsupportedHandler.
        let rw = rewritten(
            "PreToolUse",
            serde_json::json!([
                { "hooks": [ { "type": "http" } ] }
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert!(hooks.is_empty());
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].reason, HookDropReason::UnsupportedHandler);

        // A group with an "if" predicate → the predicate is extracted verbatim
        // into CanonicalHook.if_pred.
        let rw = rewritten(
            "PreToolUse",
            serde_json::json!([
                { "if": "Bash(git push *)", "hooks": [
                    { "type": "command", "command": "/p/check.sh" }
                ]}
            ]),
        );
        let mut drops = Vec::new();
        let hooks = parse_canonical_hooks("cat", "plug", &rw, &mut drops);
        assert_eq!(hooks.len(), 1);
        assert!(drops.is_empty());
        assert_eq!(hooks[0].if_pred.as_deref(), Some("Bash(git push *)"));

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

    #[test]
    fn matcher_semantics_exact_pipe_regex() {
        assert!(matcher_matches(None, "Bash"));
        assert!(matcher_matches(Some(""), "Bash"));
        assert!(matcher_matches(Some("*"), "Bash"));
        assert!(matcher_matches(Some("Bash"), "Bash"));
        assert!(!matcher_matches(Some("Bash"), "Edit"));
        assert!(matcher_matches(Some("Edit|Write"), "Write"));
        assert!(matcher_matches(Some("Edit,Write"), "Write"));
        assert!(!matcher_matches(Some("Edit,Write"), "Bash"));
        assert!(matcher_matches(Some("mcp__.*__write"), "mcp__gh__write"));
        assert!(!matcher_matches(Some("mcp__.*__write"), "Bash"));
    }

    #[test]
    fn harness_tool_inverse_map() {
        assert_eq!(cc_tool_name("gemini", "run_shell_command"), Some("Bash"));
        assert_eq!(cc_tool_name("gemini", "replace"), Some("Edit"));
        assert_eq!(cc_tool_name("codex", "Bash"), Some("Bash"));
        assert_eq!(cc_tool_name("devin", "exec"), Some("Bash"));
        assert_eq!(cc_tool_name("gemini", "unknown_tool"), None);
        // copilot-cli arm: every mapping present in cc_tool_name
        assert_eq!(cc_tool_name("copilot-cli", "bash"), Some("Bash"));
        assert_eq!(cc_tool_name("copilot-cli", "view"), Some("Read"));
        assert_eq!(cc_tool_name("copilot-cli", "create"), Some("Write"));
        assert_eq!(cc_tool_name("copilot-cli", "edit"), Some("Edit"));
        assert_eq!(cc_tool_name("copilot-cli", "grep"), Some("Grep"));
        assert_eq!(cc_tool_name("copilot-cli", "rg"), Some("Grep"));
        assert_eq!(cc_tool_name("copilot-cli", "glob"), Some("Glob"));
        assert_eq!(cc_tool_name("copilot-cli", "web_fetch"), Some("WebFetch"));
        assert_eq!(cc_tool_name("copilot-cli", "web_search"), Some("WebSearch"));
        // unmapped copilot-cli tool returns None
        assert_eq!(cc_tool_name("copilot-cli", "ask_user"), None);
    }

    #[test]
    fn manifest_roundtrips_and_is_strict() {
        let m = HookManifest {
            harness: "codex".into(),
            raw_event_passthrough: false,
            events: BTreeMap::from([(
                "PreToolUse".to_owned(),
                vec![
                    ManifestEntry {
                        plugin: "cat:plug".into(),
                        matcher: Some("Bash".into()),
                        if_pred: None,
                        handler: Handler::Command {
                            command: "/p/guard.sh".into(),
                        },
                        timeout_ms: Some(30_000),
                        cwd: None,
                        env: BTreeMap::new(),
                    },
                    // Second entry: Http handler with if_pred, exercising
                    // the rename="if" serialisation and the
                    // skip_serializing_if guards on headers/allowed_env_vars.
                    ManifestEntry {
                        plugin: "cat:plug2".into(),
                        matcher: None,
                        if_pred: Some("Bash(git push *)".into()),
                        handler: Handler::Http {
                            url: "https://example.com/hook".into(),
                            headers: BTreeMap::from([(
                                "Authorization".to_owned(),
                                "Bearer token".to_owned(),
                            )]),
                            allowed_env_vars: vec!["TOKEN".to_owned()],
                        },
                        timeout_ms: None,
                        cwd: None,
                        env: BTreeMap::new(),
                    },
                ],
            )]),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks-manifest.json");
        write_manifest(&path, &m).unwrap();
        let back = read_manifest(&path).unwrap();
        assert_eq!(back, m);
        // Strict: an unknown top-level key is rejected.
        std::fs::write(&path, r#"{"harness":"x","events":{},"bogus":1}"#).unwrap();
        assert!(read_manifest(&path).is_err());
        // Strict (locks Fix 1): a handler object with an unknown key must also
        // be rejected because Handler carries deny_unknown_fields.
        std::fs::write(
            &path,
            r#"{"harness":"x","events":{"PreToolUse":[{"plugin":"p","handler":{"type":"command","command":"x","bogus":1}}]}}"#,
        )
        .unwrap();
        assert!(read_manifest(&path).is_err());
    }
}
