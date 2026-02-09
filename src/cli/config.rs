use dialoguer::Input;
use std::fs;
use std::path::PathBuf;

pub fn init(stdout: bool, interactive: bool, output_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if interactive {
        let result = crate::cli::interactive::run_interactive(stdout)?;
        return match result.output_path {
            Some(path) => write_config_interactive(&result.yaml, path),
            None => {
                print!("{}", result.yaml);
                Ok(())
            }
        };
    }

    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("sample-config.yml");
    let config_content = fs::read_to_string(&sample_path)
        .map_err(|e| format!("Failed to read sample config: {}", e))?;

    write_config(&config_content, stdout, output_path)
}

fn write_config(config_content: &str, stdout: bool, output_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if stdout {
        print!("{}", config_content);
        return Ok(());
    }

    // If output_path is specified, write directly there without prompting
    if let Some(path) = output_path {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, config_content)?;
        println!("Config file written to {}", path.display());
        return Ok(());
    }

    // Otherwise, use default path resolution and prompt if file exists
    let config_path = resolve_default_config_path()?;

    // Use interactive write to handle prompting
    write_config_interactive(config_content, config_path)
}

fn resolve_default_config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Try to write to ~/.config/noil/config.yml first
    if let Some(home_dir) = dirs::home_dir() {
        let user_config = home_dir.join(".config/noil/config.yml");

        // Create parent directory if it doesn't exist
        if let Some(parent) = user_config.parent() {
            match fs::create_dir_all(parent) {
                Ok(_) => return Ok(user_config),
                Err(_) => {
                    // Fall back to /etc/noil/config.yml
                    eprintln!("Warning: Could not create directory {}", parent.display());
                    eprintln!("Falling back to /etc/noil/config.yml");
                }
            }
        }
    }

    Ok(PathBuf::from("/etc/noil/config.yml"))
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
