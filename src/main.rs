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
        #[arg(long, help = "Print config to stdout instead of writing to file")]
        stdout: bool,

        #[arg(long, help = "Interactively configure sources and settings")]
        interactive: bool,

        #[arg(long, help = "Write config to specified path (overwrites if exists)")]
        output_path: Option<PathBuf>,
    },
    Validate,
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
    let config_path = noil::config::resolve_config_path(cli.config.as_deref());

    // Dispatch to appropriate handler
    match cli.command {
        Some(Commands::Run) | None => {
            // Default behavior is to run
            noil::cli::run::run(config_path).await?;
        }
        Some(Commands::Config { action }) => match action {
            ConfigAction::Init { stdout, interactive, output_path } => {
                noil::cli::config::init(stdout, interactive, output_path)?;
            }
            ConfigAction::Validate => {
                noil::cli::config::validate(config_path)?;
            }
        },
    }

    Ok(())
}
