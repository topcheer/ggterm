# GGTerm 개발자 가이드

> 기여자 및 plugin 개발자용

## 개발 환경 설정

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Debug build
make build

# Run tests (2,143 tests)
make test

# Lint (must be zero warnings)
make clippy

# Format check
make fmt
```

## Feature Flag

```bash
# Standard desktop
cargo build --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Without config hot-reload
cargo build --features "desktop" --bin ggterm

# Mobile FFI (no desktop)
cargo build -p ggterm-ffi --features ssh

# P2P support
cargo build --features "desktop ai plugin plugin-lua config-watch p2p" --bin ggterm
```

## 새 터미널 프로토콜 추가

모든 SGR/CSI/OSC handler는 `crates/ggterm-core/src/term/mod.rs`에 있습니다.

1. **CSI handler** — `csi()` 메서드에 추가
2. **OSC handler** — `osc()` 메서드에 추가
3. **ESC handler** — `esc()` 메서드에 추가
4. **테스트** — 같은 파일에 테스트 추가

## 새 테마 추가

테마는 `crates/ggterm-render/src/theme.rs`에 정의됩니다.

```rust
pub fn my_theme() -> Theme {
    Theme {
        background: [20, 20, 30],
        foreground: [200, 200, 210],
        cursor: [255, 255, 255],
        selection_bg: [60, 80, 120],
        palette: DEFAULT_PALETTE,
    }
}
```

`by_name()`, `builtin_names()`, `cycle_next()`에 등록하세요.

## Window 모듈 가이드

```
window/mod.rs       — DesktopApp struct, constructor, ApplicationHandler
window/handlers.rs  — Event handler (keyboard, mouse, resize, IME)
window/actions.rs   — 비즈니스 로직 (tab/split/clipboard/theme/session)
window/render.rs    — 렌더링 (render_frame, multi-pane, overlay)
```

### 키보드 단축키 추가

1. `window/handlers.rs`에 handler 추가
2. `window/actions.rs`에 action 메서드 추가
3. 단축키 도움말에 등록 (`shortcut_help.rs`)

### Borrow Checker 패턴

**문제**: `self.active_session().app().grid()`는 `&self` 전체를 borrow합니다.

**해결책**: 직접 필드 접근:
```rust
let active = self.active;
let grid = &self.sessions[active].app().grid();
```

## 모바일 개발

### iOS Simulator

```bash
# Build Rust static lib (universal: arm64 + x86_64)
~/.cargo/bin/cargo build -p ggterm-ffi --target aarch64-apple-ios-sim --release --features "ssh p2p"
~/.cargo/bin/cargo build -p ggterm-ffi --target x86_64-apple-ios --release --features "ssh p2p"
lipo -create target/aarch64-apple-ios-sim/release/libggterm_ffi.a \
              target/x86_64-apple-ios/release/libggterm_ffi.a \
              -output mobile/ios/RustLib/libggterm_ffi.a

# Build and run Flutter
cd mobile && flutter run --debug
```

### Android

```bash
scripts/release/build-android-ffi.sh
cd mobile && flutter run
```

## 코드 스타일

- **테스트 코드가 아닌 곳에서 `.unwrap()` 사용 금지** — lock에는 `unwrap_or_else(|e| e.into_inner())` 사용
- **모든 commit 전에 `cargo fmt --all` 실행**
- **Clippy는 `-D warnings`로 통과해야 함**
- **Cell은 Copy가 아닌 Clone** — 명시적으로 `.clone()` 사용
- **편집 전 읽기** — 항상 먼저 `read_file` 수행

## 테스트

```bash
# All tests
make test

# Specific crate
cargo test -p ggterm-core --lib
cargo test --features "desktop ai plugin plugin-lua config-watch" -p ggterm-app --lib

# Single test
cargo test --features "desktop" -p ggterm-core --lib -- test_osc52
```

## CI/CD 파이프라인

| 트리거 | Workflow | Action |
|---------|----------|--------|
| Push to main / PR | `ci.yml` | fmt + clippy + test + build |
| Tag `v*` | `release-desktop.yml` | macOS .dmg + Linux .deb + Windows .zip |
| Tag `v*` | `release-mobile.yml` | Android .apk + iOS .ipa |

### 릴리스 생성

```bash
git add -A
git commit -m "release: vX.Y.Z"
git tag vX.Y.Z
git push origin main --tags
```

## 디버깅

```bash
ggterm -v     # info
ggterm -vv    # debug
ggterm -vvv   # trace
```

debug overlay를 보려면 `F1`을, 성능 모니터를 보려면 `Ctrl+Shift+G`를 누르세요.

## FFI 개발

### 새 C-ABI 함수 추가

1. `crates/ggterm-ffi/src/lib.rs` 또는 `transport.rs`에 선언
2. 구현 (lock에는 `unwrap_or_else(|e| e.into_inner())` 사용)
3. `mobile/lib/ffi/ffi_bindings.dart`에 Dart binding 추가
4. `mobile/ios/RustLib/ggterm_ffi.h`의 C header 업데이트

## Plugin 개발

### Lua Plugin

```lua
-- ~/.ggterm/plugins/myplugin.lua
function on_load()
    print("Plugin loaded!")
end
```

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

## 자주 발생하는 문제

- **"module not found"**: `lib.rs`에 module이 선언되어 있는지 확인
- **Clippy let chain**: `&&`만 지원, `||`는 사용 불가
- **폰트 렌더링**: Menlo Bold에는 box-drawing 문자가 없음; 항상 Weight::NORMAL 사용

## 주요 파일 위치

| 항목 | 위치 |
|------|-------|
| Terminal protocol | `crates/ggterm-core/src/term/mod.rs` |
| VTE parser | `crates/ggterm-core/src/vte/parser.rs` |
| Grid model | `crates/ggterm-core/src/grid/mod.rs` |
| 테마 | `crates/ggterm-render/src/theme.rs` |
| GPU pipeline | `crates/ggterm-render-wgpu/src/lib.rs` |
| DesktopApp | `crates/ggterm-app/src/window/mod.rs` |
| Event handler | `crates/ggterm-app/src/window/handlers.rs` |
| Config 시스템 | `crates/ggterm-app/src/config.rs` |
| FFI 함수 | `crates/ggterm-ffi/src/lib.rs` |
| CLI 진입점 | `crates/ggterm-app/src/bin/ggterm.rs` |

## 기여하기

1. 저장소를 Fork합니다
2. Feature branch를 생성합니다
3. `make fmt && make clippy && make test`를 실행합니다
4. 규칙적인 메시지로 commit합니다 (`feat:`, `fix:`, `docs:`)
5. Pull Request를 생성합니다

### Pull Request 체크리스트

- [ ] `cargo fmt --all -- --check` 통과
- [ ] `cargo clippy --features "..." --workspace -- -D warnings` 통과
- [ ] `cargo test --features "..." --workspace` 통과
- [ ] 테스트 코드가 아닌 곳에 새로운 `.unwrap()` 없음
