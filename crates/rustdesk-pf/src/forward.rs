
#![allow(dead_code)]
use anyhow::Result;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use hbb_common::Stream;
use tokio::net::TcpStream;
use tokio_util::codec::{BytesCodec, Framed};

/// Bidirectional raw byte pipe between a local TCP socket and an encrypted RustDesk Stream.
pub async fn run_forward(local: TcpStream, mut peer: Stream) -> Result<()> {
    peer.set_raw();
    let mut local = Framed::new(local, BytesCodec::new());
    loop {
        tokio::select! {
            res = local.next() => match res {
                Some(Ok(buf)) => {
                    peer.send_bytes(buf.freeze()).await?;
                }
                Some(Err(e)) => {
                    log::warn!("local read error: {e}");
                    break;
                }
                None => break,
            },
            res = peer.next() => match res {
                Some(Ok(bytes)) => {
                    local.send(Bytes::from(bytes)).await?;
                }
                Some(Err(e)) => {
                    log::warn!("peer read error: {e}");
                    break;
                }
                None => break,
            }
        }
    }
    Ok(())
}
