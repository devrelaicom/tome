//! The `tome status` bookshelf logo. Original art (no third-party source).
//!
//! `ART_WIDTH` is the fixed VISIBLE width of every returned line, so the
//! column zipper can place the panel at a stable offset without measuring
//! ANSI escapes.

use owo_colors::{AnsiColors, OwoColorize};

use crate::presentation::colour;

/// Visible width of each art line (2-space left margin + 25-wide frame).
pub const ART_WIDTH: usize = 27;

const PALETTE: [AnsiColors; 6] = [
    AnsiColors::Red,
    AnsiColors::Green,
    AnsiColors::Yellow,
    AnsiColors::Blue,
    AnsiColors::Magenta,
    AnsiColors::Cyan,
];

// Each shelf's spine row, exactly 23 visible chars (the frame interior).
// Tune these glyphs visually; widths MUST stay 23.
const SHELVES: [&str; 3] = [
    "▌║│█ ▐║ │▌║ █│ ║▐ │║▌  ",
    "║▐ █│║ ▌│ ║█ ▐║│ █ ╱   ",
    "│█▌ ║▐ │█ ║▌│ ▐║ █│║▐  ",
];

/// Colorize a spine row: each run of non-space characters (a "book") takes
/// the next palette colour. Spaces pass through. Plain when colour disabled.
fn paint_spines(row: &str) -> String {
    let mut out = String::new();
    let mut idx = 0usize;
    let mut prev_space = true;
    for ch in row.chars() {
        if ch == ' ' {
            out.push(' ');
            prev_space = true;
            continue;
        }
        if prev_space {
            idx += 1;
        }
        prev_space = false;
        let s = ch.to_string();
        if colour::is_enabled() {
            out.push_str(&s.color(PALETTE[idx % PALETTE.len()]).to_string());
        } else {
            out.push_str(&s);
        }
    }
    out
}

/// The full bookshelf, as `ART_WIDTH`-visible-wide lines (top-aligned).
pub fn bookshelf() -> Vec<String> {
    let margin = "  "; // 2-space left margin
    let top = format!("{margin}┌───────────────────────┐");
    let div = format!("{margin}├───────────────────────┤");
    let bot = format!("{margin}└───────────────────────┘");
    let frame = |body: &str| format!("{margin}│{body}│");

    let mut lines = Vec::new();
    lines.push(top);
    for (i, shelf) in SHELVES.iter().enumerate() {
        lines.push(frame(&paint_spines(shelf)));
        lines.push(frame(&paint_spines(shelf)));
        if i < SHELVES.len() - 1 {
            lines.push(div.clone());
        }
    }
    lines.push(bot);
    lines
}
