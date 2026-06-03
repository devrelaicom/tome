//! Phase 3 Polish (B1) — contract-pinned JSON-lines field names for `mcp.log`.
//!
//! [`contracts/log-format.md`](../specs/003-phase-3-mcp-workspaces/contracts/log-format.md)
//! §File format requires every record to carry `ts`, `level`, `target`,
//! and `msg`. `tracing-subscriber`'s default `.json()` formatter emits
//! `timestamp` / `message` — a silent divergence that breaks every
//! documented `jq` filter and any log shipper built to the contract.
//!
//! This file constructs a `tracing` subscriber wrapping the production
//! `ContractEventFormat` against an in-memory writer, emits one event
//! per level, then asserts each record's key set.

use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing::Subscriber;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

/// In-memory writer that captures every byte the formatter emits.
#[derive(Clone, Default)]
struct InMemoryWriter {
    inner: Arc<Mutex<Vec<u8>>>,
}

impl InMemoryWriter {
    fn snapshot(&self) -> String {
        let bytes = self.inner.lock().unwrap().clone();
        String::from_utf8(bytes).expect("utf-8 log output")
    }
}

impl Write for InMemoryWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for InMemoryWriter {
    type Writer = InMemoryWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Build a subscriber wrapping the production formatter against the
/// in-memory writer. Returns a `DefaultGuard` so the subscriber is
/// dropped at end of test (other tests in the same binary aren't
/// poisoned by a stale global).
fn build_isolated_subscriber(writer: InMemoryWriter) -> tracing::subscriber::DefaultGuard {
    let layer = fmt::layer()
        .event_format(tome::mcp::log::ContractEventFormat)
        .with_writer(writer);
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::set_default(Box::new(subscriber) as Box<dyn Subscriber + Send + Sync>)
}

fn parse_lines(s: &str) -> Vec<serde_json::Value> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("line must be valid JSON"))
        .collect()
}

#[test]
fn every_record_carries_the_four_required_fields() {
    let writer = InMemoryWriter::default();
    let _guard = build_isolated_subscriber(writer.clone());

    tracing::info!(target: "tome::mcp::server", "startup ok");
    tracing::warn!(target: "tome::mcp::tools", "soft warning");
    tracing::error!(target: "tome::mcp::server", "fatal");

    let records = parse_lines(&writer.snapshot());
    assert_eq!(records.len(), 3);
    for rec in &records {
        let obj = rec.as_object().expect("JSON object per line");
        for required in ["ts", "level", "target", "msg"] {
            assert!(
                obj.contains_key(required),
                "record missing `{required}`: {rec}",
            );
        }
        // The deprecated default names must NOT be present — that
        // would mean we accidentally fell back to the stock JSON
        // formatter.
        for forbidden in ["timestamp", "message"] {
            assert!(
                !obj.contains_key(forbidden),
                "record carries forbidden default field `{forbidden}`: {rec}",
            );
        }
    }
}

#[test]
fn level_is_lowercase_per_contract() {
    let writer = InMemoryWriter::default();
    let _guard = build_isolated_subscriber(writer.clone());

    tracing::info!(target: "tome::mcp::server", "i");
    tracing::warn!(target: "tome::mcp::server", "w");
    tracing::error!(target: "tome::mcp::server", "e");

    let records = parse_lines(&writer.snapshot());
    let levels: Vec<_> = records
        .iter()
        .map(|r| r["level"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(levels, ["info", "warn", "error"]);
}

#[test]
fn target_field_matches_event_target() {
    let writer = InMemoryWriter::default();
    let _guard = build_isolated_subscriber(writer.clone());

    tracing::info!(target: "tome::mcp::server", "a");
    tracing::info!(target: "tome::mcp::tools::search_skills", "b");

    let records = parse_lines(&writer.snapshot());
    assert_eq!(records[0]["target"], "tome::mcp::server");
    assert_eq!(records[1]["target"], "tome::mcp::tools::search_skills");
}

#[test]
fn structured_fields_flatten_alongside_required_fields() {
    let writer = InMemoryWriter::default();
    let _guard = build_isolated_subscriber(writer.clone());

    tracing::info!(
        target: "tome::mcp::tools::search_skills",
        query_len = 52,
        top_k = 10u64,
        matches = 7u64,
        elapsed_ms = 214u64,
        "call",
    );

    let records = parse_lines(&writer.snapshot());
    assert_eq!(records.len(), 1);
    let r = &records[0];
    assert_eq!(r["msg"], "call");
    assert_eq!(r["query_len"], 52);
    assert_eq!(r["top_k"], 10);
    assert_eq!(r["matches"], 7);
    assert_eq!(r["elapsed_ms"], 214);
}

#[test]
fn ts_is_rfc3339() {
    let writer = InMemoryWriter::default();
    let _guard = build_isolated_subscriber(writer.clone());

    tracing::info!(target: "tome::mcp::server", "ts-check");

    let records = parse_lines(&writer.snapshot());
    let ts = records[0]["ts"].as_str().expect("ts is a string");
    time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339)
        .expect("ts must parse as RFC3339");
}
