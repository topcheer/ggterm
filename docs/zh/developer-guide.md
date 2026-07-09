# GGTerm 开发者指南

> 面向贡献者和插件开发者

## 开发环境搭建

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Debug 构建
make build

# 运行测试 (2,143 个测试)
make test

# 代码检查 (必须零警告)
make clippy

# 格式检查
make fmt
```

## 功能标志

```bash
# 标准桌面
cargo build --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# 不带配置热重载
cargo build --features "desktop" --bin ggterm

# 移动端 FFI (不含桌面)
cargo build -p ggterm-ffi --features ssh

# P2P 支持
cargo build --features "desktop ai plugin plugin-lua config-watch p2p" --bin ggterm
```

## 添加新的终端协议

所有 SGR/CSI/OSC 处理器在 `crates/ggterm-core/src/term/mod.rs`。

1. **CSI 处理器** — 添加到 `csi()` 方法：
   ```rust
   b'X' => {
       // 你的处理器
   }
   b'X' if is_private => {
       // 私有模式 (? 前缀)
   }
   ```

2. **OSC 处理器** — 添加到 `osc()` 方法

3. **ESC 处理器** — 添加到 `esc()` 方法

4. **测试** — 在同一文件中添加测试

## 添加新主题

主题定义在 `crates/ggterm-render/src/theme.rs`。

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

注册到 `by_name()`、`builtin_names()`、`cycle_next()`。

## 窗口模块指南

```
window/mod.rs       — DesktopApp 结构体、构造函数、ApplicationHandler
window/handlers.rs  — 事件处理器 (键盘、鼠标、调整大小、IME)
window/actions.rs   — 业务逻辑 (标签/分屏/剪贴板/主题/会话)
window/render.rs    — 渲染 (render_frame、多面板、叠加层)
```

### 添加键盘快捷键

1. 在 `window/handlers.rs` 添加处理器
2. 在 `window/actions.rs` 添加操作方法
3. 在 `shortcut_help.rs` 注册到快捷键帮助

### 借用检查器模式

**问题**：`self.active_session().app().grid()` 借用了整个 `&self`。

**解决方案**：直接字段访问：
```rust
let active = self.active;
let grid = &self.sessions[active].app().grid();
```

## 移动端开发

### iOS 模拟器

```bash
# 构建 Rust 静态库 (通用: arm64 + x86_64)
~/.cargo/bin/cargo build -p ggterm-ffi --target aarch64-apple-ios-sim --release --features "ssh p2p"
~/.cargo/bin/cargo build -p ggterm-ffi --target x86_64-apple-ios --release --features "ssh p2p"
lipo -create target/aarch64-apple-ios-sim/release/libggterm_ffi.a \
              target/x86_64-apple-ios/release/libggterm_ffi.a \
              -output mobile/ios/RustLib/libggterm_ffi.a

# 构建并运行 Flutter
cd mobile
flutter run --debug
```

### Android

```bash
scripts/release/build-android-ffi.sh
cd mobile && flutter run
```

## 提交规范

```
feat: 添加新功能
fix: 修复 bug
refactor: 代码重构
docs: 文档更新
```

始终追加：`Co-Authored-By: ggcode <noreply@ggcode.dev>`

## 代码风格

- **非测试代码中不使用 `.unwrap()`** — 使用 `unwrap_or_else(|e| e.into_inner())`
- **每次提交前运行 `cargo fmt --all`**
- **Clippy 必须通过 `-D warnings`**
- **Cell 是 Clone 不是 Copy** — 显式使用 `.clone()`
- **编辑前必须先读取文件**

## 测试

```bash
# 所有测试
make test

# 特定 crate
cargo test -p ggterm-core --lib
cargo test --features "desktop ai plugin plugin-lua config-watch" -p ggterm-app --lib

# 单个测试
cargo test --features "desktop" -p ggterm-core --lib -- test_osc52
```

## CI/CD 流水线

| 触发条件 | 工作流 | 操作 |
|---------|--------|------|
| 推送到 main / PR | `ci.yml` | 格式检查 + clippy + 测试 + 构建 |
| Tag `v*` | `release-desktop.yml` | macOS .dmg + Linux .deb + Windows .zip |
| Tag `v*` | `release-mobile.yml` | Android .apk + iOS .ipa |

### 创建发布

```bash
git add -A
git commit -m "release: vX.Y.Z"
git tag vX.Y.Z
git push origin main --tags
```

## 调试

### 启用日志

```bash
ggterm -v     # info
ggterm -vv    # debug
ggterm -vvv   # trace
```

### 调试叠层

按 `F1` 切换屏幕调试叠层 (FPS、单元格计数、面板信息)。

### 性能监视器

按 `Ctrl+Shift+G` 切换性能监视器叠层。

## FFI 开发

### 添加新的 C-ABI 函数

1. 在 `crates/ggterm-ffi/src/lib.rs` 或 `transport.rs` 中声明
2. 实现（锁使用 `unwrap_or_else(|e| e.into_inner())`）
3. 在 `mobile/lib/ffi/ffi_bindings.dart` 添加 Dart 绑定
4. 更新 `mobile/ios/RustLib/ggterm_ffi.h` C 头文件

## 插件开发

### Lua 插件

```lua
-- ~/.ggterm/plugins/myplugin.lua
function on_load()
    print("插件已加载！")
end
```

### 插件配置

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

## 常见问题

### "module not found" 错误

确保模块在 `lib.rs` 中声明：`pub mod my_module;`

### Clippy 在 let 链上报错

Rust 2024 稳定版中 `if let` 链仅支持 `&&`，不支持 `||`。

### 字体渲染问题

- Menlo Bold 缺少制表符 → 始终使用 Weight::NORMAL
- 单元格宽度 = 'M' 的精确浮点前进值
- CJK 回退需要 `Shaping::Advanced`

## 常用文件位置

| 内容 | 位置 |
|------|------|
| 终端协议 | `crates/ggterm-core/src/term/mod.rs` |
| VTE 解析器 | `crates/ggterm-core/src/vte/parser.rs` |
| 网格模型 | `crates/ggterm-core/src/grid/mod.rs` |
| Cell 结构体 | `crates/ggterm-core/src/grid/cell.rs` |
| 主题 | `crates/ggterm-render/src/theme.rs` |
| GPU 管线 | `crates/ggterm-render-wgpu/src/lib.rs` |
| 文本转换器 | `crates/ggterm-render-wgpu/src/converter.rs` |
| DesktopApp | `crates/ggterm-app/src/window/mod.rs` |
| 事件处理器 | `crates/ggterm-app/src/window/handlers.rs` |
| 操作逻辑 | `crates/ggterm-app/src/window/actions.rs` |
| 渲染 | `crates/ggterm-app/src/window/render.rs` |
| 配置系统 | `crates/ggterm-app/src/config.rs` |
| 快捷键帮助 | `crates/ggterm-app/src/shortcut_help.rs` |
| FFI 函数 | `crates/ggterm-ffi/src/lib.rs`, `transport.rs` |
| SSH 传输 | `crates/ggterm-ssh/src/lib.rs` |
| P2P 传输 | `crates/ggterm-p2p/src/transport.rs` |
| Shell 集成 | `shell/integration.{bash,zsh,fish}` |
| 配置示例 | `config.example.toml` |
| CLI 入口 | `crates/ggterm-app/src/bin/ggterm.rs` |
| CI/CD 工作流 | `.github/workflows/` |
| 发布脚本 | `scripts/release/` |
