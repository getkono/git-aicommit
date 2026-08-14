//! The `--dry-run` report: everything that shaped the request, before the answer.
//!
//! The compressor decides what the model does and does not see, so its decisions
//! have to be inspectable. This prints the git commands run, the model chosen,
//! the full system prompt, every labeled context item, and a per-file table of
//! what was kept and why.

use std::fmt::Write as _;

use aicommit_core::{CompressionReport, Detail, GenerationRequest};

/// Roughly how many bytes of English or code make one token. Used only to put a
/// familiar number next to the byte counts; nothing depends on its accuracy.
const BYTES_PER_TOKEN: usize = 4;

/// A rough token estimate for `bytes`.
fn tokens(bytes: usize) -> usize {
    bytes.div_ceil(BYTES_PER_TOKEN)
}

/// Format a byte count as `12,345 B (~3,086 tok)`.
fn size(bytes: usize) -> String {
    format!("{} B (~{} tok)", thousands(bytes), thousands(tokens(bytes)))
}

/// Group digits for readability: `12345` -> `12,345`.
fn thousands(n: usize) -> String {
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

/// The short name shown for a detail level.
fn detail_name(detail: Detail) -> &'static str {
    match detail {
        Detail::Ledger => "ledger",
        Detail::Outline => "outline",
        Detail::Condensed => "condensed",
        Detail::Full => "full",
    }
}

/// The whole `--dry-run` report.
///
/// `git_commands` are the argument vectors already run, `model` the resolved
/// model and effort, and `report` the compressor's account of itself (absent
/// when `--no-compact` sent the diff verbatim).
pub(crate) fn report(
    git_commands: &[Vec<String>],
    model: &str,
    prompt: &GenerationRequest,
    report: Option<&CompressionReport>,
) -> String {
    let mut out = String::from("===== dry run: what would be sent =====\n");

    out.push_str("\n--- git commands ---\n");
    for args in git_commands {
        let _ = writeln!(out, "  git {}", args.join(" "));
    }

    let _ = write!(out, "\n--- model ---\n  {model}\n");

    out.push_str(&compression_section(report));

    out.push_str("\n--- system prompt ---\n");
    match prompt.system_prompt.as_deref() {
        Some(system) => {
            for line in system.lines() {
                let _ = writeln!(out, "  {line}");
            }
            let _ = writeln!(out, "  [{}]", size(system.len()));
        }
        None => out.push_str("  (none)\n"),
    }

    let _ = write!(out, "\n--- task ---\n  {}\n", prompt.prompt);

    // The context bodies verbatim, because "what exactly did the model see" is the
    // question a dry run exists to answer.
    out.push_str("\n--- context items ---\n");
    for item in &prompt.context {
        match &item.value {
            agent_text::ContextValue::Text(text) => {
                let _ = writeln!(out, "\n  === {} [{}] ===", item.label, size(text.len()));
                for line in text.lines() {
                    let _ = writeln!(out, "  {line}");
                }
            }
            other => {
                let _ = writeln!(out, "\n  === {} [{other:?}] ===", item.label);
            }
        }
    }

    out
}

/// The per-file table, or an explanation of why there isn't one.
fn compression_section(report: Option<&CompressionReport>) -> String {
    let Some(report) = report else {
        return "\n--- compression ---\n  disabled (--no-compact): the diff is sent verbatim\n"
            .to_string();
    };
    if let Some(reason) = &report.passthrough {
        return format!("\n--- compression ---\n  not applied: {reason}\n");
    }

    let mut out = String::from("\n--- compression ---\n");
    let _ = writeln!(out, "  before: {}", size(report.original_bytes));
    let _ = writeln!(
        out,
        "  after:  {}  ({} saved)",
        size(report.compressed_bytes),
        size(report.bytes_saved())
    );
    if report.preamble_bytes > 0 {
        let _ = writeln!(out, "  preamble: {}", size(report.preamble_bytes));
    }

    if !report.clusters.is_empty() {
        out.push_str("\n  repeated edits:\n");
        for c in &report.clusters {
            let _ = writeln!(
                out,
                "    s/{}/{}/  x{} across {} file(s)",
                c.from,
                c.to,
                c.occurrences,
                c.paths.len()
            );
        }
    }
    if !report.moves.is_empty() {
        out.push_str("\n  relocated blocks:\n");
        for m in &report.moves {
            let _ = writeln!(
                out,
                "    {} -> {}  ({} lines)",
                m.from_path, m.to_path, m.lines
            );
        }
    }

    let _ = write!(
        out,
        "\n  {:<9} {:>7}  {:>9}  {:<28} PATH\n",
        "DETAIL", "+/-", "BYTES", "REASON"
    );
    for file in &report.files {
        let _ = writeln!(
            out,
            "  {:<9} {:>7}  {:>9}  {:<28} {}",
            detail_name(file.detail),
            format!("+{}/-{}", file.added, file.removed),
            thousands(file.rendered_bytes),
            file.reason,
            file.path
        );
    }
    let _ = writeln!(
        out,
        "\n  {} file(s) — every one is listed above",
        report.files.len()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aicommit_core::{CompressOptions, compress_diff};

    fn prompt() -> GenerationRequest {
        GenerationRequest::new("do the thing")
            .with_system_prompt("RULES\nMORE RULES")
            .with_context(agent_text::ContextItem::text("unified diff", "DIFF"))
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn the_report_shows_commands_model_and_prompt() {
        let commands = vec![vec!["diff".to_string(), "--cached".to_string()]];
        let out = report(&commands, "haiku", &prompt(), None);
        assert!(out.contains("git diff --cached"));
        assert!(out.contains("haiku"));
        // The whole system prompt, indented, so it is inspectable verbatim.
        assert!(out.contains("  RULES"));
        assert!(out.contains("  MORE RULES"));
        assert!(out.contains("unified diff"));
        assert!(out.contains("--no-compact"));
    }

    #[test]
    fn every_file_appears_in_the_table() {
        let mut raw = String::new();
        for i in 0..12 {
            raw.push_str(&format!(
                "diff --git a/f{i}.rs b/f{i}.rs\nindex a..b 100644\n--- a/f{i}.rs\n+++ b/f{i}.rs\n@@ -1,1 +1,1 @@\n-old {i}\n+new {i}\n"
            ));
        }
        let (_, compression) = compress_diff(&raw, &CompressOptions::new(400));
        let out = report(&[], "sonnet", &prompt(), Some(&compression));

        for i in 0..12 {
            assert!(
                out.contains(&format!("f{i}.rs")),
                "missing f{i} in the table"
            );
        }
        assert!(out.contains("12 file(s) — every one is listed above"));
        assert!(out.contains("before:"));
        assert!(out.contains("after:"));
    }

    #[test]
    fn passthrough_is_explained_rather_than_tabulated() {
        let (_, compression) = compress_diff("not a diff", &CompressOptions::default());
        let out = report(&[], "haiku", &prompt(), Some(&compression));
        assert!(out.contains("not applied:"));
    }

    #[test]
    fn clusters_and_moves_are_listed() {
        let mut raw = String::new();
        for i in 0..4 {
            raw.push_str(&format!(
                "diff --git a/m{i}.py b/m{i}.py\nindex a..b 100644\n--- a/m{i}.py\n+++ b/m{i}.py\n@@ -1,2 +1,2 @@\n-call old_name({i})\n-call old_name({i}1)\n+call new_name({i})\n+call new_name({i}1)\n"
            ));
        }
        let (_, compression) = compress_diff(&raw, &CompressOptions::default());
        let out = report(&[], "haiku", &prompt(), Some(&compression));
        assert!(out.contains("repeated edits:"), "{out}");
        assert!(out.contains("s/old_name/new_name/"), "{out}");
    }
}
