use rtools::plugins::json::core::convert::xml::AsXml;
use rtools::plugins::json::core::{Json, JsonError, Rewrite};
use serde_json::Value;

#[test]
fn format_keeps_key_order_with_four_space_indent() {
    let out = Rewrite::Format
        .apply(r#"{"b":1,"a":[1,2],"c":{}}"#)
        .unwrap();
    assert_eq!(
        out,
        "{\n    \"b\": 1,\n    \"a\": [\n        1,\n        2\n    ],\n    \"c\": {}\n}"
    );
}

#[test]
fn minify_round_trips_format() {
    let src = r#"{"b":1,"a":[1,2],"c":{}}"#;
    let pretty = Rewrite::Format.apply(src).unwrap();
    assert_eq!(Rewrite::Minify.apply(&pretty).unwrap(), src);
}

#[test]
fn number_text_is_preserved() {
    let src = r#"{"big":12345678901234567890123,"dec":1.50,"exp":1e3}"#;
    let pretty = Rewrite::Format.apply(src).unwrap();
    // serde_json spells exponents with an explicit sign; the value is unchanged.
    assert_eq!(
        Rewrite::Minify.apply(&pretty).unwrap(),
        r#"{"big":12345678901234567890123,"dec":1.50,"exp":1e+3}"#
    );
}

#[test]
fn unicode_is_kept_verbatim() {
    let src = r#"{"城市":"胡杨河市","e":"😀"}"#;
    let pretty = Rewrite::Format.apply(src).unwrap();
    assert!(pretty.contains("\"城市\": \"胡杨河市\""));
    assert!(pretty.contains("😀"));
    let escaped = Rewrite::Escape.apply(src).unwrap();
    assert_eq!(Rewrite::Unescape.apply(&escaped).unwrap(), pretty);
}

#[test]
fn escape_minifies_then_escapes_without_outer_quotes() {
    let out = Rewrite::Escape.apply("{\n  \"a\": \"x\\\"y\"\n}").unwrap();
    assert_eq!(out, r#"{\"a\":\"x\\\"y\"}"#);
}

#[test]
fn unescape_accepts_quoted_and_unquoted_input() {
    let pretty = "{\n    \"a\": 1\n}";
    assert_eq!(Rewrite::Unescape.apply(r#""{\"a\":1}""#).unwrap(), pretty);
    assert_eq!(Rewrite::Unescape.apply(r#"{\"a\":1}"#).unwrap(), pretty);
}

#[test]
fn unescape_returns_plain_text_when_result_is_not_json() {
    assert_eq!(
        Rewrite::Unescape.apply(r"hello\nworld").unwrap(),
        "hello\nworld"
    );
}

#[test]
fn invalid_input_reports_line_and_column() {
    let err = Rewrite::Format.apply("{\n  \"a\": 1,\n}").unwrap_err();
    assert!(matches!(err, JsonError::Parse { line: 3, .. }), "{err:?}");
    assert!(!err.to_string().contains(" at line "), "{err}");
    assert!(matches!(
        Rewrite::Unescape.apply(r"\q"),
        Err(JsonError::Parse { .. })
    ));
}

#[test]
fn duplicate_keys_are_rejected_not_dropped() {
    let err = Rewrite::Format.apply("{\"a\":1,\n\"a\":2}").unwrap_err();
    assert!(matches!(err, JsonError::Parse { line: 2, .. }), "{err:?}");
    assert!(err.to_string().contains("Duplicate key \"a\""), "{err}");
    assert!(Rewrite::Minify.apply(r#"[{"b":{"c":1,"c":1}}]"#).is_err());
}

#[test]
fn byte_order_mark_is_ignored_when_formatting() {
    assert_eq!(
        Rewrite::Format.apply("\u{feff}{\"a\":1}").unwrap(),
        "{\n    \"a\": 1\n}"
    );
}

#[test]
fn unescape_column_points_into_unquoted_input() {
    // `ab\q`: the column must point into what the user typed, not the wrapped literal.
    let wrapped = Rewrite::Unescape.apply(r#""ab\q""#).unwrap_err();
    let bare = Rewrite::Unescape.apply(r"ab\q").unwrap_err();
    let (
        JsonError::Parse {
            column: c_wrapped, ..
        },
        JsonError::Parse { column: c_bare, .. },
    ) = (wrapped, bare)
    else {
        panic!("expected parse errors");
    };
    assert_eq!(c_bare + 1, c_wrapped);
}

#[test]
fn brief_error_text_leads_with_position() {
    let err = Rewrite::Format.apply("{\n  \"a\": 1,\n}").unwrap_err();
    assert!(
        err.brief().starts_with("Line 3, column 1: "),
        "{}",
        err.brief()
    );
    assert_eq!(JsonError::Timeout.brief(), "Timed out");
}

#[test]
fn validate_accepts_json_and_blank_text() {
    assert!(Json("\u{feff}{\"a\": [1, 2]}").validate().is_ok());
    assert!(Json("  \n").validate().is_ok());
}

#[test]
fn validate_rejects_repeated_keys() {
    let err = Json(r#"{"a": 1, "a": 2}"#).validate().unwrap_err();
    assert!(matches!(err, JsonError::Parse { line: 1, .. }), "{err:?}");
}

#[test]
fn validate_rejects_syntax_errors_and_trailing_text() {
    assert!(matches!(
        Json("{\"a\": }").validate(),
        Err(JsonError::Parse { .. })
    ));
    assert!(matches!(
        Json("{} x").validate(),
        Err(JsonError::Parse { .. })
    ));
}

#[test]
fn xml_output_reads_json_with_the_shared_parse_rules() {
    let value = Value::try_from(Json("\u{feff}{\"a\": {\"b\": 1}}")).unwrap();
    assert_eq!(AsXml(&value).to_string(), "<a>\n    <b>1</b>\n</a>\n");
    assert!(matches!(
        Value::try_from(Json(r#"{"a": 1, "a": 2}"#)),
        Err(JsonError::Parse { .. })
    ));
}
