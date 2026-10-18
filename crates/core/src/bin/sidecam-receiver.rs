//! sidecam-receiver — M1..M5

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use clap::Parser;
use tracing::{info, warn};

use sidecam_core::{AppState, PairingStore, router};

#[derive(Parser, Debug)]
#[command(name="sidecam-receiver", about="SideCam receiver — https + QR + mDNS + vcam")]
struct Args {
    #[arg(long, default_value="0.0.0.0")] host: String,
    #[arg(long, default_value_t=8080)] port: u16,
    #[arg(long)] token: Option<String>,
    #[arg(long, default_value_t=false)] qr: bool,
    #[arg(long)] vcam_device: Option<String>,
    #[arg(long, default_value_t=false)] headless: bool,
    #[arg(long, default_value_t=false)] no_tls: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())
    ).init();

    let args = Args::parse();
    let pairing = PairingStore::new();
    let token = if let Some(t)=args.token.clone(){ pairing.insert_token(t.clone()); t } else { pairing.generate_token() };
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let host_ip = local_ip().unwrap_or_else(|| args.host.clone());
    let hostname_str = std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "Laptop".into());
    let device_id = uuid::Uuid::new_v4().to_string();
    let pairing_json = serde_json::json!({
        "v": 1, "host": host_ip, "port": args.port, "token": token,
        "device_name": hostname_str, "device_id": device_id,
    });
    let pairing_str = serde_json::to_string(&pairing_json)?;
    println!("\n=== SideCam Receiver (M1..M5) ===");
    println!("Listen: {}", addr);
    println!("Pairing JSON:"); println!("{}", serde_json::to_string_pretty(&pairing_json)?);
    let ws_scheme=if args.no_tls{"ws"}else{"wss"}; let http_scheme=if args.no_tls{"http"}else{"https"};
    println!("\nWebSocket: {ws_scheme}://{}:{}/signal?token={}", host_ip, args.port, pairing_json["token"].as_str().unwrap());
    println!("Health:    {http_scheme}://{}:{}/health", host_ip, args.port);
    // QR (M3): generate SVG
    let qr_svg = sidecam_core::mdns::pairing_qr_svg(&pairing_str);
    if !qr_svg.is_empty() {
        println!("\n[QR] SVG len={} — scan di HP atau buka https://{}:{}/ di HP", qr_svg.len(), host_ip, args.port);
        // also save to file for easy open
        let _ = std::fs::write("/tmp/sidecam-qr.svg", &qr_svg);
        println!("[QR] saved /tmp/sidecam-qr.svg");
    }
    if args.no_tls { println!("\nBuka: http://{}:{}/ (http — Chrome HP butuh flag)", host_ip, args.port); }
    else { println!("\nBuka: https://{}:{}/ (self-signed → Advanced → Proceed)", host_ip, args.port); }
    println!();

    // mDNS
    sidecam_core::mdns::advertise(args.port, &host_ip, &hostname_str, &device_id);

    // vcam shared (M2)
    let mut vcam_init: Box<dyn sidecam_vcam::VirtualCamera> = if let Some(dev)=args.vcam_device.as_deref() {
        #[cfg(target_os="linux")]
        { if dev=="placeholder"{Box::new(sidecam_vcam::PlaceholderCamera::new("SideCam"))} else {Box::new(sidecam_vcam::linux::V4l2LoopbackCamera::new(dev))} }
        #[cfg(not(target_os="linux"))] { let _=dev; sidecam_vcam::create_default() }
    } else { sidecam_vcam::create_default() };
    let _ = vcam_init.open();
    let _ = vcam_init.push_frame(&sidecam_vcam::Frame::placeholder(1280,720));
    let vcam_shared = Arc::new(Mutex::new(vcam_init));
    info!("[vcam] device: {}", vcam_shared.lock().unwrap().name().to_string());

    // pairing state with secret persist
    let pairing2 = pairing.clone();
    let html_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/test-client.html");
    let mut html_content = std::fs::read_to_string(&html_path).unwrap_or_else(|_| "<h1>SideCam — test-client.html tidak ditemukan</h1>".into());
    // inject QR SVG + ws url into HTML placeholder
    if !qr_svg.is_empty() {
        let inject = format!(r#"<div id="qr-inject" style="display:none">{}</div><script>window.SIDECAM_QR_SVG=document.getElementById('qr-inject').innerHTML;window.SIDECAM_PAIRING={};</script>"#, qr_svg, pairing_str);
        html_content = html_content.replace("</body>", &format!("{}</body>", inject));
    }
    // also serve QR directly
    let qr_clone = qr_svg.clone();
    let state = AppState { pairing, host: host_ip.clone(), vcam: vcam_shared.clone() };
    let html_for_route = html_content.clone();
    let app = router(state.clone())
        .route("/", axum::routing::get({
            let html=html_for_route.clone(); move||{let html=html.clone(); async move{axum::response::Html(html)}}
        }))
        .route("/qr.svg", axum::routing::get(move||{ let svg=qr_clone.clone(); async move{ ([("content-type","image/svg+xml")], svg)} }))
        .route("/api/info", axum::routing::get({
            let pairing=pairing2.clone(); move||{ let pairing=pairing.clone(); async move{
                axum::Json(serde_json::json!({
                    "name":"SideCam","version":"0.5.0-m5",
                    "devices": pairing.list_devices().len(),
                    "ws": format!("wss://{}:{}/signal", "IP", 8080)
                }))
            }}
        }))
        .route("/api/devices", axum::routing::get({
            let pairing=pairing2.clone(); move||{ let pairing=pairing.clone(); async move{
                let devs: Vec<_>=pairing.list_devices().into_iter().map(|d| serde_json::json!({"device_id":d.device_id,"name":d.name})).collect();
                axum::Json(devs)
            }}
        }))
        .route("/api/devices/:id", axum::routing::delete({
            let pairing=pairing2.clone(); move|axum::extract::Path(id): axum::extract::Path<String>|{
                let pairing=pairing.clone(); let id=id.clone();
                async move{
                    if pairing.revoke(&id) { (axum::http::StatusCode::OK, "revoked".to_string()) }
                    else { (axum::http::StatusCode::NOT_FOUND, "not found".to_string()) }
                }
            }
        }));

    let want_preview = std::env::var("SIDECAM_PREVIEW").is_ok() || std::env::args().any(|a| a=="--preview");
    let use_tls = !args.no_tls;
    if args.headless || !want_preview {
        if !args.headless && !want_preview { println!("(preview mati default — --preview untuk jendela)"); }
        if use_tls { serve_tls(app, addr, &host_ip).await?; } else {
            let listener=tokio::net::TcpListener::bind(addr).await?;
            info!("[server] listen http {}", addr); axum::serve(listener, app).await?;
        }
        return Ok(());
    }
    if use_tls {
        let tls_config=make_tls_config(&host_ip)?;
        let server=axum_server::bind_rustls(addr, tls_config);
        info!("[server] listen https {} (preview on)", addr);
        let h=tokio::spawn(async move{ if let Err(e)=server.serve(app.into_make_service()).await{ warn!("[server] error: {}",e);} });
        run_preview_loop(h).await;
    } else {
        let listener=tokio::net::TcpListener::bind(addr).await?;
        info!("[server] listen http {} (preview on)", addr);
        let h=tokio::spawn(async move{ if let Err(e)=axum::serve(listener, app).await{ warn!("[server] error: {}",e);} });
        run_preview_loop(h).await;
    }
    Ok(())
}

fn make_tls_config(host_ip:&str)->anyhow::Result<axum_server::tls_rustls::RustlsConfig>{
    let cert=rcgen::generate_simple_self_signed(vec!["localhost".into(),"127.0.0.1".into(),host_ip.into()])?;
    let cert_der=cert.cert.der().to_vec(); let key_der=cert.key_pair.serialize_der();
    let cert_chain=vec![rustls::pki_types::CertificateDer::from(cert_der)];
    let key=rustls::pki_types::PrivateKeyDer::try_from(key_der).map_err(|e| anyhow::anyhow!("key der: {:?}",e))?;
    let mut cfg=rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(cert_chain,key)?;
    cfg.alpn_protocols=vec![b"h2".to_vec(),b"http/1.1".to_vec()];
    Ok(axum_server::tls_rustls::RustlsConfig::from_config(std::sync::Arc::new(cfg)))
}
async fn serve_tls(app:axum::Router, addr:SocketAddr, host_ip:&str)->anyhow::Result<()>{
    let cfg=make_tls_config(host_ip)?; info!("[server] listen https {}",addr);
    axum_server::bind_rustls(addr,cfg).serve(app.into_make_service()).await?; Ok(())
}
async fn run_preview_loop(server_handle:tokio::task::JoinHandle<()>){
    let frames:Arc<Mutex<u64>>=Arc::new(Mutex::new(0)); let start=std::time::Instant::now();
    let frames2=frames.clone(); let preview=std::thread::spawn(move||{run_preview(frames2,start);});
    let mut interval=tokio::time::interval(std::time::Duration::from_secs(5));
    loop{ tokio::select!{ _=interval.tick()=>{let e=start.elapsed().as_secs_f64();let f=*frames.lock().unwrap();let fps=if e>0.0{f as f64/e}else{0.0};info!("[stats] frames={} elapsed={:.1}s fps={:.1}",f,e,fps);} _=async{server_handle.is_finished()}=>{break;}} if preview.is_finished(){break;} }
}
fn local_ip()->Option<String>{
    let s=std::net::UdpSocket::bind("0.0.0.0:0").ok()?; s.connect("8.8.8.8:80").ok()?;
    s.local_addr().ok().map(|a| a.ip().to_string()).and_then(|ip| if ip.starts_with("127."){None}else{Some(ip)}).or_else(local_ip_fallback)
}
fn local_ip_fallback()->Option<String>{None}
fn run_preview(frames:Arc<Mutex<u64>>,start:std::time::Instant){
    use minifb::{Window,WindowOptions};
    let w=640;let h=360;let mut buf=vec![0u32;w*h];
    let mut window=match Window::new("SideCam — Preview",w,h,WindowOptions::default()){Ok(w)=>w,Err(e)=>{warn!("[preview] Window::new gagal: {}",e);return;}};
    window.set_target_fps(30);let mut tick:u64=0;
    while window.is_open()&&!window.is_key_down(minifb::Key::Escape){
        tick+=1;*frames.lock().unwrap()=tick;
        for y in 0..h{for x in 0..w{let r=((x*255/w)as u32+tick as u32*2)%255;let g=(y*255/h)as u32;let b=128+((tick%60)as u32*2)%64;buf[y*w+x]=(r<<16)|(g<<8)|b;}}
        let bar_y=h/2-4;for dy in 0..8{for x in 0..w{buf[(bar_y+dy)*w+x]=0xFFFFFF;}}
        if let Err(e)=window.update_with_buffer(&buf,w,h){warn!("[preview] update error: {}",e);break;}
        let _=start;
    } info!("[preview] window closed");
}
