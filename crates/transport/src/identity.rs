use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};
use zeroize::Zeroizing;

pub const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

pub(crate) fn noise<T>(result: std::result::Result<T, snow::Error>) -> Result<T> {
    result.map_err(|error| anyhow::anyhow!("Noise: {error}"))
}

pub struct Identity {
    pub public: [u8; 32],
    pub(crate) private: Zeroizing<[u8; 32]>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    public_key: String,
    private_key: String,
}

pub fn parse_public(s: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(s).context("public key must contain 64 hexadecimal characters")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("public key must be exactly 32 bytes"))
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let k = noise(snow::Builder::new(noise(PATTERN.parse())?).generate_keypair())?;
        Ok(Self {
            public: k
                .public
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid generated key"))?,
            private: Zeroizing::new(
                k.private
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid generated secret"))?,
            ),
        })
    }
    pub fn load(path: &Path) -> Result<Self> {
        let meta =
            fs::symlink_metadata(path).context("identity missing; run `desklink init` first")?;
        if !meta.is_file() || meta.len() > 4096 {
            bail!("identity must be a small regular file");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                bail!(
                    "identity permissions must be 0600; run chmod 600 {}",
                    path.display()
                );
            }
        }
        let raw = Zeroizing::new(fs::read_to_string(path)?);
        let stored: Stored = serde_json::from_str(&raw)?;
        let secret = Zeroizing::new(stored.private_key);
        let private = Zeroizing::new(parse_public(&secret)?);
        let public = parse_public(&stored.public_key)?;
        let derived = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(*private));
        if public != *derived.as_bytes() {
            bail!("identity public and private keys do not match");
        }
        Ok(Self { public, private })
    }
    /// Create only; never replace an existing identity (doing so would invalidate pairing).
    pub fn create(path: &Path) -> Result<Self> {
        let parent = path.parent().context("identity path has no parent")?;
        fs::create_dir_all(parent)?;
        let identity = Self::generate()?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let data = Zeroizing::new(serde_json::to_vec_pretty(&Stored {
            public_key: hex::encode(identity.public),
            private_key: hex::encode(*identity.private),
        })?);
        file.write_all(&data)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path)
            .context("identity already exists or cannot be created")?;
        Ok(identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_is_stable_and_never_overwritten() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("identity.json");
        let a = Identity::create(&p).unwrap();
        assert_eq!(Identity::load(&p).unwrap().public, a.public);
        assert!(Identity::create(&p).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_world_readable_secret() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("id");
        Identity::create(&p).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Identity::load(&p).is_err());
    }
}
