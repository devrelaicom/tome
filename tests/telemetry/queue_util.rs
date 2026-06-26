//! Shared helpers for the telemetry integration suite after the gauge-telemetry
//! migration.
//!
//! The kernel (`gauge-telemetry`) now owns the disk queue and the wire envelope.
//! A queued line is the kernel's `QueuedEvent`:
//!
//! ```json
//! {"event_name":"tome.search","time_unix_nano":1234,"attributes":{"surface":"cli",...}}
//! ```
//!
//! i.e. the bare event name lives under `event_name` (namespaced `tome.<name>` by
//! the client), the per-event fields are nested under `attributes`, and the
//! install/session identity is NOT in the queue line — the kernel attaches it as
//! OTLP resource attributes only at drain time. So tests read the event name from
//! `event_name` and the dimensions from `attributes`, and never assert on a
//! per-line install/session uuid (there is none in the queue).
//!
//! Every emit-assertion test must (1) ENABLE telemetry in the subprocess
//! (`[telemetry] enabled = true`, clear `CI`, set a loopback `TOME_GAUGE_ENDPOINT`
//! so the kernel `build()` validates the endpoint — emit only appends, so no
//! network occurs) and (2) read the kernel queue file at
//! `paths.telemetry_queue()`.

#![allow(dead_code)] // each member uses a subset of these helpers

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Every env var that can flip the kernel's resolved consent. A subprocess
/// emit-assertion test must clear ALL of them, then set only what it needs, so a
/// CI runner's ambient `CI=true` can't auto-disable the child.
pub const TELEMETRY_ENV_VARS: &[&str] = &[
    "TOME_TELEMETRY",
    "TOME_GAUGE_ENDPOINT",
    "GAUGE_TELEMETRY_DISABLE",
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "CIRCLECI",
    "BUILDKITE",
    "JENKINS_URL",
    "TF_BUILD",
    "TEAMCITY_VERSION",
];

/// A loopback OTLP endpoint the kernel `build()` accepts (`endpoint_allowed`
/// passes https-or-loopback). `emit` only appends to the queue, so nothing ever
/// connects here — it just lets an ENABLED handle build under test.
pub const LOOPBACK_ENDPOINT: &str = "http://127.0.0.1:1";

/// The kernel queue file under a given isolated tome root.
pub fn queue_path_in_root(tome_root: &Path) -> PathBuf {
    tome_root.join("telemetry").join("queue.jsonl")
}

/// Read every queued telemetry line under `tome_root` as parsed JSON objects.
/// Empty when the queue file doesn't exist yet.
pub fn queue_events_in_root(tome_root: &Path) -> Vec<Value> {
    let body = match std::fs::read_to_string(queue_path_in_root(tome_root)) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("queued line is JSON"))
        .collect()
}

/// Read the kernel queue under a library `Paths` (used by in-process emit tests
/// rooted at a `HomeGuard`-pinned `$HOME/.tome`).
pub fn queue_events(paths: &tome::paths::Paths) -> Vec<Value> {
    let body = match std::fs::read_to_string(paths.telemetry_queue()) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("queued line is JSON"))
        .collect()
}

/// The first queued event whose kernel `event_name` matches `name`
/// (e.g. `"tome.search"`).
pub fn first_named<'a>(events: &'a [Value], name: &str) -> Option<&'a Value> {
    events.iter().find(|e| e["event_name"] == name)
}

/// Count queued events whose kernel `event_name` matches `name`.
pub fn count_named(events: &[Value], name: &str) -> usize {
    events.iter().filter(|e| e["event_name"] == name).count()
}

/// Read one attribute of a queued event from its `attributes` map.
pub fn attr<'a>(event: &'a Value, key: &str) -> &'a Value {
    &event["attributes"][key]
}
