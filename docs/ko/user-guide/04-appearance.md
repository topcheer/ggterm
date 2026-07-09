# Part 4: 테마, 폰트 & 외관

## 테마

### 9개 내장 테마 + Auto

| 테마 | 배경 | 스타일 |
|-------|-----------|-------|
| `dark` | 어두운 회색 | 기본 다크 테마 |
| `light` | 흰색 | 밝은 환경 |
| `dracula` | 어두운 보라 | 인기 있는 다크 테마 |
| `solarized-dark` | 짙은 파랑 | 개발 집중 |
| `solarized-light` | 따뜻한 크림 | 읽기 |
| `gruvbox` | 흙빛 다크 | 레트로 웜 |
| `nord` | 북극 파랑 | 깔끔한 미니멀 |
| `tokyo-night` | 짙은 남색 | 야간 코딩 |
| `catppuccin-mocha` | 부드러운 갈색 | 따뜻한 다크 |
| `auto` | OS 따름 | 자연스러운 전환 |

### 테마 컨트롤

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+Shift+Alt+T` | 테마 순환 |
| `Ctrl+Shift+T` | 테마 순환 (대체) |

### Auto 테마

`theme = "auto"`일 때, GGTerm이 OS 외관을 감지해요:
- **macOS**: AppleInterfaceStyle
- **Linux**: GTK_THEME / gsettings
- **Windows**: Registry 확인

### 동적 색상 (OSC 10/11/12)

프로그램이 런타임에 테마 색상을 오버라이드할 수 있어요:
- `OSC 10` — 전경색 설정/조회
- `OSC 11` — 배경색 설정/조회
- `OSC 12` — 커서 색상 설정/조회
- `OSC 104/110/111/112` — 기본값으로 재설정

### 커스텀 팔레트 (OSC 4)

base16-shell, wal, pywal 같은 프로그램이 커스텀 16색 팔레트를 설정할 수 있어요:
- `OSC 4 ; N ; rgb:RR/GG/BB` — 팔레트 색상 N 설정
- `OSC 104 ; N` — 팔레트 색상 N 재설정
- 렌더러가 인덱스 색상에 오버라이드를 적용해요

## 폰트

### 폰트 컨트롤

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+=` | 확대 (폰트 크기 +1.5px) |
| `Ctrl+-` | 축소 (폰트 크기 -1.5px) |
| `Ctrl+0` | 기본 폰트 크기로 재설정 |
| `Ctrl+Shift+Wheel` | 마우스 휠로 폰트 확대 |

### 플랫폼 기본 폰트

| 플랫폼 | 폰트 |
|----------|------|
| macOS | Menlo (Regular만 — Bold 변형은 박스 그리기 글리프가 없음) |
| Linux | DejaVu Sans Mono |
| Windows | Cascadia Mono |

**굵은 텍스트**: 폰트 굵기가 아닌 밝은 색상으로 구분돼요 (xterm/Alacritty 표준).

**CJK 폴백**: `Shaping::Advanced`를 통해 CJK 문자에 대한 자동 폰트 폴백이 활성화돼요.

### 셀 크기

- 셀 너비 = 'M'의 정확한 float 어드밴스 (반올림 없음)
- 셀 높이 = 픽셀 단위 폰트 크기
- 실제 픽셀 크기는 CSI 14t/15t/16t로 보고돼요

## 배경 불투명도

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+Shift+Alt+]` | 불투명도 증가 (+5%) |
| `Ctrl+Shift+Alt+[` | 불투명도 감소 (-5%) |

불투명도 범위: 0.0 (완전 투명) ~ 1.0 (완전 불투명). Toast 알림으로 백분율이 표시돼요.

설정: `[appearance] background_opacity = 0.85`

## 창 컨트롤

| 단축키 | 동작 |
|----------|--------|
| `F11` | 전체 화면 토글 |
| `Ctrl+Shift+Enter` | 최대화 토글 |
| `Ctrl+Shift+Alt+A` | 항상 위에 표시 토글 |
| `Ctrl+Shift+B` | 상태 표시줄 토글 |

### 투명 타이틀바 (macOS)

macOS에서는 자연스러운 외관을 위해 타이틀바가 투명하게 처리돼요.

## 커서

### 커서 스타일

설정: `[appearance] cursor_style = "block"`

옵션: `block`, `underline`, `bar`

프로그램이 DECSCUSR(CSI N q)로 커서 스타일을 변경할 수 있어요.

### 커서 깜빡임

설정: `[appearance] cursor_blink = true`

- 깜빡임은 부드러운 페이드를 위해 sine-wave alpha를 사용해요
- 사용자 입력 시 깜빡임이 재설정돼요
- 깜빡임 위상이 SGR 5 깜빡임 텍스트 렌더링과 공유돼요

### 커서 줄 하이라이트

설정: `[appearance] cursor_line_highlight = false`

커서가 위치한 전체 줄을 하이라이트해요 (Vim의 `cursorline`과 같은 방식).

### 커서 효과

Command Palette를 통해:
- **cursor.trail** — 커서가 파티클 잔상을 남겨요
- **cursor.glow** — 커서에 빛나는 효과가 있어요
- **cursor.none** — 커서 효과 비활성화

## 상태 표시줄

토글: `Ctrl+Shift+B`

상태 표시줄에 표시되는 정보:
- 커서 위치 (행:열)
- 탭 수
- 현재 디렉토리 (OSC 7에서)
- 원격 호스트 (OSC 1337의 SSH 표시)
- 실행 중인 명령 + 타이머
- 진행률 백분율 (OSC 9;4에서)
- Broadcast 모드 표시
- 녹화 표시
- Pane 줌 표시
- Bell 표시
- 사운드 토글 표시
- 선택 영역 단어 수
- Config 오류 표시

## 프로필

프로필을 통해 외관 설정을 전환할 수 있어요:

```toml
[profiles.develop]
theme = "nord"
font_size = 12

[profiles.present]
theme = "light"
font_size = 18
```

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+Shift+Alt+F` | config 프로필 순환 |
| `Ctrl+Shift+Alt+P` | 프로필 순환 (대체) |

## 설정 패널

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+,` | 설정 패널 열기 |

방향키로 설정을 탐색하고, 값을 인라인으로 편집할 수 있어요.

## 디버그 오버레이

| 단축키 | 동작 |
|----------|--------|
| `F1` | 디버그 오버레이 토글 (FPS, 셀 수, pane 정보) |
| `Ctrl+Shift+G` | 성능 모니터 토글 |

## Pane별 렌더링

각 pane는 독립적인 렌더러 상태를 유지해요:
- 역방향 비디오 모드 (DECSCNM)
- 동적 전경/배경색 (OSC 10/11)
- 밑줄 색상 (SGR 58)
- 깜빡임 텍스트 위상 (SGR 5)
