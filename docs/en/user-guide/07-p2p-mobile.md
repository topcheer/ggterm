# Part 7: P2P Sharing & Mobile

## P2P Terminal Sharing

Share your desktop terminal with a mobile device via QR code — no cloud server needed.

### How It Works

GGTerm uses iroh (QUIC + NAT traversal) for direct peer-to-peer connections:
- 90%+ P2P direct connection success rate
- Automatic relay fallback for difficult NATs
- Zero operational cost (iroh public relay is free)
- Ticket: ~130 character base32 string, fits in one QR code

### Desktop (Host)

1. Press `Ctrl+Shift+Alt+Q` to open the share overlay
2. A QR code appears with your connection ticket
3. Scan the QR code with the mobile app (or copy the ticket string)
4. Once connected, the mobile device mirrors your terminal
5. Press `Esc` or `Ctrl+Shift+Alt+Q` to close sharing

The overlay shows:
- QR code (dark modules rendered as rectangles)
- Connection status (waiting / connected)
- Ticket string (for manual entry)
- Instructions

### Data Flow

- **PTY output → Mobile**: All terminal output is teed to the connected mobile device
- **Mobile input → PTY**: Mobile keyboard input is forwarded to the desktop PTY
- **Resize**: Terminal dimension changes are propagated
- **Local echo on mobile**: Mobile sees typed characters immediately (no waiting for PTY echo)

### Mobile (Client)

#### Connection Options

| Option | Description |
|--------|-------------|
| SSH | Connect to remote server (host, port, user, password) |
| Echo Test | Diagnostic — echoes typed characters (no server needed) |
| Scan QR | P2P connect to desktop terminal via QR code |
| Share Terminal | P2P host mode (Android only — requires local shell) |

#### Scan QR Flow

1. Tap **Scan QR** in the connection screen
2. Point camera at the desktop QR code
3. Terminal output appears on mobile
4. Type on mobile keyboard to send input

#### iOS vs Android

- **iOS**: SSH + P2P client (Scan QR) only — no local terminal
- **Android**: All features including local shell + P2P host

### Security

- P2P connection is encrypted (QUIC/TLS)
- SSH server key fingerprint is logged (SHA256:base64 format)
- SSH supports both password and public key authentication

## SSH Connection Manager

Store and manage SSH connections:

Via Command Palette:
- `ssh.manager` — Open SSH connection manager
- `terminal.import_ssh` — Import hosts from `~/.ssh/config`

Features:
- Host entries with name, host, port, user, auth method
- TOML persistence
- Fuzzy search
- Quick connect

## Local Shell (Android Only)

Android devices with Termux or similar can run a local shell directly in GGTerm mobile.
