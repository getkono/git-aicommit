//! Formatting numbers for the human watching the terminal.

use aicommit_core::Usage;

/// The "12,345 in / 678 out, $0.0034" summary shown when generation finishes.
/// Empty when the backend reported nothing, so the caller can omit the suffix.
pub(crate) fn metrics_line(usage: Option<&Usage>) -> String {
    let Some(u) = usage else {
        return String::new();
    };
    let input_total = u.input_tokens + u.cache_creation_input_tokens;
    let mut line = format!(
        "{} in / {} out",
        fmt_tokens(input_total),
        fmt_tokens(u.output_tokens),
    );
    if let Some(cost) = u.cost_usd {
        line.push_str(&format!(", {}", fmt_cost(cost)));
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
        // Cache-creation tokens fold into the input total.
        let usage = Usage {
            input_tokens: 12_000,
            cache_creation_input_tokens: 345,
            output_tokens: 678,
            cost_usd: Some(0.0034),
        };
        assert_eq!(metrics_line(Some(&usage)), "12,345 in / 678 out, $0.0034");

        // A backend that prices nothing gets no cost suffix.
        let unpriced = Usage {
            cost_usd: None,
            ..usage
        };
        assert_eq!(metrics_line(Some(&unpriced)), "12,345 in / 678 out");

        // A backend that reports nothing gets no line at all.
        assert_eq!(metrics_line(None), "");
    }
}
