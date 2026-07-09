# 第六部分：命令面板

命令面板（`Ctrl+Shift+P`）提供对所有 GGTerm 操作的模糊搜索访问。

## 使用命令面板

1. 按 `Ctrl+Shift+P` 打开
2. 输入以模糊搜索命令
3. `Up/Down` 导航结果
4. `Enter` 执行
5. `Esc` 关闭

## 完整命令列表

### 标签页管理

| 命令 | 功能 |
|---------|--------|
| `tab.new` | 新建标签页 |
| `tab.close` | 关闭标签页 |
| `tab.next` | 下一个标签页 |
| `tab.prev` | 上一个标签页 |
| `tab.toggle_last` | 切换最近标签页 |
| `tab.rename` | 重命名标签页 |
| `tab.move_left` | 标签页左移 |
| `tab.move_right` | 标签页右移 |
| `tab.duplicate` | 复制标签页 |
| `tab.close_others` | 关闭其他标签页 |
| `tab.toggle_pin` | 固定/取消固定标签页 |
| `tab.reopen_closed` | 重新打开已关闭的标签页 |
| `window.new` | 新建窗口 |

### 分屏窗格

| 命令 | 功能 |
|---------|--------|
| `split.horizontal` | 水平分屏 |
| `split.vertical` | 垂直分屏 |
| `split.focus_next` | 聚焦下一个窗格 |
| `split.focus_prev` | 聚焦上一个窗格 |
| `split.zoom` | 切换窗格缩放 |
| `split.balance` | 平衡窗格 |
| `split.swap` | 交换窗格内容 |
| `split.close` | 关闭当前窗格 |

### 终端操作

| 命令 | 功能 |
|---------|--------|
| `terminal.clear` | 清屏 |
| `terminal.clear_all` | 清除屏幕 + 滚动历史 |
| `terminal.reset` | 重置终端（RIS） |
| `terminal.reset_all` | 重置所有终端 |
| `terminal.select_all` | 全选文本 |
| `terminal.copy` | 复制选区 |
| `terminal.copy_cwd` | 复制当前目录 |
| `terminal.paste` | 粘贴 |
| `terminal.search` | 搜索滚动历史 |
| `terminal.open_url` | 打开光标处的 URL |
| `terminal.save_scrollback` | 保存滚动历史到文件 |
| `terminal.export_html` | 导出为 HTML |
| `terminal.copy_as_html` | 复制为 HTML |
| `terminal.copy_last_output` | 复制上一条命令输出 |
| `terminal.copy_visible` | 复制可见文本 |
| `terminal.copy_markdown` | 复制为 Markdown |
| `terminal.toggle_lock` | 切换终端锁定 |
| `terminal.scroll_mode` | 切换滚动历史浏览模式 |
| `terminal.open_in_finder` | 在 Finder/Explorer 中打开 cwd |
| `terminal.open_shell_config` | 编辑 shell 配置（.bashrc/.zshrc） |
| `terminal.import_ssh` | 从 ~/.ssh/config 导入 SSH 主机 |
| `terminal.edit_selection` | 编辑选中文本 |
| `terminal.run_selection` | 将选中文本作为命令运行 |
| `terminal.search_selection` | 在网络上搜索选中文本 |
| `terminal.send_ctrl_c_all` | 向所有窗格发送 Ctrl+C |
| `terminal.new_session` | 新建 SSH 会话 |

### 外观

| 命令 | 功能 |
|---------|--------|
| `theme.cycle` | 循环切换主题 |
| `font.zoom_in` | 放大 |
| `font.zoom_out` | 缩小 |
| `font.zoom_reset` | 重置字体大小 |
| `opacity.increase` | 增加不透明度 |
| `opacity.decrease` | 降低不透明度 |
| `view.toggle_cursor_line` | 切换光标行高亮 |

### 窗口

| 命令 | 功能 |
|---------|--------|
| `view.fullscreen` | 切换全屏 |
| `view.maximize` | 切换最大化 |
| `view.status_bar` | 切换状态栏 |
| `window.always_on_top` | 切换窗口置顶 |
| `settings.open` | 打开设置面板 |
| `config.open` | 打开配置文件 |
| `config.reload` | 重新加载配置 |

### AI

| 命令 | 功能 |
|---------|--------|
| `ai.explain` | 解释输出 |
| `ai.suggest` | 建议命令 |
| `ai.help` | AI 帮助 |

### 会话与配置文件

| 命令 | 功能 |
|---------|--------|
| `session.save` | 保存会话 |
| `session.profile` | 循环切换配置文件 |
| `ssh.manager` | 打开 SSH 连接管理器 |

### 光标特效

| 命令 | 功能 |
|---------|--------|
| `cursor.trail` | 启用光标粒子拖尾 |
| `cursor.glow` | 启用光标发光 |
| `cursor.none` | 禁用光标特效 |

### 其他

| 命令 | 功能 |
|---------|--------|
| `perf.toggle` | 切换性能监视器 |
| `sound.toggle` | 切换声音 |
| `shell.switch` | 打开 shell 切换器 |
| `workspace.next` | 下一个工作区 |
| `workspace.prev` | 上一个工作区 |
| `workspace.add` | 添加工作区 |
