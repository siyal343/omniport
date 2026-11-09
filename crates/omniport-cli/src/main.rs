use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "omniport",
    version,
    about = "OmniPort CLI & Headless CI Runner"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List active virtual and physical endpoints
    List,
    /// Create a virtual null-modem port pair
    Pair {
        #[arg(long)]
        a: String,
        #[arg(long)]
        b: String,
        #[arg(long, default_value_t = 115200)]
        baud: u32,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::List => println!("Querying omniportd endpoints..."),
        Commands::Pair { a, b, baud } => {
            println!("Requesting virtual pair {} <-> {} @ {} baud", a, b, baud);
        }
    }
}
