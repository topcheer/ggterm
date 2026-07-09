# GGTerm 用户使用手册

> **版本:** Phase 55+ | **平台:** macOS, Linux, Windows, iOS, Android

## 安装

### 从源码构建

```bash
# 前提条件: Rust stable, clang, pkg-config

# 克隆仓库
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# 构建并运行
cargo run --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Release 构建
cargo build --release --features "desktop ai plugin plugin-lua config-watch" --bin ggterm
# 二进制文件: target/release/ggterm
```

### 预编译版本

从 [GitHub Releases](https://github.com/topcheer/ggterm/releases) 下载：
- **macOS**: 通用 .dmg (Apple Silicon + Intel)
- **Linux**: .deb 包或压缩包
- **Windows**: .zip

## 命令行用法

```bash
ggterm [选项]

选项:
  -c, --cols <N>            初始列数 (默认: 80)
  -r, --rows <N>            初始行数 (默认: 24)
  -s, --shell <路径>        Shell 路径 (默认: $SHELL)
  -t, --title <标题>        窗口标题 (默认: "GGTerm")
      --theme <名称>        颜色主题 (默认: "dark")
      --font-size <像素>    字体大小 (默认: 16)
      --cell-width <像素>   单元格宽度 (默认: 8)
  -w, --working-directory <目录>  在此目录启动 shell
  -C, --config <路径>       自定义配置文件
  -e, --execute <命令...>   执行命令而非交互式 shell
      --hold                命令退出后保持终端打开
      --fullscreen          全屏启动
      --maximize            最大化启动
  -v                        详细日志 (-v info, -vv debug, -vvv trace)
```

### 示例

```bash
# 默认终端
ggterm

# 大尺寸终端 + zsh
ggterm --cols 120 --rows 40 --shell /bin/zsh

# Dracula 主题, 字号 18
ggterm --theme dracula --font-size 18

# 运行 vim 并在退出后保持
ggterm -e vim --hold

# 全屏启动
ggterm --fullscreen
```

## 配置

配置文件：`~/.ggterm/config.toml`（所有选项见 `config.example.toml`）。

### 外观

```toml
[appearance]
theme = "dark"                # dark, light, dracula, solarized-dark,
                              # solarized-light, gruvbox, nord,
                              # tokyo-night, catppuccin-mocha, auto
font_family = "monospace"
font_size = 14
cursor_style = "block"         # block | underline | bar
background_opacity = 1.0       # 0.0 透明到 1.0 不透明
cursor_blink = true            # 光标闪烁开关
```

### 终端

```toml
[terminal]
scrollback_lines = 10000
shell = ""                     # 空 = $SHELL 或 /bin/sh
restore_session = false        # 启动时恢复标签/分屏
```

### AI

```toml
[ai]
enabled = true
api_endpoint = "https://api.openai.com/v1"
model = "gpt-4"
```

### 配置文件

```toml
[profiles.develop]
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

使用 `Ctrl+Shift+Alt+P` 循环切换配置文件。

### 键盘快捷键

在 `[keybindings]` 段自定义键盘快捷键。所有选项见 `config.example.toml`。

## 主题

9 种内置主题 + 自动模式：

| 主题 | 背景色 | 适用场景 |
|------|--------|--------|
| `dark` | 深灰 | 通用 |
| `light` | 白色 | 白天 / 明亮环境 |
| `dracula` | 深紫 | 流行暗色主题 |
| `solarized-dark` | 深蓝 | 开发专注 |
| `solarized-light` | 暖白 | 阅读 |
| `gruvbox` | 复古暗色 | 温暖复古 |
| `nord` | 极地蓝 | 简约清爽 |
| `tokyo-night` | 深海军蓝 | 夜间编码 |
| `catppuccin-mocha` | 柔棕 | 温暖暗色 |
| `auto` | 跟随系统 | 无缝切换 |

循环切换主题：`Ctrl+Shift+T`

## 键盘快捷键

### 标签页

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+T` | 新建标签 |
| `Ctrl+W` | 关闭标签 |
| `Alt+1-9` | 切换到第 N 个标签 |
| `Ctrl+Tab` | 下一个标签 |
| `Ctrl+Shift+Tab` | 上一个标签 |
| `Ctrl+Shift+PageUp` | 标签左移 |
| `Ctrl+Shift+PageDown` | 标签右移 |
| `Ctrl+Shift+Alt+D` | 复制标签 |
| `Ctrl+Shift+Alt+W` | 关闭其他标签 |

### 分屏

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+Shift+D` | 水平分屏 (左右) |
| `Ctrl+Shift+\` | 垂直分屏 (上下) |
| `Ctrl+Shift+[` | 上一个面板 |
| `Ctrl+Shift+]` | 下一个面板 |
| `Alt+H/J/K/L` | Vim 风格面板导航 |
| `Ctrl+Shift+X` | 交换面板内容 |
| `Ctrl+Shift+Z` | 切换面板缩放 |
| `Ctrl+Shift+Alt+方向键` | 调整分屏比例 |
| `Ctrl+Shift+Alt+N` | 重置为单面板 |

### 字体与主题

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+=` | 放大 (字号 +1.5px) |
| `Ctrl+-` | 缩小 (字号 -1.5px) |
| `Ctrl+0` | 重置字号 |
| `Ctrl+Shift+滚轮` | 鼠标滚轮缩放字体 |
| `Ctrl+Shift+T` | 循环切换主题 |
| `Ctrl+Shift+Alt+P` | 循环切换配置文件 |

### 终端操作

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+Shift+C` | 复制选中文本 |
| `Ctrl+Shift+V` | 从剪贴板粘贴 |
| `Shift+Insert` | 粘贴 (跨平台) |
| `Ctrl+Shift+K` | 清屏 + 清除回滚 |
| `Ctrl+Shift+R` | 重置终端 (RIS) |
| `Ctrl+Shift+A` | 全选文本 |
| `Ctrl+Shift+U` | 打开光标处 URL |
| `Ctrl+Shift+Alt+S` | 导出回滚到文件 |
| `Ctrl+Shift+Alt+Up` | 滚动到标记处 |
| `Ctrl+Shift+End` | 滚动到底部 |

### 搜索

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+Shift+F` | 切换搜索栏 |
| `Enter` | 下一个匹配 |
| `Shift+Enter` | 上一个匹配 |
| `Tab` (搜索中) | 切换大小写敏感 |
| `Up/Down` (搜索中) | 搜索历史导航 |

### AI 助手

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+Shift+E` | 解释当前输出 |
| `Ctrl+Shift+S` | 建议命令 |
| `Ctrl+Shift+H` | 帮助 |
| `Ctrl+Shift+N` | 自然语言转命令 |
| `Esc` | 关闭 AI 叠层 |

### 窗口与显示

| 快捷键 | 功能 |
|--------|------|
| `F11` | 切换全屏 |
| `Ctrl+Shift+Enter` | 切换最大化 |
| `Ctrl+Shift+B` | 切换状态栏 |
| `F1` | 切换调试叠层 |
| `Ctrl+Shift+G` | 切换性能监视器 |

### 高级功能

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+Shift+P` | 命令面板 |
| `Ctrl+Shift+/` | 快捷键帮助叠层 |
| `Ctrl+Shift+L` | Shell 切换器 |
| `Ctrl+Shift+Y` | 命令历史侧栏 |
| `Ctrl+Shift+M` | 切换声音 (响铃) |
| `Ctrl+Shift+Alt+B` | 循环广播模式 |
| `Ctrl+Shift+Alt+P` | 复制当前工作目录 |
| `Ctrl+Shift+Alt+E` | 导出配置到剪贴板 |
| `Ctrl+Shift+Alt+[` | 降低不透明度 |
| `Ctrl+Shift+Alt+]` | 增加不透明度 |
| `Ctrl+Shift+Alt+Q` | 切换 P2P 共享 (QR 码) |

### 鼠标

| 操作 | 结果 |
|------|------|
| 点击+拖拽 | 文本选择 |
| Alt+点击+拖拽 | 块 (矩形) 选择 |
| 双击 | 选择单词 |
| 三击 | 选择整行 |
| 中键点击 | 粘贴选中文本 |
| Cmd/Ctrl+点击 | 打开 URL/超链接 |
| 滚轮 | 滚动回滚历史 |
| Shift+滚动 | 同步滚动所有面板 |
| Ctrl+Shift+滚动 | 缩放字体 |

## P2P 终端共享

通过 QR 码将桌面终端共享给移动设备。

### 桌面端 (主机)

1. 按 `Ctrl+Shift+Alt+Q` 打开共享叠层
2. 显示 QR 码和连接票据
3. 用移动 App 扫描 QR 码（或手动复制票据字符串）
4. 连接成功后，移动设备镜像显示你的终端
5. 按 `Esc` 或 `Ctrl+Shift+Alt+Q` 关闭共享

### 移动端 (客户端)

1. 在连接界面点击 **Scan QR**
2. 将摄像头对准桌面端 QR 码
3. 终端输出显示在移动端
4. 在移动端键盘输入发送命令

## 移动端 App

### 连接选项

| 选项 | 说明 |
|------|------|
| SSH | 通过 SSH 连接远程服务器 (主机、端口、用户名、密码) |
| Echo Test | 诊断模式 — 回显输入字符 (无需服务器) |
| Scan QR | 通过 QR 码 P2P 连接桌面终端 |
| Share Terminal | P2P 主机模式 (仅 Android — 需要本地 shell) |

### iOS 与 Android 的区别

- **iOS**: 仅 SSH + P2P 客户端 (Scan QR) — 无本地终端
- **Android**: 全部功能，包括本地 shell + P2P 主机

## Shell 集成

GGTerm 自动为 bash、zsh、fish 注入 OSC 133 标记，用于命令检测。

手动启用，将以下内容添加到 shell 配置：

```bash
# bash (~/.bashrc)
source /path/to/ggterm/shell/bash.sh

# zsh (~/.zshrc)
source /path/to/ggterm/shell/zsh.zsh

# fish (~/.config/fish/config.fish)
source /path/to/ggterm/shell/fish.fish
```

## 插件系统

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

Lua 插件示例：
```lua
-- ~/.ggterm/plugins/hello.lua
print("Hello from GGTerm plugin!")
```

## 故障排除

### 字体问题

**制表符显示为方块 (豆腐块)：**
- macOS 上 GGTerm 使用 Menlo Regular (非 Bold)，因为 Menlo Bold 缺少制表符字形。
- 粗体通过亮色显示，不使用字重。

**CJK 字符不渲染：**
- 确保启用了 `Shaping::Advanced` (默认)。
- 在系统上安装 CJK 字体。

### 终端模式异常

如果 GGTerm 崩溃后 shell 行为异常：
```bash
reset   # 或: stty sane
```

GGTerm 在正常退出时会发送重置序列 (关闭括号粘贴、关闭鼠标跟踪、光标键恢复正常、软重置)。

### 空闲时 CPU 占用高

GGTerm 在不需要重绘时休眠 50ms。如果 CPU 占用高：
- 检查是否有后台进程产生终端输出
- 禁用光标闪烁：在配置中设置 `cursor_blink = false`
- 检查 `config-watch` 是否触发了过多重载

### 会话未恢复

在 config.toml 中设置 `restore_session = true`。会话在标签/面板关闭和应用退出时自动保存。
