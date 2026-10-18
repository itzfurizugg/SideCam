# native/vcam-windows

C++ DirectShow filter untuk virtual camera SideCam di Windows.

> M1: stub saja. Real implementasi di M2.

## Rencana M2

- Basis: cek lisensi **Softcam** / **UnityCapture** sebelum pakai.
- Jika lisensi tidak cocok, tulis ulang minimal DirectShow source filter.
- Export C ABI (`extern "C"`) untuk dipanggil `crates/vcam` via FFI.
- Fase lanjut: `MFCreateVirtualCamera` untuk Windows 11.

## Trait mapping

`crates/vcam::VirtualCamera` → `sidecam_vcam_open/close/push_frame` di DLL ini.
