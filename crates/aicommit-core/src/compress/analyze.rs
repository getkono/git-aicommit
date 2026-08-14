//! Classifying what actually changed in each file, and finding the patterns that
//! repeat across them.
//!
//! Everything here is lexical: lines, tokens and hashes, never syntax. That is
//! deliberate — a commit can touch Rust, prose, CSV, LaTeX or a binary blob, and
//! the analysis has to behave the same way on all of them.

use std::collections::HashMap;

use karet_diff::Diff;
use karet_diff::FileDiff;
use karet_diff::LineKind;
use karet_filetype::Category;

/// Lines longer than this are not word-diffed when hunting for substitutions.
/// Minified and generated content lives above it and carries no useful tokens.
const MAX_SUBSTITUTION_LINE: usize = 1000;

/// The longest `from`/`to` worth calling a substitution. Past this the two lines
/// simply differ, and saying so once would not be shorter than showing them.
const MAX_SUBSTITUTION_TOKEN: usize = 80;

/// The share of a file's changed line pairs one substitution must explain before
/// the file is treated as nothing but that substitution.
const SUBSTITUTION_DOMINANCE: f64 = 0.8;

/// How many occurrences a substitution needs, across every file that shares it,
/// before it is worth reporting once instead of showing each site.
const MIN_CLUSTER_OCCURRENCES: usize = 5;

/// The shortest run of identical lines counted as a moved block. Short runs are
/// too easily coincidental (closing braces, blank lines) to be worth reporting.
const MIN_MOVE_LINES: usize = 6;

/// How many times a line may appear on the added side before it is too ambiguous
/// to anchor a move.
const MAX_MOVE_CANDIDATES: usize = 4;

/// Changed lines a data file needs before its churn is summarized rather than shown.
const BULK_DATA_LINES: usize = 200;

/// What a file's change turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    /// An ordinary content change; show as much as the budget allows.
    Normal,
    /// No hunks at all: a pure rename, a mode change, or an empty file.
    NoContent,
    /// Binary content, which git already refuses to show.
    Binary,
    /// Removed and added text are identical once all whitespace is removed.
    Reflow {
        /// True when only the line terminators changed (e.g. LF to CRLF).
        terminators_only: bool,
    },
    /// Every change is one repeated token substitution, shared with other files.
    Substitution {
        /// Index into [`Analysis::clusters`].
        cluster: usize,
        /// How many times the substitution occurs in this file.
        occurrences: usize,
    },
    /// A data file whose rows changed in bulk.
    BulkData,
    /// A lockfile or other generated artifact.
    Generated,
    /// The file's changes are blocks relocated elsewhere, not new content.
    Moved,
}

impl Kind {
    /// Whether this classification is collapsed regardless of remaining budget,
    /// because showing the body would add bytes but no meaning.
    pub(crate) fn collapses_unconditionally(&self) -> bool {
        matches!(
            self,
            Kind::NoContent
                | Kind::Binary
                | Kind::Reflow { .. }
                | Kind::Substitution { .. }
                | Kind::BulkData
                | Kind::Generated
                | Kind::Moved
        )
    }
}

/// One token substitution repeated across the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubstitutionCluster {
    /// The token being replaced.
    pub from: String,
    /// What it is replaced with.
    pub to: String,
    /// Total occurrences across every file.
    pub occurrences: usize,
    /// The paths it occurs in, in diff order.
    pub paths: Vec<String>,
}

/// A run of lines that moved rather than changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedBlock {
    /// Where the run was removed from.
    pub from_path: String,
    /// Where the identical run was added.
    pub to_path: String,
    /// How many lines moved.
    pub lines: usize,
}

/// Everything learned about a file before any budget is spent.
#[derive(Debug, Clone)]
pub(crate) struct FileFacts {
    pub(crate) path: String,
    pub(crate) kind: Kind,
    pub(crate) category: Category,
    pub(crate) added: usize,
    pub(crate) removed: usize,
}

/// The result of analyzing a whole diff.
#[derive(Debug, Clone, Default)]
pub(crate) struct Analysis {
    pub(crate) files: Vec<FileFacts>,
    pub(crate) clusters: Vec<SubstitutionCluster>,
    pub(crate) moves: Vec<MovedBlock>,
}

/// Classify every file, then fold in the patterns that only appear across files.
pub(crate) fn analyze(diff: &Diff) -> Analysis {
    let mut files: Vec<FileFacts> = diff.files.iter().map(classify_file).collect();
    let moves = find_moves(diff);
    mark_moved_files(diff, &moves, &mut files);
    let clusters = cluster_substitutions(diff, &mut files);
    Analysis {
        files,
        clusters,
        moves,
    }
}

/// Classify one file on its own evidence.
fn classify_file(file: &FileDiff) -> FileFacts {
    let (added, removed) = line_counts(file);
    let category = karet_filetype::category_for_path(std::path::Path::new(&file.path));
    let kind = if file.is_binary {
        Kind::Binary
    } else if file.hunks.is_empty() {
        Kind::NoContent
    } else if is_generated(&file.path) {
        Kind::Generated
    } else if let Some(terminators_only) = reflow_kind(file) {
        Kind::Reflow { terminators_only }
    } else if category == Category::Data && added + removed >= BULK_DATA_LINES {
        Kind::BulkData
    } else {
        Kind::Normal
    };
    FileFacts {
        path: file.path.clone(),
        kind,
        category,
        added,
        removed,
    }
}

/// Added and removed line counts for a file.
pub(crate) fn line_counts(file: &FileDiff) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for hunk in &file.hunks {
        for line in &hunk.lines {
            match line.kind {
                LineKind::Add => added += 1,
                LineKind::Remove => removed += 1,
                LineKind::Context => {}
            }
        }
    }
    (added, removed)
}

/// Whether the file is a lockfile or other machine-generated artifact.
///
/// Matched on the file name rather than the content, because these are exactly
/// the files whose content is too large and too uniform to judge cheaply.
fn is_generated(path: &str) -> bool {
    const LOCKFILES: [&str; 10] = [
        "Cargo.lock",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "poetry.lock",
        "Pipfile.lock",
        "composer.lock",
        "Gemfile.lock",
        "go.sum",
        "flake.lock",
    ];
    let name = path.rsplit('/').next().unwrap_or(path);
    if LOCKFILES.contains(&name) {
        return true;
    }
    // Minified bundles and source maps: the extension is the whole signal.
    name.ends_with(".min.js") || name.ends_with(".min.css") || name.ends_with(".map")
}

/// Whether the file's change is whitespace-only, and if so whether it was purely
/// a line-terminator change.
///
/// Compares the *concatenation* of all removed text against all added text with
/// every whitespace character removed. Doing it across line boundaries rather
/// than line-by-line is what catches prose and comment re-wrapping, where lines
/// are re-split at different points — `git diff -w` reports those as fully
/// changed because it only ignores whitespace *within* a line pair.
fn reflow_kind(file: &FileDiff) -> Option<bool> {
    let mut removed_exact = String::new();
    let mut added_exact = String::new();
    let mut changed = false;
    for hunk in &file.hunks {
        for line in &hunk.lines {
            match line.kind {
                LineKind::Remove => {
                    removed_exact.push_str(&line.content);
                    removed_exact.push('\n');
                    changed = true;
                }
                LineKind::Add => {
                    added_exact.push_str(&line.content);
                    added_exact.push('\n');
                    changed = true;
                }
                LineKind::Context => {}
            }
        }
    }
    if !changed {
        return None;
    }
    // Identical including whitespace means only the terminators can have differed.
    if removed_exact == added_exact {
        return Some(true);
    }
    let strip = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    (strip(&removed_exact) == strip(&added_exact)).then_some(false)
}

/// The single token substitution that explains most of a file's changed lines,
/// with how many lines it explains.
fn dominant_substitution(file: &FileDiff) -> Option<(String, String, usize)> {
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    let mut pairs = 0usize;
    for hunk in &file.hunks {
        for (old, new) in paired_changes(&hunk.lines) {
            pairs += 1;
            if let Some(sub) = single_substitution(old, new) {
                *counts.entry(sub).or_default() += 1;
            }
        }
    }
    if pairs == 0 {
        return None;
    }
    let (sub, count) = counts.into_iter().max_by_key(|(_, c)| *c)?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "ratio only; both counts are line counts well under 2^53"
    )]
    let ratio = count as f64 / pairs as f64;
    (ratio >= SUBSTITUTION_DOMINANCE).then_some((sub.0, sub.1, count))
}

/// Pair each removed line with the added line that replaced it, 1:1 within a run.
///
/// A run of N removals followed by M additions pairs the first `min(N, M)` of
/// each; the leftovers are pure deletions or insertions with no counterpart.
fn paired_changes(lines: &[karet_diff::DiffLine]) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Remove {
            i += 1;
            continue;
        }
        let removes_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Remove {
            i += 1;
        }
        let adds_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Add {
            i += 1;
        }
        let removes = &lines[removes_start..adds_start];
        let adds = &lines[adds_start..i];
        for (r, a) in removes.iter().zip(adds.iter()) {
            out.push((r.content.as_str(), a.content.as_str()));
        }
    }
    out
}

/// The single edit that turns `old` into `new`, as a `(from, to)` token pair.
///
/// Found by stripping the common prefix and suffix, then widening what remains
/// out to word boundaries. Widening is what makes the result *nameable*: the raw
/// difference between `return old_name(1)` and `return new_name(1)` is `old` vs
/// `new`, which says little, whereas `old_name` vs `new_name` is the edit someone
/// would describe in a commit message.
///
/// It is also why this cannot use `compute_highlights`, whose runs are split on
/// whitespace alone — that makes `old_name(1)` a single token, so the varying
/// argument would give every line a different substitution and nothing would
/// ever cluster.
fn single_substitution(old: &str, new: &str) -> Option<(String, String)> {
    if old == new || old.len() > MAX_SUBSTITUTION_LINE || new.len() > MAX_SUBSTITUTION_LINE {
        return None;
    }

    let mut start = common_prefix(old, new);
    let mut end_back = common_suffix(&old[start..], &new[start..]);

    // Grow the differing region outwards over word characters, so it names a whole
    // identifier rather than the bare letters that happen to differ. Both sides
    // share the prefix and suffix being absorbed, so testing `old` decides both.
    while start > 0 && is_word_byte(old.as_bytes()[start - 1]) {
        start -= 1;
    }
    while end_back > 0
        && old.len() - end_back > start
        && new.len() - end_back > start
        && is_word_byte(old.as_bytes()[old.len() - end_back])
    {
        end_back -= 1;
    }

    let from = old.get(start..old.len().checked_sub(end_back)?)?.trim();
    let to = new.get(start..new.len().checked_sub(end_back)?)?.trim();
    if from.is_empty() || to.is_empty() || from == to {
        return None;
    }
    if from.len() > MAX_SUBSTITUTION_TOKEN || to.len() > MAX_SUBSTITUTION_TOKEN {
        return None;
    }
    Some((from.to_string(), to.to_string()))
}

/// Bytes shared at the start of both strings, rounded down to a char boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    let mut i = 0;
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    while i < ab.len() && i < bb.len() && ab[i] == bb[i] {
        i += 1;
    }
    while !a.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Bytes shared at the end of both strings, rounded down to a char boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    let mut i = 0;
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    while i < ab.len() && i < bb.len() && ab[ab.len() - 1 - i] == bb[bb.len() - 1 - i] {
        i += 1;
    }
    while !a.is_char_boundary(a.len() - i) {
        i -= 1;
    }
    i
}

/// Whether `b` continues a word: letters, digits and underscore, plus any
/// non-ASCII byte so that accented and non-Latin words widen too.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || !b.is_ascii()
}

/// Group files that share a dominant substitution, and mark those files.
///
/// A substitution only earns a cluster once it is repeated enough to be cheaper
/// to state once than to show; below that the file keeps its normal treatment.
fn cluster_substitutions(diff: &Diff, files: &mut [FileFacts]) -> Vec<SubstitutionCluster> {
    let mut by_sub: HashMap<(String, String), Vec<(usize, usize)>> = HashMap::new();
    for (index, file) in diff.files.iter().enumerate() {
        if files[index].kind != Kind::Normal {
            continue;
        }
        if let Some((from, to, count)) = dominant_substitution(file) {
            by_sub.entry((from, to)).or_default().push((index, count));
        }
    }

    let mut clusters: Vec<SubstitutionCluster> = Vec::new();
    // Sort for determinism: the same diff must always produce the same output.
    let mut entries: Vec<_> = by_sub.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for ((from, to), sites) in entries {
        let occurrences: usize = sites.iter().map(|(_, c)| *c).sum();
        if occurrences < MIN_CLUSTER_OCCURRENCES {
            continue;
        }
        let cluster = clusters.len();
        for (index, count) in &sites {
            files[*index].kind = Kind::Substitution {
                cluster,
                occurrences: *count,
            };
        }
        clusters.push(SubstitutionCluster {
            from,
            to,
            occurrences,
            paths: sites
                .iter()
                .map(|(index, _)| files[*index].path.clone())
                .collect(),
        });
    }
    clusters
}

/// One side of the diff flattened into `(file index, trimmed text)`, in order.
fn flatten(diff: &Diff, want: LineKind) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    for (index, file) in diff.files.iter().enumerate() {
        for hunk in &file.hunks {
            for line in &hunk.lines {
                if line.kind == want {
                    let text = line.content.trim();
                    if !text.is_empty() {
                        out.push((index, text));
                    }
                }
            }
        }
    }
    out
}

/// Find runs of lines that were removed in one place and added verbatim in
/// another — a relocation rather than new content.
///
/// Purely textual, so it works on prose and configuration as well as code.
fn find_moves(diff: &Diff) -> Vec<MovedBlock> {
    let removed = flatten(diff, LineKind::Remove);
    let added = flatten(diff, LineKind::Add);
    if removed.len() < MIN_MOVE_LINES || added.len() < MIN_MOVE_LINES {
        return Vec::new();
    }

    let mut positions: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, (_, text)) in added.iter().enumerate() {
        positions.entry(text).or_default().push(index);
    }

    let mut moves = Vec::new();
    let mut i = 0;
    while i < removed.len() {
        let Some(candidates) = positions.get(removed[i].1) else {
            i += 1;
            continue;
        };
        // A line repeated all over the diff anchors nothing; skip it rather than
        // spend time on every occurrence.
        if candidates.len() > MAX_MOVE_CANDIDATES {
            i += 1;
            continue;
        }
        let best = candidates
            .iter()
            .map(|&start| (start, run_length(&removed[i..], &added[start..])))
            .max_by_key(|(_, len)| *len);
        match best {
            Some((start, len)) if len >= MIN_MOVE_LINES => {
                moves.push(MovedBlock {
                    from_path: diff.files[removed[i].0].path.clone(),
                    to_path: diff.files[added[start].0].path.clone(),
                    lines: len,
                });
                i += len;
            }
            _ => i += 1,
        }
    }
    moves
}

/// How many leading lines the two slices share verbatim.
fn run_length(removed: &[(usize, &str)], added: &[(usize, &str)]) -> usize {
    removed
        .iter()
        .zip(added.iter())
        .take_while(|((_, r), (_, a))| r == a)
        .count()
}

/// Mark files whose entire change is accounted for by relocated blocks.
///
/// A file that merely contributes a few moved lines keeps its normal treatment;
/// only one with nothing else to say collapses.
fn mark_moved_files(diff: &Diff, moves: &[MovedBlock], files: &mut [FileFacts]) {
    if moves.is_empty() {
        return;
    }
    let mut moved_out: HashMap<&str, usize> = HashMap::new();
    let mut moved_in: HashMap<&str, usize> = HashMap::new();
    for block in moves {
        *moved_out.entry(block.from_path.as_str()).or_default() += block.lines;
        *moved_in.entry(block.to_path.as_str()).or_default() += block.lines;
    }
    for (index, facts) in files.iter_mut().enumerate() {
        if facts.kind != Kind::Normal {
            continue;
        }
        let path = diff.files[index].path.as_str();
        let out = moved_out.get(path).copied().unwrap_or(0);
        let into = moved_in.get(path).copied().unwrap_or(0);
        // Blank lines are excluded from move matching, so require only that the
        // moved runs account for the bulk of each side.
        let explains = |moved: usize, total: usize| total == 0 || moved * 10 >= total * 9;
        if (out > 0 || into > 0) && explains(out, facts.removed) && explains(into, facts.added) {
            facts.kind = Kind::Moved;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Diff {
        karet_diff::parse(raw).expect("fixture parses")
    }

    /// A one-file diff whose hunk replaces `old` lines with `new` lines.
    fn replace_diff(path: &str, old: &[&str], new: &[&str]) -> String {
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

    #[test]
    fn reindentation_is_whitespace_only() {
        let diff = parse(&replace_diff(
            "a.py",
            &["  def f():", "    return 1"],
            &["    def f():", "        return 1"],
        ));
        assert_eq!(
            classify_file(&diff.files[0]).kind,
            Kind::Reflow {
                terminators_only: false
            }
        );
    }

    #[test]
    fn prose_rewrap_is_whitespace_only_across_line_boundaries() {
        // The lines are re-split at different points, so a line-by-line comparison
        // (what `git diff -w` does) sees every line as changed.
        let diff = parse(&replace_diff(
            "doc.md",
            &["alpha beta gamma", "delta epsilon"],
            &["alpha beta", "gamma delta epsilon"],
        ));
        assert_eq!(
            classify_file(&diff.files[0]).kind,
            Kind::Reflow {
                terminators_only: false
            }
        );
    }

    #[test]
    fn line_ending_change_is_reported_separately() {
        let raw = "diff --git a/f.txt b/f.txt\nindex aaa..bbb 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\n-alpha\n-beta\n+alpha\r\n+beta\r\n";
        let diff = parse(raw);
        assert_eq!(
            classify_file(&diff.files[0]).kind,
            Kind::Reflow {
                terminators_only: true
            }
        );
    }

    #[test]
    fn a_real_edit_is_not_whitespace_only() {
        let diff = parse(&replace_diff("a.rs", &["let x = 1;"], &["let x = 2;"]));
        assert_eq!(classify_file(&diff.files[0]).kind, Kind::Normal);
    }

    #[test]
    fn pure_additions_are_not_whitespace_only() {
        let raw = "diff --git a/a.rs b/a.rs\nindex aaa..bbb 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,0 +1,2 @@\n+one\n+two\n";
        assert_eq!(classify_file(&parse(raw).files[0]).kind, Kind::Normal);
    }

    #[test]
    fn lockfiles_are_generated() {
        assert!(is_generated("Cargo.lock"));
        assert!(is_generated("frontend/package-lock.json"));
        assert!(is_generated("go.sum"));
        assert!(is_generated("dist/app.min.js"));
        assert!(is_generated("dist/app.js.map"));
        assert!(!is_generated("src/lib.rs"));
        assert!(!is_generated("notes.md"));
    }

    #[test]
    fn repeated_substitutions_cluster_across_files() {
        let mut raw = String::new();
        for i in 0..3 {
            raw.push_str(&replace_diff(
                &format!("m{i}.py"),
                &["    return old_name(1)", "    return old_name(2)"],
                &["    return new_name(1)", "    return new_name(2)"],
            ));
        }
        let diff = parse(&raw);
        let analysis = analyze(&diff);
        assert_eq!(analysis.clusters.len(), 1);
        let cluster = &analysis.clusters[0];
        // The whole identifier, not the bare letters that differ: the argument
        // varies per line, so a narrower token would never have clustered.
        assert_eq!(
            (cluster.from.as_str(), cluster.to.as_str()),
            ("old_name", "new_name")
        );
        assert_eq!(cluster.occurrences, 6);
        assert_eq!(cluster.paths.len(), 3);
        assert!(
            analysis
                .files
                .iter()
                .all(|f| matches!(f.kind, Kind::Substitution { .. }))
        );
    }

    #[test]
    fn an_isolated_substitution_does_not_cluster() {
        // One occurrence is cheaper to show than to describe.
        let diff = parse(&replace_diff("a.rs", &["old_name();"], &["new_name();"]));
        let analysis = analyze(&diff);
        assert!(analysis.clusters.is_empty());
        assert_eq!(analysis.files[0].kind, Kind::Normal);
    }

    #[test]
    fn a_relocated_block_is_detected_across_files() {
        let body: Vec<String> = (0..8).map(|i| format!("line number {i}")).collect();
        let refs: Vec<&str> = body.iter().map(String::as_str).collect();
        let mut raw = replace_diff("src/util.rs", &refs, &[]);
        raw.push_str(&replace_diff("src/wrap.rs", &[], &refs));
        let analysis = analyze(&parse(&raw));

        assert_eq!(analysis.moves.len(), 1);
        let moved = &analysis.moves[0];
        assert_eq!(moved.from_path, "src/util.rs");
        assert_eq!(moved.to_path, "src/wrap.rs");
        assert_eq!(moved.lines, 8);
        assert!(analysis.files.iter().all(|f| f.kind == Kind::Moved));
    }

    #[test]
    fn a_short_run_is_not_a_move() {
        let body = ["alpha", "beta"];
        let mut raw = replace_diff("a.rs", &body, &[]);
        raw.push_str(&replace_diff("b.rs", &[], &body));
        assert!(analyze(&parse(&raw)).moves.is_empty());
    }

    #[test]
    fn bulk_data_churn_is_recognized() {
        let old: Vec<String> = (0..150).map(|i| format!("{i},{}", i * 2)).collect();
        let new: Vec<String> = (0..150).map(|i| format!("{i},{}", i * 3)).collect();
        let o: Vec<&str> = old.iter().map(String::as_str).collect();
        let n: Vec<&str> = new.iter().map(String::as_str).collect();
        let diff = parse(&replace_diff("data/rows.csv", &o, &n));
        assert_eq!(classify_file(&diff.files[0]).kind, Kind::BulkData);
    }

    #[test]
    fn a_small_data_change_stays_normal() {
        let diff = parse(&replace_diff("data/rows.csv", &["1,2"], &["1,3"]));
        assert_eq!(classify_file(&diff.files[0]).kind, Kind::Normal);
    }

    #[test]
    fn hunkless_and_binary_files_are_classified() {
        let renamed = parse(
            "diff --git a/a.rs b/b.rs\nsimilarity index 100%\nrename from a.rs\nrename to b.rs\n",
        );
        assert_eq!(classify_file(&renamed.files[0]).kind, Kind::NoContent);

        let binary = parse(
            "diff --git a/x.png b/x.png\nindex aaa..bbb 100644\nBinary files a/x.png and b/x.png differ\n",
        );
        assert_eq!(classify_file(&binary.files[0]).kind, Kind::Binary);
    }

    #[test]
    fn unconditional_collapses_exclude_normal_files() {
        assert!(!Kind::Normal.collapses_unconditionally());
        assert!(Kind::Binary.collapses_unconditionally());
        assert!(Kind::Generated.collapses_unconditionally());
        assert!(
            Kind::Reflow {
                terminators_only: false
            }
            .collapses_unconditionally()
        );
    }
}
