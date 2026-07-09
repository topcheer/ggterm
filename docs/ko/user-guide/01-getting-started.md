# Part 1: 시작하기

## 설치

### 소스에서 빌드

```bash
git clone https://github.com/topcheer/ggterm.git
cd ggterm

# Debug 빌드
cargo run --features "desktop ai plugin plugin-lua config-watch" --bin ggterm

# Release 빌드 (빠른 시작, 최적화됨)
cargo build --release --features "desktop ai plugin plugin-lua config-watch" --bin ggterm
# 바이너리: target/release/ggterm
```

### 사전 빌드된 릴리즈

[GitHub Releases](https://github.com/topcheer/ggterm/releases)에서 다운로드:
- **macOS**: Universal .dmg (Apple Silicon + Intel)
- **Linux**: .deb 패키지 또는 tarball
- **Windows**: .zip

## 명령줄 인터페이스

```bash
ggterm [OPTIONS]

Options:
  -c, --cols <N>              초기 컬럼 수 (기본값: 80)
  -r, --rows <N>              초기 행 수 (기본값: 24)
  -s, --shell <PATH>          Shell 경로 (기본값: $SHELL)
  -t, --title <TITLE>         창 제목 (기본값: "GGTerm")
      --theme <NAME>          색상 테마 (기본값: "dark")
      --font-size <PX>        픽셀 단위 폰트 크기 (기본값: 16)
      --cell-width <PX>       셀 너비 (기본값: 8)
  -w, --working-directory <DIR>  이 디렉토리에서 shell 시작
  -C, --config <PATH>         커스텀 config 파일 경로
  -e, --execute <CMD...>      대화형 shell 대신 명령 실행
      --hold                  명령 종료 후 터미널 열어두기
      --fullscreen            전체 화면 모드로 시작
      --maximize              최대화하여 시작
  -v                          상세 로깅 (-v info, -vv debug, -vvv trace)
```

### CLI 예제

```bash
# 기본 터미널
ggterm

# zsh로 큰 터미널
ggterm --cols 120 --rows 40 --shell /bin/zsh

# Dracula 테마, 폰트 크기 18
ggterm --theme dracula --font-size 18

# vim 실행 후 종료해도 유지
ggterm -e vim --hold

# 특정 디렉토리에서 전체 화면으로 시작
ggterm --fullscreen --working-directory ~/projects

# 커스텀 config 파일
ggterm --config ~/.config/ggterm/custom.toml
```

## 설정 파일

위치: `~/.ggterm/config.toml`

```toml
[appearance]
theme = "dark"                # 9개 테마 + auto
font_family = "monospace"
font_size = 14
cursor_style = "block"         # block | underline | bar
cursor_blink = true
background_opacity = 1.0       # 0.0 투명 ~ 1.0 불투명
# padding = 8                 # 픽셀 단위 콘텐츠 패딩
# cursor_line_highlight = false
# word_chars = ""             # 선택을 위한 추가 단어 문자

[terminal]
scrollback_lines = 10000
shell = ""                     # 빈 값 = $SHELL 또는 /bin/sh
restore_session = false        # 시작 시 탭/분할 복원

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# Part 8: 설정 참조

[profiles.develop]
# 프로필별 선택적 오버라이드
theme = "nord"
font_size = 12
```

## 첫 실행

1. GGTerm은 기본 shell로 단일 탭에서 시작해요
2. Shell 통합(OSC 133)이 bash/zsh/fish에 자동으로 주입돼요
3. config 파일은 최초 사용 시 `~/.ggterm/config.toml`에 생성돼요
4. 언제든 `Ctrl+Shift+/`를 누르면 모든 단축키를 볼 수 있어요

## Shell 통합

GGTerm은 다음 기능을 위해 OSC 133 마크를 자동으로 주입해요:
- 명령 감지 (프롬프트/명령/출력 경계)
- 종료 코드 추적
- 명령 히스토리 사이드바
- "마지막 명령 출력 복사" 기능

수동 설정 (자동 주입이 실패하는 경우):

```bash
# bash (~/.bashrc)
source /path/to/ggterm/shell/bash.sh

# zsh (~/.zshrc)
source /path/to/ggterm/shell/zsh.zsh

# fish (~/.config/fish/config.fish)
source /path/to/ggterm/shell/fish.fish
```
