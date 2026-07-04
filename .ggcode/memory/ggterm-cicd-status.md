## GGTerm CI/CD — COMPLETE (commit 40abd42)

### What Was Done

**3 GitHub Actions workflows:**
1. **ci.yml** (push/PR) — fmt, clippy (default + full features), test (Linux + macOS), FFI test (with ssh), build-release (Linux + macOS Intel/ARM + Windows), Flutter analyze, coverage
2. **release-desktop.yml** (tag v*) — verify gate (fmt+clippy+test) → macOS universal .dmg (lipo aarch64+x86_64), Linux .deb (cargo-deb) + tarball, Windows .zip. Auto-uploads to GitHub Release.
3. **release-mobile.yml** (tag v*) — Android APK (cargo-ndk arm64-v8a + Flutter build), iOS IPA (aarch64-apple-ios-sim + Flutter build no-codesign)

**3 release scripts:**
- `scripts/release/build-macos-app.sh` — .app bundle + .dmg from universal binary
- `scripts/release/build-android-ffi.sh` — Rust FFI .so for Android (arm64/armv7/x86_64)
- `scripts/release/build-ios-ffi.sh` — Rust FFI .a for iOS (sim/device)

**Packaging metadata:**
- `crates/ggterm-app/Cargo.toml`: `[package.metadata.deb]` for cargo-deb (Linux .deb)

**Documentation consolidation:**
- README.md: badges (CI/Release/License/Rust/Platform), download table, architecture diagram with crate LOC, 9 themes, full keyboard shortcuts, CI/CD workflow table, configuration guide

**Credential safety:**
- .gitignore hardened: signing keys (.pem/.key/.p12/.cer), mobileprovision, keystore/jks, key.properties, service-account*.json, build artifacts (.dmg/.deb/.apk/.ipa)
- Scanned all files — no real credentials in repo (only test/mock keys in ggterm-ai tests)

### CI/CD Triggers
- Push to main/PR → ci.yml
- Tag `v*` → release-desktop.yml + release-mobile.yml
- workflow_dispatch with version override on both release workflows

### Environment Variables Pattern (from ggcode reference)
- Uses `${{ secrets.GITHUB_TOKEN }}` (auto-provided by GitHub Actions)
- No custom secrets needed for basic release — only GITHUB_TOKEN for upload
- For signed releases, would need: APPLE_DEVELOPER_ID, APPLE_APP_SPECIFIC_PASSWORD, ANDROID_KEYSTORE (configured in GitHub Settings > Secrets)
