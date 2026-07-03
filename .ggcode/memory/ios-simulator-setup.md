## GGTerm iOS Simulator — COMPLETE

### Build State
- Rust static library: `aarch64-apple-ios-sim` target, 18MB libggterm_ffi.a
- Flutter iOS app: builds and runs on iPhone 14 Pro simulator (iOS 16.1)
- Connection screen displays correctly with dark theme

### Key Requirements
1. **Rustup cargo, not Homebrew**: `~/.cargo/bin/cargo build -p ggterm-ffi --target aarch64-apple-ios-sim --release` (Homebrew cargo doesn't know about rustup targets)
2. **-force_load**: Required because Dart uses `DynamicLibrary.process()` for runtime symbol lookup, so linker doesn't see compile-time references. Without force_load, all symbols get dead-stripped.
3. **xcconfig approach works**: `OTHER_LDFLAGS = $(inherited) -force_load $(SRCROOT)/RustLib/libggterm_ffi.a -lresolv` in Debug.xcconfig and Release.xcconfig
4. **Symbols in Runner.debug.dylib**: In debug builds, the real code is in `Runner.app/Runner.debug.dylib`, not `Runner.app/Runner` (which is a thin stub). 179 ggterm_ symbols confirmed present.
5. **SSH feature optional**: Dart code wraps SSH function lookups in try-catch so the app works without the `ssh` feature compiled in.
6. **ListTile assertion**: Wrap interactive ListTile in `Material(color: Colors.transparent)` when inside a ColoredBox/Container with color. Also use Scaffold backgroundColor instead of Container color.

### Files Created/Modified
- `mobile/ios/` — Full Flutter iOS project structure (flutter create --platforms=ios)
- `mobile/ios/RustLib/libggterm_ffi.a` — Static library (arm64-sim)
- `mobile/ios/RustLib/ggterm_ffi.h` — C header with all FFI function declarations
- `mobile/ios/RustLib/ggterm_ffi.modulemap` — Module map for Xcode
- `mobile/ios/build_rust_ios.sh` — Build script to rebuild Rust for iOS
- `mobile/ios/Flutter/Debug.xcconfig` — Added -force_load and LIBRARY_SEARCH_PATHS
- `mobile/ios/Flutter/Release.xcconfig` — Same
- `mobile/ios/Runner.xcodeproj/project.pbxproj` — Added libggterm_ffi.a to Frameworks phase + OTHER_LDFLAGS + LIBRARY_SEARCH_PATHS
- `mobile/lib/ffi/types.dart` — `final class GGTermCell extends Struct` (was @TypedStruct())
- `mobile/lib/ffi/ffi_bindings.dart` — SSH functions wrapped in try-catch
- `mobile/lib/connection_screen.dart` — Fixed shade950→shade900, power_plug→electrical_services, ListTile Material wrap
- `mobile/lib/terminal_screen.dart` — Fixed _onTapUp argument count
- `mobile/lib/main.dart` — Fixed Future<void> return type for onConnect

### Simulator Info
- Device: iPhone 14 Pro (79DCB17C-E866-4D62-A69D-6C15D2627DBD)
- Runtime: iOS 16.1
- Boot: `xcrun simctl boot 79DCB17C-E866-4D62-A69D-6C15D2627DBD`
- Screenshot: `xcrun simctl io <id> screenshot /tmp/ggterm_ios.png`
- Run: `cd mobile && flutter run -d 79DCB17C-E866-4D62-A69D-6C15D2627DBD --debug`
