//! Line comparison of two texts, written as one unified patch (diff spec §4.2).

use std::borrow::Cow;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::IgnoredAny;
use serde_json::Value;
use serde_json::ser::{PrettyFormatter, Serializer};
use similar::{Algorithm, DiffTag};

/// Longest time the line diff may search for a minimal result; past it the
/// result is still correct, only coarser.
const DEADLINE: Duration = Duration::from_secs(2);

/// Comparison switches that change which lines count as equal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompareOptions {
    /// Drop every whitespace character before comparing a line (`git diff -w`).
    pub ignore_whitespace: bool,
    /// Compare lines lowercased.
    pub ignore_case: bool,
    /// Compare both sides as key-sorted, pretty-printed JSON when both parse.
    pub format_json: bool,
}

/// The two texts being compared.
pub struct Texts<'a> {
    pub original: &'a str,
    pub modified: &'a str,
}

/// Outcome of one comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// Unified patch with one hunk covering every line of both texts.
    pub patch: String,
    /// `+` lines in the patch.
    pub additions: usize,
    /// `-` lines in the patch.
    pub deletions: usize,
    /// Both sides parse as JSON (after formatting, when applied).
    pub json: bool,
    /// What became of the "format JSON first" switch.
    pub format_json: FormatJson,
    /// The original has no characters once normalized.
    pub original_empty: bool,
    /// The modified text has no characters once normalized.
    pub modified_empty: bool,
}

/// What became of the "format JSON first" switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatJson {
    /// The switch is off.
    Off,
    /// Both sides were compared as formatted JSON.
    Applied,
    /// This side (the first one checked that failed) is not JSON, so the raw
    /// texts were compared.
    Unavailable(Side),
}

/// One side of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Original,
    Modified,
}

impl Texts<'_> {
    /// Compares the texts under `options`.
    pub fn compare(&self, options: CompareOptions) -> Comparison {
        let mut pair = Pair {
            original: Normalized::from(self.original),
            modified: Normalized::from(self.modified),
        };
        let format_json = if options.format_json {
            pair.format_json()
        } else {
            FormatJson::Off
        };
        let json = match format_json {
            FormatJson::Applied => true,
            // One side failed to parse just now.
            FormatJson::Unavailable(_) => false,
            FormatJson::Off => pair.is_json(),
        };
        let old = pair.original.lines();
        let new = pair.modified.lines();
        let mut comparison = Comparison {
            patch: String::with_capacity(pair.original.0.len() + pair.modified.0.len() + 64),
            additions: 0,
            deletions: 0,
            json,
            format_json,
            original_empty: old.is_empty(),
            modified_empty: new.is_empty(),
        };
        comparison.write_patch(options, &old, &new);
        comparison
    }
}

impl CompareOptions {
    /// The text `line` is compared by: without whitespace and/or lowercased.
    fn key<'a>(&self, line: &'a str) -> Cow<'a, str> {
        let mut key = Cow::Borrowed(line);
        if self.ignore_whitespace && key.contains(char::is_whitespace) {
            key = Cow::Owned(key.chars().filter(|c| !c.is_whitespace()).collect());
        }
        if self.ignore_case {
            key = Cow::Owned(key.to_lowercase());
        }
        key
    }
}

impl Comparison {
    /// Diffs the comparison keys of `old` and `new`, then writes the patch
    /// from the untouched lines and counts its `+` / `-` lines.
    fn write_patch(&mut self, options: CompareOptions, old: &[&str], new: &[&str]) {
        let old_keys: Vec<Cow<'_, str>> = old.iter().map(|&line| options.key(line)).collect();
        let new_keys: Vec<Cow<'_, str>> = new.iter().map(|&line| options.key(line)).collect();
        let ops = similar::capture_diff_slices_deadline(
            Algorithm::Myers,
            &old_keys,
            &new_keys,
            Some(Instant::now() + DEADLINE),
        );
        // A side without lines starts at 0, as `diff -u` writes it.
        let start = |lines: &[&str]| usize::from(!lines.is_empty());
        // Writing into a `String` cannot fail.
        let _ = write!(
            self.patch,
            "--- original\n+++ modified\n@@ -{},{} +{},{} @@\n",
            start(old),
            old.len(),
            start(new),
            new.len()
        );
        let write = |patch: &mut String, tag: char, lines: &[&str]| {
            for line in lines {
                patch.push(tag);
                patch.push_str(line);
                patch.push('\n');
            }
        };
        for op in ops {
            let (tag, old_range, new_range) = op.as_tag_tuple();
            if tag == DiffTag::Equal {
                // Equal keys may still differ in whitespace or case; like
                // `git diff -w`, the context line shows the modified text.
                write(&mut self.patch, ' ', &new[new_range]);
            } else {
                // Delete and Insert leave the other range empty; Replace
                // lists its removed lines before its added ones.
                self.deletions += old_range.len();
                write(&mut self.patch, '-', &old[old_range]);
                self.additions += new_range.len();
                write(&mut self.patch, '+', &new[new_range]);
            }
        }
    }
}

/// Both sides of one comparison, normalized.
struct Pair<'a> {
    original: Normalized<'a>,
    modified: Normalized<'a>,
}

impl Pair<'_> {
    /// Replaces both sides with formatted JSON when both parse; otherwise
    /// leaves them as they are and names the first side that failed.
    fn format_json(&mut self) -> FormatJson {
        let Ok(original) = serde_json::from_str::<Value>(&self.original.0) else {
            return FormatJson::Unavailable(Side::Original);
        };
        let Ok(modified) = serde_json::from_str::<Value>(&self.modified.0) else {
            return FormatJson::Unavailable(Side::Modified);
        };
        self.original = Normalized::from(original);
        self.modified = Normalized::from(modified);
        FormatJson::Applied
    }

    /// Whether both sides parse as JSON; validates without building values.
    fn is_json(&self) -> bool {
        [&self.original, &self.modified]
            .iter()
            .all(|side| serde_json::from_str::<IgnoredAny>(&side.0).is_ok())
    }
}

/// One side without a leading byte order mark, with `\r\n` read as `\n` and
/// without its final line break, so none of them counts as a difference.
struct Normalized<'a>(Cow<'a, str>);

impl<'a> From<&'a str> for Normalized<'a> {
    /// Drops a BOM (invisible, and not JSON) and one final line break, then
    /// unifies the remaining line breaks.
    fn from(text: &'a str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let text = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        Self(if text.contains("\r\n") {
            Cow::Owned(text.replace("\r\n", "\n"))
        } else {
            Cow::Borrowed(text)
        })
    }
}

impl From<Value> for Normalized<'_> {
    /// Pretty-prints the value with every object's keys sorted and a 4-space
    /// indent; the output has no `\r` and no final line break, so it is
    /// already normalized. Numbers keep their spelling (`arbitrary_precision`).
    fn from(mut value: Value) -> Self {
        value.sort_all_objects();
        let mut out = Vec::new();
        let mut serializer =
            Serializer::with_formatter(&mut out, PrettyFormatter::with_indent(b"    "));
        value
            .serialize(&mut serializer)
            .expect("serializing a JSON value into memory cannot fail");
        Self(Cow::Owned(
            String::from_utf8(out).expect("serde_json writes UTF-8"),
        ))
    }
}

impl Normalized<'_> {
    /// The text's lines; an empty text has none (a blank line is still a line).
    fn lines(&self) -> Vec<&str> {
        if self.0.is_empty() {
            Vec::new()
        } else {
            self.0.split('\n').collect()
        }
    }
}
