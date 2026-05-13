//! Small formatting helpers shared across command modules. Lives in
//! `presentation` because the output is user-facing; promoted from
//! per-command inline helpers at the rule-of-three (plugin + models +
//! status all needed a "render N bytes as a size string").

/// Render `bytes` as a human-readable MiB string with no fractional digits.
/// Uses the binary mebibyte (1 MiB = 1 048 576 B) and labels with `MB` to
/// stay consistent with the existing CLI output. (See `commands/status.rs`
/// for the formatter that switches to `KiB` / `MiB` / `GiB` suffixes.)
pub fn human_mb(bytes: u64) -> String {
    let mb = (bytes as f64 / 1_048_576.0).round() as u64;
    format!("{mb} MB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_nearest_mb() {
        assert_eq!(human_mb(0), "0 MB");
        assert_eq!(human_mb(1_048_576), "1 MB");
        assert_eq!(human_mb(45 * 1_048_576), "45 MB");
        // 280.5 MiB rounds to 281 MB.
        assert_eq!(human_mb((280.5 * 1_048_576.0) as u64), "281 MB");
    }
}
