pub mod diff;
pub mod generate;
pub mod parse;
pub mod reconcile;
pub mod types;
pub mod version;

use regex::Regex;
use std::path::{Path, PathBuf};

pub use parse::{load_config, ConfigError};
pub use types::{Config, WebConfig};

/// Expands environment variables in a string.
/// Supports $env{VAR_NAME} syntax.
/// If an environment variable is not set, it's left unchanged.
pub fn expand_env_vars(text: &str) -> String {
    // Pattern matches $env{VAR_NAME} where VAR_NAME starts with letter or underscore,
    // followed by alphanumeric characters or underscores
    let re = Regex::new(r"\$env\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();

    re.replace_all(text, |caps: &regex::Captures| {
        let var_name = caps.get(1).unwrap().as_str();

        // Try to get the environment variable
        std::env::var(var_name).unwrap_or_else(|_| {
            // If not set, return original match unchanged
            caps.get(0).unwrap().as_str().to_string()
        })
    }).to_string()
}

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
    fn test_expand_env_vars_single() {
        std::env::set_var("TEST_VAR", "test_value");
        let result = expand_env_vars("path/$env{TEST_VAR}/file");
        assert_eq!(result, "path/test_value/file");
        std::env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_expand_env_vars_multiple() {
        std::env::set_var("VAR1", "value1");
        std::env::set_var("VAR2", "value2");
        let result = expand_env_vars("$env{VAR1}/middle/$env{VAR2}");
        assert_eq!(result, "value1/middle/value2");
        std::env::remove_var("VAR1");
        std::env::remove_var("VAR2");
    }

    #[test]
    fn test_expand_env_vars_unset() {
        let result = expand_env_vars("path/$env{NONEXISTENT_VAR}/file");
        // Unset variables are left unchanged
        assert_eq!(result, "path/$env{NONEXISTENT_VAR}/file");
    }

    #[test]
    fn test_expand_env_vars_no_expansion() {
        let result = expand_env_vars("plain/path/without/vars");
        assert_eq!(result, "plain/path/without/vars");
    }

    #[test]
    fn test_expand_env_vars_partial() {
        std::env::set_var("SET_VAR", "exists");
        let result = expand_env_vars("$env{SET_VAR}/$env{UNSET_VAR}");
        assert_eq!(result, "exists/$env{UNSET_VAR}");
        std::env::remove_var("SET_VAR");
    }

    #[test]
    fn test_expand_env_vars_ignores_derived_attributes() {
        // ${attr} syntax should NOT be expanded as env vars
        let result = expand_env_vars("${client_ip}:${client_port}");
        assert_eq!(result, "${client_ip}:${client_port}");
    }

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
