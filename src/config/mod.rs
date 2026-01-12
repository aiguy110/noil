pub mod generate;
pub mod parse;
pub mod types;
pub mod version;

use std::path::{Path, PathBuf};

pub use parse::{load_config, ConfigError};
pub use types::{Config, WebConfig};
pub use version::compute_config_version;

/// Expands tilde (~) in paths to the user's home directory.
/// If the path starts with "~/" or is exactly "~", replaces it with the home directory.
/// Returns the path unchanged if it doesn't start with tilde or home directory cannot be determined.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();

    if path_str.starts_with("~/") {
        if let Some(home_dir) = dirs::home_dir() {
            return home_dir.join(&path_str[2..]);
        }
    } else if path_str == "~" {
        if let Some(home_dir) = dirs::home_dir() {
            return home_dir;
        }
    }

    path.to_path_buf()
}

/// Resolves the config file path based on explicit argument or default locations.
/// Returns the first existing path from:
/// 1. Explicit path (if provided, with tilde expansion)
/// 2. ~/.config/noil/config.yml
/// 3. /etc/noil/config.yml
pub fn resolve_config_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        // Expand tilde if present
        return Some(expand_tilde(path));
    }

    // Check ~/.config/noil/config.yml
    if let Some(home_dir) = dirs::home_dir() {
        let user_config = home_dir.join(".config/noil/config.yml");
        if user_config.exists() {
            return Some(user_config);
        }
    }

    // Check /etc/noil/config.yml
    let system_config = PathBuf::from("/etc/noil/config.yml");
    if system_config.exists() {
        return Some(system_config);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_expand_tilde_with_path() {
        let path = Path::new("~/test/path");
        let expanded = expand_tilde(path);

        // Should expand to home directory + test/path
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home.join("test/path"));
        }
    }

    #[test]
    fn test_expand_tilde_alone() {
        let path = Path::new("~");
        let expanded = expand_tilde(path);

        // Should expand to home directory
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home);
        }
    }

    #[test]
    fn test_expand_tilde_no_expansion() {
        let path = Path::new("/absolute/path");
        let expanded = expand_tilde(path);

        // Should not change
        assert_eq!(expanded, Path::new("/absolute/path"));
    }

    #[test]
    fn test_expand_tilde_relative_without_tilde() {
        let path = Path::new("relative/path");
        let expanded = expand_tilde(path);

        // Should not change
        assert_eq!(expanded, Path::new("relative/path"));
    }
}
