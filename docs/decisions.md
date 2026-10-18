# Keputusan Arsitektur

## M1

- **WebRTC crate**: `webrtc` 0.11 (webrtc-rs) dipilih karena pure Rust, API tokio. Fallback `libdatachannel` disiapkan via trait jika webrtc-rs tidak stabil (risiko PRD 12).
- **Signaling**: `axum` 0.7 + `tokio-tungstenite` via `axum::extract::ws`. Tidak pakai `actix-web` agar satu runtime tokio.
- **Token 2 menit**: in-memory `HashMap` + `Instant::now()`, cleanup lazy saat `generate_token`. Persist belum (M3).
- **LAN only**: `RTCIceServer` kosong (host candidates saja), tanpa STUN/TURN. Security PRD 10 terpenuhi.
- **VirtualCamera trait**: `crates/vcam` definisi sejak M1, isolasi Windows di `native/vcam-windows`. Linux `v4l2loopback` `/dev/video10`, fallback `Placeholder` agar M1 bisa jalan tanpa device.
- **Preview**: `minifb` 0.27 untuk jendela sederhana, skip jika `DISPLAY` tidak ada (headless). WebRTC decode real di M2, M1 log `read_rtp` saja.
- **Test client**: `apps/desktop/test-client.html` HTML vanilla (tanpa build step) untuk uji E2E tanpa Flutter. Flutter dipisah repo terpisah sesuai konfirmasi.

## Pending

- `openh264` vs `ffmpeg-next` untuk decode H.264 di M2 — ukur CPU 720p30.
- DirectShow filter: cek lisensi Softcam/UnityCapture sebelum pakai (PRD constraint).
