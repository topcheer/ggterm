# Part 7: P2P 공유 & 모바일

## P2P 터미널 공유

QR 코드를 통해 데스크톱 터미널을 모바일 기기와 공유하세요 — 클라우드 서버가 필요 없어요.

### 작동 방식

GGTerm은 직접 P2P 연결을 위해 iroh(QUIC + NAT traversal)를 사용해요:
- 90% 이상의 P2P 직접 연결 성공률
- 어려운 NAT 환경을 위한 자동 relay 폴백
- 운영 비용 제로 (iroh 공개 relay는 무료)
- Ticket: 약 130자 base32 문자열, QR 코드 하나에 들어감

### 데스크톱 (Host)

1. `Ctrl+Shift+Alt+Q`를 눌러 공유 오버레이 열기
2. 연결 ticket이 포함된 QR 코드가 나타나요
3. 모바일 앱으로 QR 코드를 스캔하세요 (또는 ticket 문자열 복사)
4. 연결되면 모바일 기기에 터미널이 미러링돼요
5. `Esc` 또는 `Ctrl+Shift+Alt+Q`를 눌러 공유 종료

오버레이에 표시되는 정보:
- QR 코드 (어두운 모듈이 사각형으로 렌더링됨)
- 연결 상태 (대기 중 / 연결됨)
- Ticket 문자열 (수동 입력용)
- 사용 방법 안내

### 데이터 흐름

- **PTY 출력 → 모바일**: 모든 터미널 출력이 연결된 모바일 기기로 전달돼요
- **모바일 입력 → PTY**: 모바일 키보드 입력이 데스크톱 PTY로 전달돼요
- **크기 조정**: 터미널 크기 변경이 전파돼요
- **모바일 로컬 에코**: 모바일에서 입력한 문자를 즉시 볼 수 있어요 (PTY echo 대기 없음)

### 모바일 (Client)

#### 연결 옵션

| 옵션 | 설명 |
|--------|-------------|
| SSH | 원격 서버에 연결 (host, port, user, password) |
| Echo Test | 진단용 — 입력한 문자를 에코 (서버 불필요) |
| Scan QR | QR 코드로 데스크톱 터미널에 P2P 연결 |
| Share Terminal | P2P host 모드 (Android 전용 — 로컬 shell 필요) |

#### Scan QR 흐름

1. 연결 화면에서 **Scan QR** 탭
2. 데스크톱 QR 코드에 카메라를 가져다 대세요
3. 터미널 출력이 모바일에 나타나요
4. 모바일 키보드로 입력하여 전송

#### iOS vs Android

- **iOS**: SSH + P2P 클라이언트(Scan QR)만 — 로컬 터미널 없음
- **Android**: 로컬 shell + P2P host를 포함한 모든 기능

### 보안

- P2P 연결은 암호화돼요 (QUIC/TLS)
- SSH 서버 키 fingerprint가 로그에 기록돼요 (SHA256:base64 형식)
- SSH는 비밀번호 및 공개 키 인증을 모두 지원해요

## SSH 연결 관리자

SSH 연결을 저장하고 관리해요:

Command Palette를 통해:
- `ssh.manager` — SSH 연결 관리자 열기
- `terminal.import_ssh` — `~/.ssh/config`에서 호스트 가져오기

기능:
- 이름, 호스트, 포트, 사용자, 인증 방법을 포함한 호스트 항목
- TOML 영속성
- 퍼지 검색
- 빠른 연결

## 로컬 Shell (Android 전용)

Termux 또는 유사한 환경이 있는 Android 기기는 GGTerm 모바일에서 로컬 shell을 직접 실행할 수 있어요.
