//! Headless controlled side: UDP RegisterPk/RegisterPeer, accept RequestRelay / PunchHole.

use crate::{peer, secure, session, setup};
use anyhow::{bail, Result};
use hbb_common::{
    config::{Config, CONNECT_TIMEOUT, REG_INTERVAL, RELAY_PORT, RENDEZVOUS_PORT},
    protobuf::Message as _,
    rendezvous_proto::*,
    sleep,
    socket_client::{self, connect_tcp, new_udp_for},
    tcp::new_listener,
    tokio::{select, time::interval},
    AddrMangle, Stream,
};
use std::time::Instant;

type Message = RendezvousMessage;

pub async fn run() -> Result<()> {
    setup::print_id_info();
    // Ensure keypair exists before first RegisterPk.
    let _ = Config::get_key_pair();
    let host = Config::get_rendezvous_server();
    if host.is_empty() {
        bail!("no rendezvous server; pass --server host[:port]");
    }
    log::info!("serving via UDP register to {host}");
    loop {
        match udp_register_loop(&host).await {
            Ok(()) => log::info!("register loop ended; reconnecting"),
            Err(e) => log::error!("register error: {e:#}; retry in 3s"),
        }
        sleep(3.).await;
    }
}

async fn udp_register_loop(host: &str) -> Result<()> {
    let host = socket_client::check_port(host, RENDEZVOUS_PORT);
    let (mut socket, addr) = new_udp_for(&host, CONNECT_TIMEOUT).await?;
    let host_prefix = host.split('.').next().unwrap_or(&host).to_string();
    Config::set_host_key_confirmed(&host_prefix, false);

    let mut timer = interval(std::time::Duration::from_secs(1));
    let mut last_register = Instant::now() - std::time::Duration::from_secs(3600);
    let mut last_resp = Instant::now() - std::time::Duration::from_secs(3600);

    send_register_udp(&mut socket, &addr, &host_prefix).await?;
    last_register = Instant::now();

    loop {
        select! {
            n = socket.next() => {
                match n {
                    Some(Ok((bytes, _from))) => {
                        if let Ok(msg) = Message::parse_from_bytes(&bytes) {
                            handle_udp(msg, &mut socket, &addr, &host, &host_prefix, &mut last_resp).await?;
                        }
                    }
                    Some(Err(e)) => bail!("udp recv: {e}"),
                    None => bail!("udp socket closed"),
                }
            }
            _ = timer.tick() => {
                if last_register.elapsed().as_millis() as i64 >= REG_INTERVAL
                    || last_resp.elapsed().as_millis() as i64 >= REG_INTERVAL
                {
                    send_register_udp(&mut socket, &addr, &host_prefix).await?;
                    last_register = Instant::now();
                }
            }
        }
    }
}

async fn send_register_udp(
    socket: &mut hbb_common::udp::FramedSocket,
    addr: &hbb_common::TargetAddr<'_>,
    host_prefix: &str,
) -> Result<()> {
    if !Config::get_key_confirmed() || !Config::get_host_key_confirmed(host_prefix) {
        let mut msg_out = Message::new();
        let pk = Config::get_key_pair().1;
        let uuid = hbb_common::get_uuid();
        if pk.is_empty() || uuid.is_empty() {
            bail!("empty pk or uuid");
        }
        msg_out.set_register_pk(RegisterPk {
            id: Config::get_id(),
            uuid: uuid.into(),
            pk: pk.into(),
            no_register_device: Config::no_register_device(),
            ..Default::default()
        });
        socket.send(&msg_out, addr.to_owned()).await?;
        log::info!("UDP RegisterPk id={}", Config::get_id());
        return Ok(());
    }
    let mut msg_out = Message::new();
    msg_out.set_register_peer(RegisterPeer {
        id: Config::get_id(),
        serial: Config::get_serial(),
        ..Default::default()
    });
    socket.send(&msg_out, addr.to_owned()).await?;
    log::debug!("UDP RegisterPeer id={}", Config::get_id());
    Ok(())
}

async fn handle_udp(
    msg: Message,
    socket: &mut hbb_common::udp::FramedSocket,
    addr: &hbb_common::TargetAddr<'_>,
    host: &str,
    host_prefix: &str,
    last_resp: &mut Instant,
) -> Result<()> {
    match msg.union {
        Some(rendezvous_message::Union::RegisterPkResponse(rpr)) => {
            *last_resp = Instant::now();
            match rpr.result.enum_value_or_default() {
                register_pk_response::Result::OK => {
                    Config::set_key_confirmed(true);
                    Config::set_host_key_confirmed(host_prefix, true);
                    log::info!("RegisterPk OK; id={} online", Config::get_id());
                    let mut msg_out = Message::new();
                    msg_out.set_register_peer(RegisterPeer {
                        id: Config::get_id(),
                        serial: Config::get_serial(),
                        ..Default::default()
                    });
                    socket.send(&msg_out, addr.to_owned()).await?;
                }
                register_pk_response::Result::UUID_MISMATCH => {
                    log::warn!("UUID_MISMATCH; regenerating id");
                    Config::set_key_confirmed(false);
                    Config::update_id();
                    let _ = Config::get_key_pair();
                    setup::print_id_info();
                    // Re-register immediately rather than waiting for REG_INTERVAL.
                    send_register_udp(socket, addr, host_prefix).await?;
                }
                other => log::error!("RegisterPkResponse: {other:?}"),
            }
        }
        Some(rendezvous_message::Union::RegisterPeerResponse(r)) => {
            *last_resp = Instant::now();
            if r.request_pk {
                Config::set_key_confirmed(false);
                send_register_udp(socket, addr, host_prefix).await?;
            } else {
                log::debug!("RegisterPeerResponse ok");
            }
        }
        Some(rendezvous_message::Union::RequestRelay(rr)) => {
            let peer = AddrMangle::decode(&rr.socket_addr);
            log::info!(
                "RequestRelay from {:?} uuid={} relay={}",
                peer,
                rr.uuid,
                rr.relay_server
            );
            let host = host.to_string();
            let key = secure::rs_key();
            tokio::spawn(async move {
                if let Err(e) = accept_relay(host, rr, key).await {
                    log::error!("accept_relay failed: {e:#}");
                }
            });
        }
        Some(rendezvous_message::Union::PunchHole(ph)) => {
            let peer = AddrMangle::decode(&ph.socket_addr);
            log::info!(
                "PunchHole from {:?} relay={} force_relay={}",
                peer,
                ph.relay_server,
                ph.force_relay
            );
            let host = host.to_string();
            let key = secure::rs_key();
            tokio::spawn(async move {
                if let Err(e) = handle_punch_hole(host, ph, key).await {
                    log::error!("handle_punch_hole failed: {e:#}");
                }
            });
        }
        Some(rendezvous_message::Union::FetchLocalAddr(fla)) => {
            let peer = AddrMangle::decode(&fla.socket_addr);
            log::info!("FetchLocalAddr from {:?} (same intranet) — listen for direct TCP", peer);
            let host = host.to_string();
            tokio::spawn(async move {
                if let Err(e) = handle_intranet(host, fla).await {
                    log::error!("handle_intranet failed: {e:#}");
                }
            });
        }
        other => log::debug!("udp msg: {other:?}"),
    }
    Ok(())
}

async fn accept_relay(hbbs_host: String, rr: RequestRelay, key: String) -> Result<()> {
    // Notify hbbs then connect to hbbr (initiate=false).
    // Do not wait for hbbs key-exchange here; RelayResponse just wakes the controller.
    let mut notify = connect_tcp(&*hbbs_host, CONNECT_TIMEOUT).await?;
    let mut msg_out = Message::new();
    msg_out.set_relay_response(RelayResponse {
        socket_addr: rr.socket_addr.to_owned(),
        version: peer::version().to_owned(),
        ..Default::default()
    });
    msg_out.mut_relay_response().set_id(Config::get_id());
    notify.send(&msg_out).await?;
    drop(notify);

    let relay = if rr.relay_server.is_empty() {
        socket_client::increase_port(&hbbs_host, 1)
    } else {
        rr.relay_server.clone()
    };
    let relay = socket_client::check_port(relay, RELAY_PORT);
    log::info!("connecting to relay {relay} uuid={}", rr.uuid);
    let mut stream = connect_tcp(relay, CONNECT_TIMEOUT).await?;
    let mut msg_out = Message::new();
    msg_out.set_request_relay(RequestRelay {
        licence_key: key.clone(),
        uuid: rr.uuid,
        ..Default::default()
    });
    stream.send(&msg_out).await?;
    peer::identity_handshake_controlled(&mut stream, true).await?;
    session::run_controlled_port_forward(stream).await
}

async fn initiate_relay(hbbs_host: String, rr: RequestRelay, key: String) -> Result<()> {
    let mut notify = connect_tcp(&*hbbs_host, CONNECT_TIMEOUT).await?;
    let mut msg_out = Message::new();
    msg_out.set_relay_response(RelayResponse {
        socket_addr: rr.socket_addr,
        version: peer::version().to_owned(),
        uuid: rr.uuid.clone(),
        relay_server: rr.relay_server.clone(),
        ..Default::default()
    });
    msg_out.mut_relay_response().set_id(Config::get_id());
    notify.send(&msg_out).await?;
    drop(notify);

    let relay = socket_client::check_port(
        if rr.relay_server.is_empty() {
            socket_client::increase_port(&hbbs_host, 1)
        } else {
            rr.relay_server
        },
        RELAY_PORT,
    );
    let mut stream = connect_tcp(relay, CONNECT_TIMEOUT).await?;
    let mut msg_out = Message::new();
    msg_out.set_request_relay(RequestRelay {
        licence_key: key,
        uuid: rr.uuid,
        ..Default::default()
    });
    stream.send(&msg_out).await?;
    peer::identity_handshake_controlled(&mut stream, true).await?;
    session::run_controlled_port_forward(stream).await
}


/// Same-intranet: notify hbbs of our listen addr, then accept the controller's direct TCP.
async fn handle_intranet(hbbs_host: String, fla: FetchLocalAddr) -> Result<()> {
    let peer_addr = AddrMangle::decode(&fla.socket_addr);
    let mut notify = connect_tcp(&*hbbs_host, CONNECT_TIMEOUT).await?;
    let local_addr = notify.local_addr();
    let relay_server = if fla.relay_server.is_empty() {
        Config::get_option("relay-server")
    } else {
        fla.relay_server.clone()
    };
    let mut msg_out = Message::new();
    msg_out.set_local_addr(LocalAddr {
        id: Config::get_id(),
        socket_addr: fla.socket_addr,
        local_addr: AddrMangle::encode(local_addr).into(),
        relay_server,
        version: peer::version().to_owned(),
        ..Default::default()
    });
    notify.send(&msg_out).await?;
    log::info!("sent LocalAddr local={local_addr:?} for peer {peer_addr:?}");
    // Drop the hbbs notify socket and re-bind the same local_addr with SO_REUSEADDR.
    drop(notify);
    let listener = new_listener(local_addr, true).await?;
    log::info!("intranet listening on {}", listener.local_addr()?);
    match hbb_common::timeout(CONNECT_TIMEOUT, listener.accept()).await {
        Ok(Ok((stream, addr))) => {
            log::info!("intranet accept from {addr} (direct)");
            stream.set_nodelay(true).ok();
            let stream_addr = stream.local_addr()?;
            let mut stream = Stream::from(stream, stream_addr);
            peer::identity_handshake_controlled(&mut stream, true).await?;
            session::run_controlled_port_forward(stream).await
        }
        Ok(Err(e)) => bail!("intranet accept error: {e}"),
        Err(_) => bail!("intranet accept timeout — controller did not connect"),
    }
}

/// Cross-NAT PunchHole: TCP punch attempt, then fall back to initiating relay.
async fn handle_punch_hole(hbbs_host: String, ph: PunchHole, key: String) -> Result<()> {
    let peer_addr = AddrMangle::decode(&ph.socket_addr);
    let relay_server = if ph.relay_server.is_empty() {
        Config::get_option("relay-server")
    } else {
        ph.relay_server.clone()
    };

    if ph.force_relay {
        log::info!("force_relay set; initiating relay");
        let rr = RequestRelay {
            uuid: uuid::Uuid::new_v4().to_string(),
            relay_server: relay_server.clone(),
            socket_addr: ph.socket_addr.clone(),
            secure: true,
            ..Default::default()
        };
        return initiate_relay(hbbs_host, rr, key).await;
    }

    // TCP punch: connect to hbbs (PunchHoleSent), try outbound to peer, listen on local port.
    let mut notify = connect_tcp(&*hbbs_host, CONNECT_TIMEOUT).await?;
    let local_addr = notify.local_addr();
    let punch = socket_client::connect_tcp_local(peer_addr, Some(local_addr), 500);
    let _ = hbb_common::timeout(800, punch).await;

    let mut msg_out = Message::new();
    msg_out.set_punch_hole_sent(PunchHoleSent {
        socket_addr: ph.socket_addr.clone(),
        id: Config::get_id(),
        relay_server: relay_server.clone(),
        version: peer::version().to_owned(),
        ..Default::default()
    });
    notify.send(&msg_out).await?;
    log::info!("sent PunchHoleSent; listening on {local_addr:?} for {peer_addr:?}");
    drop(notify);

    let listener = new_listener(local_addr, true).await?;
    match hbb_common::timeout(CONNECT_TIMEOUT, listener.accept()).await {
        Ok(Ok((stream, addr))) => {
            log::info!("TCP punch accept from {addr} (direct)");
            stream.set_nodelay(true).ok();
            let stream_addr = stream.local_addr()?;
            let mut stream = Stream::from(stream, stream_addr);
            peer::identity_handshake_controlled(&mut stream, true).await?;
            return session::run_controlled_port_forward(stream).await;
        }
        Ok(Err(e)) => log::warn!("punch accept error: {e}"),
        Err(_) => log::warn!("punch accept timeout"),
    }

    // The controller falls back to RequestRelay itself (as upstream does); opening a second
    // relay leg here with our own uuid would race it and leave an orphan session on hbbr.
    let _ = (relay_server, key);
    log::info!("punch to {peer_addr} not met; controller will relay");
    Ok(())
}

