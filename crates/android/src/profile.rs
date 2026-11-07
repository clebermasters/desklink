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

/// The keyboard-and-mouse profile listens beside the phone's own receiver.
pub const KEYBOARD_PORT: u16 = DEFAULT_PORT + 1;

/// Creates the identity and configuration on first use. Never replaces an existing identity.
pub fn ensure(dir: &Path, default_name: &str) -> Result<()> {
    ensure_on_port(dir, default_name, DEFAULT_PORT)
}

/// A computer without Desklink that this phone controls as a Bluetooth keyboard and mouse.
/// It has an identity of its own, so the controller arranges it as a separate screen, and no
/// clipboard: nothing on that computer could receive or send one.
pub fn ensure_keyboard(dir: &Path, default_name: &str) -> Result<()> {
    ensure_on_port(dir, default_name, KEYBOARD_PORT)?;
    set_clipboard(dir, false)
}

fn ensure_on_port(dir: &Path, default_name: &str, port: u16) -> Result<()> {
    let path = config_path(dir);
    let id = identity_path(&path);
    if !id.exists() {
        Identity::create(&id)?;
    }
    if !path.exists() {
        Config::new(
            clean_name(default_name),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
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

/// Makes the computer in `code` the keyboard profile's only, controlling, peer.
pub fn pair_keyboard(dir: &Path, code: &str) -> Result<String> {
    let code = PairingCode::decode(code)?;
    let key = parse_public(&code.public_key)?;
    trust_only(
        dir,
        Peer {
            public_key: hex::encode(key),
            ..code.peer()
        },
    )?;
    Ok(code.name)
}

/// Makes a computer the phone already trusts the keyboard profile's only, controlling, peer.
pub fn trust_keyboard_controller(phone_dir: &Path, dir: &Path, public_key: &str) -> Result<String> {
    let key = parse_public(public_key)?;
    let phone = Config::load(&config_path(phone_dir))?;
    let peer = phone
        .peers
        .into_iter()
        .find(|p| parse_public(&p.public_key).ok() == Some(key))
        .context("pair the controlling computer with this phone first")?;
    let name = peer.name.clone();
    trust_only(dir, peer)?;
    Ok(name)
}

fn trust_only(dir: &Path, peer: Peer) -> Result<()> {
    let path = config_path(dir);
    let own = Identity::load(&identity_path(&path))?.public;
    ensure!(
        parse_public(&peer.public_key)? != own,
        "This is the keyboard's own pairing code."
    );
    Config::modify(&path, |config| {
        if config.layout_owner.as_ref() != Some(&peer.public_key) {
            config.layout = None;
        }
        config.layout_owner = Some(peer.public_key.clone());
        config.peers = vec![peer.clone()];
        config.clipboard = false;
        config.validate_for(own)
    })?;
    Ok(())
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
    fn keyboard_profile_has_its_own_identity_port_and_single_controller() {
        let phone = tempfile::tempdir().unwrap();
        let keyboard = tempfile::tempdir().unwrap();
        ensure(phone.path(), "Phone").unwrap();
        pair(phone.path(), &code([7; 32], "Laptop"), true).unwrap();
        ensure_keyboard(keyboard.path(), "MacBook").unwrap();
        let info = info(keyboard.path(), Some("10.0.0.5")).unwrap();
        assert_eq!(info.address, format!("10.0.0.5:{KEYBOARD_PORT}"));
        assert!(!info.clipboard);
        assert_ne!(
            info.public_key,
            super::info(phone.path(), None).unwrap().public_key
        );
        assert!(
            trust_keyboard_controller(phone.path(), keyboard.path(), &hex::encode([9; 32]))
                .is_err(),
            "only computers the phone trusts"
        );
        assert_eq!(
            trust_keyboard_controller(phone.path(), keyboard.path(), &hex::encode([7; 32]))
                .unwrap(),
            "Laptop"
        );
        pair_keyboard(keyboard.path(), &code([8; 32], "Desktop")).unwrap();
        let config = Config::load(&config_path(keyboard.path())).unwrap();
        assert_eq!(config.peers.len(), 1, "a single controlling computer");
        assert_eq!(config.layout_owner, Some(hex::encode([8; 32])));
        assert!(pair_keyboard(keyboard.path(), &info.pairing_code).is_err());
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
