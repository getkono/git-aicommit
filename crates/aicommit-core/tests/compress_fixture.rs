//! Compression against a real `git diff`, rather than a hand-written one.
//!
//! `tests/fixtures/mixed-change.patch` was captured from
//! `git diff --cached -M30% -l5000` over a scratch repository built to contain, in
//! one change: a symbol renamed across several files, a genuine one-line code fix,
//! a CSS reindent, a Markdown re-wrap, a regenerated lockfile, a binary file, a
//! `chmod +x`, and a 100%-similar rename. Synthetic fixtures miss the details git
//! actually emits — `index` lines, mode lines, similarity headers — so this is the
//! test that would catch a parser or renderer that only works on tidy input.

use aicommit_core::{CompressOptions, Detail, compress_diff};

const FIXTURE: &str = include_str!("fixtures/mixed-change.patch");

/// Every path the fixture's diff touches, in the order git wrote them.
const PATHS: [&str; 10] = [
    "Cargo.lock",
    "NOTES.md",
    "logo.bin",
    "run.sh",
    "src/auth.rs",
    "src/helper.rs",
    "src/mod0.rs",
    "src/mod1.rs",
    "src/mod2.rs",
    "style.css",
];

#[test]
fn no_file_is_ever_dropped() {
    // The guarantee that separates this from truncation: even at a budget far
    // below the floor, every changed file is still named.
    for max_bytes in [50, 500, 2_000, 60_000] {
        let (out, report) = compress_diff(FIXTURE, &CompressOptions::new(max_bytes));
        assert_eq!(
            report.files.len(),
            PATHS.len(),
            "budget {max_bytes} lost a file from the report"
        );
        for path in PATHS {
            assert!(
                out.contains(path),
                "budget {max_bytes} dropped {path} from the output"
            );
        }
    }
}

#[test]
fn the_output_is_still_a_unified_diff() {
    let (out, _) = compress_diff(FIXTURE, &CompressOptions::new(1_500));
    // Round-tripping through a diff parser is what keeps the format familiar to a
    // model; a bespoke format would parse as nothing.
    let reparsed = karet_diff::parse(&out).expect("compressed output parses as a diff");
    let paths: Vec<&str> = reparsed.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, PATHS);
}

#[test]
fn the_uninformative_files_are_the_ones_collapsed() {
    let (out, report) = compress_diff(FIXTURE, &CompressOptions::new(60_000));
    let reason = |path: &str| -> String {
        report
            .files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.reason.clone())
            .unwrap_or_else(|| panic!("{path} missing from the report"))
    };

    assert_eq!(reason("style.css"), "whitespace-only reformat");
    assert_eq!(reason("NOTES.md"), "whitespace-only reformat");
    assert_eq!(reason("Cargo.lock"), "generated file");
    assert_eq!(reason("logo.bin"), "binary");
    // A pure rename and a chmod have no body to show in the first place.
    assert_eq!(reason("src/helper.rs"), "no content change");
    assert_eq!(reason("run.sh"), "no content change");

    // The identifier rename is stated once rather than at each of its sites.
    assert_eq!(report.clusters.len(), 1, "{:?}", report.clusters);
    let cluster = &report.clusters[0];
    assert_eq!(
        (cluster.from.as_str(), cluster.to.as_str()),
        ("old_name", "new_name")
    );
    assert_eq!(cluster.paths.len(), 3);

    // The one real code change is the one shown in full.
    assert_eq!(reason("src/auth.rs"), "shown in full");
    assert!(out.contains("+    if tok.expired() || tok.revoked() {"));
}

#[test]
fn the_rename_and_mode_change_survive_as_facts() {
    let (out, _) = compress_diff(FIXTURE, &CompressOptions::new(60_000));
    // These are the whole content of their respective changes, so losing them
    // would leave the model with a file it cannot say anything about.
    assert!(out.contains("rename from old_helper.rs"), "{out}");
    assert!(out.contains("rename to src/helper.rs"), "{out}");
    assert!(out.contains("old mode 100644"), "{out}");
    assert!(out.contains("new mode 100755"), "{out}");
}

#[test]
fn a_generous_budget_still_beats_the_raw_diff() {
    let (out, report) = compress_diff(FIXTURE, &CompressOptions::new(60_000));
    assert!(report.compressed);
    assert!(
        out.len() * 3 < FIXTURE.len(),
        "expected a large saving, got {} from {}",
        out.len(),
        FIXTURE.len()
    );
}

#[test]
fn detail_degrades_as_the_budget_shrinks() {
    let detail_sum = |max_bytes: usize| -> usize {
        let (_, report) = compress_diff(FIXTURE, &CompressOptions::new(max_bytes));
        report
            .files
            .iter()
            .map(|f| match f.detail {
                Detail::Ledger => 0,
                Detail::Outline => 1,
                Detail::Condensed => 2,
                Detail::Full => 3,
            })
            .sum()
    };
    let (tight, loose) = (detail_sum(50), detail_sum(60_000));
    assert!(
        tight < loose,
        "a tighter budget must buy less detail: {tight} vs {loose}"
    );
}
