# Arquitectura de GGTerm

> **Versión:** Phase 55+ | **LOC:** 71,791 Rust | **Tests:** 2,143 | **Crates:** 9

## Resumen general

GGTerm es un emulador de terminal acelerado por GPU, nativo de IA y multiplataforma, escrito en Rust. Está orientado a plataformas de escritorio (macOS/Linux/Windows) y móviles (iOS/Android).

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

## Desglose de crates

| Crate | LOC | Responsabilidad |
|-------|-----|-----------------|
| `ggterm-core` | 13,505 | VTE parser, grid model, terminal state machine, PTY transport trait |
| `ggterm-app` | 43,412 | Aplicación de escritorio: ventana winit, event loop, pestañas, splits, config, handlers |
| `ggterm-render-wgpu` | 2,979 | GPU renderer con wgpu: texto, decoraciones, SDF UI, viewport multi-pane |
| `ggterm-render` | 1,739 | Render trait, definiciones de temas (9 temas), cursor state |
| `ggterm-ffi` | 2,597 | C-ABI para Flutter: ciclo de vida de sesión, procesamiento de bytes, lectura de celdas |
| `ggterm-ai` | 2,250 | Puente del motor de IA (cliente API compatible con OpenAI) |
| `ggterm-plugin` | 3,925 | Gestor de plugins (runtimes Lua + WASM) |
| `ggterm-ssh` | 644 | Transporte SSH mediante russh 0.61 (puente async→sync) |
| `ggterm-p2p` | 740 | Compartir terminal P2P mediante iroh (QUIC + NAT traversal) |

## Arquitectura central

### 1. Motor de terminal (`ggterm-core`)

El motor de terminal implementa una máquina de estados de terminal completa compatible con VT100/VT220/xterm.

#### VTE Parser

Basado en la máquina de estados de Paul Williams (`vte/parser.rs`). Soporta:
- CSI (Control Sequence Introducer) — todos los modos estándar y privados
- OSC (Operating System Command) — 0, 2, 4, 7, 8, 9, 10-12, 52, 133, 1337, 9;4
- DCS (Device Control String) — XTGETTCAP, DECRQSS
- Secuencias ESC — DECSC/DECRC, DECPAM/DECPNM, RIS, IND, NEL
- SOS/PM/APC — consumidos (sin impresión de contenido)
- Protección contra desbordamiento del buffer de strings (OSC 64KB, DCS 1MB)

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

#### Estado de la terminal

El struct `Terminal` posee:
- Posición del cursor (fila, columna) + flag pending_wrap
- Atributos SGR (fg actual, bg, flags)
- Conjuntos de caracteres (G0, G1, conjunto activo)
- Modos (modos privados DEC + modos ANSI)
- Región de scroll (DECSTBM)
- Colores dinámicos (overrides OSC 10/11/12)
- Overrides de paleta (OSC 4, compatible con base16-shell)
- Buffer de respuestas (respuestas DA1, DSR, DECRQM)
- Marcas de comandos (OSC 133, shell integration)
- Intercambio de grid alt-screen (DECSET 47/1047/1049)
- Estado guardado DECSC (completo: cursor + SGR + charset + autowrap)

#### Trait PTY Transport

```rust
pub trait TerminalTransport {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    fn is_alive(&self) -> bool;
}
```

Implementaciones: `PtySession` (PTY local), `SshSession` (russh), `P2pTransport` (iroh QUIC).

### 2. Aplicación de escritorio (`ggterm-app`)

#### Estructura del módulo window

```
window/
├── mod.rs       — DesktopApp struct, constructor, ApplicationHandler, event loop
├── handlers.rs  — Keyboard, mouse, cursor, resize event handlers
├── actions.rs   — Tab/split/clipboard/theme/font/session/drag-drop operations
└── render.rs    — render_frame(), multi-pane rendering, overlay composition
```

#### Jerarquía de sesiones

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

#### Pipeline de renderizado

1. **Grid → TextRuns** (`converter.rs`): Cada fila del grid se convierte en entradas `TextRun` con resolución de colores del tema. Los caracteres anchos (CJK/emoji) fuerzan divisiones de run.

2. **TextRun → glyphon TextAreas** (`lib.rs`): Cada run se convierte en un `glyphon::Buffer` independiente posicionado en `(start_col × cell_width, row × cell_height)`.

3. **Decoraciones** (`lib.rs`): Subrayados, tachados y overlines generados como datos de vértices en `prepare_decorations()`.

4. **SDF UI** (`ui.wgsl`): Rectángulos redondeados para tab bar, status bar, menús y diálogos mediante shader de campos de distancia con signo (signed distance field).

5. **Renderizado multi-pane** (`gpu.rs`):
   - Un render pass para todos los panes
   - Cada pane: `set_scissor_rect()` → `set_viewport_offset()` → `render_pane_to_pass()`
   - Pass de overlay final: `render_overlays_to_pass()` (bordes, tab bar, status bar, menús)

#### Gestión de fuentes

- **macOS**: Menlo (solo Weight::NORMAL — la variante Bold carece de glifos box-drawing)
- **Linux**: DejaVu Sans Mono
- **Windows**: Cascadia Mono
- El estilo Bold se distingue por color brillante, no por peso (estándar xterm/Alacritty)
- `Shaping::Advanced` habilita el fallback de fuentes CJK
- El ancho de celda = avance flotante exacto de 'M' (sin redondeo)

### 4. Configuración (`config.rs`)

Formato TOML en `~/.ggterm/config.toml`:

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

Hot-reload: el feature `config-watch` utiliza el crate `notify` para vigilar el archivo de configuración. Los cambios detectados en `about_to_wait()` se aplican inmediatamente (cambio de tema, redimensionado de fuente, actualización de scrollback).

### 5. Mobile FFI (`ggterm-ffi`)

Funciones C-ABI para la integración con Flutter:

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

Registro global de sesiones mediante `OnceLock<Mutex<HashMap<u32, MobileSession>>>`. Los locks de Mutex utilizan `unwrap_or_else(|e| e.into_inner())` para garantizar seguridad frente a panics.

### 6. Compartir terminal P2P (`ggterm-p2p`)

Utiliza iroh 1.0 (QUIC + NAT traversal) para compartir terminal directamente entre móvil y escritorio.

```
Desktop (Host)                    Mobile (Client)
┌──────────────┐                  ┌──────────────┐
│ P2pHost      │ ◄── QUIC ──►    │ P2pClient     │
│  Endpoint    │   (P2P/relay)    │  connect()    │
│  PTY ↔ Stream│                  │  Stream ↔ Term│
└──────────────┘                  └──────────────┘
```

- Formato del ticket: base32(postcard(EndpointAddr)) — ~130 caracteres, cabe en un código QR
- Tasa de éxito de conexión P2P directa: 90%+ (con fallback al relay de iroh)
- Coste operativo cero (el relay público de iroh es gratuito)
- Un task en segundo plano de tokio puentea entre QUIC asíncrono y buffers síncronos

### 7. Integración de IA (`ggterm-ai`)

Cliente de API compatible con OpenAI que soporta:
- Explicación de comandos (Ctrl+Shift+E)
- Sugerencias de código (Ctrl+Shift+S)
- Ayuda (Ctrl+Shift+H)
- Lenguaje natural → comando (Ctrl+Shift+N)

### 8. Sistema de plugins (`ggterm-plugin`)

Runtime dual: Lua (mediante `mlua`) y WASM. Se configura mediante:

```toml
[plugins]
enabled = false
directory = "~/.ggterm/plugins"
```

Plugin de ejemplo: `examples/plugins/hello.lua`

## Soporte de protocolos

### Protocolos de terminal

| Protocolo | Secuencias | Estado |
|-----------|-----------|--------|
| Cursor | CSI A/B/C/D/E/F/G/H/f | Completo |
| Erase | ED (J), EL (K), DECSED | Completo (borrado selectivo) |
| SGR | 0-9, 21, 53-55, 58-59 | Completo (incluyendo overline, underline color) |
| DECSET | 1, 5, 6, 7, 12, 25, 47, 1000-1006, 1015-1016, 1047-1049, 2004, 2026, 2027 | Completo |
| OSC | 0, 2, 4, 7, 8, 9, 10-12, 52, 104, 110-112, 133, 1337, 9;4 | Completo |
| DCS | XTGETTCAP, DECRQSS | Completo |
| Character sets | G0/G1, US/UK/special graphics | Completo |
| DECSC/DECRC | ESC 7/8 (guardado de estado completo) | Completo |
| DECSTR | CSI ! p (soft reset) | Completo |
| Kitty keyboard | CSI > u push/pop, CSI = u | Completo |
| DA1/DA2/DA3 | DA sequences | Completo |
| DSR | CSI 6n, 5n, ?6n, ?15n, ?16n, ?11t | Completo |
| DECRQM | Mode query (ANSI + private) | Completo |

### Shell Integration

Marcas OSC 133 para detección de comandos:
- `OSC 133 ; A ST` — inicio del prompt
- `OSC 133 ; B ST` — inicio del comando
- `OSC 133 ; C ST` — inicio de la salida
- `OSC 133 ; D ; <exit_code> ST` — fin del comando

Shells soportados: bash, zsh, fish (inyectados automáticamente mediante scripts de `shell/`).

## Feature Flags

| Feature | Crate | Descripción |
|---------|-------|-------------|
| `desktop` | ggterm-app | winit window + wgpu rendering + PTY |
| `ai` | ggterm-app, ggterm-ffi | Integración del asistente de IA |
| `plugin` | ggterm-app | Framework del gestor de plugins |
| `plugin-lua` | ggterm-app | Runtime de plugins Lua (implica `plugin`) |
| `config-watch` | ggterm-app | Hot-reload de config mediante monitorización del sistema de archivos |
| `ssh` | ggterm-ffi | Transporte SSH para móvil |
| `p2p` | ggterm-app, ggterm-ffi | Compartir terminal P2P |

**Build estándar de escritorio:** `desktop ai plugin plugin-lua config-watch`

## Sistema de build

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
