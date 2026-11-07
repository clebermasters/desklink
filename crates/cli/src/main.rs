mod config;
mod runtime;
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use config::{Config, Peer};
use desklink_protocol::Edge;
use desklink_transport::{Identity, parse_public};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Keyboard, mouse and text/image clipboard sharing over authenticated encrypted connections"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Configuration path (identity is stored beside it)"
    )]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a computer identity and configuration in your own user profile.
    Init {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "0.0.0.0:24880")]
        listen: SocketAddr,
    },
    /// Print this computer's public pairing key; never prints the secret key.
    Identity,
    /// Trust a peer's public key, optionally assigning an address and a screen edge.
    Trust {
        public_key: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        edge: Option<Edge>,
    },
    /// Remove a paired computer (restart a running instance to apply changes).
    Untrust { public_key: String },
    /// Check desktop capabilities and show the configuration location without capturing input.
    Doctor,
    /// Show/request the operating system's input permissions.
    Permissions {
        #[arg(long)]
        request: bool,
    },
    /// Start sharing. Ctrl+Alt+Shift+Escape restores local control; Ctrl+C exits.
    Run {
        #[arg(long)]
        no_clipboard: bool,
    },
    /// Print the active configuration file location.
    ConfigPath,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "desklink=info,desklink_transport=info,desklink_clipboard=info".into()
            }),
        )
        .with_writer(std::io::stderr)
        .init();
    if let Err(error) = execute().await {
        eprintln!("desklink: {error:#}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<()> {
    let mut arguments: Vec<_> = std::env::args_os().collect();
    if cfg!(target_os = "macos")
        && arguments.len() == 1
        && std::env::current_exe()
            .is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
    {
        arguments.push("run".into());
    }
    let cli = Cli::parse_from(arguments);
    let path = cli.config.unwrap_or(config::default_path()?);
    match cli.command {
        Command::ConfigPath => println!("{}", path.display()),
        Command::Doctor => {
            let cap = desklink_platform::capabilities();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"platform":cap,"configuration":path,"configured":path.exists(),"clipboard_formats":["text/plain;charset=utf-8","image/png"],"port":desklink_protocol::DEFAULT_PORT,"privileged_helper":false})
                )?
            );
        }
        Command::Permissions { request } => {
            if request {
                desklink_platform::request_permissions()?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&desklink_platform::capabilities())?
            );
        }
        Command::Init { name, listen } => {
            let id_path = config::identity_path(&path);
            let identity = if id_path.exists() {
                Identity::load(&id_path)?
            } else {
                Identity::create(&id_path)?
            };
            if !path.exists() {
                let name = name
                    .or_else(|| std::env::var("HOSTNAME").ok())
                    .unwrap_or_else(|| "My computer".into());
                Config {
                    version: 1,
                    name,
                    listen,
                    clipboard: true,
                    peers: Vec::new(),
                }
                .save(&path)?;
            }
            let config = Config::load(&path)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"name":config.name,"configuration":path,"public_key":hex::encode(identity.public),"clipboard":config.clipboard})
                )?
            );
        }
        Command::Identity => println!(
            "{}",
            hex::encode(Identity::load(&config::identity_path(&path))?.public)
        ),
        Command::Trust {
            public_key,
            name,
            address,
            edge,
        } => {
            let key = parse_public(&public_key)?;
            let identity = Identity::load(&config::identity_path(&path))?;
            ensure!(key != identity.public, "cannot pair a computer with itself");
            let mut config = Config::load(&path)?;
            config
                .peers
                .retain(|p| parse_public(&p.public_key).ok() != Some(key));
            config.peers.push(Peer {
                name: name.clone(),
                public_key: hex::encode(key),
                address,
                edge,
            });
            config.save(&path)?;
            println!("Paired {name}. Restart `desklink run` to apply configuration changes.");
        }
        Command::Untrust { public_key } => {
            let key = parse_public(&public_key)?;
            let mut config = Config::load(&path)?;
            config
                .peers
                .retain(|p| parse_public(&p.public_key).ok() != Some(key));
            config.save(&path)?;
            println!("Peer removed. Restart any running instance to revoke its active connection.");
        }
        Command::Run { no_clipboard } => {
            let mut config = Config::load(&path)?;
            let _profile_lock = config::lock_profile(&path)?;
            if no_clipboard {
                config.clipboard = false;
            }
            let identity = Identity::load(&config::identity_path(&path))
                .context("cannot load computer identity")?;
            runtime::run(config, identity).await?;
        }
    }
    Ok(())
}
