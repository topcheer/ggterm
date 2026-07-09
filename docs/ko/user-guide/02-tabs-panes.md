# Part 2: 탭, Pane & 분할

## 탭

### 탭 관리

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+T` | 새 탭 열기 |
| `Ctrl+W` | 현재 탭 닫기 |
| `Alt+1-9` | N번째 탭으로 전환 |
| `Ctrl+Tab` | 다음 탭 |
| `Ctrl+Shift+Tab` | 이전 탭 |
| `Ctrl+Shift+\`` | 마지막 탭 토글 (가장 최근 두 탭 사이 전환) |
| `Ctrl+Shift+T` | 마지막으로 닫은 탭 다시 열기 |
| `Ctrl+Shift+N` | 새 창 열기 |
| `Ctrl+Shift+I` | 현재 탭 이름 변경 |
| `Ctrl+Shift+PageUp` | 탭을 왼쪽으로 이동 |
| `Ctrl+Shift+PageDown` | 탭을 오른쪽으로 이동 |
| `Ctrl+Shift+Alt+D` | 현재 탭 복제 (동일 shell + cwd) |
| `Ctrl+Shift+Alt+W` | 다른 모든 탭 닫기 |

### 탭 상호작용

- **탭 클릭**: 해당 탭으로 전환
- **탭 더블클릭**: 이름 변경
- **탭 중간 클릭**: 닫기 (브라우저 방식)
- **탭 드래그**: 같은 레벨에서 순서 변경
- **탭 우클릭**: 컨텍스트 메뉴 (닫기, 다른 탭 닫기, 오른쪽 탭 닫기, 고정/고정 해제, 분할)
- **"+" 클릭**: 드롭다운 메뉴 (새 탭, 수평 분할, 수직 분할)

### 탭 고정

Command Palette를 통해 탭을 고정하면 실수로 닫는 것을 방지할 수 있어요:
- 고정된 탭은 핀 표시가 나타나요
- 고정된 탭에서 `Ctrl+W`는 무시돼요
- 고정 해제는 Command Palette를 통해 하면 닫을 수 있어요

### 탭 제목 동기화

탭 제목이 실행 중인 프로그램과 자동으로 동기화돼요:
- OSC 0/2에서 프로그램 이름 표시 (예: "vim", "htop", "less")
- shell 이름으로 폴백 (예: "zsh", "bash")
- 백그라운드 탭에 bell이 오면 bell 표시가 나타나요
- alternate screen 모드일 때 `(alt)` 표시

## 분할 Pane

### 분할 만들기

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+Shift+D` | 수평 분할 (좌 | 우) |
| `Ctrl+Shift+\` | 수직 분할 (상 / 하) |

새 pane는 활성 pane의 작업 디렉토리(OSC 7)를 상속해요.

### Pane 이동

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+Shift+[` | 이전 pane로 포커스 |
| `Ctrl+Shift+]` | 다음 pane로 포커스 |
| `Alt+H` | 왼쪽 pane로 포커스 (vim 방식) |
| `Alt+J` | 아래 pane로 포커스 (vim 방식) |
| `Alt+K` | 위 pane로 포커스 (vim 방식) |
| `Alt+L` | 오른쪽 pane로 포커스 (vim 방식) |

- **pane 클릭**: 해당 pane로 포커스 전환
- **pane에서 마우스 휠**: 해당 pane의 콘텐츠 스크롤

### Pane 작업

| 단축키 | 동작 |
|----------|--------|
| `Ctrl+Shift+X` | 활성 pane 콘텐츠를 다음 pane와 교환 |
| `Ctrl+Shift+Z` | pane 줌 토글 (최대화/복원) |
| `Ctrl+Shift+Alt+Arrows` | 분할 비율 조정 |
| `Ctrl+Shift+Alt+B` | 분할 pane 균형 맞추기 (균등 간격) |
| `Ctrl+Shift+Alt+N` | 레이아웃을 단일 pane로 재설정 |

### Pane 줌

`Ctrl+Shift+Z`는 줌 모드를 토글해요:
- 줌 시: 활성 pane가 전체 창을 채워요
- pane 테두리가 숨겨져요
- 마우스 포커스가 활성 pane에 고정돼요
- 구분선 드래그가 비활성화돼요
- 상태 표시줄에 `ZOOM` 표시가 나타나요

### 구분선 드래그

- pane 사이의 구분선을 드래그하여 크기를 조절할 수 있어요
- 줌 모드에서는 구분선 드래그가 비활성화돼요

### 다중 Pane 렌더링

- 각 pane는 자체 터미널 grid를 독립적으로 렌더링해요
- 활성 pane는 밝은 파란색 테두리를 가져요
- 비활성 pane는 어두운 테두리를 가져요
- pane 간격: 6px
- Scissor rect를 통해 콘텐츠가 pane 경계를 넘어가지 않아요
