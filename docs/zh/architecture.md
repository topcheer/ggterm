# GGTerm 架构文档

> **版本:** Phase 55+ | **代码量:** 71,791 行 Rust | **测试:** 2,143 个 | **Crate 数:** 9

## 概述

GGTerm 是一个用 Rust 编写的 GPU 加速、AI 原生、跨平台终端模拟器。支持桌面端 (macOS/Linux/Windows) 和移动端 (iOS/Android)。

```
┌──────────────────────────────────────────────────────┐
│                    GGTerm 二进制                      │
│  crates/ggterm-app/src/bin/ggterm.rs (clap CLI)     │
├────────────┬──────────────┬───────────────────────────┤
│ ggterm-app │ ggterm-core  │ ggterm-render-wgpu       │
│  桌面应用   │  终端引擎    │  GPU 渲染                │
│  窗口管理   │  网格/单元格 │  glyphon + wgpu          │
│  标签/分屏  │  VTE 解析器  │  SDF 着色器              │
│  配置系统   │  PTY 传输    │  多面板渲染              │
├────────────┼──────────────┼───────────────────────────┤
│ ggterm-ssh │ ggterm-p2p   │ ggterm-ffi               │
│  SSH 连接  │  Iroh/QUIC   │  Flutter C-ABI 接口      │
├────────────┼──────────────┼───────────────────────────┤
│ ggterm-ai  │ ggterm-plugin│ ggterm-render            │
│  AI 桥接   │  Lua/WASM    │  主题/光标               │
└────────────┴──────────────┴───────────────────────────┘
```

## Crate 详情

| Crate | 代码量 | 职责 |
|-------|--------|------|
| `ggterm-core` | 13,505 | VTE 解析器、网格模型、终端状态机、PTY 传输 trait |
| `ggterm-app` | 43,412 | 桌面应用：winit 窗口、事件循环、标签页、分屏、配置、事件处理 |
| `ggterm-render-wgpu` | 2,979 | wgpu GPU 渲染器：文本、装饰线、SDF UI、多面板视口 |
| `ggterm-render` | 1,739 | 渲染 trait、主题定义 (9 种主题)、光标状态 |
| `ggterm-ffi` | 2,597 | Flutter 的 C-ABI 接口：会话生命周期、字节处理、单元格读取 |
| `ggterm-ai` | 2,250 | AI 引擎桥接 (OpenAI 兼容 API 客户端) |
| `ggterm-plugin` | 3,925 | 插件管理器 (Lua + WASM 运行时) |
| `ggterm-ssh` | 644 | SSH 传输 (russh 0.61，异步→同步桥接) |
| `ggterm-p2p` | 740 | P2P 终端共享 (iroh QUIC + NAT 穿透) |

## 核心架构

### 1. 终端引擎 (`ggterm-core`)

终端引擎实现了完整的 VT100/VT220/xterm 兼容终端状态机。

#### VTE 解析器

基于 Paul Williams 状态机实现 (`vte/parser.rs`)。支持：
- CSI（控制序列引导符）— 所有标准和私有模式
- OSC（操作系统命令）— 0, 2, 4, 7, 8, 9, 10-12, 52, 133, 1337, 9;4
- DCS（设备控制字符串）— XTGETTCAP, DECRQSS
- ESC 序列 — DECSC/DECRC, DECPAM/DECPNM, RIS, IND, NEL
- SOS/PM/APC — 消费但不打印负载
- 字符串缓冲区溢出保护（OSC 64KB，DCS 1MB）

#### 网格模型

```
Grid
├── rows: Vec<Row>           — 可见行 + 回滚历史
├── scrollback_len: usize    — 最大回滚历史行数
├── scroll_region: (top, bottom) — DECSTBM 滚动区域
├── display_offset: usize    — 视口滚动位置
├── content_dirty: bool      — 优化：干净时跳过重绘
└── marks: Vec<usize>        — OSC 1337 SetMark 位置

Row
├── cells: Vec<Cell>         — 每列一个单元格
└── dirty: bool

Cell (Clone, 非 Copy)
├── ch: char                 — 基础字符
├── combining: Vec<char>     — 零宽组合标记 (é, ü, emoji 修饰符)
├── fg: Color                — 前景色
├── bg: Color                — 背景色
├── underline_color: Option<Color> — SGR 58 下划线颜色
├── flags: CellFlags         — 位标志 (粗体、斜体、下划线等)
└── hyperlink: Option<String> — OSC 8 超链接 URL
```

#### 终端状态

`Terminal` 结构体持有：
- 光标位置 (row, col) + pending_wrap 标志
- SGR 属性 (当前 fg, bg, flags)
- 字符集 (G0, G1, 活动集)
- 模式 (DEC 私有模式 + ANSI 模式)
- 滚动区域 (DECSTBM)
- 动态颜色 (OSC 10/11/12 覆盖)
- 调色板覆盖 (OSC 4, 兼容 base16-shell)
- 响应缓冲区 (DA1, DSR, DECRQM 回复)
- 命令标记 (OSC 133, shell 集成)
- 交替屏幕网格交换 (DECSET 47/1047/1049)
- DECSC 保存状态 (完整：光标 + SGR + 字符集 + 自动换行)

#### PTY 传输 Trait

```rust
pub trait TerminalTransport {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    fn is_alive(&self) -> bool;
}
```

实现：`PtySession` (本地 PTY)、`SshSession` (russh)、`P2pTransport` (iroh QUIC)。

### 2. 桌面应用 (`ggterm-app`)

#### 窗口模块结构

```
window/
├── mod.rs       — DesktopApp 结构体、构造函数、ApplicationHandler、事件循环
├── handlers.rs  — 键盘、鼠标、光标、调整大小事件处理
├── actions.rs   — 标签/分屏/剪贴板/主题/字体/会话/拖放操作
└── render.rs    — render_frame()、多面板渲染、叠加层组合
```

#### 会话层次结构

```
DesktopApp
├── sessions: Vec<TabSession>     — 每个标签一个
├── active: usize                 — 活动标签索引
│
TabSession
├── panes: Vec<Option<PaneSession>> — 每个分屏一个
├── split_tree: SplitTree          — 分屏二叉树
├── title: String                  — 从 OSC 0/2 同步
└── cwd: Option<PathBuf>           — 来自 OSC 7
│
PaneSession
├── app: App                       — 终端 + 网格包装器
├── pty: PtySession                — PTY 传输
├── cwd: Option<PathBuf>           — 工作目录
└── needs_reprepare: bool          — 脏矩形优化
```

#### 事件循环 (`about_to_wait`)

```
1. 泵送 PTY → 处理字节 → 刷新终端响应
2. 检查 content_dirty → 条件性重绘
3. 轮询配置重载 → 应用主题/字体/回滚变更
4. 轮询响铃 → 播放声音、视觉闪烁
5. 轮询 OSC 52 剪贴板 → 设置/查询系统剪贴板
6. 轮询通知 → 桌面通知
7. 轮询 P2P → 转发输出、输入
8. 更新光标闪烁阶段
9. 更新 toast 倒计时
10. 从终端同步标签标题
11. 空闲休眠 (不需要重绘时休眠 50ms)
```

### 3. GPU 渲染器 (`ggterm-render-wgpu`)

#### 渲染管线

1. **网格 → TextRun** (`converter.rs`)：每行网格转换为带主题颜色解析的 `TextRun` 条目。宽字符 (CJK/emoji) 强制拆分 run。

2. **TextRun → glyphon TextArea** (`lib.rs`)：每个 run 成为独立的 `glyphon::Buffer`，定位在 `(start_col × cell_width, row × cell_height)`。

3. **装饰线** (`lib.rs`)：下划线、删除线、上划线在 `prepare_decorations()` 中生成为顶点数据。

4. **SDF UI** (`ui.wgsl`)：圆角矩形用于标签栏、状态栏、菜单、对话框，通过带符号距离场着色器实现。

5. **多面板渲染** (`gpu.rs`)：
   - 一个渲染通道渲染所有面板
   - 每个面板：`set_scissor_rect()` → `set_viewport_offset()` → `render_pane_to_pass()`
   - 最终叠加层通道：`render_overlays_to_pass()` (边框、标签栏、状态栏、菜单)

#### 字体处理

- **macOS**: Menlo (仅 Weight::NORMAL — Bold 变体缺少制表符字形)
- **Linux**: DejaVu Sans Mono
- **Windows**: Cascadia Mono
- 粗体通过亮色区分，不使用字重 (xterm/Alacritty 标准)
- `Shaping::Advanced` 启用 CJK 字体回退
- 单元格宽度 = 'M' 的精确浮点前进值 (不四舍五入)

### 4. 配置系统 (`config.rs`)

TOML 格式，位于 `~/.ggterm/config.toml`：

```toml
[appearance]
theme = "dark"           # 9 种主题 + auto
font_family = "monospace"
font_size = 14
cursor_style = "block"     # block | underline | bar
background_opacity = 1.0   # 0.0 透明 → 1.0 不透明

[terminal]
scrollback_lines = 10000
shell = ""                # 空 = $SHELL 或 /bin/sh
restore_session = false

[ai]
enabled = false
api_endpoint = ""
model = ""

[keybindings]
# 可自定义的键盘快捷键
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
# ... (见 config.example.toml)

[profiles.develop]
# 配置文件，带可选覆盖
theme = "nord"
font_size = 12
```

热重载：`config-watch` 功能使用 `notify` crate 监视配置文件。`about_to_wait()` 中检测到变更后立即应用（主题切换、字体调整、回滚更新）。

### 5. 移动端 FFI (`ggterm-ffi`)

Flutter 集成的 C-ABI 函数：

```c
// 会话生命周期
uint32_t ggterm_session_create(uint32_t cols, uint32_t rows);
void     ggterm_session_destroy(uint32_t id);

// 终端操作
uint32_t ggterm_session_process_bytes(uint32_t id, const uint8_t* data, uint32_t len);
void     ggterm_session_send_input(uint32_t id, const uint8_t* data, uint32_t len);
uint32_t ggterm_session_take_input(uint32_t id, uint8_t* buf, uint32_t max);
uint32_t ggterm_session_read_cells(uint32_t id, GGTermCell* cells, uint32_t max);

// 尺寸和光标
void     ggterm_session_dimensions(uint32_t id, uint32_t* cols, uint32_t* rows);
void     ggterm_session_cursor(uint32_t id, uint32_t* col, uint32_t* row, uint32_t* style);

// 传输
uint32_t ggterm_transport_pump(uint32_t id);
void     ggterm_transport_flush(uint32_t id);
uint32_t ggterm_transport_is_alive(uint32_t id);

// SSH (feature = "ssh")
int32_t  ggterm_ssh_connect(uint32_t id, const char* host, uint16_t port,
                            const char* user, const char* password);

// P2P (feature = "p2p")
const char* ggterm_p2p_generate_ticket(void);
int32_t  ggterm_p2p_connect(const char* ticket);
int32_t  ggterm_p2p_is_connected(uint32_t session_id);
```

全局会话注册表通过 `OnceLock<Mutex<HashMap<u32, MobileSession>>>`。Mutex 锁使用 `unwrap_or_else(|e| e.into_inner())` 确保恐慌安全。

### 6. P2P 终端共享 (`ggterm-p2p`)

使用 iroh 1.0 (QUIC + NAT 穿透) 实现移动端↔桌面端直接终端共享。

```
桌面 (主机)                       移动端 (客户端)
┌──────────────┐                  ┌──────────────┐
│ P2pHost      │ ◄── QUIC ──►    │ P2pClient     │
│  Endpoint    │   (P2P/中继)     │  connect()    │
│  PTY ↔ 流    │                  │  流 ↔ 终端    │
└──────────────┘                  └──────────────┘
```

- 票据格式：base32(postcard(EndpointAddr)) — ~130 字符，适配一个 QR 码
- P2P 直连成功率：90%+ (iroh 中继回退)
- 零运营成本 (iroh 公共中继免费)
- 后台 tokio 任务桥接异步 QUIC ↔ 同步缓冲区

### 7. AI 集成 (`ggterm-ai`)

OpenAI 兼容 API 客户端，支持：
- 命令解释 (Ctrl+Shift+E)
- 代码建议 (Ctrl+Shift+S)
- 帮助 (Ctrl+Shift+H)
- 自然语言转命令 (Ctrl+Shift+N)

### 8. 插件系统 (`ggterm-plugin`)

双运行时：Lua (通过 `mlua`) 和 WASM。通过以下配置启用：

```toml
[plugins]
enabled = false
directory = "~/.ggterm/plugins"
```

示例插件：`examples/plugins/hello.lua`

## 协议支持

### 终端协议

| 协议 | 序列 | 状态 |
|------|------|------|
| 光标 | CSI A/B/C/D/E/F/G/H/f | 完整 |
| 擦除 | ED (J), EL (K), DECSED | 完整 (选择性擦除) |
| SGR | 0-9, 21, 53-55, 58-59 | 完整 (含上划线、下划线颜色) |
| DECSET | 1, 5, 6, 7, 12, 25, 47, 1000-1006, 1015-1016, 1047-1049, 2004, 2026, 2027 | 完整 |
| OSC | 0, 2, 4, 7, 8, 9, 10-12, 52, 104, 110-112, 133, 1337, 9;4 | 完整 |
| DCS | XTGETTCAP, DECRQSS | 完整 |
| 字符集 | G0/G1, US/UK/特殊图形 | 完整 |
| DECSC/DECRC | ESC 7/8 (完整状态保存) | 完整 |
| DECSTR | CSI ! p (软重置) | 完整 |
| Kitty 键盘 | CSI > u push/pop, CSI = u | 完整 |
| DA1/DA2/DA3 | DA 序列 | 完整 |
| DSR | CSI 6n, 5n, ?6n 等 | 完整 |
| DECRQM | 模式查询 (ANSI + 私有) | 完整 |

### Shell 集成

OSC 133 标记用于命令检测：
- `OSC 133 ; A ST` — 提示符开始
- `OSC 133 ; B ST` — 命令开始
- `OSC 133 ; C ST` — 输出开始
- `OSC 133 ; D ; <exit_code> ST` — 命令退出

支持的 shell：bash、zsh、fish (通过 `shell/` 脚本自动注入)。

## 功能标志

| 功能 | Crate | 说明 |
|------|-------|------|
| `desktop` | ggterm-app | winit 窗口 + wgpu 渲染 + PTY |
| `ai` | ggterm-app, ggterm-ffi | AI 助手集成 |
| `plugin` | ggterm-app | 插件管理框架 |
| `plugin-lua` | ggterm-app | Lua 插件运行时 (隐含 `plugin`) |
| `config-watch` | ggterm-app | 通过文件系统监视热重载配置 |
| `ssh` | ggterm-ffi | 移动端 SSH 传输 |
| `p2p` | ggterm-app, ggterm-ffi | P2P 终端共享 |

**标准桌面构建：** `desktop ai plugin plugin-lua config-watch`

## 构建系统

```bash
# Debug 构建
make build

# Release 构建
make release

# 运行测试
make test                    # 2,143 个测试

# 代码检查 (零警告)
make clippy

# 格式检查
make fmt
```

## CI/CD

- **ci.yml** — 格式检查、clippy、测试 (Linux + macOS)、构建 (Linux + macOS + Windows)
- **release-desktop.yml** — Tag `v*` → macOS 通用 .dmg、Linux .deb、Windows .zip
- **release-mobile.yml** — Tag `v*` → Android APK、iOS IPA
