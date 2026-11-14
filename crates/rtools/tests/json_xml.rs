use rtools::plugins::json::core::JsonError;
use rtools::plugins::json::core::convert::xml::{AsXml, Xml};
use serde_json::{Value, json};

#[test]
fn xml_to_json_maps_attributes_repeats_and_empty() {
    let v = Value::try_from(Xml(
        r#"<user id="7"><name>Tom</name><tag>a</tag><tag>b</tag><bio/></user>"#,
    ))
    .unwrap();
    assert_eq!(
        v,
        json!({"user": {"@id": "7", "name": "Tom", "tag": ["a", "b"], "bio": null}})
    );
}

#[test]
fn xml_to_json_keeps_text_next_to_attributes() {
    assert_eq!(
        Value::try_from(Xml(r#"<p lang="en">hi</p>"#)).unwrap(),
        json!({"p": {"@lang": "en", "#text": "hi"}})
    );
}

#[test]
fn xml_to_json_rejects_malformed_input() {
    assert!(Value::try_from(Xml("<a><b></a>")).is_err());
    assert!(Value::try_from(Xml("<a/><b/>")).is_err());
}

#[test]
fn json_to_xml_uses_single_key_as_root() {
    let v = json!({"user": {"@id": "7", "name": "Tom", "tag": ["a", "b"], "bio": null}});
    assert_eq!(
        AsXml(&v).to_string(),
        "<user id=\"7\">\n    <name>Tom</name>\n    <tag>a</tag>\n    <tag>b</tag>\n    <bio/>\n</user>\n"
    );
    assert_eq!(Value::try_from(Xml(&AsXml(&v).to_string())).unwrap(), v);
}

#[test]
fn json_to_xml_wraps_other_shapes_in_root() {
    assert_eq!(
        AsXml(&json!({"a": 1, "b": true})).to_string(),
        "<root>\n    <a>1</a>\n    <b>true</b>\n</root>\n"
    );
    assert_eq!(
        AsXml(&json!({"a": [1, 2]})).to_string(),
        "<root>\n    <a>1</a>\n    <a>2</a>\n</root>\n"
    );
    assert_eq!(
        AsXml(&json!([1, 2])).to_string(),
        "<root>\n    <item>1</item>\n    <item>2</item>\n</root>\n"
    );
    assert_eq!(AsXml(&json!({})).to_string(), "<root/>\n");
}

#[test]
fn json_to_xml_escapes_and_sanitizes() {
    let v = json!({"root": {"first name": "a<b & \"c\" 'd'", "1st": "y"}});
    let xml = AsXml(&v).to_string();
    assert_eq!(
        Value::try_from(Xml(&xml)).unwrap(),
        json!({"root": {"first_name": "a<b & \"c\" 'd'", "_1st": "y"}})
    );
}

#[test]
fn namespaced_names_round_trip() {
    let v = Value::try_from(Xml(
        r#"<soap:Envelope xmlns:soap="http://x"><soap:Body>1</soap:Body></soap:Envelope>"#,
    ))
    .unwrap();
    assert_eq!(
        v,
        json!({"soap:Envelope": {"@xmlns:soap": "http://x", "soap:Body": "1"}})
    );
    assert_eq!(Value::try_from(Xml(&AsXml(&v).to_string())).unwrap(), v);
}

#[test]
fn deeply_nested_xml_is_rejected() {
    let nested = |depth: usize| format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
    assert!(Value::try_from(Xml(&nested(100))).is_ok());
    assert!(matches!(
        Value::try_from(Xml(&nested(200))),
        Err(JsonError::Xml(_))
    ));
}

#[test]
fn json_to_xml_replaces_control_characters() {
    let xml = AsXml(&json!({"a": "x\u{1}y"})).to_string();
    assert_eq!(xml, "<a>x\u{fffd}y</a>\n");
    assert!(Value::try_from(Xml(&xml)).is_ok());
}

#[test]
fn colliding_attribute_names_get_suffixes() {
    let xml = AsXml(&json!({"r": {"@x y": "1", "@x_y": "2"}})).to_string();
    assert_eq!(xml, "<r x_y=\"1\" x_y_2=\"2\"/>\n");
    assert!(Value::try_from(Xml(&xml)).is_ok());
}
