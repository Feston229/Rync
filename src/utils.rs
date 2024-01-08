use anyhow::{anyhow, Result};
use directories::ProjectDirs;
use rustls::pki_types::PrivatePkcs8KeyDer;
use sled::Db;
use std::{fs::Metadata, sync::Arc, time::UNIX_EPOCH};
use tokio::{fs, net::TcpListener};
use tokio_rustls::TlsAcceptor;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
use uuid::Uuid;
use walkdir::{DirEntry, Error};

use crate::share::Peer;

// Ignore folders and hidden files
pub fn is_tracked(e: &DirEntry) -> bool {
    if let Some(name) = e.file_name().to_str() {
        if name.starts_with(".") {
            return false;
        }
    }
    if let Ok(metadata) = e.metadata() {
        if metadata.is_file() {
            return true;
        }
    }
    false
}

pub fn extract_metadata_modified(
    filename: String,
    metadata: Result<Metadata, Error>,
) -> Option<(String, String)> {
    if let Ok(metadata) = metadata {
        if let Ok(time) = metadata.modified() {
            if let Ok(time) = time.duration_since(UNIX_EPOCH) {
                Some((filename, time.as_secs().to_string()))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    }
}

pub async fn init_storage() -> Result<Db> {
    let dirs = ProjectDirs::from("com", "ecorp", "rync").ok_or(anyhow!("Unsupported"))?;
    let data_dir = dirs.data_local_dir();
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir).await?;
        let db = sled::open(data_dir.join("db"))?;
        let generated_id = Uuid::new_v4()
            .as_hyphenated()
            .encode_upper(&mut Uuid::encode_buffer())
            .to_string();
        Uuid::new_v4();
        tracing::info!("New local id: {}", &generated_id);
        db.insert("local_id", generated_id.as_bytes())?;
        Ok(db)
    } else {
        let db = sled::open(data_dir.join("db"))?;
        Ok(db)
    }
}

pub fn gen_version() -> String {
    let version = Uuid::new_v4()
        .as_hyphenated()
        .encode_lower(&mut Uuid::encode_buffer())
        .to_string()
        .split("-")
        .nth(0)
        .unwrap()
        .to_string();
    version
}

pub fn init_tracing() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::TRACE)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    tracing::debug!("Start");
    Ok(())
}

pub async fn init_tcp_listener() -> Result<(TcpListener, u16, TlsAcceptor)> {
    let dirs = ProjectDirs::from("com", "ecorp", "rync").ok_or(anyhow!("Unsupported"))?;
    let keys_dir = dirs.data_local_dir().join("keys");
    let key: Vec<u8>;
    let cert: Vec<u8>;
    if !keys_dir.exists() {
        tracing::info!("Generating certificate");
        fs::create_dir_all(&keys_dir).await?;
        let base_cert = rcgen::generate_simple_self_signed(vec![])?;
        key = base_cert.serialize_private_key_der();
        cert = base_cert.serialize_der()?;
        fs::write(keys_dir.join("cert.der"), &cert).await?;
        fs::write(keys_dir.join("key.der"), &key).await?;
    } else {
        key = fs::read(keys_dir.join("key.der")).await?;
        cert = fs::read(keys_dir.join("cert.der")).await?;
    }
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.into()], PrivatePkcs8KeyDer::from(key).into())?;
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let serv = TcpListener::bind("0.0.0.0:0").await?;
    let port = serv.local_addr()?.port();
    Ok((serv, port, acceptor))
}

pub async fn sync_files(peer: Peer) -> Result<()> {
    // 1 Ask for snapshot of files
    // 2 Compare snapshot with local folder
    // 3 If any local files is newer -> stream them to remote
    Ok(())
}
