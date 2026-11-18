use gpui_kit::assets::IconName;
use gpui_kit::{App, Window};
use rtools::plugin::{
    ContentKind, ContentMatch, ContentRecognizer, Launch, MatchKind, Plugin, PluginManifest,
    PluginMatch, PluginRegistry, PluginView,
};

struct Fake(PluginManifest, Option<ContentRecognizer>);

impl Plugin for Fake {
    fn manifest(&self) -> &PluginManifest {
        &self.0
    }

    /// Returns the test plugin's optional recognition function.
    fn recognizer(&self) -> Option<ContentRecognizer> {
        self.1
    }

    fn open(&self, _: Launch, _: &mut Window, _: &mut App) -> PluginView {
        unreachable!("not opened in tests")
    }
}

fn registry() -> PluginRegistry {
    [
        Box::new(Fake(
            PluginManifest {
                id: "json",
                title: "JSON 编辑器",
                description: "",
                icon: IconName::Braces,
                triggers: &["json"],
                keywords: &["json"],
            },
            Some(|s| {
                s.starts_with('{').then_some(ContentMatch {
                    kind: ContentKind::Recognized,
                    message_key: "json.matches.json",
                })
            }),
        )) as Box<dyn Plugin>,
        Box::new(Fake(
            PluginManifest {
                id: "calc",
                title: "计算器",
                description: "",
                icon: IconName::Calculator,
                triggers: &[],
                keywords: &["calc", "math"],
            },
            None,
        )),
    ]
    .into_iter()
    .collect()
}

fn ids(r: &PluginRegistry, m: &[PluginMatch]) -> Vec<&'static str> {
    m.iter().map(|m| r.plugin(m.index).manifest().id).collect()
}

#[test]
fn empty_query_lists_everything_in_order() {
    let r = registry();
    assert_eq!(ids(&r, &r.search("")), ["json", "calc"]);
}

#[test]
fn keywords_and_titles_match_case_insensitively() {
    let r = registry();
    assert_eq!(ids(&r, &r.search("JS")), ["json"]);
    assert_eq!(ids(&r, &r.search("计算")), ["calc"]);
    assert_eq!(ids(&r, &r.search("MATH")), ["calc"]);
}

#[test]
fn content_matches_follow_keyword_matches_once() {
    let r = registry();
    let m = r.search("{\"a\":1}");
    assert_eq!(ids(&r, &m), ["json"]);
    assert_eq!(
        m[0].kind,
        MatchKind::Content(ContentMatch {
            kind: ContentKind::Recognized,
            message_key: "json.matches.json",
        })
    );
}

#[test]
fn search_content_ignores_keywords() {
    let r = registry();
    assert!(r.search_content("json").is_empty());
    assert_eq!(ids(&r, &r.search_content("{")), ["json"]);
}

/// Command words open an existing view without turning the search text into its document.
#[test]
fn builtin_triggers_open_without_passing_the_alias_as_content() {
    let r: PluginRegistry = rtools::plugins::builtins().into_iter().collect();
    for (query, id) in [
        (" FY ", "translate"),
        ("trans", "translate"),
        ("jqb", "clipboard"),
        ("JSON", "json"),
        ("diff", "diff"),
        ("剪贴", "clipboard"),
        ("fanyi", "translate"),
    ] {
        let hits = r.search(query);
        assert_eq!(
            (r.plugin(hits[0].index).manifest().id, &hits[0].kind),
            (id, &MatchKind::Keyword),
            "{query}"
        );
    }
}

/// A developer must resolve ambiguous trigger declarations before shipping a new built-in.
#[test]
fn builtin_triggers_are_nonempty_and_unique() {
    let mut owners = std::collections::HashMap::new();
    for plugin in rtools::plugins::builtins() {
        let manifest = plugin.manifest();
        for word in manifest.triggers {
            let word = word.trim().to_lowercase();
            assert!(!word.is_empty(), "empty trigger in {}", manifest.id);
            if let Some(other) = owners.insert(word.clone(), manifest.id) {
                panic!(
                    "trigger {word:?} is declared by {other} and {}",
                    manifest.id
                );
            }
        }
    }
}

/// Exact, prefix and substring discovery matches keep their order within each rank.
#[test]
fn metadata_ranks_are_stable_and_content_hints_do_not_claim_validity() {
    let r: PluginRegistry = ["xxxray", "xray-two", "xray", "xray-one"]
        .map(|id| {
            Box::new(Fake(
                PluginManifest {
                    id,
                    title: "title",
                    description: "",
                    icon: IconName::Calculator,
                    triggers: &[],
                    keywords: &[],
                },
                None,
            )) as Box<dyn Plugin>
        })
        .into_iter()
        .collect();
    assert_eq!(
        ids(&r, &r.search("xray")),
        ["xray", "xray-two", "xray-one", "xxxray"]
    );
    let r: PluginRegistry = rtools::plugins::builtins().into_iter().collect();
    let valid = &r.search_content(r#"{"a":1}"#)[0].kind;
    let broken = &r.search_content(r#"{"a":1,"a":2}"#)[0].kind;
    assert!(matches!((valid, broken), (MatchKind::Content(a), MatchKind::Content(b)) if a != b));
    assert_eq!(r.search(""), r.search("\u{feff}  \t"));
}

/// Structured inputs win over universal text handlers regardless of registration order.
#[test]
fn json_and_repair_candidates_precede_plain_text_even_when_registered_last() {
    use rtools::plugins::{json::ui::JsonPlugin, translate::TranslatePlugin};
    let r: PluginRegistry = [
        Box::new(TranslatePlugin) as Box<dyn Plugin>,
        Box::new(JsonPlugin),
    ]
    .into_iter()
    .collect();
    for input in [
        r#"{"text":"fy"}"#,
        "[1,2]",
        r#""fy""#,
        "123",
        "-1.2e+3",
        "true",
        "false",
        "null",
        "\u{feff}{}",
        "{\"a\":",
        r#"{"a":1,"a":2}"#,
    ] {
        let hits = r.search(input);
        assert_eq!(ids(&r, &hits), ["json", "translate"], "{input}");
        assert!(matches!(&hits[0].kind, MatchKind::Content(_)));
    }
    assert_eq!(
        ids(&r, &r.search("Please translate this sentence.")),
        ["translate"]
    );
    assert_eq!(ids(&r, &r.search("true story")), ["translate"]);
}
