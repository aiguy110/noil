pub mod generate;
pub mod parse;
pub mod types;
pub mod version;

use std::path::{Path, PathBuf};

pub use parse::{load_config, ConfigError};
pub use types::{Config, WebConfig};
pub use version::compute_config_version;

/// Resolves the config file path based on explicit argument or default locations.
/// Returns the first existing path from:
/// 1. Explicit path (if provided)
/// 2. ~/.config/noil/config.yml
/// 3. /etc/noil/config.yml
pub fn resolve_config_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
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
