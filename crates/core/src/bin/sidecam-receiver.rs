//! sidecam-receiver — binary M1: signaling server + WebRTC + preview minifb
//!
//! Konsep Rust:
//! - clap derive: CLI parsing via macro derive
//! - tokio::main: runtime async, mirip event loop Node tapi multi-thread
//! - anyhow::Result: error handling ergonomis dengan context

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use clap::Parser;
use tracing::{info, warn};

use sidecam_core::{PairingStore, AppState, router};

#[derive(Parser, Debug)]
#[command(name="sidecam-receiver", about="SideCam M1 receiver — preview WebRTC di jendela")]
struct Args {
    #[arg(long, default_value="0.0.0.0")]
    host: String,
    #[arg(long, default_value_t=8080)]
    port: u16,
    /// Token manual (jika tidak diisi, generate random)
    #[arg(long)]
    token: Option<String>,
    /// Tampilkan QR JSON di terminal
    #[arg(long, default_value_t=false)]
    qr: bool,
    /// vcam device (Linux) atau placeholder
    #[arg(long)]
    vcam_device: Option<String>,
    /// Tanpa preview window (headless, untuk CI)
    #[arg(long, default_value_t=false)]
    headless: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())
    ).init();

    let args = Args::parse();
    let pairing = PairingStore::new();

    let token = if let Some(t) = args.token.clone() {
        pairing.insert_token(t.clone());
        t
    } else {
        pairing.generate_token()
    };

    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let host_ip = local_ip().unwrap_or_else(|| args.host.clone());

    // Tampilkan info pairing
    let hostname_str = std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "Laptop".into());
    let pairing_json = serde_json::json!({
        "v": 1,
        "host": host_ip,
        "port": args.port,
        "token": token,
        "device_name": hostname_str,
        "device_id": uuid::Uuid::new_v4().to_string(),
    });
    println!("\n=== SideCam M1 Receiver ===");
    println!("Listen: {}", addr);
    println!("Pairing JSON (untuk HP/browser):");
    println!("{}", serde_json::to_string_pretty(&pairing_json)?);
    println!("\nWebSocket: ws://{}:{}/signal?token={}", host_ip, args.port, pairing_json["token"].as_str().unwrap());
    println!("Health:    http://{}:{}/health", host_ip, args.port);
    if args.qr {
        // ponytail: QR render di terminal via qrcode crate di M3; M1 cetak JSON saja
        println!("\n[qr] --qr butuh fitur M3 (qrcode crate). JSON di atas bisa di-scan manual.");
    }
    println!("\nBuka test client: http://{}:{}/  (serve file apps/desktop/test-client.html)\n", host_ip, args.port);

    // vcam setup
    let mut vcam: Box<dyn sidecam_vcam::VirtualCamera> = if let Some(dev) = args.vcam_device.as_deref() {
        #[cfg(target_os="linux")]
        {
            if dev == "placeholder" {
                Box::new(sidecam_vcam::PlaceholderCamera::new("SideCam"))
            } else {
                Box::new(sidecam_vcam::linux::V4l2LoopbackCamera::new(dev))
            }
        }
        #[cfg(not(target_os="linux"))]
        {
            let _ = dev;
            sidecam_vcam::create_default()
        }
    } else {
        sidecam_vcam::create_default()
    };
    info!("[vcam] device: {}", vcam.name());
    if let Err(e) = vcam.open() {
        warn!("[vcam] open gagal: {} (lanjut tanpa vcam)", e);
    }

    // Serve static test-client.html di / (path absolut dari workspace root)
    let state = AppState { pairing, host: host_ip.clone() };
    let html_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/test-client.html");
    let html_content = std::fs::read_to_string(&html_path).unwrap_or_else(|_| "<h1>SideCam M1 — test-client.html tidak ditemukan</h1>".into());
    let html_for_route = html_content.clone();
    let html_for_log = html_content.clone();

    // html di / (static), api di /api, health di /health, ws di /signal
    let app = router(state)
        .route("/", axum::routing::get({
            let html = html_for_route.clone();
            move || { let html = html.clone(); async move { axum::response::Html(html) } }
        }))
        .route("/api/info", axum::routing::get(|| async { "SideCam M1" }));
    let _ = html_for_log;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("[server] listen {}", addr);

    // ponytail: preview minifb buggy di Wayland; M1 headless by default.
    // Pakai --preview untuk paksa buka jendela (butuh X11).
    let want_preview = std::env::var("SIDECAM_PREVIEW").is_ok() || std::env::args().any(|a| a == "--preview");
    if args.headless || !want_preview {
        if !args.headless && !want_preview {
            println!("(preview mati default — tambah --preview atau SIDECAM_PREVIEW=1 untuk jendela)");
        }
        axum::serve(listener, app).await?;
        return Ok(());
    }

    // Preview mode eksplisit (--preview)
    let server_handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!("[server] error: {}", e);
        }
    });

    let frames: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let start = std::time::Instant::now();
    let frames2 = frames.clone();
    let preview = std::thread::spawn(move || {
        run_preview(frames2, start);
    });

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let elapsed = start.elapsed().as_secs_f64();
                let f = *frames.lock().unwrap();
                let fps = if elapsed > 0.0 { f as f64 / elapsed } else { 0.0 };
                info!("[stats] frames={} elapsed={:.1}s fps={:.1} (M1: WebRTC stats real ada di browser client)", f, elapsed, fps);
            }
            _ = async { server_handle.is_finished() } => { break; }
        }
        if preview.is_finished() { break; }
    }

    Ok(())
}

fn local_ip() -> Option<String> {
    // ponytail: pilih IP non-loopback pertama, upgrade ke getifaddrs di M3
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    s.local_addr().ok().map(|a| a.ip().to_string())
    .and_then(|ip| { if ip.starts_with("127.") { None } else { Some(ip) } })
    .or_else(|| {
        // fallback
        local_ip_fallback()
    })
}

fn local_ip_fallback() -> Option<String> {
    // baca dari hostname -I style via /proc — sederhana
    None
}

fn run_preview(frames: Arc<Mutex<u64>>, start: std::time::Instant) {
    use minifb::{Window, WindowOptions};
    let w = 640; let h = 360;
    let mut buf = vec![0u32; w * h];
    let mut window = match Window::new("SideCam M1 — Preview (placeholder, WebRTC di browser)", w, h, WindowOptions::default()) {
        Ok(w) => w,
        Err(e) => { warn!("[preview] Window::new gagal: {}", e); return; }
    };
    window.set_target_fps(30);

    let mut tick: u64 = 0;
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        tick += 1;
        *frames.lock().unwrap() = tick;
        // gradient animasi + placeholder teks via warna
        for y in 0..h {
            for x in 0..w {
                let r = ((x * 255 / w) as u32 + tick as u32 * 2) % 255;
                let g = ((y * 255 / h) as u32) as u32;
                let b = 128 + ((tick % 60) as u32 * 2) % 64;
                buf[y * w + x] = (r << 16) | (g << 8) | b;
            }
        }
        // bar indikator tengah
        let bar_y = h / 2 - 4;
        for dy in 0..8 { for x in 0..w { buf[(bar_y+dy)*w + x] = 0xFFFFFF; } }

        if let Err(e) = window.update_with_buffer(&buf, w, h) {
            warn!("[preview] update error: {}", e);
            break;
        }
        // elapsed untuk stats
        let _ = start;
    }
    info!("[preview] window closed");
}
