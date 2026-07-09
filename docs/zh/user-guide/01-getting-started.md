# 第一部分：快速入门

## 安装

### 从源码构建

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Debug 构建
cargo run --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Release 构建（启动更快，已优化）
cargo build --release --features "desktop ai plugin plugin-lua config-watch" --bin ggterm
# 二进制文件：target/release/ggterm
```

### 预构建版本

从 [GitHub Releases](https://github.com/topcheer/ggterm/releases) 下载：
- **macOS**：通用 .dmg（Apple Silicon + Intel）
- **Linux**：.deb 包或 tarball
- **Windows**：.zip

## 命令行界面

```bash
ggterm [OPTIONS]

Options:
  -c, --cols <N>              初始列数（默认：80）
  -r, --rows <N>              初始行数（默认：24）
  -s, --shell <PATH>          Shell 路径（默认：$SHELL）
  -t, --title <TITLE>         窗口标题（默认："GGTerm"）
      --theme <NAME>          颜色主题（默认："dark"）
      --font-size <PX>        字体大小，单位像素（默认：16）
      --cell-width <PX>       单元格宽度（默认：8）
  -w, --working-directory <DIR>  在此目录中启动 shell
  -C, --config <PATH>         自定义配置文件路径
  -e, --execute <CMD...>      执行命令而非交互式 shell
      --hold                  命令退出后保持终端打开
      --fullscreen            以全屏模式启动
      --maximize              以最大化启动
  -v                          详细日志（-v info, -vv debug, -vvv trace）
```

### 命令行示例

```bash
# 默认终端
ggterm

# 大尺寸终端，使用 zsh
ggterm --cols 120 --rows 40 --shell /bin/zsh

# Dracula 主题，字体大小 18
ggterm --theme dracula --font-size 18

# 运行 vim 并在退出后保持
ggterm -e vim --hold

# 全屏启动并指定目录
ggterm --fullscreen --working-directory ~/projects

# 自定义配置文件
ggterm --config ~/.config/ggterm/custom.toml
```

## 配置文件

位置：`~/.ggterm/config.toml`

```toml
[appearance]
theme = "dark"                # 9 个主题 + auto
font_family = "monospace"
font_size = 14
cursor_style = "block"         # block | underline | bar
cursor_blink = true
background_opacity = 1.0       # 0.0 透明 到 1.0 不透明
# padding = 8                 # 内容内边距，单位像素
# cursor_line_highlight = false
# word_chars = ""             # 选区额外的单词字符

[terminal]
scrollback_lines = 10000
shell = ""                     # 空值 = $SHELL 或 /bin/sh
restore_session = false        # 启动时恢复标签页/分屏

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# 参见第八部分：配置

[profiles.develop]
# 每个配置文件的可选覆盖项
theme = "nord"
font_size = 12
```

## 首次运行

1. GGTerm 以默认 shell 在单个标签页中启动
2. Shell 集成（OSC 133）会自动注入 bash/zsh/fish
3. 首次使用时在 `~/.ggterm/config.toml` 创建配置文件
4. 随时按 `Ctrl+Shift+/` 查看所有键盘快捷键

## Shell 集成

GGTerm 自动注入 OSC 133 标记，用于：
- 命令检测（提示符/命令/输出边界）
- 退出码跟踪
- 命令历史侧边栏
- "复制上一条命令输出"功能

手动设置（如果自动注入失败）：

```bash
# bash（~/.bashrc）
source /path/to/ggterm/shell/bash.sh

# zsh（~/.zshrc）
source /path/to/ggterm/shell/zsh.zsh

# fish（~/.config/fish/config.fish）
source /path/to/ggterm/shell/fish.fish
```
