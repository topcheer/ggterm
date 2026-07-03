// GGTerm FFI Header - C ABI for mobile integration
#pragma once

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

// ── Legacy single-session API ──────────────────────────────────────
void* ggterm_new(size_t cols, size_t rows);
void ggterm_free(void* handle);

// ── Session lifecycle ──────────────────────────────────────────────
uint32_t ggterm_session_create(size_t cols, size_t rows);
void ggterm_session_destroy(uint32_t id);
size_t ggterm_session_count(void);

// ── Terminal operations ────────────────────────────────────────────
void ggterm_session_process_bytes(uint32_t id, const uint8_t* data, size_t len);
void ggterm_session_send_input(uint32_t id, const uint8_t* data, size_t len);
size_t ggterm_session_take_input(uint32_t id, uint8_t* out, size_t max);

// GGTermCell: xterm color packed as u32 (0x00RRGGBB or index<<24|RGB)
typedef struct {
    uint32_t ch;           // Unicode codepoint
    uint16_t fg_color;     // 16-color palette index (0-15) or 0xFFFF for default
    uint16_t bg_color;     // 16-color palette index (0-15) or 0xFFFF for default
    uint16_t flags;        // CellFlags bits
    uint8_t fg_r, fg_g, fg_b;  // Resolved RGB foreground
    uint8_t bg_r, bg_g, bg_b;  // Resolved RGB background
} GGTermCell;

size_t ggterm_session_read_cells(uint32_t id, GGTermCell* cells, size_t max);
void ggterm_session_dimensions(uint32_t id, size_t* cols, size_t* rows);
void ggterm_session_cursor(uint32_t id, size_t* col, size_t* row);
void ggterm_session_resize(uint32_t id, size_t cols, size_t rows);
int ggterm_session_take_bell(uint32_t id);

// ── Transport ──────────────────────────────────────────────────────
size_t ggterm_transport_pump(uint32_t id);
void ggterm_transport_flush(uint32_t id);
int ggterm_transport_is_alive(uint32_t id);

// ── SSH connections ────────────────────────────────────────────────
int ggterm_ssh_connect(uint32_t id, const char* host, uint16_t port,
                       const char* user, const char* password);
int ggterm_ssh_connect_key(uint32_t id, const char* host, uint16_t port,
                           const char* user, const char* key_path);

// ── Echo transport (for testing without SSH server) ────────────────
int ggterm_echo_connect(uint32_t id);

// ── Error reporting ────────────────────────────────────────────────
const char* ggterm_last_error(void);

#ifdef __cplusplus
}
#endif
