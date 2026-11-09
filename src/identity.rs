//! Device identity and secrets.
//!
//! OpenClaw identifies each client by an Ed25519 key. The device id is the
//! SHA-256 of the raw public key; every connection signs the server's
//! challenge. Secrets live in the Windows Credential Manager through `keyring`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

const SERVICE: &str = "OpenClaw Bots";
const SEED_ENTRY: &str = "device-seed";

/// A place to keep secrets. Falls back to memory for the session when the
/// system keychain refuses, and says so.
pub struct SecretStore {
    pub keychain_ok: bool,
    memory: std::collections::HashMap<String, String>,
}

impl SecretStore {
    pub fn open() -> Self {
        let keychain_ok = keyring::Entry::new(SERVICE, "probe").is_ok();
        Self {
            keychain_ok,
            memory: Default::default(),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(v) = self.memory.get(key) {
            return Some(v.clone());
        }
        if !self.keychain_ok {
            return None;
        }
        keyring::Entry::new(SERVICE, key)
            .ok()?
            .get_password()
            .ok()
            .filter(|v| !v.is_empty())
    }

    /// Stores a secret. Returns false when it could only be kept in memory.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        if self.keychain_ok {
            if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
                if entry.set_password(value).is_ok() {
                    self.memory.remove(key);
                    return true;
                }
            }
        }
        self.memory.insert(key.to_string(), value.to_string());
        false
    }

    pub fn delete(&mut self, key: &str) {
        self.memory.remove(key);
        if self.keychain_ok {
            if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
                let _ = entry.delete_credential();
            }
        }
    }
}

/// The app's device identity.
#[derive(Clone)]
pub struct DeviceIdentity {
    key: SigningKey,
    pub device_id: String,
    pub public_key: String,
}

impl DeviceIdentity {
    pub fn load_or_create(store: &mut SecretStore) -> Self {
        let seed = store
            .get(SEED_ENTRY)
            .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
        let seed = seed.unwrap_or_else(|| {
            let mut b = [0u8; 32];
            getrandom::getrandom(&mut b).expect("random bytes");
            store.set(SEED_ENTRY, &URL_SAFE_NO_PAD.encode(b));
            b
        });
        Self::from_seed(&seed)
    }

    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let key = SigningKey::from_bytes(seed);
        let raw = key.verifying_key().to_bytes();
        let device_id = hex(&Sha256::digest(raw));
        let public_key = URL_SAFE_NO_PAD.encode(raw);
        Self {
            key,
            device_id,
            public_key,
        }
    }

    pub fn sign(&self, payload: &str) -> String {
        URL_SAFE_NO_PAD.encode(self.key.sign(payload.as_bytes()).to_bytes())
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The text a device signs on connect (the "v3" form).
#[allow(clippy::too_many_arguments)]
pub fn device_auth_payload_v3(
    device_id: &str,
    client_id: &str,
    client_mode: &str,
    role: &str,
    scopes: &[String],
    signed_at_ms: i64,
    token: &str,
    nonce: &str,
    platform: &str,
    device_family: &str,
) -> String {
    [
        "v3",
        device_id,
        client_id,
        client_mode,
        role,
        &scopes.join(","),
        &signed_at_ms.to_string(),
        token,
        nonce,
        &platform.trim().to_ascii_lowercase(),
        &device_family.trim().to_ascii_lowercase(),
    ]
    .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_matches_openclaw_layout() {
        let scopes = vec!["operator.read".to_string(), "operator.write".to_string()];
        let p = device_auth_payload_v3(
            "dev", "gateway-client", "ui", "operator", &scopes, 42, "tok", "n", " Windows ", "Desktop",
        );
        assert_eq!(
            p,
            "v3|dev|gateway-client|ui|operator|operator.read,operator.write|42|tok|n|windows|desktop"
        );
    }

    #[test]
    fn device_id_is_hash_of_public_key() {
        let id = DeviceIdentity::from_seed(&[7u8; 32]);
        let raw = URL_SAFE_NO_PAD.decode(&id.public_key).unwrap();
        assert_eq!(raw.len(), 32);
        assert_eq!(id.device_id, hex(&Sha256::digest(&raw)));
        assert!(!id.sign("x").is_empty());
    }
}
