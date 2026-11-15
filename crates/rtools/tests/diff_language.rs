use rtools::plugins::diff::core::Language;

/// Every snippet must be detected as its language.
#[track_caller]
fn assert_detects(language: Language, samples: &[&str]) {
    for sample in samples {
        assert_eq!(Language::detect(sample), language, "{sample:?}");
    }
}

#[test]
fn detects_data_formats() {
    assert_detects(
        Language::Json,
        &[
            "{\"a\": 1}",
            "  [1, 2, {\"b\": null}]",
            "\u{feff}{}",
            "[\n  \"x\"\n]",
        ],
    );
    assert_detects(
        Language::Xml,
        &[
            "<?xml version=\"1.0\"?>\n<root><a>1</a></root>",
            "<config>\n  <item id=\"1\"/>\n</config>",
        ],
    );
    assert_detects(
        Language::Html,
        &[
            "<!DOCTYPE html>\n<html><body><div>hi</div></body></html>",
            "<div class=\"card\">\n  <p>hi</p>\n</div>",
        ],
    );
    assert_detects(
        Language::Yaml,
        &[
            "name: rtools\nversion: 1\ntags:\n  - a\n  - b\n",
            "---\nservices:\n  web:\n    image: nginx\n",
        ],
    );
    assert_detects(
        Language::Toml,
        &[
            "[package]\nname = \"rtools\"\nversion = \"0.1.0\"\n",
            "name = \"x\"\n\n[dependencies]\nserde = \"1\"\n",
        ],
    );
    assert_detects(
        Language::Markdown,
        &[
            "# Title\n\nSome **bold** text and a [link](https://x.dev).\n\n- item\n",
            "## Install\n\n```bash\ncargo build\n```\n",
        ],
    );
}

#[test]
fn detects_programming_languages() {
    assert_detects(
        Language::Rust,
        &[
            "fn main() {\n    let mut x = 1;\n    println!(\"{x}\");\n}",
            "use std::fs;\n\npub fn read(path: &str) -> Result<String, io::Error> {\n    fs::read_to_string(path)\n}",
        ],
    );
    assert_detects(
        Language::Python,
        &[
            "def add(a, b):\n    return a + b\n",
            "import os\nprint(os.getcwd())\n",
            "#!/usr/bin/env python3\nprint(1)\n",
        ],
    );
    assert_detects(
        Language::Go,
        &["package main\n\nimport \"fmt\"\n\nfunc main() {\n\tx := 1\n\tfmt.Println(x)\n}"],
    );
    assert_detects(
        Language::Java,
        &[
            "public class Main {\n    public static void main(String[] args) {\n        System.out.println(1);\n    }\n}",
        ],
    );
    assert_detects(
        Language::CSharp,
        &[
            "using System;\n\nnamespace App {\n    class P {\n        static void Main() { Console.WriteLine(1); }\n    }\n}",
        ],
    );
    assert_detects(
        Language::C,
        &["#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\\n\");\n    return 0;\n}"],
    );
    assert_detects(
        Language::Cpp,
        &["#include <iostream>\n\nint main() {\n    std::cout << 1 << std::endl;\n}"],
    );
    assert_detects(
        Language::JavaScript,
        &[
            "const add = (a, b) => a + b;\nconsole.log(add(1, 2));\n",
            "function hello() {\n  return document.title;\n}\nmodule.exports = hello;\n",
        ],
    );
    assert_detects(
        Language::TypeScript,
        &[
            "interface User {\n  name: string;\n  age: number;\n}\nconst u: User = { name: 'a', age: 1 };\n",
        ],
    );
    assert_detects(
        Language::Tsx,
        &[
            "export function App(props: { title: string }) {\n  return <div className=\"app\">{props.title}</div>;\n}\n",
        ],
    );
    assert_detects(
        Language::Bash,
        &[
            "#!/bin/bash\necho hi\n",
            "for f in *.txt; do\n  echo \"$f\"\ndone\n",
            "if [ -z \"$HOME\" ]; then\n  export X=1\nfi\n",
        ],
    );
    assert_detects(
        Language::Sql,
        &[
            "SELECT id, name FROM users WHERE id = 1;",
            "create table users (\n  id integer primary key\n);\ninsert into users values (1);",
        ],
    );
    assert_detects(
        Language::Css,
        &[
            ".btn {\n  color: red;\n  margin: 0 auto;\n}\n",
            "@media (max-width: 600px) {\n  body { padding: 0; }\n}\n",
        ],
    );
}

#[test]
fn falls_back_to_plain_text() {
    assert_detects(
        Language::Plain,
        &[
            "",
            "   \n",
            "hello world\nsecond line",
            "/home/isbest/.local/bin/rtools",
            "Meeting notes: call Bob at 3pm",
        ],
    );
}

#[test]
fn names_follow_the_highlighter_registry() {
    assert_eq!(Language::Plain.name(), "text");
    assert_eq!(Language::CSharp.name(), "csharp");
    assert_eq!(Language::Cpp.name(), "cpp");
    assert_eq!(Language::Tsx.name(), "tsx");
    // The dropdown lists every language once, plain text first.
    assert_eq!(Language::ALL[0], Language::Plain);
    assert_eq!(Language::ALL.len(), 20);
}

#[test]
fn only_the_start_of_huge_text_is_read() {
    // Rust at the start, then megabytes of noise: the answer comes from the start.
    let huge = format!(
        "fn main() {{\n    let mut x = 1;\n}}\n{}",
        "noise line\n".repeat(200_000)
    );
    assert_eq!(Language::detect(&huge), Language::Rust);
}
