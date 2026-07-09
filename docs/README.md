# GGTerm Documentation

> Complete documentation in 5 languages

## Languages

| Language | Directory |
|----------|-----------|
| English | [`en/`](en/) |
| 中文 (Chinese) | [`zh/`](zh/) |
| 日本語 (Japanese) | [`ja/`](ja/) |
| 한국어 (Korean) | [`ko/`](ko/) |
| Español (Spanish) | [`es/`](es/) |

## Documentation Structure

Each language directory contains:

```
{lang}/
├── architecture.md           — System architecture, crate breakdown, protocol support
├── developer-guide.md        — Development setup, contributing, FFI, plugins
└── user-guide/               — Comprehensive user manual (8 parts)
    ├── 01-getting-started.md     — Installation, CLI, config, shell integration
    ├── 02-tabs-panes.md          — Tabs, split panes, navigation, zoom
    ├── 03-text-operations.md     — Selection, copy/paste, search, export, scroll
    ├── 04-appearance.md          — Themes, fonts, opacity, cursor, status bar, profiles
    ├── 05-advanced-features.md   — AI, command history, snippets, broadcast, recording, workspaces
    ├── 06-command-palette.md     — All 70+ command palette commands
    ├── 07-p2p-mobile.md          — P2P sharing, mobile app, SSH manager
    └── 08-config-troubleshooting.md — Full config reference, plugins, troubleshooting
```

## Quick Links

### English
- [Architecture](en/architecture.md)
- [User Guide](en/user-guide/01-getting-started.md)
- [Developer Guide](en/developer-guide.md)

### 中文
- [架构文档](zh/architecture.md)
- [用户手册](zh/user-guide/01-getting-started.md)
- [开发者指南](zh/developer-guide.md)

### 日本語
- [アーキテクチャ](ja/architecture.md)
- [ユーザーガイド](ja/user-guide/01-getting-started.md)
- [開発者ガイド](ja/developer-guide.md)

### 한국어
- [아키텍처](ko/architecture.md)
- [사용자 가이드](ko/user-guide/01-getting-started.md)
- [개발자 가이드](ko/developer-guide.md)

### Español
- [Arquitectura](es/architecture.md)
- [Guía de Usuario](es/user-guide/01-getting-started.md)
- [Guía de Desarrollador](es/developer-guide.md)

## Quick Reference

- **Config example**: [`config.example.toml`](../config.example.toml)
- **Project guide**: [`GGCODE.md`](../GGCODE.md)
- **README**: [`README.md`](../README.md)

## Stats

- **Version**: Phase 55+
- **Code**: 71,791 lines Rust across 9 crates
- **Tests**: 2,143
- **Keyboard shortcuts**: 98
- **Command palette commands**: 70+
- **Themes**: 9 + auto
- **Terminal protocols**: Full VT100/VT220/xterm + Kitty keyboard + DCS
