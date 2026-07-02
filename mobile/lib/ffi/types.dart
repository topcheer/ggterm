/// Dart mirror of Rust GGTermCell struct.
///
/// Memory layout must match `#[repr(C)]` struct in ggterm-ffi/src/lib.rs:
///   char_code: u32, flags: u16, fg: u32, bg: u32
/// Total: 12 bytes (with u16 padding to 4-byte boundary → actual size 12).
/// In practice, repr(C) on 64-bit: u32 + u16 + u32 + u32 with padding.
/// We use explicit field offsets via dart:ffi Struct.
import 'dart:ffi';

/// FFI struct matching Rust's `GGTermCell`.
///
/// Layout (repr(C)):
/// ```text
/// offset 0:  u32 char_code
/// offset 4:  u16 flags
/// offset 6:  u16 padding
/// offset 8:  u32 fg
/// offset 12: u32 bg
/// ```
/// Total size: 16 bytes.
@TypedStruct()
class GGTermCell extends Struct {
  @Uint32() external int charCode;
  @Uint16() external int flags;
  @Uint16() external int _padding;
  @Uint32() external int fg;
  @Uint32() external int bg;
}

/// Cell flags matching Rust CellFlags bits.
class CellFlags {
  static const int bold = 0x01;
  static const int faint = 0x02;
  static const int italic = 0x04;
  static const int underline = 0x08;
  static const int blink = 0x10;
  static const int reverse = 0x20;
  static const int hidden = 0x40;
  static const int strikethrough = 0x80;
  static const int wide = 0x200;
  static const int protected = 0x400;
}

/// Color packing helpers matching Rust's pack_color().
class ColorCodec {
  /// Pack RGB into u32: 0x00RRGGBB.
  static int packRgb(int r, int g, int b) {
    return (r << 16) | (g << 8) | b;
  }

  /// Check if a color u32 is indexed.
  static bool isIndexed(int packed) {
    return (packed & 0x01000000) != 0;
  }

  /// Get the ANSI index (0-255) from a packed color.
  static int getIndex(int packed) {
    return packed & 0x0000FFFF;
  }

  /// Check if a color is default (0).
  static bool isDefault(int packed) => packed == 0;

  /// Extract RGB components from a packed color.
  static (int, int, int) getRgb(int packed) {
    final r = (packed >> 16) & 0xFF;
    final g = (packed >> 8) & 0xFF;
    final b = packed & 0xFF;
    return (r, g, b);
  }
}

/// ANSI 16-color palette for indexed color resolution.
class AnsiPalette {
  static const List<int> standard16 = [
    0x000000, // 0: black
    0xCD0000, // 1: red
    0x00CD00, // 2: green
    0xCDCD00, // 3: yellow
    0x0000EE, // 4: blue
    0xCD00CD, // 5: magenta
    0x00CDCD, // 6: cyan
    0xE5E5E5, // 7: white
    0x7F7F7F, // 8: bright black (gray)
    0xFF0000, // 9: bright red
    0x00FF00, // 10: bright green
    0xFFFF00, // 11: bright yellow
    0x5C5CFF, // 12: bright blue
    0xFF00FF, // 13: bright magenta
    0x00FFFF, // 14: bright cyan
    0xFFFFFF, // 15: bright white
  ];

  /// Resolve a packed color value to RGB.
  /// [packed] is the u32 from FFI.
  /// [defaultFg] and [defaultBg] are fallback RGB values.
  static int resolve(int packed, {int defaultFg = 0xD4D4D4, int defaultBg = 0x1E1E2E}) {
    if (ColorCodec.isDefault(packed)) return defaultFg;
    if (ColorCodec.isIndexed(packed)) {
      final idx = ColorCodec.getIndex(packed);
      if (idx < standard16.length) return standard16[idx];
      return defaultFg;
    }
    return packed & 0x00FFFFFF;
  }
}
