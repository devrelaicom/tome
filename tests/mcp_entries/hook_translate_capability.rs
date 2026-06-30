//! Pin tests for per-harness plugin-hook capability declarations (US2.3).
use tome::harness::hooks_ir::PortableEvent;
use tome::harness::{HarnessModule, HookWire, TimeoutUnit};

#[test]
fn gemini_hook_support_declares_events_and_event_names() {
    let s = tome::harness::gemini::GEMINI
        .hook_support()
        .expect("gemini supports hooks");
    assert!(s.events.contains(&PortableEvent::PreToolUse));
    assert!(s.events.contains(&PortableEvent::PostToolUse));
    assert!(s.events.contains(&PortableEvent::PreCompact));
    assert!(matches!(s.timeout_unit, TimeoutUnit::Millis));
    assert!(matches!(s.wire, HookWire::ClaudeStyle));
    assert_eq!(
        tome::harness::gemini::GEMINI.hook_event_name(PortableEvent::PreToolUse),
        "BeforeTool"
    );
    assert_eq!(
        tome::harness::gemini::GEMINI.hook_event_name(PortableEvent::Stop),
        "AfterAgent"
    );
    assert_eq!(
        tome::harness::gemini::GEMINI.hook_event_name(PortableEvent::PreCompact),
        "PreCompress"
    );
    assert_eq!(
        tome::harness::gemini::GEMINI.hook_event_name(PortableEvent::SessionStart),
        "SessionStart"
    );
}

#[test]
fn devin_hook_support_declares_events_and_identity_names() {
    let s = tome::harness::devin::DEVIN
        .hook_support()
        .expect("devin supports hooks");
    assert!(s.events.contains(&PortableEvent::PreToolUse));
    assert!(s.events.contains(&PortableEvent::SessionEnd));
    // Devin drops PreCompact
    assert!(!s.events.contains(&PortableEvent::PreCompact));
    assert!(matches!(s.timeout_unit, TimeoutUnit::Seconds));
    assert!(matches!(s.wire, HookWire::ClaudeStyle));
    // Identity event names (CC PascalCase)
    assert_eq!(
        tome::harness::devin::DEVIN.hook_event_name(PortableEvent::PreToolUse),
        "PreToolUse"
    );
    assert_eq!(
        tome::harness::devin::DEVIN.hook_event_name(PortableEvent::Stop),
        "Stop"
    );
}

#[test]
fn codex_hook_support_declares_events_and_identity_names() {
    let s = tome::harness::codex::CODEX
        .hook_support()
        .expect("codex supports hooks");
    assert!(s.events.contains(&PortableEvent::PreToolUse));
    assert!(s.events.contains(&PortableEvent::PreCompact));
    // Codex drops SessionEnd
    assert!(!s.events.contains(&PortableEvent::SessionEnd));
    assert!(matches!(s.timeout_unit, TimeoutUnit::Seconds));
    assert!(matches!(s.wire, HookWire::Codex));
    assert_eq!(
        tome::harness::codex::CODEX.hook_event_name(PortableEvent::PreToolUse),
        "PreToolUse"
    );
}

#[test]
fn cursor_hook_support_declares_all_events_and_camel_names() {
    let s = tome::harness::cursor::CURSOR
        .hook_support()
        .expect("cursor supports hooks");
    assert!(s.events.contains(&PortableEvent::PreToolUse));
    assert!(s.events.contains(&PortableEvent::SessionEnd));
    assert!(s.events.contains(&PortableEvent::PreCompact));
    assert!(matches!(s.timeout_unit, TimeoutUnit::Seconds));
    assert!(matches!(s.wire, HookWire::CursorSnake));
    assert_eq!(
        tome::harness::cursor::CURSOR.hook_event_name(PortableEvent::PreToolUse),
        "preToolUse"
    );
    assert_eq!(
        tome::harness::cursor::CURSOR.hook_event_name(PortableEvent::UserPromptSubmit),
        "beforeSubmitPrompt"
    );
    assert_eq!(
        tome::harness::cursor::CURSOR.hook_event_name(PortableEvent::Stop),
        "stop"
    );
    assert_eq!(
        tome::harness::cursor::CURSOR.hook_event_name(PortableEvent::PreCompact),
        "preCompact"
    );
}

#[test]
fn copilot_cli_hook_support_declares_all_events_and_identity_names() {
    let s = tome::harness::copilot_cli::COPILOT_CLI
        .hook_support()
        .expect("copilot-cli supports hooks");
    assert!(s.events.contains(&PortableEvent::PreToolUse));
    assert!(s.events.contains(&PortableEvent::SessionEnd));
    assert!(s.events.contains(&PortableEvent::PreCompact));
    assert!(matches!(s.timeout_unit, TimeoutUnit::Seconds));
    assert!(matches!(s.wire, HookWire::CopilotFlat));
    // Identity (PascalCase = CC names)
    assert_eq!(
        tome::harness::copilot_cli::COPILOT_CLI.hook_event_name(PortableEvent::PreToolUse),
        "PreToolUse"
    );
    assert_eq!(
        tome::harness::copilot_cli::COPILOT_CLI.hook_event_name(PortableEvent::Stop),
        "Stop"
    );
}
