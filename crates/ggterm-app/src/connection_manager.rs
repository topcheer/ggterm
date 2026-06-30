//! P25-A: SSH Connection Manager
//!
//! Manages saved SSH host configurations with TOML persistence.
//! Users can save, search, and quick-connect to frequently used hosts.
//!
//! ## Configuration File
//! Stored at `~/.ggterm/connections.toml`:
//!
//! ```toml
//! [[hosts]]
//! name = "prod-web-1"
//! host = "10.0.0.10"
//! port = 22
//! user = "admin"
//! auth_method = "key"
//! key_path = "~/.ssh/id_rsa"
//!
//! [[hosts]]
//! name = "dev-server"
//! host = "dev.example.com"
//! port = 2222
//! user = "developer"
//! auth_method = "password"
//! ```

use std::path::PathBuf;

/// Authentication method for SSH connections.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    /// Password-based authentication.
    #[default]
    Password,
    /// Public key authentication.
    Key,
}

impl std::fmt::Display for AuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthMethod::Password => write!(f, "password"),
            AuthMethod::Key => write!(f, "key"),
        }
    }
}

/// A saved SSH host configuration.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct HostEntry {
    /// Friendly name for this connection (display + search key).
    pub name: String,
    /// Hostname or IP address.
    pub host: String,
    /// SSH port (default 22).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Username for SSH login.
    pub user: String,
    /// Authentication method.
    #[serde(default)]
    pub auth_method: AuthMethod,
    /// Path to private key file (for key auth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    /// Optional tags for grouping/filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

fn default_port() -> u16 {
    22
}

impl HostEntry {
    /// Create a new host entry with default port 22 and password auth.
    pub fn new(name: &str, host: &str, user: &str) -> Self {
        Self {
            name: name.to_string(),
            host: host.to_string(),
            port: 22,
            user: user.to_string(),
            auth_method: AuthMethod::Password,
            key_path: None,
            tags: Vec::new(),
        }
    }

    /// Set the port.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the auth method to key with a path.
    pub fn with_key(mut self, key_path: &str) -> Self {
        self.auth_method = AuthMethod::Key;
        self.key_path = Some(key_path.to_string());
        self
    }

    /// Set the auth method to password.
    pub fn with_password(mut self) -> Self {
        self.auth_method = AuthMethod::Password;
        self.key_path = None;
        self
    }

    /// Add a tag.
    pub fn with_tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    /// Format as `user@host:port` connection string.
    pub fn connection_string(&self) -> String {
        if self.port == 22 {
            format!("{}@{}", self.user, self.host)
        } else {
            format!("{}@{}:{}", self.user, self.host, self.port)
        }
    }

    /// Format display line for overlay: `name — user@host:port (auth)`.
    pub fn display_line(&self) -> String {
        format!(
            "{} — {} ({})",
            self.name,
            self.connection_string(),
            self.auth_method
        )
    }

    /// Fuzzy match score against a search query.
    /// Matches against name, host, user, and tags.
    pub fn fuzzy_match(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        let hay = format!(
            "{} {} {} {} {}",
            self.name,
            self.host,
            self.user,
            self.tags.join(" "),
            self.tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        hay.contains(&q)
    }
}

/// Persistent store of SSH host configurations.
///
/// Loaded from and saved to `~/.ggterm/connections.toml`.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ConnectionStore {
    /// List of saved hosts.
    #[serde(default)]
    pub hosts: Vec<HostEntry>,
}

impl ConnectionStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the default config file path: `~/.ggterm/connections.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".ggterm").join("connections.toml"))
    }

    /// Load from a TOML file. Returns empty store if file doesn't exist.
    pub fn load(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::new(),
        }
    }

    /// Load from the default path (`~/.ggterm/connections.toml`).
    pub fn load_default() -> Self {
        Self::load(&Self::default_path().unwrap_or_else(|| PathBuf::from("connections.toml")))
    }

    /// Save to a TOML file.
    pub fn save(&self, path: &PathBuf) -> Result<(), String> {
        let content = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, content).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Save to the default path.
    pub fn save_default(&self) -> Result<(), String> {
        let path = Self::default_path().ok_or("HOME not set")?;
        self.save(&path)
    }

    /// Add a host entry.
    pub fn add(&mut self, entry: HostEntry) {
        self.hosts.push(entry);
    }

    /// Remove a host by name. Returns true if removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.hosts.len();
        self.hosts.retain(|h| h.name != name);
        self.hosts.len() < before
    }

    /// Find a host by name.
    pub fn find(&self, name: &str) -> Option<&HostEntry> {
        self.hosts.iter().find(|h| h.name == name)
    }

    /// Find a host by name (mutable).
    pub fn find_mut(&mut self, name: &str) -> Option<&mut HostEntry> {
        self.hosts.iter_mut().find(|h| h.name == name)
    }

    /// Filter hosts by a search query (fuzzy match on name/host/user/tags).
    pub fn search(&self, query: &str) -> Vec<&HostEntry> {
        self.hosts.iter().filter(|h| h.fuzzy_match(query)).collect()
    }

    /// Get all tags used across all hosts (deduplicated, sorted).
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .hosts
            .iter()
            .flat_map(|h| h.tags.iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }

    /// Number of saved hosts.
    pub fn len(&self) -> usize {
        self.hosts.len()
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_host_entry_new() {
        let h = HostEntry::new("dev", "10.0.0.1", "root");
        assert_eq!(h.name, "dev");
        assert_eq!(h.host, "10.0.0.1");
        assert_eq!(h.user, "root");
        assert_eq!(h.port, 22);
        assert_eq!(h.auth_method, AuthMethod::Password);
        assert!(h.key_path.is_none());
    }

    #[test]
    fn t_host_entry_with_port() {
        let h = HostEntry::new("dev", "10.0.0.1", "root").with_port(2222);
        assert_eq!(h.port, 2222);
    }

    #[test]
    fn t_host_entry_with_key() {
        let h = HostEntry::new("dev", "10.0.0.1", "root").with_key("~/.ssh/id_rsa");
        assert_eq!(h.auth_method, AuthMethod::Key);
        assert_eq!(h.key_path.as_deref(), Some("~/.ssh/id_rsa"));
    }

    #[test]
    fn t_host_entry_with_tag() {
        let h = HostEntry::new("dev", "10.0.0.1", "root")
            .with_tag("production")
            .with_tag("web");
        assert_eq!(h.tags, vec!["production", "web"]);
    }

    #[test]
    fn t_host_entry_connection_string_default_port() {
        let h = HostEntry::new("dev", "10.0.0.1", "root");
        assert_eq!(h.connection_string(), "root@10.0.0.1");
    }

    #[test]
    fn t_host_entry_connection_string_custom_port() {
        let h = HostEntry::new("dev", "10.0.0.1", "root").with_port(2222);
        assert_eq!(h.connection_string(), "root@10.0.0.1:2222");
    }

    #[test]
    fn t_host_entry_display_line() {
        let h = HostEntry::new("prod", "prod.example.com", "admin").with_key("~/.ssh/prod_key");
        let line = h.display_line();
        assert!(line.contains("prod"));
        assert!(line.contains("admin@prod.example.com"));
        assert!(line.contains("key"));
    }

    #[test]
    fn t_host_entry_fuzzy_match_name() {
        let h = HostEntry::new("prod-web-1", "10.0.0.10", "admin");
        assert!(h.fuzzy_match("prod"));
        assert!(h.fuzzy_match("web"));
    }

    #[test]
    fn t_host_entry_fuzzy_match_host() {
        let h = HostEntry::new("dev", "dev.example.com", "root");
        assert!(h.fuzzy_match("example"));
    }

    #[test]
    fn t_host_entry_fuzzy_match_tag() {
        let h = HostEntry::new("dev", "10.0.0.1", "root").with_tag("staging");
        assert!(h.fuzzy_match("stag"));
    }

    #[test]
    fn t_host_entry_fuzzy_no_match() {
        let h = HostEntry::new("dev", "10.0.0.1", "root");
        assert!(!h.fuzzy_match("production-server-12345"));
    }

    #[test]
    fn t_host_entry_fuzzy_empty_query() {
        let h = HostEntry::new("dev", "10.0.0.1", "root");
        assert!(h.fuzzy_match("")); // empty matches all
    }

    #[test]
    fn t_connection_store_new_empty() {
        let store = ConnectionStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn t_connection_store_add() {
        let mut store = ConnectionStore::new();
        store.add(HostEntry::new("dev", "10.0.0.1", "root"));
        store.add(HostEntry::new("prod", "prod.example.com", "admin"));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn t_connection_store_remove() {
        let mut store = ConnectionStore::new();
        store.add(HostEntry::new("dev", "10.0.0.1", "root"));
        assert!(store.remove("dev"));
        assert!(store.is_empty());
        assert!(!store.remove("nonexistent"));
    }

    #[test]
    fn t_connection_store_find() {
        let mut store = ConnectionStore::new();
        store.add(HostEntry::new("dev", "10.0.0.1", "root"));
        assert!(store.find("dev").is_some());
        assert!(store.find("prod").is_none());
    }

    #[test]
    fn t_connection_store_find_mut() {
        let mut store = ConnectionStore::new();
        store.add(HostEntry::new("dev", "10.0.0.1", "root"));
        if let Some(h) = store.find_mut("dev") {
            h.port = 2222;
        }
        assert_eq!(store.find("dev").unwrap().port, 2222);
    }

    #[test]
    fn t_connection_store_search() {
        let mut store = ConnectionStore::new();
        store.add(HostEntry::new("dev-server", "10.0.0.1", "root"));
        store.add(HostEntry::new("prod-web", "prod.example.com", "admin"));
        store.add(HostEntry::new("staging", "stage.example.com", "deploy"));

        let results = store.search("dev");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "dev-server");
    }

    #[test]
    fn t_connection_store_search_empty() {
        let mut store = ConnectionStore::new();
        store.add(HostEntry::new("dev", "10.0.0.1", "root"));
        let results = store.search("");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn t_connection_store_all_tags() {
        let mut store = ConnectionStore::new();
        store.add(
            HostEntry::new("dev", "10.0.0.1", "root")
                .with_tag("production")
                .with_tag("web"),
        );
        store.add(
            HostEntry::new("prod", "10.0.0.2", "admin")
                .with_tag("production")
                .with_tag("db"),
        );

        let tags = store.all_tags();
        assert_eq!(tags, vec!["db", "production", "web"]);
    }

    #[test]
    fn t_connection_store_save_load() {
        let path = std::env::temp_dir().join("ggterm_test_connections.toml");
        let mut store = ConnectionStore::new();
        store.add(
            HostEntry::new("dev", "10.0.0.1", "root")
                .with_port(2222)
                .with_key("~/.ssh/id_rsa"),
        );

        store.save(&path).unwrap();
        let loaded = ConnectionStore::load(&path);

        assert_eq!(loaded.len(), 1);
        let h = loaded.find("dev").unwrap();
        assert_eq!(h.host, "10.0.0.1");
        assert_eq!(h.port, 2222);
        assert_eq!(h.auth_method, AuthMethod::Key);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn t_connection_store_load_nonexistent() {
        let path = PathBuf::from("/nonexistent/path/connections.toml");
        let store = ConnectionStore::load(&path);
        assert!(store.is_empty());
    }

    #[test]
    fn t_auth_method_display() {
        assert_eq!(format!("{}", AuthMethod::Password), "password");
        assert_eq!(format!("{}", AuthMethod::Key), "key");
    }

    #[test]
    fn t_auth_method_serde() {
        let h = HostEntry::new("dev", "10.0.0.1", "root").with_key("~/.ssh/id_rsa");
        let toml_str = toml::to_string(&h).unwrap();
        assert!(toml_str.contains("auth_method = \"key\""));

        let parsed: HostEntry = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.auth_method, AuthMethod::Key);
    }
}
