//! pairing.rs — token sekali pakai 2 menit, secret long-lived
//!
//! Konsep Rust:
//! - Arc<Mutex<T>> : shared ownership antar task tokio (Arc = atomik refcount, Mutex = mutual exclusion)
//! - HashMap ownership: insert/remove memindahkan ownership String

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const TOKEN_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
struct TokenEntry {
    created: Instant,
    used: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustedDevice {
    pub device_id: String,
    pub name: String,
    pub secret: String,
}

fn persist_path() -> std::path::PathBuf {
    let base = dirs_fallback();
    base.join("sidecam_devices.json")
}
fn dirs_fallback() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("APPDATA") { std::path::PathBuf::from(home).join("SideCam") }
    else if let Ok(home) = std::env::var("HOME") { std::path::PathBuf::from(home).join(".config/sidecam") }
    else { std::path::PathBuf::from(".") }
}
fn save_devices(inner: &Inner) {
    let path = persist_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
    if let Ok(j) = serde_json::to_string_pretty(&inner.devices) {
        let _ = std::fs::write(&path, j);
    }
}
fn load_devices() -> HashMap<String, TrustedDevice> {
    let path = persist_path();
    std::fs::read_to_string(&path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Store pairing — M3 persist ke file.
#[derive(Clone)]
pub struct PairingStore {
    inner: Arc<Mutex<Inner>>,
}
impl Default for PairingStore {
    fn default() -> Self {
        let devices = load_devices();
        let mut secrets = HashMap::new();
        for dev in devices.values() { secrets.insert(dev.secret.clone(), dev.clone()); }
        Self { inner: Arc::new(Mutex::new(Inner { tokens: HashMap::new(), secrets, devices })) }
    }
}

#[derive(Default)]
struct Inner {
    tokens: HashMap<String, TokenEntry>,
    secrets: HashMap<String, TrustedDevice>, // secret -> device
    devices: HashMap<String, TrustedDevice>, // device_id -> device
}

impl PairingStore {
    pub fn new() -> Self { Self::default() }

    /// Buat token baru (dipanggil saat desktop start atau via CLI)
    pub fn generate_token(&self) -> String {
        let token = Uuid::new_v4().to_string();
        let mut g = self.inner.lock().unwrap();
        g.tokens.insert(token.clone(), TokenEntry { created: Instant::now(), used: false });
        // cleanup expired
        g.tokens.retain(|_, v| v.created.elapsed() < TOKEN_TTL && !v.used);
        token
    }

    /// Insert token manual (untuk CLI --token)
    pub fn insert_token(&self, token: String) {
        let mut g = self.inner.lock().unwrap();
        g.tokens.insert(token.clone(), TokenEntry { created: Instant::now(), used: false });
    }

    /// Validasi token sekali pakai. Ok(()) = valid & consume.
    pub fn consume_token(&self, token: &str) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap();
        let e = g.tokens.get_mut(token).ok_or_else(|| "Token tidak dikenal".to_string())?;
        if e.used { return Err("Token sudah dipakai".to_string()); }
        if e.created.elapsed() >= TOKEN_TTL {
            return Err("Token kedaluwarsa (2 menit)".to_string());
        }
        e.used = true;
        Ok(())
    }

    pub fn is_token_valid(&self, token: &str) -> bool {
        let g = self.inner.lock().unwrap();
        if let Some(e) = g.tokens.get(token) {
            !e.used && e.created.elapsed() < TOKEN_TTL
        } else { false }
    }

    /// Setelah approve manual, terbitkan secret long-lived
    pub fn issue_secret(&self, device_id: String, name: String) -> String {
        let secret = Uuid::new_v4().to_string();
        let dev = TrustedDevice { device_id: device_id.clone(), name, secret: secret.clone() };
        let mut g = self.inner.lock().unwrap();
        g.secrets.insert(secret.clone(), dev.clone());
        g.devices.insert(device_id, dev);
        save_devices(&g);
        secret
    }

    pub fn validate_secret(&self, secret: &str) -> Option<TrustedDevice> {
        self.inner.lock().unwrap().secrets.get(secret).cloned()
    }

    pub fn list_devices(&self) -> Vec<TrustedDevice> {
        self.inner.lock().unwrap().devices.values().cloned().collect()
    }

    pub fn revoke(&self, device_id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        if let Some(dev) = g.devices.remove(device_id) {
            g.secrets.remove(&dev.secret);
            save_devices(&g);
            true
        } else { false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_once() {
        let s = PairingStore::new();
        let t = s.generate_token();
        assert!(s.consume_token(&t).is_ok());
        assert!(s.consume_token(&t).is_err());
    }
    #[test]
    fn secret_flow() {
        let s = PairingStore::new();
        let sec = s.issue_secret("dev1".into(), "HP Test".into());
        assert!(s.validate_secret(&sec).is_some());
        assert!(s.revoke("dev1"));
        assert!(s.validate_secret(&sec).is_none());
    }
}
