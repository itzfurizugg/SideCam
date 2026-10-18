//! server.rs — axum WebSocket signaling server

use std::sync::Arc;
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

// ── state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub pairing: PairingStore,
    pub host: String,
}

// ── query ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SignalQuery {
    pub token: Option<String>,
}

// ── router ───────────────────────────────────────────────────────────────

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

// ── WebSocket upgrade ────────────────────────────────────────────────────

async fn signal_handler(
    ws: WebSocketUpgrade,
    Query(q): Query<SignalQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let token_qs = q.token.clone();
    ws.on_upgrade(move |socket| handle_socket(socket, state, token_qs))
}

async fn handle_socket(mut socket: WebSocket, state: AppState, token_qs: Option<String>) {
    info!("[signal] koneksi baru token_qs={:?}", token_qs);

    // Auth via query param (M1 simple). M3: pakai pesan auth + mDNS.
    let mut authenticated = false;
    let mut device_name = String::new();

    // Jika token di query string, validasi langsung
    if let Some(t) = token_qs.as_deref() {
        if state.pairing.is_token_valid(t) {
            match state.pairing.consume_token(t) {
                Ok(()) => {
                    authenticated = true;
                    info!("[signal] token QS valid, authenticated");
                    // kirim auth ok
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
            // token invalid — tapi jangan tutup dulu, tunggu pesan auth (agar browser test bisa kirim tanpa token)
            info!("[signal] token QS invalid, tunggu pesan auth");
        }
    }

    // Untuk M1: jika tidak ada token_qs, izinkan koneksi tanpa auth (LAN only, dev).
    // Di M3 ini akan wajib auth.
    if token_qs.is_none() {
        authenticated = true;
    }

    // ── loop pesan ───────────────────────────────────────────────────────
    // Simpan peer connection per socket
    let mut peer: Option<Arc<webrtc::peer_connection::RTCPeerConnection>> = None;

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
                // Echo hello
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
                match handle_offer(o, &mut peer).await {
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
                    // echo back untuk trickle (jika server juga punya candidate)
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

    info!("[signal] socket selesai device={}", device_name);
}

async fn handle_offer(
    offer: crate::signaling::SdpPayload,
    peer_out: &mut Option<Arc<webrtc::peer_connection::RTCPeerConnection>>,
) -> anyhow::Result<String> {
    use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

    let pc = crate::webrtc::new_peer_connection().await?;

    // Pasang handler track sebelum set_remote_description
    // ponytail: on_track perlu di-setup sebelum answer agar tidak miss
    {
        let pc2 = Arc::clone(&pc);
        pc.on_peer_connection_state_change(Box::new(move |s| {
            info!("[webrtc] state: {:?}", s);
            Box::pin(async move {})
        }));
        // ICE candidate -> untuk M1 host only, tidak perlu trickle balik.
        // Diaktifkan di M3 untuk UX.
        pc.on_ice_candidate(Box::new(move |_c| {
            Box::pin(async move {})
        }));
        let _ = pc2;
    }

    // Track handler yang log paket
    // webrtc::on_track butuh &PeerConnection; kita set manual di sini
    {
        pc.on_track(Box::new(move |track, _recv, _trans| {
            let mime = track.codec().capability.mime_type.clone();
            info!("[webrtc] on_track mime={}", mime);
            Box::pin(async move {
                let mut n: u64 = 0;
                loop {
                    match track.read_rtp().await {
                        Ok((pkt, _)) => {
                            n += 1;
                            if n % 90 == 0 {
                                info!("[webrtc] rtp #{} payload={} seq={}", n, pkt.payload.len(), pkt.header.sequence_number);
                            }
                        }
                        Err(e) => { warn!("[webrtc] read_rtp err: {}", e); break; }
                    }
                }
            })
        }));
    }

    let desc = RTCSessionDescription::offer(offer.sdp)?;
    pc.set_remote_description(desc).await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;

    // Simpan pc agar tidak drop
    *peer_out = Some(pc);

    Ok(answer.sdp)
}
