# Part 6: Command Palette

Command Palette(`Ctrl+Shift+P`)는 모든 GGTerm 동작에 대한 퍼지 검색 접근을 제공해요.

## Command Palette 사용법

1. `Ctrl+Shift+P`를 눌러 열기
2. 입력하여 명령을 퍼지 검색
3. `Up/Down`으로 결과 탐색
4. `Enter`로 실행
5. `Esc`로 닫기

## 전체 명령 목록

### 탭 관리

| 명령 | 동작 |
|---------|--------|
| `tab.new` | 새 탭 |
| `tab.close` | 탭 닫기 |
| `tab.next` | 다음 탭 |
| `tab.prev` | 이전 탭 |
| `tab.toggle_last` | 마지막 탭 토글 |
| `tab.rename` | 탭 이름 변경 |
| `tab.move_left` | 탭 왼쪽으로 이동 |
| `tab.move_right` | 탭 오른쪽으로 이동 |
| `tab.duplicate` | 탭 복제 |
| `tab.close_others` | 다른 탭 닫기 |
| `tab.toggle_pin` | 탭 고정/해제 |
| `tab.reopen_closed` | 닫은 탭 다시 열기 |
| `window.new` | 새 창 |

### 분할 Pane

| 명령 | 동작 |
|---------|--------|
| `split.horizontal` | 수평 분할 |
| `split.vertical` | 수직 분할 |
| `split.focus_next` | 다음 pane로 포커스 |
| `split.focus_prev` | 이전 pane로 포커스 |
| `split.zoom` | pane 줌 토글 |
| `split.balance` | pane 균형 맞추기 |
| `split.swap` | pane 콘텐츠 교환 |
| `split.close` | 현재 pane 닫기 |

### 터미널 작업

| 명령 | 동작 |
|---------|--------|
| `terminal.clear` | 화면 지우기 |
| `terminal.clear_all` | 화면 + 스크롤백 지우기 |
| `terminal.reset` | 터미널 재설정 (RIS) |
| `terminal.reset_all` | 모든 터미널 재설정 |
| `terminal.select_all` | 모든 텍스트 선택 |
| `terminal.copy` | 선택 영역 복사 |
| `terminal.copy_cwd` | 현재 디렉토리 복사 |
| `terminal.paste` | 붙여넣기 |
| `terminal.search` | 스크롤백 검색 |
| `terminal.open_url` | 커서 위치의 URL 열기 |
| `terminal.save_scrollback` | 스크롤백을 파일로 저장 |
| `terminal.export_html` | HTML로 내보내기 |
| `terminal.copy_as_html` | HTML로 복사 |
| `terminal.copy_last_output` | 마지막 명령 출력 복사 |
| `terminal.copy_visible` | 보이는 텍스트 복사 |
| `terminal.copy_markdown` | Markdown으로 복사 |
| `terminal.toggle_lock` | 터미널 잠금 토글 |
| `terminal.scroll_mode` | 스크롤백 탐색 모드 토글 |
| `terminal.open_in_finder` | cwd를 Finder/Explorer에서 열기 |
| `terminal.open_shell_config` | shell 설정 편집 (.bashrc/.zshrc) |
| `terminal.import_ssh` | ~/.ssh/config에서 SSH 호스트 가져오기 |
| `terminal.edit_selection` | 선택된 텍스트 편집 |
| `terminal.run_selection` | 선택된 텍스트를 명령으로 실행 |
| `terminal.search_selection` | 선택된 텍스트를 웹에서 검색 |
| `terminal.send_ctrl_c_all` | 모든 pane에 Ctrl+C 전송 |
| `terminal.new_session` | 새 SSH 세션 |

### 외관

| 명령 | 동작 |
|---------|--------|
| `theme.cycle` | 테마 순환 |
| `font.zoom_in` | 확대 |
| `font.zoom_out` | 축소 |
| `font.zoom_reset` | 폰트 크기 재설정 |
| `opacity.increase` | 불투명도 증가 |
| `opacity.decrease` | 불투명도 감소 |
| `view.toggle_cursor_line` | 커서 줄 하이라이트 토글 |

### 창

| 명령 | 동작 |
|---------|--------|
| `view.fullscreen` | 전체 화면 토글 |
| `view.maximize` | 최대화 토글 |
| `view.status_bar` | 상태 표시줄 토글 |
| `window.always_on_top` | 항상 위에 표시 토글 |
| `settings.open` | 설정 패널 열기 |
| `config.open` | config 파일 열기 |
| `config.reload` | config 새로고침 |

### AI

| 명령 | 동작 |
|---------|--------|
| `ai.explain` | 출력 설명 |
| `ai.suggest` | 명령 제안 |
| `ai.help` | AI 도움말 |

### 세션 & 프로필

| 명령 | 동작 |
|---------|--------|
| `session.save` | 세션 저장 |
| `session.profile` | 프로필 순환 |
| `ssh.manager` | SSH 연결 관리자 열기 |

### 커서 효과

| 명령 | 동작 |
|---------|--------|
| `cursor.trail` | 커서 파티클 잔상 활성화 |
| `cursor.glow` | 커서 빛나는 효과 활성화 |
| `cursor.none` | 커서 효과 비활성화 |

### 기타

| 명령 | 동작 |
|---------|--------|
| `perf.toggle` | 성능 모니터 토글 |
| `sound.toggle` | 사운드 토글 |
| `shell.switch` | shell 전환기 열기 |
| `workspace.next` | 다음 워크스페이스 |
| `workspace.prev` | 이전 워크스페이스 |
| `workspace.add` | 워크스페이스 추가 |
