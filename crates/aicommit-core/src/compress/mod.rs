//! Fitting a change into a token budget without losing any of it.
//!
//! The problem this solves is not "make the diff smaller" but "spend a fixed
//! budget so the model can describe the whole change". Those differ in one
//! important way: truncating at a byte cap silently drops every file after the
//! cut, so a one-line fix at the end of a large refactor becomes invisible.
//!
//! So instead of truncating, every file is always represented, and detail is
//! bought with what budget remains:
//!
//! 1. Classify each file ([`analyze`]) — reformats, repeated substitutions,
//!    relocated blocks, bulk data, generated files and binaries all carry far
//!    fewer bits than their byte count suggests.
//! 2. Collapse the classes that are provably uninformative, whatever the budget.
//! 3. Spend the rest by repeatedly upgrading whichever file gains the most
//!    meaning per byte, so breadth is bought before depth.
//!
//! The analysis is entirely lexical, so it behaves identically on source code,
//! prose, CSV, LaTeX or anything else a repository might hold.

mod analyze;
mod render;

use analyze::Analysis;
use analyze::Kind;
use karet_diff::Diff;

pub use analyze::MovedBlock;
pub use analyze::SubstitutionCluster;
pub use render::Detail;

/// How much of a file's diff survived, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileReport {
    /// The file's path, as the diff names it.
    pub path: String,
    /// The level of detail it was rendered at.
    pub detail: Detail,
    /// A short human explanation, for `--dry-run`.
    pub reason: String,
    /// Lines added and removed.
    pub added: usize,
    /// Lines removed.
    pub removed: usize,
    /// Bytes this file occupies in the rendered output.
    pub rendered_bytes: usize,
}

/// What the compressor did to a whole diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompressionReport {
    /// Size of the diff handed in.
    pub original_bytes: usize,
    /// Size of the diff handed back.
    pub compressed_bytes: usize,
    /// Bytes spent on the cross-file preamble (repeated edits, moved blocks).
    /// The per-file byte counts plus this equal [`Self::compressed_bytes`].
    pub preamble_bytes: usize,
    /// One entry per file, in diff order. Never shorter than the input's file list.
    pub files: Vec<FileReport>,
    /// Substitutions repeated often enough to state once.
    pub clusters: Vec<SubstitutionCluster>,
    /// Blocks that moved rather than changed.
    pub moves: Vec<MovedBlock>,
    /// Whether anything was actually collapsed or degraded. When false the output
    /// carries the whole diff and the prompt needs no explanation of the notation.
    pub compressed: bool,
    /// Set when the diff could not be parsed and was passed through untouched.
    pub passthrough: Option<String>,
}

impl CompressionReport {
    /// Bytes saved, or zero when the output grew (a tiny diff gains a header).
    #[must_use]
    pub fn bytes_saved(&self) -> usize {
        self.original_bytes.saturating_sub(self.compressed_bytes)
    }
}

/// How much room the rendered diff may take.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompressOptions {
    /// The byte budget for the rendered diff. Treated as a target, not a hard
    /// limit: every file is represented even if the floor exceeds it.
    pub max_bytes: usize,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self {
            max_bytes: crate::DEFAULT_MAX_DIFF_BYTES,
        }
    }
}

/// Render `diff` into `opts.max_bytes`, keeping every file.
///
/// Returns the rendered diff and a report of what was done. If `diff` cannot be
/// parsed as a unified diff it is returned unchanged, with
/// [`CompressionReport::passthrough`] explaining why — the caller's commit should
/// never fail because the compressor did not recognize something.
#[must_use]
pub fn compress_diff(diff: &str, opts: &CompressOptions) -> (String, CompressionReport) {
    let parsed = match karet_diff::parse(diff) {
        Ok(parsed) if !parsed.files.is_empty() => parsed,
        Ok(_) => return passthrough(diff, "no files found in the diff"),
        Err(e) => return passthrough(diff, &e.to_string()),
    };

    let analysis = analyze::analyze(&parsed);
    let details = allocate(&parsed, &analysis, opts.max_bytes);
    finish(diff, &parsed, &analysis, &details)
}

/// Hand the diff back untouched, recording why.
fn passthrough(diff: &str, reason: &str) -> (String, CompressionReport) {
    let report = CompressionReport {
        original_bytes: diff.len(),
        compressed_bytes: diff.len(),
        passthrough: Some(reason.to_string()),
        ..Default::default()
    };
    (diff.to_string(), report)
}

/// Render the chosen levels and assemble the report.
fn finish(
    original: &str,
    parsed: &Diff,
    analysis: &Analysis,
    details: &[Detail],
) -> (String, CompressionReport) {
    let mut out = render::preamble(&analysis.clusters, &analysis.moves);
    let preamble_bytes = out.len();
    let mut files = Vec::with_capacity(parsed.files.len());
    for (index, file) in parsed.files.iter().enumerate() {
        let facts = &analysis.files[index];
        let detail = details[index];
        let text = render::render_file(file, facts, detail, &analysis.clusters);
        files.push(FileReport {
            path: facts.path.clone(),
            detail,
            reason: reason_for(facts, detail),
            added: facts.added,
            removed: facts.removed,
            rendered_bytes: text.len(),
        });
        out.push_str(&text);
    }

    let compressed = details.iter().any(|d| *d != Detail::Full)
        || analysis.files.iter().any(|f| f.kind != Kind::Normal);
    let report = CompressionReport {
        original_bytes: original.len(),
        compressed_bytes: out.len(),
        preamble_bytes,
        files,
        clusters: analysis.clusters.clone(),
        moves: analysis.moves.clone(),
        compressed,
        passthrough: None,
    };
    (out, report)
}

/// The human-readable reason a file ended up at `detail`.
fn reason_for(facts: &analyze::FileFacts, detail: Detail) -> String {
    match &facts.kind {
        Kind::Normal if detail == Detail::Full => "shown in full".to_string(),
        Kind::Normal => "reduced to fit the budget".to_string(),
        Kind::NoContent => "no content change".to_string(),
        Kind::Binary => "binary".to_string(),
        Kind::Reflow {
            terminators_only: true,
        } => "line endings only".to_string(),
        Kind::Reflow {
            terminators_only: false,
        } => "whitespace-only reformat".to_string(),
        Kind::Substitution { occurrences, .. } => {
            format!("repeated substitution x{occurrences}")
        }
        Kind::BulkData => "bulk data change".to_string(),
        Kind::Generated => "generated file".to_string(),
        Kind::Moved => "relocated content".to_string(),
    }
}

/// Choose a detail level per file, spending `max_bytes` breadth-first.
///
/// Files start at their floor — [`Detail::Ledger`], or whatever an unconditional
/// collapse pins them to — and are then upgraded one step at a time, always
/// picking the file that gains the most priority per additional byte. That
/// ordering is what buys breadth before depth: a second file's outline is worth
/// more than a first file's full context.
fn allocate(parsed: &Diff, analysis: &Analysis, max_bytes: usize) -> Vec<Detail> {
    let mut details: Vec<Detail> = vec![Detail::Ledger; parsed.files.len()];

    // Cost of each file at each level, measured by rendering it.
    let costs: Vec<Vec<usize>> = parsed
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            Detail::ASCENDING
                .iter()
                .map(|d| {
                    render::render_file(file, &analysis.files[index], *d, &analysis.clusters).len()
                })
                .collect()
        })
        .collect();

    let preamble = render::preamble(&analysis.clusters, &analysis.moves).len();
    let mut spent: usize = preamble + costs.iter().map(|c| c[0]).sum::<usize>();

    loop {
        let mut best: Option<(usize, f64, usize)> = None;
        for index in 0..parsed.files.len() {
            // A collapsed file has nothing more worth showing at any price.
            if analysis.files[index].kind.collapses_unconditionally() {
                continue;
            }
            let current = level_index(details[index]);
            let Some(next) = current
                .checked_add(1)
                .filter(|n| *n < Detail::ASCENDING.len())
            else {
                continue;
            };
            let extra = costs[index][next].saturating_sub(costs[index][current]);
            if extra == 0 {
                // Free upgrade: take it without consulting the budget.
                details[index] = Detail::ASCENDING[next];
                continue;
            }
            if spent + extra > max_bytes {
                continue;
            }
            #[expect(
                clippy::cast_precision_loss,
                reason = "value density only; inputs are byte and line counts"
            )]
            let gain = priority(&analysis.files[index]) / extra as f64;
            if best.is_none_or(|(_, best_gain, _)| gain > best_gain) {
                best = Some((index, gain, extra));
            }
        }
        let Some((index, _, extra)) = best else {
            break;
        };
        details[index] = Detail::ASCENDING[level_index(details[index]) + 1];
        spent += extra;
    }

    details
}

/// Where `detail` sits in [`Detail::ASCENDING`].
fn level_index(detail: Detail) -> usize {
    Detail::ASCENDING
        .iter()
        .position(|d| *d == detail)
        .unwrap_or(0)
}

/// How much a file's content is worth relative to others.
///
/// Size counts logarithmically: ten changed lines in each of ten files describe a
/// commit better than a hundred changed lines in one, so raw line count must not
/// dominate. Category is the other half — code and prose carry intent, data and
/// configuration usually carry consequences of it.
fn priority(facts: &analyze::FileFacts) -> f64 {
    use karet_filetype::Category;
    let weight = match facts.category {
        Category::Code | Category::Shell => 1.0,
        Category::Markup | Category::Document => 0.9,
        Category::Config => 0.7,
        Category::Data => 0.5,
        _ => 0.4,
    };
    #[expect(
        clippy::cast_precision_loss,
        reason = "log scale of a line count; precision is irrelevant here"
    )]
    let size = ((facts.added + facts.removed) as f64 + 1.0).ln();
    weight * size
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-file diff replacing `old` lines with `new` lines.
    fn file_diff(path: &str, old: &[String], new: &[String]) -> String {
        let mut s = format!(
            "diff --git a/{path} b/{path}\nindex aaa..bbb 100644\n--- a/{path}\n+++ b/{path}\n@@ -1,{} +1,{} @@\n",
            old.len(),
            new.len()
        );
        for line in old {
            s.push_str(&format!("-{line}\n"));
        }
        for line in new {
            s.push_str(&format!("+{line}\n"));
        }
        s
    }

    fn lines(prefix: &str, n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{prefix} {i}")).collect()
    }

    #[test]
    fn every_file_survives_a_tiny_budget() {
        let mut raw = String::new();
        for i in 0..25 {
            raw.push_str(&file_diff(
                &format!("src/f{i}.rs"),
                &lines("old", 40),
                &lines("new", 40),
            ));
        }
        let (out, report) = compress_diff(&raw, &CompressOptions { max_bytes: 2_000 });

        // The guarantee: a budget far too small still mentions every file, where
        // truncation would have dropped everything after the cut.
        assert_eq!(report.files.len(), 25);
        for i in 0..25 {
            assert!(out.contains(&format!("src/f{i}.rs")), "missing f{i}");
        }
        assert!(out.len() < raw.len());
    }

    #[test]
    fn a_small_diff_is_left_at_full_detail() {
        let raw = file_diff("a.rs", &lines("old", 2), &lines("new", 2));
        let (out, report) = compress_diff(&raw, &CompressOptions::default());
        assert!(report.files.iter().all(|f| f.detail == Detail::Full));
        assert!(!report.compressed);
        assert!(out.contains("-old 0"));
        assert!(out.contains("+new 1"));
    }

    #[test]
    fn breadth_is_bought_before_depth() {
        // One huge file and several small ones, with room for some but not all.
        let mut raw = file_diff("big.rs", &lines("old", 400), &lines("rewritten", 400));
        for i in 0..4 {
            // Distinct edits, so nothing clusters and each file must earn its own
            // budget rather than being collapsed into a shared note.
            raw.push_str(&file_diff(
                &format!("small{i}.rs"),
                &[format!("let a{i} = {i};")],
                &[format!("let b{i} = {};", i * 7)],
            ));
        }
        let (_, report) = compress_diff(&raw, &CompressOptions { max_bytes: 3_000 });
        let small: Vec<&FileReport> = report
            .files
            .iter()
            .filter(|f| f.path.starts_with("small"))
            .collect();
        // Every small file earns full detail before the big one consumes the budget.
        assert!(
            small.iter().all(|f| f.detail == Detail::Full),
            "{:?}",
            small.iter().map(|f| f.detail).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_reformat_collapses_even_with_budget_to_spare() {
        let old: Vec<String> = (0..300).map(|i| format!("  line {i}")).collect();
        let new: Vec<String> = (0..300).map(|i| format!("      line {i}")).collect();
        let raw = file_diff("style.css", &old, &new);
        let (out, report) = compress_diff(
            &raw,
            &CompressOptions {
                max_bytes: 1_000_000,
            },
        );

        assert!(report.compressed);
        assert_eq!(report.files[0].detail, Detail::Ledger);
        assert!(out.contains("whitespace-only reformat"));
        // The 300 identical-modulo-whitespace lines are the whole saving.
        assert!(out.len() * 20 < raw.len(), "{} vs {}", out.len(), raw.len());
    }

    #[test]
    fn repeated_substitutions_are_stated_once() {
        let mut raw = String::new();
        for i in 0..40 {
            raw.push_str(&file_diff(
                &format!("m{i}.py"),
                &(0..20)
                    .map(|j| format!("    return old_name({j})"))
                    .collect::<Vec<_>>(),
                &(0..20)
                    .map(|j| format!("    return new_name({j})"))
                    .collect::<Vec<_>>(),
            ));
        }
        let (out, report) = compress_diff(&raw, &CompressOptions::default());

        assert_eq!(report.clusters.len(), 1);
        assert!(out.contains("# repeated edits"));
        // Every file is still named, but the 800 edits are described once.
        for i in 0..40 {
            assert!(out.contains(&format!("m{i}.py")));
        }
        // Naming all 40 files is the floor the no-dropped-file guarantee imposes,
        // so the win is ~12x rather than unbounded.
        assert!(out.len() * 10 < raw.len(), "{} vs {}", out.len(), raw.len());
    }

    #[test]
    fn output_is_still_a_parseable_diff() {
        let mut raw = file_diff("a.rs", &lines("old", 30), &lines("new", 30));
        raw.push_str(&file_diff("b.md", &lines("x", 5), &lines("y", 5)));
        raw.push_str("diff --git a/i.png b/i.png\nindex aaa..bbb 100644\nBinary files a/i.png and b/i.png differ\n");
        let (out, _) = compress_diff(&raw, &CompressOptions { max_bytes: 400 });

        let reparsed = karet_diff::parse(&out).expect("compressed output is a valid diff");
        let paths: Vec<&str> = reparsed.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.rs", "b.md", "i.png"]);
    }

    #[test]
    fn unparseable_input_is_passed_through_untouched() {
        let raw = "this is not a diff at all\n";
        let (out, report) = compress_diff(raw, &CompressOptions::default());
        assert_eq!(out, raw);
        assert!(report.passthrough.is_some());
        assert!(!report.compressed);
    }

    #[test]
    fn an_empty_diff_is_passed_through() {
        let (out, report) = compress_diff("", &CompressOptions::default());
        assert_eq!(out, "");
        assert!(report.passthrough.is_some());
    }

    #[test]
    fn the_report_accounts_for_every_file() {
        let mut raw = file_diff("a.rs", &lines("old", 3), &lines("new", 3));
        raw.push_str(&file_diff("b.rs", &lines("old", 3), &lines("new", 3)));
        let (out, report) = compress_diff(&raw, &CompressOptions::default());
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.compressed_bytes, out.len());
        assert_eq!(report.original_bytes, raw.len());
        assert_eq!(
            report.preamble_bytes + report.files.iter().map(|f| f.rendered_bytes).sum::<usize>(),
            out.len(),
            "the preamble plus every file must account for the whole output"
        );
    }

    #[test]
    fn results_are_deterministic() {
        let mut raw = String::new();
        for i in 0..6 {
            raw.push_str(&file_diff(
                &format!("f{i}.rs"),
                &(0..10)
                    .map(|j| format!("call old_name({i}, {j});"))
                    .collect::<Vec<_>>(),
                &(0..10)
                    .map(|j| format!("call new_name({i}, {j});"))
                    .collect::<Vec<_>>(),
            ));
        }
        let opts = CompressOptions { max_bytes: 1_500 };
        let (a, ra) = compress_diff(&raw, &opts);
        let (b, rb) = compress_diff(&raw, &opts);
        assert_eq!(a, b);
        assert_eq!(ra, rb);
    }
}
