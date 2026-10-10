//! Headless RustDesk port-forward CLI.
//! No gtk, scrap, enigo, clipboard, sciter, or pulse.

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
        /// Custom API base (e.g. https://api.example.com:41112)
        #[arg(long)]
        api: Option<String>,
        /// Host header when --api uses a raw IP (fake-ip workaround)
        #[arg(long)]
        api_host: Option<String>,
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
        #[arg(long)]
        api: Option<String>,
        #[arg(long)]
        api_host: Option<String>,
        /// Account username for API login (token goes into PunchHole/RequestRelay)
        #[arg(long)]
        user: Option<String>,
        /// Account password (prefer env RUSTDESK_PF_ACCOUNT_PASSWORD)
        #[arg(long)]
        account_password: Option<String>,
        /// Pre-obtained access_token (skips login)
        #[arg(long)]
        token: Option<String>,
    },
    /// Login to API and store access_token in local config (password never printed).
    Login {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        api: String,
        #[arg(long)]
        api_host: Option<String>,
        #[arg(long)]
        user: String,
        #[arg(long)]
        account_password: Option<String>,
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
            api,
            api_host,
        } => {
            setup::apply(config.as_deref(), server.as_deref(), relay.as_deref(), key.as_deref())?;
            if let Some(a) = api { setup::set_api(&a); }
            if let Some(h) = api_host { setup::set_api_host(&h); }
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
            api,
            api_host,
            user,
            account_password,
            token,
        } => {
            setup::apply(config.as_deref(), server.as_deref(), relay.as_deref(), key.as_deref())?;
            if let Some(a) = api { setup::set_api(&a); }
            if let Some(h) = api_host { setup::set_api_host(&h); }
            if let Some(tok) = token {
                setup::set_access_token(&tok);
            } else {
                let user = user.or_else(|| std::env::var("RUSTDESK_PF_ACCOUNT_USER").ok());
                let pass = account_password
                    .or_else(|| std::env::var("RUSTDESK_PF_ACCOUNT_PASSWORD").ok());
                if let (Some(u), Some(p)) = (user, pass) {
                    let api = hbb_common::config::Config::get_option("api-server");
                    if api.is_empty() {
                        anyhow::bail!("--api (or api-server) required for account login");
                    }
                    account::login(&api, &u, &p).await?;
                }
            }
            let mappings = connect::parse_forwards(&local)?;
            connect::run(&peer_id, mappings, password.as_deref(), force_relay).await
        }
        Cmd::Login {
            config,
            api,
            api_host,
            user,
            account_password,
        } => {
            setup::apply(config.as_deref(), None, None, None)?;
            setup::set_api(&api);
            if let Some(h) = api_host { setup::set_api_host(&h); }
            let pass = account_password
                .or_else(|| std::env::var("RUSTDESK_PF_ACCOUNT_PASSWORD").ok())
                .ok_or_else(|| anyhow::anyhow!("pass --account-password or env RUSTDESK_PF_ACCOUNT_PASSWORD"))?;
            let tok = account::login(&api, &user, &pass).await?;
            println!("login_ok token_len={}", tok.len());
            Ok(())
        }
        Cmd::Id { config } => {
            setup::apply(config.as_deref(), None, None, None)?;
            setup::print_id_info();
            Ok(())
        }
    }
}
