use clap::{Parser, Subcommand};
use omniport_proto::{RpcClient, SerialConfigDto};

#[derive(Parser, Debug)]
#[command(
    name = "omniport",
    version,
    about = "OmniPort CLI & Headless CI Controller"
)]
struct Cli {
    /// Address of omniportd RPC server
    #[arg(long, default_value = "127.0.0.1:50051", global = true)]
    server: String,

    /// Output responses in raw JSON format
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Query omniportd daemon health and status
    Health,

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

    /// Remove an existing endpoint
    RemoveEndpoint {
        #[arg(long)]
        id: u16,
    },

    /// Establish a routing rule between endpoints
    Route {
        #[arg(long)]
        src: u16,
        #[arg(long, value_delimiter = ',')]
        dst: Vec<u16>,
        #[arg(long, default_value_t = false)]
        bidirectional: bool,
    },

    /// Remove an active route rule
    RemoveRoute {
        #[arg(long)]
        id: u32,
    },

    /// Request omniportd daemon to shut down
    Stop,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let mut client = match RpcClient::connect(&cli.server).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to omniportd at {}: {}", cli.server, e);
            eprintln!("Please ensure 'omniportd' is running.");
            std::process::exit(1);
        }
    };

    match cli.command {
        Commands::Health => {
            let (status, uptime, active_eps, active_routes) = client.health_check().await?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": status,
                        "uptime_secs": uptime,
                        "active_endpoints": active_eps,
                        "active_routes": active_routes
                    })
                );
            } else {
                println!("OmniPort Daemon Health:");
                println!("  Status:           {}", status);
                println!("  Uptime:           {}s", uptime);
                println!("  Active Endpoints: {}", active_eps);
                println!("  Active Routes:    {}", active_routes);
            }
        }

        Commands::List => {
            let endpoints = client.list_endpoints().await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&endpoints)?);
            } else if endpoints.is_empty() {
                println!("No active endpoints registered.");
            } else {
                println!(
                    "{:<6} {:<16} {:<20} {:<10} {:<8}",
                    "ID", "NAME", "TYPE", "BAUD", "ACTIVE"
                );
                println!("{:-<62}", "");
                for ep in endpoints {
                    println!(
                        "{:<6} {:<16} {:<20} {:<10} {:<8}",
                        ep.id, ep.name, ep.port_type, ep.baud_rate, ep.is_active
                    );
                }
            }
        }

        Commands::Pair { a, b, baud } => {
            let cfg = SerialConfigDto {
                baud_rate: baud,
                data_bits: 8,
                stop_bits: 1,
                parity: 0,
            };
            let pair_id = client.create_virtual_pair(&a, &b, cfg).await?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "pair_id": pair_id,
                        "port_a": a,
                        "port_b": b,
                        "baud": baud
                    })
                );
            } else {
                println!(
                    "Successfully allocated virtual pair #{} ({} <-> {} @ {} baud)",
                    pair_id, a, b, baud
                );
            }
        }

        Commands::RemoveEndpoint { id } => {
            client.remove_endpoint(id).await?;
            if cli.json {
                println!("{}", serde_json::json!({ "removed_endpoint_id": id }));
            } else {
                println!("Endpoint {} removed.", id);
            }
        }

        Commands::Route {
            src,
            dst,
            bidirectional,
        } => {
            let route_id = client.add_route(src, dst.clone(), bidirectional).await?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "route_id": route_id,
                        "source": src,
                        "targets": dst,
                        "bidirectional": bidirectional
                    })
                );
            } else {
                println!(
                    "Route #{} created (source {} -> targets {:?}, bidi: {})",
                    route_id, src, dst, bidirectional
                );
            }
        }

        Commands::RemoveRoute { id } => {
            client.remove_route(id).await?;
            if cli.json {
                println!("{}", serde_json::json!({ "removed_route_id": id }));
            } else {
                println!("Route {} removed.", id);
            }
        }

        Commands::Stop => {
            client.shutdown().await?;
            if cli.json {
                println!("{}", serde_json::json!({ "shutdown": true }));
            } else {
                println!("Shutdown signal delivered to omniportd.");
            }
        }
    }

    Ok(())
}
