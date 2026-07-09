# Part 8: 설정, 플러그인 & 문제 해결

## 설정 파일

위치: `~/.ggterm/config.toml`

### 모든 설정 옵션

```toml
[appearance]
theme = "dark"                  # 아래 테마 목록 참조
font_family = "monospace"        # 폰트 패밀리 이름
font_size = 14                   # 픽셀 단위 폰트 크기
cell_width = 8                   # 픽셀 단위 셀 너비
cell_height = 16                 # 픽셀 단위 셀 높이
cursor_style = "block"           # block | underline | bar
cursor_blink = true              # 커서 깜빡임 켜기/끄기
background_opacity = 1.0         # 0.0 투명 ~ 1.0 불투명
padding = 8                      # 픽셀 단위 콘텐츠 패딩
cursor_line_highlight = false    # 커서 줄 하이라이트 (Vim 방식)
word_chars = ""                  # 선택을 위한 추가 단어 문자

[terminal]
scrollback_lines = 10000         # 최대 스크롤백 히스토리
shell = ""                       # 빈 값 = $SHELL 또는 /bin/sh
restore_session = false           # 시작 시 탭/분할 복원

[ai]
enabled = false
api_endpoint = ""
model = ""

[plugins]
enabled = false
directory = "~/.ggterm/plugins"

[keybindings]
# 단축키 커스터마이즈 (아래 참조)
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
# 프로필별 선택적 오버라이드
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

### 설정 관리 단축키

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+Shift+,` (Cmd+,) | 편집기에서 config 파일 열기 |
| `Ctrl+Shift+O` | config 파일 열기 (대체) |
| `Ctrl+Shift+J` | shell 설정 편집 (.bashrc/.zshrc) |
| `Ctrl+Shift+Alt+E` | config를 클립보드로 내보내기 (TOML) |
| `Ctrl+Shift+Alt+I` | 클립보드에서 config 가져오기 |
| `Ctrl+Shift+Alt+R` | config를 기본값으로 재설정 |
| `Ctrl+Shift+Alt+L` | 파일에서 config 새로고침 |
| `Ctrl+,` | 설정 패널 열기 |

### 핫 리로드

`config-watch` 기능으로 `config.toml`의 변경 사항이 자동으로 감지돼요:
- 테마 변경이 즉시 적용돼요
- 폰트 크기 변경이 즉시 적용돼요
- 스크롤백 줄 제한이 업데이트돼요
- Toast 알림: "Config 새로고침됨"

## 단축키 커스터마이즈

모든 단축키는 `[keybindings]` 섹션에서 커스터마이즈할 수 있어요. 키 형식:

- 단일 키: `F11`, `Escape`, `Tab`, `Enter`
- 조합 키: `Ctrl+T`, `Ctrl+Shift+D`, `Alt+H`
- 특수: `Ctrl+Shift+/`, `Ctrl+Shift+\`

## 플러그인

### Lua 플러그인

```toml
[plugins]
enabled = true
directory = "~/.ggterm/plugins"
```

플러그인 예제:
```lua
-- ~/.ggterm/plugins/hello.lua
function on_load()
    print("Hello from GGTerm plugin!")
end

function on_resize(cols, rows)
    -- 터미널 크기 변경에 반응
end
```

### 플러그인 생명주기

1. 플러그인은 시작 시 설정된 디렉토리에서 로드돼요
2. 플러그인이 로드될 때 `on_load()`가 호출돼요
3. `mlua`를 통한 Lua 런타임

## 세션 영속성

```toml
[terminal]
restore_session = false  # 기본값: 깨끗한 시작
# restore_session = true  # 마지막 세션에서 탭/분할 복원
```

- pane 또는 탭이 닫힐 때 세션이 즉시 저장돼요
- `restore_session = true`로 시작하면 탭/분할/작업 디렉토리가 복원돼요
- 창 위치와 크기도 영속화돼요

## SSH 설정

GGTerm은 다음에서 SSH 설정을 읽어요:
- `~/.ssh/config` (Command Palette를 통해 가져오기 가능)
- 연결 관리자는 항목을 TOML에 저장해요
- 비밀번호 및 공개 키 인증을 지원해요

## 터미널 프로토콜 지원

GGTerm은 포괄적인 터미널 프로토콜을 구현해요:

| 프로토콜 | 예시 | 상태 |
|----------|---------|--------|
| SGR | 굵게, 이탤릭, 밑줄, 깜빡임, 취소선, 오버라인 | 전체 |
| 커서 | CSI A/B/C/D/E/F/G/H, SCP/RCP, DECSC/DECRC | 전체 |
| 지우기 | ED, EL, DECSED (선택적) | 전체 |
| 스크롤 | SU, SD, DECSET 7727 (alt scroll) | 전체 |
| 모드 | DECSET 1/5/6/7/12/25/47/1000-1006/1015-1016/1047-1049/2004/2026/2027 | 전체 |
| OSC | 0/2/4/7/8/9/10-12/52/104/110-112/133/1337/9;4 | 전체 |
| DCS | XTGETTCAP, DECRQSS | 전체 |
| DA | DA1/DA2/DA3 | 전체 |
| DSR | 커서 위치, 상태, 창 상태 | 전체 |
| DECRQM | 모든 표준 + private 모드 | 전체 |
| Kitty keyboard | CSI > u push/pop, CSI = u | 전체 |
| 문자 집합 | G0/G1, US/UK/special graphics | 전체 |
| DECSCUSR | 커서 모양 변경 (6가지 스타일) | 전체 |
| Alt screen | DECSET 47/1047/1049, grid 저장/복원 포함 | 전체 |

## 문제 해결

### 폰트 문제

**박스 그리기 문자가 네모(tofu)로 표시되는 경우:**
- macOS: Menlo Regular가 사용돼요 (Bold가 아님) — Menlo Bold에는 박스 그리기 글리프가 없기 때문이에요
- 굵게는 폰트 굵기가 아닌 밝은 색상으로 표시돼요

**CJK 문자가 렌더링되지 않는 경우:**
- `Shaping::Advanced`가 활성화되어 있는지 확인하세요 (기본값)
- 시스템에 CJK 폰트를 설치하세요

### 터미널이 잘못된 모드에서 빠져나오지 않는 경우

GGTerm이 비정상 종료 후 shell이 이상하게 동작하는 경우:
```bash
reset   # 또는: stty sane
```

GGTerm은 정상 종료 시 재설정 시퀀스를 전송해요:
- Bracketed paste 끄기
- 마우스 추적 끄기
- 커서 키 일반 모드
- 커서 표시
- 키패드 숫자 모드
- 소프트 리셋 (DECSTR)

### 대기 중 CPU 사용량이 높은 경우

GGTerm은 다시 그릴 필요가 없을 때 50ms 동안 대기해요. CPU 사용량이 높은 경우:
- 터미널 출력을 생성하는 백그라운드 프로세스를 확인하세요
- 커서 깜빡임 비활성화: `cursor_blink = false`
- `config-watch`가 과도한 새로고침을 트리거하는지 확인하세요

### 세션이 복원되지 않는 경우

config.toml에서 `restore_session = true`를 설정하세요. 세션은 탭/pane가 닫힐 때와 앱 종료 시 저장돼요.

### 탭 바 텍스트가 보이지 않는 경우

이전에 알려진 버그였어요 (수정됨). 오버레이 렌더링 순서: 배경 먼저, 그 다음 텍스트.

### 창 위치가 영속화되지 않는 경우

창 geometry는 세션 데이터와 함께 저장돼요. `restore_session = true`를 활성화하세요.

### SSH 연결 문제

- 서버 키 fingerprint가 확인을 위해 로그에 기록돼요
- 비밀번호 및 공개 키 인증을 모두 지원해요
- 비동기 I/O로 연결 중 UI 멈춤을 방지해요

### P2P 연결 문제

- 두 기기가 모두 온라인 상태인지 확인하세요
- 방화벽 설정을 확인하세요 (QUIC은 UDP를 사용해요)
- QR 스캔이 실패하면 수동 ticket 입력을 시도해 보세요
- iroh relay 폴백이 대부분의 NAT 시나리오를 처리해요

### 도움말 얻기

- `Ctrl+Shift+/`를 눌러 앱 내 단축키 도움말 확인
- `Ctrl+Shift+H`를 눌러 AI 기반 도움말 사용
- 로그 확인: `ggterm -vv`로 디버그 출력 보기
- GitHub Issues: https://github.com/topcheer/ggterm/issues
