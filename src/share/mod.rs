use anyhow::Result;
use bincode::deserialize;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpStream, UdpSocket};
use tokio_rustls::server::TlsStream;

#[derive(Clone)]
pub struct Peer {
    peer_id: String,
    endpoint: String,
}

impl From<(String, String)> for Peer {
    fn from(data: (String, String)) -> Self {
        let (peer_id, endpoint) = data;
        Self { peer_id, endpoint }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncFile {
    filename: String,
    timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SecondaryResponse {
    data: Vec<SyncFile>,
}

pub async fn process_conn(mut stream: TlsStream<TcpStream>, addr: SocketAddr) -> Result<()> {
    tracing::info!("Incoming connection from: {}", addr);
    let mut buf = [0; 8];
    let stream = stream.get_mut().0;
    if let Err(e) = stream.read_exact(&mut buf).await {
        tracing::error!("{}", e);
    }
    let len = u64::from_be_bytes(buf);

    let mut serialized_data = vec![0; len as usize];
    if let Err(e) = stream.read_exact(&mut serialized_data).await {
        tracing::error!("{}", e);
    }
    let secondary_response: SecondaryResponse = deserialize(&serialized_data)?;
    tracing::debug!("Got -> {:?}", &secondary_response);

    Ok(())
}

pub async fn init_multicast_socket() -> Result<UdpSocket> {
    let multicast_addr: Ipv4Addr = "239.0.0.1".parse()?;
    let port: u16 = 23235;

    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).await?;
    socket.join_multicast_v4(multicast_addr.clone(), Ipv4Addr::UNSPECIFIED)?;
    // HARDCODE !!!!
    socket.join_multicast_v4(multicast_addr.clone(), "192.168.122.1".parse()?)?;
    Ok(socket)
}
