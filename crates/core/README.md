# crates/core

Signaling + WebRTC + pairing.

## Build & Test

```bash
cargo test -p sidecam-core
cargo check
cargo build --bin sidecam-receiver
```

## Bin

```
cargo run --bin sidecam-receiver -- --headless
cargo run --bin sidecam-receiver -- --port 8080 --token xxx
```

Lihat `--help` untuk opsi.

## Protokol

WS `ws://host:port/signal?token=...` pesan JSON `hello/auth/offer/answer/ice/config/bye` (PRD 8.4).
