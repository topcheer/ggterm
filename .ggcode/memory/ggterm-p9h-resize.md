P9-H Terminal Resize Enhancement complete (commit 7aba84d):
- resize.rs module: CellDims::from_pixels() + ResizeDebouncer (26 tests)
- window.rs: compute_cell_dimensions() with MIN_COLS=10, MIN_ROWS=3 clamping
- handle_resize() defers to apply_pending_resize() with 100ms debounce
- apply_pending_resize() called from about_to_wait()
- Added pending_resize + last_resize_time fields to DesktopApp
- RESIZE_DEBOUNCE_MS=100 constant
- 10 window.rs tests for compute_cell_dimensions + constants
- 1151 total tests, clippy clean, fmt clean