//! Controller side: local -L listeners and peer tunnel.
//!
//! Milestone 1: real local TCP accept loop; peer punch/relay + login stubbed
//! (next milestone wires PunchHoleRequest / RequestRelay + PortForward login).

use anyhow::{bail, Context, Result};
use hbb_common::config::Config;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Forward {
    pub bind: SocketAddr,
    pub remote_host: String,
    pub remote_port: u16,
}

/// Parse OpenSSH-style `[bind_addr:]lport:host:rport`.
pub fn parse_forwards(specs: &[String]) -> Result<Vec<Forward>> {
    let mut out = Vec::new();
    for s in specs {
        out.push(parse_one(s).with_context(|| format!("bad -L '{s}'"))?);
    }
    Ok(out)
}

fn parse_one(s: &str) -> Result<Forward> {
    // Split from the right: host may contain colons (IPv6) only if bracketed — keep simple for now.
    let parts: Vec<&str> = s.split(':').collect();
    let (bind_ip, lport, host, rport) = match parts.as_slice() {
        [lport, host, rport] => ("127.0.0.1", *lport, *host, *rport),
        [bind, lport, host, rport] => (*bind, *lport, *host, *rport),
        _ => bail!("expected [bind:]lport:host:rport, got {s}"),
    };
    let lport: u16 = lport.parse().context("local port")?;
    let rport: u16 = rport.parse().context("remote port")?;
    let bind: SocketAddr = format!("{bind_ip}:{lport}").parse().context("bind addr")?;
    Ok(Forward {
        bind,
        remote_host: host.to_string(),
        remote_port: rport,
    })
}

pub async fn run(
    peer_id: &str,
    mappings: Vec<Forward>,
    password: Option<&str>,
    force_relay: bool,
) -> Result<()> {
    if mappings.is_empty() {
        bail!("at least one -L mapping required");
    }
    let password = password.unwrap_or("").to_string();
    let rendezvous = Config::get_rendezvous_server();
    log::info!(
        "connect peer={peer_id} rendezvous={rendezvous} force_relay={force_relay} mappings={mappings:?}"
    );
    if password.is_empty() {
        log::warn!("no --password given; login will fail until provided");
    }

    let mut handles = Vec::new();
    for m in mappings {
        let peer_id = peer_id.to_string();
        let password = password.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = listen_one(m, peer_id, password, force_relay).await {
                log::error!("listener ended: {e:#}");
            }
        }));
    }
    // Wait forever (or until all listeners die).
    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

async fn listen_one(
    m: Forward,
    peer_id: String,
    _password: String,
    force_relay: bool,
) -> Result<()> {
    let listener = TcpListener::bind(m.bind).await?;
    log::info!(
        "listening on {} -> {}:{} via peer {peer_id} (force_relay={force_relay})",
        m.bind,
        m.remote_host,
        m.remote_port
    );
    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!(
            "accepted {addr}; peer session for {}:{} not yet implemented — closing",
            m.remote_host,
            m.remote_port
        );
        // TODO: PunchHoleRequest / RequestRelay → secure peer Stream →
        // Hash/LoginRequest{PortForward{host,port}} → forward::run_forward(sock, stream)
        drop(sock);
    }
}
