## GGTerm P2P 共享 — 方案决策

### 决定: 使用 Iroh (QUIC + NAT Traversal)

研究结论：Iroh 是 GGTerm P2P 共享功能的最优打洞方案。

### 关键对比
- **Iroh (推荐)**: NodeTicket ~130字符(1个QR码)，QUIC原生，90%+ P2P直连成功率，自动relay fallback
- **WebRTC (否决)**: SDP 3.8KB需4+个QR码，UX灾难
- **自研QUIC+STUN (否决)**: 工作量过大
- **中继服务器 (备选)**: 非P2P但最可靠

### 架构设计
- 新增 crate: `ggterm-p2p` (iroh + QUIC)
- `P2pTransport` 实现 `TerminalTransport` trait
- 桌面: iroh Endpoint → NodeTicket → QR码
- 移动: 扫码 → iroh connect → QUIC流 → 终端
- 默认零运营成本 (iroh公开relay免费)

### 关键依赖
- `iroh = "0.33"` — QUIC + NAT traversal + relay
- `qrcode = "0.14"` — QR码生成
- `mobile_scanner` (Flutter) — QR码扫描

### 实现路线
- Phase 1: ggterm-p2p crate (P2pTransport + host/client)
- Phase 2: QR码 + 桌面UI (分享菜单 + QR overlay)
- Phase 3: 移动端FFI + Flutter扫码
- Phase 4: 安全 + 体验优化 (连接确认、断线重连)