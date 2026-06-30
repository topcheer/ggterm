//! Scrollback text search — search through terminal history.
//!
//! Activated by `Ctrl+Shift+F`. Provides forward/backward search through
//! the grid's visible rows and scrollback history.
//!
//! ## Usage
//!
//! 1. `Ctrl+Shift+F` opens the search bar
//! 2. Type to search — first match is highlighted automatically
//! 3. `Enter` / `Ctrl+N` jumps to next match
//! 4. `Shift+Enter` / `Ctrl+P` jumps to previous match
//! 5. `Esc` closes the search bar

use ggterm_core::Grid;

/// Search state for scrollback text search.
#[derive(Debug, Clone)]
pub struct SearchState {
    /// Whether the search bar is visible.
    pub visible: bool,
    /// Current search query string.
    pub query: String,
    /// All matched positions (row, col) in absolute scrollback coordinates.
    /// Row 0 = oldest scrollback row. Row `scrollback_len` = first visible row.
    matches: Vec<SearchMatch>,
    /// Index into `matches` for the currently highlighted match.
    current_match: Option<usize>,
    /// Whether the last search was case-insensitive.
    pub case_insensitive: bool,
}

/// A single search match location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    /// Absolute row index (0 = oldest scrollback, scrollback_len + 0 = first visible row).
    pub abs_row: usize,
    /// Column where the match starts.
    pub col: usize,
    /// Length of the matched text.
    pub len: usize,
}

impl SearchState {
    /// Create a new search state (hidden by default).
    pub fn new() -> Self {
        Self {
            visible: false,
            query: String::new(),
            matches: Vec::new(),
            current_match: None,
            case_insensitive: true,
        }
    }

    /// Open the search bar.
    pub fn open(&mut self) {
        self.visible = true;
        self.query.clear();
        self.matches.clear();
        self.current_match = None;
    }

    /// Close the search bar and clear state.
    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.matches.clear();
        self.current_match = None;
    }

    /// Toggle the search bar visibility.
    pub fn toggle(&mut self) {
        if self.visible {
            self.close();
        } else {
            self.open();
        }
    }

    /// Append a character to the search query and re-search.
    pub fn type_char(&mut self, c: char, grid: &Grid) {
        self.query.push(c);
        self.execute_search(grid);
    }

    /// Toggle case sensitivity and re-execute the search.
    pub fn toggle_case(&mut self, grid: &Grid) {
        self.case_insensitive = !self.case_insensitive;
        if !self.query.is_empty() {
            self.execute_search(grid);
        }
    }

    /// Remove the last character from the query and re-search.
    pub fn backspace(&mut self, grid: &Grid) {
        self.query.pop();
        if self.query.is_empty() {
            self.matches.clear();
            self.current_match = None;
        } else {
            self.execute_search(grid);
        }
    }

    /// Set the query directly and search.
    pub fn set_query(&mut self, query: &str, grid: &Grid) {
        self.query = query.to_string();
        self.execute_search(grid);
    }

    /// Execute the search across scrollback + visible grid.
    fn execute_search(&mut self, grid: &Grid) {
        self.matches.clear();
        self.current_match = None;

        if self.query.is_empty() {
            return;
        }

        let scrollback_len = grid.scrollback_len();
        let query_lower = if self.case_insensitive {
            self.query.to_lowercase()
        } else {
            self.query.clone()
        };

        // Search scrollback rows (oldest to newest).
        for i in 0..scrollback_len {
            let row_text = grid.scrollback_row_text(i);
            if let Some(text) = row_text {
                self.find_in_row(&text, i, &query_lower);
            }
        }

        // Search visible rows.
        for row in 0..grid.height() {
            if let Some(row_text) = grid.row_text(row) {
                let abs_row = scrollback_len + row;
                self.find_in_row(&row_text, abs_row, &query_lower);
            }
        }

        // Set current match to the first one.
        self.current_match = if self.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Find all occurrences of the query in a single row's text.
    fn find_in_row(&mut self, text: &str, abs_row: usize, query_lower: &str) {
        let search_text = if self.case_insensitive {
            text.to_lowercase()
        } else {
            text.to_string()
        };

        let mut start = 0;
        while let Some(pos) = search_text[start..].find(query_lower) {
            let col = start + pos;
            self.matches.push(SearchMatch {
                abs_row,
                col,
                len: query_lower.len(),
            });
            start = col + query_lower.len();
            if start >= search_text.len() {
                break;
            }
        }
    }

    /// Jump to the next match.
    pub fn next_match(&mut self) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let idx = match self.current_match {
            Some(i) => (i + 1) % self.matches.len(),
            None => 0,
        };
        self.current_match = Some(idx);
        self.matches.get(idx).copied()
    }

    /// Jump to the previous match.
    pub fn prev_match(&mut self) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let idx = match self.current_match {
            Some(0) | None => self.matches.len() - 1,
            Some(i) => i - 1,
        };
        self.current_match = Some(idx);
        self.matches.get(idx).copied()
    }

    /// Get the current highlighted match.
    pub fn current(&self) -> Option<SearchMatch> {
        self.current_match
            .and_then(|i| self.matches.get(i).copied())
    }

    /// Number of matches found.
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Index of the current match (0-based), or None.
    pub fn current_index(&self) -> Option<usize> {
        self.current_match
    }

    /// All matches (for highlight rendering).
    pub fn matches(&self) -> &[SearchMatch] {
        &self.matches
    }

    /// Render the search bar status text.
    ///
    /// Format: `Search: <query> [3/10]` or `Search: <query> (no matches)`
    pub fn status_text(&self) -> String {
        if self.matches.is_empty() {
            if self.query.is_empty() {
                "Search: ".to_string()
            } else {
                format!("Search: {} (no matches)", self.query)
            }
        } else if let Some(idx) = self.current_match {
            format!(
                "Search: {} [{}/{}]",
                self.query,
                idx + 1,
                self.matches.len()
            )
        } else {
            format!("Search: {} [{} matches]", self.query, self.matches.len())
        }
    }

    /// Convert an absolute match row to a scrollback display offset.
    ///
    /// Returns `Some(offset)` where offset is how many lines to scroll up
    /// from the bottom of the visible grid.
    pub fn scroll_offset_for_match(&self, m: SearchMatch, grid: &Grid) -> usize {
        let scrollback_len = grid.scrollback_len();
        if m.abs_row >= scrollback_len {
            // Match is in visible area — no scroll needed.
            0
        } else {
            // Scroll so that the match row is visible.
            scrollback_len.saturating_sub(m.abs_row)
        }
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ggterm_core::{Cell, Grid};

    fn make_grid() -> Grid {
        let mut g = Grid::with_scrollback(20, 5, 100);
        // Visible rows:
        // Row 0: "hello world"
        // Row 1: "foo bar baz"
        // Row 2: "hello again"
        // Row 3: "test line"
        // Row 4: "end"
        let row0: Vec<Cell> = "hello world".chars().map(Cell::with_char).collect();
        let row1: Vec<Cell> = "foo bar baz".chars().map(Cell::with_char).collect();
        let row2: Vec<Cell> = "hello again".chars().map(Cell::with_char).collect();
        let row3: Vec<Cell> = "test line".chars().map(Cell::with_char).collect();
        let row4: Vec<Cell> = "end".chars().map(Cell::with_char).collect();
        for (col, cell) in row0.into_iter().enumerate() {
            g[(col, 0)] = cell;
        }
        for (col, cell) in row1.into_iter().enumerate() {
            g[(col, 1)] = cell;
        }
        for (col, cell) in row2.into_iter().enumerate() {
            g[(col, 2)] = cell;
        }
        for (col, cell) in row3.into_iter().enumerate() {
            g[(col, 3)] = cell;
        }
        for (col, cell) in row4.into_iter().enumerate() {
            g[(col, 4)] = cell;
        }
        g
    }

    fn make_grid_with_scrollback() -> Grid {
        let mut g = Grid::with_scrollback(20, 3, 100);
        // Row 0: "ABC"
        g[(0, 0)] = Cell::with_char('A');
        g[(1, 0)] = Cell::with_char('B');
        g[(2, 0)] = Cell::with_char('C');
        g.scroll_up(1); // "ABC" -> scrollback[0]

        // Row 0: "XYZ"
        g[(0, 0)] = Cell::with_char('X');
        g[(1, 0)] = Cell::with_char('Y');
        g[(2, 0)] = Cell::with_char('Z');
        g.scroll_up(1); // "XYZ" -> scrollback[1]

        // Now visible:
        // Row 0: "foo"
        g[(0, 0)] = Cell::with_char('f');
        g[(1, 0)] = Cell::with_char('o');
        g[(2, 0)] = Cell::with_char('o');
        // Row 1: "bar"
        g[(0, 1)] = Cell::with_char('b');
        g[(1, 1)] = Cell::with_char('a');
        g[(2, 1)] = Cell::with_char('r');
        // Row 2: "baz"
        g[(0, 2)] = Cell::with_char('b');
        g[(1, 2)] = Cell::with_char('a');
        g[(2, 2)] = Cell::with_char('z');
        g
    }

    #[test]
    fn t_search_initial_state() {
        let s = SearchState::new();
        assert!(!s.visible);
        assert!(s.query.is_empty());
        assert_eq!(s.match_count(), 0);
        assert!(s.current().is_none());
    }

    #[test]
    fn t_open_close() {
        let mut s = SearchState::new();
        s.open();
        assert!(s.visible);
        s.close();
        assert!(!s.visible);
    }

    #[test]
    fn t_toggle() {
        let mut s = SearchState::new();
        assert!(!s.visible);
        s.toggle();
        assert!(s.visible);
        s.toggle();
        assert!(!s.visible);
    }

    #[test]
    fn t_simple_search() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        assert_eq!(s.match_count(), 2); // "hello world" + "hello again"
    }

    #[test]
    fn t_search_no_match() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("xyz", &g);
        assert_eq!(s.match_count(), 0);
        assert!(s.current().is_none());
    }

    #[test]
    fn t_search_case_insensitive() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("HELLO", &g);
        assert_eq!(s.match_count(), 2); // case insensitive by default
    }

    #[test]
    fn t_search_case_sensitive() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.case_insensitive = false;
        s.set_query("HELLO", &g);
        assert_eq!(s.match_count(), 0); // no uppercase HELLO in "hello world\nhello world"
    }

    #[test]
    fn t_toggle_case_sensitivity() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("HELLO", &g);
        assert_eq!(s.match_count(), 2); // case insensitive
        s.toggle_case(&g); // → case sensitive
        assert!(!s.case_insensitive);
        assert_eq!(s.match_count(), 0); // no uppercase match
        s.toggle_case(&g); // → back to insensitive
        assert!(s.case_insensitive);
        assert_eq!(s.match_count(), 2);
    }

    #[test]
    fn t_current_index_tracking() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        assert_eq!(s.match_count(), 2);
        assert_eq!(s.current_index(), Some(0));
        s.next_match();
        assert_eq!(s.current_index(), Some(1));
    }

    #[test]
    fn t_next_prev_match() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        assert_eq!(s.match_count(), 2);

        let m1 = s.current().unwrap();
        let m2 = s.next_match().unwrap();
        assert_ne!(m1.abs_row, m2.abs_row);

        // Wrap around back to first match.
        let m3 = s.next_match().unwrap();
        assert_eq!(m3.abs_row, m1.abs_row);

        // Go back.
        let m4 = s.prev_match().unwrap();
        assert_eq!(m4.abs_row, m2.abs_row);
    }

    #[test]
    fn t_search_empty_query() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("", &g);
        assert_eq!(s.match_count(), 0);
    }

    #[test]
    fn t_type_and_backspace() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        s.type_char('f', &g);
        s.type_char('o', &g);
        s.type_char('o', &g);
        assert_eq!(s.match_count(), 1); // "foo bar baz"
        assert_eq!(s.query, "foo");

        s.backspace(&g);
        assert_eq!(s.query, "fo");
        s.backspace(&g);
        assert_eq!(s.query, "f");
        s.backspace(&g);
        assert!(s.query.is_empty());
        assert_eq!(s.match_count(), 0);
    }

    #[test]
    fn t_multiple_matches_per_row() {
        let mut g = Grid::new(20, 2);
        // Row 0: "ab ab ab"
        for (i, ch) in "ab ab ab".chars().enumerate() {
            g[(i, 0)] = Cell::with_char(ch);
        }
        let mut s = SearchState::new();
        s.set_query("ab", &g);
        assert_eq!(s.match_count(), 3); // 3 "ab" in one row
    }

    #[test]
    fn t_search_with_scrollback() {
        let g = make_grid_with_scrollback();
        let mut s = SearchState::new();
        s.set_query("foo", &g);
        assert_eq!(s.match_count(), 1); // visible row 0 "foo"
    }

    #[test]
    fn t_status_text_no_matches() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        s.set_query("xyz", &g);
        assert_eq!(s.status_text(), "Search: xyz (no matches)");
    }

    #[test]
    fn t_status_text_with_matches() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        assert_eq!(s.status_text(), "Search: hello [1/2]");
    }

    #[test]
    fn t_status_text_empty_query() {
        let s = SearchState::new();
        assert_eq!(s.status_text(), "Search: ");
    }

    #[test]
    fn t_scroll_offset_for_visible_match() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        let m = s.current().unwrap();
        let offset = s.scroll_offset_for_match(m, &g);
        assert_eq!(offset, 0); // visible row, no scroll needed
    }

    #[test]
    fn t_scroll_offset_for_scrollback_match() {
        let g = make_grid_with_scrollback();
        // scrollback has "ABC" at row 0, "XYZ" at row 1
        let mut s = SearchState::new();
        s.set_query("abc", &g);
        assert_eq!(s.match_count(), 1);
        let m = s.current().unwrap();
        let offset = s.scroll_offset_for_match(m, &g);
        assert!(offset > 0); // need to scroll up to see it
    }

    #[test]
    fn t_next_match_wrap_around() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        let first = s.current().unwrap();
        // Go to next (last match)
        s.next_match();
        // Next again should wrap to first
        let wrapped = s.next_match().unwrap();
        assert_eq!(wrapped.abs_row, first.abs_row);
        assert_eq!(wrapped.col, first.col);
    }

    #[test]
    fn t_prev_match_wrap_around() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        // Prev from first should go to last
        let prev = s.prev_match().unwrap();
        assert_eq!(prev.abs_row, s.matches[1].abs_row);
    }

    #[test]
    fn t_close_clears_everything() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        s.set_query("hello", &g);
        assert_eq!(s.match_count(), 2);
        s.close();
        assert!(!s.visible);
        assert!(s.query.is_empty());
        assert_eq!(s.match_count(), 0);
    }
}
