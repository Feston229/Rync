use anyhow::{anyhow, Result};
use async_once::AsyncOnce;
use lazy_static::lazy_static;
use local_ip_address::local_ip;
use sled::Db;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::{
    net::{TcpListener, UdpSocket},
    task::JoinSet,
};
use tokio_rustls::TlsAcceptor;
use walkdir::WalkDir;

use crate::config::MULTICAST_IP;
use crate::config::MULTICAST_PORT;
use crate::share::Peer;
use crate::utils::{gen_version, init_tcp_listener, init_tracing, is_tracked, sync_files};
use crate::{config::Config, utils::extract_metadata_modified};
use crate::{
    share::{init_multicast_socket, process_conn},
    utils::init_storage,
};

lazy_static! {
    pub static ref MULTICAST_SOCKET: AsyncOnce<UdpSocket> = AsyncOnce::new(async {
        init_multicast_socket()
            .await
            .expect("Failed to init multicast socket")
    });
    pub static ref TCP_PORT: RwLock<u16> = RwLock::new(0);
    pub static ref LIVE_PEERS_KNOWN: RwLock<Vec<Peer>> = RwLock::new(vec![]);
    pub static ref LIVE_PEERS_UNKNOWN: RwLock<Vec<Peer>> = RwLock::new(vec![]);
}

pub async fn run() -> Result<()> {
    // Tracing init
    init_tracing()?;

    // Config init
    let config = tokio::spawn(Config::init());

    // Storage init (for tracking file changes)
    let db = tokio::spawn(init_storage());

    // Init tcp listener (for file transfers)
    let tcp_handle = tokio::spawn(init_tcp_listener());

    // Start poolers
    let config = config.await??;
    let db = db.await??;
    let (serv, port, acceptor) = tcp_handle.await??;
    let mut set = JoinSet::new();
    set.spawn(send_pooling(config.clone(), db.clone()));
    set.spawn(recv_pooling(serv, acceptor.clone()));
    set.spawn(multicast_pooling_sender(db.clone()));
    set.spawn(multicast_pooling_receiver(port));
    set.spawn(update_pooling());

    // Should never happen
    while let Some(res) = set.join_next().await {
        return res?;
    }
    Ok(())
}

async fn manually_added_peers() -> Result<()> {
    Ok(())
}

// Used to share file changes with peers
async fn send_pooling(config: Config, db: Db) -> Result<()> {
    tracing::debug!("Start send_pooling");
    let sync_dir = &config.directory;
    let interval = &config.interval;
    loop {
        // Enumerate files in target dir
        // Where 1String is full path
        // And 2String is modified timestamp
        let files: Vec<(String, String)> = WalkDir::new(sync_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| is_tracked(e))
            .map(|e| (e.path().display().to_string(), e.metadata()))
            .filter_map(|(filename, metadata)| extract_metadata_modified(filename, metadata))
            .collect();

        // Constantly iterates over all files
        // and compare modified timestamp with saved one
        for (filename, timestamp) in files {
            if db.contains_key(&filename)? {
                if let Some(value) = db.get(&filename)? {
                    if value != timestamp {
                        db.compare_and_swap(
                            &filename,
                            db.get(&filename)?,
                            Some(timestamp.as_bytes()),
                        )??;
                        db.compare_and_swap(
                            format!("{}_ver", &filename),
                            db.get(format!("{}_ver", &filename))?,
                            Some(gen_version().as_bytes()),
                        )??;
                    }
                }
            } else {
                db.insert(&filename, &*timestamp)?;
                db.insert(format!("{}_ver", &filename), gen_version().as_bytes())?;
            }
        }

        tokio::time::sleep(Duration::from_secs(*interval)).await;
    }
}

// Used to receive file changes from peers
async fn recv_pooling(serv: TcpListener, acceptor: TlsAcceptor) -> Result<()> {
    tracing::debug!("Start recv_pooling");

    loop {
        match serv.accept().await {
            Ok((stream, addr)) => {
                if let Ok(stream) = acceptor.accept(stream).await {
                    tokio::spawn(async move {
                        if let Err(e) = process_conn(stream, addr).await {
                            tracing::error!("{}", e);
                        }
                    });
                }
            }
            Err(e) => {
                tracing::error!("{}", e)
            }
        }
    }
}

// Used to discover other peers (sender)
async fn multicast_pooling_sender(db: Db) -> Result<()> {
    let socket = MULTICAST_SOCKET.get().await;
    let local_id = String::from_utf8(
        db.get("local_id")?
            .ok_or(anyhow!("local id is missing"))?
            .to_vec(),
    )?;
    let default_msg = format!("rync_{}", &local_id);

    loop {
        if let Err(e) = socket
            .send_to(
                default_msg.as_bytes(),
                SocketAddr::new(MULTICAST_IP.parse()?, MULTICAST_PORT),
            )
            .await
        {
            tracing::error!("{}", e)
        }

        tracing::debug!("Sent ping to multicast group");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

// Used to discover other peers (receiver) (available in local network)
async fn multicast_pooling_receiver(tcp_port: u16) -> Result<()> {
    let socket = MULTICAST_SOCKET.get().await;
    let mut buf: Vec<u8> = vec![];
    loop {
        match socket.recv_buf_from(&mut buf).await {
            Ok((size, addr)) => {
                // Skip ping from myself
                if addr.ip() == local_ip()? {
                    continue;
                }
                let msg = String::from_utf8_lossy(&buf[..size]).to_string();
                // Ping message -> reply with our tcp port
                if msg.starts_with("rync_") {
                    if let Some(peer_id) = msg.split("_").nth(1) {
                        // TODO:
                        // Validate if peer is known
                        tracing::debug!(
                            "Received ping from {} with id {} sending tcp port back",
                            &addr,
                            &peer_id
                        );

                        if let Err(e) = socket
                            .send_to(
                                tcp_port.to_string().as_bytes(),
                                SocketAddr::new(MULTICAST_IP.parse()?, MULTICAST_PORT),
                            )
                            .await
                        {
                            tracing::error!("{}", e)
                        }

                        let mut unknown_peers = LIVE_PEERS_UNKNOWN.write().await;
                        unknown_peers.push(Peer::from((peer_id.to_string(), addr.to_string())));
                    }
                }
                // Incoming tcp port -> add host to online peers
                else {
                    tracing::debug!("Incoming msg -> {} from -> {}", &msg, &addr);
                }
            }
            Err(e) => tracing::error!("{}", e),
        }
        buf.clear();
    }
}

async fn update_pooling() -> Result<()> {
    loop {
        let peers = LIVE_PEERS_UNKNOWN.read().await;
        if !peers.is_empty() {
            for peer in peers.clone().into_iter() {
                tokio::spawn(async move {
                    if let Err(e) = sync_files(peer).await {
                        tracing::error!("{e}");
                    }
                });
            }
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
