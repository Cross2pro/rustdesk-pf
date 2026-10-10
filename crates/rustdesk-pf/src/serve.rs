//! Headless controlled side: register with hbbs and accept port-forward sessions.
//!
//! Milestone 1: real RegisterPk / RegisterPeer loop + secure_tcp.
//! Incoming PunchHole / RequestRelay are logged; peer session accept is stubbed
//! (next milestone wires create_relay + Hash/Login/PortForward).

use crate::{secure, setup};
use anyhow::{anyhow, bail, Result};
use hbb_common::{
    config::{Config, CONNECT_TIMEOUT, REG_INTERVAL, RENDEZVOUS_PORT},
    protobuf::Message as _,
    rendezvous_proto::*,
    sleep,
    socket_client::{self, connect_tcp},
    tokio::{select, time::interval},
    Stream,
};
use std::time::Instant;

type Message = RendezvousMessage;

pub async fn run() -> Result<()> {
    setup::print_id_info();
    let host = Config::get_rendezvous_server();
    if host.is_empty() {
        bail!("no rendezvous server configured; pass --server host[:port]");
    }
    log::info!("serving; connecting to rendezvous {host}");
    loop {
        match register_loop(&host).await {
            Ok(()) => log::info!("rendezvous loop ended; reconnecting"),
            Err(e) => log::error!("rendezvous error: {e:#}; retry in 3s"),
        }
        sleep(3.).await;
    }
}

async fn register_loop(host: &str) -> Result<()> {
    let host = socket_client::check_port(host, RENDEZVOUS_PORT);
    let mut conn = connect_tcp(host.clone(), CONNECT_TIMEOUT).await?;
    let key = secure::rs_key();
    secure::secure_tcp(&mut conn, &key).await?;

    let host_prefix = host.split('.').next().unwrap_or(&host).to_string();
    Config::set_host_key_confirmed(&host_prefix, false);

    let mut timer = interval(std::time::Duration::from_secs(1));
    let mut last_register;
    let mut last_recv = Instant::now();

    // Immediate register
    send_register(&mut conn, &host_prefix).await?;
    last_register = Instant::now();

    loop {
        select! {
            res = conn.next() => {
                last_recv = Instant::now();
                let bytes = res.ok_or_else(|| anyhow!("rendezvous connection reset"))??;
                if bytes.is_empty() {
                    // heartbeat
                    conn.send_bytes(bytes::Bytes::new()).await?;
                    continue;
                }
                let msg = Message::parse_from_bytes(&bytes)?;
                handle_msg(msg, &mut conn, &host_prefix).await?;
            }
            _ = timer.tick() => {
                if last_recv.elapsed().as_secs() > 90 {
                    bail!("rendezvous idle timeout");
                }
                if last_register.elapsed().as_millis() as i64 >= REG_INTERVAL {
                    send_register(&mut conn, &host_prefix).await?;
                    last_register = Instant::now();
                }
            }
        }
    }
}

async fn send_register(conn: &mut Stream, host_prefix: &str) -> Result<()> {
    if !Config::get_key_confirmed() || !Config::get_host_key_confirmed(host_prefix) {
        let mut msg_out = Message::new();
        let pk = Config::get_key_pair().1;
        let uuid = hbb_common::get_uuid();
        msg_out.set_register_pk(RegisterPk {
            id: Config::get_id(),
            uuid: uuid.into(),
            pk: pk.into(),
            no_register_device: Config::no_register_device(),
            ..Default::default()
        });
        conn.send(&msg_out).await?;
        log::info!("sent RegisterPk id={}", Config::get_id());
        return Ok(());
    }
    let mut msg_out = Message::new();
    msg_out.set_register_peer(RegisterPeer {
        id: Config::get_id(),
        serial: Config::get_serial(),
        ..Default::default()
    });
    conn.send(&msg_out).await?;
    log::debug!("sent RegisterPeer id={}", Config::get_id());
    Ok(())
}

async fn handle_msg(msg: Message, conn: &mut Stream, host_prefix: &str) -> Result<()> {
    match msg.union {
        Some(rendezvous_message::Union::RegisterPkResponse(rpr)) => {
            match rpr.result.enum_value_or_default() {
                register_pk_response::Result::OK => {
                    Config::set_key_confirmed(true);
                    Config::set_host_key_confirmed(host_prefix, true);
                    log::info!("RegisterPk OK; id={} online", Config::get_id());
                    // Follow with RegisterPeer
                    let mut msg_out = Message::new();
                    msg_out.set_register_peer(RegisterPeer {
                        id: Config::get_id(),
                        serial: Config::get_serial(),
                        ..Default::default()
                    });
                    conn.send(&msg_out).await?;
                }
                register_pk_response::Result::UUID_MISMATCH => {
                    log::warn!("UUID_MISMATCH; regenerating id");
                    Config::set_key_confirmed(false);
                    Config::update_id();
                    setup::print_id_info();
                }
                other => {
                    log::error!("RegisterPkResponse: {other:?}");
                }
            }
        }
        Some(rendezvous_message::Union::RegisterPeerResponse(r)) => {
            log::debug!("RegisterPeerResponse request_pk={}", r.request_pk);
            if r.request_pk {
                Config::set_key_confirmed(false);
            }
        }
        Some(rendezvous_message::Union::PunchHole(ph)) => {
            let addr = hbb_common::AddrMangle::decode(&ph.socket_addr);
            log::info!(
                "PunchHole from {:?} relay={} (peer accept not yet implemented in rustdesk-pf)",
                addr,
                ph.relay_server
            );
            // TODO: PunchHoleSent + create_tcp_connection / port-forward accept
        }
        Some(rendezvous_message::Union::RequestRelay(rr)) => {
            let addr = hbb_common::AddrMangle::decode(&rr.socket_addr);
            log::info!(
                "RequestRelay from {:?} uuid={} relay={} (peer accept not yet implemented)",
                addr,
                rr.uuid,
                rr.relay_server
            );
            // TODO: RelayResponse + connect hbbr + Hash/Login/PortForward
        }
        Some(rendezvous_message::Union::FetchLocalAddr(fla)) => {
            let addr = hbb_common::AddrMangle::decode(&fla.socket_addr);
            log::info!(
                "FetchLocalAddr from {:?} (intranet punch; not yet implemented)",
                addr
            );
        }
        Some(rendezvous_message::Union::ConfigureUpdate(cu)) => {
            if !cu.rendezvous_servers.is_empty() {
                log::info!("ConfigureUpdate rendezvous_servers={:?}", cu.rendezvous_servers);
            }
        }
        other => {
            log::debug!("unhandled rendezvous msg: {other:?}");
        }
    }
    Ok(())
}
