use dialoguer::Input;
use std::fs;
use std::path::PathBuf;

pub fn init(stdout: bool, mode: Option<&str>, interactive: bool) -> Result<(), Box<dyn std::error::Error>> {
    if interactive {
        let result = crate::cli::interactive::run_interactive(mode, stdout)?;
        return match result.output_path {
            Some(path) => write_config_interactive(&result.yaml, path),
            None => {
                print!("{}", result.yaml);
                Ok(())
            }
        };
    }

    // TODO: Phase 6 will remove --mode entirely and use a single unified sample config.
    // For now, map mode values to existing sample files for backwards compatibility.
    let sample_file = match mode.unwrap_or("standalone") {
        "standalone" => "sample-config.yml",
        "collector" => "collector-config.yml",
        "parent" => "parent-config.yml",
        other => {
            return Err(format!(
                "Invalid mode '{}'. Valid modes are: standalone, collector, parent",
                other
            )
            .into());
        }
    };

    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join(sample_file);
    let config_content = fs::read_to_string(&sample_path)
        .map_err(|e| format!("Failed to read sample config: {}", e))?;

    write_config(&config_content, stdout)
}

fn write_config(config_content: &str, stdout: bool) -> Result<(), Box<dyn std::error::Error>> {
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

fn write_config_interactive(config_content: &str, mut path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        // Check if file already exists
        if path.exists() {
            eprintln!("File already exists at {}", path.display());

            let options = &["Overwrite", "Choose a different path", "Print to stdout instead"];
            let choice = dialoguer::Select::new()
                .with_prompt("What would you like to do?")
                .items(options)
                .default(0)
                .interact()?;

            match choice {
                0 => {
                    // Overwrite - fall through to write
                }
                2 => {
                    print!("{}", config_content);
                    return Ok(());
                }
                _ => {
                    let path_str: String = Input::new()
                        .with_prompt("Config file path")
                        .default(path.display().to_string())
                        .interact_text()?;
                    path = PathBuf::from(path_str);
                    continue;
                }
            }
        }

        // Try to create parent dirs and write
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("Cannot create directory {}: {}", parent.display(), e);

                let options = &["Choose a different path", "Print to stdout instead"];
                let choice = dialoguer::Select::new()
                    .with_prompt("What would you like to do?")
                    .items(options)
                    .default(0)
                    .interact()?;

                if choice == 1 {
                    print!("{}", config_content);
                    return Ok(());
                }

                let path_str: String = Input::new()
                    .with_prompt("Config file path")
                    .default(path.display().to_string())
                    .interact_text()?;
                path = PathBuf::from(path_str);
                continue;
            }
        }

        match fs::write(&path, config_content) {
            Ok(()) => {
                println!("Config file written to {}", path.display());
                return Ok(());
            }
            Err(e) => {
                eprintln!("Cannot write to {}: {}", path.display(), e);

                let options = &["Choose a different path", "Print to stdout instead"];
                let choice = dialoguer::Select::new()
                    .with_prompt("What would you like to do?")
                    .items(options)
                    .default(0)
                    .interact()?;

                if choice == 1 {
                    print!("{}", config_content);
                    return Ok(());
                }

                let path_str: String = Input::new()
                    .with_prompt("Config file path")
                    .default(path.display().to_string())
                    .interact_text()?;
                path = PathBuf::from(path_str);
            }
        }
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
