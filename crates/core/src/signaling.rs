//! signaling.rs — pesan WebSocket JSON sesuai PRD 8.4
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SignalMessage {
    Hello(HelloPayload),
    Auth(AuthPayload),
    Offer(SdpPayload),
    Answer(SdpPayload),
    Ice(IcePayload),
    Config(ConfigPayload),
    Bye(ByePayload),
    // server -> client
    AuthOk { secret: Option<String> },
    AuthErr { reason: String },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloPayload {
    pub device_id: String,
    pub name: String,
    pub app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdpPayload {
    pub sdp: String,
    pub sdp_type: String, // "offer" | "answer"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcePayload {
    pub candidate: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdp_mid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdp_mline_index: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPayload {
    pub resolution: String, // "720p" | "1080p"
    pub fps: u32,
    pub facing: String, // "front" | "back"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByePayload {
    pub reason: String,
}

impl SignalMessage {
    pub fn parse(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hello_roundtrip() {
        let m = SignalMessage::Hello(HelloPayload { device_id: "d1".into(), name: "HP".into(), app_version: "1.0".into() });
        let j = m.to_json();
        let p = SignalMessage::parse(&j).unwrap();
        assert!(matches!(p, SignalMessage::Hello(_)));
    }
    #[test]
    fn offer_roundtrip() {
        let m = SignalMessage::Offer(SdpPayload { sdp: "v=0".into(), sdp_type: "offer".into() });
        let j = m.to_json();
        assert!(j.contains("offer"));
        SignalMessage::parse(&j).unwrap();
    }
}
