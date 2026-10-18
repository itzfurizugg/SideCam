# installer

## M4 — Windows
- `install.ps1` — daftarkan DLL virtual camera (`regsvr32 sidecam_vcam.dll`, butuh admin sekali), tambah firewall rule Private port 8080, cleanup saat `-Uninstall`.
- `sidecam.iss` — (stub) Inno Setup. Build real butuh Windows + Inno Setup 6.

## Linux dev
```bash
sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=SideCam exclusive_caps=1
cargo run --bin sidecam-receiver -- --headless --vcam-device /dev/video10
# test
curl -sk https://127.0.0.1:8080/health
ffplay /dev/video10
```

## Uninstall
```powershell
powershell -ExecutionPolicy Bypass -File installer/install.ps1 -Uninstall
```
