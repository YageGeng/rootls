//! The only test in this binary, because the interface language is process-global.

use rtools::plugins::json::core::{InputKind, JsonError, Rewrite};
use rtools::{plugin::Plugin, plugins::translate::TranslatePlugin};

/// Plugin and error labels follow the selected interface locale.
#[test]
fn messages_follow_the_interface_language() {
    rust_i18n::set_locale("zh-CN");
    // The parser's detail text is captured when the error is created.
    let err = Rewrite::Format.apply("{\"a\":1,\n\"a\":2}").unwrap_err();
    assert!(err.brief().starts_with("第 2 行第 "), "{}", err.brief());
    assert!(err.to_string().contains("重复的键 \"a\""), "{err}");
    assert_eq!(JsonError::Timeout.to_string(), "执行超时");
    assert_eq!(InputKind::UrlParams.label(), "URL 参数");

    assert_eq!(
        rtools::plugins::translate::core::TranslateError::MissingKey.label(),
        "translate.error_key"
    );
    let cached = TranslatePlugin.recognizer().unwrap()("hello").unwrap();
    let json_hit = rtools::plugins::json::ui::JsonPlugin.recognizer().unwrap()("{}").unwrap();
    assert_eq!(cached.label().as_ref(), "翻译这段文本");
    assert_eq!(json_hit.label().as_ref(), "识别为 JSON");

    rust_i18n::set_locale("en");
    assert_eq!(JsonError::Timeout.to_string(), "Timed out");
    assert_eq!(InputKind::UrlParams.label(), "URL parameters");
    assert_eq!(cached.label().as_ref(), "Translate this text");
    assert_eq!(json_hit.label().as_ref(), "Recognized as JSON");
}
