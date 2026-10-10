//! Headless RustDesk port-forward CLI.
//! No gtk, scrap, enigo, clipboard, sciter, or pulse.

#![allow(dead_code)]

mod connect;
mod forward;
mod peer;
mod secure;
mod serve;
mod session;
mod setup;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "rustdesk-pf", version, about = "Headless RustDesk TCP port-forward CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Headless controlled side: register with hbbs, accept port-forward sessions.
    Serve {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        relay: Option<String>,
        #[arg(long)]
        key: Option<String>,
    },
    /// Controller: local -L listeners tunneled via RustDesk link.
    Connect {
        peer_id: String,
        #[arg(short = 'L', long = "local", required = true)]
        local: Vec<String>,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        relay: Option<String>,
        #[arg(long)]
        key: Option<String>,
        /// Force relay (skip hole punch). Recommended for first tests.
        #[arg(long)]
        force_relay: bool,
    },
    Id {
        #[arg(long)]
        config: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve {
            config,
            password,
            server,
            relay,
            key,
        } => {
            setup::apply(config.as_deref(), server.as_deref(), relay.as_deref(), key.as_deref())?;
            if let Some(p) = password {
                setup::set_password(&p)?;
            } else {
                setup::ensure_password()?;
            }
            setup::enable_tunnel();
            serve::run().await
        }
        Cmd::Connect {
            peer_id,
            local,
            password,
            config,
            server,
            relay,
            key,
            force_relay,
        } => {
            setup::apply(config.as_deref(), server.as_deref(), relay.as_deref(), key.as_deref())?;
            let mappings = connect::parse_forwards(&local)?;
            connect::run(&peer_id, mappings, password.as_deref(), force_relay).await
        }
        Cmd::Id { config } => {
            setup::apply(config.as_deref(), None, None, None)?;
            setup::print_id_info();
            Ok(())
        }
    }
}
