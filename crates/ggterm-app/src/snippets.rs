//! P25-C: Code Snippet Manager
//!
//! Save and organize frequently used terminal commands for quick access.
//! Snippets are persisted to `~/.ggterm/snippets.toml`.
//!
//! ## Configuration File
//! ```toml
//! [[snippets]]
//! name = "list-large-files"
//! command = "find . -type f -size +100M -exec ls -lh {} \\;"
//! description = "Find files larger than 100MB"
//!
//! [[snippets]]
//! name = "kill-port"
//! command = "lsof -ti :{port} | xargs kill -9"
//! description = "Kill process on a port (replace {port})"
//! tags = ["process", "debug"]
//! ```

use std::path::PathBuf;

/// A saved command snippet.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Snippet {
    /// Short name for identification and search.
    pub name: String,
    /// The command to execute. May contain `{placeholder}` variables.
    pub command: String,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional tags for grouping/filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Snippet {
    /// Create a new snippet with name and command.
    pub fn new(name: &str, command: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            description: None,
            tags: Vec::new(),
        }
    }

    /// Set the description.
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = Some(desc.to_string());
        self
    }

    /// Add a tag.
    pub fn with_tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    /// Format display line for overlay: `name — description (tags)`.
    pub fn display_line(&self) -> String {
        let mut line = self.name.clone();
        if let Some(ref desc) = self.description {
            line.push_str(" — ");
            line.push_str(desc);
        }
        if !self.tags.is_empty() {
            line.push_str(&format!(" [{}]", self.tags.join(", ")));
        }
        line
    }

    /// Extract `{placeholder}` variable names from the command string.
    ///
    /// Returns them in order of first appearance. Useful for prompting
    /// the user to fill in values before sending to PTY.
    pub fn placeholders(&self) -> Vec<String> {
        let mut result = Vec::new();
        let mut chars = self.command.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '{' {
                let mut name = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '}' {
                        chars.next();
                        break;
                    }
                    name.push(c);
                    chars.next();
                }
                let trimmed = name.trim().to_string();
                if !trimmed.is_empty() && !result.contains(&trimmed) {
                    result.push(trimmed);
                }
            }
        }
        result
    }

    /// Fill in `{placeholder}` variables in the command string.
    ///
    /// `values` is a map of placeholder name → replacement value.
    /// Returns the command with all placeholders substituted.
    /// Unfilled placeholders remain as `{name}`.
    pub fn fill(&self, values: &std::collections::HashMap<String, String>) -> String {
        let mut result = self.command.clone();
        for key in self.placeholders() {
            if let Some(val) = values.get(&key) {
                result = result.replace(&format!("{{{}}}", key), val);
            }
        }
        result
    }

    /// Fuzzy match against name, command, description, and tags.
    pub fn fuzzy_match(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        let hay = format!(
            "{} {} {} {}",
            self.name,
            self.command,
            self.description.as_deref().unwrap_or(""),
            self.tags.join(" ")
        )
        .to_lowercase();
        hay.contains(&q)
    }
}

/// Persistent store of code snippets.
///
/// Loaded from and saved to `~/.ggterm/snippets.toml`.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SnippetStore {
    /// List of saved snippets.
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

impl SnippetStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the default config file path: `~/.ggterm/snippets.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".ggterm").join("snippets.toml"))
    }

    /// Load from a TOML file. Returns empty store if file doesn't exist.
    pub fn load(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::new(),
        }
    }

    /// Load from the default path.
    pub fn load_default() -> Self {
        Self::load(&Self::default_path().unwrap_or_else(|| PathBuf::from("snippets.toml")))
    }

    /// Save to a TOML file.
    pub fn save(&self, path: &PathBuf) -> Result<(), String> {
        let content = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // Atomic write: write to temp file, then rename.
        // Prevents file corruption if the process is killed mid-write.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &content).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| {
            // Clean up temp file on rename failure.
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        Ok(())
    }

    /// Save to the default path.
    pub fn save_default(&self) -> Result<(), String> {
        let path = Self::default_path().ok_or("HOME not set")?;
        self.save(&path)
    }

    /// Add a snippet.
    pub fn add(&mut self, snippet: Snippet) {
        self.snippets.push(snippet);
    }

    /// Remove a snippet by name. Returns true if removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.snippets.len();
        self.snippets.retain(|s| s.name != name);
        self.snippets.len() < before
    }

    /// Find a snippet by name.
    pub fn find(&self, name: &str) -> Option<&Snippet> {
        self.snippets.iter().find(|s| s.name == name)
    }

    /// Filter snippets by a search query (fuzzy).
    pub fn search(&self, query: &str) -> Vec<&Snippet> {
        self.snippets
            .iter()
            .filter(|s| s.fuzzy_match(query))
            .collect()
    }

    /// Number of saved snippets.
    pub fn len(&self) -> usize {
        self.snippets.len()
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.snippets.is_empty()
    }

    /// Get default starter snippets for new users.
    pub fn defaults() -> Vec<Snippet> {
        vec![
            Snippet::new("ports", "lsof -i :{port}")
                .with_description("Show processes on a port")
                .with_tag("network"),
            Snippet::new("kill-port", "lsof -ti :{port} | xargs kill -9")
                .with_description("Kill process on port")
                .with_tag("network"),
            Snippet::new("git-log", "git log --oneline --graph --all -20")
                .with_description("Compact git log")
                .with_tag("git"),
            Snippet::new("docker-clean", "docker system prune -af")
                .with_description("Remove all unused Docker data")
                .with_tag("docker"),
            Snippet::new("large-files", "du -sh * | sort -rh | head -20")
                .with_description("Find largest files/dirs in cwd")
                .with_tag("disk"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_snippet_new() {
        let s = Snippet::new("test", "echo hello");
        assert_eq!(s.name, "test");
        assert_eq!(s.command, "echo hello");
        assert!(s.description.is_none());
        assert!(s.tags.is_empty());
    }

    #[test]
    fn t_snippet_with_description() {
        let s = Snippet::new("test", "echo hi").with_description("Print greeting");
        assert_eq!(s.description.as_deref(), Some("Print greeting"));
    }

    #[test]
    fn t_snippet_with_tags() {
        let s = Snippet::new("test", "echo hi")
            .with_tag("shell")
            .with_tag("basic");
        assert_eq!(s.tags, vec!["shell", "basic"]);
    }

    #[test]
    fn t_snippet_display_line() {
        let s = Snippet::new("test", "echo hi")
            .with_description("Print greeting")
            .with_tag("shell");
        let line = s.display_line();
        assert!(line.contains("test"));
        assert!(line.contains("Print greeting"));
        assert!(line.contains("shell"));
    }

    #[test]
    fn t_snippet_placeholders_none() {
        let s = Snippet::new("test", "ls -la /tmp");
        assert!(s.placeholders().is_empty());
    }

    #[test]
    fn t_snippet_placeholders_single() {
        let s = Snippet::new("test", "lsof -i :{port}");
        assert_eq!(s.placeholders(), vec!["port"]);
    }

    #[test]
    fn t_snippet_placeholders_multiple() {
        let s = Snippet::new("test", "ssh {user}@{host} -p {port}");
        let phs = s.placeholders();
        assert!(phs.contains(&"user".to_string()));
        assert!(phs.contains(&"host".to_string()));
        assert!(phs.contains(&"port".to_string()));
        assert_eq!(phs.len(), 3);
    }

    #[test]
    fn t_snippet_placeholders_dedup() {
        let s = Snippet::new("test", "{x} {y} {x}");
        assert_eq!(s.placeholders(), vec!["x", "y"]);
    }

    #[test]
    fn t_snippet_placeholders_empty_braces() {
        let s = Snippet::new("test", "echo {} {}");
        assert!(s.placeholders().is_empty());
    }

    #[test]
    fn t_snippet_fill() {
        let s = Snippet::new("test", "lsof -i :{port}");
        let mut values = std::collections::HashMap::new();
        values.insert("port".to_string(), "8080".to_string());
        assert_eq!(s.fill(&values), "lsof -i :8080");
    }

    #[test]
    fn t_snippet_fill_multiple() {
        let s = Snippet::new("test", "ssh {user}@{host} -p {port}");
        let mut values = std::collections::HashMap::new();
        values.insert("user".to_string(), "admin".to_string());
        values.insert("host".to_string(), "10.0.0.1".to_string());
        values.insert("port".to_string(), "2222".to_string());
        assert_eq!(s.fill(&values), "ssh admin@10.0.0.1 -p 2222");
    }

    #[test]
    fn t_snippet_fill_partial() {
        let s = Snippet::new("test", "{a} {b} {c}");
        let mut values = std::collections::HashMap::new();
        values.insert("a".to_string(), "1".to_string());
        // b and c not filled
        let result = s.fill(&values);
        assert_eq!(result, "1 {b} {c}");
    }

    #[test]
    fn t_snippet_fuzzy_match_name() {
        let s = Snippet::new("kill-port", "lsof ...");
        assert!(s.fuzzy_match("kill"));
        assert!(s.fuzzy_match("port"));
    }

    #[test]
    fn t_snippet_fuzzy_match_command() {
        let s = Snippet::new("test", "docker system prune");
        assert!(s.fuzzy_match("docker"));
    }

    #[test]
    fn t_snippet_fuzzy_match_tag() {
        let s = Snippet::new("test", "cmd").with_tag("network");
        assert!(s.fuzzy_match("network"));
    }

    #[test]
    fn t_snippet_fuzzy_match_description() {
        let s = Snippet::new("test", "cmd").with_description("Kill processes");
        assert!(s.fuzzy_match("kill"));
    }

    #[test]
    fn t_snippet_fuzzy_no_match() {
        let s = Snippet::new("test", "ls -la");
        assert!(!s.fuzzy_match("docker-compose-magic-xyz"));
    }

    #[test]
    fn t_snippet_fuzzy_empty() {
        let s = Snippet::new("test", "ls -la");
        assert!(s.fuzzy_match("")); // empty matches all
    }

    #[test]
    fn t_store_new_empty() {
        let store = SnippetStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn t_store_add_remove() {
        let mut store = SnippetStore::new();
        store.add(Snippet::new("a", "cmd-a"));
        store.add(Snippet::new("b", "cmd-b"));
        assert_eq!(store.len(), 2);
        assert!(store.remove("a"));
        assert_eq!(store.len(), 1);
        assert!(!store.remove("nonexistent"));
    }

    #[test]
    fn t_store_find() {
        let mut store = SnippetStore::new();
        store.add(Snippet::new("test", "echo hi"));
        assert!(store.find("test").is_some());
        assert!(store.find("missing").is_none());
    }

    #[test]
    fn t_store_search() {
        let mut store = SnippetStore::new();
        store.add(Snippet::new("git-log", "git log --oneline"));
        store.add(Snippet::new("docker-clean", "docker system prune"));
        store.add(Snippet::new("disk-usage", "du -sh *"));

        let results = store.search("git");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "git-log");
    }

    #[test]
    fn t_store_save_load() {
        let path = std::env::temp_dir().join("ggterm_test_snippets.toml");
        let mut store = SnippetStore::new();
        store.add(Snippet::new("test", "echo hello").with_description("Test snippet"));

        store.save(&path).unwrap();
        let loaded = SnippetStore::load(&path);

        assert_eq!(loaded.len(), 1);
        let s = loaded.find("test").unwrap();
        assert_eq!(s.command, "echo hello");
        assert_eq!(s.description.as_deref(), Some("Test snippet"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn t_store_load_nonexistent() {
        let path = PathBuf::from("/nonexistent/snippets.toml");
        let store = SnippetStore::load(&path);
        assert!(store.is_empty());
    }

    #[test]
    fn t_store_defaults_not_empty() {
        let defaults = SnippetStore::defaults();
        assert!(!defaults.is_empty());
        assert!(defaults.iter().any(|s| s.name == "git-log"));
        assert!(defaults.iter().any(|s| s.name == "docker-clean"));
    }

    #[test]
    fn t_store_defaults_have_placeholders() {
        let defaults = SnippetStore::defaults();
        let kill_port = defaults.iter().find(|s| s.name == "kill-port").unwrap();
        assert!(kill_port.placeholders().contains(&"port".to_string()));
    }
}
