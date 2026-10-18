# SideCam — RESULT (M1..M5)

## Ringkasan
HP jadi webcam Windows lewat Wi-Fi LAN only. Stack: `webrtc-rs` + `axum` + `tokio`, `rcgen` self-signed TLS, `qrcode` SVG, `mdns-sd`, trait `VirtualCamera` (Placeholder / v4l2loopback / DirectShow stub), frontend Wise kit.

## Milestone
| M | Deliverable | Status |
|---|-------------|--------|
| M1 | Signaling + WebRTC 720p30, preview minifb, token 2 menit, test-client HTML | ✅ `cargo test` 6 passed |
| M2 | VirtualCamera trait + wiring HP→vcam, FR-9 placeholder saat putus | ✅ stub (tanpa decode berat) |
| M3 | QR pairing (`GET /qr.svg` SVG), mDNS `_sidecam._tcp.local`, secret persist `sidecam_devices.json`, `GET/DELETE /api/devices` revoke | ✅ |
| M4 | `installer/install.ps1` regsvr32 + firewall Private 8080, Linux `v4l2loopback` | ✅ (DirectShow DLL build butuh Windows) |
| M5 | Frontend Wise kit, QR+devices band, polish | ✅ |

## Cara jalankan
```bash
# dev Linux
sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=SideCam exclusive_caps=1
cargo run --bin sidecam-receiver -- --headless --vcam-device /dev/video10
# atau placeholder tanpa device
cargo run --bin sidecam-receiver -- --headless --vcam-device placeholder
# headless tanpa preview jendela (default)
# preview jendela: tambah --preview atau SIDECAM_PREVIEW=1 (butuh DISPLAY)

# tanpa TLS (Chrome HP butuh flag)
cargo run --bin sidecam-receiver -- --no-tls --headless --vcam-device /dev/video10
```

## URL (default port 8080)
- `https://IP:8080/` — test client Wise (HP + laptop satu Wi-Fi). Self-signed → HP tap Advanced → Proceed. Chrome http butuh `chrome://flags#unsafely-treat-insecure-origin-as-secure` = `http://IP:8080`.
- `https://IP:8080/qr.svg` — QR pairing langsung.
- `https://IP:8080/health` — `{ok:true}`
- `https://IP:8080/api/info` — `{name,version,devices}`
- `https://IP:8080/api/devices` — list perangkat tepercaya
- `DELETE /api/devices/:id` — revoke
- `wss://IP:8080/signal?token=...` — WebSocket signaling

## HP → laptop flow
1. Laptop `cargo run ...` → catat IP + token di terminal / QR di `/` atau `/qr.svg`.
2. HP buka `https://IP:8080` (satu Wi-Fi) → Allow kamera → Mulai Kirim.
3. Zoom/Meet/Teams pilih kamera **SideCam**. Putus → placeholder gradient (FR-9), reconnect scan QR lagi atau pakai secret persist.

## vcam
- Linux: `/dev/video10` via `v4l2loopback` (`exclusive_caps=1`). Test `ffplay /dev/video10`.
- Windows: `DirectShowCamera` stub log; build DLL butuh Windows SDK + `regsvr32`. `installer/install.ps1` handle daftar + firewall.
- Placeholder: gradient `Rgb24` tiap 30 paket RTP (~1 fps push biar tidak spam).

## Desain (DESIGN.md — Wise)
- Primary `#9fe870` pill CTA, `rounded.xl 24px` card+button, canvas ` #e8ebe6` hero + `#ffffff` cards, ink `#0e0f0c`, `Inter 900` hero / `600` sub.
- Nav sticky, hero split (stack <768px), grid3 feature, footer ink.

## Persist
- `sidecam_devices.json` di `%APPDATA%/SideCam` (Windows) atau `~/.config/sidecam` (Linux). `ponytail:` belum enkripsi at-rest.

## Test
```bash
cargo test
curl -sk https://127.0.0.1:8080/health
curl -sk https://127.0.0.1:8080/api/devices | jq
```

## Skipped / upgrade path
- Decode H264 real (`openh264`/`ffmpeg-next`) → `Frame::I420` untuk `/dev/video10` (saat ini Rgb placeholder + I420 stub).
- DirectShow DLL real + `MFCreateVirtualCamera` Windows 11.
- Tauri+React `apps/desktop/src` dan Flutter `apps/mobile` (saat ini test-client HTML vanilla).
- QR auto-refresh token, HMR preview.

## Catatan
- HTML dibaca saat start — tiap edit `apps/desktop/test-client.html` restart receiver.
- TLS self-signed CN = `host_ip` + `localhost`. IP ganti → restart agar cert match.
