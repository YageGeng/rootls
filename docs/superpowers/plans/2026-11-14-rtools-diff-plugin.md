# rtools Text Diff Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a second built-in plugin, "Text Diff": edit two texts side by side, then view their line diff (split/unified, word/char emphasis, prev/next change, folded context) with live comparison switches.

**Architecture:** A gpui-free core (`plugins/diff/core/compare.rs`) normalizes both texts, diffs per-line comparison keys with `similar`, and writes a unified patch of the original lines; the UI (`plugins/diff/ui/view.rs`) parses that patch with GPUI Kit's `DiffFile::parse` and renders it with the `Diff` component. `Draft` moves from the JSON plugin into the plugin contract so both plugins share it without depending on each other. Switches persist in a new `[diff]` settings table.

**Tech Stack:** Rust 2024, gpui-kit 0.9.1 (`Diff`, `DiffState`, `EditorState`, `Button`, `ButtonGroup`, `Kbd`), `similar` 2.7 (already in `Cargo.lock`), `serde_json` 1.0.154 (`arbitrary_precision`, `preserve_order`), rust-i18n.

**Spec:** `docs/superpowers/specs/2026-11-14-rtools-diff-plugin-design.md` (approved). Main design: `docs/superpowers/specs/2026-11-13-rtools-design.md`.

## Global Constraints

- Follow `AGENTS.md` in full:
  - an English doc comment on every new fn, and comments on non-trivial logic;
  - write `Arc::clone(&x)`;
  - tests only in `crates/rtools/tests/`;
  - dependencies declared in root `[workspace.dependencies]`, in semantic groups, sorted, with sub-crates using `{ workspace = true }`;
  - call `tracing::warn!(...)` fully qualified, with format args (no structured fields);
  - logs only on error and lifecycle paths;
  - operations live as methods on the owning struct;
  - use std conversion traits (`From`/`TryFrom`/`FromStr`/`Display`) instead of ad-hoc converters.
- No Chinese in `crates/rtools/src`; only `locales/app.yml` and tests may contain it.
- Never bind `Alt+Space`. Shortcuts:
  - `Ctrl+Enter` / `Cmd+Enter` opens the compare view;
  - `Esc` returns to editing;
  - `Alt+↑` / `Alt+↓` go to the previous / next change.
- The two plugins are independent: nothing under `plugins/diff` imports `plugins::json`, and the JSON plugin gains no diff entry point.
- **No intermediate commits.** Work in place on `main` (the user's established workflow). Make exactly one commit at the very end, and only when the user asks. Read `.gitmessage` first; title: `feat(rtools): add text diff plugin`.
- UI quality comes first. The edit view's bottom bar matches the JSON editor footer (`px_2 py_1p5 border_t_1`, `small` ghost icon buttons, `Separator::vertical().h_4().mx_1()`). Use theme tokens only; no raw colors.
- Gates for every task:
  - `cargo test` green;
  - `cargo clippy --all-targets -- -D warnings` clean;
  - `cargo fmt --all --check` clean;
  - `grep -rnP '\p{Han}' crates/rtools/src` prints nothing.

## Review Focus

1. **Text that looks like patch syntax.** Original/modified lines such as `--- a`, `+++ b`, `@@ -1 +1 @@`, `\ No newline at end of file`, ` x`, `+x`, `-x` and empty lines must still yield a patch that `DiffFile::parse` accepts, with matching counts. Owned by Task 2 test `patch_syntax_lines_round_trip_through_diff_file_parse`.
2. **Unicode whitespace and case.**
   - Under ignore-whitespace, U+3000, NBSP and tab must count as whitespace.
   - Under ignore-case, `ÄÖ` must equal `äö`.
   - Owned by Task 2 tests `ignore_whitespace_covers_unicode_spaces` and `ignore_case_compares_lowercased_lines`.
3. **Large, mostly different inputs** (20 000 lines) must finish near the 2 s deadline, with counts consistent with the parsed patch. Owned by Task 2 test `large_inputs_finish_with_consistent_counts`.
4. **"Format JSON" must preserve number spelling** (`1.50`, integers beyond 64 bits) while sorting keys, so it never hides or invents a difference. Owned by Task 2 test `format_json_keeps_number_spelling`.
5. **Two drafts in one directory** (`original.txt` and `modified.txt`) must not clobber each other, and the JSON plugin's existing `draft.json` must keep restoring. Owned by Task 1 test `drafts_with_different_names_share_a_directory`.

---

### Task 1: Move `Draft` into the plugin contract

**Files:**
- Move: `crates/rtools/src/plugin.rs` → `crates/rtools/src/plugin/mod.rs` (`git mv`; content unchanged except the new `mod`/`pub use`)
- Move: `crates/rtools/src/plugins/json/core/draft.rs` → `crates/rtools/src/plugin/draft.rs` (`git mv`)
- Modify: `crates/rtools/src/plugins/json/core/mod.rs` (drop `mod draft;` and `pub use draft::Draft;`)
- Modify: `crates/rtools/src/plugins/json/ui/view.rs:19,75` (import `crate::plugin::Draft`; `Draft::new(&launch.data_dir, "draft.json")`)
- Move test: `crates/rtools/tests/json_draft.rs` → `crates/rtools/tests/plugin_draft.rs` (`git mv`)

**Interfaces:**
- Produces: `rtools::plugin::Draft` with:
  - `pub fn new(data_dir: &Path, file_name: &str) -> Self`, which stores `<data_dir>/<file_name>` and uses the temp file `<data_dir>/<file_name>.<pid>-<n>.tmp`;
  - `load(&self) -> io::Result<Option<String>>` and `save(&self, text: &str) -> io::Result<()>`, both unchanged.

- [ ] **Step 1: Update the tests to the new API.** In `tests/plugin_draft.rs`:
  - import `rtools::plugin::Draft`;
  - every existing `Draft::new(&dir)` becomes `Draft::new(&dir, "draft.json")`, so the existing assertions (including `names == ["draft.json"]` and the `draft.json` written by `invalid_utf8_draft_is_reported`) still hold;
  - add:

```rust
#[test]
fn drafts_with_different_names_share_a_directory() {
    let dir = temp_dir("names");
    let original = Draft::new(&dir, "original.txt");
    let modified = Draft::new(&dir, "modified.txt");
    original.save("left").unwrap();
    modified.save("right").unwrap();
    assert_eq!(original.load().unwrap().as_deref(), Some("left"));
    assert_eq!(modified.load().unwrap().as_deref(), Some("right"));
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["modified.txt", "original.txt"]);
    fs::remove_dir_all(&dir).unwrap();
}
```

- [ ] **Step 2: Run the test and confirm it fails.** Run `cargo test --test plugin_draft`. Expected: compile error, because `rtools::plugin::Draft` is unresolved.
- [ ] **Step 3: Do the moves and add the `file_name` parameter.**
  - Do the `git mv`s listed above.
  - In `plugin/mod.rs`, add `mod draft;` and `pub use draft::Draft;`.
  - Update the doc comments: the module becomes "Autosaved plugin text", and the struct is "A plugin's draft file, `<data_dir>/<file_name>`".
  - The JSON view keeps the file name `draft.json`.
- [ ] **Step 4: Run all tests.** Run `cargo test`. Expected: all pass, including the 5 tests in `plugin_draft`.
- [ ] **Step 5: No commit** (single commit at the end).

---

### Task 2: Diff core (`Texts::compare`)

**Files:**
- Modify: `Cargo.toml` (add `similar = "2"` to the `# data formats` group, alphabetically after `serde_json`).
- Modify: `crates/rtools/Cargo.toml` (add `similar = { workspace = true }` at the same position).
- Create: `crates/rtools/src/plugins/diff/mod.rs` (`//! Text diff plugin: ...`; `pub mod core;` here, `pub mod ui;` is added in Task 4).
- Create: `crates/rtools/src/plugins/diff/core/mod.rs` (must not depend on gpui; `mod compare; pub use compare::{CompareOptions, Comparison, FormatJson, Side, Texts};`).
- Create: `crates/rtools/src/plugins/diff/core/compare.rs`.
- Modify: `crates/rtools/src/plugins/mod.rs` (add `pub mod diff;` before `pub mod json;`).
- Test: `crates/rtools/tests/diff_compare.rs`.

**Interfaces:**
- Produces the following names, exactly as in spec §4.2:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompareOptions { pub ignore_whitespace: bool, pub ignore_case: bool, pub format_json: bool }
pub struct Texts<'a> { pub original: &'a str, pub modified: &'a str }
impl Texts<'_> { pub fn compare(&self, options: CompareOptions) -> Comparison }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub patch: String, pub additions: usize, pub deletions: usize,
    pub json: bool, pub format_json: FormatJson,
    pub original_empty: bool, pub modified_empty: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum FormatJson { Off, Applied, Unavailable(Side) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Side { Original, Modified }
```

  `original_empty` and `modified_empty` are an additive ruling: spec rule 6 says the UI checks emptiness first, and only the core sees the normalized text.

- [ ] **Step 1: Write the failing tests** in `tests/diff_compare.rs`. Use this shared helper:

```rust
use gpui_kit::component::diff::DiffFile;
use rtools::plugins::diff::core::{CompareOptions, Comparison, FormatJson, Side, Texts};

fn compare(original: &str, modified: &str, options: CompareOptions) -> Comparison {
    Texts { original, modified }.compare(options)
}
const PLAIN: CompareOptions = CompareOptions { ignore_whitespace: false, ignore_case: false, format_json: false };
const WS: CompareOptions = CompareOptions { ignore_whitespace: true, ..PLAIN };
const CASE: CompareOptions = CompareOptions { ignore_case: true, ..PLAIN };
const JSON: CompareOptions = CompareOptions { format_json: true, ..PLAIN };
```

Tests (name → assertions):

| Test | Assertions |
|---|---|
| `changed_line_writes_one_full_context_hunk` | `compare("a\nb\nc", "a\nB\nc", PLAIN)` gives `patch == "--- original\n+++ modified\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"`, `additions == 1`, `deletions == 1`, `format_json == FormatJson::Off`, `!json` |
| `insertion_and_deletion_are_counted` | `compare("a\nb", "b\nc", PLAIN)`: patch ends with `"@@ -1,2 +1,2 @@\n-a\n b\n+c\n"`, additions 1, deletions 1 |
| `identical_texts_have_no_changes` | `compare("x\ny", "x\ny", PLAIN)`: additions 0, deletions 0 |
| `line_endings_and_final_newline_are_not_differences` | `compare("a\r\nb\r\n", "a\nb", PLAIN)`: 0 / 0, patch ends with `"@@ -1,2 +1,2 @@\n a\n b\n"` |
| `ignore_whitespace_shows_the_modified_text` | `compare("if x {\n  y\n}", "if x {\n\ty\n}", WS)`: 0 / 0 and `patch.contains("\n \ty\n")`; `compare("a b", "ab", WS)`: 0 / 0; `compare("a b", "ab", PLAIN)`: 1 / 1 |
| `ignore_whitespace_covers_unicode_spaces` | `compare("a\u{3000}b", "a\u{a0}b\t", WS)`: 0 / 0; the same pair with PLAIN: 1 / 1 |
| `ignore_case_compares_lowercased_lines` | `compare("Hello ÄÖ", "hello äö", CASE)`: 0 / 0; with PLAIN: 1 / 1 |
| `switches_combine` | `compare("  FOO", "foo", CompareOptions { ignore_case: true, ..WS })`: 0 / 0; with WS only: 1 / 1; with CASE only: 1 / 1 |
| `format_json_ignores_key_order_and_layout` | `compare(r#"{"b":1,"a":[{"d":2,"c":3}]}"#, "{\n  \"a\": [ {\"c\": 3, \"d\": 2} ],\n  \"b\": 1\n}", JSON)`: `format_json == Applied`, 0 / 0, `json`, `patch.contains("\n     \"a\": [\n")` (context space + 4-space indent) |
| `format_json_keeps_number_spelling` | `compare(r#"{"n": 1.50, "big": 12345678901234567890}"#, r#"{"big":12345678901234567890,"n":1.5}"#, JSON)`: Applied, 1 / 1, `patch.contains("-    \"n\": 1.50\n")`, `patch.contains("+    \"n\": 1.5\n")`, `patch.contains("     \"big\": 12345678901234567890,\n")` |
| `format_json_unavailable_names_the_first_invalid_side` | `compare("{bad", "{}", JSON)`: `Unavailable(Side::Original)`, `!json`, `patch.contains("-{bad\n")`; `compare("{}", "[", JSON)`: `Unavailable(Side::Modified)`; `compare("x", "y", JSON)`: `Unavailable(Side::Original)` |
| `json_flag_without_formatting` | `compare(r#"{"a":1}"#, r#"{"a":2}"#, PLAIN)`: `json`, `format_json == Off`; `compare("a", "{}", PLAIN)`: `!json` |
| `empty_sides_are_reported` | `compare("", "a", PLAIN)`: `original_empty`, `!modified_empty`, `patch.ends_with("@@ -0,0 +1,1 @@\n+a\n")`, additions 1. `compare("\n", "", PLAIN)`: both empty. `compare("  ", "", PLAIN)`: `!original_empty` (whitespace is not empty), `modified_empty` |
| `patch_syntax_lines_round_trip_through_diff_file_parse` | Original: `["--- a", "+++ b", "@@ -1 +1 @@", "\\ No newline at end of file", " lead", "+plus", "-minus", "", "tail"].join("\n")`. Modified: the same list with `"+plus"` → `"+PLUS"`, `"@@ -1 +1 @@"` removed, `"--- c"` appended. For that pair and for `("", original)` and `(original, "")`: `let files = DiffFile::parse(&c.patch).unwrap(); assert_eq!(files.len(), 1); assert_eq!((files[0].additions(), files[0].deletions()), (c.additions, c.deletions));` |
| `large_inputs_finish_with_consistent_counts` | Original: `(0..20_000).map(\|i\| format!("line {i}"))`. Modified: `(0..20_000).map(\|i\| format!("line {}", i * 7919 % 20_011))`, joined by `\n`. Time `compare(.., PLAIN)` and assert `elapsed < Duration::from_secs(10)`; `additions == deletions` (equal line counts); `DiffFile::parse` counts equal `(additions, deletions)` |

- [ ] **Step 2: Run the tests and confirm they fail.** Run `cargo test --test diff_compare`. Expected: compile error, because `rtools::plugins::diff` is unresolved.
- [ ] **Step 3: Implement `Texts::compare` in `compare.rs`.**
  - `const DEADLINE: Duration = Duration::from_secs(2);`.
  - Put the per-step work in private methods on `Texts`/`Comparison` or small private structs. Do not chain free functions (AGENTS rule 11). The steps run in this order:
    1. **Normalize each side:** `replace("\r\n", "\n")`, then strip one trailing `\n`. A side is empty if the result is `""`, and then it has zero lines. Otherwise its lines are `split('\n')`.
    2. **Format JSON, when the switch is on:** parse each normalized side with `serde_json::from_str::<Value>`. If both parse, call `sort_all_objects()` on each and pretty-print it with `PrettyFormatter::with_indent(b"    ")`; the result is `Applied`. Otherwise keep the normalized text and record `Unavailable(first failing side, Original checked first)`.
    3. **`json` flag:** both final texts parse as `Value`. When `Applied`, this is true without re-parsing.
    4. **Comparison keys:** one `Cow<str>` per line. Ignore-whitespace filters out every `char::is_whitespace` char; ignore-case applies `to_lowercase()`.
    5. **Diff the keys:** `similar::capture_diff_slices_deadline(Algorithm::Myers, &old_keys, &new_keys, Some(Instant::now() + DEADLINE))`.
    6. **Write the patch from the `DiffOp`s:**
       - write `--- original\n+++ modified\n@@ -S,N +S,M @@\n`, where `S` is `1`, or `0` when that side has 0 lines;
       - `Equal` writes ` ` + the **modified** line;
       - `Delete` writes `-` + the original line;
       - `Insert` writes `+` + the modified line;
       - `Replace` writes all its `-` lines, then all its `+` lines;
       - every line ends with `\n`.
    7. **Counts:** count the `+` and `-` lines while writing.
- [ ] **Step 4: Run the tests and confirm they pass.** Run `cargo test --test diff_compare`, then `cargo test`. Expected: all pass.
- [ ] **Step 5: No commit.**

---

### Task 3: `[diff]` settings table

**Files:**
- Modify: `crates/rtools/src/settings/mod.rs` (add `DiffSettings` and `DiffLayout`; add `Settings::diff` after `json`, and include it in `Default`).
- Test: `crates/rtools/tests/settings.rs`.

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffSettings { pub layout: DiffLayout, pub ignore_whitespace: bool, pub ignore_case: bool, pub format_json: bool, pub char_level: bool }
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffLayout { #[default] Split, Unified }
// Settings gains `pub diff: DiffSettings` (doc: the `[diff]` table, changed only from the compare view)
```

- [ ] **Step 1: Write the failing tests.**
  - In `defaults_follow_the_theme_and_system`, add `assert_eq!(settings.diff, DiffSettings::default()); assert_eq!(settings.diff.layout, DiffLayout::Split); assert!(!settings.diff.char_level);`.
  - In `toml_round_trips`, add `diff: DiffSettings { layout: DiffLayout::Unified, ignore_whitespace: true, ignore_case: false, format_json: true, char_level: true }`.
  - Add `diff_table_fills_missing_keys`: `"[diff]\nlayout = \"unified\"\nignore_case = true\n".parse::<Settings>().unwrap().diff` equals `DiffSettings { layout: DiffLayout::Unified, ignore_case: true, ..Default::default() }`.
  - Add `diff_layout_is_written_lowercase`: `Settings::default().to_string()` contains `"[diff]"` and `"layout = \"split\""`.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `cargo test --test settings`. Expected: compile error, because `DiffSettings` and `DiffLayout` are unresolved.
- [ ] **Step 3: Implement the types and the field.** Extend the `Default` doc comment to cover the new table.
- [ ] **Step 4: Run all tests.** Run `cargo test`. Expected: all pass.
- [ ] **Step 5: No commit.**

---

### Task 4: Plugin, edit view and compare view

**Files:**
- Create: `crates/rtools/src/plugins/diff/ui/mod.rs`, containing:
  - `MANIFEST` and `DiffPlugin`;
  - `impl From<&DiffSettings> for CompareOptions`;
  - `impl From<DiffLayout> for DiffMode`;
  - `pub fn bind_keys(cx: &mut App)`.
- Create: `crates/rtools/src/plugins/diff/ui/view.rs` (`DiffPluginView`).
- Modify: `crates/rtools/src/plugins/diff/mod.rs` (add `pub mod ui;`).
- Modify: `crates/rtools/src/main.rs`:
  - register `Box::new(DiffPlugin)` after `JsonPlugin`;
  - call `rtools::plugins::diff::ui::bind_keys(cx)` after `settings::window::bind_keys(cx)`.
- Modify: `crates/rtools/src/assets.rs` (append `GitCompare, ArrowLeftRight, Eraser, Space, Type, UnfoldVertical` to `ExtraIcons`; `CaseSensitive`, `ChevronUp` and `ChevronDown` are already default icons).
- Modify: `crates/rtools/locales/app.yml` (add a top-level `diff:` section with `en` / `zh-CN`; full key list below).

**Interfaces:**
- Consumes:
  - Task 1: `Draft::new(&launch.data_dir, "original.txt")` and `Draft::new(&launch.data_dir, "modified.txt")`.
  - Task 2: `Texts`, `CompareOptions`, `Comparison`, `FormatJson`, `Side`.
  - Task 3: `SettingsStore`'s `settings.diff`.
- Produces, for Task 5:
  - `actions!(diff, [BackToEdit])`;
  - `pub(super) const COMPARE_CONTEXT: &str = "DiffCompare"`;
  - these `DiffPluginView` fields:
    - `focus: FocusHandle`;
    - `original` and `modified: Entity<EditorState>`;
    - `diff: Entity<DiffState>`;
    - `mode: Mode` (`Edit | Compare`);
    - `options: CompareOptions` (the options of the latest comparison);
    - `summary: Option<Summary>`;
    - `unchanged_expanded: bool`;
    - `drafts: Arc<[Draft; 2]>`;
    - `unsaved: bool`;
    - `save_task` and `compare_task: Task<()>`;
    - `_subscriptions`;
  - `Summary { additions, deletions, format_json: FormatJson, outcome: Outcome }`;
  - `Outcome { Diff, Identical, OriginalEmpty, ModifiedEmpty, BothEmpty, Failed(SharedString) }`;
  - `fn compare(&mut self, cx: &mut Context<Self>)`, which recomputes from the current settings;
  - `fn apply_settings(&mut self, cx: &mut Context<Self>)`, which syncs the `DiffState` mode and inline unit from settings.

**Locale keys** (`diff.`*), English / Chinese:

| Key | English | Chinese |
|---|---|---|
| `title` | Text Diff | 文本对比 |
| `description` | Compare two texts | 对比两段文本的差异 |
| `original` | Original | 原文 |
| `modified` | Modified | 修改后 |
| `one_line` | 1 line | 1 行 |
| `lines` | %{count} lines | %{count} 行 |
| `compare` | Compare | 对比 |
| `swap` | Swap sides | 交换两侧 |
| `clear` | Clear both sides | 清空两侧 |
| `edit` | Edit | 编辑 |
| `previous` | Previous change | 上一处 |
| `next` | Next change | 下一处 |
| `ignore_whitespace` | Ignore whitespace | 忽略空白 |
| `ignore_case` | Ignore case | 忽略大小写 |
| `format_json` | Format JSON first | 先格式化 JSON |
| `format_json_unavailable_original` | Format JSON first: the original is not valid JSON | 先格式化 JSON：原文不是合法的 JSON |
| `format_json_unavailable_modified` | Format JSON first: the modified text is not valid JSON | 先格式化 JSON：修改后的文本不是合法的 JSON |
| `char_level` | Highlight changed characters | 字符级高亮 |
| `split` | Split | 并排 |
| `unified` | Unified | 统一 |
| `expand_unchanged` | Expand all unchanged lines | 展开全部未改动 |
| `collapse_unchanged` | Collapse unchanged lines | 折叠未改动 |
| `identical` | The texts are identical | 两段文本相同 |
| `identical_options` | With the current comparison options | 按当前对比选项 |
| `empty_original` | The original is empty | 原文为空 |
| `empty_modified` | The modified text is empty | 修改后为空 |
| `empty_both` | Both sides are empty | 两侧都为空 |
| `cannot_show` | Cannot show the differences | 无法显示差异 |

- [ ] **Step 1: Write the plugin module.**
  - `MANIFEST`:
    - `id: "diff"`, `title: "diff.title"`, `description: "diff.description"`;
    - `icon: IconName::GitCompare`, `keywords: &["diff", "compare"]`;
    - no `accepts`.
  - `open` builds the view via `cx.new(|cx| DiffPluginView::new(launch, window, cx))` and returns `PluginView::new(view, cx)`.
  - `From<&DiffSettings> for CompareOptions` copies the three switches.
  - `From<DiffLayout> for DiffMode` maps `Split` → `DiffMode::Split` and `Unified` → `DiffMode::Unified`. The orphan rule allows it because `DiffLayout` is local.
  - `bind_keys` binds `escape` → `BackToEdit` in `COMPARE_CONTEXT`.
- [ ] **Step 2: Build `DiffPluginView::new(launch, window, cx)`.**
  - Ignore `launch.input`; it is always `None` because the plugin has no `accepts`.
  - Load both drafts. A read error is logged as `tracing::warn!("reading the diff draft {} failed: {}", name, err)`, and that side starts empty.
  - Editors: `EditorState::new(window, cx).line_number(true).soft_wrap(false).searchable(true)`, with no language (plain text). The editor defaults to soft wrap, hence the explicit `false`.
  - `diff = cx.new(|cx| DiffState::new(Vec::new(), cx))`. It starts in unified mode with word emphasis.
  - Before returning, `new` calls `view.apply_settings(cx)`, so mode and inline unit come from the settings in one place:
    - `apply_settings` calls `set_mode(DiffMode::from(layout))` only when `diff.read(cx).mode()` differs;
    - it calls `set_inline_unit(Some(if char_level { Character } else { Word }))` only when `inline_unit()` differs;
    - it then calls `cx.notify()`.
    - Task 5 subscribes it to settings changes and extends it.
  - `focus = cx.focus_handle()`. This is the stable handle the launcher re-focuses on every summon.
  - Subscriptions:
    1. `InputEvent::Change` on either editor → `schedule_save` + `cx.notify()` (refreshes the line counts).
    2. `on_app_quit` and `on_release` → `flush_draft`.
  - Do not use `cx.on_focus` for the focus forward. It registers on the window passed at construction, and `Ctrl+D` (`Shell::detach`) moves the view into a new window, where it would never fire.
  - Copy the draft machinery from the JSON view: `SAVE_DEBOUNCE` of 1 s; the save task clones both ropes and saves both drafts on the background executor; `flush_draft` saves synchronously.
- [ ] **Step 3: Edit view rendering.**
  - Two panes, each `w_1_2`, separated by `border_l_1` on the right pane.
  - Each pane has an `h_8` header strip with `px_3`, `border_b_1`, and `text_xs`:
    - the side name in muted foreground on the left;
    - on the right, `diff.one_line` or `diff.lines(count)` from `text().lines_len()`, hidden when the editor is empty.
  - Below the header: `Editor::new(..).bordered(false).appearance(false)`, sized `h_full`.
  - Footer: ghost `small` icon buttons with tooltips, `swap` (`ArrowLeftRight`) and `clear` (`Eraser`), then a spacer, then a primary `small` button `diff.compare` with a `Kbd` child for the editor's binding (`cmd-enter` on macOS, else `ctrl-enter`).
  - `swap`: read both texts, then `replace_all` each editor with the other's text. `clear`: `replace_all("")` on both. Both stay undoable.
- [ ] **Step 4: Mode switching and keys.**
  - On the root element:
    - `.track_focus(&self.focus)`;
    - `.capture_action(on_enter)`: when `action.secondary && mode == Edit`, call `show_compare`; otherwise `cx.propagate()`. The action is `gpui_kit::component::input::Enter`.
    - `.capture_action(close_search)` on `Escape`, an exact copy of the JSON view's version, active in edit mode only (propagate in compare mode);
    - in compare mode, `.key_context(COMPARE_CONTEXT)` and `.on_action(back_to_edit)`.
  - `show_compare`:
    - set `mode = Compare` and `summary = None`, so a stale result never shows;
    - `window.focus(&self.focus)`;
    - call `compare`.
  - `back_to_edit`: set `mode = Edit`, then focus `original` directly with `original.update(cx, |s, cx| s.focus(window, cx))`.
  - Focus forward, in `render`: when `mode == Edit && self.focus.is_focused(window)`, call `cx.defer_in(window, |this, window, cx| ..)` to focus `original`. `render` always gets the view's current window, so the forward also works after detaching. The launcher's summon and `Shell::detach` both focus `PluginView::focus` and then redraw, so the forward runs right away.
- [ ] **Step 5: `compare`.**
  - Read `self.options = CompareOptions::from(&settings.diff)` and clone both ropes.
  - Replace `compare_task`. Dropping the old task cancels it, so there is no revision counter. The new task:
    1. On the background executor, runs `Texts { .. }.compare(options)` and chooses the outcome there, returning `(Summary, Vec<DiffFile>)`. `DiffFile` is `Send + Sync`, so a big patch never parses on the UI thread. The outcome is chosen in this order:
       - both empty → `BothEmpty`, original empty → `OriginalEmpty`, modified empty → `ModifiedEmpty`;
       - `additions + deletions == 0` → `Identical`;
       - otherwise `DiffFile::parse(&patch)`. `Ok` → map `with_language("json")` when `json` and use `Diff`. `Err(err)` → `Failed(err.to_string().into())`.
    2. Back on the UI thread:
       - for `Failed`, log `tracing::warn!("parsing the generated diff failed: {}", err)`;
       - for `Diff`, call `diff.set_files(files, cx)` and set `unchanged_expanded = false` (`set_files` resets expansion);
       - store the summary and call `cx.notify()`.
- [ ] **Step 6: Compare view rendering.**
  - Body when the outcome is `Diff`: `Diff::new(&self.diff).header_visible(false).hunk_separator(DiffHunkSeparator::Simple).line_number(true)`, filling the area.
  - Otherwise a centered status page in the style of the JSON result error state (`v_flex items_center justify_center gap_2 px_6`, an icon `size_5`, a `text_sm` title, a muted `text_xs` detail):

    | Outcome | Icon | Title | Detail |
    |---|---|---|---|
    | `Identical` | `CircleCheck` (success) | `diff.identical` | `diff.identical_options` when `options.ignore_whitespace \|\| options.ignore_case \|\| format_json == Applied` |
    | `*Empty` | `FileText` (muted) | `empty_original` / `empty_modified` / `empty_both` | — |
    | `Failed` | `CircleAlert` (danger) | `diff.cannot_show` | the error text |
    | none yet (`summary` is `None`) | — | — | an empty body |

  - Footer for this task: `+N` (`theme.success`) and `−M` (`theme.danger`) in `text_xs`, then a spacer, then an outline `small` button `diff.edit` with a `Kbd` child for `escape` that calls `back_to_edit`. Task 5 fills in the middle of the footer.
- [ ] **Step 7: Run the gates.** Run `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all --check`, and `grep -rnP '\p{Han}' crates/rtools/src`. Expected: all green, and the grep prints nothing.
- [ ] **Step 8: No commit.**

---

### Task 5: Compare toolbar (nav, switches, layout, unfold)

**Files:**
- Modify: `crates/rtools/src/plugins/diff/ui/view.rs`.
- Modify: `crates/rtools/src/plugins/diff/ui/mod.rs` (add bindings).

**Interfaces:**
- Consumes everything Task 4 produces.
- Produces `actions!(diff, [BackToEdit, PreviousChange, NextChange])`, with `alt-up` → `PreviousChange` and `alt-down` → `NextChange` bound in `COMPARE_CONTEXT`.

- [ ] **Step 1: Navigation.**
  - Add `on_action` handlers to the compare root that call `diff.update(|s, cx| s.previous_change(cx))` / `next_change(cx)`.
  - After the stats, add `Separator::vertical().h_4().mx_1()`, then ghost `small` icon buttons `ChevronUp` / `ChevronDown`. Each uses `.tooltip_with_action(t!("diff.previous"/"diff.next"), &PreviousChange/&NextChange, Some(COMPARE_CONTEXT))`, so the tooltip shows the real binding.
- [ ] **Step 2: Switches.**
  - After a separator, add four ghost `small` icon buttons, each `.selected(on).toggled(on)` with a tooltip:

    | Switch | Icon | Settings field |
    |---|---|---|
    | Ignore whitespace | `Space` | `ignore_whitespace` |
    | Ignore case | `CaseSensitive` | `ignore_case` |
    | Format JSON first | `Braces` | `format_json` |
    | Highlight changed characters | `Type` | `char_level` |

  - A click only calls `SettingsStore::update(cx, |s| s.diff.<field> = !s.diff.<field>)`.
  - When `summary.format_json` is `Unavailable(side)`, "Format JSON first" stays clickable but renders at `opacity(0.5)`. Its tooltip becomes `diff.format_json_unavailable_original` or `_modified`. This is a ruling: a truly disabled button could not be switched off.
- [ ] **Step 3: Apply settings.**
  - Add a subscription `cx.observe_global::<SettingsStore>(|this, cx| this.apply_settings(cx))`. One path then serves toolbar clicks, the settings reload on every summon, and hand-edited `settings.toml`.
  - Extend Task 4's `apply_settings`: when `mode == Compare` and `CompareOptions::from(&settings.diff) != self.options`, call `compare`. `compare` keeps the old `summary` visible until the new result lands (spec §3.2, last bullet).
- [ ] **Step 4: Layout and unfold.**
  - After a flex spacer, add `ButtonGroup::new("layout").small()` with `Button`s `split` / `unified` (`label` from `diff.split` / `diff.unified`, `.selected(..)`). `on_click` gets `&Vec<usize>` and writes `settings.diff.layout` through `SettingsStore::update`.
  - Then add a ghost `small` icon button for unfolding:
    - when `unchanged_expanded` is false: icon `UnfoldVertical`, tooltip `diff.expand_unchanged`, calls `expand_unchanged`;
    - when true: icon `FoldVertical`, tooltip `diff.collapse_unchanged`, calls `collapse_unchanged`;
    - each click flips `unchanged_expanded`.
  - Then the Edit button from Task 4.
  - Hide nav, unfold and the stats unless the outcome is `Diff`. The switches and layout stay visible on every compare state, so "identical with options" can be switched off.
  - Width: the whole toolbar must fit on one row at the launcher's minimum panel width (`MIN_WIDTH = 30` rem in `launcher/mod.rs`), in both locales (spec §5).
    - My estimate for English with four-digit stats is about 31–34 rem.
    - Every group is `flex_shrink_0`; only the spacer shrinks.
    - If the in-app check (Step 6, item 8) shows overflow, raise `MIN_WIDTH` to the measured English toolbar width, rounded up to a whole rem, and record a ruling. Keep the approved labels and Kbd hints.
- [ ] **Step 5: Run the gates** (same as Task 4, Step 7). Expected: all green.
- [ ] **Step 6: In-app verification.**
  - Gated: run only when the user is not using the machine. Back up `~/.local/share/rtools/{json,diff}/` and `~/.config/rtools/settings.toml` first, and restore them afterwards.
  - Before each input, check a screenshot pixel to confirm the overlay is showing.
  - Use the uinput driver plus `spectacle` screenshots to verify:
    1. Ctrl+Enter with an editor focused opens the compare view.
    2. Esc returns to edit, and a second Esc returns to the launcher list.
    3. Hiding the launcher from the compare view and summoning it again keeps Esc and Alt+↑↓ working.
    4. Each switch toggles its selected state and recomputes without flicker.
    5. Split and Unified both render.
    6. Identical, empty and invalid-JSON states look right.
    7. Drafts survive a restart.
    8. At the minimum panel width, in English, the compare toolbar fits on one row.
    9. After `Ctrl+D` detaches the plugin, typing goes straight into the Original editor, and Ctrl+Enter / Esc still switch views.
- [ ] **Step 7: No commit.**

---

### Task 6: Docs and housekeeping

**Files:**
- Modify: `.gitignore` (add `.superpowers/`).
- Modify: `docs/superpowers/specs/2026-11-13-rtools-design.md`:
  - §2 tree: `src/plugin/` (contract + `draft.rs`) and `src/plugins/diff/{core,ui}/`;
  - §2.1 data formats row: add `similar` (line diff for the diff plugin);
  - §5.6: the draft now lives at `crate::plugin::Draft`, `Draft::new(data_dir, file_name)`;
  - list the diff plugin and link its spec.
- Modify: `README.md`:
  - a "文本对比" feature section, next to the JSON editor section;
  - the `[diff]` block in the config example;
  - the diff drafts `~/.local/share/rtools/diff/{original,modified}.txt` in the data table;
  - shortcut rows: `Ctrl+Enter` (compare), `Esc` (back to edit), `Alt+↑/↓` (prev/next change);
  - mention `plugins/diff` in the code structure line.

- [ ] **Step 1: Make the edits above.**
- [ ] **Step 2: Run the gates once more.** Run `cargo test`, the clippy and fmt checks, and the Han grep. Expected: all green.
- [ ] **Step 3: Stop the visual companion server** with `scripts/stop-server.sh` from the brainstorming skill directory, using session dir `.superpowers/brainstorm/26012-1794721148/`.
- [ ] **Step 4: Single commit, only when the user asks.** Read `.gitmessage` first, then `git add -A` and `git commit -m "feat(rtools): add text diff plugin"`. Never push unless asked.
