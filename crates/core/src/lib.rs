//! sidecam-core — signaling + WebRTC + pairing
pub mod pairing;
pub mod signaling;
pub mod server;
pub mod webrtc;
pub mod mdns;
pub mod decode;
pub mod pipeline;

pub use pairing::PairingStore;
pub use server::{admin_router, router, AppState};
