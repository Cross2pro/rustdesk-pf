//! Peer identity handshake (SignedId / PublicKey).

use crate::secure::{create_symmetric_key_msg_pub, decode_id_pk, get_rs_pk};
use anyhow::{anyhow, bail, Result};
use base::message_proto::*;
use bytes::Bytes;
use hbb_common::{
    config::{Config, CONNECT_TIMEOUT, READ_TIMEOUT},
    protobuf::Message as _,
    sodiumoxide::crypto::{box_, sign},
    tcp, timeout, Stream,
};

const VERSION: &str = "1.5.0-pf";

pub fn version() -> &'static str {
    VERSION
}

/// Controlled: send SignedId, receive PublicKey, secure the stream.
pub async fn identity_handshake_controlled(stream: &mut Stream, secure: bool) -> Result<()> {
    let (sk, pk) = Config::get_key_pair();
    if !(secure && pk.len() == sign::PUBLICKEYBYTES && sk.len() == sign::SECRETKEYBYTES) {
        log::warn!("identity handshake skipped (secure={secure})");
        return Ok(());
    }
    let mut sk_buf = [0u8; sign::SECRETKEYBYTES];
    sk_buf.copy_from_slice(&sk);
    let sk = sign::SecretKey(sk_buf);
    let (our_pk_b, our_sk_b) = box_::gen_keypair();
    let advertised = tcp::KX_VERSION_LATEST;
    let mut msg_out = Message::new();
    msg_out.set_signed_id(SignedId {
        id: sign::sign(
            &hbb_common::rendezvous_proto::IdPk {
                id: Config::get_id(),
                pk: Bytes::from(our_pk_b.0.to_vec()),
                kx_version: advertised,
                ..Default::default()
            }
            .write_to_bytes()
            .unwrap_or_default(),
            &sk,
        )
        .into(),
        ..Default::default()
    });
    timeout(CONNECT_TIMEOUT, stream.send(&msg_out)).await??;
    let first = match timeout(CONNECT_TIMEOUT, stream.next()).await {
        Ok(v) => v,
        Err(_) => {
            log::warn!("no PublicKey from controller; continuing unsecured");
            return Ok(());
        }
    };
    let Some(res) = first else {
        log::warn!("peer closed during handshake; continuing unsecured");
        return Ok(());
    };
    let bytes = res?;
    if bytes.is_empty() {
        log::warn!("empty handshake frame; continuing unsecured");
        return Ok(());
    }
    let Ok(msg_in) = Message::parse_from_bytes(&bytes) else {
        log::warn!("unparseable handshake; continuing unsecured");
        return Ok(());
    };
    let Some(message::Union::PublicKey(pk_msg)) = msg_in.union else {
        log::warn!("expected PublicKey, got other; continuing unsecured");
        return Ok(());
    };
    if pk_msg.asymmetric_value.len() == box_::PUBLICKEYBYTES {
        let key =
            tcp::Encrypt::decode(&pk_msg.symmetric_value, &pk_msg.asymmetric_value, &our_sk_b)?;
        if pk_msg.kx_version > advertised {
            bail!(
                "key exchange version {} not offered (max {})",
                pk_msg.kx_version,
                advertised
            );
        }
        stream.set_negotiated_key(
            key,
            false,
            &tcp::KxTranscript {
                initiator_pk: &pk_msg.asymmetric_value,
                responder_pk: &our_pk_b.0,
                advertised,
                picked: pk_msg.kx_version,
            },
        )?;
        log::info!("peer stream secured (controlled)");
    } else if pk_msg.asymmetric_value.is_empty() {
        log::warn!("empty PublicKey; continuing unsecured");
    } else {
        bail!("invalid public key length from peer");
    }
    Ok(())
}

/// Controller: verify peer SignedId using hbbs-provided signed pk blob, send PublicKey.
pub async fn secure_connection_controller(
    peer_id: &str,
    signed_id_pk: &[u8],
    rs_key: &str,
    conn: &mut Stream,
) -> Result<()> {
    let key_str = if rs_key.is_empty() {
        hbb_common::config::RS_PUB_KEY
    } else {
        rs_key
    };
    let sign_pk = resolve_sign_pk(peer_id, signed_id_pk, key_str);
    let Some(sign_pk) = sign_pk else {
        conn.send(&Message::new()).await?;
        log::warn!("continuing without peer stream encryption");
        return Ok(());
    };

    let Some(res) = timeout(READ_TIMEOUT, conn.next()).await? else {
        bail!("no SignedId from peer");
    };
    let bytes = res?;
    finish_signed_id_handshake(peer_id, sign_pk, conn, bytes).await
}

/// Like [`secure_connection_controller`], but the first frame was already read (e.g. after
/// checking it is not an hbbr RelayResponse refuse).
pub async fn secure_connection_controller_with_first(
    peer_id: &str,
    signed_id_pk: &[u8],
    rs_key: &str,
    conn: &mut Stream,
    first: bytes::BytesMut,
) -> Result<()> {
    let key_str = if rs_key.is_empty() {
        hbb_common::config::RS_PUB_KEY
    } else {
        rs_key
    };
    let sign_pk = resolve_sign_pk(peer_id, signed_id_pk, key_str);
    let Some(sign_pk) = sign_pk else {
        conn.send(&Message::new()).await?;
        log::warn!("continuing without peer stream encryption");
        return Ok(());
    };
    finish_signed_id_handshake(peer_id, sign_pk, conn, first).await
}

fn resolve_sign_pk(
    peer_id: &str,
    signed_id_pk: &[u8],
    key_str: &str,
) -> Option<sign::PublicKey> {
    let mut sign_pk = None;
    if !signed_id_pk.is_empty() {
        if let Some(rs_pk) = get_rs_pk(key_str) {
            if let Ok((id, pk)) = decode_id_pk(signed_id_pk, &rs_pk) {
                if id == peer_id {
                    sign_pk = Some(sign::PublicKey(pk));
                } else {
                    log::error!("signed id {id} != expected peer {peer_id}");
                }
            }
        }
        if sign_pk.is_none() {
            log::error!("could not verify peer identity under rendezvous key");
        }
    }
    sign_pk
}

async fn finish_signed_id_handshake(
    peer_id: &str,
    sign_pk: sign::PublicKey,
    conn: &mut Stream,
    bytes: impl AsRef<[u8]>,
) -> Result<()> {
    let msg_in = Message::parse_from_bytes(bytes.as_ref())?;
    let Some(message::Union::SignedId(si)) = msg_in.union else {
        log::error!("expected SignedId from peer");
        conn.send(&Message::new()).await?;
        return Ok(());
    };

    // SignedId.id = sign_peer(IdPk{id, box_pk, kx_version})
    let raw = sign::verify(&si.id, &sign_pk).map_err(|_| anyhow!("SignedId signature mismatch"))?;
    let idpk = hbb_common::rendezvous_proto::IdPk::parse_from_bytes(&raw)?;
    if idpk.id != peer_id {
        bail!("peer id mismatch in SignedId");
    }
    if idpk.pk.len() != 32 {
        bail!("bad ephemeral pk length");
    }
    let mut their_pk_b = [0u8; 32];
    their_pk_b.copy_from_slice(&idpk.pk);
    let kx_version = idpk.kx_version;
    let (asymmetric_value, symmetric_value, key) = create_symmetric_key_msg_pub(their_pk_b);
    let picked = tcp::kx_version_for(kx_version);
    let mut msg_out = Message::new();
    msg_out.set_public_key(PublicKey {
        asymmetric_value: asymmetric_value.clone(),
        symmetric_value,
        kx_version: picked,
        ..Default::default()
    });
    timeout(CONNECT_TIMEOUT, conn.send(&msg_out)).await??;
    conn.set_negotiated_key(
        key,
        true,
        &tcp::KxTranscript {
            initiator_pk: &asymmetric_value,
            responder_pk: &their_pk_b,
            advertised: kx_version,
            picked,
        },
    )?;
    log::info!("peer stream secured (controller)");
    Ok(())
}
