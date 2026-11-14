use gpui_kit::assets::IconName;
use gpui_kit::{App, SharedString, Window};
use rtools::plugin::{
    Launch, MatchKind, Plugin, PluginManifest, PluginMatch, PluginRegistry, PluginView,
};

struct Fake(PluginManifest, Option<&'static str>);

impl Plugin for Fake {
    fn manifest(&self) -> &PluginManifest {
        &self.0
    }

    fn accepts(&self, input: &str) -> Option<SharedString> {
        self.1
            .filter(|_| input.starts_with('{'))
            .map(SharedString::from)
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
                keywords: &["json"],
            },
            Some("识别为 JSON"),
        )) as Box<dyn Plugin>,
        Box::new(Fake(
            PluginManifest {
                id: "calc",
                title: "计算器",
                description: "",
                icon: IconName::Calculator,
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
    assert_eq!(m[0].kind, MatchKind::Content("识别为 JSON".into()));
}

#[test]
fn search_content_ignores_keywords() {
    let r = registry();
    assert!(r.search_content("json").is_empty());
    assert_eq!(ids(&r, &r.search_content("{")), ["json"]);
}
