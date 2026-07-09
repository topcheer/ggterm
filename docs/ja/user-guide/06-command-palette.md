# Part 6: コマンドパレット

コマンドパレット（`Ctrl+Shift+P`）は、すべての GGTerm アクションへのファジー検索アクセスを提供します。

## コマンドパレットの使用

1. `Ctrl+Shift+P` を押して開く
2. 入力してコマンドをファジー検索
3. `Up/Down` で結果をナビゲート
4. `Enter` で実行
5. `Esc` で閉じる

## 完全なコマンドリスト

### タブ管理

| コマンド | アクション |
|---------|--------|
| `tab.new` | 新しいタブ |
| `tab.close` | タブを閉じる |
| `tab.next` | 次のタブ |
| `tab.prev` | 前のタブ |
| `tab.toggle_last` | 最後のタブに切り替え |
| `tab.rename` | タブの名前を変更 |
| `tab.move_left` | タブを左に移動 |
| `tab.move_right` | タブを右に移動 |
| `tab.duplicate` | タブを複製 |
| `tab.close_others` | 他のタブを閉じる |
| `tab.toggle_pin` | タブをピン留め/解除 |
| `tab.reopen_closed` | 閉じたタブを再び開く |
| `window.new` | 新しいウィンドウ |

### スプリットペイン

| コマンド | アクション |
|---------|--------|
| `split.horizontal` | 水平スプリット |
| `split.vertical` | 垂直スプリット |
| `split.focus_next` | 次のペインにフォーカス |
| `split.focus_prev` | 前のペインにフォーカス |
| `split.zoom` | ペインズームの切り替え |
| `split.balance` | ペインを均等化 |
| `split.swap` | ペインのコンテンツを交換 |
| `split.close` | 現在のペインを閉じる |

### ターミナル操作

| コマンド | アクション |
|---------|--------|
| `terminal.clear` | 画面をクリア |
| `terminal.clear_all` | 画面 + スクロールバックをクリア |
| `terminal.reset` | ターミナルをリセット（RIS） |
| `terminal.reset_all` | すべてのターミナルをリセット |
| `terminal.select_all` | すべてのテキストを選択 |
| `terminal.copy` | 選択範囲をコピー |
| `terminal.copy_cwd` | 現在のディレクトリをコピー |
| `terminal.paste` | ペースト |
| `terminal.search` | スクロールバックを検索 |
| `terminal.open_url` | カーソル位置の URL を開く |
| `terminal.save_scrollback` | スクロールバックをファイルに保存 |
| `terminal.export_html` | HTML としてエクスポート |
| `terminal.copy_as_html` | HTML としてコピー |
| `terminal.copy_last_output` | 最後のコマンド出力をコピー |
| `terminal.copy_visible` | 表示テキストをコピー |
| `terminal.copy_markdown` | Markdown としてコピー |
| `terminal.toggle_lock` | ターミナルロックの切り替え |
| `terminal.scroll_mode` | スクロールバックブラウズモードの切り替え |
| `terminal.open_in_finder` | cwd を Finder/Explorer で開く |
| `terminal.open_shell_config` | シェル設定を編集（.bashrc/.zshrc） |
| `terminal.import_ssh` | ~/.ssh/config から SSH ホストをインポート |
| `terminal.edit_selection` | 選択テキストを編集 |
| `terminal.run_selection` | 選択テキストをコマンドとして実行 |
| `terminal.search_selection` | 選択テキストを Web で検索 |
| `terminal.send_ctrl_c_all` | すべてのペインに Ctrl+C を送信 |
| `terminal.new_session` | 新しい SSH セッション |

### 外観

| コマンド | アクション |
|---------|--------|
| `theme.cycle` | テーマを順に切り替え |
| `font.zoom_in` | ズームイン |
| `font.zoom_out` | ズームアウト |
| `font.zoom_reset` | フォントサイズをリセット |
| `opacity.increase` | 不透明度を上げる |
| `opacity.decrease` | 不透明度を下げる |
| `view.toggle_cursor_line` | カーソル行ハイライトの切り替え |

### ウィンドウ

| コマンド | アクション |
|---------|--------|
| `view.fullscreen` | フルスクリーンの切り替え |
| `view.maximize` | 最大化の切り替え |
| `view.status_bar` | ステータスバーの切り替え |
| `window.always_on_top` | 常に最前面の切り替え |
| `settings.open` | 設定パネルを開く |
| `config.open` | 設定ファイルを開く |
| `config.reload` | 設定をリロード |

### AI

| コマンド | アクション |
|---------|--------|
| `ai.explain` | 出力を説明 |
| `ai.suggest` | コマンドを提案 |
| `ai.help` | AI ヘルプ |

### セッションとプロファイル

| コマンド | アクション |
|---------|--------|
| `session.save` | セッションを保存 |
| `session.profile` | プロファイルを順に切り替え |
| `ssh.manager` | SSH 接続マネージャーを開く |

### カーソルエフェクト

| コマンド | アクション |
|---------|--------|
| `cursor.trail` | カーソルパーティクルトレールを有効化 |
| `cursor.glow` | カーソルグローを有効化 |
| `cursor.none` | カーソルエフェクトを無効化 |

### その他

| コマンド | アクション |
|---------|--------|
| `perf.toggle` | パフォーマンスモニターの切り替え |
| `sound.toggle` | サウンドの切り替え |
| `shell.switch` | シェルスイッチャーを開く |
| `workspace.next` | 次のワークスペース |
| `workspace.prev` | 前のワークスペース |
| `workspace.add` | ワークスペースを追加 |
