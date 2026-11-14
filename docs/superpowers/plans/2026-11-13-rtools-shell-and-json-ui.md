# rtools Shell & JSON UI Implementation Plan（计划 2/2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修完计划 1 遗留的 11 个 Minor 问题；做出 `rtools-plugin` 插件契约、启动器外壳（单实例、快捷键、弹窗、搜索、粘贴拦截、插件挂载/返回/分离）和 JSON 插件界面，视觉符合 spec §5.7。

**Architecture:** `crates/rtools-plugin` 定义 `Plugin` trait 与 `PluginRegistry`；`crates/rtools` 拆成 lib（可测逻辑：命令、单实例、粘贴模型）+ bin（GPUI 外壳：`Shell` 全局、`Launcher` 视图、快捷键）；`plugins/rtools-json` 新增 `ui` 模块实现 `JsonPlugin`。外壳与插件只通过 `rtools-plugin` 交互；`src/core` 仍不依赖 gpui。

**Tech Stack:** Rust stable、gpui-kit 0.9.1（feature `syntect`）、interprocess 2.4、global-hotkey 0.10、ashpd 0.14（`global_shortcuts` + `async-io`，仅 Linux）、dirs 7、tracing 0.1、tracing-subscriber 0.3、futures 0.3、thiserror 2.0；core 沿用计划 1 依赖。

**Spec:** `docs/superpowers/specs/2026-11-13-rtools-design.md`（交互 §4.2、视觉 §5.7）

## Global Constraints

- **不做中途提交**：所有任务只改工作区，Task 11 最后做唯一一次提交（用户要求）。
- 计划 1 的全部全局约束继续有效（`AGENTS.md` 各条、stable 工具链、`src/core` 不依赖 gpui、缩进 4、过滤超时 2 s、内存 1 GiB、中文用户文案）。
- 依赖全部在根 `[workspace.dependencies]` 声明并分组：`# internal`（`rtools-json`、`rtools-plugin`）、`# ui`（`gpui-kit`）、`# script`、`# data formats`、`# platform`（`ashpd`、`dirs`、`global-hotkey`、`interprocess`）、`# infra`（`futures`、`thiserror`、`tracing`、`tracing-subscriber`）；子 crate 一律 `{ workspace = true }`，`ashpd` 在子 crate 用 `[target.'cfg(target_os = "linux")'.dependencies]`。
- UI 只依赖 `gpui-kit`（`use gpui_kit::*;`、`gpui_kit::component::…`）；不得发明 API，用前先在 `~/.cargo/registry/src/*/gpui-*` 中核对签名。
- 视觉：严格按 spec §5.7；颜色只取 `cx.theme()` token，间距/字号用 rem 助手；原始像素只允许出现在窗口尺寸与主题 token 定义（`window_radius = px(12.)`）这两类有注释的边界。
- 前端代码按 `AGENTS.md` 第 3 条豁免 TDD，但每个 UI 任务以「运行 + 截图核对」结束；可测的逻辑（命令解析、单实例、粘贴模型、注册表、草稿、核心修复）一律先写失败测试。
- 公开数据类型不暴露 `pub` 字段，用构造器 + 读方法（gpui-kit 编码规范）；转换/解析优先 `From`/`TryFrom`/`FromStr`/`Display`/`FromIterator`。
- 截图核对方法：`spectacle -b -n -f -o /tmp/x.png`，再裁出窗口区域查看；明暗两套主题都要看。

## Review Focus

1. 弹窗失焦关闭后再呼出（快捷键或 `rtools toggle`），插件视图与输入框内容必须原样恢复、不崩溃（实体跨窗口复用）→ Task 9 Step「状态恢复」核对。
2. 把 4.5 MB JSON 粘贴进启动器搜索框：不得卡死，必须变成「已粘贴 4.4 MB」标签，回车后 JSON 插件以格式化后的内容打开 → Task 7 `intercepts_multiline_and_large_text` + Task 9 核对。
3. 在大文档上快速连续输入过滤表达式：旧结果不得覆盖新结果（序号丢弃）→ Task 8 Step 实现要求 + Task 9 核对。
4. 草稿文件损坏（非 UTF-8）或写入失败：插件照常打开、内存内容不丢、只记 warn → Task 4 `invalid_utf8_draft_is_reported`、Task 8 实现要求。
5. 上次进程崩溃留下的 socket、或第二次启动 `rtools`：不得卡住启动，第二个进程只发命令后退出 → Task 6 `send_without_server_fails_fast`、`server_round_trip` + Task 9 核对。

---

### Task 1: 计划 1 遗留 —— 错误位置、BOM、URL 参数

**Files:**
- Modify: `plugins/rtools-json/src/core/detect.rs`、`plugins/rtools-json/src/core/format.rs`、`plugins/rtools-json/src/core/convert/query.rs`
- Test: `plugins/rtools-json/tests/detect.rs`、`plugins/rtools-json/tests/format.rs`

**Interfaces:**
- Consumes: 计划 1 的 `Normalized: FromStr`、`InputKind::sniff`、`JsonFormatter`、`Query`
- Produces: 行为修正，无新签名

- [ ] **Step 1: 追加失败测试**

`tests/detect.rs`：

```rust
#[test]
fn error_positions_follow_original_text() {
    assert!(matches!(norm("\n\n{\"a\":1,}"), Err(JsonError::Parse { line: 3, .. })));
}

#[test]
fn byte_order_mark_is_ignored() {
    assert_eq!(norm("\u{feff}{\"a\":1}").unwrap().0, InputKind::Json);
    assert_eq!(InputKind::sniff("\u{feff}{"), Some(InputKind::Json));
}

#[test]
fn bare_params_keep_raw_question_mark_and_hash() {
    assert_eq!(
        query_to_json("a=1&next=https://y.com/?b=2"),
        json!({"a": "1", "next": "https://y.com/?b=2"})
    );
    assert_eq!(query_to_json("color=#fff&size=2"), json!({"color": "#fff", "size": "2"}));
    assert_eq!(query_to_json("https://x.com/p?q=1#frag"), json!({"q": "1"}));
}
```

`tests/format.rs`：

```rust
#[test]
fn byte_order_mark_is_ignored_when_formatting() {
    assert_eq!(JsonFormatter::default().format("\u{feff}{\"a\":1}").unwrap(), "{\n    \"a\": 1\n}");
}

#[test]
fn unescape_column_points_into_unquoted_input() {
    // `ab\q`: the bad escape starts at column 3 of what the user typed.
    let wrapped = JsonFormatter::default().unescape(r#""ab\q""#).unwrap_err();
    let bare = JsonFormatter::default().unescape(r"ab\q").unwrap_err();
    let (JsonError::Parse { column: c_wrapped, .. }, JsonError::Parse { column: c_bare, .. }) = (wrapped, bare) else {
        panic!("expected parse errors");
    };
    assert_eq!(c_bare + 1, c_wrapped);
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --no-fail-fast 2>&1 | grep -E 'FAILED|test result'`
Expected：上面 5 个新测试 FAIL，其余通过。

- [ ] **Step 3: 实现**

- BOM：`Normalized::from_str`、`InputKind::sniff`、`JsonFormatter` 的内部解析入口在处理前去掉开头的 `'\u{feff}'`。
- 行列号：`Normalized::from_str` 的 JSON 分支把**未 trim** 的原文（只去 BOM）交给 `formatter.format`；XML/URL/YAML 分支照旧用 trim 后文本。
- `unescape` 对不带引号的输入，在第 1 行的错误上把 `column` 减 1（补偿前置的 `"`）。
- `Query`：只有当第一个 `?` 之前的部分不含 `=` 时（`?a=1`、完整 URL）才截取 `?` 之后并去掉 `#` 片段；否则视为裸参数整体解析。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --no-fail-fast 2>&1 | grep -E 'FAILED|test result'`
Expected：全部通过。

---

### Task 2: 计划 1 遗留 —— 过滤引擎健壮性

**Files:**
- Modify: `plugins/rtools-json/src/core/filter.rs`
- Test: `plugins/rtools-json/tests/filter.rs`

**Interfaces:**
- Consumes: `FilterEngine::{spawn, set_document, evaluate}`
- Produces: 签名不变；错误文本以 JS 错误名开头（如 `SyntaxError: …`）

- [ ] **Step 1: 追加失败测试**

```rust
#[test]
fn unserialisable_values_fall_back_to_string() {
    let e = engine(DOC);
    assert_eq!(block_on(e.evaluate(";10n")).unwrap(), "10");
    assert_eq!(block_on(e.evaluate(";const o = {}; o.self = o; o")).unwrap(), "[object Object]");
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
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --test filter --no-fail-fast 2>&1 | grep -E 'FAILED|test result'`
Expected：3 个新测试 FAIL。

- [ ] **Step 3: 实现**

- 初始化时（`Worker::new`）执行一段 JS，在闭包里捕获 `JSON.parse`、`JSON.stringify`、`String`、`Reflect.apply` 以及 `eval`（闭包内 `var eval = <捕获的 eval>`，sloppy 模式下仍是 direct eval，`this` 绑定不变），用 `Object.defineProperty(globalThis, name, { value, writable: false, configurable: false })` 安装两个函数：
  - `__rtoolsLoad(src)`：保存原文到闭包；
  - `__rtoolsPrepare()`：用捕获的 parse 重建闭包内的 `doc`（不计时）；
  - `__rtoolsRun(code)`：`apply(function () { return eval(code) }, doc, [])`；结果 `undefined` → `'undefined'`；`stringify` 用 `try/catch`，抛错或返回 `undefined` 时用捕获的 `String`。
- Rust 端：`SetDocument` 调 `__rtoolsLoad`；求值时先 `__rtoolsPrepare`（无截止时间），再在截止时间内 `__rtoolsRun`。不再使用 `__src`/`__doc`/`__code` 全局。
- `script_error`：错误文本用 `name + ": " + message` 的第一行（异常对象读 `name`、`message` 属性；非 Error 值用 `Coerced<String>`）。
- 工作线程创建失败（`thread::Builder::spawn` 出错）映射为 `JsonError::EngineStopped`。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --test filter --no-fail-fast 2>&1 | grep -E 'FAILED|test result'`
Expected：全部通过（`timeout_then_recovers` 仍在 2.5 s 内）。

---

### Task 3: 计划 1 遗留 —— YAML 精确数字、XML 非法字符与属性撞名、spec 措辞

**Files:**
- Create: `plugins/rtools-json/src/core/convert/yaml.rs`
- Modify: `plugins/rtools-json/src/core/convert/mod.rs`（`pub mod yaml;`）、`plugins/rtools-json/src/core/convert/xml.rs`、`plugins/rtools-json/src/core/detect.rs`（改用 `Yaml`）、`plugins/rtools-json/src/core/error.rs`（恢复 `Yaml(String)` 变体）、`docs/superpowers/specs/2026-11-13-rtools-design.md`
- Test: `plugins/rtools-json/tests/yaml.rs`、`plugins/rtools-json/tests/xml.rs`

**Interfaces:**
- Consumes: `JsonError`、`Members`
- Produces:
  ```rust
  // rtools_json::core::convert::yaml
  pub struct Yaml<'a>(pub &'a str);
  impl TryFrom<Yaml<'_>> for Value { type Error = JsonError; } // mapping/sequence only
  // JsonError gains: #[error("YAML 解析失败：{0}")] Yaml(String)
  ```
  `Normalized::from_str` 把 `Yaml` 的任何错误仍映射为 `JsonError::Unrecognized`（行为不变）。

- [ ] **Step 1: 写失败测试**

`tests/yaml.rs`：

```rust
use rtools_json::core::convert::yaml::Yaml;
use serde_json::{Value, json};

#[test]
fn yaml_numbers_keep_their_text() {
    let v = Value::try_from(Yaml("a: 12345678901234567890123\nb: 1.50\nc: 7\nd: [1, 2.0]\n")).unwrap();
    assert_eq!(v.to_string(), r#"{"a":12345678901234567890123,"b":1.50,"c":7,"d":[1,2.0]}"#);
}

#[test]
fn yaml_only_converts_mappings_and_sequences() {
    assert!(Value::try_from(Yaml("just text")).is_err());
    assert!(Value::try_from(Yaml("a: [1")).is_err());
}

#[test]
fn yaml_numbers_outside_json_syntax_use_their_value() {
    assert_eq!(Value::try_from(Yaml("h: 0x1F\nu: 1_000\n")).unwrap(), json!({"h": 31, "u": 1000}));
}
```

`tests/xml.rs` 追加：

```rust
#[test]
fn json_to_xml_replaces_control_characters() {
    let xml = json_to_xml(&json!({"a": "x\u{1}y"}));
    assert_eq!(xml, "<a>x\u{fffd}y</a>\n");
    assert!(xml_to_json(&xml).is_ok());
}

#[test]
fn colliding_attribute_names_get_suffixes() {
    let xml = json_to_xml(&json!({"r": {"@x y": "1", "@x_y": "2"}}));
    assert_eq!(xml, "<r x_y=\"1\" x_y_2=\"2\"/>\n");
    assert!(xml_to_json(&xml).is_ok());
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --no-fail-fast 2>&1 | grep -E 'FAILED|test result|^error'`
Expected：`yaml` 测试编译失败（`Yaml` 不存在），两个 XML 新测试 FAIL。

- [ ] **Step 3: 实现**

- `Yaml`：用 `serde_saphyr::from_str` 反序列化到私有树 `YamlNode`（`Null`/`Bool`/`Number(serde_json::Number)`/`String`/`Seq(Vec<Spanned<YamlNode>>)`/`Map(Vec<(String, Spanned<YamlNode>)>)`，通过 `deserialize_any` + 自定义 `Visitor` 构建）。转换为 `Value` 时，数字节点用 `span.byte_offset()`/`byte_len()` 从原文切出字面量，`serde_json::Number::from_str` 成功就用它（`arbitrary_precision` 保留原文），否则用反序列化得到的数值。顶层不是 mapping/sequence → `JsonError::Yaml("顶层必须是对象或数组")`；解析错误 → `JsonError::Yaml(err.to_string())`。
- `AsXml`：输出文本与属性值前，把 XML 1.0 不允许的字符（除 `\t` `\n` `\r` 外的 C0 控制字符、`U+FFFE`、`U+FFFF`）替换为 `U+FFFD`；同一元素上清洗后重名的属性依次加后缀 `_2`、`_3`……
- spec：§5.3 括号里把「唯一例外」改为「例外：指数补正号且 `E` 变小写，如 `1E3` → `1e+3`，数值不变」；§2.1 data formats 一行加入 `serde`。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --no-fail-fast 2>&1 | grep -E 'FAILED|test result'`
Expected：全部通过。

---

### Task 4: 草稿持久化（core）

**Files:**
- Create: `plugins/rtools-json/src/core/draft.rs`
- Modify: `plugins/rtools-json/src/core/mod.rs`（`mod draft; pub use draft::Draft;`）
- Test: `plugins/rtools-json/tests/draft.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Draft { path: PathBuf }
  impl Draft {
      /// Draft stored at `<data_dir>/draft.json`.
      pub fn new(data_dir: &Path) -> Self;
      /// `Ok(None)` when no draft exists; `InvalidData` when the file is not UTF-8.
      pub fn load(&self) -> io::Result<Option<String>>;
      /// Writes `draft.json.tmp` then renames it over `draft.json`; creates the directory.
      pub fn save(&self, text: &str) -> io::Result<()>;
  }
  ```

- [ ] **Step 1: 写失败测试 `tests/draft.rs`**

```rust
use rtools_json::core::Draft;
use std::{fs, io, path::PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rtools-draft-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn missing_draft_loads_as_none() {
    assert_eq!(Draft::new(&temp_dir("missing")).load().unwrap(), None);
}

#[test]
fn save_creates_directory_and_round_trips() {
    let dir = temp_dir("roundtrip").join("nested");
    let draft = Draft::new(&dir);
    draft.save("{\"城市\": \"胡杨河市\"}").unwrap();
    assert_eq!(draft.load().unwrap().as_deref(), Some("{\"城市\": \"胡杨河市\"}"));
    draft.save("2").unwrap();
    assert_eq!(draft.load().unwrap().as_deref(), Some("2"));
    assert!(!dir.join("draft.json.tmp").exists());
}

#[test]
fn invalid_utf8_draft_is_reported() {
    let dir = temp_dir("utf8");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("draft.json"), [0xff, 0xfe, 0x00]).unwrap();
    assert_eq!(Draft::new(&dir).load().unwrap_err().kind(), io::ErrorKind::InvalidData);
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --test draft`
Expected：编译失败，`Draft` 不存在。

- [ ] **Step 3: 实现 `draft.rs`**（`fs::read` + `String::from_utf8` 映射 `InvalidData`；`NotFound` → `Ok(None)`）

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --test draft`
Expected：3 个测试 PASS。

---

### Task 5: `rtools-plugin` 插件契约

**Files:**
- Create: `crates/rtools-plugin/Cargo.toml`（依赖 `gpui-kit`）、`crates/rtools-plugin/src/lib.rs`、`crates/rtools-plugin/tests/registry.rs`
- Modify: 根 `Cargo.toml`（`# internal` 加 `rtools-json`、`rtools-plugin`；`# ui` 加 `gpui-kit = { version = "0.9.1", features = ["syntect"] }`）

**Interfaces:**
- Produces:
  ```rust
  use gpui_kit::assets::IconName;

  pub struct PluginManifest { /* id, title, description, icon, keywords */ }
  impl PluginManifest {
      pub const fn new(id: &'static str, title: &'static str) -> Self;
      pub const fn description(self, description: &'static str) -> Self;
      pub const fn icon(self, icon: IconName) -> Self;
      pub const fn keywords(self, keywords: &'static [&'static str]) -> Self;
      pub fn id(&self) -> &'static str;   // + title(), description_text(), icon_name(), keyword_list()
  }

  pub struct Launch { /* input: Option<String>, data_dir: PathBuf */ }
  impl Launch {
      pub fn new(data_dir: PathBuf) -> Self;
      pub fn with_input(self, input: Option<String>) -> Self;
      pub fn data_dir(&self) -> &Path;
      pub fn take_input(&mut self) -> Option<String>;
  }

  pub trait Plugin: 'static {
      fn manifest(&self) -> &PluginManifest;
      fn accepts(&self, _input: &str) -> Option<SharedString> { None }
      fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> AnyView;
  }

  #[derive(Debug, Clone, PartialEq)]
  pub enum MatchKind { Keyword, Content(SharedString) }
  #[derive(Debug, Clone, PartialEq)]
  pub struct PluginMatch { /* index, kind */ }   // readers: index(), kind()

  pub struct PluginRegistry { plugins: Vec<Box<dyn Plugin>> }
  impl FromIterator<Box<dyn Plugin>> for PluginRegistry;
  impl PluginRegistry {
      pub fn plugin(&self, index: usize) -> &dyn Plugin;          // panics on bad index (indexes come from search)
      pub fn search(&self, query: &str) -> Vec<PluginMatch>;      // keyword hits, then content hits; empty → all
      pub fn search_content(&self, text: &str) -> Vec<PluginMatch>; // content hits only
  }
  ```
  读方法命名避免与 const 构造器冲突（`description_text`、`icon_name`、`keyword_list`）。`new` 的默认值：`description` 为空串、`icon` 为 `IconName::Puzzle`、`keywords` 为空。

- [ ] **Step 1: 写失败测试 `tests/registry.rs`**

```rust
use gpui_kit::assets::IconName;
use gpui_kit::{AnyView, App, SharedString, Window};
use rtools_plugin::{Launch, MatchKind, Plugin, PluginManifest, PluginRegistry};

struct Fake(PluginManifest, Option<&'static str>);

impl Plugin for Fake {
    fn manifest(&self) -> &PluginManifest { &self.0 }
    fn accepts(&self, input: &str) -> Option<SharedString> {
        self.1.filter(|_| input.starts_with('{')).map(SharedString::from)
    }
    fn open(&self, _: Launch, _: &mut Window, _: &mut App) -> AnyView { unreachable!("not opened in tests") }
}

fn registry() -> PluginRegistry {
    [
        Box::new(Fake(PluginManifest::new("json", "JSON 编辑器").icon(IconName::Braces).keywords(&["json"]), Some("识别为 JSON"))) as Box<dyn Plugin>,
        Box::new(Fake(PluginManifest::new("calc", "计算器").keywords(&["calc", "math"]), None)),
    ]
    .into_iter()
    .collect()
}

fn ids(r: &PluginRegistry, m: &[rtools_plugin::PluginMatch]) -> Vec<&'static str> {
    m.iter().map(|m| r.plugin(m.index()).manifest().id()).collect()
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
    assert_eq!(m[0].kind(), &MatchKind::Content("识别为 JSON".into()));
}

#[test]
fn search_content_ignores_keywords() {
    let r = registry();
    assert!(r.search_content("json").is_empty());
    assert_eq!(ids(&r, &r.search_content("{")), ["json"]);
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-plugin`
Expected：编译失败（crate 内容不存在）。

- [ ] **Step 3: 实现 `src/lib.rs`**

关键字匹配：`query.to_lowercase()` 是 `title` 或任一 keyword 的小写子串；同一插件既命中关键字又命中内容时只按 `Keyword` 出现一次。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-plugin && cargo clippy -p rtools-plugin --tests -- -Dwarnings`
Expected：4 个测试 PASS，无警告。

---

### Task 6: 命令行与单实例

**Files:**
- Create: `crates/rtools/src/lib.rs`（`pub mod cli; pub mod instance;`；后续任务各自追加自己的模块）、`crates/rtools/src/cli.rs`、`crates/rtools/src/instance.rs`、`crates/rtools/tests/instance.rs`
- Modify: `crates/rtools/Cargo.toml`（依赖 `interprocess`、`thiserror`）

**Interfaces:**
- Produces:
  ```rust
  // rtools::cli
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum ShellCommand { Toggle, Quit }
  impl FromStr for ShellCommand { type Err = UnknownCommand; }   // "toggle" | "quit", trimmed, ASCII case-insensitive
  impl fmt::Display for ShellCommand;                              // "toggle" / "quit"
  #[derive(Debug, thiserror::Error)]
  #[error("未知命令：{0}（可用：toggle、quit）")]
  pub struct UnknownCommand(String);

  // rtools::instance
  pub struct InstanceName(String);
  impl InstanceName {
      pub fn new(name: impl Into<String>) -> Self;
      /// `rtools-<USER or USERNAME or "default">.sock`
      pub fn for_current_user() -> Self;
      /// Connects to a running instance and writes one command line.
      pub fn send(&self, command: ShellCommand) -> io::Result<()>;
      /// Binds the listener (`try_overwrite(true)` to replace a stale socket).
      pub fn bind(&self) -> io::Result<InstanceServer>;
  }
  pub struct InstanceServer { /* listener */ }
  impl InstanceServer {
      /// Blocking iterator: one parsed command per accepted connection.
      pub fn incoming(self) -> impl Iterator<Item = io::Result<ShellCommand>>;
  }
  ```
  名字用 `interprocess::local_socket::GenericNamespaced`；无效命令行内容映射为 `io::ErrorKind::InvalidData`。

- [ ] **Step 1: 写失败测试 `tests/instance.rs`**

```rust
use rtools::cli::ShellCommand;
use rtools::instance::InstanceName;
use std::thread;

fn unique(tag: &str) -> InstanceName {
    InstanceName::new(format!("rtools-test-{}-{tag}.sock", std::process::id()))
}

#[test]
fn commands_parse_and_display() {
    assert_eq!(" Toggle ".parse::<ShellCommand>().unwrap(), ShellCommand::Toggle);
    assert_eq!("quit".parse::<ShellCommand>().unwrap().to_string(), "quit");
    assert!("open".parse::<ShellCommand>().unwrap_err().to_string().contains("toggle"));
}

#[test]
fn send_without_server_fails_fast() {
    assert!(unique("absent").send(ShellCommand::Toggle).is_err());
}

#[test]
fn server_round_trip() {
    let name = unique("roundtrip");
    let server = name.bind().unwrap();
    let reader = thread::spawn(move || server.incoming().take(2).collect::<Vec<_>>());
    name.send(ShellCommand::Toggle).unwrap();
    name.send(ShellCommand::Quit).unwrap();
    let got: Vec<_> = reader.join().unwrap().into_iter().map(Result::unwrap).collect();
    assert_eq!(got, [ShellCommand::Toggle, ShellCommand::Quit]);
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools --test instance`
Expected：编译失败。

- [ ] **Step 3: 实现 `cli.rs`、`instance.rs`**（`send`：`Stream::connect` 后写 `format!("{command}\n")`；`incoming`：对每个连接 `BufReader::read_line` 后 `parse`）

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools --test instance`
Expected：3 个测试 PASS。

---

### Task 7: 粘贴拦截模型

**Files:**
- Create: `crates/rtools/src/paste.rs`、`crates/rtools/tests/paste.rs`
- Modify: `crates/rtools/src/lib.rs`（`pub mod paste;`）

**Interfaces:**
- Produces:
  ```rust
  /// Single-line pastes up to this size stay in the search box.
  pub const PASTE_INLINE_LIMIT: usize = 4 * 1024;
  pub struct PastedInput { text: String }
  impl PastedInput {
      /// `Some` for multi-line text or text larger than `PASTE_INLINE_LIMIT` bytes.
      pub fn intercept(text: &str) -> Option<Self>;
      pub fn text(&self) -> &str;
      pub fn into_text(self) -> String;
      /// "已粘贴 812 B" / "已粘贴 3.2 KB" / "已粘贴 1.5 MB" (1024-based, one decimal).
      pub fn size_label(&self) -> String;
      /// First 80 chars with whitespace runs collapsed to one space; "…" when cut.
      pub fn preview(&self) -> String;
  }
  ```

- [ ] **Step 1: 写失败测试 `tests/paste.rs`**

```rust
use rtools::paste::{PASTE_INLINE_LIMIT, PastedInput};

#[test]
fn intercepts_multiline_and_large_text() {
    assert!(PastedInput::intercept("json").is_none());
    assert!(PastedInput::intercept("{\n  \"a\": 1\n}").is_some());
    assert!(PastedInput::intercept(&"a".repeat(PASTE_INLINE_LIMIT)).is_none());
    assert!(PastedInput::intercept(&"a".repeat(PASTE_INLINE_LIMIT + 1)).is_some());
}

#[test]
fn size_labels() {
    let label = |n: usize| PastedInput::intercept(&format!("x\n{}", "a".repeat(n - 2))).unwrap().size_label();
    assert_eq!(label(812), "已粘贴 812 B");
    assert_eq!(label(3277), "已粘贴 3.2 KB");
    assert_eq!(label(1_572_864), "已粘贴 1.5 MB");
}

#[test]
fn preview_collapses_whitespace_and_truncates_by_char() {
    let p = PastedInput::intercept("{\n    \"城市\":   \"胡杨河市\"\n}").unwrap();
    assert_eq!(p.preview(), "{ \"城市\": \"胡杨河市\" }");
    let long = PastedInput::intercept(&format!("x\n{}", "胡".repeat(100))).unwrap();
    assert_eq!(long.preview().chars().count(), 81);
    assert!(long.preview().ends_with('…'));
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools --test paste`
Expected：编译失败。

- [ ] **Step 3: 实现 `paste.rs`**

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools --test paste`
Expected：3 个测试 PASS。

---

### Task 8: JSON 插件界面

**Files:**
- Create: `plugins/rtools-json/src/ui/mod.rs`（`JsonPlugin`）、`plugins/rtools-json/src/ui/view.rs`（`JsonPluginView`）
- Modify: `plugins/rtools-json/Cargo.toml`（加 `# internal` `rtools-plugin`、`# ui` `gpui-kit`、`# infra` `tracing`）、`plugins/rtools-json/src/lib.rs`（`pub mod ui;`）、`plugins/rtools-json/src/core/detect.rs`（`InputKind::label`）、`plugins/rtools-json/src/core/format.rs`（`array_len`）、`plugins/rtools-json/src/core/mod.rs`（`pub use format::array_len;`）
- Test: `plugins/rtools-json/tests/format.rs`（`array_len`）、`plugins/rtools-json/tests/detect.rs`（`label`）

**Interfaces:**
- Consumes: `PluginManifest`、`Plugin`、`Launch`（Task 5）；`Draft`（Task 4）；core 全部
- Produces:
  ```rust
  // rtools_json::core
  impl InputKind { pub fn label(self) -> &'static str }        // "JSON" | "XML" | "URL 参数" | "YAML"
  /// Element count when `text` is a JSON array, without building the values.
  pub fn array_len(text: &str) -> Option<usize>;
  // rtools_json::ui
  pub struct JsonPlugin;      // impl rtools_plugin::Plugin
  pub struct JsonPluginView;  // impl Render; built only through JsonPlugin::open
  ```

- [ ] **Step 1: 写失败测试（逻辑部分）**

```rust
// tests/format.rs
#[test]
fn array_len_counts_top_level_items_only() {
    use rtools_json::core::array_len;
    assert_eq!(array_len("[1, [2, 3], {\"a\": 4}]"), Some(3));
    assert_eq!(array_len("{\"a\": 1}"), None);
    assert_eq!(array_len("not json"), None);
}

// tests/detect.rs
#[test]
fn kinds_have_user_facing_labels() {
    assert_eq!(InputKind::UrlParams.label(), "URL 参数");
    assert_eq!(InputKind::Json.label(), "JSON");
}
```

Run: `cargo test -p rtools-json --no-fail-fast 2>&1 | grep -E 'FAILED|test result|^error'` → Expected：编译失败。实现 `label`、`array_len`（`serde_json::from_str::<Vec<serde::de::IgnoredAny>>`）后再跑 → Expected：全部 PASS。

- [ ] **Step 2: 实现 `JsonPlugin`（`ui/mod.rs`）**

- 清单：`PluginManifest::new("json", "JSON 编辑器").description("格式化、过滤、转换 JSON").icon(IconName::Braces).keywords(&["json", "jsoneditor"])`。
- `accepts`：`InputKind::sniff` 命中 → `Some(format!("识别为 {}", kind.label()))`；否则输入含 `\n` 且 `input.parse::<Normalized>()` 成功 → 同样返回；其余 `None`。
- `open`：`cx.new(|cx| JsonPluginView::new(launch, window, cx)).into()`。

- [ ] **Step 3: 实现 `JsonPluginView`（`ui/view.rs`）**

结构：持有 `left: Entity<EditorState>`、`right: Entity<EditorState>`（只读）、`filter: Entity<InputState>`、`engine: Option<FilterEngine>`、`formatter: JsonFormatter`、`draft: Draft`、`pane: ResultPane`、`doc_error: Option<JsonError>`、`latest: u64`（过滤请求序号）、`filter_task`/`save_task`/`sync_task: Task<()>`、`_subscriptions`。`enum ResultPane { Hidden, Filter(Result<String, JsonError>), Xml(Result<String, JsonError>) }`。

行为（逐条对应 spec §5.1–§5.7）：
1. 初始内容：`launch.take_input()` 有值 → `parse::<Normalized>()`，成功用其文本，失败用原文并把错误放入 `doc_error`；否则 `draft.load()`（出错 `tracing::warn!` 并当作空）。
2. 左侧 `InputEvent::Change`：1 s 后 `draft.save`（失败 `tracing::warn!`，不影响内容）；300 ms 后校验原文（`formatter.minify` 的错误写入 `doc_error`）并 `engine.set_document`，随后若过滤框非空重新求值；`pane` 从 `Xml` 回到过滤模式。
3. 过滤框变化：200 ms 防抖 → `latest += 1` → `engine.evaluate(expr)` → 结果到达时序号不等于 `latest` 则丢弃；过滤框为空且不在 `Xml` 模式 → `Hidden`。
4. 工具栏：格式化/压缩/转义/去转义对左侧文本调用对应 `JsonFormatter` 方法，成功用 `left.replace_all`（可撤销），失败写入 `doc_error`。转 XML：过滤框非空且结果为 `Ok` 时转换过滤结果，否则转换左侧；`Value` 解析后 `AsXml(&v).to_string()` → `ResultPane::Xml`。
5. 结果栏：`Filter(Ok)`/`Xml(Ok)` 用 `right.set_value` 显示；标题「过滤结果」或「XML」，`array_len` 有值时加「· N 项」，「复制」按钮写剪贴板（`ClipboardItem::new_string`）。`Err` 显示居中错误态，标题行不显示项数与复制。
6. 左侧粘贴：`Editor::on_paste` 闭包捕获 `WeakEntity<Self>`；文档为空或 `selected_range` 覆盖全文时，`Normalized::from_str(clipboard)` 成功 → `replace_all(normalized.text)` 并返回 `true`；否则返回 `false` 走默认插入。
7. 渲染：严格按 spec §5.7「JSON 插件」（两编辑器 `bordered(false).appearance(false)`、结果栏左分隔线与标题行、底栏 `this` 前缀过滤框与三组文字按钮、左侧错误条）。视图本身不渲染顶栏（顶栏归外壳）。
8. `FilterEngine::spawn` 失败：`tracing::error!`，`engine = None`，过滤结果显示「过滤引擎不可用」错误态。

- [ ] **Step 4: 编译与静态检查**

Run: `cargo build --workspace && cargo clippy --workspace --tests --examples -- -Dwarnings && cargo test --workspace --no-fail-fast 2>&1 | grep -E 'FAILED|test result'`
Expected：构建成功、无警告、测试全部通过。界面运行核对在 Task 9 完成。

---

### Task 9: 启动器外壳与主程序

**Files:**
- Create: `crates/rtools/src/shell.rs`（`Shell` 全局）、`crates/rtools/src/launcher.rs`（`Launcher` 视图）、`crates/rtools/src/assets.rs`（`AppAssets`）、`crates/rtools/src/theme.rs`
- Modify: `crates/rtools/src/main.rs`、`crates/rtools/src/lib.rs`（加 `pub mod assets; pub mod launcher; pub mod shell; pub mod theme;`）、`crates/rtools/Cargo.toml`（加 `gpui-kit`、`rtools-plugin`、`rtools-json`、`dirs`、`futures`、`tracing`、`tracing-subscriber`（feature `env-filter`））
- Modify: 根 `Cargo.toml`（`# platform`、`# infra` 新依赖）

**Interfaces:**
- Consumes: `PluginRegistry`/`PluginMatch`/`MatchKind`/`Launch`（Task 5）、`ShellCommand`/`InstanceName`（Task 6）、`PastedInput`（Task 7）、`JsonPlugin`（Task 8）
- Produces:
  ```rust
  // rtools::theme
  /// Product theme tokens: window radius 12px, editor + gutter backgrounds = theme background.
  pub fn apply(cx: &mut App);
  // rtools::assets
  icon_assets!(ExtraIcons, [Braces, Power, AppWindow]);   // gpui_kit::assets::icon_assets
  pub struct AppAssets;                                      // ExtraIcons first, then Assets
  // rtools::shell
  pub struct Shell { /* registry: Rc<PluginRegistry>, data_root: PathBuf, launcher: Option<Entity<Launcher>>, popup: Option<AnyWindowHandle> */ }
  impl Global for Shell {}
  impl Shell {
      pub fn init(registry: PluginRegistry, data_root: PathBuf, cx: &mut App);
      pub fn toggle(cx: &mut App);             // close if open, else open popup (reusing the Launcher entity)
      pub fn close_popup(cx: &mut App);
      pub fn detach(view: AnyView, title: SharedString, cx: &mut App); // normal titled window
  }
  // rtools::launcher
  actions!(launcher, [SelectPrev, SelectNext, Confirm, Back, Hide, Detach]);
  pub fn bind_keys(cx: &mut App);
  pub struct Launcher { /* query, pasted, rows, selected, mode, plugin cache, … */ }
  ```

- [ ] **Step 1: 主题与资源**

`theme::apply`：`Theme::update` 设 `window_radius = px(12.)`（注释说明是产品主题 token），并克隆 `highlight_theme` 把 `style.editor_background`、`style.editor_gutter_background` 设为 `Some(theme.background)`。`AppAssets` 实现 `AssetSource`：`load`/`list` 先查 `ExtraIcons` 再查 `gpui_kit::assets::Assets`。

- [ ] **Step 2: 窗口选项**

`Shell` 内部方法 `popup_options(&self, content: Size<Pixels>, cx: &App) -> WindowOptions`：
- `cx.compositor_name() == "Wayland"` → `WindowKind::LayerShell`（`Layer::Overlay`、`Anchor::empty()`、`KeyboardInteractivity::OnDemand`、namespace `"rtools"`）；否则 `WindowKind::PopUp`，bounds 用 `Bounds::centered(None, size, cx)`。`LayerShell` 用 `#[cfg(target_os = "linux")]` 包住。
- 都设 `titlebar: None`、`window_background: Transparent`、`window_decorations: Some(Client)`；窗口尺寸 = 内容尺寸 + `window_border::window_paddings` 的阴影内边距。
- 分离窗口：`WindowKind::Normal`、`titlebar` 标题为插件名、默认尺寸 960×680。

- [ ] **Step 3: `Launcher` 视图**

- 状态：`query: Entity<InputState>`（占位符「搜索插件，或粘贴内容」，粘贴后「已粘贴内容，选择处理方式」）、`pasted: Option<PastedInput>`、`rows: Vec<Row>`（`enum Row { Plugin(PluginMatch), Quit }`，分组：插件 / 系统；有粘贴时只列「可处理此内容」）、`selected: usize`、`mode: Mode`（`List` | `Plugin { id, title, icon }`）、`views: HashMap<&'static str, AnyView>`。
- 查询 `InputEvent::Change` → 重算 `rows`（`registry.search` 或 `search_content(pasted)`），`selected = 0`，同步窗口高度。
- `Input::on_paste`：`PastedInput::intercept` 命中 → 存入 `pasted`、返回 `true`。
- 动作（`bind_keys`）：`up`/`down`/`enter` → `SelectPrev`/`SelectNext`/`Confirm`，上下文 `"LauncherList"`；`escape` 在 `"LauncherList"` → `Hide`（有粘贴标签时先清标签），在 `"LauncherPlugin"` → `Back`；`secondary-d` 在 `"LauncherPlugin"` → `Detach`。
- `Confirm`：`Row::Quit` → `cx.quit()`；关键字命中 → 复用 `views` 中的视图或 `plugin.open(Launch::new(dir), …)`；内容命中 → 总是新建视图，`Launch::new(dir).with_input(Some(原文))`（原文 = 粘贴文本或查询文本），覆盖缓存。`dir = data_root/<id>`，先 `create_dir_all`（失败 `tracing::warn!`）。进入后 `mode = Plugin`，窗口改为 800×600。
- `Back`：`mode = List`，焦点回到查询框，窗口高度随列表。`Detach`：从 `views` 取出视图 → `Shell::detach` → `mode = List` → `Shell::close_popup`。
- 失焦（`observe_window_activation`，`!is_window_active()`）→ `Shell::close_popup`。
- 高度：由 rem 常量计算，乘 `window.rem_size()`，通过 `window.resize` 同步；常量集中定义并注释为窗口尺寸边界。初值（按截图微调，保持相对关系）：搜索框行 3.25、分组标题 2、行 3.5、列表上下内边距合计 0.5、底栏 2.75、粘贴标签行 2.25、空态 6（单位 rem）；宽度 50 rem。
- 渲染：严格按 spec §5.7「窗口 / 启动器 / 插件顶栏」。行用 `ListItem::new(row_id).selected(i == selected).on_click(...)`，`row_id` 用插件 id（不用下标）。

- [ ] **Step 4: `main.rs`**

1. 参数：无参数 → `ShellCommand::Toggle`；有参数 → `parse`，失败打印错误并 `std::process::exit(2)`。
2. `tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).init()`。
3. `InstanceName::for_current_user().send(command)` 成功 → 退出；`Quit` 且无实例 → 打印「rtools 未在运行」退出。
4. 否则 `bind()`（失败 `tracing::error!` 后退出），后台线程读取 `incoming()`，经 `futures::channel::mpsc::unbounded` 转给应用任务：`Toggle` → `Shell::toggle`，`Quit` → `cx.quit()`。
5. `application().with_assets(AppAssets).with_quit_mode(QuitMode::Explicit).run(...)`：`gpui_kit::init` → `Theme::sync_system_appearance(None, cx)` → `theme::apply` → `launcher::bind_keys` → `Shell::init([Box::new(JsonPlugin)].into_iter().collect(), dirs::data_dir()/rtools, cx)` → `Shell::toggle`（首次启动直接显示）。每次 `Shell::toggle` 打开弹窗前再执行一次 `sync_system_appearance` + `theme::apply`。
6. 生命周期日志：启动、IPC 绑定、打开插件、分离、退出；不记录用户内容。

- [ ] **Step 5: 运行核对（截图，明暗各一套）**

Run: `cargo build --release -p rtools && ./target/release/rtools &`
逐项核对并截图：
- 启动器列表：圆角、阴影、分组、图标块、选中行、底栏提示，与原型一致。
- 输入 `js` 只剩 JSON 编辑器；输入 `zzz` 显示「没有匹配的插件」。
- `Enter` 进入 JSON 插件（800×600）；左侧显示草稿或空；`Esc` 返回列表；再 `Esc` 隐藏。
- **状态恢复**：进入插件输入一些 JSON → 点击窗口外（失焦关闭）→ `./target/release/rtools toggle` → 再次进入插件，内容仍在。
- `wl-copy < target/spike/big.json`（S2 生成的 4.5 MB 文件，若已清理则用 Python 重新生成）后在搜索框 `Ctrl+V`：出现「已粘贴 4.4 MB」标签且不卡；`Enter` 后插件以格式化内容打开。
- 过滤：输入 `.length`、快速改成 `.map(x => x.name)`，结果栏只显示最后一次的结果。
- 工具栏各按钮 + `Ctrl+Z` 撤销格式化；转 XML；复制。
- `Ctrl+D` 分离成带标题栏的窗口，可继续编辑。
- `./target/release/rtools quit` 退出。

---

### Task 10: 全局快捷键

**Files:**
- Create: `crates/rtools/src/hotkey.rs`
- Modify: `crates/rtools/src/lib.rs`（`pub mod hotkey;`）、`crates/rtools/src/main.rs`（启动 `HotkeyService`）、`crates/rtools/Cargo.toml`（`global-hotkey`；Linux 目标下 `ashpd`）

**Interfaces:**
- Produces:
  ```rust
  pub struct HotkeyService { /* manager: Option<GlobalHotKeyManager>, portal: Option<Task<()>> */ }
  impl HotkeyService {
      /// Registers Alt+Space: XDG GlobalShortcuts portal on Wayland, global-hotkey elsewhere.
      /// Failures are logged with the `rtools toggle` fallback hint; the service then does nothing.
      pub fn start(on_toggle: futures::channel::mpsc::UnboundedSender<()>, cx: &mut App) -> Self;
  }
  ```
  `main.rs` 持有返回的 `HotkeyService`（放进 `Shell` 或 app 闭包里保活），接收端循环调用 `Shell::toggle`。

- [ ] **Step 1: 实现**

- 非 Wayland：主线程 `GlobalHotKeyManager::new()`，`register(HotKey::new(Some(Modifiers::ALT), Code::Space))`；`GlobalHotKeyEvent::set_event_handler(Some(move |e| if e.state == HotKeyState::Pressed { let _ = tx.unbounded_send(()); }))`。
- Wayland（`#[cfg(target_os = "linux")]` 且 `cx.compositor_name() == "Wayland"`）：`cx.background_spawn` 中 `GlobalShortcuts::new().await` → `create_session(Default::default())` → `bind_shortcuts(&session, &[NewShortcut::new("toggle", "呼出 rtools").preferred_trigger(Some("ALT+space"))], None, Default::default())`（对 `Request` 取 `response()`）→ `receive_activated()` 流中 `shortcut_id() == "toggle"` 时发送；`session` 在任务内保活。
- 任一步失败：`tracing::warn!("全局快捷键不可用（{}），请在系统设置中把快捷键绑定到命令 `rtools toggle`", err)`。

- [ ] **Step 2: 运行核对**

Run: `cargo build --release -p rtools && ./target/release/rtools &`
Expected：KDE Wayland 首次启动弹出系统的快捷键确认；按 `Alt+Space` 切换弹窗显示/隐藏。X11/macOS/Windows 本机无法验证，在最终报告中注明。

---

### Task 10b: 双击 Ctrl 呼出（2026-11-13 用户追加）

**Files:**
- Create: `crates/rtools/src/hotkey/mod.rs`（由原 `hotkey.rs` 改为目录模块）、`crates/rtools/src/hotkey/double_tap.rs`、`crates/rtools/src/hotkey/evdev_source.rs`（Linux）、`crates/rtools/src/hotkey/rdev_source.rs`（macOS/Windows）、`crates/rtools/tests/double_tap.rs`
- Modify: 根 `Cargo.toml`（`# platform` 加 `evdev`、`rdev`）、`crates/rtools/Cargo.toml`（Linux 目标 `evdev`；macOS/Windows 目标 `rdev`）

**Interfaces:**
- Produces:
  ```rust
  // rtools::hotkey::double_tap
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum KeyKind { Ctrl, Other }
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum KeyAction { Press, Release, Repeat }
  #[derive(Debug, Clone, Copy)] pub struct KeyEvent { pub kind: KeyKind, pub action: KeyAction, pub at: Instant }
  #[derive(Default)] pub struct DoubleTap { /* pressed_at, interrupted, last_tap, armed */ }
  impl DoubleTap {
      pub const TAP_MAX: Duration;   // 300 ms
      pub const GAP_MAX: Duration;   // 350 ms
      /// Feeds one key event; true when it completes a double tap of Ctrl.
      pub fn feed(&mut self, event: KeyEvent) -> bool;
  }
  ```
  `HotkeyService::start` 签名不变；Linux 先试 evdev，失败按平台退回（Wayland → portal `Alt+Space`，X11 → `global-hotkey` `Alt+Space`），并 `tracing::warn!` 提示 `sudo usermod -aG input $USER`。

- [ ] **Step 1: 写失败测试 `tests/double_tap.rs`**：两次快速单击触发一次；`Ctrl+C`（Ctrl 按下期间按其他键）后的单击不触发；两次单击间隔 500 ms 不触发；按住 600 ms 不算单击；按住期间的 `Repeat` 被忽略；三连击只触发一次。
- [ ] **Step 2: 运行确认失败** → `cargo test -p rtools --test double_tap`
- [ ] **Step 3: 实现 `DoubleTap`，测试通过**
- [ ] **Step 4: evdev 源**：`evdev::enumerate()` 中支持 `KEY_LEFTCTRL` 与 `KEY_A` 的设备视为键盘；一个都打不开即返回错误。每个键盘一个阻塞读线程，把 `KeyEvent` 送入同一个 `std::sync::mpsc`；一个判定线程跑 `DoubleTap`，触发时向 `on_toggle` 发送。不记录任何按键。
- [ ] **Step 5: rdev 源**（macOS/Windows）：`rdev::listen` 在专用线程中运行，`ControlLeft`/`ControlRight` 映射为 `Ctrl`，其余按键为 `Other`；`listen` 返回错误时 `tracing::warn!`（macOS 提示授予辅助功能权限）。本机无法验证，最终报告注明。
- [ ] **Step 6: 运行核对**：用户加入 `input` 组并重新登录后，双击 Ctrl 能切换弹窗；`Ctrl+C` 等组合键不会误触发。

---

### Task 10c: 系统托盘（2026-11-13 用户追加，spec §4.5）

**Files:**
- Create: `assets/icons/rtools.svg`、`assets/icons/rtools-tray.svg`、`assets/icons/*.png`、`packaging/render-icons.sh`、`crates/rtools/src/tray/mod.rs`、`crates/rtools/src/tray/linux.rs`、`crates/rtools/src/tray/desktop.rs`（macOS/Windows）
- Modify: 根 `Cargo.toml`（`# platform` 加 `ksni`、`tray-icon`；`# data formats` 加 `image`）、`crates/rtools/Cargo.toml`、`crates/rtools/src/lib.rs`、`crates/rtools/src/main.rs`、`packaging/linux/dev.rtools.rtools.desktop`（`Icon=dev.rtools.rtools`）

**Interfaces:**
- Consumes: `ShellCommand`（Task 6）、IPC 命令通道（Task 9 `main.rs`）
- Produces:
  ```rust
  // rtools::tray
  /// Shows the tray icon; clicks and menu items arrive as ShellCommands on `commands`.
  pub fn start(commands: UnboundedSender<ShellCommand>, data_root: &Path, cx: &mut App) -> anyhow::Result<Tray>;
  pub struct Tray { /* keeps the platform tray alive */ }
  ```

- [ ] **Step 1: 资源**：写入两个 SVG，运行 `packaging/render-icons.sh` 生成 PNG。
- [ ] **Step 2: Linux（ksni）**：`Tray` 实现 `id`/`title`/`icon_theme_path`/`icon_name`/`icon_pixmap`/`tool_tip`/`activate`/`menu`；启动时把内嵌 `rtools-tray.svg` 写入 `<data_root>/icons/hicolor/scalable/apps/`；`cx.background_spawn` 中 `spawn()`，保留 `Handle`。
- [ ] **Step 3: macOS/Windows（tray-icon）**：主线程 `TrayIconBuilder` + 菜单；`MenuEvent`/`TrayIconEvent` 回调转发 `ShellCommand`。本机无法编译验证，最终报告注明。
- [ ] **Step 4: 接入 `main.rs`**：与 IPC 共用 `ShellCommand` 通道；失败 `tracing::warn!`。
- [ ] **Step 5: 运行核对**：截图 KDE 面板（浅/深色主题），左键切换启动器，右键菜单两项均生效。

---

### Task 11: 全量验证、设计核对、最终审查与唯一提交

**Files:** 无新增（只修复验证中发现的问题）

- [ ] **Step 1: 全量检查**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E 'FAILED|test result' && cargo clippy --workspace --tests --examples -- -Dwarnings && cargo fmt --all --check`
Expected：全部通过，无警告。

- [ ] **Step 2: 设计核对**

对 Task 9 的截图逐条运行 Design Guides 的「Design review checklist」8 项与「Accessibility checklist」，并核对 spec §5.7 每一条；不符合项当场修复后重新截图。

- [ ] **Step 3: 用户走查**

请用户按 Task 9 Step 5 清单亲自操作一遍（含 `Alt+Space`），记录反馈并修复。

- [ ] **Step 4: 最终审查 + 唯一提交**

按 executing-plans 的 Final Review 派最强模型审查整个改动（`git diff HEAD` 的工作区改动），Critical/Important 按 RED→GREEN 修复。然后：

```bash
git add -A
git commit -m "feat(shell): add launcher shell and json plugin ui"
```

提交前确认 `git status` 中没有 `target/` 或临时文件。
