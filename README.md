# SideCam

HP jadi webcam Windows lewat Wi-Fi (LAN only). PRD: `docs/PRD.md`.

## Struktur (PRD 7.2)

```
apps/desktop/         # Tauri app (M3), + test-client.html untuk M1
crates/core/          # Rust: signaling, WebRTC, pairing
crates/vcam/          # Rust: trait VirtualCamera (Windows DirectShow / Linux v4l2loopback / Placeholder)
native/vcam-windows/  # C++ DirectShow filter (M2)
installer/            # Inno Setup / WiX (M4)
docs/                 # PRD, decisions, measurements, licenses
```

## M1 — Cara Build & Jalankan

### Syarat

- Rust stable (1.80+), Cargo workspace
- Ubuntu dev ok. `minifb` butuh `libx11-dev libxkbcommon-dev` untuk preview window. Headless tidak butuh.

```bash
sudo apt install libx11-dev libxkbcommon-dev libwayland-dev
```

### Build

```bash
cargo build --bin sidecam-receiver
cargo test
```

### Jalankan receiver

```bash
# headless (tanpa jendela, untuk server/CI)
cargo run --bin sidecam-receiver -- --headless

# dengan preview window (butuh DISPLAY)
cargo run --bin sidecam-receiver

# port & token manual
cargo run --bin sidecam-receiver -- --port 8080 --token mysecret --headless
```

Terminal akan cetak:
- `Pairing JSON` + `ws://IP:PORT/signal?token=...`
- `Health: http://IP:PORT/health`
- `Buka test client: http://IP:PORT/`

### Test dengan browser (ganti HP di M1)

1. Buka `http://IP:8080/` di HP dan laptop di Wi-Fi yang sama.
2. Isi Host = IP dari terminal, Token = token dari terminal (atau kosongkan jika receiver tanpa token check).
3. Klik **Mulai Kirim**, izinkan kamera.
4. Cek `cargo run` log: `offer sdp len=...`, `rtp #90 ...`, `PC state: connected`.

### Health check

```bash
curl http://localhost:8080/health
curl http://localhost:8080/
```

## Lisensi

MIT. Pin versi dependency di `Cargo.toml` workspace. Daftar lisensi: `docs/licenses.md`.

## Milestone

- **M1**: signaling + WebRTC 720p30, preview, token 2 menit (saat ini)
- **M2**: virtual camera (DirectShow + v4l2loopback)
- **M3**: QR + mDNS + Tauri UI
- **M4**: installer + firewall
