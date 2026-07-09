# 第八部分：配置、插件与故障排除

## 配置文件

位置：`~/.ggterm/config.toml`

### 所有配置选项

```toml
[appearance]
theme = "dark"                  # 参见下方主题列表
font_family = "monospace"        # 字体族名称
font_size = 14                   # 字体大小，单位像素
cell_width = 8                   # 单元格宽度，单位像素
cell_height = 16                 # 单元格高度，单位像素
cursor_style = "block"           # block | underline | bar
cursor_blink = true              # 光标闪烁开关
background_opacity = 1.0         # 0.0 透明 到 1.0 不透明
padding = 8                      # 内容内边距，单位像素
cursor_line_highlight = false    # 高亮光标行（Vim 风格）
word_chars = ""                  # 选区额外的单词字符

[terminal]
scrollback_lines = 10000         # 最大滚动历史行数
shell = ""                       # 空值 = $SHELL 或 /bin/sh
restore_session = false           # 启动时恢复标签页/分屏

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# 自定义键盘快捷键（见下方）
new_tab = "Ctrl+T"
close_tab = "Ctrl+W"
new_split_horizontal = "Ctrl+Shift+D"
new_split_vertical = "Ctrl+Shift+\"
focus_next_pane = "Ctrl+Shift+]"
focus_prev_pane = "Ctrl+Shift+["
copy = "Ctrl+Shift+C"
paste = "Ctrl+Shift+V"
search = "Ctrl+Shift+F"
toggle_fullscreen = "F11"
zoom_in = "Ctrl+="
zoom_out = "Ctrl+-"
zoom_reset = "Ctrl+0"
reset_terminal = "Ctrl+Shift+R"
clear_screen = "Ctrl+Shift+K"
select_all = "Ctrl+Shift+A"
cycle_theme = "Ctrl+Shift+T"
open_url = "Ctrl+Shift+U"
command_palette = "Ctrl+Shift+P"
copy_cwd = "Ctrl+Shift+Alt+P"

[profiles.develop]
# 每个配置文件的可选覆盖项
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

### 配置管理快捷键

| 快捷键 | 功能 |
|----------|--------|
| `Ctrl+Shift+,`（Cmd+,） | 在编辑器中打开配置文件 |
| `Ctrl+Shift+O` | 打开配置文件（替代方式） |
| `Ctrl+Shift+J` | 编辑 shell 配置（.bashrc/.zshrc） |
| `Ctrl+Shift+Alt+E` | 导出配置到剪贴板（TOML） |
| `Ctrl+Shift+Alt+I` | 从剪贴板导入配置 |
| `Ctrl+Shift+Alt+R` | 重置配置为默认值 |
| `Ctrl+Shift+Alt+L` | 从文件重新加载配置 |
| `Ctrl+,` | 打开设置面板 |

### 热重载

启用 `config-watch` 功能后，对 `config.toml` 的更改会自动检测：
- 主题更改即时生效
- 字体大小更改即时生效
- 滚动历史行数限制更新
- Toast 通知："Config reloaded"

## 自定义快捷键

所有快捷键均可在 `[keybindings]` 部分自定义。按键格式：

- 单键：`F11`、`Escape`、`Tab`、`Enter`
- 组合键：`Ctrl+T`、`Ctrl+Shift+D`、`Alt+H`
- 特殊：`Ctrl+Shift+/`、`Ctrl+Shift+\`

## 插件

### Lua 插件

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

示例插件：
```lua
-- ~/.ggterm/plugins/hello.lua
function on_load()
    print("Hello from GGTerm plugin!")
end

function on_resize(cols, rows)
    -- 响应终端调整大小
end
```

### 插件生命周期

1. 插件在启动时从配置目录加载
2. 加载插件时调用 `on_load()`
3. 通过 `mlua` 提供 Lua 运行时

## 会话持久化

```toml
[terminal]
restore_session = false  # 默认：全新启动
# restore_session = true  # 恢复上次会话的标签页/分屏
```

- 窗格或标签页关闭时立即保存会话
- 启动时若 `restore_session = true`，恢复标签页/分屏/工作目录
- 窗口位置和大小也会持久化

## SSH 配置

GGTerm 从以下位置读取 SSH 配置：
- `~/.ssh/config`（可通过命令面板导入）
- 连接管理器以 TOML 存储条目
- 支持密码和公钥认证

## 终端协议支持

GGTerm 实现了全面的终端协议集：

| 协议 | 示例 | 状态 |
|----------|---------|--------|
| SGR | 粗体、斜体、下划线、闪烁、删除线、上划线 | 完整 |
| 光标 | CSI A/B/C/D/E/F/G/H, SCP/RCP, DECSC/DECRC | 完整 |
| 擦除 | ED, EL, DECSED（选择性擦除） | 完整 |
| 滚动 | SU, SD, DECSET 7727（备用屏幕滚动） | 完整 |
| 模式 | DECSET 1/5/6/7/12/25/47/1000-1006/1015-1016/1047-1049/2004/2026/2027 | 完整 |
| OSC | 0/2/4/7/8/9/10-12/52/104/110-112/133/1337/9;4 | 完整 |
| DCS | XTGETTCAP, DECRQSS | 完整 |
| DA | DA1/DA2/DA3 | 完整 |
| DSR | 光标位置、状态、窗口状态 | 完整 |
| DECRQM | 所有标准 + 私有模式 | 完整 |
| Kitty 键盘 | CSI > u push/pop, CSI = u | 完整 |
| 字符集 | G0/G1, US/UK/特殊图形 | 完整 |
| DECSCUSR | 光标形状切换（6 种样式） | 完整 |
| 备用屏幕 | DECSET 47/1047/1049 带网格保存/恢复 | 完整 |

## 故障排除

### 字体问题

**制表符显示为方块（tofu）：**
- macOS：使用 Menlo Regular（非 Bold），因为 Menlo Bold 缺少制表符字形
- 粗体通过亮色显示，而非字重

**CJK 字符不渲染：**
- 确保启用了 `Shaping::Advanced`（默认启用）
- 在系统上安装 CJK 字体

### 终端卡在错误模式

如果 GGTerm 崩溃后 shell 行为异常：
```bash
reset   # 或：stty sane
```

GGTerm 在正常退出时发送重置序列：
- 关闭括号粘贴
- 关闭鼠标跟踪
- 方向键恢复正常模式
- 光标可见
- 小键盘数字模式
- 软重置（DECSTR）

### 空闲时 CPU 使用率过高

GGTerm 在不需要重绘时休眠 50ms。如果 CPU 使用率过高：
- 检查是否有后台进程产生终端输出
- 禁用光标闪烁：`cursor_blink = false`
- 检查 `config-watch` 是否触发了过多重载

### 会话未恢复

在 config.toml 中设置 `restore_session = true`。会话在标签页/窗格关闭和应用退出时保存。

### 标签栏文字不可见

这是一个已知 bug（已修复）。覆盖层渲染顺序：先背景，后文字。

### 窗口位置未持久化

窗口几何信息与会话数据一起保存。启用 `restore_session = true`。

### SSH 连接问题

- 服务器密钥指纹已记录以供验证
- 同时支持密码和公钥认证
- 非阻塞 I/O 防止连接期间 UI 冻结

### P2P 连接问题

- 确保两台设备都在线
- 检查防火墙设置（QUIC 使用 UDP）
- 如果 QR 扫描失败，尝试手动输入 ticket
- iroh 中继回退可处理大多数 NAT 场景

### 获取帮助

- 按 `Ctrl+Shift+/` 获取应用内快捷键帮助
- 按 `Ctrl+Shift+H` 获取 AI 驱动的帮助
- 查看日志：`ggterm -vv` 获取调试输出
- GitHub Issues：https://github.com/topcheer/ggterm/issues
