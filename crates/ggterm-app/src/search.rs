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
    /// Whether the last next/prev call wrapped around the match list.
    last_wrapped: bool,
    /// Whether the last search was case-insensitive.
    pub case_insensitive: bool,
    /// Whether regex search mode is active.
    pub regex_mode: bool,
    /// History of past search queries (most recent first).
    history: Vec<String>,
    /// Current position in history navigation (None = editing new query).
    history_idx: Option<usize>,
    /// Query text saved before navigating history, for restoring.
    saved_query: String,
    /// Last query when search was closed, for F3 continue-search.
    last_closed_query: String,
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
            last_wrapped: false,
            case_insensitive: true,
            regex_mode: false,
            history: Vec::new(),
            history_idx: None,
            saved_query: String::new(),
            last_closed_query: String::new(),
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
        // Save query for F3 "continue search" before clearing.
        self.last_closed_query = self.query.clone();
        // Save query to history before clearing.
        self.save_to_history();
        self.visible = false;
        self.query.clear();
        self.matches.clear();
        self.current_match = None;
        self.history_idx = None;
    }

    /// Toggle the search bar visibility.
    pub fn toggle(&mut self) {
        if self.visible {
            self.close();
        } else {
            self.open();
        }
    }

    /// Resume search from last closed query (F3 / Shift+F3).
    /// Returns true if a query was restored.
    pub fn resume_from_last(&mut self, grid: &Grid) -> bool {
        if self.last_closed_query.is_empty() {
            return false;
        }
        self.query = self.last_closed_query.clone();
        self.visible = true;
        self.execute_search(grid);
        true
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

    /// Toggle regex search mode and re-execute the search.
    pub fn toggle_regex(&mut self, grid: &Grid) {
        self.regex_mode = !self.regex_mode;
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

    /// Re-execute the current search against the updated grid.
    /// Called when new terminal output arrives while the search panel
    /// is open — keeps match positions in sync with scrollback changes.
    pub fn refresh(&mut self, grid: &Grid) {
        if self.visible && !self.query.is_empty() {
            self.execute_search(grid);
        }
    }

    /// Execute the search across scrollback + visible grid.
    fn execute_search(&mut self, grid: &Grid) {
        self.matches.clear();
        self.current_match = None;

        if self.query.is_empty() {
            return;
        }

        if self.regex_mode {
            self.execute_regex_search(grid);
        } else {
            self.execute_literal_search(grid);
        }

        // Set current match to the first one.
        self.current_match = if self.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Literal substring search.
    fn execute_literal_search(&mut self, grid: &Grid) {
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
    }

    /// Regex search using simple pattern matching (no external dependency).
    /// Supports: . * + ? | ( ) [ ] ^ $ and character classes.
    fn execute_regex_search(&mut self, grid: &Grid) {
        let re = match SimpleRegex::compile(&self.query, self.case_insensitive) {
            Some(r) => r,
            None => return, // Invalid regex — no matches.
        };

        let scrollback_len = grid.scrollback_len();

        // Search scrollback rows.
        for i in 0..scrollback_len {
            if let Some(text) = grid.scrollback_row_text(i) {
                for m in re.find_iter(&text) {
                    self.matches.push(SearchMatch {
                        abs_row: i,
                        col: m.0,
                        len: m.1,
                    });
                }
            }
        }

        // Search visible rows.
        for row in 0..grid.height() {
            if let Some(row_text) = grid.row_text(row) {
                let abs_row = scrollback_len + row;
                for m in re.find_iter(&row_text) {
                    self.matches.push(SearchMatch {
                        abs_row,
                        col: m.0,
                        len: m.1,
                    });
                }
            }
        }
    }

    /// Find all occurrences of the query in a single row's text.
    fn find_in_row(&mut self, text: &str, abs_row: usize, query_lower: &str) {
        // Use Cow to avoid allocation in the case-sensitive path.
        let search_text = if self.case_insensitive {
            std::borrow::Cow::Owned(text.to_lowercase())
        } else {
            std::borrow::Cow::Borrowed(text)
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
        let (idx, wrapped) = match self.current_match {
            Some(i) if i + 1 < self.matches.len() => (i + 1, false),
            Some(_) => (0, true),
            None => (0, false),
        };
        self.current_match = Some(idx);
        self.last_wrapped = wrapped;
        self.matches.get(idx).copied()
    }

    /// Jump to the previous match.
    pub fn prev_match(&mut self) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let (idx, wrapped) = match self.current_match {
            Some(0) | None => (self.matches.len() - 1, true),
            Some(i) => (i - 1, false),
        };
        self.current_match = Some(idx);
        self.last_wrapped = wrapped;
        self.matches.get(idx).copied()
    }

    /// Whether the last next/prev call wrapped around.
    pub fn last_wrapped(&self) -> bool {
        self.last_wrapped
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

    /// Save current query to search history (deduplicated, max 20 entries).
    fn save_to_history(&mut self) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.history.retain(|h| h != &q);
        self.history.insert(0, q);
        self.history.truncate(20);
    }

    /// Navigate to the previous (older) search query in history.
    /// Returns true if history was navigated (query changed).
    pub fn history_prev(&mut self, grid: &Grid) -> bool {
        if self.history.is_empty() {
            return false;
        }
        match self.history_idx {
            None => {
                // Save current query, go to most recent history entry.
                self.saved_query = self.query.clone();
                self.history_idx = Some(0);
            }
            Some(i) => {
                if i + 1 < self.history.len() {
                    self.history_idx = Some(i + 1);
                } else {
                    return false; // Already at oldest entry
                }
            }
        }
        let idx = self.history_idx.unwrap_or(0);
        if idx >= self.history.len() {
            return false;
        }
        self.query = self.history[idx].clone();
        self.execute_search(grid);
        true
    }

    /// Navigate to the next (newer) search query in history.
    /// Returns true if history was navigated.
    pub fn history_next(&mut self, grid: &Grid) -> bool {
        match self.history_idx {
            Some(i) if i > 0 => {
                self.history_idx = Some(i - 1);
                self.query = self.history[i - 1].clone();
                self.execute_search(grid);
                true
            }
            Some(0) => {
                // Restore the query being typed before history navigation.
                self.history_idx = None;
                self.query = self.saved_query.clone();
                self.execute_search(grid);
                true
            }
            _ => false,
        }
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

// ── SimpleRegex ──────────────────────────────────────────────────────────
// A minimal regex engine for terminal scrollback search.
// Supports: . * + ? | ( ) [a-z] [^a-z] ^ $ \d \w \s
// Designed to be dependency-free and sufficient for terminal search use cases.

struct SimpleRegex {
    /// Compiled AST of the pattern.
    pattern: RegexNode,
    /// Case-insensitive matching.
    case_insensitive: bool,
}

/// Regex AST node.
#[derive(Debug, Clone)]
enum RegexNode {
    /// Literal character match.
    Char(char),
    /// Any character (.)
    Any,
    /// Character class ([a-z], [^0-9])
    Class { chars: Vec<char>, negated: bool },
    /// Digit (\d)
    Digit,
    /// Word character (\w)
    Word,
    /// Whitespace (\s)
    Space,
    /// Alternation (a|b|c)
    Alternation(Vec<Vec<RegexNode>>),
    /// Zero or more (x*)
    Star(Box<RegexNode>),
    /// One or more (x+)
    Plus(Box<RegexNode>),
    /// Zero or one (x?)
    Optional(Box<RegexNode>),
    /// Group (...)
    Group(Vec<RegexNode>),
    /// Start of line (^)
    StartAnchor,
    /// End of line ($)
    EndAnchor,
}

impl SimpleRegex {
    /// Compile a regex pattern. Returns None if the pattern is invalid.
    fn compile(pattern: &str, case_insensitive: bool) -> Option<Self> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut parser = RegexParser { chars, pos: 0 };
        let nodes = parser.parse_alternation()?;
        if parser.pos != parser.chars.len() {
            return None; // Unexpected trailing characters.
        }
        Some(Self {
            pattern: RegexNode::Alternation(nodes),
            case_insensitive,
        })
    }

    /// Find all matches in text. Returns (start_col, length) pairs.
    fn find_iter<'a>(&'a self, text: &'a str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = text.chars().collect();
        // Pre-compute byte offsets for each char position to avoid O(n²) scanning.
        let mut byte_offsets = Vec::with_capacity(chars.len() + 1);
        byte_offsets.push(0);
        let mut acc = 0;
        for &c in &chars {
            acc += c.len_utf8();
            byte_offsets.push(acc);
        }

        let mut results = Vec::new();
        let mut start = 0;

        while start <= chars.len() {
            if let Some(len) = self.match_at(&chars, start)
                && len > 0
            {
                results.push((
                    byte_offsets[start],
                    byte_offsets[start + len] - byte_offsets[start],
                ));
                start += len; // Skip past this match.
                continue;
            }
            start += 1;
        }

        results
    }

    /// Try to match the pattern at the given position. Returns match length in chars.
    fn match_at(&self, chars: &[char], start: usize) -> Option<usize> {
        if let RegexNode::Alternation(branches) = &self.pattern {
            for branch in branches {
                if let Some(len) = self.match_nodes(branch, chars, start) {
                    return Some(len);
                }
            }
        }
        None
    }

    /// Match a sequence of nodes against chars starting at pos.
    fn match_nodes(&self, nodes: &[RegexNode], chars: &[char], pos: usize) -> Option<usize> {
        self.match_nodes_impl(nodes, 0, chars, pos)
    }

    fn match_nodes_impl(
        &self,
        nodes: &[RegexNode],
        ni: usize,
        chars: &[char],
        pos: usize,
    ) -> Option<usize> {
        if ni >= nodes.len() {
            return Some(0);
        }

        let node = &nodes[ni];

        match node {
            RegexNode::Star(inner) => {
                let max = self.max_repeats(inner, chars, pos);
                for count in (0..=max).rev() {
                    let mut p = pos;
                    let mut ok = true;
                    for _ in 0..count {
                        if let Some(len) = self.match_single(inner, chars, p) {
                            p += len;
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    if ok && let Some(rest) = self.match_nodes_impl(nodes, ni + 1, chars, p) {
                        return Some(p - pos + rest);
                    }
                }
                None
            }
            RegexNode::Plus(inner) => {
                let max = self.max_repeats(inner, chars, pos);
                for count in (1..=max).rev() {
                    let mut p = pos;
                    let mut ok = true;
                    for _ in 0..count {
                        if let Some(len) = self.match_single(inner, chars, p) {
                            p += len;
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    if ok && let Some(rest) = self.match_nodes_impl(nodes, ni + 1, chars, p) {
                        return Some(p - pos + rest);
                    }
                }
                None
            }
            RegexNode::Optional(inner) => {
                if let Some(len) = self.match_single(inner, chars, pos)
                    && let Some(rest) = self.match_nodes_impl(nodes, ni + 1, chars, pos + len)
                {
                    return Some(len + rest);
                }
                self.match_nodes_impl(nodes, ni + 1, chars, pos)
            }
            _ => match node {
                RegexNode::StartAnchor => {
                    if pos != 0 {
                        return None;
                    }
                    self.match_nodes_impl(nodes, ni + 1, chars, pos)
                }
                RegexNode::EndAnchor => {
                    if pos != chars.len() {
                        return None;
                    }
                    self.match_nodes_impl(nodes, ni + 1, chars, pos)
                }
                _ => {
                    if let Some(len) = self.match_single(node, chars, pos)
                        && let Some(rest) = self.match_nodes_impl(nodes, ni + 1, chars, pos + len)
                    {
                        return Some(len + rest);
                    }
                    None
                }
            },
        }
    }

    /// Match a single node (non-quantified) at pos. Returns match length.
    fn match_single(&self, node: &RegexNode, chars: &[char], pos: usize) -> Option<usize> {
        if pos >= chars.len() {
            return None;
        }
        let c = chars[pos];
        match node {
            RegexNode::Char(expected) => {
                if self.char_eq(*expected, c) {
                    Some(1)
                } else {
                    None
                }
            }
            RegexNode::Any => Some(1),
            RegexNode::Class {
                chars: class,
                negated,
            } => {
                let matched = class.iter().any(|&cc| self.char_eq(cc, c));
                if matched != *negated { Some(1) } else { None }
            }
            RegexNode::Digit => {
                if c.is_ascii_digit() {
                    Some(1)
                } else {
                    None
                }
            }
            RegexNode::Word => {
                if c.is_alphanumeric() || c == '_' {
                    Some(1)
                } else {
                    None
                }
            }
            RegexNode::Space => {
                if c.is_whitespace() {
                    Some(1)
                } else {
                    None
                }
            }
            RegexNode::Group(inner) => self.match_nodes(inner, chars, pos),
            _ => None,
        }
    }

    /// Maximum number of repetitions a quantified node can match.
    fn max_repeats(&self, node: &RegexNode, chars: &[char], start: usize) -> usize {
        let mut count = 0;
        let mut pos = start;
        while pos < chars.len() {
            if let Some(len) = self.match_single(node, chars, pos) {
                pos += len;
                count += 1;
                if count > chars.len() {
                    break;
                }
            } else {
                break;
            }
        }
        count
    }

    fn char_eq(&self, a: char, b: char) -> bool {
        if a == b {
            true
        } else if self.case_insensitive {
            a.eq_ignore_ascii_case(&b)
        } else {
            false
        }
    }
}

/// Simple recursive descent regex parser.
struct RegexParser {
    chars: Vec<char>,
    pos: usize,
}

impl RegexParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// Parse alternation: expr ('|' expr)*
    fn parse_alternation(&mut self) -> Option<Vec<Vec<RegexNode>>> {
        let mut branches = Vec::new();
        let first = self.parse_sequence()?;
        branches.push(first);

        while self.peek() == Some('|') {
            self.next();
            let branch = self.parse_sequence()?;
            branches.push(branch);
        }

        Some(branches)
    }

    /// Parse a sequence of atoms with optional quantifiers.
    fn parse_sequence(&mut self) -> Option<Vec<RegexNode>> {
        let mut nodes = Vec::new();

        loop {
            match self.peek() {
                None | Some(')') | Some('|') => break,
                _ => {}
            }

            let atom = self.parse_atom()?;
            let atom = self.parse_quantifier(atom);
            nodes.push(atom);
        }

        Some(nodes)
    }

    /// Parse a quantifier (*, +, ?) after an atom.
    fn parse_quantifier(&mut self, node: RegexNode) -> RegexNode {
        match self.peek() {
            Some('*') => {
                self.next();
                RegexNode::Star(Box::new(node))
            }
            Some('+') => {
                self.next();
                RegexNode::Plus(Box::new(node))
            }
            Some('?') => {
                self.next();
                RegexNode::Optional(Box::new(node))
            }
            _ => node,
        }
    }

    /// Parse a single atom (char, escape, class, group, anchor).
    fn parse_atom(&mut self) -> Option<RegexNode> {
        let c = self.next()?;

        match c {
            '.' => Some(RegexNode::Any),
            '^' => Some(RegexNode::StartAnchor),
            '$' => Some(RegexNode::EndAnchor),
            '(' => {
                let branches = self.parse_alternation()?;
                if self.next() != Some(')') {
                    return None;
                }
                if branches.len() == 1 {
                    let branch = branches.into_iter().next()?;
                    Some(RegexNode::Group(branch))
                } else {
                    Some(RegexNode::Group(vec![RegexNode::Alternation(branches)]))
                }
            }
            '[' => self.parse_class(),
            '\\' => self.parse_escape(),
            _ => Some(RegexNode::Char(c)),
        }
    }

    /// Parse character class [a-z] or [^a-z].
    fn parse_class(&mut self) -> Option<RegexNode> {
        let mut negated = false;
        let mut chars = Vec::new();

        if self.peek() == Some('^') {
            self.next();
            negated = true;
        }

        while let Some(c) = self.peek() {
            if c == ']' {
                self.next();
                return Some(RegexNode::Class { chars, negated });
            }
            self.next();
            // Handle ranges like a-z.
            if self.peek() == Some('-') {
                let save = self.pos;
                self.next(); // consume '-'
                if let Some(end) = self.peek()
                    && end != ']'
                {
                    self.next();
                    let start = c as u32;
                    let end = end as u32;
                    for code in start..=end {
                        if let Some(ch) = char::from_u32(code) {
                            chars.push(ch);
                        }
                    }
                    continue;
                }
                self.pos = save; // Not a range, restore.
            }
            chars.push(c);
        }

        None // Unterminated class.
    }

    /// Parse escape sequence (\d, \w, \s, \n, \t, \.).
    fn parse_escape(&mut self) -> Option<RegexNode> {
        let c = self.next()?;
        match c {
            'd' => Some(RegexNode::Digit),
            'w' => Some(RegexNode::Word),
            's' => Some(RegexNode::Space),
            'n' => Some(RegexNode::Char('\n')),
            't' => Some(RegexNode::Char('\t')),
            'r' => Some(RegexNode::Char('\r')),
            // Escaped special chars: \. \* \+ \? \\ \( \) \[ \] \^ \$ \|
            _ => Some(RegexNode::Char(c)),
        }
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
    fn t_next_match_wrapped_flag() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        // set_query sets current_match to 0 (first of 2 matches).
        // First next: 0→1, no wrap.
        s.next_match();
        assert!(!s.last_wrapped(), "first next should not wrap");
        // Second next: 1→0, wraps.
        s.next_match();
        assert!(s.last_wrapped(), "second next should wrap");
    }

    #[test]
    fn t_prev_match_wrapped_flag() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.set_query("hello", &g);
        // set_query sets current_match to 0. Prev wraps to last (index 1).
        s.prev_match();
        assert!(s.last_wrapped(), "first prev should wrap");
        // Next prev: 1→0, no wrap.
        s.prev_match();
        assert!(!s.last_wrapped(), "second prev should not wrap");
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

    // ═══════════════════════════════════════════════════════════════
    //  Search history tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn t_history_saved_on_close() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        s.set_query("hello", &g);
        s.close();
        // Internal history is not directly accessible, but we can verify
        // by reopening and navigating history.
        s.open();
        assert!(s.history_prev(&g));
        assert_eq!(s.query, "hello");
    }

    #[test]
    fn t_history_deduplication() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        s.set_query("foo", &g);
        s.close();
        s.open();
        s.set_query("bar", &g);
        s.close();
        s.open();
        s.set_query("foo", &g); // duplicate
        s.close();
        s.open();
        // Navigate history: most recent should be "foo" (moved to front)
        s.history_prev(&g);
        assert_eq!(s.query, "foo");
        s.history_prev(&g);
        assert_eq!(s.query, "bar");
    }

    #[test]
    fn t_history_next_restores_saved_query() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        s.set_query("old_query", &g);
        s.close();
        s.open();
        s.type_char('x', &g);
        assert_eq!(s.query, "x");
        // Go to history, then come back
        s.history_prev(&g);
        assert_eq!(s.query, "old_query");
        s.history_next(&g);
        assert_eq!(s.query, "x"); // restored
    }

    #[test]
    fn t_history_empty_returns_false() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        assert!(!s.history_prev(&g));
        assert!(!s.history_next(&g));
    }

    // ── Regex search tests ──────────────────────────────────────────────

    #[test]
    fn t_regex_simple_literal() {
        let re = SimpleRegex::compile("abc", false).unwrap();
        let text = "xxabcyy";
        let matches: Vec<_> = re.find_iter(text);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, 2); // byte offset
        assert_eq!(matches[0].1, 3); // byte length
    }

    #[test]
    fn t_regex_dot_matches_any() {
        let re = SimpleRegex::compile("a.c", true).unwrap();
        assert_eq!(re.find_iter("abc").len(), 1);
        assert_eq!(re.find_iter("aXc").len(), 1);
        assert_eq!(re.find_iter("ac").len(), 0);
    }

    #[test]
    fn t_regex_star_quantifier() {
        let re = SimpleRegex::compile("ab*c", true).unwrap();
        assert_eq!(re.find_iter("ac").len(), 1);
        assert_eq!(re.find_iter("abc").len(), 1);
        assert_eq!(re.find_iter("abbbc").len(), 1);
    }

    #[test]
    fn t_regex_plus_quantifier() {
        let re = SimpleRegex::compile("ab+c", true).unwrap();
        assert_eq!(re.find_iter("ac").len(), 0);
        assert_eq!(re.find_iter("abc").len(), 1);
        assert_eq!(re.find_iter("abbbc").len(), 1);
    }

    #[test]
    fn t_regex_optional_quantifier() {
        let re = SimpleRegex::compile("colou?r", true).unwrap();
        assert_eq!(re.find_iter("color").len(), 1);
        assert_eq!(re.find_iter("colour").len(), 1);
    }

    #[test]
    fn t_regex_char_class() {
        let re = SimpleRegex::compile("[0-9]+", true).unwrap();
        let matches = re.find_iter("abc123def456");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn t_regex_negated_class() {
        let re = SimpleRegex::compile("[^0-9]+", true).unwrap();
        let matches = re.find_iter("abc123def");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].1, 3); // "abc"
        assert_eq!(matches[1].1, 3); // "def"
    }

    #[test]
    fn t_regex_digit_class() {
        let re = SimpleRegex::compile(r"\d+", true).unwrap();
        let matches = re.find_iter("port 8080 and 443");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn t_regex_word_class() {
        let re = SimpleRegex::compile(r"\w+", true).unwrap();
        let matches = re.find_iter("foo bar_baz");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn t_regex_whitespace_class() {
        let re = SimpleRegex::compile(r"\s+", true).unwrap();
        let matches = re.find_iter("a b  c");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn t_regex_case_insensitive() {
        let re = SimpleRegex::compile("error", true).unwrap();
        assert_eq!(re.find_iter("ERROR here").len(), 1);
        assert_eq!(re.find_iter("Error msg").len(), 1);
    }

    #[test]
    fn t_regex_case_sensitive_no_match() {
        let re = SimpleRegex::compile("error", false).unwrap();
        assert_eq!(re.find_iter("ERROR").len(), 0);
    }

    #[test]
    fn t_regex_alternation() {
        let re = SimpleRegex::compile("cat|dog", true).unwrap();
        assert_eq!(re.find_iter("I have a cat and a dog").len(), 2);
    }

    #[test]
    fn t_regex_invalid_returns_none() {
        assert!(SimpleRegex::compile("[", true).is_none());
        assert!(SimpleRegex::compile("(", true).is_none());
    }

    #[test]
    fn t_regex_escaped_special() {
        let re = SimpleRegex::compile(r"a\.b", true).unwrap();
        assert_eq!(re.find_iter("a.b").len(), 1);
        assert_eq!(re.find_iter("axb").len(), 0);
    }

    #[test]
    fn t_search_state_regex_mode_toggle() {
        let g = make_grid();
        let mut s = SearchState::new();
        assert!(!s.regex_mode);
        s.toggle_regex(&g);
        assert!(s.regex_mode);
        s.toggle_regex(&g);
        assert!(!s.regex_mode);
    }

    #[test]
    fn t_search_state_regex_search_finds() {
        let g = make_grid();
        let mut s = SearchState::new();
        s.open();
        s.regex_mode = true;
        s.set_query("hel.o", &g);
        // "hello" on lines 0 and 2 should match "hel.o"
        assert!(s.match_count() >= 2);
    }

    #[test]
    fn t_search_state_regex_digit_search() {
        let g = make_grid_with_scrollback();
        let mut s = SearchState::new();
        s.open();
        s.regex_mode = true;
        // Make_grid_with_scrollback has "back 1" and "back 2" etc
        // Search for any word with regex \w+
        s.set_query(r"\w+", &g);
        assert!(s.match_count() >= 1);
    }
}
