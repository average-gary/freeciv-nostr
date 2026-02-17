//! CLI tool for testing and debugging freeciv-nostr networking.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "freeciv-nostr-cli",
    about = "freeciv-nostr debugging and testing tool"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the version of the freeciv-nostr libraries.
    Version,
}

fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Version => {
            println!("freeciv-nostr {}", env!("CARGO_PKG_VERSION"));
        }
    }
}
