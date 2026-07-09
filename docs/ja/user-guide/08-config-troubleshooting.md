# Part 8: 設定、プラグインとトラブルシューティング

## 設定ファイル

場所: `~/.ggterm/config.toml`

### すべての設定オプション

```toml
[appearance]
theme = "dark"                  # 下記のテーマリストを参照
font_family = "monospace"        # フォントファミリー名
font_size = 14                   # フォントサイズ（ピクセル単位）
cell_width = 8                   # セル幅（ピクセル単位）
cell_height = 16                 # セル高（ピクセル単位）
cursor_style = "block"           # block | underline | bar
cursor_blink = true              # カーソルブリンクのオン/オフ
background_opacity = 1.0         # 0.0 透明 ～ 1.0 不透明
padding = 8                      # コンテンツのパディング（ピクセル単位）
cursor_line_highlight = false    # カーソル行をハイライト（Vim スタイル）
word_chars = ""                  # 選択用の追加ワード文字

[terminal]
scrollback_lines = 10000         # スクロールバック履歴の最大値
shell = ""                       # 空 = $SHELL または /bin/sh
restore_session = false           # 起動時にタブ/スプリットを復元

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# キーボードショートカットをカスタマイズ（下記を参照）
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
# プロファイルごとのオプションオーバーライド
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

### 設定管理ショートカット

| ショートカット | アクション |
|----------|--------|
| `Ctrl+Shift+,`（Cmd+,） | 設定ファイルをエディタで開く |
| `Ctrl+Shift+O` | 設定ファイルを開く（代替） |
| `Ctrl+Shift+J` | シェル設定を編集（.bashrc/.zshrc） |
| `Ctrl+Shift+Alt+E` | 設定をクリップボードにエクスポート（TOML） |
| `Ctrl+Shift+Alt+I` | クリップボードから設定をインポート |
| `Ctrl+Shift+Alt+R` | 設定をデフォルトにリセット |
| `Ctrl+Shift+Alt+L` | ファイルから設定をリロード |
| `Ctrl+,` | 設定パネルを開く |

### ホットリロード

`config-watch` 機能により、`config.toml` の変更が自動的に検出されます：
- テーマの変更が即座に適用されます
- フォントサイズの変更が即座に適用されます
- スクロールバック行数制限が更新されます
- トースト通知：「Config reloaded」

## キーバインドのカスタマイズ

すべてのキーバインドは `[keybindings]` セクションでカスタマイズできます。キーのフォーマット：

- 単一キー：`F11`、`Escape`、`Tab`、`Enter`
- 修飾キー付き：`Ctrl+T`、`Ctrl+Shift+D`、`Alt+H`
- 特殊：`Ctrl+Shift+/`、`Ctrl+Shift+\`

## プラグイン

### Lua プラグイン

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

プラグインの例：
```lua
-- ~/.ggterm/plugins/hello.lua
function on_load()
    print("Hello from GGTerm plugin!")
end

function on_resize(cols, rows)
    -- ターミナルリサイズに反応
end
```

### プラグインライフサイクル

1. プラグインは起動時に設定されたディレクトリからロードされます
2. プラグインのロード時に `on_load()` が呼び出されます
3. `mlua` による Lua ランタイム

## セッション永続化

```toml
[terminal]
restore_session = false  # デフォルト: クリーンな起動
# restore_session = true  # 最後のセッションからタブ/スプリットを復元
```

- ペインまたはタブが閉じられるとセッションが即座に保存されます
- `restore_session = true` で起動すると、タブ/スプリット/作業ディレクトリが復元されます
- ウィンドウの位置とサイズも永続化されます

## SSH 設定

GGTerm は以下から SSH 設定を読み取ります：
- `~/.ssh/config`（Command Palette からインポート可能）
- 接続マネージャーはエントリを TOML に保存
- パスワード認証と公開鍵認証をサポート

## ターミナルプロトコルサポート

GGTerm は包括的なターミナルプロトコルセットを実装しています：

| プロトコル | 例 | 状態 |
|----------|---------|--------|
| SGR | Bold、italic、underline、blink、strikethrough、overline | Full |
| Cursor | CSI A/B/C/D/E/F/G/H、SCP/RCP、DECSC/DECRC | Full |
| Erase | ED、EL、DECSED（selective） | Full |
| Scroll | SU、SD、DECSET 7727（alt scroll） | Full |
| Modes | DECSET 1/5/6/7/12/25/47/1000-1006/1015-1016/1047-1049/2004/2026/2027 | Full |
| OSC | 0/2/4/7/8/9/10-12/52/104/110-112/133/1337/9;4 | Full |
| DCS | XTGETTCAP、DECRQSS | Full |
| DA | DA1/DA2/DA3 | Full |
| DSR | Cursor position、status、window state | Full |
| DECRQM | All standard + private modes | Full |
| Kitty keyboard | CSI > u push/pop、CSI = u | Full |
| Character sets | G0/G1、US/UK/special graphics | Full |
| DECSCUSR | Cursor shape change（6 styles） | Full |
| Alt screen | DECSET 47/1047/1049 with grid save/restore | Full |

## トラブルシューティング

### フォントの問題

**ボックス描画文字が四角（tofu）で表示される：**
- macOS：Menlo Regular が使用されます（Bold ではない）。Menlo Bold にはボックス描画グリフがないためです
- Bold は明るい色で表現され、ウェイトではありません

**CJK 文字がレンダリングされない：**
- `Shaping::Advanced` が有効になっていることを確認してください（デフォルト）
- システムに CJK フォントをインストールしてください

### ターミナルが誤ったモードのままになった場合

GGTerm のクラッシュ後、シェルが異常動作する場合：
```bash
reset   # または: stty sane
```

GGTerm は正常終了時にリセットシーケンスを送信します：
- Bracketed paste off
- Mouse tracking off
- Cursor keys normal
- Cursor visible
- Keypad numeric
- Soft reset（DECSTR）

### アイドル時の高い CPU 使用率

GGTerm は再描画が不要な場合に50ms スリープします。CPU 使用率が高い場合：
- ターミナル出力を生成するバックグラウンドプロセスを確認してください
- カーソルブリンクを無効化：`cursor_blink = false`
- `config-watch` が過剰なリロードを引き起こしていないか確認してください

### セッションが復元されない

config.toml で `restore_session = true` を設定してください。セッションはタブ/ペインのクローズ時およびアプリ終了時に保存されます。

### タブバーのテキストが見えない

これは既知のバグでした（修正済み）。オーバーレイのレンダリング順序：背景を先に描画し、次にテキストを描画します。

### ウィンドウの位置が永続化されない

ウィンドウジオメトリはセッションデータと共に保存されます。`restore_session = true` を有効にしてください。

### SSH 接続の問題

- サーバーキーのフィンガープリントが確認用にログに記録されます
- パスワード認証と公開鍵認証の両方をサポート
- ノンブロッキング I/O により接続中の UI フリーズを防止します

### P2P 接続の問題

- 両方のデバイスがオンラインであることを確認してください
- ファイアウォール設定を確認してください（QUIC は UDP を使用します）
- QR スキャンが失敗した場合、手動でチケットを入力してみてください
- iroh リレーフォールバックがほとんどの NAT シナリオを処理します

### ヘルプの入手

- `Ctrl+Shift+/` でアプリ内ショートカットヘルプを表示
- `Ctrl+Shift+H` で AI 搭載ヘルプを表示
- ログの確認：`ggterm -vv` でデバッグ出力
- GitHub Issues: https://github.com/topcheer/ggterm/issues
