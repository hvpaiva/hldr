use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "hldr",
    version,
    about = "Command-line interface for hvpaiva.dev"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the client version
    Version,
}

fn main() {
    match Cli::parse().command {
        Command::Version => println!("hldr {}", hldr_core::VERSION),
    }
}
