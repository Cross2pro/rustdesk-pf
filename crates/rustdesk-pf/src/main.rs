//! Headless RustDesk port-forward CLI.
//! No gtk, scrap, enigo, clipboard, sciter, or pulse.
//!
//! Every flag can also come from an environment variable (see `--help`),
//! so a systemd `EnvironmentFile=` can hold secrets instead of the unit/argv.

#![allow(dead_code)]

mod account;
mod connect;
mod forward;
mod peer;
mod secure;
mod serve;
mod session;
mod setup;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "rustdesk-pf", version, about = "Headless RustDesk TCP port-forward CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Rendezvous / relay / API settings shared by all subcommands.
#[derive(Args, Debug, Clone)]
struct Common {
    /// Config directory (ID, keypair, password hash, token live here)
    #[arg(long, env = "RUSTDESK_PF_CONFIG")]
    config: Option<String>,
    /// ID / rendezvous server host[:port] (default port 21116)
    #[arg(long, env = "RUSTDESK_PF_SERVER")]
    server: Option<String>,
    /// Relay server host[:port] (default: ID server port+1)
    #[arg(long, env = "RUSTDESK_PF_RELAY")]
    relay: Option<String>,
    /// Server public key
    #[arg(long, env = "RUSTDESK_PF_KEY")]
    key: Option<String>,
    /// API server base URL, e.g. https://api.example.com:21114
    #[arg(long, env = "RUSTDESK_PF_API")]
    api: Option<String>,
    /// Host header when --api uses a raw IP (fake-ip DNS workaround)
    #[arg(long, env = "RUSTDESK_PF_API_HOST")]
    api_host: Option<String>,
    /// Account username (API login → access_token)
    #[arg(long, env = "RUSTDESK_PF_ACCOUNT_USER")]
    user: Option<String>,
    /// Account password (prefer the env var; never put it on argv in production)
    #[arg(long, env = "RUSTDESK_PF_ACCOUNT_PASSWORD", hide_env_values = true)]
    account_password: Option<String>,
    /// Pre-obtained access_token (skips login)
    #[arg(long, env = "RUSTDESK_PF_TOKEN", hide_env_values = true)]
    token: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Headless controlled side: register with hbbs, accept port-forward sessions.
    Serve {
        #[command(flatten)]
        common: Common,
        /// Permanent device password (what controllers type)
        #[arg(long, env = "RUSTDESK_PF_PASSWORD", hide_env_values = true)]
        password: Option<String>,
    },
    /// Controller: local -L listeners tunneled via RustDesk link.
    Connect {
        peer_id: String,
        /// [bind:]lport:host:rport (host/rport as seen from the peer)
        #[arg(short = 'L', long = "local", required = true)]
        local: Vec<String>,
        #[command(flatten)]
        common: Common,
        /// Peer's device password
        #[arg(long, env = "RUSTDESK_PF_PEER_PASSWORD", hide_env_values = true)]
        password: Option<String>,
        /// Skip hole punching, go straight to relay
        #[arg(long)]
        force_relay: bool,
    },
    /// Login to API and store access_token in the config dir.
    Login {
        #[command(flatten)]
        common: Common,
    },
    /// Print ID and effective settings.
    Id {
        #[command(flatten)]
        common: Common,
    },
}

/// Apply common settings; log in to the API if credentials are given.
async fn apply_common(c: &Common, need_token: bool) -> Result<()> {
    let peer_id = c.server.as_deref();
    setup::apply(c.config.as_deref(), peer_id, c.relay.as_deref(), c.key.as_deref())?;
    if let Some(a) = &c.api {
        setup::set_api(a);
    }
    if let Some(h) = &c.api_host {
        setup::set_api_host(h);
    }
    if let Some(t) = &c.token {
        setup::set_access_token(t);
        return Ok(());
    }
    if let (Some(u), Some(p)) = (&c.user, &c.account_password) {
        let api = hbb_common::config::Config::get_option("api-server");
        if api.is_empty() {
            anyhow::bail!("--api is required for account login");
        }
        account::login(&api, u, p).await?;
    } else if need_token && account::get_token().is_empty() {
        log::warn!("no account token: servers that require login will refuse hole punching");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve { common, password } => {
            apply_common(&common, false).await?;
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
            common,
            password,
            force_relay,
        } => {
            apply_common(&common, true).await?;
            let mappings = connect::parse_forwards(&local)?;
            let peer_id: String = peer_id.chars().filter(|c| !c.is_whitespace()).collect();
            connect::run(&peer_id, mappings, password.as_deref(), force_relay).await
        }
        Cmd::Login { common } => {
            if common.user.is_none() || common.account_password.is_none() {
                anyhow::bail!("login needs --user and --account-password (or env)");
            }
            apply_common(&common, true).await?;
            println!("login_ok token_len={}", account::get_token().len());
            Ok(())
        }
        Cmd::Id { common } => {
            setup::apply(common.config.as_deref(), common.server.as_deref(), common.relay.as_deref(), common.key.as_deref())?;
            setup::print_id_info();
            Ok(())
        }
    }
}
