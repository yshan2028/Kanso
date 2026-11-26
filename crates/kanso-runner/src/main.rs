use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kanso-runner", version, about = "Headless remote CI worker for Kanso")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Print runner capability and readiness status
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Status) | None => {
            println!(
                "kanso-runner v{} ready (local/remote CI worker foundation)",
                env!("CARGO_PKG_VERSION")
            );
        }
    }
    Ok(())
}
