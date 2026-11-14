//! Ephemeral pairing and device sessions. Secrets never enter monitor snapshots or logs.
use crate::app::ids::nonce;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
const DEVICE_TTL: Duration = Duration::from_secs(30 * 86400);
pub const PAIR_TTL: Duration = Duration::from_secs(120);
#[derive(Clone)]
pub struct Device {
    pub id: u64,
    pub name: String,
    pub expires: Instant,
    pub remote: bool,
}
pub struct Auth {
    pub active: bool,
    pub pairing: Option<(String, Instant)>,
    pub devices: HashMap<String, Device>,
}
impl Default for Auth {
    fn default() -> Self {
        Self {
            active: true,
            pairing: None,
            devices: HashMap::new(),
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
impl Auth {
    pub fn pair_token(&mut self) -> String {
        let token = format!("{}{}", nonce(), nonce());
        self.pairing = Some((token.clone(), Instant::now() + PAIR_TTL));
        token
    }
    pub fn pair(&mut self, token: &str, name: &str, remote: bool) -> Result<(String, Device), &'static str> {
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
            remote,
        };
        self.devices.insert(secret.clone(), device.clone());
        Ok((secret, device))
    }
    pub fn device(&self, token: &str) -> Option<Device> {
        self.active
            .then(|| self.devices.get(token))
            .flatten()
            .filter(|d| d.expires > Instant::now())
            .cloned()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_is_expiring_rotating_and_single_use() {
        let mut auth = Auth::default();
        let old = auth.pair_token();
        let token = auth.pair_token();
        assert!(auth.pair(&old, "phone", false).is_err());
        assert!(auth.pair("wrong", "phone", false).is_err());
        let (cookie, _) = auth.pair(&token, "phone", false).unwrap();
        assert!(auth.pair(&token, "phone", false).is_err());
        assert!(auth.device(&cookie).is_some());
        auth.active = false;
        assert!(auth.device(&cookie).is_none());
        auth.active = true;
        let expired = auth.pair_token();
        auth.pairing.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(1);
        assert!(auth.pair(&expired, "phone", false).is_err());
        auth.devices.get_mut(&cookie).unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(auth.device(&cookie).is_none());
    }
}
