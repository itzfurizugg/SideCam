# PRD: SideCam

**Versi:** 1.0
**Tujuan dokumen:** acuan lengkap untuk membangun project dari nol. Dibaca oleh developer dan Claude Code.

---

## 1. Ringkasan

SideCam memungkinkan HP dipakai sebagai webcam laptop Windows lewat Wi-Fi. Pengguna membuka app di HP, scan QR di laptop, lalu HP langsung muncul sebagai kamera di Zoom, Google Meet, Teams, dan aplikasi video call lain. Tidak perlu USB debugging dan tidak perlu membuka opsi developer.

**Inspirasi UX:** Continuity Camera (Apple) dan interconnectivity Xiaomi.

**Masalah yang diselesaikan:** webcam bawaan banyak laptop (contoh: Axioo) kualitasnya jelek, sedangkan kamera HP jauh lebih bagus. Solusi yang ada (DroidCam, Iriun, Camo) masih terlalu ribet untuk orang awam.

## 2. Platform Target

| Sisi | Platform |
|---|---|
| HP | Android (prioritas), iOS (fase berikutnya) |
| Desktop | Windows 10 dan Windows 11 |

Linux hanya dipakai untuk kebutuhan pengembangan, bukan target rilis.

## 3. Tujuan

- Setup pertama kali selesai kurang dari 2 menit tanpa membuka pengaturan developer.
- Koneksi berikutnya otomatis: buka app di HP, laptop langsung mendeteksi.
- Muncul sebagai webcam biasa di aplikasi video call populer.
- Latensi end-to-end di bawah 200 ms pada Wi-Fi 5 GHz yang sehat.
- Video minimal 720p 30fps, opsi 1080p 30fps.

## 4. Bukan Tujuan (Versi 1)

- Server cloud atau relay internet (hanya jaringan lokal).
- Mode USB.
- Efek video (filter, background blur).
- Audio/mikrofon.
- Target rilis macOS dan Linux.

## 5. Pengguna dan Prinsip UX

**Pengguna utama:** orang awam (pelajar, pekerja, guru) yang butuh kamera lebih bagus untuk video call dan tidak paham hal teknis.

**Prinsip UX:**

- Maksimal 3 langkah untuk tersambung.
- Tidak ada istilah teknis di UI (tanpa kata IP, port, codec).
- Pesan error berbahasa Indonesia yang sederhana, disertai solusi yang jelas.

## 6. User Stories

1. Sebagai pengguna, saya memasang app di laptop dan HP, scan QR sekali, lalu HP saya menjadi webcam.
2. Sebagai pengguna, saya cukup membuka app di HP dan laptop otomatis tersambung tanpa scan ulang.
3. Sebagai pengguna, saya memilih kamera SideCam di Zoom/Meet seperti memilih webcam biasa.
4. Sebagai pengguna, saya bisa ganti kamera depan/belakang dan memilih resolusi dari HP.
5. Sebagai pengguna, saya bisa mencabut izin HP yang pernah terhubung dari laptop.

## 7. Arsitektur

```
HP (kamera, encode H.264)
   -> WebRTC (jaringan lokal)
   -> Receiver desktop Rust (decode)
   -> Virtual camera Windows
   -> Zoom / Meet / Teams
```

### 7.1 Komponen

| Komponen | Stack | Tanggung jawab |
|---|---|---|
| `mobile` | Flutter/Dart, `flutter_webrtc`, `camera`, `nsd` (mDNS), `mobile_scanner` | Scan QR, auto-discovery, kirim stream kamera, pengaturan kamera, foreground service |
| `desktop-core` | Rust, `tokio`, `axum` atau `tokio-tungstenite`, `mdns-sd`, `webrtc-rs` (fallback: `libdatachannel`), `openh264` atau `ffmpeg-next` | Signaling server, iklan mDNS, terima stream WebRTC, decode H.264, kirim frame ke virtual camera, kelola pairing dan perangkat tepercaya |
| `desktop-ui` | Tauri + React + TypeScript | QR pairing, status koneksi, preview kecil, daftar perangkat |
| `virtual-camera` | C++ DirectShow filter (basis Softcam atau UnityCapture, cek lisensi), C ABI untuk FFI dari Rust | Mendaftarkan kamera SideCam di Windows, menerima frame lewat shared memory. Fase lanjutan: Media Foundation Virtual Camera (`MFCreateVirtualCamera`) untuk Windows 11 |
| `installer` | Inno Setup atau WiX | Pasang app, daftarkan DLL virtual camera (`regsvr32`, hak admin sekali saja), tambah aturan firewall, bersihkan saat uninstall |

> **Penting:** WebRTC dan decode video **tidak boleh** berjalan di dalam webview Tauri. Mengirim frame dari webview ke Rust lambat dan membuat video patah-patah. Webview hanya untuk UI.

### 7.2 Struktur Repo (monorepo)

```
.
├── apps/
│   ├── mobile/              # Flutter app
│   └── desktop/             # Tauri app (src-tauri = Rust, src = React)
├── crates/
│   ├── core/                # Rust: signaling, WebRTC, decode, pairing
│   └── vcam/                # Rust wrapper + FFI, trait VirtualCamera
├── native/
│   └── vcam-windows/        # C++ DirectShow filter
├── installer/               # Skrip installer
└── docs/                    # Protokol, keputusan arsitektur, hasil pengukuran
```

Trait `VirtualCamera` di `crates/vcam` punya dua implementasi: Windows (DirectShow) untuk produk, dan Linux (`v4l2loopback`) untuk kebutuhan pengembangan.

## 8. Pairing dan Protokol

### 8.1 Discovery

- Metode: mDNS.
- Service type: `_SideCam._tcp.local.`
- TXT records: `v=1`, `name=<nama laptop>`, `id=<device uuid>`.

### 8.2 Pairing Pertama Kali

1. Desktop menampilkan QR berisi JSON: `{v, host, port, token, device_name, device_id}`.
2. Token sekali pakai dan kedaluwarsa dalam 2 menit.
3. HP scan QR dan membuka WebSocket ke `ws://host:port/signal?token=...`.
4. Desktop menampilkan dialog konfirmasi: "Izinkan <nama HP>?".
5. Setelah disetujui, desktop menerbitkan long-lived secret yang disimpan di HP dan di desktop (daftar perangkat tepercaya).

### 8.3 Reconnect

HP menemukan desktop via mDNS, lalu memakai long-lived secret untuk autentikasi tanpa scan ulang. Desktop menolak perangkat yang sudah dicabut.

### 8.4 Pesan Signaling (WebSocket)

| Pesan | Isi |
|---|---|
| `hello` | `{device_id, name, app_version}` |
| `auth` | `{secret}` atau `{token}` |
| `offer` | `{sdp}` |
| `answer` | `{sdp}` |
| `ice` | `{candidate}` |
| `config` | `{resolution, fps, facing}` |
| `bye` | `{reason}` |

### 8.5 Media

- Transport: WebRTC (DTLS-SRTP bawaan).
- Codec: H.264 (hardware encoder HP bila tersedia).
- Default: 1280x720 @ 30fps. Opsi: 1920x1080 @ 30fps.
- ICE: host candidates saja (tanpa STUN/TURN di versi 1).

## 9. Functional Requirements

| ID | Prioritas | Requirement |
|---|---|---|
| FR-1 | P0 | Desktop menjalankan signaling server dan menampilkan QR pairing. |
| FR-2 | P0 | HP dapat scan QR dan memulai stream video ke desktop. |
| FR-3 | P0 | Desktop menampilkan preview stream di UI. |
| FR-4 | P0 | Frame yang diterima dikirim ke virtual camera Windows bernama SideCam. |
| FR-5 | P0 | SideCam terdeteksi dan menampilkan video di Zoom, Google Meet (Chrome), dan Teams. |
| FR-6 | P1 | Auto-discovery mDNS dan reconnect otomatis tanpa scan ulang. |
| FR-7 | P1 | Dialog konfirmasi pairing dan daftar perangkat tepercaya yang bisa dicabut. |
| FR-8 | P1 | Ganti kamera depan/belakang, pilih resolusi, dan mirror dari HP. |
| FR-9 | P1 | Saat HP terputus, virtual camera menampilkan frame placeholder (bukan hitam atau beku) dan mencoba menyambung ulang otomatis. |
| FR-10 | P1 | Installer sekali klik yang mendaftarkan DLL virtual camera dan aturan firewall. |
| FR-11 | P2 | Jalur Media Foundation Virtual Camera untuk Windows 11. |
| FR-12 | P2 | Versi iOS pada app mobile. |

## 10. Non-Functional Requirements

**Performa**

- Latensi end-to-end di bawah 200 ms pada Wi-Fi 5 GHz.
- CPU desktop di bawah 15% pada 720p30 di laptop kelas menengah.
- Tidak ada memory leak pada sesi 2 jam.

**Keamanan**

- Semua traffic hanya di jaringan lokal.
- Token pairing sekali pakai dan berumur pendek.
- Perangkat baru wajib disetujui manual di desktop.
- Tidak ada telemetri atau pengiriman data ke luar tanpa persetujuan pengguna.

**Keandalan**

- Reconnect otomatis saat Wi-Fi putus sebentar.
- Tidak crash bila virtual camera dibuka beberapa aplikasi sekaligus.

**UX**

- Seluruh teks UI dalam Bahasa Indonesia, dengan struktur siap untuk bahasa lain.
- Pesan error menjelaskan penyebab dan langkah perbaikan, misalnya: "HP dan laptop harus di Wi-Fi yang sama".

## 11. Milestone

### M1: Prototype jaringan tanpa UI

HP mengirim video lewat WebRTC ke receiver Rust yang menampilkannya di jendela sederhana. Pairing masih manual (IP dan token di CLI).

**Acceptance:**
- Video 720p30 stabil selama 10 menit.
- Latensi terukur dan dicatat di `docs/`.

### M2: Virtual camera

Frame dari receiver dikirim ke virtual camera. Definisikan trait `VirtualCamera` dengan implementasi Windows (DirectShow) dan Linux (`v4l2loopback`).

**Acceptance:**
- SideCam terdeteksi di Zoom, Meet, dan Teams pada Windows.
- Tidak ada frame beku saat HP diputus.

### M3: Pairing dan UX

QR pairing, mDNS, dialog konfirmasi, daftar perangkat tepercaya, UI Tauri + React.

**Acceptance:**
- Pengguna awam menyelesaikan setup pertama kali tanpa bantuan.
- Reconnect otomatis berhasil tanpa scan ulang.

### M4: Installer dan polesan

Installer, aturan firewall, foreground service Android, pengaturan kamera/resolusi, penanganan error.

**Acceptance:**
- Instalasi bersih pada Windows 10 dan 11.
- Uninstall membersihkan registrasi DLL dan aturan firewall.

### M5: Opsional

Media Foundation Virtual Camera untuk Windows 11, versi iOS, mode USB.

## 12. Risiko dan Mitigasi

| Risiko | Mitigasi |
|---|---|
| `webrtc-rs` belum semulus WebRTC di browser | Siapkan abstraksi transport. Fallback ke `libdatachannel` (C++) atau kirim H.264 lewat UDP/TCP untuk MVP. |
| Virtual camera DirectShow tidak terdeteksi di sebagian aplikasi (terutama berbasis UWP atau Media Foundation) | Uji sejak M2 di Zoom, Meet, Teams, WhatsApp Desktop, OBS. Siapkan jalur MF Virtual Camera. |
| Firewall Windows memblokir koneksi masuk | Installer menambahkan aturan untuk jaringan privat. UI memberi petunjuk jika koneksi gagal. |
| Android mematikan proses kamera saat layar dikunci atau mode hemat baterai | Foreground service dengan notifikasi, wake lock, dan panduan pengecualian optimasi baterai di dalam app. |
| Lisensi library virtual camera tidak cocok untuk distribusi | Periksa lisensi Softcam/UnityCapture sebelum dipakai. Tulis ulang bila perlu. |

## 13. Lingkungan Pengembangan

- Pengembangan utama di Linux (Ubuntu). Target rilis Windows.
- Bagian spesifik Windows (DirectShow, installer, firewall) hanya bisa diuji di mesin Windows, jadi isolasi di balik abstraksi supaya sebagian besar logika bisa dikembangkan dan diuji di Linux.
- Uji akhir M2 dan seterusnya dilakukan di laptop Windows target.

## 14. Pertanyaan Terbuka

- Nama produk final (SideCam hanya nama kerja).
- Versi Windows pada laptop target (10 atau 11), untuk menentukan jalur virtual camera yang diuji lebih dulu.
- Apakah iOS perlu masuk rilis pertama atau cukup Android.
- Apakah audio/mikrofon HP perlu masuk di versi 2.