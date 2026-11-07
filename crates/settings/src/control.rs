//! Private, bounded local control protocol. No clipboard contents or key events are exposed.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

pub const MAX_CONTROL: usize = 65536;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Status,
    Apply,
    Stop,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerStatus {
    pub public_key: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    pub name: String,
    pub peers: Vec<PeerStatus>,
    pub active_computer: Option<String>,
    pub layout_revision: Option<u64>,
    pub layout_owner: Option<String>,
    pub clipboard: bool,
    pub error: Option<String>,
    pub restart_required: bool,
}

pub fn socket_path(config: &Path) -> PathBuf {
    config.with_file_name("control.sock")
}

pub fn request(config: &Path, message: Request) -> Result<Status> {
    let mut socket = UnixStream::connect(socket_path(config)).context("Sharing is not running.")?;
    socket.set_read_timeout(Some(Duration::from_millis(600)))?;
    socket.set_write_timeout(Some(Duration::from_millis(600)))?;
    let bytes = serde_json::to_vec(&message)?;
    socket.write_all(&(bytes.len() as u32).to_be_bytes())?;
    socket.write_all(&bytes)?;
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let n = u32::from_be_bytes(header) as usize;
    ensure!(n > 0 && n <= MAX_CONTROL, "invalid control response size");
    let mut body = vec![0; n];
    socket.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}
