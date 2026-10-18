# Pengukuran M1

Isi setelah uji 720p30 stabil 10 menit.

## Cara ukur

1. Jalankan receiver:
   ```bash
   cargo run --bin sidecam-receiver -- --headless 2>&1 | tee m1.log
   ```
2. Buka `http://IP:8080/` di HP (atau laptop kedua) + klik Mulai Kirim (720p).
3. Biarkan 10 menit. Catat:
   - Log `rtp #...` tiap ~3 detik (90 paket) — hitung fps gap.
   - Browser `getStats()` di test-client (fps, bytesSent).
   - `top` / `htop` CPU receiver.

## Template hasil

| Metrik | Nilai | Keterangan |
|--------|-------|------------|
| Durasi uji |  | menit |
| Resolusi | 1280x720 30fps | |
| Wi-Fi | 5GHz / 2.4GHz | |
| Latensi offer→answer |  | ms |
| Latensi RTP (est.) |  | ms |
| CPU receiver (avg) |  | % |
| Paket hilang |  | % |
| Reconnect |  |  |

> Latensi end-to-end (kamera → preview) target <200ms (PRD 10). M1 belum punya render frame, latensi diukur via `getStats` jitter + RTT WebRTC.

## Log sample

```
[webrtc] on_track mime=video/H264
[webrtc] rtp #90 payload=1200 seq=90
...
```
