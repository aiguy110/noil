use std::fs;
use std::path::PathBuf;

pub fn init(stdout: bool, mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Determine which sample config to use based on mode
    let config_content = match mode {
        "standalone" => {
            // Read from samples/sample-config.yml
            let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("samples")
                .join("sample-config.yml");
            fs::read_to_string(&sample_path)
                .map_err(|e| format!("Failed to read sample config: {}", e))?
        }
        "collector" => {
            // Read from samples/collector-config.yml
            let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("samples")
                .join("collector-config.yml");
            fs::read_to_string(&sample_path)
                .map_err(|e| format!("Failed to read collector config: {}", e))?
        }
        "parent" => {
            // Read from samples/parent-config.yml
            let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("samples")
                .join("parent-config.yml");
            fs::read_to_string(&sample_path)
                .map_err(|e| format!("Failed to read parent config: {}", e))?
        }
        _ => {
            return Err(format!(
                "Invalid mode '{}'. Valid modes are: standalone, collector, parent",
                mode
            )
            .into());
        }
    };

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

pub fn validate(config_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path.ok_or("No config file found. Use --config to specify a path.")?;

    println!("Validating config file: {}", path.display());

    // Load and validate the config
    match crate::config::load_config(&path) {
        Ok(_) => {
            println!("✓ Config is valid");
            Ok(())
        }
        Err(e) => {
            eprintln!("✗ Config validation failed:\n{}", e);
            std::process::exit(1);
        }
    }
}
