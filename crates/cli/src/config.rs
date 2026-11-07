use anyhow::{Context, Result, ensure};
use desklink_protocol::Edge;
use desklink_transport::parse_public;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u16,
    pub name: String,
    pub listen: SocketAddr,
    #[serde(default = "enabled")]
    pub clipboard: bool,
    #[serde(default)]
    pub peers: Vec<Peer>,
}
fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub name: String,
    pub public_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<Edge>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let meta =
            fs::metadata(path).context("configuration missing; run `desklink init` first")?;
        ensure!(meta.len() <= 65536, "configuration is too large");
        let config: Self =
            toml::from_str(&fs::read_to_string(path)?).context("invalid configuration")?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "unsupported configuration version");
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 64
                && !self.name.chars().any(char::is_control),
            "computer name must be 1–64 visible characters"
        );
        ensure!(self.listen.port() != 0, "listen port cannot be zero");
        ensure!(self.peers.len() <= 16, "at most 16 peers are supported");
        let mut keys = HashSet::new();
        let mut edges = HashSet::new();
        for peer in &self.peers {
            ensure!(
                !peer.name.trim().is_empty()
                    && peer.name.len() <= 64
                    && !peer.name.chars().any(char::is_control),
                "invalid peer name"
            );
            ensure!(
                keys.insert(parse_public(&peer.public_key)?),
                "duplicate peer public key"
            );
            if let Some(edge) = peer.edge {
                ensure!(
                    edges.insert(format!("{edge:?}")),
                    "only one peer may occupy each edge"
                );
            }
            if let Some(address) = &peer.address {
                ensure!(
                    address.len() <= 255
                        && !address.chars().any(|c| c.is_whitespace() || c.is_control()),
                    "invalid peer address"
                );
                let (host, port) = address
                    .rsplit_once(':')
                    .context("peer address must be hostname:port or [IPv6]:port")?;
                ensure!(
                    !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p != 0),
                    "invalid peer address or port"
                );
            }
        }
        Ok(())
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .context("configuration has no parent folder")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}

pub fn default_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "desklink")
        .context("could not determine the user configuration folder")?;
    Ok(dirs.config_dir().join("config.toml"))
}
pub fn identity_path(config: &Path) -> PathBuf {
    config.with_file_name("identity.json")
}

/// The OS releases the advisory lock automatically if the process crashes.
pub fn lock_profile(config: &Path) -> Result<fs::File> {
    let path = config.with_file_name("run.lock");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        ensure!(meta.is_file(), "profile lock must be a regular file");
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    file.try_lock()
        .context("this profile is already running, or its lock is unavailable")?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_edge_configuration() {
        let mut c = Config {
            version: 1,
            name: "a".into(),
            listen: "127.0.0.1:24880".parse().unwrap(),
            clipboard: true,
            peers: vec![],
        };
        for k in [1, 2] {
            c.peers.push(Peer {
                name: "p".into(),
                public_key: hex::encode([k; 32]),
                address: None,
                edge: Some(Edge::Left),
            });
        }
        assert!(c.validate().is_err());
    }
    #[test]
    fn config_round_trip_is_atomic() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        let c = Config {
            version: 1,
            name: "Desk A".into(),
            listen: "127.0.0.1:24880".parse().unwrap(),
            clipboard: true,
            peers: vec![],
        };
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().name, c.name);
    }
    #[test]
    fn only_one_instance_can_own_a_profile() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        let lock = lock_profile(&path).unwrap();
        assert!(lock_profile(&path).is_err());
        drop(lock);
        assert!(lock_profile(&path).is_ok());
    }
}
