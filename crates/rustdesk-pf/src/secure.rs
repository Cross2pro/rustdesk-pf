//! Rendezvous key-exchange helpers, adapted from upstream `src/common.rs`.

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use hbb_common::{
    config::{Config, CONNECT_TIMEOUT, READ_TIMEOUT, RS_PUB_KEY},
    protobuf::Message as _,
    rendezvous_proto::*,
    sodiumoxide::crypto::{box_, secretbox, sign},
    timeout, Stream,
};

fn get_pk(pk: &[u8]) -> Option<[u8; 32]> {
    if pk.len() == 32 {
        let mut tmp = [0u8; 32];
        tmp.copy_from_slice(pk);
        Some(tmp)
    } else {
        None
    }
}

pub fn get_rs_pk(str_base64: &str) -> Option<sign::PublicKey> {
    let Ok(pk) = hbb_common::base64::decode(str_base64) else {
        return None;
    };
    get_pk(&pk).map(sign::PublicKey)
}

pub fn rs_key() -> String {
    let key = Config::get_option("key");
    if key.is_empty() {
        RS_PUB_KEY.to_owned()
    } else {
        key
    }
}

pub fn create_symmetric_key_msg_pub(their_pk_b: [u8; 32]) -> (Bytes, Bytes, secretbox::Key) {
    let their_pk_b = box_::PublicKey(their_pk_b);
    let (our_pk_b, out_sk_b) = box_::gen_keypair();
    let key = secretbox::gen_key();
    let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
    let sealed_key = box_::seal(&key.0, &nonce, &their_pk_b, &out_sk_b);
    (Vec::from(our_pk_b.0).into(), sealed_key.into(), key)
}

/// Secure the TCP stream to hbbs/hbbr with the server's signed key exchange.
pub async fn secure_tcp(conn: &mut Stream, key: &str) -> Result<()> {
    let Some(rs_pk) = get_rs_pk(key) else {
        bail!("invalid rendezvous public key");
    };
    let first = match timeout(2_000, conn.next()).await {
        Ok(v) => v,
        Err(_) => {
            log::warn!("no key exchange from rendezvous within timeout; continuing");
            return Ok(());
        }
    };
    match first {
        Some(Ok(bytes)) => {
            if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
                if let Some(rendezvous_message::Union::KeyExchange(ex)) = msg_in.union {
                    if ex.keys.len() != 1 {
                        bail!("invalid key exchange message");
                    }
                    let their_pk_b = sign::verify(&ex.keys[0], &rs_pk)
                        .map_err(|_| anyhow!("signature mismatch in key exchange"))?;
                    let their_pk_b =
                        get_pk(&their_pk_b).context("wrong their public length in key exchange")?;
                    if their_pk_b[31] & 0x80 != 0 || !ex.signed_params.is_empty() {
                        let params = sign::verify(&ex.signed_params, &rs_pk)
                            .ok()
                            .and_then(|signed| {
                                let params =
                                    signed.strip_prefix(hbb_common::tcp::KX_PARAMS_DOMAIN)?;
                                KxParams::parse_from_bytes(params).ok()
                            })
                            .ok_or_else(|| {
                                anyhow!("missing or invalid signed key exchange parameters")
                            })?;
                        if params.pk[..] != their_pk_b[..] || params.version != ex.version {
                            bail!("key exchange version or public key does not match signature");
                        }
                    }
                    let (asymmetric_value, symmetric_value, key) =
                        create_symmetric_key_msg_pub(their_pk_b);
                    let picked = hbb_common::tcp::kx_version_for(ex.version);
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_key_exchange(KeyExchange {
                        keys: vec![asymmetric_value.clone(), symmetric_value],
                        version: picked,
                        ..Default::default()
                    });
                    timeout(CONNECT_TIMEOUT, conn.send(&msg_out)).await??;
                    conn.set_negotiated_key(
                        key,
                        true,
                        &hbb_common::tcp::KxTranscript {
                            initiator_pk: &asymmetric_value,
                            responder_pk: &their_pk_b,
                            advertised: ex.version,
                            picked,
                        },
                    )?;
                    log::info!("rendezvous connection secured");
                    return Ok(());
                }
            }
        }
        _ => {}
    }
    // Older servers may not do key exchange; tolerate like upstream secure_tcp.
    log::warn!("no key exchange from rendezvous; continuing unencrypted to hbbs");
    Ok(())
}

pub fn decode_id_pk(signed: &[u8], key: &sign::PublicKey) -> Result<(String, [u8; 32])> {
    let res = IdPk::parse_from_bytes(
        &sign::verify(signed, key).map_err(|_| anyhow!("signature mismatch"))?,
    )?;
    let pk = get_pk(&res.pk).ok_or_else(|| anyhow!("wrong their public length"))?;
    Ok((res.id, pk))
}
