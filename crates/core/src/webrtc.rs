//! webrtc.rs — peer connection via webrtc-rs
//!
//! Konsep Rust:
//! - Arc: shared ownership peer connection (dipakai di beberapa callback)
//! - async/await + tokio: non-blocking I/O, mirip Promise di JS tapi zero-cost
//! - ownership move ke closure: `move ||` memindahkan ownership ke callback

use std::sync::Arc;
use anyhow::Result;
use tracing::{info, warn};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

pub type PeerConnection = Arc<RTCPeerConnection>;

/// Buat MediaEngine yang support H.264 + VP8 (fallback)
fn build_api() -> Result<webrtc::api::API> {
    let mut m = MediaEngine::default();
    // H.264 — codec HP
    m.register_codec(
        webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: "video/H264".to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: 102,
            ..Default::default()
        },
        webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video,
    )?;
    // VP8 fallback (browser test client)
    m.register_codec(
        webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: "video/VP8".to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: 96,
            ..Default::default()
        },
        webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Video,
    )?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;
    Ok(APIBuilder::new().with_media_engine(m).with_interceptor_registry(registry).build())
}

/// Buat peer connection baru (host ICE only — tanpa STUN/TURN, PRD 8.5)
pub async fn new_peer_connection() -> Result<PeerConnection> {
    let api = build_api()?;
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer { urls: vec![], ..Default::default() }],
        ..Default::default()
    };
    let pc = api.new_peer_connection(config).await?;
    Ok(Arc::new(pc))
}

/// Helper: buat track lokal untuk menjawab offer (jika desktop juga kirim, tidak di M1)
pub fn create_video_track() -> Arc<TrackLocalStaticSample> {
    Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: "video/H264".to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f".to_owned(),
            rtcp_feedback: vec![],
        },
        "video".to_owned(),
        "sidecam".to_owned(),
    ))
}

/// Pasang handler track masuk — dipanggil saat HP mengirim video.
/// `on_frame_bytes` dipanggil per RTP packet payload (ponytail: reassembly + decode di M2, M1 log saja)
pub async fn on_track<F>(pc: &PeerConnection, on_packet: F)
where
    F: FnMut(Vec<u8>, u32) + Send + 'static,
{
    let pc2 = Arc::clone(pc);
    pc.on_track(Box::new(move |track, _receiver, _transceiver| {
        let mime = track.codec().capability.mime_type.clone();
        info!("[webrtc] track masuk: {}", mime);
        let track2 = track.clone();
        Box::pin(async move {
            // Baca RTP loop — untuk M1 cukup hitung paket
            // Real decode di M2; M1 butuh stabilitas koneksi
            let mut count: u64 = 0;
            loop {
                match track2.read_rtp().await {
                    Ok((pkt, _attr)) => {
                        count += 1;
                        if count % 90 == 0 {
                            info!("[webrtc] paket video #{} size={} mime={}", count, pkt.payload.len(), mime);
                        }
                        // ponytail: decode H264 payload -> Frame -> vcam.push_frame di M2
                        // on_packet(pkt.payload.to_vec(), pkt.header.sequence_number);
                        let _ = &pkt;
                    }
                    Err(e) => {
                        warn!("[webrtc] read_rtp error: {}", e);
                        break;
                    }
                }
            }
            let _ = pc2;
            let _ = on_packet;
        })
    }));
}
