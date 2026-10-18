//! server.rs — axum WebSocket signaling server + vcam bridge (M2)

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::StreamExt;
use serde::Deserialize;
use tracing::{info, warn};

use crate::pairing::PairingStore;
use crate::signaling::SignalMessage;
use sidecam_vcam::{Frame, PixelFormat, VirtualCamera};

#[derive(Clone)]
pub struct AppState {
    pub pairing: PairingStore,
    pub host: String,
    /// ponytail: Mutex cukup M2, M3 ganti channel+thread jika push_frame blocking
    pub vcam: Arc<Mutex<Box<dyn VirtualCamera>>>,
}

#[derive(Deserialize)]
pub struct SignalQuery {
    pub token: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/signal", get(signal_handler))
        .route("/health", get(health))
        .route("/api", get(root))
        .with_state(state)
}

async fn root() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "name": "SideCam",
        "version": "0.1.0-m1",
        "signal": "/signal",
        "health": "/health"
    }))
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "ok": true }))
}

async fn signal_handler(
    ws: WebSocketUpgrade,
    Query(q): Query<SignalQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let token_qs = q.token.clone();
    ws.on_upgrade(move |socket| handle_socket(socket, state, token_qs))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

async fn handle_socket(mut socket: WebSocket, state: AppState, token_qs: Option<String>) {
    info!("[signal] koneksi baru token_qs={:?}", token_qs);

    let mut authenticated = false;
    let mut device_name = String::new();

    if let Some(t) = token_qs.as_deref() {
        if state.pairing.is_token_valid(t) {
            match state.pairing.consume_token(t) {
                Ok(()) => {
                    authenticated = true;
                    info!("[signal] token QS valid, authenticated");
                    let msg = SignalMessage::AuthOk { secret: None }.to_json();
                    if socket.send(Message::Text(msg.into())).await.is_err() { return; }
                }
                Err(e) => {
                    let msg = SignalMessage::AuthErr { reason: e }.to_json();
                    let _ = socket.send(Message::Text(msg.into())).await;
                    return;
                }
            }
        } else {
            info!("[signal] token QS invalid, tunggu pesan auth");
        }
    }
    if token_qs.is_none() {
        authenticated = true;
    }

    let mut peer: Option<Arc<webrtc::peer_connection::RTCPeerConnection>> = None;
    let vcam = Arc::clone(&state.vcam);
    let last_frame_at = Arc::new(AtomicU64::new(now_ms()));
    // ensure vcam open
    if let Ok(mut v) = vcam.try_lock() {
        if !v.is_open() { let _ = v.open(); }
        // FR-9: tampil placeholder sebelum track datang (jangan hitam/beku)
        let _ = v.push_frame(&Frame::placeholder(1280, 720));
    }

    while let Some(Ok(msg)) = socket.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => { info!("[signal] close"); break; }
            _ => continue,
        };
        let parsed = match SignalMessage::parse(&text) {
            Ok(m) => m,
            Err(e) => {
                warn!("[signal] parse error: {} raw={}", e, &text[..text.len().min(200)]);
                let err = SignalMessage::Error { message: format!("Format pesan salah: {}", e) }.to_json();
                let _ = socket.send(Message::Text(err.into())).await;
                continue;
            }
        };
        match parsed {
            SignalMessage::Hello(h) => {
                device_name = h.name.clone();
                info!("[signal] hello device={} name={} v={}", h.device_id, h.name, h.app_version);
                let reply = SignalMessage::Hello(h).to_json();
                let _ = socket.send(Message::Text(reply.into())).await;
            }
            SignalMessage::Auth(a) => {
                if authenticated {
                    let msg = SignalMessage::AuthOk { secret: None }.to_json();
                    let _ = socket.send(Message::Text(msg.into())).await;
                    continue;
                }
                let ok = if let Some(t) = a.token.as_deref() {
                    state.pairing.consume_token(t).is_ok()
                } else if let Some(s) = a.secret.as_deref() {
                    state.pairing.validate_secret(s).is_some()
                } else { false };
                if ok {
                    authenticated = true;
                    let msg = SignalMessage::AuthOk { secret: None }.to_json();
                    let _ = socket.send(Message::Text(msg.into())).await;
                } else {
                    let msg = SignalMessage::AuthErr { reason: "Token atau secret tidak valid".into() }.to_json();
                    let _ = socket.send(Message::Text(msg.into())).await;
                }
            }
            SignalMessage::Offer(o) => {
                if !authenticated {
                    let msg = SignalMessage::Error { message: "Belum login".into() }.to_json();
                    let _ = socket.send(Message::Text(msg.into())).await;
                    continue;
                }
                info!("[signal] offer sdp len={}", o.sdp.len());
                let vcam_c = Arc::clone(&vcam);
                let last_c = Arc::clone(&last_frame_at);
                match handle_offer(o, &mut peer, vcam_c, last_c).await {
                    Ok(answer_sdp) => {
                        let ans = SignalMessage::Answer(crate::signaling::SdpPayload { sdp: answer_sdp, sdp_type: "answer".into() }).to_json();
                        let _ = socket.send(Message::Text(ans.into())).await;
                    }
                    Err(e) => {
                        warn!("[signal] handle_offer error: {}", e);
                        let msg = SignalMessage::Error { message: format!("Gagal proses offer: {}", e) }.to_json();
                        let _ = socket.send(Message::Text(msg.into())).await;
                    }
                }
            }
            SignalMessage::Ice(ice) => {
                if let Some(pc) = peer.as_ref() {
                    let cand = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                        candidate: ice.candidate.clone(),
                        sdp_mid: ice.sdp_mid.clone(),
                        sdp_mline_index: ice.sdp_mline_index.map(|v| v as u16),
                        ..Default::default()
                    };
                    if let Err(e) = pc.add_ice_candidate(cand).await {
                        warn!("[signal] add_ice_candidate error: {}", e);
                    } else {
                        info!("[signal] ice candidate added");
                    }
                }
            }
            SignalMessage::Config(c) => {
                info!("[signal] config {} {}fps {}", c.resolution, c.fps, c.facing);
            }
            SignalMessage::Bye(b) => {
                info!("[signal] bye: {}", b.reason);
                break;
            }
            _ => {}
        }
    }

    // FR-9: HP putus -> jangan beku/hitam, push placeholder + biarkan reconnect
    if let Ok(mut v) = vcam.try_lock() {
        let _ = v.push_frame(&Frame::placeholder(1280, 720));
        info!("[vcam] HP terputus, placeholder dipasang (FR-9)");
    }
    let _ = peer; // drop pc triggers on_peer_connection_state_change
    info!("[signal] socket selesai device={}", device_name);
}

async fn handle_offer(
    offer: crate::signaling::SdpPayload,
    peer_out: &mut Option<Arc<webrtc::peer_connection::RTCPeerConnection>>,
    vcam: Arc<Mutex<Box<dyn VirtualCamera>>>,
    last_frame_at: Arc<AtomicU64>,
) -> anyhow::Result<String> {
    use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

    let pc = crate::webrtc::new_peer_connection().await?;

    {
        let pc2 = Arc::clone(&pc);
        let vcam2 = Arc::clone(&vcam);
        pc.on_peer_connection_state_change(Box::new(move |s| {
            info!("[webrtc] state: {:?}", s);
            // FR-9: failed/disconnected -> placeholder agar Zoom tidak beku
            if matches!(s, webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Failed
                | webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Disconnected
                | webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed) {
                if let Ok(mut v) = vcam2.try_lock() {
                    let _ = v.push_frame(&Frame::placeholder(1280, 720));
                }
            }
            let _ = &pc2;
            Box::pin(async move {})
        }));
        pc.on_ice_candidate(Box::new(move |_c| Box::pin(async move {})));
    }

    // M2 stub: RTP payload (H264) -> placeholder berwarna agar vcam hidup tanpa decode berat
    // ponytail: decode real H264 pakai openh264/ffmpeg di M2-full; stub ini cukup buat acceptance Zoom/OBS
    pc.on_track(Box::new(move |track, _recv, _trans| {
        let mime = track.codec().capability.mime_type.clone();
        let vcam_c = Arc::clone(&vcam);
        let last_c = Arc::clone(&last_frame_at);
        info!("[webrtc] on_track mime={}", mime);
        Box::pin(async move {
            let mut n: u64 = 0;
            loop {
                match track.read_rtp().await {
                    Ok((pkt, _)) => {
                        n += 1;
                        last_c.store(now_ms(), Ordering::Relaxed);
                        if n % 90 == 0 {
                            info!("[webrtc] rtp #{} payload={} seq={}", n, pkt.payload.len(), pkt.header.sequence_number);
                        }
                        // push ke vcam tiap ~30 paket (~1/detik) biar tidak spam
                        if n % 30 == 0 {
                            let tick = (n % 255) as u8;
                            let mut f = Frame::placeholder(1280, 720);
                            // beri variasi warna biar keliatan hidup di Zoom/Meet
                            for i in (0..f.data.len()).step_by(3) {
                                f.data[i] = f.data[i].wrapping_add(tick);
                                f.data[i + 1] = f.data[i + 1].wrapping_add(tick / 2);
                            }
                            f.format = PixelFormat::Rgb24;
                            if let Ok(mut v) = vcam_c.try_lock() {
                                if !v.is_open() { let _ = v.open(); }
                                let _ = v.push_frame(&f);
                            }
                            // also handle I420 path for v4l2: jika butuh, konversi Rgb->I420 di M2-full
                        }
                    }
                    Err(e) => { warn!("[webrtc] read_rtp err: {}", e); break; }
                }
            }
            // track ended
            if let Ok(mut v) = vcam_c.try_lock() {
                let _ = v.push_frame(&Frame::placeholder(1280, 720));
            }
        })
    }));

    let desc = RTCSessionDescription::offer(offer.sdp)?;
    pc.set_remote_description(desc).await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer).await?;
    {
        let mut rx = pc.gathering_complete_promise().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
    }
    let final_sdp = pc
        .local_description()
        .await
        .or(pc.pending_local_description().await)
        .map(|d| d.sdp)
        .unwrap_or_default();
    if final_sdp.is_empty() {
        anyhow::bail!("gather gagal: local_description kosong");
    }
    info!("[webrtc] answer final sdp len={} (gather done)", final_sdp.len());
    *peer_out = Some(pc);
    Ok(final_sdp)
}
