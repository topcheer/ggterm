## GGTerm Phase 7 — COMPLETE (commit a9ddc70)

### Build State
- 8 crates (added ggterm-ffi, ggterm-ssh)
- 1417 lib tests ALL PASS (7 ignored)

### Phase 7 Tasks (5 tasks, all done)
| Task | Owner | Tests | Description |
|------|-------|-------|-------------|
| P7-A | me_pm | 22 | C-ABI FFI: ggterm_new/free/process_bytes/send_input/take_input/read_cells/dimensions/cursor/resize/take_bell |
| P7-B | dd_dev (me) | 15 (6 ign) | SshSession + impl TerminalTransport (russh 0.61) |
| P7-C | gg_dev | — | Flutter UI Shell (7 Dart files, 1341 lines) |
| P7-D | me_pm | 6 | TerminalTransport trait + PtySession impl |
| P7-E | ggcxf_dev + me_pm | 13 | SessionManager, ScreenData, TerminalSession (api.rs) |

### P7-B: My Contribution
**New crate: `crates/ggterm-ssh/` (3 files)**

1. **Cargo.toml** — `russh = "0.61"` (keys module built-in, no separate russh-keys crate needed)
2. **src/error.rs** — `SshError` enum (8 variants: Connection, Handshake, Auth, Channel, Key, Io, SessionClosed, Runtime) + From<russh::Error>
3. **src/lib.rs** — `SshSession` struct + `impl TerminalTransport`

**API:**
- `SshSession::connect(host, port, user, password) -> Result<Self>` — password auth
- `SshSession::connect_with_key(host, port, user, key_path) -> Result<Self>` — public key auth
- `impl TerminalTransport` — read()/write()/resize()/is_alive()
- Internal `tokio::runtime::Runtime` bridges async russh → sync trait
- `Drop` auto-disconnects via `Disconnect::ByApplication`

**russh 0.61 API Notes:**
- `authenticate_password` returns `AuthResult` (use `.success()`)
- `authenticate_publickey` needs `PrivateKeyWithHashAlg::new(Arc::new(*key), None)`
- `Channel::data(&[u8])` takes AsyncRead directly
- `Channel::window_change(cols, rows, 0, 0)` for PTY resize
- `client::Handler` uses native async fn (no async_trait needed)
- `AuthMethod::PublicKey(Box<PrivateKey>)` (clippy large-enum-variant fix)
- `russh-keys` is NOT needed for russh 0.61 (keys are built-in as `russh::keys::*`)

**Tests (15 total, 6 ignored):**
- Error display/from conversions: 11 tests
- Signature/compilation verification: 4 tests
- Network tests: 6 tests (#[ignore], need real SSH server)

### Test Count
- Phase 23 complete: 1615 tests (desktop)
- Phase 7 complete: 1417 tests (mobile foundation, different feature flags)