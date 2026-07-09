# Part 1: はじめに

## インストール

### ソースからビルド

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# デバッグビルド
cargo run --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# リリースビルド（起動が速く、最適化済み）
cargo build --release --features "desktop ai plugin plugin-lua config-watch" --bin ggterm
# バイナリ: target/release/ggterm
```

### ビルド済みリリース

[GitHub Releases](https://github.com/topcheer/ggterm/releases) からダウンロードしてください：
- **macOS**: ユニバーサル .dmg（Apple Silicon + Intel）
- **Linux**: .deb パッケージまたは tarball
- **Windows**: .zip

## コマンドラインインターフェース

```bash
ggterm [OPTIONS]

Options:
  -c, --cols <N>              初期列数（デフォルト: 80）
  -r, --rows <N>              初期行数（デフォルト: 24）
  -s, --shell <PATH>          シェルのパス（デフォルト: $SHELL）
  -t, --title <TITLE>         ウィンドウタイトル（デフォルト: "GGTerm"）
      --theme <NAME>          カラーテーマ（デフォルト: "dark"）
      --font-size <PX>        フォントサイズ（ピクセル単位、デフォルト: 16）
      --cell-width <PX>       セル幅（デフォルト: 8）
  -w, --working-directory <DIR>  このディレクトリでシェルを開始
  -C, --config <PATH>         カスタム設定ファイルのパス
  -e, --execute <CMD...>      インタラクティブシェルの代わりにコマンドを実行
      --hold                  コマンド終了後もターミナルを開いたままにする
      --fullscreen            フルスクリーンモードで開始
      --maximize              最大化して開始
  -v                          詳細ログ（-v info, -vv debug, -vvv trace）
```

### CLI の例

```bash
# デフォルトターミナル
ggterm

# 大きなターミナルで zsh を使用
ggterm --cols 120 --rows 40 --shell /bin/zsh

# Dracula テーマ、フォントサイズ 18
ggterm --theme dracula --font-size 18

# vim を実行し、終了後も保持
ggterm -e vim --hold

# フルスクリーンで特定のディレクトリから開始
ggterm --fullscreen --working-directory ~/projects

# カスタム設定ファイル
ggterm --config ~/.config/ggterm/custom.toml
```

## 設定ファイル

場所: `~/.ggterm/config.toml`

```toml
[appearance]
theme = "dark"                # 9テーマ + auto
font_family = "monospace"
font_size = 14
cursor_style = "block"         # block | underline | bar
cursor_blink = true
background_opacity = 1.0       # 0.0 透明 ～ 1.0 不透明
# padding = 8                 # コンテンツのパディング（ピクセル単位）
# cursor_line_highlight = false
# word_chars = ""             # 選択用の追加ワード文字

[terminal]
scrollback_lines = 10000
shell = ""                     # 空 = $SHELL または /bin/sh
restore_session = false        # 起動時にタブ/スプリットを復元

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# Part 8: 設定を参照

[profiles.develop]
# プロファイルごとのオプションオーバーライド
theme = "nord"
font_size = 12
```

## 初回起動

1. GGTerm はデフォルトシェルを単一タブで起動します
2. シェル統合（OSC 133）は bash/zsh/fish に自動的に注入されます
3. 初回使用時に `~/.ggterm/config.toml` が作成されます
4. いつでも `Ctrl+Shift+/` を押すとすべてのキーボードショートカットを確認できます

## シェル統合

GGTerm は OSC 133 マークを自動注入し、以下の機能を提供します：
- コマンド検出（プロンプト/コマンド/出力の境界）
- 終了コードの追跡
- コマンド履歴サイドバー
- 「最後のコマンド出力をコピー」機能

自動注入が失敗した場合の手動セットアップ：

```bash
# bash (~/.bashrc)
source /path/to/ggterm/shell/bash.sh

# zsh (~/.zshrc)
source /path/to/ggterm/shell/zsh.zsh

# fish (~/.config/fish/config.fish)
source /path/to/ggterm/shell/fish.fish
```
