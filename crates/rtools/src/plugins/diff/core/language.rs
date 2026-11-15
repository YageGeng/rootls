//! Guessing the programming language of pasted text, for syntax colors
//! (diff spec §3.1). Pasted text has no file name, so the guess reads the
//! start of the text: a shebang, how data formats begin, and markers typical
//! of each language.

use std::fmt;

use rust_i18n::t;

/// Lowest marker score that counts as a guess; below it the text is plain.
const MIN_SCORE: u32 = 4;

/// A language the diff plugin can color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Plain,
    Json,
    Xml,
    Html,
    Yaml,
    Toml,
    Markdown,
    Rust,
    Python,
    Go,
    Java,
    CSharp,
    C,
    Cpp,
    JavaScript,
    TypeScript,
    Tsx,
    Bash,
    Sql,
    Css,
}

/// Markers of a language with their weights; each counts once if present.
type Markers = &'static [(&'static str, u32)];

/// Markers of the languages scored by their content, in tie-break order.
/// JavaScript stands for its family; TypeScript and TSX are told apart after.
const MARKERS: [(Language, Markers); 11] = [
    (
        Language::Rust,
        &[
            ("fn ", 2),
            ("let mut ", 3),
            ("pub fn ", 3),
            ("impl ", 2),
            ("use std::", 3),
            ("#[derive", 3),
            ("println!", 3),
            ("&self", 2),
            ("-> ", 1),
            ("::", 1),
            ("Result<", 2),
            ("Option<", 2),
            ("&str", 2),
        ],
    ),
    (
        Language::Python,
        &[
            ("def ", 3),
            ("elif ", 3),
            ("__init__", 3),
            ("print(", 2),
            ("):\n", 2),
            ("self.", 1),
            ("None", 1),
            ("import ", 1),
            (":\n", 1),
        ],
    ),
    (
        Language::Go,
        &[
            ("package ", 2),
            ("func ", 3),
            (":= ", 3),
            ("fmt.", 3),
            ("import (", 3),
        ],
    ),
    (
        Language::Java,
        &[
            ("public class ", 3),
            ("public static void ", 3),
            ("System.out.", 3),
            ("import java.", 3),
            ("@Override", 3),
            ("String[]", 2),
        ],
    ),
    (
        Language::CSharp,
        &[
            ("using System", 3),
            ("namespace ", 2),
            ("Console.", 3),
            ("{ get; set; }", 3),
            ("static void Main", 3),
        ],
    ),
    (
        Language::C,
        &[
            ("#include <", 3),
            ("printf(", 3),
            ("int main(", 3),
            ("malloc(", 3),
            ("NULL", 2),
            ("void", 1),
        ],
    ),
    (
        Language::Cpp,
        &[
            ("#include <iostream>", 5),
            ("#include <", 3),
            ("std::", 3),
            ("cout", 3),
            ("template<", 3),
            ("nullptr", 3),
            ("int main(", 2),
        ],
    ),
    (
        Language::JavaScript,
        &[
            ("function ", 2),
            ("const ", 2),
            ("=>", 2),
            ("===", 2),
            ("console.log", 3),
            ("require(", 3),
            ("module.exports", 3),
            ("export default", 2),
            ("document.", 3),
        ],
    ),
    (
        Language::Bash,
        &[
            ("echo ", 2),
            ("fi\n", 3),
            ("then", 2),
            ("done", 2),
            ("; do", 3),
            ("esac", 3),
            ("$(", 2),
            ("${", 2),
            ("\"$", 2),
            ("export ", 2),
        ],
    ),
    (
        Language::Css,
        &[
            ("@media", 4),
            ("color:", 2),
            ("margin:", 2),
            ("padding:", 2),
            ("display:", 2),
            ("border:", 2),
            ("px;", 2),
            ("!important", 3),
        ],
    ),
    (
        Language::Sql,
        // Matched against the lowercased sample.
        &[
            ("select ", 2),
            (" from ", 2),
            (" where ", 2),
            ("insert into", 3),
            ("create table", 4),
            ("primary key", 3),
            (" join ", 2),
            ("group by", 3),
            ("order by", 3),
        ],
    ),
];

/// Markers that turn a JavaScript guess into TypeScript.
const TYPESCRIPT: Markers = &[
    ("interface ", 3),
    (": string", 3),
    (": number", 3),
    (": boolean", 3),
    ("import type", 3),
    ("as const", 3),
    ("readonly ", 2),
    ("enum ", 2),
];
/// Markers of JSX, which turns a TypeScript guess into TSX.
const JSX: Markers = &[("className=", 3), ("return <", 3), ("</div>", 2), ("/>", 1)];

impl Language {
    /// Bytes read from the start of a text; enough to recognize a language,
    /// and cheap enough to guess again on every edit.
    pub const SAMPLE_BYTES: usize = 4096;

    /// Every language, as the language dropdown lists them.
    pub const ALL: [Language; 20] = [
        Language::Plain,
        Language::Bash,
        Language::C,
        Language::Cpp,
        Language::CSharp,
        Language::Css,
        Language::Go,
        Language::Html,
        Language::Java,
        Language::JavaScript,
        Language::Json,
        Language::Markdown,
        Language::Python,
        Language::Rust,
        Language::Sql,
        Language::Toml,
        Language::Tsx,
        Language::TypeScript,
        Language::Xml,
        Language::Yaml,
    ];

    /// Guesses the language from the start of `text`; plain when unsure.
    pub fn detect(text: &str) -> Self {
        Sample::from(text).language()
    }

    /// The name GPUI Kit's highlighter registry knows the language by.
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "text",
            Self::Json => "json",
            Self::Xml => "xml",
            Self::Html => "html",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Markdown => "markdown",
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Bash => "bash",
            Self::Sql => "sql",
            Self::Css => "css",
        }
    }
}

impl fmt::Display for Language {
    /// Writes the language as people name it; plain text in the interface language.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Plain => return f.write_str(&t!("diff.plain_text")),
            Self::Json => "JSON",
            Self::Xml => "XML",
            Self::Html => "HTML",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::CSharp => "C#",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
            Self::Bash => "Bash",
            Self::Sql => "SQL",
            Self::Css => "CSS",
        })
    }
}

/// The start of a text, with leading whitespace and a BOM dropped, that
/// every check reads.
struct Sample<'a>(&'a str);

impl<'a> From<&'a str> for Sample<'a> {
    /// Keeps at most [`Language::SAMPLE_BYTES`], cut at a character boundary.
    fn from(text: &'a str) -> Self {
        let text = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        let mut end = text.len().min(Language::SAMPLE_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Self(&text[..end])
    }
}

impl Sample<'_> {
    /// The guess: a shebang or a data format's opening first, then the
    /// language whose markers score highest.
    fn language(&self) -> Language {
        let text = self.0;
        if let Some(shebang) = text.strip_prefix("#!") {
            let interpreter = shebang.lines().next().unwrap_or_default();
            return if interpreter.contains("python") {
                Language::Python
            } else if ["node", "deno", "bun"]
                .iter()
                .any(|js| interpreter.contains(js))
            {
                Language::JavaScript
            } else if interpreter.contains("sh") {
                Language::Bash
            } else {
                Language::Plain
            };
        }
        match text.chars().next() {
            None => return Language::Plain,
            Some('{') => return Language::Json,
            Some('[') => {
                return if self.toml_header() {
                    Language::Toml
                } else {
                    Language::Json
                };
            }
            Some('<') => return self.markup(),
            _ => {}
        }
        let lower = text.to_lowercase();
        let (mut best, mut best_score) = (Language::Plain, 0);
        let mut consider = |language, score| {
            if score > best_score {
                (best, best_score) = (language, score);
            }
        };
        for (language, markers) in MARKERS {
            let haystack = if language == Language::Sql {
                &lower
            } else {
                text
            };
            let mut score = score_of(haystack, markers);
            if language == Language::JavaScript {
                score += score_of(text, TYPESCRIPT) + score_of(text, JSX);
            }
            if language == Language::Css && text.starts_with('.') {
                score += 2;
            }
            if language == Language::Python && self.python_import() {
                score += 2;
            }
            consider(language, score);
        }
        consider(Language::Yaml, self.shaped_lines(Self::yaml_line));
        consider(Language::Toml, self.shaped_lines(Self::toml_line));
        consider(Language::Markdown, self.markdown());
        if best_score < MIN_SCORE {
            return Language::Plain;
        }
        // JavaScript stands for its family: type annotations make it
        // TypeScript, and markup inside those makes it TSX.
        match best {
            Language::JavaScript if score_of(text, TYPESCRIPT) >= 3 => {
                if score_of(text, JSX) >= 3 {
                    Language::Tsx
                } else {
                    Language::TypeScript
                }
            }
            language => language,
        }
    }

    /// Whether the text opens with a TOML table header like `[package]`
    /// rather than a JSON array.
    fn toml_header(&self) -> bool {
        let first = self.0.lines().next().unwrap_or_default().trim();
        first
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .is_some_and(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
            })
    }

    /// Whether a line imports the way only Python does: `import os` or
    /// `from os import path`, with no quotes or semicolon (JavaScript, Go and
    /// Java imports have one of them).
    fn python_import(&self) -> bool {
        self.0.lines().any(|line| {
            let bare = !line.contains(['"', '\'', ';']);
            bare && (line.starts_with("import ")
                || line.starts_with("from ") && line.contains(" import "))
        })
    }

    /// HTML when the markup uses well-known HTML tags, XML otherwise.
    fn markup(&self) -> Language {
        let lower = self.0.to_lowercase();
        if lower.starts_with("<?xml") {
            return Language::Xml;
        }
        let html = [
            "<!doctype html",
            "<html",
            "<head",
            "<body",
            "<div",
            "<span",
            "<p>",
            "<a ",
            "<script",
        ];
        if html.iter().any(|tag| lower.contains(tag)) {
            Language::Html
        } else {
            Language::Xml
        }
    }

    /// Two points per line of the given shape, when nearly every line has it.
    fn shaped_lines(&self, shaped: fn(&str) -> bool) -> u32 {
        let lines: Vec<&str> = self
            .0
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let matching = lines.iter().filter(|line| shaped(line)).count();
        if matching >= 2 && matching * 5 >= lines.len() * 4 {
            2 * matching as u32
        } else {
            0
        }
    }

    /// A YAML line: `key:` or `key: value`, a list item, or a document marker.
    fn yaml_line(line: &str) -> bool {
        if line == "---" || line.starts_with("- ") {
            return true;
        }
        line.split_once(':').is_some_and(|(key, value)| {
            is_key(key) && (value.is_empty() || value.starts_with(' ')) && !line.ends_with(';')
        })
    }

    /// A TOML line: `key = value`, or a table header.
    fn toml_line(line: &str) -> bool {
        if line.starts_with('[') && line.ends_with(']') {
            return true;
        }
        line.split_once('=')
            .is_some_and(|(key, value)| is_key(key.trim()) && !value.trim().is_empty())
    }

    /// Markdown markers: headings, code fences, emphasis, links and list items.
    fn markdown(&self) -> u32 {
        let mut score = score_of(self.0, &[("**", 2), ("](", 2)]);
        for line in self.0.lines() {
            score += if line.starts_with("```") {
                3
            } else if line.starts_with('#') && line.trim_start_matches('#').starts_with(' ') {
                2
            } else if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("> ") {
                1
            } else {
                0
            };
        }
        score
    }
}

/// The weights of the markers found in `text`, each counted once.
fn score_of(text: &str, markers: Markers) -> u32 {
    markers
        .iter()
        .filter(|(marker, _)| text.contains(marker))
        .map(|(_, weight)| weight)
        .sum()
}

/// Whether `key` is a bare identifier key like `name` or `build-dir`.
fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}
