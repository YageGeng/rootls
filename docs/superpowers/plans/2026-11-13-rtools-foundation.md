# rtools Foundation Implementation Plan（计划 1/2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 跑完 spec §8 的 4 个 spike 并把结论回写 spec；搭好 Cargo workspace；完成 `rtools-json` 的纯逻辑核心（格式化、XML/URL/YAML 转换、粘贴识别、JS 过滤引擎），全部有测试。

**Architecture:** 根 `Cargo.toml` 改为虚拟 workspace（`crates/*`、`plugins/*`）。`plugins/rtools-json/src/core/` 是不依赖 gpui 的纯逻辑模块，对外通过 `rtools_json::core::*` 暴露，测试放在 `plugins/rtools-json/tests/`。外壳、`rtools-plugin` 与 JSON 插件 UI 依赖 spike 结论，放在计划 2，等 spike 结论出来后再写。

**Tech Stack:** Rust 最新 stable（edition 2024；执行时为 1.100.0）、serde_json 1.0、quick-xml 0.42、serde-saphyr 2.1、form_urlencoded 1.2、rquickjs 0.14、futures 0.3、thiserror 2.0；spike 额外用 gpui-kit 0.9.1。

**Spec:** `docs/superpowers/specs/2026-11-13-rtools-design.md`

**Branch:** 直接在 `main` 上实现。

## Global Constraints

- 工具链：`rust-toolchain.toml` 使用 `stable`（跟随最新稳定版）；edition `2024`；workspace resolver `"3"`。
- `AGENTS.md`：新增/修改的非平凡逻辑写英文注释，每个新函数写英文函数级文档注释。
- `AGENTS.md`：克隆 `Arc<T>` 字段写 `Arc::clone(&self.field)`。
- `AGENTS.md`：测试只能放在 crate 的 `tests/` 目录或 `#[cfg(test)] mod tests`；生产代码里不得有仅为测试存在的代码。
- `AGENTS.md`：所有依赖在根 `[workspace.dependencies]` 声明；内部 crate 在最前；分组间空一行并加分类注释；组内按字母排序；子 crate 一律 `{ workspace = true }`。
- `AGENTS.md`：tracing 宏写全路径（`tracing::warn!("...")`），不用结构化字段语法；只在生命周期边界与错误路径记日志，不记录用户 JSON 与表达式全文。
- `AGENTS.md`：共享上下文的操作做成持有上下文的 struct 的方法；无上下文的叶子算法可以是自由函数；单参数且参数是项目类型的 helper 写成该类型的方法。
- 提交信息遵循 `.gitmessage`：`<type>(<scope>): <short imperative summary>`，只写标题。
- 每次提交前：`cargo fmt --all` 与 `cargo clippy --workspace --tests --examples -- -Dwarnings` 通过（pre-commit 会自动跑）。
- `plugins/rtools-json/src/core/` 不得 `use gpui` 或 `gpui_kit`。
- 缩进 4 空格（`INDENT = 4`）；过滤超时 2 秒（`FILTER_TIMEOUT`）；QuickJS 内存上限 1 GiB（`FILTER_MEMORY_LIMIT = 1 << 30`）。
- 过滤语义：执行 `(function(){ return eval(code) }).call(doc)`，`code = "this" + 用户输入`；输出 `JSON.stringify(result, null, 4)`，无法序列化时 `String(result)`。
- `serde_json` 开启 `preserve_order` 与 `arbitrary_precision`。
- 面向用户的错误文案用中文（显示在状态栏）。

## Review Focus

1. 写坏的 JSON（尾随逗号、缺引号）粘贴进来，必须返回带行列号的解析错误，不能被 YAML/URL 规则静默"转换"成别的东西 → Task 5 `broken_json_reports_parse_error_instead_of_yaml`。
2. 数字原文（`1.50`、`1e3`、`12345678901234567890123`）经格式化/压缩后保持不变（左侧原文被原地改写） → Task 3 `number_text_is_preserved`。
3. 中文与 emoji（截图里就是中文数据）在格式化、转义往返、过滤结果中保持原样、不被 `\u` 转义 → Task 3 `unicode_is_kept_verbatim`、Task 6 `unicode_keys_and_values`。
4. 一次超时之后过滤引擎必须还能继续工作（中断不能把 runtime 弄坏） → Task 6 `timeout_then_recovers`。
5. 大文档（约 8 MB，就是截图的场景）上过滤：`JSON.parse` 不计入 2 秒超时，过滤必须能出结果 → Task 6 `large_document_is_not_cut_by_timeout`。

---

### Task 1: Spike S1–S4（代码用完即弃，只提交 spec 结论）

**Files:**
- Create (throwaway, git-ignored): `target/spike/Cargo.toml`、`target/spike/rust-toolchain.toml`（复制根目录那份）、`target/spike/src/main.rs`、`target/spike/gen.py`
- Modify: `docs/superpowers/specs/2026-11-13-rtools-design.md` §8（表格加一列「结论」）

**Interfaces:**
- Consumes: 无
- Produces: spec §8 的结论，计划 2 依赖以下事实：gpui 的导入路径与版本；`EditorState` 的折叠 API 名（或"无"）；JSON 高亮引擎；弹窗隐藏机制（隐藏 vs 关闭重开）；`AnyView` 能否跨窗口（决定 `Plugin` 契约是否要加状态导出方法）；可撤销的整体替换 API 名（或"无"）；识别整文档替换的事件用法；`Ctrl+D` 在 `Editor` 中的默认绑定；单行 `Input` 粘贴多行/大段文本的表现；release 下 `serde_json` 格式化 8 MB 的耗时。

- [ ] **Step 1: 建 spike 工程**

`target/spike/Cargo.toml` 顶部写空的 `[workspace]`（与仓库 workspace 隔离），依赖 `gpui-kit = { version = "0.9.1", features = ["syntect"] }`。运行 `cd target/spike && cargo build`。
Expected：在 1.97.1 下编译成功。失败时记录报错和所需的最低 Rust 版本；若原因是缺少系统库（wayland、xkbcommon、vulkan、fontconfig 等），列出缺少的包，请用户用 `pacman` 安装后重试。

- [ ] **Step 2: S1 —— gpui 导入路径与 LayerShell**

在 `~/.cargo/registry/src/*/` 中找到 gpui-kit 依赖的 gpui 源码，确认：gpui 版本、是否以 `gpui_kit::gpui` 重新导出、`WindowKind::LayerShell` 与 `gpui::layer_shell::LayerShellOptions` 是否存在、是否需要开启 `wayland` feature。
Expected：在 spike 的 `main.rs` 里引用这些类型能编译通过。

- [ ] **Step 3: S2 —— 大文件编辑器**

`gen.py` 生成约 21 万行的 JSON（形如 `[{"city": "胡杨河市", "name": "兵团一二五团", "id": "659010500000"}, …]`，缩进 4）到 `target/spike/big.json`。窗口左右各放一个 `Editor`：`EditorState::new(window, cx).language("json").line_number(true).folding(true)`，左侧 `default_value(big.json)`，右侧 `readonly(true)`。
记录：从启动到首帧的耗时（用 `Instant` 打印）；按住 PageDown 滚动是否卡顿（**需要人工观察**）；是否有 JSON 高亮；在 gpui-kit 源码中 grep `fold` 找"全部折叠/展开"的公开 API。
另外确认：
- 有没有能进撤销历史的整体替换接口（grep `undo`、`replace`、`transact`），以及 `set_value` 之后 `Ctrl+Z` 的行为。
- 订阅 `InputEditsEvent`，全选后粘贴时，能否判断出"本次编辑替换了整个文档"。
- `Editor` 默认键位里 `Ctrl+D` / `Cmd+D` 绑定了什么。
- 窗口里加一个单行 `Input`，粘贴 `big.json` 的前 100 行和整个文件：换行是否保留，界面是否卡顿（**需要人工观察**）。
- spike 临时加 `serde_json`（`arbitrary_precision`、`preserve_order`），用 `cargo run --release` 测一次 `big.json` 的解析 + 美化输出耗时。

- [ ] **Step 4: S3 —— KDE Wayland 弹窗**

用 `WindowKind::LayerShell`（不锚定、`KeyboardInteractivity::OnDemand`）开一个 800×60 的窗口，订阅窗口激活状态变化，失焦时打印日志并尝试隐藏；再试一次关闭后重新打开。
记录：失焦事件是否触发；有无可用的隐藏 API（若无，则关闭重开是否可行）。macOS/Windows 本机无法验证，在结论里标注"未验证，按 `WindowKind::PopUp` 假设"。

- [ ] **Step 5: S4 —— 视图跨窗口**

在窗口 A 中创建一个持有 `EditorState` 的视图实体，转为 `AnyView`；关闭 A，在新窗口 B 中渲染同一个 `AnyView`；再试一次把它同时从弹窗移到另一个普通窗口。
记录：B 中能否正常渲染、获得焦点、输入文字。若不行，结论写明"`Plugin` 契约需增加状态导出方法"。

- [ ] **Step 6: 回写结论并提交**

在 spec §8 表格增加「结论」列，逐项写结论（S1 写明版本号与导入路径）。

```bash
git add docs/superpowers/specs/2026-11-13-rtools-design.md
git commit -m "docs(spec): record spike S1-S4 findings"
```

---

### Task 2: Workspace 骨架

**Files:**
- Modify: `Cargo.toml`（改为虚拟 workspace）
- Create: `crates/rtools/Cargo.toml`、`crates/rtools/src/main.rs`（内容从 `src/main.rs` 原样移过来）
- Delete: `src/main.rs`、`src/`
- Create: `plugins/rtools-json/Cargo.toml`、`plugins/rtools-json/src/lib.rs`、`plugins/rtools-json/src/core/mod.rs`

**Interfaces:**
- Consumes: 无
- Produces: crate `rtools_json`，带 `pub mod core;`。后续任务在 `core/mod.rs` 中添加子模块和 `pub use` 重新导出，测试通过 `rtools_json::core::…` 引用。

- [ ] **Step 1: 先单独提交仓库配置文件**

这些文件已经由用户写好，只需纳入版本控制（`CLAUDE.md` 是指向 `AGENTS.md` 的软链接，按软链接提交）：

```bash
git add AGENTS.md CLAUDE.md RTK.md .codex .rtk .gitignore .gitmessage .pre-commit-config.yaml .typos.toml rust-toolchain.toml
git commit -m "chore(repo): add project configuration files"
```

- [ ] **Step 2: 写根 `Cargo.toml`**

```toml
[workspace]
resolver = "3"
members = ["crates/*", "plugins/*"]

[workspace.package]
edition = "2024"
version = "0.1.0"

[workspace.dependencies]
# internal
rtools-json = { path = "plugins/rtools-json" }

# script
rquickjs = "0.14"

# data formats
form_urlencoded = "1.2"
quick-xml = "0.42"
serde-saphyr = "2.1"
serde_json = { version = "1.0", features = ["arbitrary_precision", "preserve_order"] }

# infra
futures = "0.3"
thiserror = "2.0"
```

本计划只声明本计划用到的依赖，其余依赖在计划 2 中加入。

- [ ] **Step 3: 写子 crate 清单**

`crates/rtools/Cargo.toml`：`name = "rtools"`，`edition.workspace = true`，`version.workspace = true`，暂时没有依赖。
`plugins/rtools-json/Cargo.toml`：`name = "rtools-json"`，依赖分组同根目录：`# script` rquickjs；`# data formats` form_urlencoded、quick-xml、serde-saphyr、serde_json；`# infra` futures、thiserror，全部写 `{ workspace = true }`。
`src/lib.rs` 只写 `pub mod core;`，带英文 crate 级文档注释；`src/core/mod.rs` 写一行模块文档注释，说明"pure logic, must not depend on gpui"。
移动入口：`mkdir -p crates/rtools/src && mv src/main.rs crates/rtools/src/main.rs && rmdir src`（`src/` 从未被提交过，直接移动即可）。

- [ ] **Step 4: 验证**

Run: `cargo build --workspace && cargo clippy --workspace --tests -- -Dwarnings && cargo run -p rtools`
Expected：构建成功，clippy 无警告，输出 `Hello, world!`。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml Cargo.lock crates plugins
git commit -m "build(workspace): split into crates and plugins workspace"
```

---

### Task 3: `JsonError` + `JsonFormatter`

**Files:**
- Create: `plugins/rtools-json/src/core/error.rs`、`plugins/rtools-json/src/core/format.rs`
- Modify: `plugins/rtools-json/src/core/mod.rs`（`mod error; mod format; pub use error::JsonError; pub use format::{INDENT, JsonFormatter};`）
- Test: `plugins/rtools-json/tests/format.rs`

**Interfaces:**
- Consumes: Task 2 的 crate
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, thiserror::Error)]
  pub enum JsonError {
      #[error("JSON 解析失败（第 {line} 行第 {column} 列）：{message}")]
      Parse { line: usize, column: usize, message: String },
      #[error("XML 解析失败：{0}")]  Xml(String),
      #[error("无法识别的输入格式")]  Unrecognized,
      #[error("表达式错误：{0}")]    Script(String),
      #[error("执行超时")]           Timeout,
      #[error("过滤引擎已停止")]     EngineStopped,
  }
  impl From<serde_json::Error> for JsonError   // -> Parse { line, column, message }; message drops serde_json's " at line N column M" suffix

  pub const INDENT: usize = 4;
  pub struct JsonFormatter { indent: usize }   // Default: indent = INDENT
  impl JsonFormatter {
      pub fn format(&self, src: &str) -> Result<String, JsonError>;
      pub fn minify(&self, src: &str) -> Result<String, JsonError>;
      pub fn escape(&self, src: &str) -> Result<String, JsonError>;
      pub fn unescape(&self, src: &str) -> Result<String, JsonError>;
      pub fn format_value(&self, value: &serde_json::Value) -> String;
  }
  ```

- [ ] **Step 1: 写失败的测试 `tests/format.rs`**

```rust
use rtools_json::core::{JsonError, JsonFormatter};

#[test]
fn format_keeps_key_order_with_four_space_indent() {
    let out = JsonFormatter::default().format(r#"{"b":1,"a":[1,2],"c":{}}"#).unwrap();
    assert_eq!(out, "{\n    \"b\": 1,\n    \"a\": [\n        1,\n        2\n    ],\n    \"c\": {}\n}");
}

#[test]
fn minify_round_trips_format() {
    let f = JsonFormatter::default();
    let src = r#"{"b":1,"a":[1,2],"c":{}}"#;
    assert_eq!(f.minify(&f.format(src).unwrap()).unwrap(), src);
}

#[test]
fn number_text_is_preserved() {
    let src = r#"{"big":12345678901234567890123,"dec":1.50,"exp":1e3}"#;
    let f = JsonFormatter::default();
    assert_eq!(f.minify(&f.format(src).unwrap()).unwrap(), src);
}

#[test]
fn unicode_is_kept_verbatim() {
    let f = JsonFormatter::default();
    let src = r#"{"城市":"胡杨河市","e":"😀"}"#;
    let pretty = f.format(src).unwrap();
    assert!(pretty.contains("\"城市\": \"胡杨河市\""));
    assert!(pretty.contains("😀"));
    assert_eq!(f.unescape(&f.escape(src).unwrap()).unwrap(), pretty);
}

#[test]
fn escape_minifies_then_escapes_without_outer_quotes() {
    let out = JsonFormatter::default().escape("{\n  \"a\": \"x\\\"y\"\n}").unwrap();
    assert_eq!(out, r#"{\"a\":\"x\\\"y\"}"#);
}

#[test]
fn unescape_accepts_quoted_and_unquoted_input() {
    let f = JsonFormatter::default();
    assert_eq!(f.unescape(r#""{\"a\":1}""#).unwrap(), "{\n    \"a\": 1\n}");
    assert_eq!(f.unescape(r#"{\"a\":1}"#).unwrap(), "{\n    \"a\": 1\n}");
}

#[test]
fn unescape_returns_plain_text_when_result_is_not_json() {
    assert_eq!(JsonFormatter::default().unescape(r"hello\nworld").unwrap(), "hello\nworld");
}

#[test]
fn invalid_input_reports_line_and_column() {
    let err = JsonFormatter::default().format("{\n  \"a\": 1,\n}").unwrap_err();
    assert!(matches!(err, JsonError::Parse { line: 3, .. }), "{err:?}");
    assert!(!err.to_string().contains(" at line "), "{err}");
    assert!(matches!(JsonFormatter::default().unescape(r"\q"), Err(JsonError::Parse { .. })));
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --test format`
Expected：编译失败，提示 `JsonFormatter` / `JsonError` 未定义。

- [ ] **Step 3: 实现 `error.rs` 和 `format.rs`**

- 解析统一走 `serde_json::from_str::<Value>`。
- 美化输出用 `serde_json::ser::PrettyFormatter::with_indent` 配合 `" ".repeat(self.indent)`。
- `escape`：先 `minify`，再 `serde_json::to_string(&minified)`，最后去掉首尾引号。
- `unescape`：去掉首尾空白后，若首尾都是 `"` 就按 `String` 解析；否则在两端加 `"` 再按 `String` 解析。得到的字符串如果能解析成 JSON 就 `format`，否则原样返回。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --test format`
Expected：8 个测试全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add plugins/rtools-json
git commit -m "feat(json): add formatter with escape and unescape"
```

---

### Task 4: XML ↔ JSON

**Files:**
- Create: `plugins/rtools-json/src/core/convert/mod.rs`（`pub mod xml;`）、`plugins/rtools-json/src/core/convert/xml.rs`
- Modify: `plugins/rtools-json/src/core/mod.rs`（`pub mod convert;`）
- Test: `plugins/rtools-json/tests/xml.rs`

**Interfaces:**
- Consumes: `JsonError::Xml`、`INDENT`
- Produces:
  ```rust
  // rtools_json::core::convert::xml
  pub fn xml_to_json(src: &str) -> Result<serde_json::Value, JsonError>;
  pub fn json_to_xml(value: &serde_json::Value) -> String;   // ends with '\n'
  ```

- [ ] **Step 1: 写失败的测试 `tests/xml.rs`**

```rust
use rtools_json::core::convert::xml::{json_to_xml, xml_to_json};
use serde_json::json;

#[test]
fn xml_to_json_maps_attributes_repeats_and_empty() {
    let v = xml_to_json(r#"<user id="7"><name>Tom</name><tag>a</tag><tag>b</tag><bio/></user>"#).unwrap();
    assert_eq!(v, json!({"user": {"@id": "7", "name": "Tom", "tag": ["a", "b"], "bio": null}}));
}

#[test]
fn xml_to_json_keeps_text_next_to_attributes() {
    assert_eq!(xml_to_json(r#"<p lang="en">hi</p>"#).unwrap(), json!({"p": {"@lang": "en", "#text": "hi"}}));
}

#[test]
fn xml_to_json_rejects_malformed_input() {
    assert!(xml_to_json("<a><b></a>").is_err());
    assert!(xml_to_json("<a/><b/>").is_err());
}

#[test]
fn json_to_xml_uses_single_key_as_root() {
    let v = json!({"user": {"@id": "7", "name": "Tom", "tag": ["a", "b"], "bio": null}});
    assert_eq!(
        json_to_xml(&v),
        "<user id=\"7\">\n    <name>Tom</name>\n    <tag>a</tag>\n    <tag>b</tag>\n    <bio/>\n</user>\n"
    );
    assert_eq!(xml_to_json(&json_to_xml(&v)).unwrap(), v);
}

#[test]
fn json_to_xml_wraps_other_shapes_in_root() {
    assert_eq!(json_to_xml(&json!({"a": 1, "b": true})), "<root>\n    <a>1</a>\n    <b>true</b>\n</root>\n");
    assert_eq!(json_to_xml(&json!({"a": [1, 2]})), "<root>\n    <a>1</a>\n    <a>2</a>\n</root>\n");
    assert_eq!(json_to_xml(&json!([1, 2])), "<root>\n    <item>1</item>\n    <item>2</item>\n</root>\n");
    assert_eq!(json_to_xml(&json!({})), "<root/>\n");
}

#[test]
fn json_to_xml_escapes_and_sanitizes() {
    let v = json!({"root": {"first name": "a<b & \"c\" 'd'", "1st": "y"}});
    let xml = json_to_xml(&v);
    assert_eq!(
        xml_to_json(&xml).unwrap(),
        json!({"root": {"first_name": "a<b & \"c\" 'd'", "_1st": "y"}})
    );
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --test xml`
Expected：编译失败，`convert::xml` 不存在。

- [ ] **Step 3: 实现 `xml.rs`**

- 读取用 `quick_xml::Reader::from_str`，配合元素栈构建 `Value`：
  - 只取 `Start/Empty/End/Text/CData/Eof` 事件，忽略注释、PI、声明。
  - 文本先 trim，trim 后为空的文本丢弃。
  - 子元素按名字分组，保持文档顺序；重名的子元素转成数组。
  - 根元素有多个或标签不匹配时，返回 `JsonError::Xml`。
- 写出部分用私有的 `XmlWriter { out: String, depth: usize }`：
  - 规则见 spec §5.5。
  - 转义统一用 `quick_xml::escape::escape`。
  - 名称清洗：非 `[A-Za-z0-9_.-]` 的字符（包括空格）替换为 `_`；以数字、`-` 或 `.` 开头时加前缀 `_`。
  - 数组中嵌套数组时，元素内部写成 `<item>` 子元素。
  - 空对象和 `null` 都写成自闭合元素（如 `<root/>`）。
  - 只有文本的元素写成单行，带子元素的元素换行并缩进 `INDENT`。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --test xml`
Expected：6 个测试全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add plugins/rtools-json
git commit -m "feat(json): add xml and json conversion"
```

---

### Task 5: URL Params、YAML 与 `InputDetector`

**Files:**
- Create: `plugins/rtools-json/src/core/convert/query.rs`、`plugins/rtools-json/src/core/detect.rs`
- Modify: `plugins/rtools-json/src/core/convert/mod.rs`（`pub mod query;`）、`plugins/rtools-json/src/core/mod.rs`（`mod detect; pub use detect::{InputDetector, InputKind, Normalized};`）
- Test: `plugins/rtools-json/tests/detect.rs`

**Interfaces:**
- Consumes: `JsonFormatter::{format, format_value}`、`xml_to_json`、`JsonError::{Parse, Xml, Unrecognized}`
- Produces:
  ```rust
  // rtools_json::core::convert::query
  pub fn query_to_json(src: &str) -> serde_json::Value;   // object; repeated keys -> array of strings

  // rtools_json::core
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum InputKind { Json, Xml, UrlParams, Yaml }
  impl InputKind {
      /// Cheap prefix guess for `Plugin::accepts`; never parses, never returns Yaml.
      pub fn sniff(input: &str) -> Option<InputKind>;
  }
  #[derive(Debug, PartialEq)]
  pub struct Normalized { pub kind: InputKind, pub text: String }
  pub struct InputDetector<'a> { formatter: &'a JsonFormatter }
  impl<'a> InputDetector<'a> {
      pub fn new(formatter: &'a JsonFormatter) -> Self;
      /// Detects the input format and converts it to formatted JSON text.
      pub fn normalize(&self, input: &str) -> Result<Normalized, JsonError>;
  }
  ```

- [ ] **Step 1: 写失败的测试 `tests/detect.rs`**

```rust
use rtools_json::core::convert::query::query_to_json;
use rtools_json::core::{InputDetector, InputKind, JsonError, JsonFormatter};
use serde_json::json;

fn norm(s: &str) -> Result<(InputKind, String), JsonError> {
    let f = JsonFormatter::default();
    InputDetector::new(&f).normalize(s).map(|n| (n.kind, n.text))
}

#[test]
fn query_merges_repeated_keys_and_decodes() {
    assert_eq!(query_to_json("?a=1&b=2&a=3"), json!({"a": ["1", "3"], "b": "2"}));
    assert_eq!(query_to_json("https://x.com/p?name=%E5%BC%A0&q=a+b#frag"), json!({"name": "张", "q": "a b"}));
    assert_eq!(query_to_json("flag"), json!({"flag": ""}));
}

#[test]
fn json_is_formatted() {
    assert_eq!(norm("  {\"a\":1}").unwrap(), (InputKind::Json, "{\n    \"a\": 1\n}".into()));
}

#[test]
fn broken_json_reports_parse_error_instead_of_yaml() {
    assert!(matches!(norm("{\n  \"a\": 1,\n}"), Err(JsonError::Parse { line: 3, .. })));
    assert!(matches!(norm("[1, 2"), Err(JsonError::Parse { .. })));
}

#[test]
fn xml_url_and_yaml_are_converted() {
    assert_eq!(norm("<a>1</a>").unwrap(), (InputKind::Xml, "{\n    \"a\": \"1\"\n}".into()));
    assert_eq!(norm("a=1&b=2").unwrap().0, InputKind::UrlParams);
    assert_eq!(norm("https://x.com/?a=1").unwrap().0, InputKind::UrlParams);
    assert_eq!(
        norm("name: rtools\ntags:\n  - a\n").unwrap(),
        (InputKind::Yaml, "{\n    \"name\": \"rtools\",\n    \"tags\": [\n        \"a\"\n    ]\n}".into())
    );
}

#[test]
fn unrecognized_inputs() {
    for s in ["", "   ", "hello world", "a: 1", "just\nplain text"] {
        assert_eq!(norm(s), Err(JsonError::Unrecognized), "{s:?}");
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
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --test detect`
Expected：编译失败，`InputDetector` 等不存在。

- [ ] **Step 3: 实现 `query.rs` 和 `detect.rs`**

- `query_to_json`：
  - 输入含 `?` 时只取 `?` 之后的部分，并去掉 `#` 及其之后的内容。
  - 用 `form_urlencoded::parse` 解析。
  - 同名 key 第二次出现时，在原位置把值改成数组。
- `normalize` 按 spec §5.4 的顺序判断，全部基于 trim 之后的文本：
  1. 以 `{` 或 `[` 开头：调用 `formatter.format`，错误原样返回。
  2. 以 `<` 开头：`xml_to_json`，再 `format_value`。
  3. 单行、不含空白字符、包含 `=`：`query_to_json`。
  4. 含 `\n`，并且 `serde_saphyr::from_str::<Value>` 的结果是对象或数组：转成 JSON。YAML 解析报错时返回 `Unrecognized`，因为此时不能确定用户贴的是 YAML。
  5. 以上都不符合：返回 `Unrecognized`。
- `sniff` 与规则 1–3 的前缀和单行判断一致，但不做任何解析。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --test detect`
Expected：6 个测试全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add plugins/rtools-json
git commit -m "feat(json): detect and normalize pasted input"
```

---

### Task 6: `FilterEngine`

**Files:**
- Create: `plugins/rtools-json/src/core/filter.rs`
- Modify: `plugins/rtools-json/src/core/mod.rs`（`mod filter; pub use filter::{FILTER_MEMORY_LIMIT, FILTER_TIMEOUT, FilterEngine};`）
- Test: `plugins/rtools-json/tests/filter.rs`

**Interfaces:**
- Consumes: `JsonError::{Script, Timeout, EngineStopped}`
- Produces:
  ```rust
  pub const FILTER_TIMEOUT: Duration = Duration::from_secs(2);
  pub const FILTER_MEMORY_LIMIT: usize = 1 << 30;
  pub struct FilterEngine { /* Sender<Job> to the worker thread */ }
  impl FilterEngine {
      /// Starts the worker thread (expression timeout FILTER_TIMEOUT) and waits until QuickJS is initialised.
      pub fn spawn() -> Result<Self, JsonError>;
      /// Replaces the source text; parsed lazily on the next evaluation.
      pub fn set_document(&self, text: Arc<str>);
      /// Evaluates `"this" + expr` against the document.
      pub fn evaluate(&self, expr: &str) -> impl Future<Output = Result<String, JsonError>> + Send + 'static;
  }
  ```
  计划 2 的 UI 用 `cx.spawn` 等待 `evaluate` 返回的 future，并用自己的序号丢弃过时结果。

- [ ] **Step 1: 写失败的测试 `tests/filter.rs`**

```rust
use futures::executor::block_on;
use rtools_json::core::{FILTER_TIMEOUT, FilterEngine, JsonError};
use std::{sync::Arc, time::{Duration, Instant}};

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
    assert_eq!(block_on(e.evaluate(".list.map(x=>x.id)")).unwrap(), "[\n    1,\n    2\n]");
}

#[test]
fn semicolon_starts_a_full_expression() {
    let e = engine(DOC);
    assert_eq!(block_on(e.evaluate(";Object.keys(this)")).unwrap(), "[\n    \"name\",\n    \"list\"\n]");
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
    assert!(matches!(block_on(e.evaluate(".name.")), Err(JsonError::Script(_))));
    let bad = engine("{bad");
    assert!(matches!(block_on(bad.evaluate(".a")), Err(JsonError::Script(_))));
}

#[test]
fn timeout_then_recovers() {
    let e = engine(DOC);
    let start = Instant::now();
    assert_eq!(block_on(e.evaluate(";while(true){}")), Err(JsonError::Timeout));
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
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p rtools-json --test filter`
Expected：编译失败，`FilterEngine` 不存在。

- [ ] **Step 3: 实现 `filter.rs`**

- 工作线程：
  - 启动时在线程内创建 `rquickjs::Runtime` 和 `Context::full`，并调用 `set_memory_limit(FILTER_MEMORY_LIMIT)`。
  - 初始化结果通过 `std::sync::mpsc` 回传给 `spawn`，`spawn` 等到结果后才返回。
  - 主循环 `recv` 任务：`enum Job { SetDocument(Arc<str>), Evaluate(String, oneshot::Sender<Result<String, JsonError>>) }`。
  - channel 关闭时退出循环。
- 文档缓存：
  - 收到 `SetDocument` 时只保存原文，并把"已解析"标记清掉。
  - 求值前如果还没解析，就把原文以字符串形式设为全局 `__src`，执行 `globalThis.__doc = JSON.parse(__src)`。解析失败返回 `Script`。
- 求值：
  - 设置全局 `__code = "this" + expr`。
  - 执行 `(function(){ const r = (function(){ return eval(__code) }).call(__doc); if (r === undefined) return 'undefined'; const s = JSON.stringify(r, null, 4); return s === undefined ? String(r) : s; })()`。
  - 不用严格模式，以保证 `this` 的绑定。
- 超时：
  - 中断回调读取共享的截止时间 `Arc<Mutex<Option<Instant>>>`，克隆时写 `Arc::clone(&…)`。
  - 只在执行用户表达式前设置截止时间（`Instant::now() + FILTER_TIMEOUT`），结束后清空；`JSON.parse` 不设截止时间。
  - 每次执行前把 `timed_out` 复位为 false；回调触发时置位。执行抛错且 `timed_out` 为真时返回 `Timeout`。
- 错误：其他 JS 异常通过 `ctx.catch()` 转成字符串，返回 `Script`。
- 外部接口：
  - `set_document` 发送失败时直接忽略，因为引擎已经停止。
  - `evaluate` 发送失败，或 oneshot 被取消时，都返回 `EngineStopped`。
  - `evaluate` 在返回 future 之前就把任务发出去（立即发送），future 只负责等待结果。
  - core 不记日志：错误一律返回给调用方，由计划 2 的 UI 负责记日志。
- 在超时与截止时间检查处写 `// ponytail:` 注释："jobs run sequentially; stale evaluations are discarded by the UI sequence number, add queue draining if typing outpaces evaluation"。另在文档解析处写 `// ponytail:` 注释："integers beyond 2^53 lose precision in JSON.parse, upgrade with a BigInt-aware parser"。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p rtools-json --test filter`
Expected：9 个测试全部 PASS；`timeout_then_recovers` 耗时不超过 2.5 秒。

- [ ] **Step 5: 全量验证**

Run: `cargo test --workspace && cargo clippy --workspace --tests --examples -- -Dwarnings && cargo fmt --all --check`
Expected：全部通过，没有警告。

- [ ] **Step 6: 提交**

```bash
git add plugins/rtools-json Cargo.toml Cargo.lock
git commit -m "feat(json): add quickjs filter engine with timeout"
```

---

## 计划 2（不在本计划内，Task 1 完成后再写）

内容包括：`rtools-plugin`（`Plugin` trait 与 `PluginRegistry` 及其测试）、外壳（单实例 IPC、各平台快捷键、弹窗与搜索、插件挂载、分离窗口）、JSON 插件 UI（双编辑器、过滤框与序号、工具栏、状态栏、草稿原子写入）。它的接口依赖 Task 1 的 spike 结论。
