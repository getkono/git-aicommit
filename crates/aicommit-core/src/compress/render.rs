//! Turning a classified file back into diff text at a chosen level of detail.
//!
//! The output stays a unified diff, because that is the format models read best.
//! Anything the compressor wants to say about a file it says on a `#` comment
//! line where the body would have been, so the shape never becomes something
//! unfamiliar.

use std::fmt::Write as _;

use karet_diff::FileDiff;
use karet_diff::FileStatus;
use karet_diff::Hunk;
use karet_diff::LineKind;

use crate::compress::analyze::FileFacts;
use crate::compress::analyze::Kind;
use crate::compress::analyze::SubstitutionCluster;

/// Content lines longer than this are cut, with the tail replaced by a marker.
const MAX_LINE: usize = 500;

/// A run of same-kind lines longer than this is elided down to its first few.
const MAX_RUN: usize = 12;

/// Sample rows shown per side for a bulk data change.
const DATA_SAMPLE: usize = 3;

/// How much of a file's diff is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detail {
    /// The header and a one-line note; no body.
    Ledger,
    /// Hunk headers with their enclosing scope, but no content.
    Outline,
    /// Changed lines only, with long runs and long lines elided.
    Condensed,
    /// Every hunk in full, with context.
    Full,
}

impl Detail {
    /// The levels a file can be shown at, cheapest first.
    pub(crate) const ASCENDING: [Detail; 4] = [
        Detail::Ledger,
        Detail::Outline,
        Detail::Condensed,
        Detail::Full,
    ];
}

/// Render one file at `detail`, including its `diff --git` header.
pub(crate) fn render_file(
    file: &FileDiff,
    facts: &FileFacts,
    detail: Detail,
    clusters: &[SubstitutionCluster],
) -> String {
    let mut out = header(file);
    if let Some(note) = collapse_note(file, facts, clusters) {
        for line in note.lines() {
            let _ = writeln!(out, "# {line}");
        }
        // A collapsed file says its piece and stops; the body is the thing being
        // saved. Bulk data still shows a sample, since the values are the change.
        if facts.kind == Kind::BulkData && detail > Detail::Ledger {
            out.push_str(&data_sample(file));
        }
        return out;
    }

    match detail {
        Detail::Ledger => {
            let _ = writeln!(
                out,
                "# {} changed lines, body omitted",
                facts.added + facts.removed
            );
        }
        Detail::Outline => {
            let _ = writeln!(
                out,
                "# hunk headers only, {} changed lines",
                facts.added + facts.removed
            );
            for hunk in &file.hunks {
                let _ = writeln!(out, "{}", hunk.header);
            }
        }
        Detail::Condensed => {
            let _ = writeln!(out, "# context omitted");
            for hunk in &file.hunks {
                out.push_str(&condensed_hunk(hunk));
            }
        }
        Detail::Full => {
            for hunk in &file.hunks {
                out.push_str(&full_hunk(hunk));
            }
        }
    }
    out
}

/// The `diff --git` line plus the rename/copy/mode facts worth a model's attention.
fn header(file: &FileDiff) -> String {
    let old_path = file.old_path.as_deref().unwrap_or(&file.path);
    let mut out = format!("diff --git a/{} b/{}\n", old_path, file.path);
    match file.status {
        FileStatus::Renamed { similarity } => {
            let _ = writeln!(
                out,
                "rename from {old_path}\nrename to {}\nsimilarity index {similarity}%",
                file.path
            );
        }
        FileStatus::Copied { similarity } => {
            let _ = writeln!(
                out,
                "copy from {old_path}\ncopy to {}\nsimilarity index {similarity}%",
                file.path
            );
        }
        FileStatus::Added => out.push_str("new file\n"),
        FileStatus::Removed => out.push_str("deleted file\n"),
        FileStatus::TypeChanged => out.push_str("file type changed\n"),
        FileStatus::Unmerged => out.push_str("unmerged (conflicted)\n"),
        _ => {}
    }
    if file.mode_changed() {
        let old = file.old_mode.unwrap_or_default();
        let new = file.new_mode.unwrap_or_default();
        let _ = writeln!(out, "old mode {old:06o}\nnew mode {new:06o}");
    }
    out
}

/// The one-line explanation for a file whose body is deliberately not shown.
fn collapse_note(
    file: &FileDiff,
    facts: &FileFacts,
    clusters: &[SubstitutionCluster],
) -> Option<String> {
    let (added, removed) = (facts.added, facts.removed);
    match &facts.kind {
        Kind::Normal => None,
        Kind::NoContent => Some("no content change".to_string()),
        Kind::Binary => Some("binary file changed".to_string()),
        Kind::Reflow {
            terminators_only: true,
        } => Some(format!("line endings changed only, +{added}/-{removed}")),
        Kind::Reflow {
            terminators_only: false,
        } => Some(format!(
            "whitespace-only reformat, +{added}/-{removed} (text identical ignoring whitespace)"
        )),
        Kind::Substitution {
            cluster,
            occurrences,
        } => {
            let c = clusters.get(*cluster)?;
            // The preamble already explains the substitution; per file, only the
            // identity and the count are worth the bytes.
            Some(format!("s/{}/{}/ x{occurrences}", c.from, c.to))
        }
        Kind::BulkData => Some(format!(
            "bulk data change, +{added}/-{removed} rows in {}",
            file.path
        )),
        Kind::Generated => Some(format!("generated file, +{added}/-{removed}")),
        Kind::Moved => Some(format!(
            "relocated content only, +{added}/-{removed} (see moved blocks above)"
        )),
    }
}

/// A few representative rows from each side of a bulk data change.
fn data_sample(file: &FileDiff) -> String {
    let mut out = String::new();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for hunk in &file.hunks {
        for line in &hunk.lines {
            match line.kind {
                LineKind::Remove if removed.len() < DATA_SAMPLE => removed.push(&line.content),
                LineKind::Add if added.len() < DATA_SAMPLE => added.push(&line.content),
                _ => {}
            }
        }
    }
    if removed.is_empty() && added.is_empty() {
        return out;
    }
    out.push_str("# sample rows:\n");
    for line in removed {
        let _ = writeln!(out, "-{}", clip(line));
    }
    for line in added {
        let _ = writeln!(out, "+{}", clip(line));
    }
    out
}

/// A hunk with every line, as git wrote it.
fn full_hunk(hunk: &Hunk) -> String {
    let mut out = format!("{}\n", hunk.header);
    for line in &hunk.lines {
        let _ = writeln!(out, "{}{}", prefix(line.kind), clip(&line.content));
    }
    out
}

/// A hunk with context dropped and long runs elided, headed by a recomputed
/// range so the result is still a well-formed `-U0`-style hunk.
fn condensed_hunk(hunk: &Hunk) -> String {
    let changed: Vec<&karet_diff::DiffLine> = hunk
        .lines
        .iter()
        .filter(|l| l.kind != LineKind::Context)
        .collect();
    if changed.is_empty() {
        return String::new();
    }
    let old_start = changed
        .iter()
        .find_map(|l| l.old_lineno)
        .unwrap_or(hunk.old_start);
    let new_start = changed
        .iter()
        .find_map(|l| l.new_lineno)
        .unwrap_or(hunk.new_start);
    let old_count = changed
        .iter()
        .filter(|l| l.kind == LineKind::Remove)
        .count();
    let new_count = changed.iter().filter(|l| l.kind == LineKind::Add).count();

    let mut out = format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@");
    if let Some(scope) = hunk.scope.as_deref() {
        let _ = write!(out, " {scope}");
    }
    out.push('\n');

    // Elide the middle of a long same-kind run: the first few lines carry the
    // shape of the change, and a count carries the size.
    let mut i = 0;
    while i < changed.len() {
        let kind = changed[i].kind;
        let start = i;
        while i < changed.len() && changed[i].kind == kind {
            i += 1;
        }
        let run = &changed[start..i];
        for line in run.iter().take(MAX_RUN) {
            let _ = writeln!(out, "{}{}", prefix(kind), clip(&line.content));
        }
        if run.len() > MAX_RUN {
            let _ = writeln!(out, "# … {} more {} lines", run.len() - MAX_RUN, verb(kind));
        }
    }
    out
}

fn prefix(kind: LineKind) -> char {
    match kind {
        LineKind::Add => '+',
        LineKind::Remove => '-',
        LineKind::Context => ' ',
    }
}

fn verb(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Add => "added",
        LineKind::Remove => "removed",
        LineKind::Context => "context",
    }
}

/// Cut an over-long line on a char boundary, marking what was dropped.
fn clip(line: &str) -> String {
    if line.len() <= MAX_LINE {
        return line.to_string();
    }
    let mut end = MAX_LINE;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [{} more chars]", &line[..end], line.len() - end)
}

/// The preamble describing patterns that span files, emitted once above the diffs.
pub(crate) fn preamble(
    clusters: &[SubstitutionCluster],
    moves: &[super::analyze::MovedBlock],
) -> String {
    let mut out = String::new();
    if !clusters.is_empty() {
        out.push_str("# repeated edits (shown once, omitted at each site):\n");
        for c in clusters {
            let _ = writeln!(
                out,
                "#   s/{}/{}/ — {} occurrences in {} file(s): {}",
                c.from,
                c.to,
                c.occurrences,
                c.paths.len(),
                summarize_paths(&c.paths)
            );
        }
    }
    if !moves.is_empty() {
        out.push_str("# relocated blocks (content unchanged):\n");
        for m in moves {
            let _ = writeln!(
                out,
                "#   {} -> {} ({} lines)",
                m.from_path, m.to_path, m.lines
            );
        }
    }
    out
}

/// List paths, naming the first few and counting the rest.
fn summarize_paths(paths: &[String]) -> String {
    const SHOWN: usize = 4;
    if paths.len() <= SHOWN {
        return paths.join(", ");
    }
    format!(
        "{}, and {} more",
        paths[..SHOWN].join(", "),
        paths.len() - SHOWN
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compress::analyze;

    fn parse(raw: &str) -> karet_diff::Diff {
        karet_diff::parse(raw).expect("fixture parses")
    }

    fn facts_for(diff: &karet_diff::Diff) -> Vec<FileFacts> {
        analyze::analyze(diff).files
    }

    const EDIT: &str = "diff --git a/a.rs b/a.rs\nindex aaa..bbb 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 +1,4 @@ fn main() {\n ctx one\n-let x = 1;\n+let x = 2;\n ctx two\n";

    #[test]
    fn full_detail_keeps_context_and_scope() {
        let diff = parse(EDIT);
        let facts = facts_for(&diff);
        let out = render_file(&diff.files[0], &facts[0], Detail::Full, &[]);
        assert!(out.starts_with("diff --git a/a.rs b/a.rs\n"));
        assert!(out.contains("@@ -1,4 +1,4 @@ fn main() {"));
        assert!(out.contains(" ctx one\n"));
        assert!(out.contains("-let x = 1;\n"));
        assert!(out.contains("+let x = 2;\n"));
    }

    #[test]
    fn condensed_drops_context_and_recomputes_the_range() {
        let diff = parse(EDIT);
        let facts = facts_for(&diff);
        let out = render_file(&diff.files[0], &facts[0], Detail::Condensed, &[]);
        assert!(!out.contains(" ctx one"));
        assert!(out.contains("-let x = 1;"));
        assert!(out.contains("+let x = 2;"));
        // One removal and one addition, starting at the lines they occupy.
        assert!(out.contains("@@ -2,1 +2,1 @@ fn main() {"), "{out}");
    }

    #[test]
    fn outline_keeps_only_headers() {
        let diff = parse(EDIT);
        let facts = facts_for(&diff);
        let out = render_file(&diff.files[0], &facts[0], Detail::Outline, &[]);
        assert!(out.contains("@@ -1,4 +1,4 @@ fn main() {"));
        assert!(!out.contains("let x"));
    }

    #[test]
    fn ledger_keeps_only_the_header_and_a_count() {
        let diff = parse(EDIT);
        let facts = facts_for(&diff);
        let out = render_file(&diff.files[0], &facts[0], Detail::Ledger, &[]);
        assert!(out.contains("diff --git a/a.rs b/a.rs"));
        assert!(out.contains("# 2 changed lines, body omitted"));
        assert!(!out.contains("let x"));
    }

    #[test]
    fn detail_levels_shrink_monotonically() {
        let diff = parse(EDIT);
        let facts = facts_for(&diff);
        let sizes: Vec<usize> = Detail::ASCENDING
            .iter()
            .map(|d| render_file(&diff.files[0], &facts[0], *d, &[]).len())
            .collect();
        assert!(
            sizes.windows(2).all(|w| w[0] <= w[1]),
            "levels must not grow as detail drops: {sizes:?}"
        );
    }

    #[test]
    fn a_reformatted_file_states_why_and_shows_nothing() {
        let raw = "diff --git a/s.css b/s.css\nindex aaa..bbb 100644\n--- a/s.css\n+++ b/s.css\n@@ -1,2 +1,2 @@\n-  a { b: c }\n-  d { e: f }\n+    a { b: c }\n+    d { e: f }\n";
        let diff = parse(raw);
        let facts = facts_for(&diff);
        // Even at full detail the body is worth nothing, so it is not shown.
        let out = render_file(&diff.files[0], &facts[0], Detail::Full, &[]);
        assert!(out.contains("whitespace-only reformat, +2/-2"));
        assert!(!out.contains("a { b: c }"));
    }

    #[test]
    fn a_rename_header_carries_its_paths() {
        let diff = parse(
            "diff --git a/old.rs b/new.rs\nsimilarity index 95%\nrename from old.rs\nrename to new.rs\n",
        );
        let facts = facts_for(&diff);
        let out = render_file(&diff.files[0], &facts[0], Detail::Full, &[]);
        assert!(out.contains("rename from old.rs"));
        assert!(out.contains("rename to new.rs"));
        assert!(out.contains("similarity index 95%"));
        assert!(out.contains("# no content change"));
    }

    #[test]
    fn a_mode_change_is_stated_in_octal() {
        let diff = parse("diff --git a/s.sh b/s.sh\nold mode 100644\nnew mode 100755\n");
        let facts = facts_for(&diff);
        let out = render_file(&diff.files[0], &facts[0], Detail::Full, &[]);
        assert!(out.contains("old mode 100644\nnew mode 100755"), "{out}");
    }

    #[test]
    fn long_lines_are_clipped_on_a_char_boundary() {
        let long = "é".repeat(MAX_LINE);
        let clipped = clip(&long);
        assert!(clipped.contains("more chars]"));
        assert!(clipped.len() < long.len() + 40);
    }

    #[test]
    fn long_runs_are_elided_with_a_count() {
        let mut raw = String::from(
            "diff --git a/big.txt b/big.txt\nindex aaa..bbb 100644\n--- a/big.txt\n+++ b/big.txt\n@@ -1,30 +1,0 @@\n",
        );
        for i in 0..30 {
            raw.push_str(&format!("-line {i}\n"));
        }
        let diff = parse(&raw);
        let facts = facts_for(&diff);
        let out = render_file(&diff.files[0], &facts[0], Detail::Condensed, &[]);
        assert!(out.contains("-line 0"));
        assert!(out.contains(&format!("# … {} more removed lines", 30 - MAX_RUN)));
        assert!(!out.contains("-line 29"));
    }

    #[test]
    fn the_preamble_names_clusters_and_moves() {
        let clusters = [SubstitutionCluster {
            from: "old".into(),
            to: "new".into(),
            occurrences: 800,
            paths: (0..40).map(|i| format!("m{i}.py")).collect(),
        }];
        let moves = [analyze::MovedBlock {
            from_path: "a.rs".into(),
            to_path: "b.rs".into(),
            lines: 180,
        }];
        let out = preamble(&clusters, &moves);
        assert!(out.contains("s/old/new/ — 800 occurrences in 40 file(s)"));
        assert!(out.contains("and 36 more"));
        assert!(out.contains("a.rs -> b.rs (180 lines)"));
        // Every preamble line is a comment, so it cannot be read as diff content.
        assert!(out.lines().all(|l| l.starts_with('#')));
    }

    #[test]
    fn an_empty_preamble_costs_nothing() {
        assert!(preamble(&[], &[]).is_empty());
    }
}
