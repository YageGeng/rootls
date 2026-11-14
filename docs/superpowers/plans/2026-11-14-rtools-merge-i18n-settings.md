# rtools 计划 3：合并 crate、审查修复、i18n、设置、视觉统一

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans (inline, user choice). Steps use checkbox (`- [ ]`) syntax.

**Goal:** 把 `rtools-plugin` 与 `rtools-json` 并入 `rtools` 单 crate；修复全量审查的问题；加入 i18n（中/英）、设置（字体、字号、语言）与 Zed 风格设置窗口；统一视觉（图标工具栏 + tooltip、轻量控件、自绘标题栏）。

**Spec:** `docs/superpowers/specs/2026-11-13-rtools-design.md`（§2、§4.6、§4.7、§5.7）

## Global Constraints
- 不做中途提交；全部完成后 `git add -A` 做唯一提交（`.gitmessage` 格式）。git 历史已被用户重置，不得改写。
- `AGENTS.md` 全部规则；**尽量少的 helper 函数**：单次调用的 helper 内联；单参数且为项目类型的 helper 改为方法；同 crate 内的纯数据类型用 `pub(crate)` 字段而非 getter/builder。
- 转换/解析用 `From`/`TryFrom`/`FromStr`/`Display`/`FromIterator`。
- Linux 不得注册 `Alt+Space`；呼出仅双击 Ctrl。
- 每个任务结束：`cargo test --workspace`、`cargo clippy --workspace --tests --examples -- -Dwarnings`、`cargo fmt --all --check` 全绿；UI 任务另需 release 运行 + `spectacle` 截图核对（明暗主题）。
- 进度记在 `.superpowers/sdd/2026-11-14-rtools-merge-i18n-settings/progress.md`。

---

### Task 1: 合并为单 crate
- [ ] `crates/rtools-plugin/src/lib.rs` → `crates/rtools/src/plugin.rs`；`plugins/rtools-json/src/core` → `crates/rtools/src/plugins/json/core`；`.../ui` → `crates/rtools/src/plugins/json/ui`（`plugins/mod.rs`、`plugins/json/mod.rs`）。
- [ ] 测试移到 `crates/rtools/tests/`（文件名加前缀 `json_`、`plugin_`），`use rtools_json::core` → `use rtools::plugins::json::core`，`rtools_plugin` → `rtools::plugin`。
- [ ] `include_str!/include_bytes!` 相对路径改正；删除 `crates/rtools-plugin`、`plugins/`；根 `Cargo.toml` members 改 `["crates/rtools"]`，删 internal 组；`crates/rtools/Cargo.toml` 合并依赖（script、data formats、futures、thiserror、tracing）。
- [ ] 验证：测试数 ≥ 76 且全绿。

### Task 2: 审查 Important 修复（先写失败测试）
1. YAML 预算：`Yaml` 用 `serde_saphyr::from_str_with_options`，`Options.budget = Some(Budget { max_nodes: usize::MAX, max_events: usize::MAX, max_depth: 128, ..Default::default() })`（保留别名限制）。测试：2.8 MB 的 YAML 列表能转换；70 层嵌套能转换。
2. `accepts` 便宜化：`JsonPlugin::accepts` 多行时只做 `Value::try_from(Yaml(input.trim())).is_ok()`，不格式化；删 `.then().flatten()` 链。`Launcher` 在 `pasted.is_some()` 时查询框 `Change` 不再 `refresh_rows`（内容行只在 `intercept_paste` 算一次）。
3. 校验只跑一遍：新增 `JsonFormatter::validate(&self, src) -> Result<(), JsonError>`（仅 `UniqueKeys` + BOM 去除）；`view.rs` 的自由函数 `validate` 删除；`new` 中 `Normalized` 成功时 `doc_error = None`；`UniqueKeys::visit_map` 用 `seen.replace(key)` 免 clone。测试：`validate` 对重复 key / 语法错误 / 尾随文本报错，对合法输入 Ok。
4. 契约去 helper：`PluginManifest`、`Launch` 改 `pub` 字段（struct 字面量构造，`static MANIFEST`）；删 9+4 个 builder/getter；`PluginView` 去掉 `Clone` derive；`Launcher.views` 改以注册表下标 `usize` 为键。
5. 平台转发：`hotkey` 用 `#[cfg] pub use evdev_source::start` / `rdev_source::start`；`tray` 统一签名后 `pub use linux::{start, LinuxTray as Tray}` / `desktop::{start, DesktopTray as Tray}`；删除 unsupported 桩；`hotkey::start` 接收 `UnboundedSender<ShellCommand>` 直接发 `Toggle`，删除 main 中第二条通道与循环。

### Task 3: 审查 Minor 修复
- 删除 `Confirm` action/绑定/`on_confirm`，回车只走 `PressEnter`；去掉随之无用的 `mode` 守卫重复。
- 内联：`shell::window_size_for` → `Launcher::sync_window_size`；`Shell::popup_options` → `open_popup`；`view::error_copy` → `render_result`（超时文案用 `FILTER_TIMEOUT.as_secs()`）；`filter::sloppy` → `Worker::new`；`detect::yaml_to_json` → `from_str`（`let kind = InputKind::sniff(text).unwrap_or(InputKind::Yaml)` 合并分支）。
- `strip_bom` 只留一个 `pub(crate) fn`（core/format.rs），detect 复用。
- `theme::apply` 内含 `Theme::sync_system_appearance`，删 main 中重复调用。
- 零拷贝：`set_document(text.into())`（`SharedString` → `Arc<str>`）；`Shown.text: SharedString`；`state.text().len_bytes()` 取长度；`schedule_save` 克隆 rope 后在后台转字符串；`array_len` 在后台执行，XML 结果 `count: None`；`PastedInput::intercept(String)`；`TreeBuilder::close` 用 `mem::take(&mut node.name)`；`XmlWriter` 直接写入 `fmt::Formatter`（`write!(f, "{:w$}", "", w = depth * INDENT)`）。
- 转 XML 收进 core：`JsonFormatter::to_xml(&self, src) -> Result<String, JsonError>`（用 core `parse`，含 BOM/重复 key 规则）；结果栏为 XML 时再点「转为 XML」仍以过滤结果（若有）为源，过滤出错时报错而非静默回退。
- 草稿：每个视图独立临时文件名（`draft.json.<pid>-<n>.tmp`）；`cx.on_app_quit` 与视图释放时 flush 待写草稿。
- 托盘左键与失焦：`Shell` 记录最近一次失焦关闭时间，300 ms 内的 `Toggle` 视为「关闭」而不重开。
- 单实例安全：Linux 改用 `$XDG_RUNTIME_DIR/rtools.sock`（文件 socket，0700 目录）；连接读取设 2 s 超时；`accept` 连续出错时退出循环并 `tracing::error!`。
- 公开面收窄：`FILTER_MEMORY_LIMIT`、`INDENT`、`APP_ID`、`Shell::{forget_popup, detach}`、`Launcher` 等改 `pub(crate)`；`InstanceName` 改 `pub` 元组字段，删 `new`；`desktop.rs` 错误用 `#[from]`；`Draft::load` 用 `read_to_string`；`JsonFormatter.indent` 去掉（用常量）。
- 注释：修正 `filter.rs` 过时注释（`__doc`、ponytail 位置）、`lib.rs` 模块文档；evdev 读线程出错 `tracing::warn!`。
- 测试代码：一行包装 helper（`norm`、`query_to_json`、`xml_to_json`、`json_to_xml`）内联到用处。

### Task 4: 设置模型 + i18n（先写失败测试）
- 依赖：`toml`、`sys-locale`、`rust-i18n`（`# data formats` / `# platform` / `# infra`）。
- `settings.rs`：`#[derive(Serialize, Deserialize, Default)] pub struct Settings { ui_font: Option<String>, mono_font: Option<String>, font_size: f32, language: Language }`，`enum Language { System, ZhCn, En }`（`serde(rename_all = "kebab-case")`）；`impl FromStr for Settings`（TOML）与 `Display`（TOML）；`Settings::load(path) -> Settings`（缺失/损坏→默认+warn）、`save(&self, path) -> io::Result<()>`（原子写）；`font_size` 夹在 12–20。测试：默认值、往返、缺字段、损坏文件、越界字号。
- `i18n.rs`：`rust_i18n::i18n!("locales", fallback = "en")`（在 lib.rs）；`Language::resolve(self) -> &'static str`（System → `sys_locale::get_locale()` 以 `zh` 开头得 `zh-CN`，否则 `en`）；`apply(&Settings, cx)` 设置 `rust_i18n::set_locale` 与 `gpui_kit::component::set_locale`、`Theme::update`（font_family/mono_font_family/font_size）。测试：`resolve` 映射。
- `locales/app.yml`：迁移所有用户可见字符串（启动器、插件、托盘、设置、`JsonError` 文案）为 `t!` key（中英）。

### Task 5: 设置窗口（Zed 风格，见 spec §5.7）
- `settings_window.rs`（或 `settings.rs` 内 `SettingsView`）：按原型 `target/design/src/main.rs` 的 `SettingsProto` 实现，绑定真实状态：选择即 `Settings` 更新 → `apply` → `save`；「在设置文件中编辑」调用 `cx.open_with_system(path)`；侧边栏搜索按标题/说明过滤行；`Esc` 关闭窗口。
- 窗口：`TitleBar::window_options()` + `window_decorations: Some(Client)`，标题栏 `TitleBar::new()` 左侧留空；单实例（`Shell` 持有窗口句柄，已开则 `activate_window`）。
- 入口：托盘菜单在「打开启动器」后加「设置…」（发送新命令 `ShellCommand::Settings`，CLI 也支持 `rtools settings`）；启动器「系统」分组加「设置」行（图标 `settings`）。

### Task 6: 视觉统一
- JSON 工具栏：ghost `small` 图标按钮 + `tooltip(t!(...))`，图标见 spec §5.7；结果栏复制保留图标+文字。
- 过滤框、启动器搜索框：`appearance(false)` + `secondary` 填充、圆角 `radius`。
- 分离窗口：`TitleBar::window_options()` + Client 装饰 + `TitleBar::new()`，下方挂插件视图。
- 主题：`theme::apply` 中侧边栏 token 保持；所有窗口字号层级按 spec。
- 截图核对：启动器、JSON 插件（含 tooltip 悬停需人工）、设置窗口、分离窗口，明暗各一。

### Task 7: 收尾
- 全量验证；对照 Design review checklist；用户走查（双击 Ctrl、托盘、设置、语言切换、字体切换、工具栏 tooltip）。
- 最终审查（新审查代理，最强模型）→ Important 修复 → 唯一提交。
