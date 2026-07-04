## GGTerm P2P Phase 3: Flutter Integration — COMPLETE

### Files Created (3 new)
1. `mobile/lib/ffi/p2p_bindings.dart` (209 lines) — P2P FFI bindings
   - `P2pBindings` class with optional symbol lookup
   - 4 C-ABI functions: connect, hostTicket, isConnected, generateTicket
   - Optional free_string for C memory cleanup
   - `isAvailable` flag — graceful degradation when P2P not compiled
   - `P2pBindings.autoload()` and `P2pBindings(DynamicLibrary)` constructors

2. `mobile/lib/screens/qr_scan_screen.dart` (336 lines) — QR scanning screen
   - `mobile_scanner` integration with camera lifecycle management
   - Scan → ticket → FFI connect → 15s timeout wait → TerminalScreen
   - Torch toggle, camera flip actions
   - Error overlay with automatic retry

3. `mobile/lib/screens/share_screen.dart` (389 lines) — Share/host screen
   - `qr_flutter` QR code rendering from Iroh NodeTicket
   - State machine: generating → waiting → connected/error
   - Connection polling via `isConnected(0)`
   - Manual ticket copy fallback (ExpansionTile)
   - Regenerate ticket button

### Files Modified (3 existing)
4. `mobile/pubspec.yaml` — Added `mobile_scanner: ^5.0.0` + `qr_flutter: ^4.1.0`
5. `mobile/lib/connection_screen.dart` — Added `onScanQr`/`onShare` callbacks + 2 P2P buttons
6. `mobile/lib/main.dart` — `P2pBindings.autoload()` init + `_onScanQr()`/`_onShare()` navigation

### C API Mapping (confirmed with ggcxf_dev)
| C Function | Dart Method |
|---|---|
| `ggterm_p2p_connect(ticket)` | `P2pBindings.connect(String) -> int` |
| `ggterm_p2p_generate_ticket()` | `P2pBindings.generateTicket() -> String?` |
| `ggterm_p2p_host_ticket(session_id)` | `P2pBindings.hostTicket(int) -> String?` |
| `ggterm_p2p_is_connected(session_id)` | `P2pBindings.isConnected(int) -> bool` |
| `ggterm_p2p_free_string(ptr)` | Auto-called if available |

### Key Design Decisions
1. Optional symbol lookup — P2P functions wrapped in try/catch, `isAvailable` flag
2. `_p2pFreeString` — nullable, older builds may not have it
3. QR scan: 15s timeout for QUIC connection, auto-retry on failure
4. Share mode: polls `isConnected(0)` for host-side connection detection
5. Connection screen: P2P buttons only shown when `isAvailable` is true
6. Import paths: screens at `lib/screens/`, FFI at `lib/ffi/` (consistent with ggcxf_dev's spec)

### Pending: Rust FFI
Waiting for ggcxf_dev to implement the Rust side (`ggterm_p2p_*` functions in ggterm-ffi).
The Dart bindings are ready to work once the C symbols are available.