//! Formatting numbers for the human watching the terminal.

use aicommit_core::Usage;

/// The "12,345 in / 678 out, $0.0034" summary shown when generation finishes.
/// Empty when the agent reported nothing, so the caller can omit the suffix.
pub(crate) fn metrics_line(usage: Option<&Usage>) -> String {
    let Some(u) = usage else {
        return String::new();
    };
    let mut line = match (u.total_input_tokens, u.output_tokens) {
        (Some(input), Some(output)) => {
            format!("{} in / {} out", fmt_tokens(input), fmt_tokens(output))
        }
        (Some(input), None) => format!("{} in", fmt_tokens(input)),
        (None, Some(output)) => format!("{} out", fmt_tokens(output)),
        (None, None) => String::new(),
    };
    if let Some(cost) = u.cost_usd {
        if !line.is_empty() {
            line.push_str(", ");
        }
        line.push_str(&fmt_cost(cost));
    }
    line
}

/// Format a token count with thousands separators (e.g. 12345 -> "12,345").
fn fmt_tokens(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Format cost as "$0.0034".
fn fmt_cost(usd: f64) -> String {
    format!("${usd:.4}")
}

/// Human-readable byte size for the auto-model notice, e.g. 48128 -> "47 KB".
pub(crate) fn fmt_size(bytes: usize) -> String {
    if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_tokens_separators() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1000), "1,000");
        assert_eq!(fmt_tokens(12345), "12,345");
        assert_eq!(fmt_tokens(1_000_000), "1,000,000");
    }

    #[test]
    fn fmt_size_units() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(1024), "1 KB");
        assert_eq!(fmt_size(48_128), "47 KB");
    }

    #[test]
    fn metrics_line_shape() {
        // agent-text reports an already-normalized total input count.
        let usage = Usage {
            total_input_tokens: Some(12_345),
            cached_input_tokens: Some(100),
            cache_write_input_tokens: Some(345),
            output_tokens: Some(678),
            cost_usd: Some(0.0034),
        };
        assert_eq!(metrics_line(Some(&usage)), "12,345 in / 678 out, $0.0034");

        // An agent that prices nothing gets no cost suffix.
        let unpriced = Usage {
            cost_usd: None,
            ..usage
        };
        assert_eq!(metrics_line(Some(&unpriced)), "12,345 in / 678 out");

        let cost_only = Usage {
            cost_usd: Some(0.0034),
            ..Default::default()
        };
        assert_eq!(metrics_line(Some(&cost_only)), "$0.0034");

        // An agent that reports nothing gets no line at all.
        assert_eq!(metrics_line(None), "");
    }
}
