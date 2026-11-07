//! This phone's identity, trusted computers and settings, stored in the app's private files.
use anyhow::{Context, Result, ensure};
use desklink_protocol::DEFAULT_PORT;
use desklink_settings::{
    Config, Peer, identity_path,
    pairing::{PairingCode, advertised_address},
};
use desklink_transport::{Identity, parse_public};
use serde::Serialize;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

pub fn config_path(dir: &Path) -> PathBuf {
    dir.join("config.toml")
}

/// Creates the identity and configuration on first use. Never replaces an existing identity.
pub fn ensure(dir: &Path, default_name: &str) -> Result<()> {
    let path = config_path(dir);
    let id = identity_path(&path);
    if !id.exists() {
        Identity::create(&id)?;
    }
    if !path.exists() {
        Config::new(
            clean_name(default_name),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_PORT),
        )
        .save(&path)?;
    }
    Config::load(&path)?;
    Ok(())
}

fn clean_name(name: &str) -> String {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let mut name = name.trim().to_string();
    while name.len() > 64 {
        name.pop();
    }
    if name.is_empty() {
        "Android phone".into()
    } else {
        name
    }
}

#[derive(Debug, Serialize)]
pub struct PeerInfo {
    pub name: String,
    pub public_key: String,
    pub address: Option<String>,
    pub controller: bool,
}

#[derive(Debug, Serialize)]
pub struct Info {
    pub name: String,
    pub public_key: String,
    pub pairing_code: String,
    pub address: String,
    pub clipboard: bool,
    pub peers: Vec<PeerInfo>,
}

/// `address` is the phone's reachable `ip` (from Android's network APIs); when absent it is
/// guessed from the default route.
pub fn info(dir: &Path, address: Option<&str>) -> Result<Info> {
    let path = config_path(dir);
    let config = Config::load(&path)?;
    let identity = Identity::load(&identity_path(&path))?;
    let address = match address.filter(|a| !a.is_empty()) {
        Some(ip) => {
            let ip: IpAddr = ip.parse().context("invalid phone address")?;
            SocketAddr::new(ip, config.listen.port()).to_string()
        }
        None => advertised_address(config.listen),
    };
    let public_key = hex::encode(identity.public);
    let pairing_code = PairingCode {
        version: 1,
        name: config.name.clone(),
        public_key: public_key.clone(),
        address: address.clone(),
    }
    .encode()?;
    Ok(Info {
        peers: config
            .peers
            .iter()
            .map(|p| PeerInfo {
                name: p.name.clone(),
                public_key: p.public_key.clone(),
                address: p.address.clone(),
                controller: config.layout_owner.as_ref() == Some(&p.public_key),
            })
            .collect(),
        name: config.name,
        public_key,
        pairing_code,
        address,
        clipboard: config.clipboard,
    })
}

/// Decodes a pairing code so the user can confirm it before anything is trusted.
pub fn preview(code: &str) -> Result<PairingCode> {
    PairingCode::decode(code)
}

/// Trusts the computer in `code`. With `controller`, it also becomes the layout controller,
/// so its screen arrangement decides where the phone sits.
pub fn pair(dir: &Path, code: &str, controller: bool) -> Result<String> {
    let path = config_path(dir);
    let code = PairingCode::decode(code)?;
    let key = parse_public(&code.public_key)?;
    let own = Identity::load(&identity_path(&path))?.public;
    ensure!(key != own, "This is the phone's own pairing code.");
    let public_key = hex::encode(key);
    Config::modify(&path, |config| {
        config
            .peers
            .retain(|p| parse_public(&p.public_key).ok() != Some(key));
        config.peers.push(Peer {
            public_key: public_key.clone(),
            ..code.peer()
        });
        if controller {
            if config.layout_owner.as_ref() != Some(&public_key) {
                config.layout = None;
            }
            config.layout_owner = Some(public_key.clone());
        }
        config.validate_for(own)
    })?;
    Ok(code.name)
}

pub fn unpair(dir: &Path, public_key: &str) -> Result<()> {
    let key = parse_public(public_key)?;
    Config::modify(&config_path(dir), |config| {
        config
            .peers
            .retain(|p| parse_public(&p.public_key).ok() != Some(key));
        if config
            .layout_owner
            .as_deref()
            .is_some_and(|owner| parse_public(owner).ok() == Some(key))
        {
            config.layout_owner = None;
            config.layout = None;
        }
        Ok(())
    })?;
    Ok(())
}

pub fn set_name(dir: &Path, name: &str) -> Result<()> {
    let name = name.trim().to_string();
    Config::modify(&config_path(dir), |config| {
        config.name = name;
        Ok(())
    })?;
    Ok(())
}

pub fn set_clipboard(dir: &Path, enabled: bool) -> Result<()> {
    Config::modify(&config_path(dir), |config| {
        config.clipboard = enabled;
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(key: [u8; 32], name: &str) -> String {
        PairingCode {
            version: 1,
            name: name.into(),
            public_key: hex::encode(key),
            address: "192.168.1.20:24880".into(),
        }
        .encode()
        .unwrap()
    }

    #[test]
    fn first_run_creates_a_stable_identity_and_pairing_code() {
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path(), "Pixel\n 9").unwrap();
        let first = info(dir.path(), Some("10.0.0.5")).unwrap();
        assert_eq!(first.name, "Pixel 9");
        assert_eq!(first.address, "10.0.0.5:24880");
        ensure(dir.path(), "Other").unwrap();
        let again = info(dir.path(), None).unwrap();
        assert_eq!(again.public_key, first.public_key);
        let decoded = PairingCode::decode(&first.pairing_code).unwrap();
        assert_eq!(decoded.public_key, first.public_key);
        assert!(info(dir.path(), Some("not an ip")).is_err());
    }

    #[test]
    fn pairing_sets_controller_and_unpairing_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path(), "Phone").unwrap();
        assert_eq!(
            pair(dir.path(), &code([7; 32], "Laptop"), true).unwrap(),
            "Laptop"
        );
        pair(dir.path(), &code([8; 32], "Desktop"), false).unwrap();
        // Re-pairing replaces the entry instead of duplicating it.
        pair(dir.path(), &code([7; 32], "Laptop 2"), true).unwrap();
        let config = Config::load(&config_path(dir.path())).unwrap();
        assert_eq!(config.peers.len(), 2);
        assert_eq!(config.layout_owner, Some(hex::encode([7; 32])));
        let listed = info(dir.path(), None).unwrap();
        assert!(
            listed
                .peers
                .iter()
                .any(|p| p.name == "Laptop 2" && p.controller)
        );
        unpair(dir.path(), &hex::encode([7; 32])).unwrap();
        let config = Config::load(&config_path(dir.path())).unwrap();
        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.layout_owner, None);
    }

    #[test]
    fn refuses_own_code_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path(), "Phone").unwrap();
        let own = info(dir.path(), None).unwrap().pairing_code;
        assert!(pair(dir.path(), &own, true).is_err());
        assert!(pair(dir.path(), "desklink://pair/garbage", true).is_err());
        assert!(preview("https://example.com").is_err());
        assert!(
            Config::load(&config_path(dir.path()))
                .unwrap()
                .peers
                .is_empty()
        );
    }

    #[test]
    fn settings_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path(), "Phone").unwrap();
        assert!(set_name(dir.path(), "").is_err());
        set_name(dir.path(), " Work phone ").unwrap();
        set_clipboard(dir.path(), false).unwrap();
        let config = Config::load(&config_path(dir.path())).unwrap();
        assert_eq!(config.name, "Work phone");
        assert!(!config.clipboard);
    }
}
