# GGTerm Architecture

> **Version:** Phase 55+ | **LOC:** 71,791 Rust | **Tests:** 2,143 | **Crates:** 9

## Overview

GGTerm は、Rust で記述された GPU アクセラレーション対応の AI ネイティブクロスプラットフォームターミナルエミュレータです。デスクトップ（macOS/Linux/Windows）およびモバイル（iOS/Android）プラットフォームをターゲットとしています。

```
┌──────────────────────────────────────────────────────┐
│                    GGTerm Binary                     │
│  crates/ggterm-app/src/bin/ggterm.rs (clap CLI)     │
├────────────┬──────────────┬───────────────────────────┤
│ ggterm-app │ ggterm-core  │ ggterm-render-wgpu       │
│  Desktop   │  Terminal    │  GPU Rendering           │
│  Window    │  Grid/Cell   │  glyphon + wgpu          │
│  Tabs/Split│  VTE Parser  │  SDF Shaders             │
│  Config    │  PTY Trait   │  Per-Pane Rendering      │
├────────────┼──────────────┼───────────────────────────┤
│ ggterm-ssh │ ggterm-p2p   │ ggterm-ffi               │
│  SSH Conn  │  Iroh/QUIC   │  C-ABI for Flutter       │
├────────────┼──────────────┼───────────────────────────┤
│ ggterm-ai  │ ggterm-plugin│ ggterm-render            │
│  AI Bridge │  Lua/WASM    │  Theme/Cursor            │
└────────────┴──────────────┴───────────────────────────┘
```

## Crate Breakdown

| Crate | LOC | Responsibility |
|-------|-----|---------------|
| `ggterm-core` | 13,505 | VTE parser, grid model, terminal state machine, PTY transport trait |
| `ggterm-app` | 43,412 | デスクトップアプリ: winit window, event loop, tabs, splits, config, handlers |
| `ggterm-render-wgpu` | 2,979 | wgpu GPU renderer: text, decorations, SDF UI, multi-pane viewport |
| `ggterm-render` | 1,739 | Render trait, theme definitions（9 themes）, cursor state |
| `ggterm-ffi` | 2,597 | C-ABI for Flutter: session lifecycle, byte processing, cell reading |
| `ggterm-ai` | 2,250 | AI engine bridge（OpenAI 互換 API client） |
| `ggterm-plugin` | 3,925 | Plugin manager（Lua + WASM runtimes） |
| `ggterm-ssh` | 644 | SSH transport via russh 0.61（async→sync bridge） |
| `ggterm-p2p` | 740 | P2P terminal sharing via iroh（QUIC + NAT traversal） |

## Core Architecture

### 1. Terminal Engine (`ggterm-core`)

Terminal engine は、完全な VT100/VT220/xterm 互換の terminal state machine を実装しています。

#### VTE Parser

Paul Williams state machine（`vte/parser.rs`）に基づいています。以下をサポートしています:
- CSI (Control Sequence Introducer) — すべての standard および private modes
- OSC (Operating System Command) — 0, 2, 4, 7, 8, 9, 10-12, 52, 133, 1337, 9;4
- DCS (Device Control String) — XTGETTCAP, DECRQSS
- ESC sequences — DECSC/DECRC, DECPAM/DECPNM, RIS, IND, NEL
- SOS/PM/APC — 消費されます（payload は出力されません）
- String buffer overflow protection（OSC 64KB, DCS 1MB）

#### Grid Model

```
Grid
├── rows: Vec<Row>           — visible + scrollback
├── scrollback_len: usize    — max scrollback history
├── scroll_region: (top, bottom) — DECSTBM margins
├── display_offset: usize    — viewport scroll position
├── content_dirty: bool      — optimization: skip redraw when clean
└── marks: Vec<usize>        — OSC 1337 SetMark positions

Row
├── cells: Vec<Cell>         — one per column
└── dirty: bool

Cell (Clone, not Copy)
├── ch: char                 — base character
├── combining: Vec<char>     — zero-width marks (é, ü, emoji)
├── fg: Color                — foreground color
├── bg: Color                — background color
├── underline_color: Option<Color> — SGR 58
├── flags: CellFlags         — bitflags (bold, italic, underline, etc.)
└── hyperlink: Option<String> — OSC 8 URL
```

#### Terminal State

`Terminal` struct は以下を保持します:
- Cursor position（row, col）+ pending_wrap flag
- SGR attributes（current fg, bg, flags）
- Character sets（G0, G1, active set）
- Modes（DEC private + ANSI modes）
- Scroll region（DECSTBM）
- Dynamic colors（OSC 10/11/12 overrides）
- Palette overrides（OSC 4, base16-shell 互換）
- Response buffer（DA1, DSR, DECRQM replies）
- Command marks（OSC 133, shell integration）
- Alt-screen grid swap（DECSET 47/1047/1049）
- DECSC saved state（full: cursor + SGR + charset + autowrap）

#### PTY Transport Trait

```rust
pub trait TerminalTransport {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    fn is_alive(&self) -> bool;
}
```

実装: `PtySession`（local PTY）, `SshSession`（russh）, `P2pTransport`（iroh QUIC）。

### 2. Desktop Application (`ggterm-app`)

#### Window Module Structure

```
window/
├── mod.rs       — DesktopApp struct, constructor, ApplicationHandler, event loop
├── handlers.rs  — Keyboard, mouse, cursor, resize event handlers
├── actions.rs   — Tab/split/clipboard/theme/font/session/drag-drop operations
└── render.rs    — render_frame(), multi-pane rendering, overlay composition
```

#### Session Hierarchy

```
DesktopApp
├── sessions: Vec<TabSession>     — one per tab
├── active: usize                 — active tab index
│
TabSession
├── panes: Vec<Option<PaneSession>> — one per split pane
├── split_tree: SplitTree          — binary tree of splits
├── title: String                  — synced from OSC 0/2
└── cwd: Option<PathBuf>           — from OSC 7
│
PaneSession
├── app: App                       — terminal + grid wrapper
├── pty: PtySession                — PTY transport
├── cwd: Option<PathBuf>           — working directory
└── needs_reprepare: bool          — dirty rect optimization
```

#### Event Loop (`about_to_wait`)

```
1. Pump PTY → process bytes → flush terminal responses
2. Check content_dirty → conditional redraw
3. Poll config reload → apply theme/font/scrollback changes
4. Poll bell → play sound, visual flash
5. Poll OSC 52 clipboard → set/query system clipboard
6. Poll notifications → desktop notification
7. Poll P2P → tee output, forward input
8. Update cursor blink phase
9. Update toast countdown
10. Sync tab titles from terminal
11. Idle sleep (50ms when no redraw needed)
```

### 3. GPU Renderer (`ggterm-render-wgpu`)

#### Rendering Pipeline

1. **Grid → TextRuns**（`converter.rs`）: 各 grid row は theme color resolution とともに `TextRun` エントリに変換されます。Wide chars（CJK/emoji）は run の分割を強制します。

2. **TextRun → glyphon TextAreas**（`lib.rs`）: 各 run は `(start_col × cell_width, row × cell_height)` の位置に配置された独立した `glyphon::Buffer` になります。

3. **Decorations**（`lib.rs`）: Underline, strikethrough, overline は `prepare_decorations()` 内で vertex data として生成されます。

4. **SDF UI**（`ui.wgsl`）: Tab bar, status bar, menu, dialog のための signed distance field shader による角丸矩形描画。

5. **Multi-Pane Rendering**（`gpu.rs`）:
   - すべての pane に対して1つの render pass
   - 各 pane: `set_scissor_rect()` → `set_viewport_offset()` → `render_pane_to_pass()`
   - 最終 overlay pass: `render_overlays_to_pass()`（borders, tab bar, status bar, menus）

#### Font Handling

- **macOS**: Menlo（Weight::NORMAL のみ — Bold variant は box-drawing glyph を欠落しています）
- **Linux**: DejaVu Sans Mono
- **Windows**: Cascadia Mono
- Bold は weight ではなく bright color で区別されます（xterm/Alacritty standard）
- `Shaping::Advanced` により CJK font fallback が有効になります
- Cell width = 'M' の正確な float advance（丸めなし）

### 4. Configuration (`config.rs`)

`~/.ggterm/config.toml` の TOML 形式:

```toml
[appearance]
theme = "dark"           # 9 themes + auto
font_family = "monospace"
font_size = 14
cursor_style = "block"     # block | underline | bar
background_opacity = 1.0   # 0.0 transparent → 1.0 opaque

[terminal]
scrollback_lines = 10000
shell = ""                # empty = $SHELL or /bin/sh
restore_session = false

[ai]
enabled = false
api_endpoint = ""
model = ""

[keybindings]
# Customizable keyboard shortcuts
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
# ... (see config.example.toml)

[profiles.develop]
# Profile with optional overrides
theme = "nord"
font_size = 12
```

Hot-reload: `config-watch` feature は `notify` crate を使用して config file を監視します。`about_to_wait()` で検出された変更は即座に適用されます（theme switch, font resize, scrollback update）。

### 5. Mobile FFI (`ggterm-ffi`)

Flutter integration 用の C-ABI 関数:

```c
// Session lifecycle
uint32_t ggterm_session_create(uint32_t cols, uint32_t rows);
void     ggterm_session_destroy(uint32_t id);

// Terminal operations
uint32_t ggterm_session_process_bytes(uint32_t id, const uint8_t* data, uint32_t len);
void     ggterm_session_send_input(uint32_t id, const uint8_t* data, uint32_t len);
uint32_t ggterm_session_take_input(uint32_t id, uint8_t* buf, uint32_t max);
uint32_t ggterm_session_read_cells(uint32_t id, GGTermCell* cells, uint32_t max);

// Dimensions & cursor
void     ggterm_session_dimensions(uint32_t id, uint32_t* cols, uint32_t* rows);
void     ggterm_session_cursor(uint32_t id, uint32_t* col, uint32_t* row, uint32_t* style);

// Transport
uint32_t ggterm_transport_pump(uint32_t id);
void     ggterm_transport_flush(uint32_t id);
uint32_t ggterm_transport_is_alive(uint32_t id);

// SSH (feature = "ssh")
int32_t  ggterm_ssh_connect(uint32_t id, const char* host, uint16_t port,
                            const char* user, const char* password);
int32_t  ggterm_ssh_connect_key(uint32_t id, const char* host, uint16_t port,
                                 const char* user, const char* key_path);

// P2P (feature = "p2p")
const char* ggterm_p2p_generate_ticket(void);
const char* ggterm_p2p_host_ticket(uint32_t session_id);
int32_t  ggterm_p2p_connect(const char* ticket);
int32_t  ggterm_p2p_is_connected(uint32_t session_id);
```

Global session registry は `OnceLock<Mutex<HashMap<u32, MobileSession>>>` で実装されています。Mutex lock は panic safety のため `unwrap_or_else(|e| e.into_inner())` を使用します。

### 6. P2P Terminal Sharing (`ggterm-p2p`)

iroh 1.0（QUIC + NAT traversal）を使用して、モバイルとデスクトップ間の直接的な terminal sharing を実現します。

```
Desktop (Host)                    Mobile (Client)
┌──────────────┐                  ┌──────────────┐
│ P2pHost      │ ◄── QUIC ──►    │ P2pClient     │
│  Endpoint    │   (P2P/relay)    │  connect()    │
│  PTY ↔ Stream│                  │  Stream ↔ Term│
└──────────────┘                  └──────────────┘
```

- Ticket format: base32(postcard(EndpointAddr)) — 約130文字、QR code 1枚に収まります
- P2P direct connection 成功率: 90%+（iroh relay fallback あり）
- 運用コストゼロ（iroh public relay は無料）
- Background tokio task が async QUIC ↔ sync buffers をブリッジします

### 7. AI Integration (`ggterm-ai`)

OpenAI 互換 API client。以下をサポートしています:
- Command explanation（Ctrl+Shift+E）
- Code suggestions（Ctrl+Shift+S）
- Help（Ctrl+Shift+H）
- Natural language → command（Ctrl+Shift+N）

### 8. Plugin System (`ggterm-plugin`)

デュアルランタイム: Lua（`mlua` 経由）および WASM。以下で設定します:

```toml
[plugins]
enabled = false
directory = "~/.ggterm/plugins"
```

Plugin の例: `examples/plugins/hello.lua`

## Protocol Support

### Terminal Protocols

| Protocol | Sequences | Status |
|----------|-----------|--------|
| Cursor | CSI A/B/C/D/E/F/G/H/f | Full |
| Erase | ED (J), EL (K), DECSED | Full (selective erase) |
| SGR | 0-9, 21, 53-55, 58-59 | Full (including overline, underline color) |
| DECSET | 1, 5, 6, 7, 12, 25, 47, 1000-1006, 1015-1016, 1047-1049, 2004, 2026, 2027 | Full |
| OSC | 0, 2, 4, 7, 8, 9, 10-12, 52, 104, 110-112, 133, 1337, 9;4 | Full |
| DCS | XTGETTCAP, DECRQSS | Full |
| Character sets | G0/G1, US/UK/special graphics | Full |
| DECSC/DECRC | ESC 7/8 (full state save) | Full |
| DECSTR | CSI ! p (soft reset) | Full |
| Kitty keyboard | CSI > u push/pop, CSI = u | Full |
| DA1/DA2/DA3 | DA sequences | Full |
| DSR | CSI 6n, 5n, ?6n, ?15n, ?16n, ?11t | Full |
| DECRQM | Mode query (ANSI + private) | Full |

### Shell Integration

Command detection 用の OSC 133 marks:
- `OSC 133 ; A ST` — prompt start
- `OSC 133 ; B ST` — command start
- `OSC 133 ; C ST` — output start
- `OSC 133 ; D ; <exit_code> ST` — command exit

対応シェル: bash, zsh, fish（`shell/` scripts により auto-injection）。

## Feature Flags

| Feature | Crate | Description |
|---------|-------|-------------|
| `desktop` | ggterm-app | winit window + wgpu rendering + PTY |
| `ai` | ggterm-app, ggterm-ffi | AI assistant integration |
| `plugin` | ggterm-app | Plugin manager framework |
| `plugin-lua` | ggterm-app | Lua plugin runtime（`plugin` を内包） |
| `config-watch` | ggterm-app | Hot-reload config via filesystem watching |
| `ssh` | ggterm-ffi | SSH transport for mobile |
| `p2p` | ggterm-app, ggterm-ffi | P2P terminal sharing |

**Standard desktop build:** `desktop ai plugin plugin-lua config-watch`

## Build System

```bash
# Debug build
make build

# Release build
make release

# Run tests
make test                    # 2,143 tests

# Lint (zero warnings)
make clippy

# Format check
make fmt
```

## CI/CD

- **ci.yml** — fmt, clippy, test (Linux + macOS), build (Linux + macOS + Windows)
- **release-desktop.yml** — Tag `v*` → macOS universal .dmg, Linux .deb, Windows .zip
- **release-mobile.yml** — Tag `v*` → Android APK, iOS IPA
