//! Validated per-user settings shared by the native editor, CLI and runtime.
pub mod control;
pub mod pairing;
use anyhow::{Context, Result, ensure};
use desklink_protocol::{Edge, Geometry, Layout, Screen};
use desklink_transport::parse_public;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u16,
    pub name: String,
    pub listen: SocketAddr,
    #[serde(default = "enabled")]
    pub clipboard: bool,
    #[serde(default)]
    pub peers: Vec<Peer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// None means this computer controls its layout. Another key explicitly opts into that controller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_owner: Option<String>,
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
    pub fn accept_layout(&mut self, own: [u8; 32], sender: [u8; 32], layout: Layout) -> Result<()> {
        let sender = hex::encode(sender);
        ensure!(
            self.layout_owner.as_ref() == Some(&sender) && layout.owner == sender,
            "This computer is not the selected layout controller."
        );
        ensure!(
            self.layout
                .as_ref()
                .is_none_or(|current| current.revision < layout.revision),
            "A newer layout is already saved."
        );
        let mut next = self.clone();
        next.layout = Some(layout);
        next.validate_for(own)?;
        self.layout = next.layout;
        Ok(())
    }
    pub fn validate_for(&self, own: [u8; 32]) -> Result<()> {
        self.validate()?;
        let own = hex::encode(own);
        ensure!(
            !self.peers.iter().any(|p| p.public_key == own),
            "A computer cannot be paired with itself."
        );
        if let Some(layout) = &self.layout {
            ensure!(
                layout.owner == self.layout_owner.as_ref().unwrap_or(&own).as_str(),
                "Select the controller before accepting its layout."
            );
            ensure!(
                layout.screen(&own).is_some(),
                "The controller has not placed this computer in its layout."
            );
            if layout.owner == own {
                for screen in &layout.screens {
                    ensure!(
                        screen.id == own || self.peers.iter().any(|p| p.public_key == screen.id),
                        "Pair {} before placing it in the layout.",
                        screen.name
                    );
                }
            }
        }
        Ok(())
    }
    pub fn new(name: String, listen: SocketAddr) -> Self {
        Self {
            version: 1,
            name,
            listen,
            clipboard: true,
            peers: vec![],
            layout: None,
            layout_owner: None,
        }
    }
    pub fn editable_layout(&self, own: [u8; 32]) -> Layout {
        if let Some(layout) = &self.layout {
            return layout.clone();
        }
        let own = hex::encode(own);
        let mut layout = Layout::single(own, self.name.clone());
        for (i, peer) in self.peers.iter().enumerate() {
            let mut rect = Geometry {
                x: 240 * (i as i32 + 1),
                y: 0,
                width: 240,
                height: 150,
            };
            match peer.edge {
                Some(Edge::Left) => rect.x = -240,
                Some(Edge::Right) => rect.x = 240,
                Some(Edge::Top) => {
                    rect.x = 0;
                    rect.y = -150;
                }
                Some(Edge::Bottom) => {
                    rect.x = 0;
                    rect.y = 150;
                }
                None => {}
            }
            layout.screens.push(Screen {
                id: peer.public_key.clone(),
                name: peer.name.clone(),
                rect,
            });
        }
        layout
    }
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
            if self.layout.is_none()
                && let Some(edge) = peer.edge
            {
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
        if let Some(owner) = &self.layout_owner {
            parse_public(owner)?;
            ensure!(
                self.peers.iter().any(|p| p.public_key == *owner),
                "The layout controller must be a paired computer."
            );
        }
        if let Some(layout) = &self.layout {
            layout.validate()?;
            ensure!(
                self.layout_owner
                    .as_ref()
                    .is_none_or(|o| *o == layout.owner),
                "Layout does not match its configured controller."
            );
        }
        Ok(())
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let _lock = settings_lock(path)?;
        self.save_unlocked(path)
    }
    fn save_unlocked(&self, path: &Path) -> Result<()> {
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

    pub fn digest(path: &Path) -> Result<String> {
        use sha2::Digest;
        Ok(hex::encode(sha2::Sha256::digest(fs::read(path)?)))
    }
    pub fn save_checked(&self, path: &Path, expected: &str) -> Result<String> {
        let _lock = settings_lock(path)?;
        ensure!(
            Self::digest(path)? == expected,
            "Settings changed in another window. Reload before saving."
        );
        self.save_unlocked(path)?;
        Self::digest(path)
    }
    pub fn modify(path: &Path, change: impl FnOnce(&mut Config) -> Result<()>) -> Result<Config> {
        let _lock = settings_lock(path)?;
        let mut current = Self::load(path)?;
        change(&mut current)?;
        current.save_unlocked(path)?;
        Ok(current)
    }
}

fn settings_lock(path: &Path) -> Result<fs::File> {
    let parent = path
        .parent()
        .context("Settings have no parent directory.")?;
    fs::create_dir_all(parent)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path.with_file_name("settings.lock"))?;
    file.lock()?;
    Ok(file)
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
            layout: None,
            layout_owner: None,
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
            layout: None,
            layout_owner: None,
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
    #[test]
    fn layout_sync_checks_controller_revision_and_keeps_pairing_rights_separate() {
        let a = hex::encode([1; 32]);
        let b = hex::encode([2; 32]);
        let mut config = Config::new("Receiver".into(), "127.0.0.1:24880".parse().unwrap());
        config.peers.push(Peer {
            name: "Controller".into(),
            public_key: a.clone(),
            address: None,
            edge: None,
        });
        config.layout_owner = Some(a.clone());
        let mut layout = Layout::single(a, "Main".into());
        layout.screens.push(Screen {
            id: b,
            name: "Receiver".into(),
            rect: Geometry {
                x: 240,
                y: 0,
                width: 240,
                height: 150,
            },
        });
        assert!(
            config
                .accept_layout([2; 32], [3; 32], layout.clone())
                .is_err()
        );
        assert!(config.layout.is_none());
        config
            .accept_layout([2; 32], [1; 32], layout.clone())
            .unwrap();
        assert_eq!(config.peers.len(), 1);
        assert!(config.accept_layout([2; 32], [1; 32], layout).is_err());
    }
    #[test]
    fn optimistic_save_does_not_overwrite_another_process() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let c = Config::new("Initial".into(), "127.0.0.1:24880".parse().unwrap());
        c.save(&path).unwrap();
        let before = Config::digest(&path).unwrap();
        Config::modify(&path, |c| {
            c.name = "Changed elsewhere".into();
            Ok(())
        })
        .unwrap();
        assert!(c.save_checked(&path, &before).is_err());
        assert_eq!(Config::load(&path).unwrap().name, "Changed elsewhere");
    }
}
