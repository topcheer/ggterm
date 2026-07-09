# 第四部分：主题、字体与外观

## 主题

### 9 个内置主题 + Auto

| 主题 | 背景 | 风格 |
|-------|-----------|-------|
| `dark` | 深灰色 | 默认暗色主题 |
| `light` | 白色 | 明亮环境 |
| `dracula` | 深紫色 | 流行暗色主题 |
| `solarized-dark` | 深蓝色 | 开发者专注 |
| `solarized-light` | 暖奶油色 | 阅读 |
| `gruvbox` | 泥土暗色 | 复古暖色 |
| `nord` | 北极蓝 | 简洁极简 |
| `tokyo-night` | 深海军蓝 | 夜间编码 |
| `catppuccin-mocha` | 柔棕色 | 温暖暗色 |
| `auto` | 跟随系统 | 无缝切换 |

### 主题控制

| 快捷键 | 功能 |
|----------|--------|
| `Ctrl+Shift+Alt+T` | 循环切换主题 |
| `Ctrl+Shift+T` | 循环切换主题（替代方式） |

### Auto 主题

当设置 `theme = "auto"` 时，GGTerm 检测系统外观：
- **macOS**：AppleInterfaceStyle
- **Linux**：GTK_THEME / gsettings
- **Windows**：注册表检查

### 动态颜色（OSC 10/11/12）

程序可在运行时覆盖主题颜色：
- `OSC 10` — 设置/查询前景色
- `OSC 11` — 设置/查询背景色
- `OSC 12` — 设置/查询光标颜色
- `OSC 104/110/111/112` — 重置为默认值

### 自定义调色板（OSC 4）

base16-shell、wal、pywal 等程序可以设置自定义 16 色调色板：
- `OSC 4 ; N ; rgb:RR/GG/BB` — 设置调色板颜色 N
- `OSC 104 ; N` — 重置调色板颜色 N
- 渲染器将覆盖应用于索引颜色

## 字体

### 字体控制

| 快捷键 | 功能 |
|----------|--------|
| `Ctrl+=` | 放大（字体大小 +1.5px） |
| `Ctrl+-` | 缩小（字体大小 -1.5px） |
| `Ctrl+0` | 重置为默认字体大小 |
| `Ctrl+Shift+滚轮` | 用鼠标滚轮缩放字体 |

### 平台默认字体

| 平台 | 字体 |
|----------|------|
| macOS | Menlo（仅 Regular — Bold 变体缺少制表符字形） |
| Linux | DejaVu Sans Mono |
| Windows | Cascadia Mono |

**粗体文本**：通过亮色区分，而非字体粗细（xterm/Alacritty 标准）。

**CJK 回退**：`Shaping::Advanced` 启用 CJK 字符的自动字体回退。

### 单元格尺寸

- 单元格宽度 = 'M' 的精确浮点字宽（无四舍五入）
- 单元格高度 = 字体大小（像素）
- 通过 CSI 14t/15t/16t 报告真实像素尺寸

## 背景透明度

| 快捷键 | 功能 |
|----------|--------|
| `Ctrl+Shift+Alt+]` | 增加不透明度（+5%） |
| `Ctrl+Shift+Alt+[` | 降低不透明度（-5%） |

不透明度范围：0.0（完全透明）到 1.0（完全不透明）。Toast 通知显示百分比。

配置：`[appearance] background_opacity = 0.85`

## 窗口控制

| 快捷键 | 功能 |
|----------|--------|
| `F11` | 切换全屏 |
| `Ctrl+Shift+Enter` | 切换最大化 |
| `Ctrl+Shift+Alt+A` | 切换窗口置顶 |
| `Ctrl+Shift+B` | 切换状态栏 |

### 透明标题栏（macOS）

在 macOS 上，标题栏设为透明以实现无缝外观。

## 光标

### 光标样式

配置：`[appearance] cursor_style = "block"`

选项：`block`、`underline`、`bar`

程序可通过 DECSCUSR（CSI N q）更改光标样式。

### 光标闪烁

配置：`[appearance] cursor_blink = true`

- 闪烁使用正弦波 alpha 实现平滑淡入淡出
- 用户输入时重置闪烁
- 闪烁相位与 SGR 5 闪烁文本渲染共享

### 光标行高亮

配置：`[appearance] cursor_line_highlight = false`

高亮光标所在的整行（类似 Vim 的 `cursorline`）。

### 光标特效

通过命令面板：
- **cursor.trail** — 光标留下粒子拖尾
- **cursor.glow** — 光标发光效果
- **cursor.none** — 禁用光标特效

## 状态栏

切换：`Ctrl+Shift+B`

状态栏显示：
- 光标位置（行:列）
- 标签页数量
- 当前目录（来自 OSC 7）
- 远程主机（来自 OSC 1337 的 SSH 指示器）
- 运行中的命令 + 计时器
- 进度百分比（来自 OSC 9;4）
- 广播模式指示器
- 录制指示器
- 窗格缩放指示器
- Bell 指示器
- 声音开关指示器
- 选区字数统计
- 配置错误指示器

## 配置文件

配置文件允许在不同外观配置间切换：

```toml
[profiles.develop]
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

| 快捷键 | 功能 |
|----------|--------|
| `Ctrl+Shift+Alt+F` | 循环切换配置文件 |
| `Ctrl+Shift+Alt+P` | 循环切换配置文件（替代方式） |

## 设置面板

| 快捷键 | 功能 |
|----------|--------|
| `Ctrl+,` | 打开设置面板 |

使用方向键导航设置，内联编辑值。

## 调试覆盖层

| 快捷键 | 功能 |
|----------|--------|
| `F1` | 切换调试覆盖层（FPS、单元格计数、窗格信息） |
| `Ctrl+Shift+G` | 切换性能监视器 |

## 逐窗格渲染

每个窗格维护独立的渲染器状态：
- 反显模式（DECSCNM）
- 动态前景/背景色（OSC 10/11）
- 下划线颜色（SGR 58）
- 闪烁文本相位（SGR 5）
