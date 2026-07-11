# GGTerm Makefile

TAGS := desktop,ai,plugin,plugin-lua,config-watch
BINARY := target/release/ggterm

.PHONY: build release test test-ffi test-p2p clippy fmt bundle macos linux windows clean install run ci-ci install-shell-integration

# Debug build
build:
	cargo build --features "$(TAGS)" --bin ggterm

# Run
run:
	cargo run --features "$(TAGS)" --bin ggterm

# Release build (optimized + stripped)
release:
	cargo build --release --features "$(TAGS)" --bin ggterm

# Run tests
test:
	cargo test --features "$(TAGS)" --workspace

# Run FFI tests (with SSH feature for mobile)
test-ffi:
	cargo test -p ggterm-ffi --features ssh --lib

# Run P2P tests
test-p2p:
	cargo test -p ggterm-p2p

# Lint
clippy:
	cargo clippy --features "$(TAGS)" --workspace -- -D warnings

# Format check
fmt:
	cargo fmt --all -- --check

fmt-fix:
	cargo fmt --all

# ── Platform packaging ────────────────────────────────────────────────

# macOS: build .app bundle
# Requires: cargo install cargo-bundle
macos: release
	cargo bundle --release --features "$(TAGS)" --bundle macos
	@echo "App bundle: target/release/bundle/osx/GGTerm.app"

# Linux: build .deb package
# Requires: cargo install cargo-deb
linux: release
	cargo deb --features "$(TAGS)" -p ggterm-app
	@echo "DEB: target/debian/ggterm_*.deb"

# Linux: build AppImage
# Requires: cargo-appimage or linuxdeploy
appimage: release
	@command -v appimagetool >/dev/null 2>&1 || { echo "Install appimagetool first"; exit 1; }
	mkdir -p AppDir/usr/bin
	cp $(BINARY) AppDir/usr/bin/ggterm
	cp assets/ggterm.desktop AppDir/
	cp assets/icon-512.png AppDir/ggterm.png
	ARCH=x86_64 appimagetool AppDir GGTerm-x86_64.AppImage
	@echo "AppImage: GGTerm-x86_64.AppImage"

# Windows: build MSI installer
# Requires: cargo install cargo-wix (run on Windows)
windows: release
	cargo wix --features "$(TAGS)" -p ggterm-app
	@echo "MSI: target/wix/ggterm_*.msi"

# Generic bundle alias
bundle: macos

# Install to system
install: release
	cp $(BINARY) /usr/local/bin/ggterm || sudo cp $(BINARY) /usr/local/bin/ggterm
	mkdir -p /usr/local/share/applications
	cp assets/ggterm.desktop /usr/local/share/applications/ || true
	@echo "Installed: /usr/local/bin/ggterm"

# Install shell integration scripts for detected shells
install-shell-integration:
	@mkdir -p $(HOME)/.config/ggterm/shell
	@cp shell/bash.sh $(HOME)/.config/ggterm/shell/
	@cp shell/zsh.zsh $(HOME)/.config/ggterm/shell/
	@cp shell/fish.fish $(HOME)/.config/ggterm/shell/
	@if [ -f "$(HOME)/.bashrc" ] && ! grep -q "ggterm/shell/bash.sh" "$(HOME)/.bashrc"; then \
		echo "" >> $(HOME)/.bashrc; \
		echo '# GGTerm shell integration' >> $(HOME)/.bashrc; \
		echo 'source $(HOME)/.config/ggterm/shell/bash.sh' >> $(HOME)/.bashrc; \
		echo "Added integration to ~/.bashrc"; \
	fi
	@if [ -f "$(HOME)/.zshrc" ] && ! grep -q "ggterm/shell/zsh.zsh" "$(HOME)/.zshrc"; then \
		echo "" >> $(HOME)/.zshrc; \
		echo '# GGTerm shell integration' >> $(HOME)/.zshrc; \
		echo 'source $(HOME)/.config/ggterm/shell/zsh.zsh' >> $(HOME)/.zshrc; \
		echo "Added integration to ~/.zshrc"; \
	fi
	@if [ -d "$(HOME)/.config/fish" ]; then \
		mkdir -p $(HOME)/.config/fish/conf.d; \
		cp shell/fish.fish $(HOME)/.config/fish/conf.d/ggterm.fish; \
		echo "Installed fish integration to ~/.config/fish/conf.d/ggterm.fish"; \
	fi
	@echo "Shell integration installed. Restart your shell or source the script."

# Clean
clean:
	cargo clean
	rm -rf AppDir *.AppImage
