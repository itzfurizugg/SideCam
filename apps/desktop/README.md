# apps/desktop

- **M1**: `test-client.html` — browser test client tanpa build step, serve di `http://IP:8080/` oleh `sidecam-receiver`.
- **M3**: Tauri + React + TypeScript (QR pairing, preview, daftar perangkat). Webview hanya UI, WebRTC tetap di Rust.

## M1 Test

Buka `http://IP:8080/` di HP/laptop di Wi-Fi yang sama → isi Host/Token → Mulai Kirim.

File `test-client.html` juga bisa dibuka langsung (file://) dan isi host manual.
