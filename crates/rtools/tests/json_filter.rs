use futures::executor::block_on;
use rtools::plugins::json::core::{FILTER_TIMEOUT, FilterEngine, JsonError};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn engine(doc: &str) -> FilterEngine {
    let e = FilterEngine::spawn().unwrap();
    e.set_document(Arc::from(doc));
    e
}
const DOC: &str = r#"{"name":"rtools","list":[{"id":1},{"id":2}]}"#;

#[test]
fn path_and_method_expressions() {
    let e = engine(DOC);
    assert_eq!(block_on(e.evaluate(".name")).unwrap(), "\"rtools\"");
    assert_eq!(block_on(e.evaluate("[\"name\"]")).unwrap(), "\"rtools\"");
    assert_eq!(
        block_on(e.evaluate(".list.map(x=>x.id)")).unwrap(),
        "[\n    1,\n    2\n]"
    );
}

#[test]
fn semicolon_starts_a_full_expression() {
    let e = engine(DOC);
    assert_eq!(
        block_on(e.evaluate(";Object.keys(this)")).unwrap(),
        "[\n    \"name\",\n    \"list\"\n]"
    );
}

#[test]
fn empty_expression_returns_whole_document() {
    let e = engine(r#"{"a":1}"#);
    assert_eq!(block_on(e.evaluate("")).unwrap(), "{\n    \"a\": 1\n}");
}

#[test]
fn non_serialisable_results() {
    let e = engine(DOC);
    assert_eq!(block_on(e.evaluate(".missing")).unwrap(), "undefined");
    assert_eq!(block_on(e.evaluate(";(()=>1)")).unwrap(), "()=>1");
}

#[test]
fn errors_are_reported() {
    let e = engine(DOC);
    assert!(matches!(
        block_on(e.evaluate(".name.")),
        Err(JsonError::Script(_))
    ));
    let bad = engine("{bad");
    assert!(matches!(
        block_on(bad.evaluate(".a")),
        Err(JsonError::Script(_))
    ));
}

#[test]
fn timeout_then_recovers() {
    let e = engine(DOC);
    let start = Instant::now();
    assert_eq!(
        block_on(e.evaluate(";while(true){}")),
        Err(JsonError::Timeout)
    );
    assert!(start.elapsed() < FILTER_TIMEOUT + Duration::from_millis(500));
    assert_eq!(block_on(e.evaluate(".name")).unwrap(), "\"rtools\"");
}

#[test]
fn document_replacement_invalidates_cache() {
    let e = engine(r#"{"a":1}"#);
    assert_eq!(block_on(e.evaluate(".a")).unwrap(), "1");
    e.set_document(Arc::from(r#"{"a":2}"#));
    assert_eq!(block_on(e.evaluate(".a")).unwrap(), "2");
}

#[test]
fn large_document_is_not_cut_by_timeout() {
    let item = r#"{"city":"胡杨河市","name":"兵团一二五团","id":"659010500000"}"#;
    let doc = format!("[{}]", vec![item; 100_000].join(","));
    let e = engine(&doc);
    assert_eq!(block_on(e.evaluate(".length")).unwrap(), "100000");
}

#[test]
fn unicode_keys_and_values() {
    let e = engine(r#"{"城市":"胡杨河市"}"#);
    assert_eq!(block_on(e.evaluate(".城市")).unwrap(), "\"胡杨河市\"");
}

#[test]
fn side_effects_do_not_leak_between_evaluations() {
    let e = engine(r#"{"list":[1,2,3]}"#);
    let reversed = "[\n    3,\n    2,\n    1\n]";
    assert_eq!(block_on(e.evaluate(".list.reverse()")).unwrap(), reversed);
    assert_eq!(block_on(e.evaluate(".list.reverse()")).unwrap(), reversed);
    assert_eq!(
        block_on(e.evaluate(".list")).unwrap(),
        "[\n    1,\n    2,\n    3\n]"
    );
}

#[test]
fn expressions_run_in_sloppy_mode() {
    let e = engine(DOC);
    assert_eq!(
        block_on(e.evaluate(";x = this.name; x")).unwrap(),
        "\"rtools\""
    );
}

#[test]
fn unserialisable_values_fall_back_to_string() {
    let e = engine(DOC);
    assert_eq!(block_on(e.evaluate(";10n")).unwrap(), "10");
    assert_eq!(
        block_on(e.evaluate(";const o = {}; o.self = o; o")).unwrap(),
        "[object Object]"
    );
}

#[test]
fn error_kind_is_named() {
    let e = engine(DOC);
    let err = block_on(e.evaluate(".name.")).unwrap_err().to_string();
    assert!(err.contains("SyntaxError"), "{err}");
    let err = block_on(e.evaluate(".missing.x")).unwrap_err().to_string();
    assert!(err.contains("TypeError"), "{err}");
}

#[test]
fn overwriting_builtins_does_not_break_the_engine() {
    let e = engine(DOC);
    let _ = block_on(e.evaluate(
        ";JSON.stringify = null; JSON.parse = null; String = null; eval = null; Reflect.apply = null",
    ));
    assert_eq!(block_on(e.evaluate(".name")).unwrap(), "\"rtools\"");
    e.set_document(Arc::from(r#"{"a":1}"#));
    assert_eq!(block_on(e.evaluate(".a")).unwrap(), "1");
}
