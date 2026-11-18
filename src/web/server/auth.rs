//! Ephemeral pairing and persistent hashed browser credentials. Secrets never enter snapshots or logs.
use super::Ingress;
use crate::app::ids::nonce;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const DEVICE_TTL: Duration = Duration::from_secs(30 * 86400);
pub const PAIR_TTL: Duration = Duration::from_secs(120);
#[derive(Clone)]
pub struct Device {
    pub id: u64,
    pub name: String,
    pub expires: Instant,
    pub remote: bool,
    pub ingress: Ingress,
}
pub struct Auth {
    pub active: bool,
    pub pairing: Option<(String, Instant)>,
    pub devices: HashMap<String, Device>,
    storage: Option<PathBuf>,
}
impl Default for Auth {
    fn default() -> Self {
        Self {
            active: true,
            pairing: None,
            devices: HashMap::new(),
            storage: None,
        }
    }
}
fn equal(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}
fn fingerprint(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
#[derive(Serialize, Deserialize)]
struct SavedDevice {
    fingerprint: String,
    id: u64,
    name: String,
    expires: u64,
    ingress: Ingress,
}
#[derive(Serialize, Deserialize)]
struct SavedAuth {
    version: u32,
    devices: Vec<SavedDevice>,
}
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl Auth {
    pub fn load(path: PathBuf) -> Result<Self> {
        let mut auth = Self {
            storage: Some(path.clone()),
            ..Self::default()
        };
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(auth),
            Err(error) => return Err(error).context("Cannot read saved browser authorization"),
        };
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .context("Cannot read saved browser authorization")?;
        anyhow::ensure!(bytes.len() <= 65536, "Invalid saved browser authorization");
        let saved: SavedAuth =
            serde_json::from_slice(&bytes).context("Invalid saved browser authorization")?;
        anyhow::ensure!(
            saved.version == 1 && saved.devices.len() <= 16,
            "Unsupported saved browser authorization"
        );
        let now = unix_now();
        for device in saved.devices {
            anyhow::ensure!(
                device.fingerprint.len() == 64
                    && device.fingerprint.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid browser fingerprint"
            );
            if device.expires <= now {
                continue;
            }
            anyhow::ensure!(
                device.expires - now <= DEVICE_TTL.as_secs(),
                "Invalid browser expiry"
            );
            auth.devices.insert(
                device.fingerprint,
                Device {
                    id: device.id,
                    name: device.name.chars().take(80).collect(),
                    expires: Instant::now() + Duration::from_secs(device.expires - now),
                    remote: device.ingress != Ingress::Local,
                    ingress: device.ingress,
                },
            );
        }
        Ok(auth)
    }
    fn persist(&self) -> Result<()> {
        let Some(path) = &self.storage else {
            return Ok(());
        };
        let now = Instant::now();
        let unix = unix_now();
        let devices = self
            .devices
            .iter()
            .filter(|(_, d)| d.expires > now)
            .map(|(fingerprint, d)| SavedDevice {
                fingerprint: fingerprint.clone(),
                id: d.id,
                name: d.name.clone(),
                expires: unix + d.expires.saturating_duration_since(now).as_secs(),
                ingress: d.ingress,
            })
            .collect();
        let bytes = serde_json::to_vec(&SavedAuth {
            version: 1,
            devices,
        })?;
        let parent = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".web-devices-{}.tmp", nonce()));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result.context("Cannot save browser authorization")
    }
    pub fn revoke(&mut self, id: u64) -> Result<()> {
        let previous = self.devices.clone();
        self.devices.retain(|_, device| device.id != id);
        if let Err(error) = self.persist() {
            self.devices = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn pair_token(&mut self) -> String {
        let token = format!("{}{}", nonce(), nonce());
        self.pairing = Some((token.clone(), Instant::now() + PAIR_TTL));
        token
    }
    pub fn pair(
        &mut self,
        token: &str,
        name: &str,
        ingress: Ingress,
    ) -> Result<(String, Device), &'static str> {
        if !self.active
            || !self
                .pairing
                .as_ref()
                .is_some_and(|(secret, until)| *until > Instant::now() && equal(token, secret))
        {
            return Err(
                "Pairing link expired or already used. Generate a new link on your computer.",
            );
        }
        self.devices
            .retain(|_, device| device.expires > Instant::now());
        // Do not evict active devices implicitly; local CLI can revoke them explicitly.
        if self.devices.len() >= 16 {
            return Err("Device limit reached. Revoke an existing device on your computer.");
        }
        self.pairing = None;
        let secret = format!("{}{}", nonce(), nonce());
        let device = Device {
            id: u64::from_str_radix(&nonce()[..16], 16).unwrap(),
            name: name.chars().take(80).collect(),
            expires: Instant::now() + DEVICE_TTL,
            remote: ingress != Ingress::Local,
            ingress,
        };
        let fingerprint = fingerprint(&secret);
        self.devices.insert(fingerprint.clone(), device.clone());
        if self.persist().is_err() {
            self.devices.remove(&fingerprint);
            return Err("Could not save browser authorization. Try pairing again.");
        }
        Ok((secret, device))
    }
    pub fn device(&self, token: &str) -> Option<Device> {
        self.active
            .then(|| self.devices.get(&fingerprint(token)))
            .flatten()
            .filter(|d| d.expires > Instant::now())
            .cloned()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_authorization_survives_restart_without_saving_secrets_or_pairing() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("devices.json");
        let mut auth = Auth::load(path.clone()).unwrap();
        let mut cookies = Vec::new();
        for ingress in [
            Ingress::Local,
            Ingress::Tunnel,
            Ingress::Ngrok,
            Ingress::Tuna,
        ] {
            let token = auth.pair_token();
            let (cookie, device) = auth.pair(&token, "test browser", ingress).unwrap();
            let bytes = std::fs::read_to_string(&path).unwrap();
            assert!(!bytes.contains(&cookie));
            assert!(!bytes.contains(&token));
            cookies.push((cookie, device));
        }
        let unused_qr = auth.pair_token();
        let mut restored = Auth::load(path.clone()).unwrap();
        assert!(restored.pairing.is_none());
        assert!(
            restored
                .pair(&unused_qr, "browser", Ingress::Local)
                .is_err()
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for (cookie, original) in &cookies {
            let device = restored.device(cookie).unwrap();
            assert_eq!(device.id, original.id);
            assert!(device.ingress == original.ingress);
            assert_eq!(device.remote, original.remote);
            assert!(device.expires <= original.expires);
        }
        restored.revoke(cookies[0].1.id).unwrap();
        let restored = Auth::load(path.clone()).unwrap();
        assert!(restored.device(&cookies[0].0).is_none());
        assert!(restored.device(&cookies[1].0).is_some());
        let mut saved: SavedAuth = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for device in &mut saved.devices {
            device.expires = unix_now() - 1;
        }
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(Auth::load(path).unwrap().devices.is_empty());
    }
    #[test]
    fn storage_errors_do_not_admit_or_resurrect_devices() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("devices.json");
        let mut auth = Auth::load(path.clone()).unwrap();
        std::fs::create_dir(&path).unwrap();
        let token = auth.pair_token();
        assert!(auth.pair(&token, "browser", Ingress::Local).is_err());
        assert!(auth.devices.is_empty());
        std::fs::remove_dir(&path).unwrap();
        let token = auth.pair_token();
        let (cookie, device) = auth.pair(&token, "browser", Ingress::Local).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(auth.revoke(device.id).is_err());
        assert!(auth.device(&cookie).is_some());
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "invalid").unwrap();
        assert!(Auth::load(path).is_err());
    }
    #[test]
    fn pairing_is_expiring_rotating_and_single_use() {
        let mut auth = Auth::default();
        let old = auth.pair_token();
        let token = auth.pair_token();
        assert!(auth.pair(&old, "phone", Ingress::Local).is_err());
        assert!(auth.pair("wrong", "phone", Ingress::Local).is_err());
        let (cookie, _) = auth.pair(&token, "phone", Ingress::Local).unwrap();
        assert!(auth.pair(&token, "phone", Ingress::Local).is_err());
        assert!(auth.device(&cookie).is_some());
        auth.active = false;
        assert!(auth.device(&cookie).is_none());
        auth.active = true;
        let expired = auth.pair_token();
        auth.pairing.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(1);
        assert!(auth.pair(&expired, "phone", Ingress::Local).is_err());
        auth.devices.get_mut(&fingerprint(&cookie)).unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert!(auth.device(&cookie).is_none());
    }
}
