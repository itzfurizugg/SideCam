//! pairing.rs — token sekali pakai 120s, secret 32B CSPRNG hashed SHA-256, approval pending 60s, rate limit 5/60s
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use base64::Engine as _;

const TOKEN_TTL: Duration = Duration::from_secs(120);
const PENDING_TTL: Duration = Duration::from_secs(60);
const RATE_LIMIT: usize = 5;
const RATE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct TokenEntry { created: Instant, used: bool }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustedDevice {
    pub device_id: String,
    pub name: String,
    /// ponytail: stored as base64(SHA256(secret_plain)). Old files with `secret` plaintext auto-migrated on load.
    #[serde(default, alias="secret")]
    pub secret_hash: String,
    // keep old field for reading but not writing
    #[serde(skip_serializing, default)]
    pub secret: String,
}

impl TrustedDevice {
    fn new(device_id: String, name: String, secret_hash: String) -> Self {
        Self { device_id, name, secret_hash, secret: String::new() }
    }
}

#[derive(Debug, Clone)]
pub struct PendingDevice {
    pub device_id: String,
    pub name: String,
    pub created: Instant,
    pub secret_plain: String,
    pub secret_hash: String,
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
    // serialize as map device_id -> {device_id,name,secret_hash}
    let mut out: HashMap<String, serde_json::Value> = HashMap::new();
    for (k, v) in &inner.devices {
        out.insert(k.clone(), serde_json::json!({"device_id": v.device_id, "name": v.name, "secret_hash": v.secret_hash}));
    }
    if let Ok(j) = serde_json::to_string_pretty(&out) { let _ = std::fs::write(&path, j); }
}
fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}
fn gen_secret_plain() -> String {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("CSPRNG failed");
    base64::engine::general_purpose::STANDARD.encode(buf)
}
fn load_devices() -> HashMap<String, TrustedDevice> {
    let path = persist_path();
    let s = match std::fs::read_to_string(&path) { Ok(s)=>s, Err(_)=>return HashMap::new() };
    let v: serde_json::Value = match serde_json::from_str(&s) { Ok(v)=>v, Err(_)=>return HashMap::new() };
    let mut out: HashMap<String, TrustedDevice> = HashMap::new();
    if let Some(map) = v.as_object() {
        for (k, val) in map {
            let device_id = val.get("device_id").and_then(|x| x.as_str()).unwrap_or(k).to_string();
            let name = val.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let secret_hash = val.get("secret_hash").and_then(|x| x.as_str()).map(|x| x.to_string());
            let secret_plain = val.get("secret").and_then(|x| x.as_str()).map(|x| x.to_string());
            let hash = if let Some(h) = secret_hash {
                // if also has plaintext secret and hash looks like not migrated, keep hash
                h
            } else if let Some(p) = secret_plain.clone() {
                hash_secret(&p)
            } else { continue; };
            out.insert(k.clone(), TrustedDevice::new(device_id, name, hash));
        }
        // migrate if needed: rewrite file with hashes only
        if out.values().any(|_| true) {
            // check if file contained plaintext secret without hash
            let had_plaintext = v.as_object().map(|m| m.values().any(|val| val.get("secret").is_some() && val.get("secret_hash").is_none())).unwrap_or(false);
            if had_plaintext {
                let tmp_inner = Inner { tokens: HashMap::new(), secrets: HashMap::new(), devices: out.clone(), pending: HashMap::new(), rate: HashMap::new() };
                save_devices(&tmp_inner);
            }
        }
    }
    out
}

#[derive(Default)]
struct Inner {
    tokens: HashMap<String, TokenEntry>,
    secrets: HashMap<String, TrustedDevice>, // hash -> device (for fast lookup via hash)
    devices: HashMap<String, TrustedDevice>, // device_id -> device
    pending: HashMap<String, PendingDevice>, // device_id -> pending
    rate: HashMap<IpAddr, Vec<Instant>>,
}

#[derive(Clone)]
pub struct PairingStore { inner: Arc<Mutex<Inner>> }

impl Default for PairingStore {
    fn default() -> Self {
        let devices = load_devices();
        let mut secrets = HashMap::new();
        for dev in devices.values() { secrets.insert(dev.secret_hash.clone(), dev.clone()); }
        Self { inner: Arc::new(Mutex::new(Inner { tokens: HashMap::new(), secrets, devices, pending: HashMap::new(), rate: HashMap::new() })) }
    }
}
impl PairingStore {
    pub fn new() -> Self { Self::default() }

    pub fn generate_token(&self) -> String {
        let token = Uuid::new_v4().to_string();
        let mut g = self.inner.lock().unwrap();
        g.tokens.insert(token.clone(), TokenEntry { created: Instant::now(), used: false });
        g.tokens.retain(|_, v| v.created.elapsed() < TOKEN_TTL && !v.used);
        token
    }
    pub fn insert_token(&self, token: String) {
        let mut g = self.inner.lock().unwrap();
        g.tokens.insert(token, TokenEntry { created: Instant::now(), used: false });
    }

    /// Rate limit check 5/60s per IP. Call before consume. Returns Err if limited.
    pub fn check_rate(&self, ip: IpAddr) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        let entry = g.rate.entry(ip).or_default();
        entry.retain(|t| t.elapsed() < RATE_WINDOW);
        if entry.len() >= RATE_LIMIT { return Err("Rate limit 5/menit".into()); }
        entry.push(now);
        Ok(())
    }

    /// Validasi token once. Ok = valid & consume (mark used). Caller must then call `create_pending` if need approval flow.
    pub fn consume_token(&self, token: &str) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap();
        let e = g.tokens.get_mut(token).ok_or_else(|| "Token tidak dikenal".to_string())?;
        if e.used { return Err("Token sudah dipakai".to_string()); }
        if e.created.elapsed() >= TOKEN_TTL { return Err("Token kedaluwarsa (2 menit)".to_string()); }
        e.used = true;
        Ok(())
    }
    pub fn is_token_valid(&self, token: &str) -> bool {
        let g = self.inner.lock().unwrap();
        if let Some(e) = g.tokens.get(token) { !e.used && e.created.elapsed() < TOKEN_TTL } else { false }
    }

    /// Buat pending approval 60s. Harus dipanggil setelah consume_token sukses.
    /// Returns secret_plain untuk dikirim setelah approve.
    pub fn create_pending(&self, device_id: String, name: String) -> String {
        let plain = gen_secret_plain();
        let hash = hash_secret(&plain);
        let mut g = self.inner.lock().unwrap();
        // cleanup expired pending
        g.pending.retain(|_, v| v.created.elapsed() < PENDING_TTL);
        g.pending.insert(device_id.clone(), PendingDevice { device_id: device_id.clone(), name: name.clone(), created: Instant::now(), secret_plain: plain.clone(), secret_hash: hash });
        // prompt di headless (stdout)
        println!("[pairing] Pending approval: device={} name={} — ketik 'y' untuk approve, 'n' tolak (60s timeout)", device_id, name);
        plain
    }
    pub fn get_pending(&self, device_id: &str) -> Option<PendingDevice> {
        let mut g = self.inner.lock().unwrap();
        g.pending.retain(|_, v| v.created.elapsed() < PENDING_TTL);
        g.pending.get(device_id).cloned()
    }
    pub fn is_pending(&self, device_id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        g.pending.retain(|_, v| v.created.elapsed() < PENDING_TTL);
        g.pending.contains_key(device_id)
    }
    /// Approve via admin POST. If approve=true, device jadi trusted dan secret_plain dikembalikan once.
    pub fn approve(&self, device_id: &str, approve: bool) -> Result<Option<String>, String> {
        let mut g = self.inner.lock().unwrap();
        g.pending.retain(|_, v| v.created.elapsed() < PENDING_TTL);
        let p = g.pending.remove(device_id).ok_or_else(|| "Pending tidak ditemukan/kedaluwarsa".to_string())?;
        if !approve { return Ok(None); }
        let dev = TrustedDevice::new(p.device_id.clone(), p.name.clone(), p.secret_hash.clone());
        g.secrets.insert(p.secret_hash.clone(), dev.clone());
        g.devices.insert(p.device_id.clone(), dev);
        save_devices(&g);
        Ok(Some(p.secret_plain))
    }
    /// Helper untuk headless y/n prompt: approve pending device directly
    pub fn approve_headless(&self, device_id: &str) -> Result<String, String> {
        self.approve(device_id, true)?.ok_or_else(|| "Ditolak".to_string())
    }

    /// Setelah approve, secret_plain hanya dikirim sekali via AuthOk. Selanjutnya client pakai secret itu.
    pub fn issue_secret(&self, device_id: String, name: String) -> String {
        // untuk API auto-approve (legacy) — tetap pakai CSPRNG+hash
        let plain = gen_secret_plain();
        let hash = hash_secret(&plain);
        let dev = TrustedDevice::new(device_id.clone(), name, hash.clone());
        let mut g = self.inner.lock().unwrap();
        g.secrets.insert(hash, dev.clone());
        g.devices.insert(device_id, dev);
        save_devices(&g);
        plain
    }

    pub fn validate_secret(&self, secret: &str) -> Option<TrustedDevice> {
        let h = hash_secret(secret);
        let g = self.inner.lock().unwrap();
        // constant-time compare across all? We already hash, can do lookup + constant-time verify
        if let Some(dev) = g.secrets.get(&h) {
            // verify constant-time
            let stored = dev.secret_hash.as_bytes();
            let computed = h.as_bytes();
            if stored.len() == computed.len() && stored.ct_eq(computed).unwrap_u8()==1 {
                return Some(dev.clone());
            }
        }
        // fallback linear constant-time search (defense if hash collision store mismatch)
        for dev in g.secrets.values() {
            if dev.secret_hash.as_bytes().len() == h.as_bytes().len() && dev.secret_hash.as_bytes().ct_eq(h.as_bytes()).unwrap_u8()==1 {
                return Some(dev.clone());
            }
        }
        None
    }

    pub fn list_devices(&self) -> Vec<TrustedDevice> {
        self.inner.lock().unwrap().devices.values().cloned().collect()
    }
    pub fn revoke(&self, device_id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        if let Some(dev) = g.devices.remove(device_id) {
            g.secrets.remove(&dev.secret_hash);
            save_devices(&g);
            true
        } else { false }
    }
    pub fn list_pending(&self) -> Vec<PendingDevice> {
        let mut g = self.inner.lock().unwrap();
        g.pending.retain(|_, v| v.created.elapsed() < PENDING_TTL);
        g.pending.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn token_once() {
        let s = PairingStore::new();
        let t = s.generate_token();
        assert!(s.consume_token(&t).is_ok());
        assert!(s.consume_token(&t).is_err());
    }
    #[test] fn secret_flow_hashed() {
        let s = PairingStore::new();
        let sec = s.issue_secret("dev1".into(), "HP Test".into());
        assert!(s.validate_secret(&sec).is_some());
        assert!(s.revoke("dev1"));
        assert!(s.validate_secret(&sec).is_none());
    }
    #[test] fn pending_flow() {
        let s = PairingStore::new();
        let t = s.generate_token();
        assert!(s.consume_token(&t).is_ok());
        let _plain = s.create_pending("devX".into(), "HP X".into());
        assert!(s.is_pending("devX"));
        let res = s.approve("devX", true).unwrap();
        assert!(res.is_some());
        assert!(s.validate_secret(&res.unwrap()).is_some());
    }
    #[test] fn rate_limit() {
        let s = PairingStore::new();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        for _ in 0..5 { assert!(s.check_rate(ip).is_ok()); }
        assert!(s.check_rate(ip).is_err());
    }
}
