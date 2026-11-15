# rtools 文本对比插件（diff）设计

日期：2026-11-14 · 状态：待审阅 · 关联：`2026-11-13-rtools-design.md`（主设计，插件契约见 §3）

## 1. 目标与范围

新增第二个内置插件「文本对比」：粘贴或编辑两段文本，查看它们的差异。交互参照 Zed 的对比视图（并排 / 统一、词级高亮、上下一处、折叠未改动行），但对比视图只读，编辑在单独的编辑视图里完成。

- 与 JSON 编辑器是**两个独立插件，互不联动**：JSON 编辑器不加任何对比入口；diff 插件不引用 `plugins::json` 的任何代码（「先格式化 JSON」由 diff 插件自己实现）。两者只共同依赖插件契约（`crate::plugin`）。
- 本版不做：启动器粘贴内容识别（任何文本都能对比，对每次粘贴都出「对比」行只会是噪音）、打开磁盘文件对比、边编辑边对比、三方合并。

## 2. 插件身份

| 项 | 值 |
|---|---|
| id | `diff`（同时是数据目录名） |
| 标题键 / 文案 | `diff.title`：文本对比 / Text Diff |
| 描述键 / 文案 | `diff.description`：对比两段文本的差异 / Compare two texts |
| 图标 | `IconName::GitCompare`（加入 `ExtraIcons`） |
| 关键词 | `diff`、`compare`（本地化标题本身也参与匹配，输入「对比」可找到） |
| 内容识别 | 不实现 `Plugin::accepts`（默认 `None`） |

在 `main.rs` 中注册于 JSON 编辑器之后。

## 3. 界面（视觉稿已确认：工具栏开关用方案 A「图标开关」）

插件挂在启动器面板（或分离窗口）里，标题栏沿用启动器现有的「← 图标 标题 … 分离」。插件本身有两个视图。

### 3.1 编辑视图（打开插件后的起点）

- 左右两个编辑器，各占一半，中间 1 px 分隔线：左「原文 / Original」，右「修改后 / Modified」。
- 每个编辑器上方一条细标题条（`h_8`，下分隔线）：左侧 `text_xs` muted 名称，右侧 muted「N 行 / N lines」。
- 编辑器：按语言着色：默认从两侧开头约 4 KB 自动识别（shebang、JSON / XML / HTML 的开头、各语言的典型写法，先看原文、原文像纯文本再看修改后），底栏的语言下拉框可改为指定语言（「自动 · 识别结果」或具体语言）；编辑器与对比视图用同一语言。可着色的语言：Bash、C、C++、C#、CSS、Go、HTML、Java、JavaScript、JSON、Markdown、Python、Rust、SQL、TOML、TSX、TypeScript、XML、YAML；显示行号、开启内置搜索替换（`Ctrl+F` / `Ctrl+H`）、不自动换行。
- 底部工具栏（与 JSON 编辑器底栏同样的高度、边框与按钮尺寸）：
  - 左：ghost 小号图标按钮「交换两侧」（`ArrowLeftRight`）、「清空两侧」（`Eraser`），悬停显示提示；两者都通过 `replace_all` 写入，可撤销。分隔线后是语言下拉框（可搜索）。
  - 右：primary 小号按钮「对比 / Compare」，内嵌快捷键提示 `Ctrl+Enter`（macOS `Cmd+Enter`）。
- 两侧内容分别自动保存为草稿（停止输入 1 秒后；退出程序或视图释放时立即写盘，同 JSON 插件），下次打开插件时恢复。

### 3.2 对比视图（只读）

- 主体是 GPUI Kit 的 `Diff` 组件（`DiffState` 由插件视图持有）：
  - 隐藏组件自带的文件头（`header_visible(false)`），统计数字放在我们的工具栏里；
  - 显示行号；变化行用 `+` / `−` 标记；删除行红底、新增行绿底，变化的词（或字符）加深；
  - 默认显示全部行（2026-11-15 用户要求）；用底栏按钮切换为折叠后，每处变化前后保留 3 行上下文，其余未改动区域折叠为「显示 N 行未改动」，点击逐段展开（每次 20 行，组件默认值）；展开 / 折叠的选择写入 `[diff].fold_unchanged`，每个新结果都按它显示；
  - 并排时长行在各自栏内换行（`Diff::soft_wrap`），两栏各占一半宽度；统一视图不换行，长行横向滚动；
  - 块分隔用 `DiffHunkSeparator::Simple`（细分隔线）：补丁只有一个覆盖全文的 `@@` 块，默认的 `Metadata` 会把 `@@ -1,11 +1,13 @@` 原样显示在顶部；
  - 着色语言：下拉框指定的语言；自动时两侧（格式化后）都是合法 JSON 则按 JSON，否则按识别结果。
- 底部工具栏（方案 A，从左到右）：
  1. 统计：`+N`（成功色）`−M`（危险色），`text_xs`，中间留空格；
  2. 竖分隔线；「上一处」「下一处」（`ChevronUp` / `ChevronDown`，ghost 小号图标按钮，提示含快捷键 `Alt+↑` / `Alt+↓`）；
  3. 竖分隔线；四个开关，ghost 小号图标按钮，开启时为选中态（底色），悬停显示说明：
     - 忽略空白（`Space`）
     - 忽略大小写（`CaseSensitive`）
     - 先格式化 JSON（`Braces`）；不可用时显示为禁用样式，提示说明哪一侧不是合法 JSON
     - 字符级高亮（`Type`）；关闭时为词级高亮
  4. 弹性空白；
  5. 分段切换「并排 / 统一」（`DiffMode::Split` / `Unified`）；
  6. 「展开 / 折叠全部未改动」（`UnfoldVertical` / `FoldVertical`，按当前状态切换图标与提示）；
  7. outline 小号按钮「编辑 / Edit」，内嵌快捷键提示 `Esc`。
- 状态页（替代 `Diff` 组件，居中显示：图标 + 标题 + muted 说明，风格同 JSON 结果区的错误状态）：
  - 两段相同：「两段文本相同 / The texts are identical」；「忽略空白」或「忽略大小写」开启、或 JSON 已格式化（`format_json = Applied`）时，说明为「按当前对比选项 / With the current comparison options」，否则不显示说明；
  - 某一侧为空：「原文为空 / The original is empty」或「修改后为空 / The modified text is empty」（两侧都空时提示两侧都为空）；
  - 补丁解析失败（不应发生）：「无法显示差异 / Cannot show the differences」+ 错误摘要，同时 `tracing::warn!`。
- 键位：对比视图中 `Esc` 回到编辑视图，再按一次 `Esc` 回到启动器列表（现有行为）；`Alt+↑` / `Alt+↓` 跳到上一处 / 下一处变化。编辑视图中 `Ctrl+Enter`（macOS `Cmd+Enter`）进入对比视图。
- 重新计算期间（切换前三个开关后）继续显示上一次的结果，新结果到达后替换，不闪烁、不显示加载状态。
- 进入对比视图后，结果 250 ms 内没到就显示「正在对比… / Comparing…」，更快的结果不显示任何过渡。

## 4. 架构

```
crates/rtools/src/
├── plugin/              # 插件契约（原 plugin.rs 改为目录）
│   ├── mod.rs           # 契约本身，内容不变
│   ├── draft.rs         # Draft（从 plugins/json/core/draft.rs 移来）与 Autosave
│   └── search.rs        # SearchPanels：实现它的视图获得「Esc 先关搜索框」
├── ui/                  # 启动器与各插件共用的界面件：底栏 `Toolbar`（含其按钮与分隔线）、状态页 `StatusPage`、无需 Root 的 tooltip（`ButtonExt`）；按键提示直接取自快捷键绑定（`Kbd::binding_for_action`）
└── plugins/diff/
    ├── mod.rs
    ├── core/            # 不依赖 gpui
    │   ├── mod.rs
    │   └── compare.rs   # 选项、文本规范化、行级 diff → 统一补丁 + 统计
    └── ui/
        ├── mod.rs       # DiffPlugin：manifest + Plugin 实现
        └── view/
            ├── mod.rs   # DiffPluginView：状态、对比任务、编辑区与对比区
            └── toolbar.rs # 编辑视图与对比视图的底栏
```

### 4.1 共享的草稿（`crate::plugin::Draft`）

`Draft` 原在 JSON 插件内，但与 JSON 无关，移入插件契约作为基础设施：

- `Draft::new(data_dir: &Path, file_name: &str)`：草稿文件为 `<data_dir>/<file_name>`，临时文件规则不变（进程号 + 序号，原子替换）。
- JSON 插件改为 `Draft::new(dir, "draft.json")`（文件名不变，已有草稿照常恢复）；diff 插件使用 `original.txt` 与 `modified.txt`。
- 停顿 1 秒写盘、退出或视图释放时立即写盘的逻辑同样收进契约：`Autosave`（`FromIterator<Draft>`），两个插件只调用 `schedule` / `flush`。
- 两个插件共用的还有 `plugin::SearchPanels`（trait，提供 Esc 先关搜索框的 `close_search`）和 `crate::ui` 的界面件；插件之间仍互不引用。插件自己的快捷键经契约里的 `Plugin::bind_keys` 注册，`main.rs` 只调用 `PluginRegistry::bind_keys`。

### 4.2 核心（`plugins/diff/core/compare.rs`）

公开接口（名称以此为准）：

```rust
/// Comparison switches that change which lines count as equal.
pub struct CompareOptions { pub ignore_whitespace: bool, pub ignore_case: bool, pub format_json: bool }

/// The two texts being compared.
pub struct Texts<'a> { pub original: &'a str, pub modified: &'a str }

impl Texts<'_> {
    /// Compares the texts under `options`.
    pub fn compare(&self, options: CompareOptions) -> Comparison;
}

/// Outcome of one comparison.
pub struct Comparison {
    pub patch: String,          // unified patch covering every line of both texts
    pub additions: usize,
    pub deletions: usize,
    pub json: bool,             // both sides parse as JSON (after formatting, when applied)
    pub format_json: FormatJson,
    pub original_empty: bool,   // rule 6; only the core sees the normalized text
    pub modified_empty: bool,
}

/// What became of the "format JSON first" switch.
pub enum FormatJson { Off, Applied, Unavailable(Side) }

/// One side of a comparison.
pub enum Side { Original, Modified }
```

规则：

1. **规范化**（总是执行）：去掉开头的字节顺序标记（BOM，不可见且不是 JSON）；`\r\n` 视同 `\n`；末尾是否有换行不算差异。
2. **先格式化 JSON**（开关开启时）：两侧都能解析为 JSON 时，各自递归按键名排序对象、以 4 空格缩进输出，再进入比较，`format_json = Applied`；任一侧解析失败则按原文比较，`format_json = Unavailable(第一个失败的一侧)`。开关关闭为 `Off`。使用 `serde_json`（工作区已开启 `arbitrary_precision`，数字写法不变），不调用 JSON 插件。
3. **比较键**：逐行生成比较键：忽略空白时去掉行内所有空白字符（同 `git diff -w`），忽略大小写时转为小写；两项可叠加。用 `similar` 对比较键序列做 Myers 行级 diff，限时 2 秒，超时得到的是正确但不一定最短的结果。
4. **生成补丁**：只用原始行文本（规范化后、未去空白、未转小写）写出一个覆盖两段全部行的统一补丁（`--- original` / `+++ modified` / 单个 `@@` 块，空的一侧写作 `-0,0` 或 `+0,0`）。比较键相同但原文不同的行作为上下文行，显示修改后一侧的文本（同 `git diff -w`）。
5. **统计**：`additions` / `deletions` 为补丁中的 `+` / `-` 行数；两者都为 0 即「两段相同」。
6. **为空**：规范化后（换行符统一、去掉末尾换行）没有任何字符的一侧为空；只有空白字符不算空。界面先判断「为空」，再判断「两段相同」。
7. **JSON 着色**：两侧（格式化后，若已格式化）都能解析为 JSON 时 `json = true`。

### 4.3 界面（`plugins/diff/ui/view/`）

- `DiffPluginView` 持有：两个 `EditorState`、一个 `DiffState`、当前视图（编辑 / 对比）、两个 `Draft`、最近一次的 `Comparison` 摘要（统计、`format_json`、是否两段相同或某侧为空）、后台对比任务。
- 进入对比视图、或在对比视图中切换前三个开关：取两侧文本（克隆 rope）与选项，在后台线程执行 `Texts::compare`，并在同一个后台任务里 `DiffFile::parse(patch)` → `with_language("json")`（若 `json`）。`DiffFile` 是 `Send + Sync`，大补丁的解析不占界面线程。完成后在界面线程调用 `DiffState::set_files`，并刷新工具栏。新请求替换旧任务（任务被丢弃即取消）。
- 「字符级高亮」只调用 `DiffState::set_inline_unit(Character / Word)`；「并排 / 统一」只调用 `set_mode`；都不重新计算。
- 「展开 / 折叠全部未改动」只切换 `fold_unchanged`，设置观察者再调用 `expand_unchanged` / `collapse_unchanged`；新结果 `set_files` 后按该设置重新展开；上下一处调用 `previous_change` / `next_change`。
- 编辑视图里的修改不会自动刷新对比视图；每次进入对比视图都重新计算，因此不会看到过期结果。
- 焦点：启动器在插件打开时记下一次 `PluginView::focus`，之后每次呼出都聚焦它，因此插件视图提供**自己的**稳定焦点句柄（`Focusable` 返回它）。编辑视图下它获得焦点时立即转交给「原文」编辑器；对比视图显示差异时转交给 `Diff` 本体（`↑↓`、`PageUp`/`PageDown` 直接滚动），显示状态页时停在视图本身；对比视图的 key context 挂在视图根元素上，因此焦点在 `Diff` 本体里时 `Esc` / `Alt+↑↓` 照样生效。转交在 `render` 里完成：发现自身句柄有焦点时，延迟聚焦「原文」编辑器。不用 `cx.on_focus`，因为它绑定在创建视图时的窗口上，`Ctrl+D` 分离到独立窗口后就不会再触发。
- 键位：
  - 对比视图：在插件的 key context（如 `DiffCompare`）里绑定 `escape` → 回到编辑视图、`alt-up` / `alt-down` → 上一处 / 下一处；该 context 比启动器的插件 context 更深，`Esc` 先回编辑视图。
  - 编辑视图：编辑器自身把 `ctrl-enter` / `cmd-enter` 绑定为 `Enter { secondary: true }` 并消耗掉，插件 context 里的绑定收不到。因此在插件视图根元素上以**捕获阶段**处理 GPUI Kit 的 `Enter` 动作：`secondary == true` 时进入对比视图并停止传播，否则继续传播（与 JSON 编辑器捕获 `Esc` 的做法相同）。
- 开关到选项的转换：`impl From<&DiffSettings> for CompareOptions` 放在插件的 `ui` 模块（`settings` 模块依赖 gpui，`core` 不能依赖它）。

### 4.4 设置（`settings.toml` 的 `[diff]` 表）

```rust
#[serde(default)]
pub struct DiffSettings {
    pub layout: DiffLayout,       // "split"（默认）/ "unified"
    pub ignore_whitespace: bool,  // 默认 false
    pub ignore_case: bool,        // 默认 false
    pub format_json: bool,        // 默认 false
    pub char_level: bool,         // 默认 false（词级）
    pub fold_unchanged: bool,     // 默认 false（显示全部行）
}
```

- `Settings` 新增字段 `diff: DiffSettings`，位于 `json` 之后（TOML 要求表写在普通键之后）。
- 对比视图的开关与「并排 / 统一」通过 `SettingsStore::update` 修改并立即写回文件；设置窗口不增加 diff 页面（避免同一开关出现在两处）。手动改文件后，下次呼出启动器生效（现有行为）。

### 4.5 其他改动

- 依赖：`similar = "3"`，加入根 `Cargo.toml` 的「data formats」分组，子 crate 以 `{ workspace = true }` 引用。GPUI Kit 仍间接引入 2.7，两个版本并存；3.x 在大输入上快得多：实测 2 万行的病态输入，2.7 会撞上 2 秒上限、退化成整段替换，3.2 只需 60–130 ms，并给出真实的差异。
- 图标：`ExtraIcons` 增加 `GitCompare`、`ArrowLeftRight`、`Eraser`、`Space`、`CaseSensitive`（若已在默认集中则不重复）、`Type`、`ChevronUp`、`ChevronDown`、`UnfoldVertical`（`Braces`、`FoldVertical` 已有）。
- 文案：`locales/app.yml` 新增 `diff.*`（中英）：标题、描述、原文、修改后、行数、对比、交换两侧、清空两侧、编辑、上一处、下一处、四个开关的说明、格式化 JSON 不可用（含哪一侧）、并排、统一、展开 / 折叠全部未改动、两段相同、按当前对比选项、原文为空、修改后为空、两侧都为空、无法显示差异。
- `.gitignore` 增加 `.superpowers/`（视觉稿等工作文件）。
- 主设计文档：§2 依赖表加 `similar`，插件列表加入文本对比，`Draft` 位置改为插件契约。

## 5. 边界情况

| 情况 | 行为 |
|---|---|
| 一侧为空 / 两侧都空 | 对比视图显示对应状态页，不显示 `Diff` |
| 两段相同（考虑开关后） | 状态页「两段文本相同」；有开关生效时附「按当前对比选项」 |
| 开了「先格式化 JSON」但有一侧不是 JSON | 开关显示为禁用样式（仍可点击关闭），并说明哪一侧；按原文比较 |
| 「先格式化 JSON」时对象里有重复的键 | 按 `serde_json` 的规则只保留最后一个值（同 `jq`）；只影响对比显示，编辑视图里的原文不变 |
| 面板拉到最窄 | 启动器的最小宽度由 30 rem 提高到 35 rem，对比工具栏在中英文下都完整显示，不截断 |
| `\r\n` 与 `\n`、末尾换行 | 不算差异 |
| 超大或病态输入 | 后台计算；超过 2 秒得到较粗但正确的 diff；`Diff` 组件虚拟化渲染；被新请求替代的对比不再解析补丁 |
| 「先格式化 JSON」与指数写法 | `1e2`、`1E2` 都写成 `1e+2`（`serde_json` 的规则），只差指数写法的数字不算差异 |
| 补丁解析失败 | 状态页「无法显示差异」+ `tracing::warn!` |
| 草稿读写失败 | `tracing::warn!`，不影响编辑（同 JSON 插件） |

## 6. 测试

- 核心先写测试（新增 `tests/diff_compare.rs`）：
  - 普通修改、插入、删除的统计与补丁内容；
  - 两段相同 → 统计为 0；
  - 忽略空白：只差缩进 / 空格的行算相同，补丁显示修改后一侧的原文；
  - 忽略大小写：只差大小写的行算相同；
  - 先格式化 JSON：键顺序不同、紧凑与格式化写法不同的两段 JSON 判为相同；一侧非法时为 `Unavailable(该侧)` 且按原文比较；
  - `\r\n` / `\n` 与末尾换行不算差异；
  - 生成的补丁能被 GPUI Kit 的 `DiffFile::parse` 解析，且其 `additions()` / `deletions()` 与 `Comparison` 一致。
- `tests/settings.rs`：`[diff]` 的默认值与 TOML 往返。
- 草稿测试随 `Draft` 迁移到 `tests/plugin_draft.rs`，并覆盖文件名参数。
- 界面实测要点：`Ctrl+Enter` 在编辑器有焦点时也能进入对比视图；对比视图里 `Esc` 先回编辑视图；从对比视图关闭启动器再呼出，`Esc` / `Alt+↑↓` 仍然有效（焦点句柄正确）；`Ctrl+D` 分离后编辑视图可以直接输入；面板最窄时工具栏完整显示。
- 界面按 `AGENTS.md` 第 3 条不写单元测试；实现后在运行中的应用里用受控的虚拟鼠标方式验证（仅在用户不使用电脑时进行）。

## 7. 决策记录

| 决策 | 理由 |
|---|---|
| 编辑与对比分开，对比视图只读 | 用户选定；GPUI Kit 编辑器无法插入占位行对齐两侧，可编辑对比需要自研编辑器 |
| 用 GPUI Kit `Diff` 渲染、`similar` 计算并自行生成补丁 | 对齐、折叠、词级 / 字符级高亮、虚拟化由组件提供；自生成补丁才能在忽略空白 / 大小写时仍显示原文 |
| 四个开关都在对比视图里实时切换 | 用户要求 |
| 工具栏开关用图标（方案 A） | 用户选定；与 JSON 编辑器工具栏风格一致（仅视觉一致，无联动） |
| 「先格式化 JSON」同时按键名排序 | 目的是去掉格式与键顺序带来的噪音 |
| 开关与布局写入 `[diff]`，不进设置窗口 | 只在对比视图里操作，避免同一开关两处出现 |
| `Draft` 移入插件契约 | 两个插件都要用；不让 diff 插件依赖 JSON 插件 |
| 不做启动器粘贴识别 | 任何文本都能对比，识别行只会是噪音 |
| `similar` 用 3.x（2026-11-14 用户决定升级） | 大输入上比已在依赖树中的 2.7 快一到两个数量级，2 秒上限很少触发 |
| `Ctrl+Enter` 用捕获阶段处理 | 编辑器把它绑定为自己的 `Enter { secondary }` 并消耗掉 |
