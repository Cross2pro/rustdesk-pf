//! Controller: RequestRelay (and PunchHoleRequest) then Hash/Login/PortForward.

use crate::{peer, secure, session};
use anyhow::{bail, Context, Result};
use hbb_common::{
    config::{Config, LocalConfig, CONNECT_TIMEOUT, READ_TIMEOUT, RELAY_PORT, RENDEZVOUS_PORT},
    protobuf::Message as _,
    rendezvous_proto::*,
    socket_client::{self, connect_tcp},
    Stream,
};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use uuid::Uuid;

type Rmsg = RendezvousMessage;

fn access_token() -> String {
    LocalConfig::get_option("access_token")
}


#[derive(Debug, Clone)]
pub struct Forward {
    pub bind: SocketAddr,
    pub remote_host: String,
    pub remote_port: u16,
}

pub fn parse_forwards(specs: &[String]) -> Result<Vec<Forward>> {
    let mut out = Vec::new();
    for s in specs {
        out.push(parse_one(s).with_context(|| format!("bad -L '{s}'"))?);
    }
    Ok(out)
}

fn parse_one(s: &str) -> Result<Forward> {
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
    if password.is_empty() {
        bail!("--password is required");
    }
    let rendezvous = Config::get_rendezvous_server();
    log::info!(
        "connect peer={peer_id} rendezvous={rendezvous} force_relay={force_relay} mappings={mappings:?}"
    );

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
    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

async fn listen_one(
    m: Forward,
    peer_id: String,
    password: String,
    force_relay: bool,
) -> Result<()> {
    let listener = TcpListener::bind(m.bind).await?;
    log::info!(
        "listening on {} -> {}:{} via peer {peer_id}",
        m.bind,
        m.remote_host,
        m.remote_port
    );
    loop {
        let (sock, addr) = listener.accept().await?;
        log::info!("accepted {addr}; establishing session…");
        let peer_id = peer_id.clone();
        let password = password.clone();
        let host = m.remote_host.clone();
        let port = m.remote_port as i32;
        tokio::spawn(async move {
            match establish(&peer_id, force_relay).await {
                Ok(stream) => {
                    if let Err(e) =
                        session::run_controller_port_forward(stream, sock, &password, &peer_id, &host, port)
                            .await
                    {
                        log::error!("forward session ended: {e:#}");
                    }
                }
                Err(e) => log::error!("establish failed: {e:#}"),
            }
        });
    }
}

async fn establish(peer_id: &str, force_relay: bool) -> Result<Stream> {
    let key = secure::rs_key();
    let mut rendezvous = Config::get_rendezvous_server();
    if rendezvous.is_empty() {
        bail!("no rendezvous server");
    }
    rendezvous = socket_client::check_port(rendezvous, RENDEZVOUS_PORT);

    // Match upstream: even with --force-relay, go through PunchHoleRequest{force_relay=true}
    // so the controlled side runs initiate_relay. Some commercial hbbr builds reject a naked
    // controller RequestRelay with RelayResponse{{refuse_reason=请登录…}}.
    match punch_then_maybe_relay(peer_id, &rendezvous, &key, force_relay).await {
        Ok(s) => Ok(s),
        Err(e) => {
            if force_relay {
                log::warn!("force-relay punch path failed ({e:#}); trying direct RequestRelay");
            } else {
                log::warn!("punch failed ({e:#}); falling back to relay");
            }
            request_relay(peer_id, &rendezvous, &key).await
        }
    }
}

async fn punch_then_maybe_relay(peer_id: &str, rendezvous: &str, key: &str, force_relay: bool) -> Result<Stream> {
    let mut socket = connect_tcp(rendezvous, CONNECT_TIMEOUT).await?;
    let _ = secure::secure_tcp(&mut socket, key).await;

    let mut msg_out = Rmsg::new();
    let token = access_token();
    if token.is_empty() {
        log::warn!("no access_token — custom servers may reject PunchHole (login with --user/--api)");
    }
    msg_out.set_punch_hole_request(PunchHoleRequest {
        id: peer_id.to_owned(),
        token: token.clone(),
        licence_key: key.to_owned(),
        conn_type: ConnType::PORT_FORWARD.into(),
        version: peer::version().to_owned(),
        force_relay,
        ..Default::default()
    });
    socket.send(&msg_out).await?;
    log::info!(
        "sent PunchHoleRequest to {peer_id} token_len={} force_relay={force_relay}",
        token.len()
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        if std::time::Instant::now() > deadline {
            bail!("punch timeout");
        }
        let Some(Ok(bytes)) = socket.next_timeout(READ_TIMEOUT).await else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let msg = Rmsg::parse_from_bytes(&bytes)?;
        match msg.union {
            Some(rendezvous_message::Union::PunchHoleResponse(ph)) => {
                if !ph.other_failure.is_empty() {
                    bail!("punch failure: {}", ph.other_failure);
                }
                // Only treat Failure as error when socket_addr is empty (real failure responses).
                let peer_addr = hbb_common::AddrMangle::decode(&ph.socket_addr);
                if peer_addr.port() == 0 {
                    if let Ok(f) = ph.failure.enum_value() {
                        bail!("punch failure: {f:?}");
                    }
                    bail!("punch response with empty peer addr");
                }
                let signed_pk: Vec<u8> = ph.pk.clone().into();
                let is_local = ph.is_local();
                let timeout_ms = if is_local { 3_000 } else { CONNECT_TIMEOUT };
                log::info!(
                    "PunchHoleResponse peer={peer_addr:?} is_local={is_local} relay={} force_relay={force_relay}",
                    ph.relay_server
                );
                if force_relay {
                    // Upstream would wait for controlled initiate_relay, but some commercial
                    // hbbs strip PunchHole.force_relay (we observe force_relay=false on serve).
                    // A prior PunchHole still "arms" relay on those servers; a naked RequestRelay
                    // without punch is refused with 请登录. So punch first, then RequestRelay.
                    if ph.relay_server.is_empty() {
                        log::info!("force_relay: empty relay in punch response; waiting");
                        continue;
                    }
                    log::info!(
                        "force_relay: punch armed; controller RequestRelay via {}",
                        ph.relay_server
                    );
                    return request_relay_with_server(
                        peer_id, rendezvous, &ph.relay_server, key,
                    )
                    .await;
                }
                match socket_client::connect_tcp_local(peer_addr, None, timeout_ms).await {
                    Ok(mut conn) => {
                        log::info!("direct TCP connected to {peer_addr} (punch/intranet)");
                        peer::secure_connection_controller(peer_id, &signed_pk, key, &mut conn)
                            .await?;
                        return Ok(conn);
                    }
                    Err(e) => {
                        log::warn!("direct connect to {peer_addr} failed: {e:#}");
                        if !ph.relay_server.is_empty() {
                            log::info!("falling back to relay {}", ph.relay_server);
                            return request_relay_with_server(
                                peer_id, rendezvous, &ph.relay_server, key,
                            )
                            .await;
                        }
                        bail!("direct failed and no relay_server in punch response");
                    }
                }
            }
            Some(rendezvous_message::Union::RelayResponse(rr)) => {
                let signed_pk: Vec<u8> = rr.pk().to_vec();
                return finish_relay(peer_id, &rr.relay_server, &rr.uuid, key, &signed_pk).await;
            }
            other => log::debug!("rendezvous: {other:?}"),
        }
    }
}

async fn request_relay(peer_id: &str, rendezvous: &str, key: &str) -> Result<Stream> {
    let relay = {
        let r = Config::get_option("relay-server");
        if r.is_empty() {
            socket_client::increase_port(rendezvous, 1)
        } else {
            r
        }
    };
    request_relay_with_server(peer_id, rendezvous, &relay, key).await
}

async fn request_relay_with_server(
    peer_id: &str,
    rendezvous: &str,
    relay_server: &str,
    key: &str,
) -> Result<Stream> {
    log::info!("connecting to hbbs {rendezvous} for relay…");
    let mut socket = connect_tcp(rendezvous, CONNECT_TIMEOUT).await?;
    log::info!("hbbs tcp up; key-exchange (may skip)…");
    let _ = secure::secure_tcp(&mut socket, key).await;
    let uuid = Uuid::new_v4().to_string();
    let relay_server = socket_client::check_port(relay_server, RELAY_PORT);
    let mut msg_out = Rmsg::new();
    let token = access_token();
    msg_out.set_request_relay(RequestRelay {
        id: peer_id.to_owned(),
        uuid: uuid.clone(),
        relay_server: relay_server.clone(),
        secure: true,
        licence_key: key.to_owned(),
        token: token.clone(),
        conn_type: ConnType::PORT_FORWARD.into(),
        ..Default::default()
    });
    socket.send(&msg_out).await?;
    log::info!("hbbs connected; sent RequestRelay uuid={uuid} relay={relay_server}");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(18);
    let mut signed_pk = Vec::new();
    loop {
        if std::time::Instant::now() > deadline {
            bail!("relay response timeout");
        }
        let Some(Ok(bytes)) = socket.next_timeout(3000).await else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let msg = Rmsg::parse_from_bytes(&bytes)?;
        if let Some(rendezvous_message::Union::RelayResponse(rr)) = msg.union {
            if !rr.refuse_reason.is_empty() {
                bail!("relay refused: {}", rr.refuse_reason);
            }
            signed_pk = rr.pk().to_vec();
            let rs = if rr.relay_server.is_empty() {
                relay_server.clone()
            } else {
                rr.relay_server
            };
            let uid = if rr.uuid.is_empty() {
                uuid.clone()
            } else {
                rr.uuid
            };
            drop(socket);
            return finish_relay(peer_id, &rs, &uid, key, &signed_pk).await;
        }
    }
}

async fn finish_relay(
    peer_id: &str,
    relay_server: &str,
    uuid: &str,
    key: &str,
    signed_pk: &[u8],
) -> Result<Stream> {
    let relay = socket_client::check_port(relay_server, RELAY_PORT);
    log::info!("connecting to relay {relay} uuid={uuid}");
    let mut conn = connect_tcp(relay, CONNECT_TIMEOUT).await?;
    let mut msg_out = Rmsg::new();
    msg_out.set_request_relay(RequestRelay {
        licence_key: key.to_owned(),
        id: peer_id.to_owned(),
        uuid: uuid.to_owned(),
        conn_type: ConnType::PORT_FORWARD.into(),
        ..Default::default()
    });
    log::info!("relay RequestRelay sent licence_key_len={}", key.len());
    conn.send(&msg_out).await?;
    // Some commercial hbbr builds reply with RelayResponse{{refuse_reason}} instead of
    // pairing. That frame is still length-prefixed; parsing it as peer Message yields
    // "Incorrect varint". Peek one frame before the SignedId handshake.
    finish_relay_after_request(peer_id, signed_pk, key, &mut conn).await?;
    Ok(conn)
}

async fn finish_relay_after_request(
    peer_id: &str,
    signed_pk: &[u8],
    key: &str,
    conn: &mut Stream,
) -> Result<()> {
    // Race: commercial hbbr may push RelayResponse{{refuse_reason}}; otherwise the peer
    // sends SignedId. Protobuf is permissive — only treat as refuse when union is
    // RelayResponse; otherwise always try the peer Message path.
    let Some(res) = hbb_common::timeout(READ_TIMEOUT, conn.next()).await? else {
        bail!("relay/peer closed before handshake (no SignedId)");
    };
    let bytes = res?;
    if bytes.is_empty() {
        bail!("empty frame from relay/peer before handshake");
    }
    if let Ok(rmsg) = Rmsg::parse_from_bytes(&bytes) {
        if let Some(rendezvous_message::Union::RelayResponse(rr)) = rmsg.union {
            if !rr.refuse_reason.is_empty() {
                bail!("relay refused: {}", rr.refuse_reason);
            }
            // Empty refuse_reason — ignore and try peer parse below.
            log::debug!("RelayResponse from hbbr without refuse_reason; trying peer handshake");
        }
    }
    peer::secure_connection_controller_with_first(peer_id, signed_pk, key, conn, bytes).await
}
