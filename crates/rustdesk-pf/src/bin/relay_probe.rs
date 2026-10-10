use hbb_common::{
    config::CONNECT_TIMEOUT, protobuf::Message as _, rendezvous_proto::*,
    socket_client::connect_tcp, tokio,
};
#[tokio::main]
async fn main() -> hbb_common::ResultType<()> {
    let uuid = "test-uuid-probe-0001".to_string();
    let relay = "127.0.0.1:21117";
    let mut msg = RendezvousMessage::new();
    msg.set_request_relay(RequestRelay {
        uuid: uuid.clone(),
        ..Default::default()
    });
    let bytes = msg.write_to_bytes()?;
    println!("protobuf len={} hex={}", bytes.len(), bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>());
    let mut a = connect_tcp(relay, CONNECT_TIMEOUT).await?;
    println!("A local={:?}", a.local_addr());
    a.send(&msg).await?;
    println!("A sent");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let mut b = connect_tcp(relay, CONNECT_TIMEOUT).await?;
    println!("B local={:?}", b.local_addr());
    b.send(&msg).await?;
    println!("B sent");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    a.send_bytes(bytes::Bytes::from_static(b"ping")).await?;
    match b.next_timeout(3000).await {
        Some(Ok(buf)) => println!("B got {} {:?}", buf.len(), &buf[..]),
        x => println!("B result {x:?}"),
    }
    Ok(())
}
