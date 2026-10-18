# native/vcam-windows

C++ DirectShow source filter untuk SideCam (Windows).

## Status M5
- Stub FFI siap: `crates/vcam` `DirectShowCamera` panggil `sidecam_vcam_open/close/push_frame` via `LoadLibrary`.
- Build real DLL butuh Windows + DirectShow SDK. Di Linux stub log saja.
- Alternatif lisensi: Softcam/UnityCapture — cek sebelum pakai. Jika tidak cocok, tulis ulang minimal filter (PRD 12).
- Fase lanjut: `MFCreateVirtualCamera` Windows 11.

## Trait mapping
`crates/vcam::VirtualCamera` → `sidecam_vcam_*` exports.

## Build Windows
```bat
cl /LD vcam.cpp /I "%WindowsSdkDir%\Include" strmiids.lib
regsvr32 sidecam_vcam.dll
```
