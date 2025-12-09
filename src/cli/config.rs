use crate::config::generate::generate_starter_config;
use std::fs;
use std::path::PathBuf;

pub fn init(stdout: bool) -> Result<(), Box<dyn std::error::Error>> {
    let config_content = generate_starter_config();

    if stdout {
        print!("{}", config_content);
        Ok(())
    } else {
        // Try to write to ~/.config/noil/config.yml first
        let config_path = if let Some(home_dir) = dirs::home_dir() {
            let user_config = home_dir.join(".config/noil/config.yml");

            // Create parent directory if it doesn't exist
            if let Some(parent) = user_config.parent() {
                match fs::create_dir_all(parent) {
                    Ok(_) => Some(user_config),
                    Err(_) => {
                        // Fall back to /etc/noil/config.yml
                        eprintln!("Warning: Could not create directory {}", parent.display());
                        eprintln!("Falling back to /etc/noil/config.yml");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let config_path = config_path.unwrap_or_else(|| PathBuf::from("/etc/noil/config.yml"));

        // Check if file already exists
        if config_path.exists() {
            eprintln!(
                "Error: Config file already exists at {}",
                config_path.display()
            );
            eprintln!("Remove it first or use --stdout to print the config");
            std::process::exit(1);
        }

        // Create parent directory for /etc/noil if needed
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the config file
        fs::write(&config_path, config_content)?;

        println!("Config file written to {}", config_path.display());
        Ok(())
    }
}
