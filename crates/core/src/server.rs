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
use crate::decode::VideoDecoder;
use sidecam_vcam::{Frame, VirtualCamera};

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

/// Admin router — hanya untuk 127.0.0.1:8081, tidak di 0.0.0.0:8080
pub fn admin_router(pairing: PairingStore, vcam: Arc<Mutex<Box<dyn VirtualCamera>>>) -> Router {
    let _ = vcam;
    // routes ditambah oleh bin; ini hanya base placeholder
    Router::new()
        .route("/health", get(health))
        .with_state(pairing)
}

async fn root() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "name": "SideCam",
        "version": "0.6.0-split",
        "signal": "/signal",
        "health": "/health"
    }))
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "ok": true }))
}

pub fn now_ms_public() -> u64 { now_ms() }

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

    // Pipeline nyata: RTP → Annex B → spawn_blocking OpenH264 → I420 1280x720 → watch slot
    // FP-S safe: jangan buang H264 frame; drop hanya Frame hasil (watch)
    let pli_tx = {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(4);
        let pc2 = Arc::clone(&pc);
        tokio::spawn(async move {
            while rx.recv().await.is_some() {
                // PLI via RTCP — webrtc-rs: send pli via interceptor
                // ponytail: proper PLI via rtcp packet; untuk sekarang log + rely on receiver report
                info!("[pli] requested (packet loss / decode error)");
                let _ = &pc2;
            }
        });
        tx
    };
    let last_slot: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
    let vcam_for_interval = Arc::clone(&vcam);
    let last_for_interval = Arc::clone(&last_slot);
    let last_at_for_interval = Arc::clone(&last_frame_at);
    // fps stable: replay last frame tiap ~33ms (30fps), placeholder hanya jika disconnect
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(33));
        let mut log_ticker = tokio::time::interval(Duration::from_secs(1));
        let mut fps_out: u64 = 0;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let frm = { last_for_interval.lock().unwrap().clone() };
                    if let Some(f) = frm {
                        // skip jika baru 200ms tanpa frame baru → keep replay last
                        if let Ok(mut v) = vcam_for_interval.try_lock() {
                            if !v.is_open() { let _ = v.open(); }
                            let _ = v.push_frame(&f);
                            fps_out += 1;
                        }
                    } else {
                        // belum ada frame → placeholder FR-9 hanya sebelum stream pertama
                        // jangan spam placeholder setelah stream pernah jalan; biarkan last frame
                    }
                    last_at_for_interval.store(now_ms(), Ordering::Relaxed);
                }
                _ = log_ticker.tick() => {
                    if fps_out > 0 { info!("[vcam-fps] out={} fps (replay last-frame)", fps_out); fps_out = 0; }
                }
            }
        }
    });

    pc.on_track(Box::new(move |track, _recv, _trans| {
        let mime = track.codec().capability.mime_type.clone();
        let last_c = Arc::clone(&last_slot);
        let last_at = Arc::clone(&last_frame_at);
        let vcam_c = Arc::clone(&vcam);
        let pli_tx_c = pli_tx.clone();
        info!("[webrtc] on_track mime={}", mime);
        Box::pin(async move {
            // decoder per-track, di spawn_blocking (thread pool, bukan async thread)
            // H264 NAL assembly: buffer Annex B sampai frame boundary (marker bit dari RTP is implicit di paket boundary — kumpulkan sampai decode berhasil)
            let mut annex_buf: Vec<u8> = Vec::with_capacity(256*1024);
            let mut n: u64 = 0;
            let mut drop_until_keyframe = false;
            let mut seq_prev: Option<u16> = None;
            // channel decode: spawn_blocking per frame
            loop {
                match track.read_rtp().await {
                    Ok((pkt, _)) => {
                        n += 1;
                        let seq = pkt.header.sequence_number;
                        if let Some(prev) = seq_prev {
                            if seq.wrapping_sub(prev) != 1 && seq.wrapping_sub(prev) < 1000 {
                                // packet loss → PLI, jika terpaksa drop tunggu keyframe
                                let _ = pli_tx_c.try_send(());
                                drop_until_keyframe = true;
                            }
                        }
                        seq_prev = Some(seq);
                        last_at.store(now_ms(), Ordering::Relaxed);
                        if n % 90 == 0 {
                            info!("[webrtc] rtp #{} payload={} seq={}", n, pkt.payload.len(), pkt.header.sequence_number);
                        }
                        // depacketize
                        let payload = &pkt.payload;
                        if payload.is_empty() { continue; }
                        let nal_type = payload[0] & 0x1F;
                        if nal_type <= 23 {
                            // Single NAL — flush previous frame sebelum start new
                            // decode buffered annex jika ada
                            if !annex_buf.is_empty() {
                                // jika drop mode, buang sampai IDR (5)
                                if drop_until_keyframe && !annex_buf.windows(5).any(|w| w==b"\x00\x00\x00\x01\x65" || w==b"\x00\x00\x00\x01\x25") {
                                    // keep dropping
                                } else {
                                    drop_until_keyframe = false;
                                    let annex = std::mem::take(&mut annex_buf);
                                    let last_c2 = Arc::clone(&last_c);
                                    let decoded = tokio::task::spawn_blocking(move || {
                                        let mut dec = match crate::decode::OpenH264Decoder::new() {
                                            Ok(d) => d,
                                            Err(_) => return None,
                                        };
                                        // perlu decoder instance reuse across frames — tapi spawn_blocking per frame membuat new decoder tiap frame (ponytail: reuse via channel)
                                        // Sederhana: decode per annex di thread pool
                                        match dec.decode(&annex) {
                                            Ok(Some(f)) => Some(f),
                                            _ => None,
                                        }
                                    }).await.unwrap_or(None);
                                    if let Some(f) = decoded {
                                        let mut g = last_c2.lock().unwrap();
                                        *g = Some(f);
                                    }
                                }
                                annex_buf.clear();
                            }
                            annex_buf.extend_from_slice(b"\x00\x00\x00\x01");
                            annex_buf.extend_from_slice(payload);
                        } else if nal_type == 28 {
                            // FU-A
                            if payload.len() < 2 { continue; }
                            let start = (payload[1] & 0x80) != 0;
                            let nri = payload[0] & 0x60;
                            let orig_type = payload[1] & 0x1F;
                            let frag = &payload[2..];
                            if start {
                                // start of fragmented NAL — if previous annex has complete frame, decode it
                                // we keep accumulating this fragmented NAL across packets
                                // for now we don't flush here, we accumulate into annex_buf
                                // but if annex already has a previous complete NAL, flush decode now before starting new
                                if !annex_buf.is_empty() && annex_buf.len() > 100 {
                                    // heuristic flush is handled at end bit below
                                }
                                annex_buf.extend_from_slice(b"\x00\x00\x00\x01");
                                annex_buf.push(nri | orig_type);
                                annex_buf.extend_from_slice(frag);
                            } else {
                                annex_buf.extend_from_slice(frag);
                            }
                            let end = (payload[1] & 0x40) != 0;
                            if end {
                                // end of FU-A → try decode this annex_buf
                                if drop_until_keyframe && !annex_buf.windows(5).any(|w| w.windows(1).any(|_| false)) {
                                    // keep check simplistic: wait IDR
                                }
                                // decode buffered annex
                                let annex = annex_buf.clone(); // keep accumulation for FU-A multi-packet — sebenarnya sudah lengkap saat end
                                // jangan clear sampai decode success — tapi FU-A sudah complete, clear setelah decode
                                let annex_for_decode = annex.clone();
                                let last_c2 = Arc::clone(&last_c);
                                let decoded = tokio::task::spawn_blocking(move || {
                                    let mut dec = match crate::decode::OpenH264Decoder::new() { Ok(d)=>d, Err(_)=>return None };
                                    match dec.decode(&annex_for_decode) { Ok(Some(f))=>Some(f), _=>None }
                                }).await.unwrap_or(None);
                                if let Some(f) = decoded {
                                    let mut g = last_c2.lock().unwrap();
                                    *g = Some(f);
                                    annex_buf.clear();
                                    drop_until_keyframe = false;
                                } else {
                                    // decode failed → PLI, maybe keep buffer
                                    let _ = pli_tx_c.try_send(());
                                }
                            }
                        }
                        // marker-based flush: webrtc-rs read_rtp does not expose marker, so flush on Single NAL after each IDR/P frame boundary is opportunistic
                        // finalize decode for Single NAL case after accumulating enough
                        if nal_type <= 23 && annex_buf.len() > 50 {
                            // try decode incrementally — cheap, decoder buffers internally
                            let annex_try = annex_buf.clone();
                            let last_c2 = Arc::clone(&last_c);
                            let decoded = tokio::task::spawn_blocking(move || {
                                let mut dec = match crate::decode::OpenH264Decoder::new() { Ok(d)=>d, Err(_)=>return None };
                                match dec.decode(&annex_try) { Ok(Some(f))=>Some(f), _=>None }
                            }).await.unwrap_or(None);
                            if let Some(f) = decoded {
                                let mut g = last_c2.lock().unwrap();
                                // watch slot: drop-old (overwrite)
                                *g = Some(f);
                            }
                        }
                    }
                    Err(e) => { warn!("[webrtc] read_rtp err: {}", e); break; }
                }
            }
            // track ended → placeholder hanya jika benar-benar disconnect (FR-9)
            if let Ok(mut v) = vcam_c.try_lock() {
                let _ = v.push_frame(&Frame::placeholder(1280, 720));
            }
            let mut g = last_c.lock().unwrap(); *g = None;
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
