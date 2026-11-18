use rtools::launcher::paste::{PASTE_INLINE_LIMIT, PastedInput};

#[test]
fn intercepts_multiline_and_large_text() {
    assert!(PastedInput::intercept("json".into()).is_none());
    assert!(PastedInput::intercept("{\n  \"a\": 1\n}".into()).is_some());
    let raw = "[\r\t10,\r\t20\r]";
    assert_eq!(PastedInput::intercept(raw.into()).unwrap().into_text(), raw);
    assert!(PastedInput::intercept("a".repeat(PASTE_INLINE_LIMIT)).is_none());
    assert!(PastedInput::intercept("a".repeat(PASTE_INLINE_LIMIT + 1)).is_some());
}

#[test]
fn size_labels() {
    let label = |n: usize| {
        PastedInput::intercept(format!("x\n{}", "a".repeat(n - 2)))
            .unwrap()
            .size_label()
    };
    assert_eq!(label(812), "812 B");
    assert_eq!(label(3277), "3.2 KB");
    assert_eq!(label(1_572_864), "1.5 MB");
}

#[test]
fn preview_collapses_whitespace_and_truncates_by_char() {
    let p = PastedInput::intercept("{\n    \"城市\":   \"胡杨河市\"\n}".into()).unwrap();
    assert_eq!(p.preview(), "{ \"城市\": \"胡杨河市\" }");
    let long = PastedInput::intercept(format!("x\n{}", "胡".repeat(100))).unwrap();
    assert_eq!(long.preview().chars().count(), 81);
    assert!(long.preview().ends_with('…'));
    let first = long.preview();
    let second = long.preview();
    assert_eq!(
        first.as_ptr(),
        second.as_ptr(),
        "redraws share the preview text"
    );
}
