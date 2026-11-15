use std::time::{Duration, Instant};

use gpui_kit::component::diff::DiffFile;
use rtools::plugins::diff::core::{CompareOptions, Comparison, FormatJson, Side, Texts};

fn compare(original: &str, modified: &str, options: CompareOptions) -> Comparison {
    Texts { original, modified }.compare(options)
}

const PLAIN: CompareOptions = CompareOptions {
    ignore_whitespace: false,
    ignore_case: false,
    format_json: false,
};
const WS: CompareOptions = CompareOptions {
    ignore_whitespace: true,
    ..PLAIN
};
const CASE: CompareOptions = CompareOptions {
    ignore_case: true,
    ..PLAIN
};
const JSON: CompareOptions = CompareOptions {
    format_json: true,
    ..PLAIN
};

/// `(additions, deletions)` of a comparison.
fn counts(c: &Comparison) -> (usize, usize) {
    (c.additions, c.deletions)
}

/// The patch must parse as one file whose counts agree with the comparison.
fn assert_parses(c: &Comparison) {
    let files = DiffFile::parse(&c.patch).unwrap();
    assert_eq!(files.len(), 1, "{}", c.patch);
    assert_eq!((files[0].additions(), files[0].deletions()), counts(c));
}

#[test]
fn changed_line_writes_one_full_context_hunk() {
    let c = compare("a\nb\nc", "a\nB\nc", PLAIN);
    assert_eq!(
        c.patch,
        "--- original\n+++ modified\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"
    );
    assert_eq!(counts(&c), (1, 1));
    assert_eq!(c.format_json, FormatJson::Off);
    assert!(!c.json);
}

#[test]
fn insertion_and_deletion_are_counted() {
    let c = compare("a\nb", "b\nc", PLAIN);
    assert!(
        c.patch.ends_with("@@ -1,2 +1,2 @@\n-a\n b\n+c\n"),
        "{}",
        c.patch
    );
    assert_eq!(counts(&c), (1, 1));
}

#[test]
fn identical_texts_have_no_changes() {
    assert_eq!(counts(&compare("x\ny", "x\ny", PLAIN)), (0, 0));
}

#[test]
fn line_endings_and_final_newline_are_not_differences() {
    let c = compare("a\r\nb\r\n", "a\nb", PLAIN);
    assert_eq!(counts(&c), (0, 0));
    assert!(
        c.patch.ends_with("@@ -1,2 +1,2 @@\n a\n b\n"),
        "{}",
        c.patch
    );
}

#[test]
fn ignore_whitespace_shows_the_modified_text() {
    let c = compare("if x {\n  y\n}", "if x {\n\ty\n}", WS);
    assert_eq!(counts(&c), (0, 0));
    assert!(c.patch.contains("\n \ty\n"), "{}", c.patch);
    assert_eq!(counts(&compare("a b", "ab", WS)), (0, 0));
    assert_eq!(counts(&compare("a b", "ab", PLAIN)), (1, 1));
}

#[test]
fn ignore_whitespace_covers_unicode_spaces() {
    assert_eq!(counts(&compare("a\u{3000}b", "a\u{a0}b\t", WS)), (0, 0));
    assert_eq!(counts(&compare("a\u{3000}b", "a\u{a0}b\t", PLAIN)), (1, 1));
}

#[test]
fn ignore_case_compares_lowercased_lines() {
    assert_eq!(counts(&compare("Hello ÄÖ", "hello äö", CASE)), (0, 0));
    assert_eq!(counts(&compare("Hello ÄÖ", "hello äö", PLAIN)), (1, 1));
}

#[test]
fn switches_combine() {
    let both = CompareOptions {
        ignore_case: true,
        ..WS
    };
    assert_eq!(counts(&compare("  FOO", "foo", both)), (0, 0));
    assert_eq!(counts(&compare("  FOO", "foo", WS)), (1, 1));
    assert_eq!(counts(&compare("  FOO", "foo", CASE)), (1, 1));
}

#[test]
fn format_json_ignores_key_order_and_layout() {
    let c = compare(
        r#"{"b":1,"a":[{"d":2,"c":3}]}"#,
        "{\n  \"a\": [ {\"c\": 3, \"d\": 2} ],\n  \"b\": 1\n}",
        JSON,
    );
    assert_eq!(c.format_json, FormatJson::Applied);
    assert_eq!(counts(&c), (0, 0));
    assert!(c.json);
    assert!(c.patch.contains("\n     \"a\": [\n"), "{}", c.patch);
}

#[test]
fn format_json_keeps_number_spelling() {
    let c = compare(
        r#"{"n": 1.50, "big": 12345678901234567890}"#,
        r#"{"big":12345678901234567890,"n":1.5}"#,
        JSON,
    );
    assert_eq!(c.format_json, FormatJson::Applied);
    assert_eq!(counts(&c), (1, 1));
    assert!(c.patch.contains("-    \"n\": 1.50\n"), "{}", c.patch);
    assert!(c.patch.contains("+    \"n\": 1.5\n"), "{}", c.patch);
    assert!(
        c.patch.contains("     \"big\": 12345678901234567890,\n"),
        "{}",
        c.patch
    );
}

#[test]
fn format_json_unavailable_names_the_first_invalid_side() {
    let c = compare("{bad", "{}", JSON);
    assert_eq!(c.format_json, FormatJson::Unavailable(Side::Original));
    assert!(!c.json);
    assert!(c.patch.contains("-{bad\n"), "{}", c.patch);
    assert_eq!(
        compare("{}", "[", JSON).format_json,
        FormatJson::Unavailable(Side::Modified)
    );
    assert_eq!(
        compare("x", "y", JSON).format_json,
        FormatJson::Unavailable(Side::Original)
    );
}

#[test]
fn json_flag_without_formatting() {
    let c = compare(r#"{"a":1}"#, r#"{"a":2}"#, PLAIN);
    assert!(c.json);
    assert_eq!(c.format_json, FormatJson::Off);
    assert!(!compare("a", "{}", PLAIN).json);
}

#[test]
fn empty_sides_are_reported() {
    let c = compare("", "a", PLAIN);
    assert!(c.original_empty);
    assert!(!c.modified_empty);
    assert!(c.patch.ends_with("@@ -0,0 +1,1 @@\n+a\n"), "{}", c.patch);
    assert_eq!(c.additions, 1);

    let c = compare("\n", "", PLAIN);
    assert!(c.original_empty && c.modified_empty);

    let c = compare("  ", "", PLAIN);
    assert!(!c.original_empty, "whitespace is not empty");
    assert!(c.modified_empty);
}

#[test]
fn patch_syntax_lines_round_trip_through_diff_file_parse() {
    let lines = [
        "--- a",
        "+++ b",
        "@@ -1 +1 @@",
        "\\ No newline at end of file",
        " lead",
        "+plus",
        "-minus",
        "",
        "tail",
    ];
    let original = lines.join("\n");
    let mut changed: Vec<&str> = lines
        .iter()
        .filter(|line| **line != "@@ -1 +1 @@")
        .map(|line| if *line == "+plus" { "+PLUS" } else { line })
        .collect();
    changed.push("--- c");
    let modified = changed.join("\n");

    let pair = compare(&original, &modified, PLAIN);
    assert_eq!(counts(&pair), (2, 2));
    assert_parses(&pair);
    assert_parses(&compare("", &original, PLAIN));
    assert_parses(&compare(&original, "", PLAIN));
}

#[test]
fn large_inputs_finish_with_consistent_counts() {
    let original: Vec<String> = (0..20_000).map(|i| format!("line {i}")).collect();
    let modified: Vec<String> = (0..20_000)
        .map(|i| format!("line {}", i * 7919 % 20_011))
        .collect();
    let started = Instant::now();
    let c = compare(&original.join("\n"), &modified.join("\n"), PLAIN);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(c.additions, c.deletions);
    assert!(c.additions > 0);
    assert_parses(&c);
}

#[test]
fn a_leading_bom_is_not_a_difference() {
    let c = compare("\u{feff}{\"a\": 1}", "{\"a\": 1}", PLAIN);
    assert_eq!(counts(&c), (0, 0));
    assert!(c.json, "a BOM must not hide that both sides are JSON");
    assert_eq!(
        compare("\u{feff}{\"b\":1,\"a\":2}", "{\"a\":2,\"b\":1}", JSON).format_json,
        FormatJson::Applied
    );
}

#[test]
fn pathological_input_stops_near_the_deadline() {
    // Lines drawn from a small alphabet match everywhere, which keeps Myers
    // searching until the 2 s deadline cuts it short.
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut line = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        format!("{}", seed % 50)
    };
    let original: Vec<String> = (0..200_000).map(|_| line()).collect();
    let modified: Vec<String> = (0..200_000).map(|_| line()).collect();
    let started = Instant::now();
    let c = compare(&original.join("\n"), &modified.join("\n"), PLAIN);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(8), "{elapsed:?}");
    assert_eq!(c.additions, c.deletions);
    assert_parses(&c);
}
