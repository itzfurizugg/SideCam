//! sidecam-core — signaling + WebRTC + pairing
pub mod pairing;
pub mod signaling;
pub mod server;
pub mod webrtc;
pub mod mdns;

pub use pairing::PairingStore;
pub use server::{router, AppState};
