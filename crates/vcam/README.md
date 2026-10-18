# crates/vcam

Trait `VirtualCamera` + implementasi per OS. Kode Windows isolasi di sini, tidak bocor ke `crates/core`.

## Trait

```rust
trait VirtualCamera: Send {
  fn open(&mut self) -> Result<()>;
  fn push_frame(&mut self, frame: &Frame) -> Result<()>;
  fn close(&mut self) -> Result<()>;
  fn is_open(&self) -> bool;
  fn name(&self) -> &str;
}
```

## Implementasi

- `PlaceholderCamera` — selalu ada, untuk M1 tanpa device
- `linux::V4l2LoopbackCamera` — `/dev/video10` (Linux dev)
- `windows::DirectShowCamera` — stub M1, FFI ke `native/vcam-windows` di M2

## Build & Test

```bash
cargo test -p sidecam-vcam
```

Bikin loopback (Linux):
```bash
sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=SideCam exclusive_caps=1
v4l2-ctl --list-devices
```
