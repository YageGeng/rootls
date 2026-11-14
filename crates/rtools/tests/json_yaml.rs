use rtools::plugins::json::core::convert::yaml::Yaml;
use serde_json::{Value, json};

#[test]
fn yaml_numbers_keep_their_text() {
    let v = Value::try_from(Yaml(
        "a: 12345678901234567890123\nb: 1.50\nc: 7\nd: [1, 2.0]\n",
    ))
    .unwrap();
    assert_eq!(
        v.to_string(),
        r#"{"a":12345678901234567890123,"b":1.50,"c":7,"d":[1,2.0]}"#
    );
}

#[test]
fn yaml_only_converts_mappings_and_sequences() {
    assert!(Value::try_from(Yaml("just text")).is_err());
    assert!(Value::try_from(Yaml("a: [1")).is_err());
}

#[test]
fn yaml_numbers_outside_json_syntax_use_their_value() {
    assert_eq!(
        Value::try_from(Yaml("h: 0x1F\nu: 1_000\n")).unwrap(),
        json!({"h": 31, "u": 1000})
    );
}

#[test]
fn large_yaml_is_not_cut_by_the_parser_budget() {
    let list = "- 1\n".repeat(300_000);
    let value = Value::try_from(Yaml(&list)).unwrap();
    assert_eq!(value.as_array().map(Vec::len), Some(300_000));
}
