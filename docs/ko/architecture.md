# GGTerm 아키텍처

> **버전:** Phase 55+ | **LOC:** 71,791 Rust | **테스트:** 2,143 | **Crate:** 9

## 개요

GGTerm은 Rust로 작성된 GPU 가속, AI 네이티브, 크로스 플랫폼 터미널 에뮬레이터입니다. 데스크톱(macOS/Linux/Windows) 및 모바일(iOS/Android) 플랫폼을 지원합니다.

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

## Crate 분류

| Crate | LOC | 담당 |
|-------|-----|------|
| `ggterm-core` | 13,505 | VTE parser, grid model, 터미널 상태 머신, PTY transport trait |
| `ggterm-app` | 43,412 | 데스크톱 앱: winit window, event loop, tab, split, config, handler |
| `ggterm-render-wgpu` | 2,979 | wgpu GPU renderer: 텍스트, decoration, SDF UI, multi-pane viewport |
| `ggterm-render` | 1,739 | Render trait, 테마 정의 (9개 테마), cursor state |
| `ggterm-ffi` | 2,597 | Flutter용 C-ABI: 세션 수명 주기, 바이트 처리, cell 읽기 |
| `ggterm-ai` | 2,250 | AI 엔진 브릿지 (OpenAI 호환 API 클라이언트) |
| `ggterm-plugin` | 3,925 | Plugin manager (Lua + WASM 런타임) |
| `ggterm-ssh` | 644 | russh 0.61을 통한 SSH transport (async→sync 브릿지) |
| `ggterm-p2p` | 740 | iroh를 통한 P2P 터미널 공유 (QUIC + NAT traversal) |

## 핵심 아키텍처

### 1. 터미널 엔진 (`ggterm-core`)

터미널 엔진은 완전한 VT100/VT220/xterm 호환 터미널 상태 머신을 구현합니다.

#### VTE Parser

Paul Williams 상태 머신을 기반으로 합니다 (`vte/parser.rs`). 지원 기능:
- CSI (Control Sequence Introducer) — 모든 표준 및 private mode
- OSC (Operating System Command) — 0, 2, 4, 7, 8, 9, 10-12, 52, 133, 1337, 9;4
- DCS (Device Control String) — XTGETTCAP, DECRQSS
- ESC sequence — DECSC/DECRC, DECPAM/DECPNM, RIS, IND, NEL
- SOS/PM/APC — 소비됨 (출력 없음)
- 문자열 버퍼 오버플로우 보호 (OSC 64KB, DCS 1MB)

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

#### 터미널 상태

`Terminal` struct는 다음을 보유합니다:
- Cursor 위치 (row, col) + pending_wrap flag
- SGR 속성 (현재 fg, bg, flags)
- 문자셋 (G0, G1, 활성 셋)
- Mode (DEC private + ANSI mode)
- 스크롤 영역 (DECSTBM)
- 동적 색상 (OSC 10/11/12 override)
- 팔레트 override (OSC 4, base16-shell 호환)
- 응답 버퍼 (DA1, DSR, DECRQM 응답)
- 명령어 마크 (OSC 133, shell integration)
- Alt-screen grid 교체 (DECSET 47/1047/1049)
- DECSC 저장 상태 (전체: cursor + SGR + charset + autowrap)

#### PTY Transport Trait

```rust
pub trait TerminalTransport {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    fn is_alive(&self) -> bool;
}
```

구현체: `PtySession` (로컬 PTY), `SshSession` (russh), `P2pTransport` (iroh QUIC).

### 2. 데스크톱 애플리케이션 (`ggterm-app`)

#### Window 모듈 구조

```
window/
├── mod.rs       — DesktopApp struct, constructor, ApplicationHandler, event loop
├── handlers.rs  — Keyboard, mouse, cursor, resize event handler
├── actions.rs   — Tab/split/clipboard/theme/font/session/drag-drop 작업
└── render.rs    — render_frame(), multi-pane 렌더링, overlay 합성
```

#### 세션 계층 구조

```
DesktopApp
├── sessions: Vec<TabSession>     — tab당 하나
├── active: usize                 — 활성 tab 인덱스
│
TabSession
├── panes: Vec<Option<PaneSession>> — split pane당 하나
├── split_tree: SplitTree          — split의 이진 트리
├── title: String                  — OSC 0/2에서 동기화
└── cwd: Option<PathBuf>           — OSC 7에서 가져옴
│
PaneSession
├── app: App                       — terminal + grid 래퍼
├── pty: PtySession                — PTY transport
├── cwd: Option<PathBuf>           — 작업 디렉토리
└── needs_reprepare: bool          — dirty rect 최적화
```

#### Event Loop (`about_to_wait`)

```
1. Pump PTY → process bytes → flush terminal responses
2. Check content_dirty → 조건부 다시 그리기
3. Poll config reload → theme/font/scrollback 변경 적용
4. Poll bell → 사운드 재생, visual flash
5. Poll OSC 52 clipboard → 시스템 클립보드 설정/조회
6. Poll notification → 데스크톱 알림
7. Poll P2P → 출력 tee, 입력 전달
8. Cursor blink 단계 갱신
9. Toast 카운트다운 갱신
10. Terminal에서 tab title 동기화
11. 유휴 대기 (다시 그리기 불필요 시 50ms)
```

### 3. GPU Renderer (`ggterm-render-wgpu`)

#### 렌더링 파이프라인

1. **Grid → TextRun** (`converter.rs`): 각 grid 행이 테마 색상 해상도와 함께 `TextRun` 항목으로 변환됩니다. 와이드 문자(CJK/emoji)는 run 분할을 강제합니다.

2. **TextRun → glyphon TextArea** (`lib.rs`): 각 run은 `(start_col × cell_width, row × cell_height)` 위치에 배치된 독립적인 `glyphon::Buffer`가 됩니다.

3. **Decoration** (`lib.rs`): 밑줄, 취소선, 윗줄은 `prepare_decorations()`에서 vertex 데이터로 생성됩니다.

4. **SDF UI** (`ui.wgsl`): tab bar, status bar, menu, dialog를 위한 signed distance field shader 기반의 둥근 사각형.

5. **Multi-Pane 렌더링** (`gpu.rs`):
   - 모든 pane에 대해 하나의 render pass
   - 각 pane: `set_scissor_rect()` → `set_viewport_offset()` → `render_pane_to_pass()`
   - 최종 overlay pass: `render_overlays_to_pass()` (테두리, tab bar, status bar, menu)

#### 폰트 처리

- **macOS**: Menlo (Weight::NORMAL 전용 — Bold 변형은 box-drawing glyph 누락)
- **Linux**: DejaVu Sans Mono
- **Windows**: Cascadia Mono
- Bold는 weight가 아닌 밝은 색상으로 구분됩니다 (xterm/Alacritty 표준)
- `Shaping::Advanced`는 CJK 폰트 폴백을 활성화합니다
- Cell 너비 = 'M'의 정확한 float advance (반올림 없음)

### 4. 설정 (`config.rs`)

`~/.ggterm/config.toml`의 TOML 형식:

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

Hot-reload: `config-watch` 기능은 `notify` crate을 사용하여 config 파일을 감시합니다. `about_to_wait()`에서 감지된 변경 사항은 즉시 적용됩니다 (테마 전환, 폰트 크기 변경, scrollback 업데이트).

### 5. 모바일 FFI (`ggterm-ffi`)

Flutter 연동을 위한 C-ABI 함수:

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

`OnceLock<Mutex<HashMap<u32, MobileSession>>>`를 통한 전역 세션 레지스트리. Mutex lock은 panic 안전성을 위해 `unwrap_or_else(|e| e.into_inner())`를 사용합니다.

### 6. P2P 터미널 공유 (`ggterm-p2p`)

iroh 1.0 (QUIC + NAT traversal)을 사용하여 직접 모바일↔데스크톱 터미널 공유를 구현합니다.

```
Desktop (Host)                    Mobile (Client)
┌──────────────┐                  ┌──────────────┐
│ P2pHost      │ ◄── QUIC ──►    │ P2pClient     │
│  Endpoint    │   (P2P/relay)    │  connect()    │
│  PTY ↔ Stream│                  │  Stream ↔ Term│
└──────────────┘                  └──────────────┘
```

- Ticket 형식: base32(postcard(EndpointAddr)) — 약 130자, QR 코드 1개에 수용 가능
- P2P 직접 연결 성공률: 90% 이상 (iroh relay 폴백)
- 운영 비용 없음 (iroh 공개 relay 무료)
- 백그라운드 tokio task가 async QUIC ↔ sync 버퍼를 브릿지합니다

### 7. AI 연동 (`ggterm-ai`)

OpenAI 호환 API 클라이언트가 지원하는 기능:
- 명령어 설명 (Ctrl+Shift+E)
- 코드 제안 (Ctrl+Shift+S)
- 도움말 (Ctrl+Shift+H)
- 자연어 → 명령어 (Ctrl+Shift+N)

### 8. Plugin 시스템 (`ggterm-plugin`)

이중 런타임: Lua (`mlua` 경유) 및 WASM. 다음을 통해 설정합니다:

```toml
[plugins]
enabled = false
directory = "~/.ggterm/plugins"
```

Plugin 예시: `examples/plugins/hello.lua`

## 프로토콜 지원

### 터미널 프로토콜

| 프로토콜 | Sequence | 상태 |
|----------|-----------|--------|
| Cursor | CSI A/B/C/D/E/F/G/H/f | Full |
| Erase | ED (J), EL (K), DECSED | Full (selective erase) |
| SGR | 0-9, 21, 53-55, 58-59 | Full (overline, underline color 포함) |
| DECSET | 1, 5, 6, 7, 12, 25, 47, 1000-1006, 1015-1016, 1047-1049, 2004, 2026, 2027 | Full |
| OSC | 0, 2, 4, 7, 8, 9, 10-12, 52, 104, 110-112, 133, 1337, 9;4 | Full |
| DCS | XTGETTCAP, DECRQSS | Full |
| 문자셋 | G0/G1, US/UK/special graphics | Full |
| DECSC/DECRC | ESC 7/8 (전체 상태 저장) | Full |
| DECSTR | CSI ! p (soft reset) | Full |
| Kitty keyboard | CSI > u push/pop, CSI = u | Full |
| DA1/DA2/DA3 | DA sequence | Full |
| DSR | CSI 6n, 5n, ?6n, ?15n, ?16n, ?11t | Full |
| DECRQM | Mode 조회 (ANSI + private) | Full |

### Shell Integration

명령어 감지를 위한 OSC 133 마크:
- `OSC 133 ; A ST` — 프롬프트 시작
- `OSC 133 ; B ST` — 명령어 시작
- `OSC 133 ; C ST` — 출력 시작
- `OSC 133 ; D ; <exit_code> ST` — 명령어 종료

지원 shell: bash, zsh, fish (`shell/` 스크립트를 통한 자동 주입).

## Feature Flag

| Feature | Crate | 설명 |
|---------|-------|------|
| `desktop` | ggterm-app | winit window + wgpu 렌더링 + PTY |
| `ai` | ggterm-app, ggterm-ffi | AI 어시스턴트 연동 |
| `plugin` | ggterm-app | Plugin manager 프레임워크 |
| `plugin-lua` | ggterm-app | Lua plugin 런타임 (`plugin` 포함) |
| `config-watch` | ggterm-app | 파일 시스템 감시를 통한 config hot-reload |
| `ssh` | ggterm-ffi | 모바일용 SSH transport |
| `p2p` | ggterm-app, ggterm-ffi | P2P 터미널 공유 |

**표준 데스크톱 빌드:** `desktop ai plugin plugin-lua config-watch`

## 빌드 시스템

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
