use std::path::PathBuf;

pub async fn run(config_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if config_path.is_none() {
        eprintln!("Error: config not found");
        eprintln!("Searched locations:");
        eprintln!("  ~/.config/noil/config.yml");
        eprintln!("  /etc/noil/config.yml");
        eprintln!("\nUse --config <path> to specify a config file, or run 'noil config init' to generate one.");
        std::process::exit(1);
    }

    let _config_path = config_path.unwrap();

    // TODO: Load config, initialize pipeline, start processing
    todo!("implement run command")
}
