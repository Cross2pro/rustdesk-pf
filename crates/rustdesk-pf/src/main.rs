#![allow(dead_code)]
//! Headless RustDesk port-forward CLI.
//! Reuses hbb_common (protocol / config / crypto) and base::message_proto.
//! No gtk, scrap, enigo, clipboard, sciter, or pulse.

mod connect;
mod secure;
mod forward;
mod serve;
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
        /// Config directory (sets hbb_common APP_DIR). Default: platform config dir for app "rustdesk-pf".
        #[arg(long)]
        config: Option<String>,
        /// Permanent password (stored hashed). If omitted, an existing one is kept or auto-generated.
        #[arg(long)]
        password: Option<String>,
        /// Rendezvous / ID server host[:port] (default port 21116).
        #[arg(long)]
        server: Option<String>,
        /// Relay server host[:port] (optional; hbbs may provide one).
        #[arg(long)]
        relay: Option<String>,
        /// Public key of the ID/relay server (base64).
        #[arg(long)]
        key: Option<String>,
    },
    /// Controller side: open local listeners and tunnel to peer via RustDesk link.
    Connect {
        /// Peer RustDesk ID.
        peer_id: String,
        /// Local forward, OpenSSH-style: [bind:]lport:host:rport (repeatable).
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
        /// Force relay (skip hole punch).
        #[arg(long)]
        force_relay: bool,
    },
    /// Print local ID / password / rendezvous settings.
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
