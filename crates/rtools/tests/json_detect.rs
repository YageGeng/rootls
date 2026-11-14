use rtools::plugins::json::core::convert::query::Query;
use rtools::plugins::json::core::{InputKind, JsonError, Normalized};
use serde_json::{Value, json};

#[test]
fn query_merges_repeated_keys_and_decodes() {
    assert_eq!(
        Value::from(Query("?a=1&b=2&a=3")),
        json!({"a": ["1", "3"], "b": "2"})
    );
    assert_eq!(
        Value::from(Query("https://x.com/p?name=%E5%BC%A0&q=a+b#frag")),
        json!({"name": "张", "q": "a b"})
    );
    assert_eq!(Value::from(Query("flag")), json!({"flag": ""}));
}

#[test]
fn json_is_formatted() {
    assert_eq!(
        "  {\"a\":1}".parse::<Normalized>().unwrap(),
        Normalized("{\n    \"a\": 1\n}".into())
    );
}

#[test]
fn broken_json_reports_parse_error_instead_of_yaml() {
    assert!(matches!(
        "{\n  \"a\": 1,\n}".parse::<Normalized>(),
        Err(JsonError::Parse { line: 3, .. })
    ));
    assert!(matches!(
        "[1, 2".parse::<Normalized>(),
        Err(JsonError::Parse { .. })
    ));
}

#[test]
fn xml_url_and_yaml_are_converted() {
    assert_eq!(
        "<a>1</a>".parse::<Normalized>().unwrap(),
        Normalized("{\n    \"a\": \"1\"\n}".into())
    );
    assert_eq!(
        "a=1&b=2".parse::<Normalized>().unwrap(),
        Normalized("{\n    \"a\": \"1\",\n    \"b\": \"2\"\n}".into())
    );
    assert_eq!(
        "https://x.com/?a=1".parse::<Normalized>().unwrap(),
        Normalized("{\n    \"a\": \"1\"\n}".into())
    );
    assert_eq!(
        "name: rtools\ntags:\n  - a\n"
            .parse::<Normalized>()
            .unwrap(),
        Normalized("{\n    \"name\": \"rtools\",\n    \"tags\": [\n        \"a\"\n    ]\n}".into())
    );
}

#[test]
fn unrecognized_inputs() {
    for s in ["", "   ", "hello world", "a: 1", "just\nplain text"] {
        assert_eq!(
            s.parse::<Normalized>(),
            Err(JsonError::Unrecognized),
            "{s:?}"
        );
    }
}

#[test]
fn sniff_is_prefix_based() {
    assert_eq!(InputKind::sniff(" {\"a\""), Some(InputKind::Json));
    assert_eq!(InputKind::sniff("[1"), Some(InputKind::Json));
    assert_eq!(InputKind::sniff("<x"), Some(InputKind::Xml));
    assert_eq!(InputKind::sniff("a=1"), Some(InputKind::UrlParams));
    assert_eq!(InputKind::sniff("hello"), None);
    assert_eq!(InputKind::sniff("name: x\nb: y"), None);
}

#[test]
fn quoted_fragment_is_treated_as_json_not_yaml() {
    assert_eq!(InputKind::sniff("\"a\": 1"), Some(InputKind::Json));
    assert!(matches!(
        "\"a\": 1,\n\"b\": 2".parse::<Normalized>(),
        Err(JsonError::Parse { .. })
    ));
}

#[test]
fn error_positions_follow_original_text() {
    assert!(matches!(
        "\n\n{\"a\":1,}".parse::<Normalized>(),
        Err(JsonError::Parse { line: 3, .. })
    ));
}

#[test]
fn byte_order_mark_is_ignored() {
    assert_eq!(
        "\u{feff}{\"a\":1}".parse::<Normalized>().unwrap(),
        Normalized("{\n    \"a\": 1\n}".into())
    );
    assert_eq!(InputKind::sniff("\u{feff}{"), Some(InputKind::Json));
}

#[test]
fn bare_params_keep_raw_question_mark_and_hash() {
    assert_eq!(
        Value::from(Query("a=1&next=https://y.com/?b=2")),
        json!({"a": "1", "next": "https://y.com/?b=2"})
    );
    assert_eq!(
        Value::from(Query("color=#fff&size=2")),
        json!({"color": "#fff", "size": "2"})
    );
    assert_eq!(
        Value::from(Query("https://x.com/p?q=1#frag")),
        json!({"q": "1"})
    );
}

#[test]
fn kinds_have_user_facing_labels() {
    assert_eq!(InputKind::UrlParams.label(), "URL parameters");
    assert_eq!(InputKind::Json.label(), "JSON");
}
