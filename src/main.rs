use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "noil")]
#[command(about = "Log correlation system", long_about = None)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Run,
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    Init {
        #[arg(long)]
        stdout: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "noil=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    // Resolve config path
    let config_path = resolve_config_path(cli.config);

    // Dispatch to appropriate handler
    match cli.command {
        Some(Commands::Run) | None => {
            // Default behavior is to run
            noil::cli::run::run(config_path).await?;
        }
        Some(Commands::Config { action }) => match action {
            ConfigAction::Init { stdout } => {
                noil::cli::config::init(stdout)?;
            }
        },
    }

    Ok(())
}

fn resolve_config_path(explicit_path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit_path {
        return Some(path);
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
