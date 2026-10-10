//! Hash / LoginRequest(PortForward) / LoginResponse and raw forward.

use crate::{forward, peer};
use anyhow::{bail, Result};
use base::message_proto::*;
use hbb_common::{
    config::{
        decode_permanent_password_h1_from_storage, local_permanent_password_storage_is_usable_for_auth,
        Config, CONNECT_TIMEOUT, READ_TIMEOUT,
    },
    protobuf::Message as _,
    timeout, Stream,
};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn hash_password(plain: &str, salt: &str, challenge: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(plain.as_bytes());
    h.update(salt.as_bytes());
    let h1 = h.finalize();
    let mut h = Sha256::new();
    h.update(&h1);
    h.update(challenge.as_bytes());
    h.finalize().to_vec()
}

fn verify_password(lr_password: &[u8], hash: &Hash) -> bool {
    let (storage, salt) = Config::get_local_permanent_password_storage_and_salt();
    if storage.is_empty()
        || !local_permanent_password_storage_is_usable_for_auth(&storage, &salt)
    {
        log::warn!("no usable permanent password storage");
        return false;
    }
    let Some(h1) = decode_permanent_password_h1_from_storage(&storage) else {
        // legacy plaintext
        let mut h = Sha256::new();
        h.update(storage.as_bytes());
        h.update(hash.salt.as_bytes());
        let h1 = h.finalize();
        let mut h2 = Sha256::new();
        h2.update(&h1);
        h2.update(hash.challenge.as_bytes());
        return constant_time_eq(&h2.finalize()[..], lr_password);
    };
    let mut h2 = Sha256::new();
    h2.update(&h1);
    h2.update(hash.challenge.as_bytes());
    constant_time_eq(&h2.finalize()[..], lr_password)
}

fn tunnel_enabled() -> bool {
    let v = Config::get_option("enable-tunnel");
    v.is_empty() || v == "Y" || v == "true"
}

/// Controlled side after peer stream is secured: Hash → Login → connect target → pipe.
pub async fn run_controlled_port_forward(mut stream: Stream) -> Result<()> {
    let hash = Hash {
        salt: Config::get_effective_permanent_password_salt(),
        challenge: Config::get_auto_password(6),
        ..Default::default()
    };
    let mut msg = Message::new();
    msg.set_hash(hash.clone());
    stream.send(&msg).await?;
    log::info!("sent Hash, waiting for LoginRequest");

    loop {
        let res = timeout(READ_TIMEOUT, stream.next()).await?;
        let bytes = match res {
            Some(Ok(b)) => b,
            Some(Err(e)) => bail!("read before login failed: {e}"),
            None => bail!("peer closed before login"),
        };
        let msg_in = Message::parse_from_bytes(&bytes)?;
        match msg_in.union {
            Some(message::Union::LoginRequest(lr)) => {
                let Some(login_request::Union::PortForward(pf)) = lr.union.as_ref() else {
                    send_login_error(&mut stream, "only PortForward login is supported").await?;
                    bail!("not a port-forward login");
                };
                if !tunnel_enabled() {
                    send_login_error(&mut stream, "tunnel disabled").await?;
                    bail!("enable-tunnel is off");
                }
                if !verify_password(&lr.password, &hash) {
                    send_login_error(&mut stream, "Wrong Password").await?;
                    bail!("password rejected");
                }
                let host = if pf.host.is_empty() {
                    "localhost".to_string()
                } else {
                    pf.host.clone()
                };
                let port = pf.port;
                let addr = format!("{host}:{port}");
                log::info!("authorized port-forward to {addr}");
                let local = match timeout(3000, TcpStream::connect(&addr)).await {
                    Ok(Ok(s)) => s,
                    Ok(Err(e)) => {
                        send_login_error(
                            &mut stream,
                            &format!("Failed to access remote {addr}: {e}"),
                        )
                        .await?;
                        bail!("connect target failed: {e}");
                    }
                    Err(_) => {
                        send_login_error(
                            &mut stream,
                            &format!("Failed to access remote {addr}: timeout"),
                        )
                        .await?;
                        bail!("connect target timeout");
                    }
                };
                // LoginResponse with PeerInfo
                let mut pi = PeerInfo {
                    username: hbb_common::whoami::username(),
                    version: peer::version().to_string(),
                    hostname: hbb_common::whoami::hostname(),
                    platform: hbb_common::whoami::platform().to_string(),
                    ..Default::default()
                };
                pi.features = Some(Features {
                    port_forward_mux: false,
                    ..Default::default()
                })
                .into();
                let mut lr = LoginResponse::new();
                lr.set_peer_info(pi);
                let mut msg = Message::new();
                msg.set_login_response(lr);
                stream.send(&msg).await?;
                stream.set_raw();
                log::info!("port-forward session up → {addr}");
                return forward::run_forward(local, stream).await;
            }
            Some(message::Union::TestDelay(t)) => {
                let mut msg = Message::new();
                msg.set_test_delay(t);
                stream.send(&msg).await?;
            }
            other => log::debug!("ignored pre-login msg: {other:?}"),
        }
    }
}

async fn send_login_error(stream: &mut Stream, err: &str) -> Result<()> {
    let mut lr = LoginResponse::new();
    lr.set_error(err.to_string());
    let mut msg = Message::new();
    msg.set_login_response(lr);
    stream.send(&msg).await?;
    Ok(())
}

/// Controller: wait Hash → LoginRequest(PortForward) → PeerInfo → raw pipe with `local`.
pub async fn run_controller_port_forward(
    mut stream: Stream,
    local: TcpStream,
    password: &str,
    peer_id: &str,
    remote_host: &str,
    remote_port: i32,
) -> Result<()> {
    loop {
        let res = timeout(READ_TIMEOUT, stream.next()).await?;
        let Some(Ok(bytes)) = res else {
            bail!("peer closed before Hash");
        };
        let msg_in = Message::parse_from_bytes(&bytes)?;
        match msg_in.union {
            Some(message::Union::Hash(hash)) => {
                let pw = hash_password(password, &hash.salt, &hash.challenge);
                // Controlled rejects with "Offline" unless username == peer's id.
                let mut lr = LoginRequest {
                    username: peer_id.to_owned(),
                    password: pw.into(),
                    my_id: Config::get_id(),
                    my_name: hbb_common::whoami::username(),
                    my_platform: hbb_common::whoami::platform().to_string(),
                    session_id: hbb_common::rand::random(),
                    version: peer::version().to_string(),
                    ..Default::default()
                };
                lr.set_port_forward(PortForward {
                    host: remote_host.to_string(),
                    port: remote_port,
                    multiplex: false,
                    ..Default::default()
                });
                let mut msg = Message::new();
                msg.set_login_request(lr);
                stream.send(&msg).await?;
                log::info!("sent LoginRequest PortForward {remote_host}:{remote_port}");
            }
            Some(message::Union::LoginResponse(lr)) => match lr.union {
                Some(login_response::Union::Error(err)) => {
                    bail!("login error: {err}");
                }
                Some(login_response::Union::PeerInfo(pi)) => {
                    log::info!(
                        "login ok peer={}@{} ver={}",
                        pi.username,
                        pi.hostname,
                        pi.version
                    );
                    stream.set_raw();
                    return forward::run_forward(local, stream).await;
                }
                _ => {}
            },
            Some(message::Union::TestDelay(t)) => {
                let mut msg = Message::new();
                msg.set_test_delay(t);
                let _ = stream.send(&msg).await;
            }
            other => log::debug!("ignored: {other:?}"),
        }
    }
}

#[allow(dead_code)]
pub async fn connect_timeout_ok() -> Result<()> {
    let _ = CONNECT_TIMEOUT;
    Ok(())
}
