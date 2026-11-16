# rtools 设计文档

- 日期：2026-11-13
- 状态：待审阅
- 范围：子项目 1（启动器外壳）+ 子项目 2（JSON 插件）；第二个插件「文本对比」见 `2026-11-14-rtools-diff-plugin-design.md`

## 1. 背景与目标

uTools 是跨平台桌面效率工具：快捷键呼出搜索框，按关键字或输入内容匹配插件，插件在弹窗内展开。
rtools 用 Rust + GPUI（gpui-kit）实现同类启动器，第一个插件复刻 uTools 官方「JSON 编辑器」（v1.7.1）。

### 1.1 已确认的决策

| 决策 | 结论 |
|---|---|
| 定位 | 类 uTools 启动器平台，JSON 是第一个插件 |
| 插件模型 | 内置插件：Rust `Plugin` trait，编译进同一二进制 |
| 平台 | Linux（Wayland + X11）、macOS、Windows |
| 窗口形态 | uTools 式弹窗（无边框、每次呼出居中、可拖动、插件视图可调整大小、置顶、失焦或点击面板外隐藏），插件可分离为普通窗口 |
| 工程结构 | Cargo workspace；`rtools-json` 放在 `plugins/` 下，核心逻辑以模块边界隔离 |
| JSON 插件 v1 范围 | 必做功能 + 格式转换（JSON 转 XML）+ 粘贴自动识别（URL Params / XML / YAML） |
| 编码规范 | 遵守仓库根目录 `AGENTS.md` |
| 视觉风格 | **美观至上**：UI 必须经过设计，不接受"能用就行"的默认组件堆砌。视觉规范见 §5.7（已经原型确认） |

### 1.2 成功标准

1. 任意平台按快捷键（或执行 `rtools toggle`）呼出弹窗，输入 `json` 或粘贴 JSON 后回车进入 JSON 插件。
2. 21 万行级（约 4.5 MB）JSON：打开与格式化各在 2 秒内完成，滚动无明显卡顿。更大的输入不在目标内（实际使用不会出现）。
3. 底部 `this` 过滤框边输入边出结果，语义与原版一致。
4. 卡死的表达式（如 `;while(true){}`）2 秒内被中断，界面不冻结。

### 1.3 不做（v1）

- 大整数保精度（超过 2^53 的整数在过滤结果中丢精度，与原版一致）
- 多层转义自动还原、嵌套 JSON 字符串子视图、多标签
- JSON 转 TypeScript
- 本地应用启动、超级面板、拼音匹配、第三方/动态插件、托盘图标、开机自启、设置界面
- 左右分栏可拖动调整宽度
- 全部折叠/展开（gpui-kit 0.9.1 没有公开接口，见 §8 S2；单个折叠由 tree-sitter 高亮提供，见 §5.1）

## 2. 工程结构（2026-11-14 用户决定：插件不再拆 crate）

```
Cargo.toml                 # [workspace] members = ["crates/rtools"]; [workspace.dependencies] 统一声明
crates/rtools/
  src/plugin/              # 插件契约（Plugin trait、PluginManifest、Launch、PluginRegistry），共享的草稿 Draft 与 Autosave（draft.rs），Esc 关闭搜索框（search.rs）
  src/ui/                  # 启动器与插件共用的界面件：底栏 `Toolbar`（含其按钮与分隔线）、状态页 `StatusPage`、无需 Root 的 tooltip（`ButtonExt`）；按键提示直接取自快捷键绑定（`Kbd::binding_for_action`）
  src/plugins/json/core/   # JSON 纯逻辑，不得引用 gpui
  src/plugins/json/ui/     # JSON 插件界面
  src/plugins/diff/core/   # 文本对比纯逻辑（行级 diff → 统一补丁），不得引用 gpui
  src/plugins/diff/ui/     # 文本对比插件界面（编辑视图 + 对比视图）
  src/{cli,instance,shell,theme,assets}.rs, src/{hotkey,tray,launcher,settings}/（launcher/panel.rs：覆盖层面板的位置与大小）
  locales/app.yml          # rust-i18n 翻译
  tests/                   # 全部集成测试
assets/icons/              # 应用与托盘图标
packaging/linux/           # .desktop、用户级安装脚本 install.sh
```

所有插件先写在 `rtools` crate 内，通过 `plugin` 模块的 `Plugin` trait 解耦；将来需要独立 crate 时只搬模块。

### 2.1 根依赖分组

按 `AGENTS.md` 第 6、7 条：全部在根 `[workspace.dependencies]` 声明，子 crate 只写 `{ workspace = true }`；分组之间空一行并加分类注释，组内按字母排序。

| 分组 | 依赖 |
|---|---|
| ui | `gpui-kit`（feature `tree-sitter`：JSON 高亮与折叠）、`tree-sitter-xml`（XML 结果高亮，GPUI Kit 未内置 XML 语法） |
| script | `rquickjs` |
| data formats | `form_urlencoded`、`image`（托盘图标 PNG 解码）、`quick-xml`、`serde`、`serde-saphyr`（YAML 反序列化为 `serde_json::Value`）、`serde_json`（feature `preserve_order`、`arbitrary_precision`）、`similar`（文本对比的行级 diff）、`toml`（设置文件） |
| platform | `dirs`、`evdev`（仅 Linux）、`interprocess`、`ksni`（仅 Linux）、`rdev`（仅 macOS/Windows）、`sys-locale`、`tray-icon`（仅 macOS/Windows） |
| infra | `futures`、`rust-i18n`、`thiserror`、`tracing`、`tracing-subscriber` |

## 3. 插件契约（`crate::plugin`）

```rust
/// Static metadata the shell uses to list and match a plugin.
pub struct PluginManifest {
    pub id: &'static str,                  // "json"; also names the data dir
    pub title: &'static str,               // "JSON 编辑器"
    pub description: &'static str,         // "格式化、过滤、转换 JSON"
    pub icon: IconName,                    // gpui_kit::assets::IconName::Braces
    pub keywords: &'static [&'static str], // ["json", "jsoneditor"]
}

/// Everything a plugin receives when the shell opens it.
pub struct Launch {
    pub input: Option<String>, // content-match text only; None when opened by keyword
    pub data_dir: PathBuf,     // dirs::data_dir()/rtools/<id>/
}

pub trait Plugin: 'static {
    /// Returns the plugin's static metadata.
    fn manifest(&self) -> &PluginManifest;
    /// Cheap content match: `Some(reason)` (e.g. "识别为 JSON") when the plugin
    /// can handle the raw text, shown as the row's secondary line.
    fn accepts(&self, _input: &str) -> Option<SharedString> { None }
    /// Builds the plugin root view.
    fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> AnyView;
    /// Registers the plugin's key bindings once at startup (default: none).
    fn bind_keys(&self, _cx: &mut App) {}
}

pub struct PluginRegistry { plugins: Vec<Box<dyn Plugin>> }
```

- `PluginRegistry::search(&self, query)` 返回 `Vec<PluginMatch>`：先列关键字命中（关键字与标题，不区分大小写的子串匹配），再列内容命中（`accepts` 返回 `Some`）。空查询列出全部插件。
- `accepts` 每次输入都会调用，只能做轻量判断（首字符、前几 KB）；真正的解析在 `open` 中进行。
- `data_dir` 由外壳创建并保证存在。
- `input` 只在按内容匹配进入时传入（搜索框文字或粘贴的原文）；按关键字进入时为 `None`，插件读取草稿。
- 插件视图可以跨窗口复用（S4 已验证），契约不需要状态交接方法。

## 4. 外壳（`crates/rtools`）

### 4.1 进程与命令

- 命令：`rtools`（启动或呼出）、`rtools toggle`、`rtools quit`。
- 启动时先作为客户端连接本地 socket（`interprocess`，Unix 为 socket，Windows 为命名管道），发出命令后退出。
- 连接失败则成为首个实例：启动 `InstanceServer`（接收 `Toggle` / `Quit`）和 `HotkeyService`，后台常驻、不显示窗口。

### 4.2 交互流程

1. **呼出**：`Launcher::toggle` 显示启动器（宽 800，高度随内容），搜索框自动获得焦点并全选上次输入；空查询时列出全部插件。
2. **搜索**：边输入边调用 `registry.search`，`↑`/`↓` 选择，`Enter` 进入，鼠标单击也可进入。外壳自带一条「退出 rtools」（分组「系统」），不属于插件。没有匹配时显示「没有匹配的插件」。
   - **粘贴拦截**：粘贴内容为多行或超过 4 KB 时，原文另存为待处理输入，搜索框上方显示「已粘贴 N KB」标签（按 `Esc` 或点标签上的 × 清除；有标签时第一次 `Esc` 只清除标签，不隐藏窗口）；内容匹配与传给插件的都是原文。原因：单行输入框会吞掉换行（YAML 因此无法解析），大段文本也会拖慢渲染。
3. **进入插件**：窗口长到 800×600，顶栏变为「← 返回 · 插件图标 · 插件名 · 在独立窗口中打开」，下方挂载插件视图。返回插件列表：点 `←`，或按 `Esc`（编辑器自身的 `Esc`，如关闭查找面板，优先）。插件视图实体按插件 id 缓存在 `Launcher`，返回后再进入会恢复原状态。
4. **隐藏**：在插件列表按 `Esc`、再按一次快捷键、或窗口失去焦点时关闭弹窗窗口（Linux 上 GPUI 没有窗口隐藏能力），插件视图实体由 `Launcher` 持有，下次呼出时在新窗口里重新挂载，状态保留。应用使用 `QuitMode::Explicit`，没有窗口时进程继续常驻。
5. **分离**：`Ctrl+D`（macOS 为 `Cmd+D`；编辑器默认键位未占用，S2 已确认）或顶栏按钮：插件移入普通窗口（有标题栏、可缩放），弹窗恢复初始状态。

### 4.3 平台差异

平台相关代码隔离在外壳 crate 内：macOS / Windows 用 `cfg` 在编译期区分；Linux 同一个二进制可能跑在 Wayland 或 X11 上，因此在运行时按 GPUI 实际使用的后端选择弹窗与快捷键实现。

| 平台 | 弹窗 | 快捷键 |
|---|---|---|
| Linux Wayland | 全屏透明 `WindowKind::LayerShell` 覆盖层（四边锚定、`exclusive_zone = -1`、取第一个输出屏幕，键盘交互 `OnDemand`），不经 GPUI Kit 的窗口根（否则整屏不透明并带窗口边框），启动器在其中自绘面板：每次呼出屏幕正中（插件视图按自身高度居中，列表按显示时的高度居中，输入过滤改变行数时顶边不动），拖插件标题栏或列表底栏移动；插件视图可拖四边与四角（面板外 0.5 rem 的感应带）调整大小，最小 30×20 rem，大小在本次运行内保留，列表始终保持自身默认宽度；点面板外即隐藏（2026-11-14：layer-shell 表面创建后无法移动，原「不锚定居中」方案也因 GPUI 按去掉阴影内边距的尺寸设置表面大小而偏右下） | **双击 Ctrl**：读 `/dev/input`（evdev，用户需在 `input` 组） |
| Linux X11 | `WindowKind::PopUp`，无标题栏，在鼠标所在屏幕居中；拖标题栏或底栏交给系统移动窗口 | **双击 Ctrl**：evdev |
| macOS | 同上 | **双击 Ctrl**：Quartz `CGEventTap` 只读监听原始按键与修饰键事件；需授予「辅助功能」或「输入监控」权限，不查询输入法布局或转换字符 |
| Windows | 同上 | **双击 Ctrl**：`rdev` 全局键盘监听 |

**不注册任何组合键作为退路**（用户 2026-11-13 明确：Linux 上 `Alt+Space` 是 KRunner 的默认快捷键，坚决不能占用）。双击 Ctrl 不可用时只记日志，提示加入 `input` 组 / 授予权限 / 自行把快捷键绑定到 `rtools toggle`。

**双击 Ctrl 的判定**（2026-11-13 用户决定）：一次「单击」= Ctrl 按下到松开不超过 300 ms，且期间没有按其他键；第二次单击的按下距第一次松开不超过 350 ms 即触发。按住时的自动重复忽略；左右 Ctrl 等价。键盘事件只用于这一判定，绝不记录或保存任何按键。加入 `input` 组意味着该用户的所有程序都能读取键盘输入，这是用户知情后的选择。

macOS 监听使用独立线程的 RunLoop；不调用 `rdev` 的字符转换，因为该路径在后台线程查询输入源时会触发系统队列断言。已按住的 Shift/Option/Command/Fn 会阻止 Ctrl 连击，左右 Ctrl 同时按住视为一次整体按下；系统暂时停用事件监听时清空未完成的连击并重新启用，权限不足时记录提示，保留菜单栏和命令行入口。

所有窗口的 Wayland `app_id` 为 `dev.rtools.rtools`，与仓库中的 `packaging/linux/dev.rtools.rtools.desktop` 对应，桌面据此显示名称并归组窗口。`packaging/linux/install.sh` 安装到用户目录：剥离符号的二进制（`~/.local/bin/rtools`）、`.desktop`，以及以同名 `dev.rtools.rtools` 安装进 hicolor 的应用图标（scalable SVG + 32/256 PNG）。

已知 GPUI 问题：Wayland 下 `primary_display()` 恒为 `None`，覆盖层改用 `displays()` 的第一项；多屏时不跟随焦点屏幕（代码处有 `ponytail:` 注释）。

### 4.5 系统托盘（2026-11-13 用户追加，图标方案 C「r.」已确认）

- **交互**：左键单击 = 显示/隐藏启动器；悬停提示「rtools — 双击 Ctrl 呼出」；右键菜单「打开启动器 / —— / 退出 rtools」。托盘事件转成 `ShellCommand::Toggle` / `ShellCommand::Quit`，走与 `rtools toggle` 相同的通道。
- **图标**（`assets/icons/`）：
  - `rtools.svg`：彩色应用图标（蓝 `#2563EB` → 青 `#06B6D4` 渐变圆角方块 + 白色「r.」），用于 `.desktop`、窗口列表、macOS/Windows 托盘。
  - `rtools-tray.svg`：单色面板字形（22px 像素网格、笔画 2.2px），使用 KDE `ColorScheme-Text` 类，随面板明暗自动着色。
  - 预渲染 PNG（`rtools-22.png`、`rtools-32.png`、`rtools-44.png`、`rtools-256.png`、macOS 模板图 `rtools-tray-template-44.png`），由 `packaging/render-icons.sh`（`rsvg-convert`）生成。两个 SVG 的字形都按描边外框几何居中。
- **Linux**：`ksni`（StatusNotifierItem，D-Bus，无 GTK，`async-io`）。程序把内嵌的 `rtools-tray.svg` 写到 `<data_root>/icons/hicolor/scalable/apps/dev.rtools.rtools-tray.svg`，`icon_theme_path` 指向 `<data_root>/icons`，`icon_name` 为 `dev.rtools.rtools-tray`；同时提供 22/44px pixmap 兜底。
- **macOS / Windows**：`tray-icon`；macOS 用单色字形作模板图，Windows 用 32px 彩色图标。本机无法验证。
- **失败**：托盘创建失败只记 `tracing::warn!`，双击 Ctrl 与 `rtools toggle` 不受影响。

### 4.6 设置（2026-11-14，视觉见 §5.7「设置窗口」）

- 文件：`dirs::config_dir()/rtools/settings.toml`（`toml` + `serde`），字段：`ui_font`、`mono_font`（`Option<String>`，缺省跟随主题）、`font_size`（12–20，默认 16）、`window_radius`（窗口圆角，0–24，默认 4）、`language`（`system` / `zh-CN` / `en`），以及插件分组 `[json]`：`soft_wrap`（JSON 编辑器自动换行，默认关）。缺字段用默认值，越界数值收进范围；文件损坏时 `tracing::warn!` 并用默认值，不覆盖原文件。
- 应用：`Theme::update` 设置 `font_family`、`mono_font_family`、`font_size`（即 rem）、`window_radius`，`i18n` 切换语言；插件在渲染时读取 `SettingsStore` 中自己的分组。修改立即生效并写回文件。
- 入口：托盘菜单「设置…」、启动器「系统」分组「设置」。设置窗口是普通窗口（自绘标题栏），单实例：已打开时只激活。

### 4.7 国际化（2026-11-14）

- `rust-i18n`（gpui-kit 已依赖，与组件共用全局语言），翻译文件 `crates/rtools/locales/app.yml`，首批 `zh-CN`、`en`；`sys-locale` 读取系统语言，`language = "system"` 时以 `zh` 开头选 `zh-CN`，否则 `en`。
- 所有面向用户的文字（启动器、插件、托盘、设置、错误提示）都通过 `t!` 取得；`JsonError` 的 `Display` 也走翻译。

### 4.4 主要 struct

- `Launcher`：弹窗状态（当前查询、选中项、已打开的插件视图）。
- `InstanceServer`：IPC 服务端，把命令转成 `Launcher` 上的操作。
- `hotkey::start`：按平台启动双击 Ctrl 监听（判定逻辑在纯逻辑的 `DoubleTap` 中，有单元测试），失败只记日志。

## 5. JSON 插件（`crate::plugins::json`）

### 5.1 界面（`plugins/json/ui/`，`JsonPluginView`）

下图只是结构示意，不是视觉稿；视觉设计见 §1.1「视觉风格」。

```
┌─────────────────────────────┬─────────────────────────────┐
│ Editor (editable)           │ Editor (readonly, gray bg)  │
│ line numbers ·              │ filter / conversion result  │
│ search · tree-sitter json   │                             │
├─────────────────────────────┴─────────────────────────────┤
│ this│ filter input……………………   {} ⊘ │ ≑ " <> ⧉            │
│ status line                                               │
└───────────────────────────────────────────────────────────┘
```

- 右侧面板显示最近一次操作的结果：执行转换后显示转换结果；修改过滤框或左侧内容后切回过滤结果；过滤框为空且没有转换结果时隐藏。左右固定各占 50%。
- 工具栏：

| 按钮 | 功能 |
|---|---|
| `{}` | 格式化（缩进 4 空格） |
| `⊘` | 去除转义：接受带首尾引号的 JSON 字符串字面量（`"{\"a\":1}"`）或不带引号的转义文本（`{\"a\":1}`），还原后若是合法 JSON 则格式化 |
| `≑` | 压缩 |
| `"` | 转义：先压缩，再转义为字符串内容（不含首尾引号），便于直接粘进代码 |
| `<>` | 转 XML，结果显示在右侧 |
| `⧉` | 复制：右侧显示时复制右侧，否则复制左侧 |

- 转换结果显示在右侧，不覆盖左侧原文。过滤框有表达式时，转换的对象是过滤后的结果。
- 格式化、压缩、转义、去转义原地改写左侧，**必须可撤销**：使用 `EditorState::replace_all`（记入撤销历史，S2 已确认），不用会清空历史的 `set_value`。
- 状态栏显示解析错误位置（行:列）、执行超时、转换类型等信息。
- 不阻塞界面：整文档粘贴（及启动器带入的内容）的识别转换、工具栏改写、转 XML、文档校验都在后台线程执行；结果按编辑版本号落地，期间若左侧又被编辑则丢弃，不覆盖新的输入。
- XML 结果高亮：插件首次打开时向 GPUI Kit 的 `LanguageRegistry` 注册 `xml`（`tree-sitter-xml`），右侧随结果切换 `json` / `xml`（不用内置 HTML 语法：标签名里的 `_` 会断开）。
- 高亮与折叠（2026-11-14 由 syntect 改为 tree-sitter）：对象与数组跨 3 行以上即可折叠（syntect 后端只按 `{}` 计算折叠）；首次高亮 10.9 MB 由 1.72 s 降到 0.77 s，编辑后增量重解析。
- 搜索（2026-11-14）：两侧编辑器都开启 GPUI Kit 内置搜索栏，`Ctrl+F`（macOS `Cmd+F`）打开；左侧可编辑，搜索栏带「替换」切换（也可 `Ctrl+H` 直接进入替换），右侧只读只有搜索。`Esc` 先关闭打开的搜索栏（GPUI Kit 的输入框会把 `Esc` 用于移动焦点，故在插件视图上以捕获阶段处理）。
- 自动换行由设置 `[json] soft_wrap` 控制，默认关闭：开启时编辑器每次整文档替换都要测量每一行（5 MB 文档约 1.9 秒，在界面线程上）。

### 5.2 过滤引擎（`plugins/json/core/filter.rs`，`FilterEngine`）

- **线程模型**：一个专用线程独占 QuickJS 的 `Runtime` + `Context`。界面通过 channel 投递任务，用 `futures` oneshot 接收结果，在 `cx.spawn` 中等待。
- **文档隔离**：`set_document(Arc<str>)` 只保存原文；每次求值前都从原文重新 `JSON.parse` 出一份新文档（不计入超时），表达式的副作用（`.reverse()`、`.sort()`、赋值）不会污染下一次结果。原文不合法时缓存这个错误，直到换文档。
- **语义**（与原版一致）：以非严格模式执行 `(function(){ return eval(code) }).call(doc)`，其中 `code = "this" + 用户输入`。
  - `.name` → `this.name`
  - `[0].id` → `this[0].id`
  - `;Object.keys(this)` → `this;` 是一条空语句，`eval` 返回最后一条语句的值
- **输出**：`JSON.stringify(result, null, 4)`；结果无法序列化（`undefined`、函数等）时用 `String(result)`。
- **安全措施**：
  - 中断回调：用户表达式单次执行超过 2 秒即中断，返回「执行超时」。文档的 `JSON.parse` 不计入超时（耗时只与文档大小有关、不会死循环），否则大文件上过滤会永远超时
  - 内存上限 1 GiB
  - 输入停顿 200ms 后才求值
  - 新请求替换旧的求值任务（任务被丢弃即取消），只有最新的结果会落到界面
- **已知限制**：超过 2^53 的整数丢精度，代码处加 `ponytail:` 注释标明，后续做 BigInt 时从此处升级。

### 5.3 格式化（`plugins/json/core/format.rs`，`Json` / `Pretty` / `Rewrite`）

- 对象中出现重复的键时报错（带行列号），不静默保留其中一个值，因为结果会写回左侧原文。
- 解析：`Value::try_from(Json(text))`；校验（不构建值）：`Json(text).validate()`；缩进输出：`Pretty(&value)`。
- 工具栏改写 `Rewrite::{Format, Minify, Escape, Unescape}.apply(text)`；`Unescape(Escape(x))` 得到 `x` 格式化后的结果。
- `serde_json` 开启 `preserve_order`（不打乱键的顺序）和 `arbitrary_precision`（格式化/压缩时数字精度与写法不变，如 `1.50`、超过 u64 的整数；例外：指数补正号且 `E` 变小写，如 `1e3` → `1e+3`、`1E3` → `1e+3`，数值不变）。左侧原文会被原地改写，数字被改写属于数据损坏。

### 5.4 粘贴识别（`plugins/json/core/detect.rs`，`impl FromStr for Normalized`）

触发时机：打开插件时 `Launch.input` 有值，或左侧发生整文档粘贴：通过 `Editor::on_paste` 拦截，文档为空或已全选时消费这次粘贴，识别转换后用 `replace_all` 写入（一步可撤销）；识别失败则原样写入。按顺序判断：

1. 首字符是 `{`、`[` 或 `"` → 按 JSON 解析：成功则自动格式化；失败直接返回带行列号的解析错误，不再尝试后续规则（避免把写坏的 JSON 当成 YAML 静默转换）
2. 首字符是 `<` → XML，转为 JSON
3. 单行、无空白字符、含 `=`（可带 `?` 前缀或是完整 URL）→ URL Params，转为 JSON；重复的 key 合并为数组，值一律为字符串
4. 多行，且按 YAML 解析结果为对象或数组 → YAML，转为 JSON
5. 以上都不是 → 保留原文，状态栏显示解析错误

### 5.5 格式转换（`plugins/json/core/convert/`）

转换一律通过标准 trait 表达：`Value::try_from(Xml(text))`、`AsXml(&value).to_string()`（`Display`）、`Value::from(Query(text))`。

**XML ↔ JSON**（双向对称，可往返）：
- 属性 → `@name`
- 文本 → `#text`；元素只含纯文本时直接映射为字符串
- 重复子元素 → 数组；空元素（无属性、无子元素、无文本）→ `null`
- XML → JSON：根元素名作为最外层 key，如 `<user>…</user>` → `{"user": …}`
- JSON → XML：只有一个 key 且值不是数组的对象，以该 key 为根元素；其他情况外层包 `<root>`；数组中的元素用 `<item>`；`null` → 自闭合元素
- 文本与属性值转义 `& < > " '`；不合法的 XML 名称把非法字符替换为 `_`（保留 `:` 命名空间前缀），以数字、`-`、`.`、`:` 开头时加前缀 `_`
- XML 元素嵌套最多 128 层（与 serde_json 的递归上限一致），超过则报错，避免深层输入耗尽栈导致整个启动器崩溃

### 5.6 草稿

- 左侧内容变化 1 秒后写入 `<data_dir>/draft.json`：先写临时文件，再 rename 覆盖。
- 草稿由插件契约里的 `crate::plugin::Draft` 实现（`Draft::new(data_dir, file_name)`），文本对比插件用同一实现保存 `original.txt` 与 `modified.txt`。
- 打开插件时优先使用 `Launch.input`，否则读取草稿。

### 5.7 视觉规范（2026-11-13 经原型确认）

遵循 GPUI Kit Design Guides；颜色、圆角、间距、字号一律取主题 token 与 rem 刻度，原始像素只用于窗口尺寸这类物理边界。

**窗口**
- 弹窗与分离窗口都用客户端装饰（`WindowDecorations::Client`）+ 透明背景，由 `Root` 绘制圆角、边框与阴影。
- 产品主题层设置 `window_radius = 12px`（主题 token，非调用点字面量）；明暗主题跟随系统（`Theme::sync_system_appearance`）。
- 启动器宽 800，高度随内容；插件模式 800×600。

**启动器**
- 搜索框：无边框（`appearance(false)`）、前置搜索图标，与列表之间一条分隔线。
- 列表：用 `ListItem` 行，分组标题（「插件」「系统」「可处理此内容」）为 `text_xs` muted。每行：32×32 图标块（1px 边框、`radius` 圆角、无填充，保证选中时仍可见）+ 标题（`text_sm`）+ 说明（`text_xs` muted）。选中行由 `ListItem` 的选中填充表示，不加左侧竖条。
- 底栏：`Kbd` 提示「Enter 打开」「Esc 隐藏」，不放品牌字样。
- 粘贴后：搜索框上方一行「已粘贴 N KB」`Tag` + 单行 muted 内容预览（前 80 字符）；搜索框占位符变为「已粘贴内容，选择处理方式」；列表只列「可处理此内容」分组，行标题「用 {插件名} 打开」，说明为 `accepts` 返回的识别结果。
- 空结果：列表区居中 muted「没有匹配的插件」。

**插件顶栏**（弹窗内）
- 高 `h_10`：ghost 小号 `←`（tooltip「返回插件列表 (Esc)」）· muted 插件图标 · 插件名（`text_sm`）· 右侧 ghost 小号图标按钮 `app-window`（tooltip「在独立窗口中打开 (Ctrl+D)」）。分离窗口里不显示此顶栏，标题栏显示插件名。

**JSON 插件**
- 左右两个 `Editor` 都 `bordered(false)` + `appearance(false)`，与窗口同一底色；右侧结果栏以左分隔线区分。
- 结果栏不设标题行（2026-11-14 用户决定）：直接是只读编辑器；复制由底部工具栏的 `⧉` 负责（右侧显示结果时复制结果，否则复制左侧原文）。
- 底栏：过滤 `Input`（小号、等宽字体、前缀为 muted 等宽「this」）占满剩余宽度；右侧 ghost 小号**文字**按钮，分三组并以竖分隔线隔开：格式化 / 压缩 | 转义 / 去转义 | 转 XML。
- 左侧 JSON 错误：编辑器底部一条提示（上分隔线、`text_xs`、danger 色 + `circle-alert` 图标），文字「第 L 行第 C 列：消息」。
- 过滤/转换错误：结果栏居中显示 danger 图标 + 标题（如「表达式错误」）+ muted 详情。
- 编辑器行号栏底色与正文一致：主题层把 `highlight_theme.style.editor_background` 与 `editor_gutter_background` 设为 `background`，每次明暗切换后重新应用。

**全局统一规则（2026-11-14 经对照 Zed 确认）**
- 字号三级：正文/控件 `text_base`、说明与次要文字 `text_sm`、页面标题 `text_lg font_medium`；控件统一组件默认中号尺寸，导航项 `h_8` 与之等高。
- 输入框、下拉、数字输入：无边框（`appearance(false)`）+ `secondary` 浅色填充；下拉图标 `ChevronsUpDown`。
- 单一底色，区域之间只用 `border` 细线；侧边栏用 `sidebar` token。
- 普通窗口（设置、分离插件）用 `WindowDecorations::Client` + gpui-kit `TitleBar`，标题栏左侧留空。
- JSON 插件工具栏：ghost 图标按钮 + tooltip（`list-indent-increase` 格式化、`fold-vertical` 压缩、`quote` 转义、`remove-formatting` 去转义、`code-xml` 转为 XML）。

**设置窗口**：左侧 `w_64` 侧边栏（搜索框、导航「外观 / 语言 / JSON 编辑器」、底部 `Esc 关闭`）；右侧 `px_8`：顶行设置文件路径（等宽 muted）+ 描边按钮「在设置文件中编辑」，页标题与副标题，分组标题（`text_sm` muted + 下分隔线），每项一行（标题 + 说明 | `w_56` 控件栏，`py_5`，下分隔线）。

## 6. 错误处理与日志

- `plugins/json/core` 统一返回 `Result<_, JsonError>`（`Display` 随界面语言本地化），错误尽量带行号和列号，显示在状态栏，不 panic。
- 草稿写入失败：`tracing::warn!` + 状态栏提示，内存中的内容不丢。
- 外壳：IPC 绑定失败、双击 Ctrl 不可用都记录日志（提示 `rtools toggle` 与权限设置）。
- 日志只记：启动、IPC 绑定成功/失败、快捷键注册成功/失败及退回方案、打开插件、分离窗口、错误路径。不记录用户 JSON 内容和过滤表达式全文。
- 宏调用写完整路径，如 `tracing::info!("opened plugin {}", id)`，不用结构化字段语法。

## 7. 测试

- GPUI 界面代码按 `AGENTS.md` 第 3 条豁免单元测试。
- 测试只放在 crate 的 `tests/` 目录或 `#[cfg(test)] mod tests` 中。
- `plugins/rtools-json/tests/` 覆盖：
  - 格式化 / 压缩往返；`escape` 后 `unescape` 还原；`unescape` 对带引号和不带引号两种输入都能处理
  - 粘贴识别（`Normalized::from_str`）：每种格式的正例与反例
  - XML ↔ JSON 往返；URL Params 重复 key 合并为数组
  - `FilterEngine`：`.name`、`[0]`、`;Object.keys(this)`、语法错误、`;while(true){}` 在 2 秒内超时、原文非法 JSON 时报错
- `crates/rtools-plugin`：`PluginRegistry::search` 的排序（关键字命中在前、内容命中在后）与大小写不敏感。
- 提交前通过 pre-commit：`cargo fmt`、`cargo clippy --tests --examples -- -Dwarnings`。

## 8. Spike（先验证，代码用完即弃）

| # | 验证项 | 失败时的退路 | 结论（2026-11-13，KDE Wayland，release 构建） |
|---|---|---|---|
| S1 | gpui-kit 0.9.1 在当时固定的 rust 1.97.1 下可编译；gpui-kit 内置的 gpui 包含 `WindowKind::LayerShell` | 升级工具链，或直接依赖对应版本的 gpui | ✅ 1.97.1 编译通过；工具链后改为 `stable`，1.100.0 复验通过。gpui 为 `gpui-pre` 0.3.12，通过 `use gpui_kit::*` 导入（也可 `gpui_kit::gpui`）；组件在 `gpui_kit::component`。`WindowKind::LayerShell` 与 `gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions}` 存在，依赖 gpui 默认开启的 `wayland` feature |
| S2 | `Editor` + syntect 的 JSON 高亮；21 万行加载与滚动流畅；有无全部折叠/展开接口；同时开两个实例；可撤销的整体替换接口；`InputEditsEvent` 能否识别整文档替换；`Ctrl+D` 默认键位；release 构建下 `serde_json` 格式化 8 MB 的耗时；向单行 `Input` 粘贴多行/大段文本的表现 | 无折叠接口则暂时去掉 `⤒`/`⤓`；性能不达标则对超大文件关闭高亮；无可撤销替换则插件自存一步撤销；`Ctrl+D` 冲突则改 `Ctrl+Shift+D` | ⚠️ 高亮正常，两个实例同时工作。**折叠：syntect 下不显示折叠箭头，且没有公开的全部折叠/展开接口 → v1 去掉折叠与 `⤒`/`⤓`**。可撤销整体替换：`EditorState::replace_all` ✅。整文档粘贴：`Editor::on_paste` 可拿到原文，`InputEditsEvent` 也能看出 `start=0, old_end=旧长度`。`Ctrl+D`/`Cmd+D` 未被编辑器占用 ✅。首帧：21 万行 / 4.5 MB 为 1.6–1.9 s ✅；46.8 万行 / 10 MB 为 3.07 s（超出目标规模，不要求；关闭高亮同样约 1.8 s，瓶颈在编辑器本身，不在高亮）。`serde_json` 解析+美化：4.5 MB 20 ms，10 MB 54 ms。整文件粘贴进编辑器有明显卡顿（同为编辑器载入成本）。单行 `Input` 粘贴：未取得有效观测，按 §4.2 粘贴拦截处理（`Input::on_paste` 钩子存在） |
| S3 | KDE Wayland 下 LayerShell 弹窗能否失焦隐藏；各平台窗口能否隐藏后再显示 | 改为关闭窗口、重新打开 | ⚠️ LayerShell 窗口在 KDE 上能打开，失焦事件（`observe_window_activation`）正常触发。Linux 上 `App::hide` 未实现、无窗口级隐藏 → 采用退路：失焦即 `remove_window`，再呼出时新建窗口；需 `QuitMode::Explicit`。弹窗是否居中未单独确认。macOS/Windows 未验证，按 `WindowKind::PopUp` 假设 |
| S4 | 插件视图（`AnyView`）能否在关闭/新建窗口后、或分离到另一窗口后继续渲染和获取焦点 | 在 `Plugin` 契约中增加状态导出（文本）方法，新窗口用导出的内容新建实例 | ✅ 同一个视图实体（含 `InputState`、`EditorState`）在关闭窗口后，于新的 LayerShell 窗口和普通窗口之间来回挂载 4 轮，渲染、聚焦、输入均正常 → 契约不需要状态导出方法 |

Spike 结论回写到本文档后再进入实现。

## 9. 待定事项

无。

## 10. 实施顺序

计划 1（`docs/superpowers/plans/2026-11-13-rtools-foundation.md`）：

1. Spike S1–S4，结论回写本文档
2. Workspace 骨架
3. `rtools-json` core：format → xml → detect → filter（TDD）

计划 2（spike 完成后编写）：

4. `rtools-plugin`
5. 外壳：单实例 / IPC → 弹窗与搜索（含粘贴拦截）→ 插件挂载 → 分离窗口 → 各平台快捷键
6. `rtools-json` UI：编辑器、过滤框、工具栏、草稿
