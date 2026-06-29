# Phase 2: VT Compatibility — Design Document

## Overview

Phase 1 built a functional terminal core: VTE parser → Terminal state machine → Grid → GPU renderer.
Phase 2 closes the VT compatibility gaps so that **vim, less, htop, tmux, and man** all work correctly.

**Success criteria**: `vim` opens, edits, and exits without screen corruption. `less` paginates cleanly. `htop` updates in-place without flicker. `tmux` renders status bars.

---

## Current Implementation Audit

### Already working (term/mod.rs)

| Category | Commands | Status |
|---|---|---|
| Cursor movement | CUU/A, CUD/B, CUF/C, CUB/D, CNL/E, CPL/F, CHA/G, CUP/H, HVP/f, VPA/d | ✅ Complete |
| Erase | ED/J (0,1,2,3), EL/K (0,1,2) | ✅ Complete |
| Scroll | SU/S, SD/T, DECSTBM/r | ✅ Complete |
| Edit | IL/L, DL/M, DCH/P, ICH/@, ECH/X | ✅ Complete |
| Tab | HTS (ESC H), TBC/g (0,3), CHT/I, CBT/Z | ✅ Complete |
| SGR | Full 0-107 + 38;5;n + 38;2;r;g;b | ✅ Complete |
| ESC | DECSC/7, DECRC/8, RIS/c, IND/D, NEL/E, RI/M, HTS/H | ✅ Complete |
| DEC modes | 1, 6, 7, 25, 2004 | ✅ Complete |
| Control chars | BEL, BS, HT, LF/VT/FF, CR | ✅ Complete |
| UTF-8 | Multi-byte reassembly, wide char, combining | ✅ Complete |

### Critical gaps

| Gap | Impact | Task |
|---|---|---|
| Alt screen = bool flag only, no buffer swap | vim/less destroy screen on exit | P2-1 |
| No character set support (DEC Special Graphics) | Box-drawing chars render as letters | P2-4 |
| ED mode 3 doesn't clear scrollback | Scrollback persists after program exit | P2-2 |
| No device attribute reports | Programs can't detect terminal capabilities | P2-5 |
| No DECSCUSR (cursor shape) | Cursor style stuck at block | P2-2 |

---

## Task Assignments

### P2-1: Alternate Screen Buffer — dd_dev (P0, highest priority)

**Problem**: `set_dec_mode` (line 249) only sets `modes.alt_screen = true/false`. No Grid is saved/restored.

**Design**:

```rust
pub struct Terminal {
    grid: Grid,                // primary (or alt, whichever is active)
    alt_grid: Option<Grid>,    // the inactive buffer (lazily allocated)
    // ...
}
```

**DECSET 1049** (most common — used by vim/less):
1. Save cursor position (DECSC equivalent)
2. Switch to alternate buffer (swap `grid` ↔ `alt_grid`)
3. Clear the alternate buffer
4. Cursor → (0, 0)

**DECRST 1049**:
1. Switch back to primary buffer (swap back)
2. Restore cursor position (DECRC equivalent)

**DECSET 1047** (simpler variant):
1. Switch to alternate buffer (no cursor save)
2. Clear alt buffer

**DECRST 1047**:
1. Switch back to primary buffer
2. Clear the alternate buffer on exit

**DECSET 47** (oldest variant):
1. Switch to alternate buffer (no cursor save, no clear)

**DECRST 47**:
1. Switch back to primary buffer

**Implementation approach**: Use `std::mem::swap` to exchange `grid` and `alt_grid` without copying. The render layer always reads `terminal.grid()`, so no render changes needed.

**Tests** (minimum 12):
1. DECSET 1049 saves primary content, switches to cleared alt
2. DECRST 1049 restores primary content + cursor
3. Writing to alt doesn't affect primary grid
4. DECSET/DECRST 1047 (no cursor save)
5. DECSET/DECRST 47 (no clear)
6. Nested DECSET (idempotent — stays in alt)
7. Nested DECRST (idempotent — stays in primary)
8. Alt screen has independent scroll region
9. Resize works in alt screen
10. SGR state preserved across screen switch
11. Scrollback only in primary (alt has no scrollback)
12. Origin mode resets on screen switch

### P2-2: CSI Extensions + DECSET/DECRST — gg_dev

**Current gaps to fill**:

1. **ED mode 3** (`CSI 3J`): Clear scrollback buffer
   ```rust
   2 => { self.grid.clear(); }
   3 => { self.grid.clear_scrollback(); }  // NEW: needs Grid::clear_scrollback()
   ```

2. **DECSCUSR** (`CSI Ps SP q`): Cursor shape selection
   ```rust
   // In csi(), when intermediates contain b' ' (SP) and final_byte == b'q'
   // Ps: 0=blink block, 1=blink block, 2=steady block,
   //     3=blink underline, 4=steady underline, 5=blink bar, 6=steady bar
   ```
   Add `cursor_style: CursorStyle` to Terminal (enum: Block, Underline, Bar) + `blink: bool`.

3. **REP** (`CSI Ps b`): Repeat preceding character N times.

4. **SCO Save/Restore** (`CSI s` / `CSI u`): Alternative cursor save.

5. **DECSET 5 (DECSCNM)**: Reverse video mode — swap fg/bg globally.

6. **DECSET 12**: Cursor blink (different from DECSCUSR).

**Tests**: 10+ for all new commands.

### P2-3: DECSTBM + Tab Stops Polish — ggcxf_dev

**Already implemented**: DECSTBM (`CSI Pt;Pb r`) and tab stops (HTS, TBC, CHT, CBT) all work.

**Gaps to fill**:

1. **DECSTBM edge case**: `CSI r` (no params) should reset scroll region to full screen (currently `param()` forces minimum 1).

2. **DECALN** (`ESC # 8`): Fill entire screen with 'E' for alignment testing.
   ```rust
   // In esc(), handle intermediate b'#' and final b'8'
   b'8' if intermediates.contains(&b'#') => {
       for y in 0..self.grid.height() {
           for x in 0..self.grid.width() {
               self.grid.put_char(x, y, 'E');
           }
       }
   }
   ```

3. **Scroll region + cursor interaction**: Verify CUU/CUD respect scroll region boundaries (already implemented, needs test coverage).

4. **Tab stops on resize**: Currently resets to default 8-column stops. Acceptable per VT spec.

5. **DECSTBM with origin mode**: When DECOM is set, DECSTBM origin is relative to scroll region. Verify/fix.

**Tests**: 10+ covering DECALN, DECSTBM edge cases, tab stop + scroll region.

### P2-4: Character Sets (G0/G1/SCS) — me_pm

**Current**: No charset support. All output assumed UTF-8.

**Design**:

```rust
#[derive(Clone, Copy, PartialEq)]
pub enum Charset {
    Ascii,       // USASCII (default)
    DecSpecial,  // DEC Special Graphics (box drawing)
}

pub struct Terminal {
    // ...
    g0_charset: Charset,
    g1_charset: Charset,
    active_charset: bool,  // false=G0, true=G1
}
```

**Control codes**:
- `SI` (0x0F / LS0): Activate G0
- `SO` (0x0E / LS1): Activate G1

**ESC sequences** (designate charset):
- `ESC ( B` → G0 = USASCII
- `ESC ( 0` → G0 = DEC Special Graphics
- `ESC ) B` → G1 = USASCII
- `ESC ) 0` → G1 = DEC Special Graphics

**DEC Special Graphics translation table** (0x5f-0x7e):
```
_ → blank  ` → ◆   a → ▒   b → HT   c → FF
d → CR/LF  e → NL   f → °   g → ±   h → NL
i → VT     j → ┘   k → ┐   l → ┌   m → └
n → ┼      o → ⎺   p → ⎻   q → ─   r → ⎼
s → ⎽      t → ├   u → ┤   v → ┴   w → ┬
x → ─      y → ≤   z → ≥   { → π   | → ≠
} → £      ~ → ·
```

**Implementation**: In `print()`, after UTF-8 decode, if active charset is DecSpecial, translate char if it's in range 0x5f-0x7e.

**Tests**: 12+ covering all 32 DEC Special Graphics chars, SI/SO switching, ESC ( 0 / ESC ) 0 / ESC ( B.

### P2-5: vttest + Device Reports — me_pm

**Device attribute reports** (needed for ncurses, vim detection):

1. **DA1** (`CSI > c` or `CSI > 0 c`): Primary device attributes
   Response: `CSI ? 64 ; 1 ; 2 ; 4 ; 6 ; 9 ; 15 ; 16 ; 22 c`
   (64=rows, features: 128-color, ANSI colors, rectangular editing, etc.)

2. **DA2** (`CSI > c`): Secondary device attributes
   Response: `CSI > 0 ; 100 ; 0 c` (terminal type=0, firmware=100)

3. **DA3** (`CSI = c`): Tertiary device attributes
   Response: `CSI = ! | 00000000 c`

4. **CPR** (`CSI 6 n`): Cursor position report
   Response: `CSI {row} ; {col} R`

5. **DECXCPR** (`CSI ? 6 n`): Extended cursor position report
   Response: `CSI ? {row} ; {col} ; {page} R`

6. **DECSTR** (`ESC [ ! p`): Soft terminal reset — reset to defaults without RIS.

**Note**: Device reports require a response channel. The Terminal struct needs an output buffer or callback to send responses back through the PTY.

**Design**: Add `pending_responses: VecDeque<String>` to Terminal. The app layer drains this after each `feed()` and writes to PTY.

---

## Integration Notes

### Render layer
Zero changes needed. The renderer always reads `terminal.grid()`. Alt screen swap is transparent.

### App layer (app.rs)
- After `app.pump()`, drain `terminal.pending_responses()` and write to PTY.
- `cursor_style()` getter for rendering cursor shape.

### Grid layer
- Add `clear_scrollback()` for ED mode 3.
- Existing `scroll_region()` and `set_scroll_region()` are sufficient.

### Parser layer
No changes. OSC 133 handling (P3-A, parallel track) goes in `osc()` which already works.

---

## Coordination

### Avoiding conflicts in term/mod.rs

| Field | Task | Conflict risk |
|---|---|---|
| `alt_grid`, `saved_alt_cursor` | P2-1 only | None |
| `cursor_style`, `cursor_blink` | P2-2 only | None |
| `g0_charset`, `g1_charset`, `active_charset` | P2-4 only | None |
| `pending_responses` | P2-5 only | None |
| `set_dec_mode()` | P2-1 (alt screen) + P2-2 (new modes) | **Coordinate**: dd_dev and gg_dev both touch this method |

**Rule**: dd_dev owns `set_dec_mode` lines 249 first. gg_dev adds new modes after dd_dev merges P2-1. Or work in separate worktrees and merge sequentially.

### Merge order
1. P2-1 (dd_dev) — highest priority, merge first
2. P2-3 (ggcxf_dev) — independent, can merge anytime
3. P2-4 (me_pm) — independent, can merge anytime
4. P2-2 (gg_dev) — merge after P2-1 (touches set_dec_mode)
5. P2-5 (me_pm) — merge last (needs Grid::clear_scrollback from P2-2)

### P3-A (parallel)
ggcxf_dev works on OSC 133 in `osc()` handler + new `command_marks` field. Completely independent of all P2 work. Can merge anytime.
