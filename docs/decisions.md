# Keputusan Arsitektur

## M1

- **WebRTC crate**: `webrtc` 0.11 (webrtc-rs) dipilih karena pure Rust, API tokio. Fallback `libdatachannel` disiapkan via trait jika webrtc-rs tidak stabil (risiko PRD 12).
- **Signaling**: `axum` 0.7 + `tokio-tungstenite` via `axum::extract::ws`. Tidak pakai `actix-web` agar satu runtime tokio.
- **Token 2 menit**: in-memory `HashMap` + `Instant::now()`, cleanup lazy saat `generate_token`. Persist belum (M3).
- **LAN only**: `RTCIceServer` kosong (host candidates saja), tanpa STUN/TURN. Security PRD 10 terpenuhi.
- **VirtualCamera trait**: `crates/vcam` definisi sejak M1, isolasi Windows di `native/vcam-windows`. Linux `v4l2loopback` `/dev/video10`, fallback `Placeholder` agar M1 bisa jalan tanpa device.
- **Preview**: `minifb` 0.27 untuk jendela sederhana, skip jika `DISPLAY` tidak ada (headless). WebRTC decode real di M2, M1 log `read_rtp` saja.
- **Test client**: `apps/desktop/test-client.html` HTML vanilla (tanpa build step) untuk uji E2E tanpa Flutter. Flutter dipisah repo terpisah sesuai konfirmasi.

## M2

- **VirtualCamera wiring**: `server.rs` `handle_offer` `on_track` → `vcam.push_frame(Frame::placeholder)` tiap 30 paket. Tanpa decode berat, cukup buat acceptance Zoom/OBS (FR-4/FR-9). Placeholder FR-9 juga dipasang saat `handle_socket` close + `on_peer_connection_state_change` Failed/Disconnected. `ponytail:` decode real H264 `openh264`/`ffmpeg-next` di M2-full.
- **Format**: `PlaceholderCamera` `Rgb24`, `V4l2LoopbackCamera` tulis `&[u8]` langsung. `DirectShowCamera` stub log di Linux.

## M3

- **QR**: `qrcode` 0.14 render SVG inline + `GET /qr.svg`. JSON `{v,host,port,token,device_name,device_id}` PRD 8.2, token 2 menit.
- **mDNS**: `mdns-sd` service `_sidecam._tcp.local.` TXT `v=1,name,id`. Advert thread keep-alive.
- **Persist**: `PairingStore` persist `sidecam_devices.json` di `%APPDATA%/SideCam` (Windows) atau `~/.config/sidecam` (Linux). `Default` load existing secrets. `ponytail:` belum enkripsi at-rest, add when threat model device theft.
- **Devices API**: `GET /api/devices`, `DELETE /api/devices/:id` (revoke FR-7).

## M4

- **Installer**: `installer/install.ps1` `regsvr32` + `New-NetFirewallRule Private 8080` + uninstall cleanup. Inno Setup stub. Di Linux `modprobe v4l2loopback`.
- **TLS default**: `rcgen` self-signed CN `host_ip` + `localhost`. `https/wss` bypass SecureContext HP. `--no-tls` fallback http (Chrome HP butuh `chrome://flags` flag).

## M5

- **Polish**: frontend Wise fully skinned (`#9fe870` pill 24px, `Inter 900` hero, `canvas-soft #e8ebe6`). QR + devices band di `test-client.html`.
- **DirectShow note**: build real butuh Windows SDK; lisensi Softcam/UnityCapture cek manual.

## Pending

- `openh264` vs `ffmpeg-next` decode — ukur CPU 720p30 di M2-full.
- DirectShow DLL build + `MFCreateVirtualCamera` Windows 11 path.
