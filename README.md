# rtools

一个用 Rust 编写、常驻后台的桌面效率启动器，灵感来自 uTools。界面基于 [GPUI](https://www.gpui.rs) 与 [gpui-kit](https://gpui-kit.com) 绘制，双击 `Ctrl` 随时呼出；内置插件有 **JSON 编辑器** 和 **文本对比**。

> 目前主要在 Linux（KDE Plasma · Wayland）上开发和验证。macOS 支持源码构建和 `.app` 打包；X11、macOS、Windows 的运行时功能尚未完整验证。

## 功能

### 启动器

- **双击 `Ctrl` 呼出 / 隐藏**：只认「单独轻按两次」（每次按住不超过 300 ms，两次间隔不超过 350 ms），`Ctrl+C` 之类的组合键不会误触。刻意不提供 `Alt+Space` 等组合键（KDE 上它属于 KRunner）。
- **托盘图标**：左键切换启动器；右键菜单为「打开启动器 / 设置… / 退出 rtools」。
- **单实例**：再次运行 `rtools` 不会启动第二个进程，而是把命令转给正在运行的实例。
- **搜索插件**：按名称或关键词匹配；列表下方是「设置」「退出 rtools」两个系统项。
- **粘贴识别**：在搜索框里粘贴多行或超过 4 KB 的内容时，内容会收成一个「已粘贴」标签，并列出能处理它的插件，回车即可带着内容打开。
- **Wayland 下的窗口**：启动器是一层全屏透明覆盖层，面板每次呼出都在屏幕正中；拖动插件标题栏或列表底栏可以移动，拖动插件面板的四边与四角可以调整大小；点击面板外即隐藏。
- **分离窗口**：在插件里按 `Ctrl+D`（macOS 为 `Cmd+D`），可以把它移到独立窗口继续使用。

### JSON 编辑器

- **语法高亮与折叠**：由 tree-sitter 驱动，对象和数组都能折叠；可在设置里开启自动换行。
- **搜索 / 替换**：`Ctrl+F` 搜索，`Ctrl+H` 替换（macOS 为 `Cmd+F` / `Cmd+Shift+F`），`Esc` 关闭；右侧结果区只读，只能搜索。
- **整文档粘贴自动识别**：编辑器为空或已全选时粘贴，JSON、XML、URL 参数（`a=1&b=2` 或完整 URL）、多行 YAML 都会转成格式化后的 JSON；无法识别时原样粘贴。
- **工具栏**：格式化（4 空格缩进）、压缩、转义、去除转义、转为 XML、复制。改写左侧原文的操作都可以 `Ctrl+Z` 撤销。
- **不丢数据**：保留键的顺序和数字的原始写法（如 `1.50`、超过 64 位的整数）；遇到重复的键会直接报错并给出行列号，而不是悄悄只保留其中一个值。
- **JS 过滤**：底部输入框里的表达式会接在 `this` 后面，在 QuickJS 中执行，结果显示在右侧：

  | 输入 | 含义 |
  |---|---|
  | `.data.items` | 取 `this.data.items` |
  | `[0].id` | 取第一项的 `id` |
  | `.items.map(x => x.name)` | 取出每一项的 `name` |
  | `.items.filter(x => x.age > 30).length` | 统计满足条件的项数 |
  | `;Object.keys(this)` | 列出顶层的所有键 |

  每次求值都会从原文重新解析，表达式中的修改（如 `.reverse()`）不会影响下一次；单次执行超过 2 秒会自动中断。
- **转为 XML**：有过滤表达式时转换过滤结果，否则转换原文；XML 结果同样带语法高亮。
- **草稿自动保存**：编辑停顿 1 秒后保存，下次打开时自动恢复。
- **后台处理**：识别转换、格式化、校验和转 XML 都在后台线程完成，大文档也不会卡住界面（开启自动换行后，超大文档的载入会明显变慢）。

### 文本对比

输入 `diff`、`compare` 或「对比」打开。

- **编辑视图**：左右两个编辑器分别是「原文」和「修改后」，带行号，可用 `Ctrl+F` / `Ctrl+H` 搜索替换。代码按语言着色：默认自动识别，也可在底栏的语言下拉框里指定（Bash、C、C++、C#、CSS、Go、HTML、Java、JavaScript、JSON、Markdown、Python、Rust、SQL、TOML、TSX、TypeScript、XML、YAML）。底栏可以交换两侧、清空两侧（都能 `Ctrl+Z` 撤销）。两侧内容停止输入 1 秒后自动保存，下次打开时恢复。
- **对比视图**：按 `Ctrl+Enter`（macOS 为 `Cmd+Enter`）或点「对比」进入，只读：
  - 可切换并排 / 统一两种布局；
  - 删除行红底、新增行绿底，变化的词（或字符）加深；
  - 默认显示全部行；底栏按钮可切换为只保留每处变化前后 3 行、其余折叠（可逐段展开），选择会记住；
  - 按同一语言语法着色；
  - 底栏显示 `+N −M` 统计，`Alt+↑` / `Alt+↓` 跳到上一处 / 下一处变化，`Esc` 回到编辑。
- **对比开关**：在对比视图底栏随时切换，结果立即刷新，选择会记住：
  - 忽略空白（同 `git diff -w`）；
  - 忽略大小写；
  - 先格式化 JSON（按键名排序、4 空格缩进后再比，去掉格式和键顺序带来的差异）；
  - 字符级高亮（默认按词）。
- **不算差异**：`\r\n` 与 `\n`、末尾有没有换行。
- **后台计算**：大文本也不会卡住界面；超过 2 秒的计算会给出正确但可能不是最短的结果。

### 设置

从托盘菜单、启动器里的「设置」项或 `rtools settings` 打开设置窗口，修改立即生效：

- **外观**：明暗模式（跟随系统 / 浅色 / 深色）、浅色主题、深色主题、界面字体、编辑器字体、字号（12–20）、窗口圆角（0–24）。内置 GPUI Kit 官方 36 种配色和 2 种默认配色，包含 Catppuccin、Tokyo Night、Ayu、Gruvbox、Solarized 等；主题下拉框支持搜索，界面与代码配色同步切换并保存，离线可用。
- **语言**：跟随系统 / 简体中文 / English
- **JSON 编辑器**：自动换行

## 安装（macOS）

依赖 Rust stable 和 Xcode Command Line Tools（未安装时运行 `xcode-select --install`）。打包使用 macOS 自带的 `sips`、`iconutil`、`plutil` 和 `codesign`，无需安装额外的图标转换工具。

在项目根目录执行：

```bash
./packaging/macos/bundle.sh
```

脚本会构建 release 版本，在 `target/release/rtools.app` 中放入可执行文件、应用信息和包含 Retina 尺寸的应用图标，并完成本地临时签名与校验。版本号取自 Cargo 包元数据，图标由 `assets/icons/rtools.svg` 生成；菜单栏图标已经内嵌在可执行文件中。产物对应当前 Mac 的原生架构，不是 Universal Binary。

把生成的 `rtools.app` 拖到「应用程序」，或安装到当前用户目录后启动：

```bash
mkdir -p "$HOME/Applications"
ditto target/release/rtools.app "$HOME/Applications/rtools.app"
open "$HOME/Applications/rtools.app"
```

更新时先从菜单栏退出 rtools，再重新打包并复制。打包脚本不会自动安装应用、创建命令行快捷方式或设置开机启动；安装 `.app` 后仍可直接调用包内程序：

```bash
"$HOME/Applications/rtools.app/Contents/MacOS/rtools" toggle
```

双击 `Ctrl` 使用 macOS 原始键盘事件监听，不读取输入法布局或转换输入字符。需要在「系统设置 → 隐私与安全性 → 输入监控」中添加安装好的 `rtools.app` 并开启权限，然后重启应用；从终端启动时可能还需要为终端应用授权。未授权时可通过菜单栏呼出。

**更新后双击 Ctrl 失效**：默认的临时签名（ad-hoc）会随构建内容变化，系统可能仍显示旧版本已授权，但新版本不满足旧授权的签名要求。先退出 rtools，在「输入监控」中选中旧的 rtools 条目并点减号移除，再点加号选择当前安装的 `rtools.app`、开启权限，最后重新启动。应用启动时会检查当前构建的实际权限，不把缺少键盘事件的监听报告为已启用。

如果钥匙串中已有代码签名证书，可在每次构建时使用同一身份，避免因 ad-hoc 签名变化反复重新授权：

```bash
CODESIGN_IDENTITY='Apple Development: Your Name (TEAMID)' ./packaging/macos/bundle.sh
```

首次从临时签名切换为证书签名后仍需重新授权。脚本默认使用本地临时签名，不执行 Apple 公证；对外发布仍需另行配置 Developer ID 签名和公证流程。

## 安装（Linux）

依赖：

- Rust stable（仓库中的 `rust-toolchain.toml` 会自动选用）
- C 编译器（QuickJS 与 tree-sitter 需要编译 C 代码）
- GPUI 在 Linux 上的常规依赖：Wayland / X11 开发库、xkbcommon、可用的 Vulkan 驱动

```bash
git clone https://github.com/YageGeng/rootls.git
cd rootls
./packaging/linux/install.sh
```

脚本会构建 release 版本，并安装到当前用户目录：

| 内容 | 位置 |
|---|---|
| 可执行文件（已去除调试符号） | `~/.local/bin/rtools` |
| 桌面入口 | `~/.local/share/applications/dev.rtools.rtools.desktop` |
| 应用图标 | `~/.local/share/icons/hicolor/{scalable,32x32,256x256}/apps/` |

双击 `Ctrl` 需要读取 `/dev/input` 下的键盘事件。请把自己加入 `input` 组，然后**注销并重新登录**：

```bash
sudo usermod -aG input $USER
```

没有这项权限时其他功能照常可用，只是需要用托盘图标或下面的命令呼出启动器，也可以把 `rtools toggle` 绑定到系统快捷键。

## 使用

### 命令行

| 命令 | 作用 |
|---|---|
| `rtools` 或 `rtools toggle` | 显示或隐藏启动器；未运行时先启动再显示 |
| `rtools settings` | 打开设置窗口 |
| `rtools quit` | 退出后台进程 |

### 快捷键

| 位置 | 按键 | 作用 |
|---|---|---|
| 任意位置 | 双击 `Ctrl` | 显示 / 隐藏启动器 |
| 启动器列表 | `↑` `↓` / `Enter` | 选择 / 打开 |
| 启动器列表 | `Esc` | 有已粘贴的内容时清除，否则隐藏 |
| 插件 | `Ctrl+D` | 分离到独立窗口 |
| JSON 编辑器 | `Ctrl+F` / `Ctrl+H` | 搜索 / 替换 |
| 文本对比（编辑） | `Ctrl+Enter` | 进入对比视图 |
| 文本对比（对比） | `Alt+↑` / `Alt+↓` | 上一处 / 下一处变化 |
| 文本对比（对比） | `Esc` | 回到编辑视图 |
| 设置窗口 | `Esc` | 关闭 |

## 配置与数据

设置保存在 `~/.config/rtools/settings.toml`，可以在设置窗口里点「在设置文件中编辑」直接修改；手动修改会在下次呼出启动器时生效。

```toml
ui_font = "Inter"              # 省略则跟随主题
mono_font = "JetBrains Mono"   # 省略则跟随主题
font_size = 16.0               # 12–20
window_radius = 4.0            # 0–24
language = "system"            # system / zh-CN / en
appearance = "system"          # system / light / dark，默认跟随系统
light_theme = "Default Light"  # 也可选择 Catppuccin Latte、Solarized Light 等
dark_theme = "Default Dark"    # 也可选择 Catppuccin Mocha、Tokyo Night 等

[json]
soft_wrap = false

[diff]                         # 由文本对比的对比视图修改
layout = "split"               # split（并排）/ unified（统一）
ignore_whitespace = false
ignore_case = false
format_json = false
char_level = false             # false 为词级高亮
fold_unchanged = false         # true 时折叠未改动的行
```

| 数据 | 位置 |
|---|---|
| JSON 编辑器草稿 | `~/.local/share/rtools/json/draft.json` |
| 文本对比草稿 | `~/.local/share/rtools/diff/original.txt`、`modified.txt` |
| 单实例套接字 | `$XDG_RUNTIME_DIR/rtools.sock` |

## 开发

```bash
cargo run --release                          # 运行
cargo test                                   # 测试
cargo clippy --all-targets -- -D warnings    # 静态检查
cargo fmt --all                              # 格式化
```

- **代码结构**（`crates/rtools/src`）：`shell` 管理窗口与命令，`launcher` 是启动器，`settings` 是设置与设置窗口，`hotkey` 负责双击 `Ctrl`，`tray` 是托盘，`instance` 负责单实例，`plugin` 定义插件契约（含共享的草稿自动保存），`ui` 是启动器与插件共用的界面件；插件位于 `plugins/`（`json`、`diff`），每个都分为不依赖界面的 `core` 和界面层 `ui`；插件之间互不引用。
- **设计文档**：`docs/superpowers/specs/2026-11-13-rtools-design.md`，文本对比见 `2026-11-14-rtools-diff-plugin-design.md`
- **约定**：代码规范见 `AGENTS.md`，提交信息格式见 `.gitmessage`；仓库配置了 pre-commit（fmt + clippy）。

## 已知限制

- 过滤结果中超过 2^53 的整数会丢失精度（JavaScript 数字的限制）；格式化、压缩等不经过 JavaScript 的操作不受影响。
- 多显示器时，Wayland 下的启动器总是出现在第一块屏幕上。
- 调整后的面板大小只在本次运行期间保留。
- 文本对比「先格式化 JSON」时，数字的指数写法会统一（`1e2` 写成 `1e+2`），只差指数写法的数字不算差异。
- 手改 `settings.toml` 写错时按默认值运行；之后在界面里改设置，原文件会先另存为 `settings.toml.broken` 再写入。
- macOS 运行时功能及 Windows 尚未完整验证。
