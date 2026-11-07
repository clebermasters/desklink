use crate::{Config, Peer};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingCode {
    pub version: u8,
    pub name: String,
    pub public_key: String,
    pub address: String,
}

impl PairingCode {
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "desklink://pair/{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?)
        ))
    }
    pub fn decode(input: &str) -> Result<Self> {
        ensure!(input.len() <= 4096, "Pairing code is too long.");
        let code = input
            .trim()
            .strip_prefix("desklink://pair/")
            .context("Paste a Desklink pairing code from the other computer.")?;
        let value: Self = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(code)?)?;
        value.validate()?;
        Ok(value)
    }
    pub fn peer(&self) -> Peer {
        Peer {
            name: self.name.clone(),
            public_key: self.public_key.to_lowercase(),
            address: Some(self.address.clone()),
            edge: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported pairing code version.");
        let mut config = Config::new("Pairing".into(), "127.0.0.1:24880".parse()?);
        config.peers.push(self.peer());
        config.validate()
    }
}

pub fn advertised_address(listen: std::net::SocketAddr) -> String {
    if !listen.ip().is_unspecified() {
        return listen.to_string();
    }
    let guessed = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.0.2.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    std::net::SocketAddr::new(guessed, listen.port()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_round_trips_only_public_connection_details() {
        let code = PairingCode {
            version: 1,
            name: "Laptop".into(),
            public_key: hex::encode([7; 32]),
            address: "192.168.1.2:24880".into(),
        };
        let parsed = PairingCode::decode(&code.encode().unwrap()).unwrap();
        assert_eq!(parsed.public_key, code.public_key);
        assert_eq!(parsed.peer().address, Some(code.address));
        assert!(PairingCode::decode("wrong").is_err());
    }
}
