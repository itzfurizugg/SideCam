//! sidecam-receiver — split listeners: public 0.0.0.0:8080 + admin 127.0.0.1:8081
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use clap::Parser;
use tracing::{info, warn};
use sidecam_core::{AppState, PairingStore, router, admin_router};

#[derive(Parser, Debug)]
#[command(name="sidecam-receiver", about="SideCam receiver — public 8080 + admin 127.0.0.1:8081")]
struct Args {
    #[arg(long, default_value="0.0.0.0")] host: String,
    #[arg(long, default_value_t=8080)] port: u16,
    #[arg(long, default_value_t=8081)] admin_port: u16,
    #[arg(long)] token: Option<String>,
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
    let host_ip = local_ip().unwrap_or_else(|| args.host.clone());
    let hostname_str = std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "Laptop".into());
    let device_id = uuid::Uuid::new_v4().to_string();
    let pairing_json = serde_json::json!({
        "v": 1, "host": host_ip, "port": args.port, "token": token,
        "device_name": hostname_str, "device_id": device_id,
    });
    let pairing_str = serde_json::to_string(&pairing_json)?;
    println!("\n=== SideCam Receiver (split listeners) ===");
    let scheme=if args.no_tls{"http"}else{"https"};
    let ws_scheme=if args.no_tls{"ws"}else{"wss"};
    println!("Public: {scheme}://{}:{}/  +  {ws_scheme}://{}:{}/signal  (0.0.0.0:{})", host_ip, args.port, host_ip, args.port, args.port);
    println!("Admin : {scheme}://127.0.0.1:{}/  (only localhost)  /qr.svg /api/devices /api/approve", args.admin_port);
    println!("Pairing JSON:\n{}", serde_json::to_string_pretty(&pairing_json)?);
    let qr_svg = sidecam_core::mdns::pairing_qr_svg(&pairing_str);
    if !qr_svg.is_empty() { let _ = std::fs::write("/tmp/sidecam-qr.svg", &qr_svg); println!("[QR] saved /tmp/sidecam-qr.svg len={}", qr_svg.len()); }
    println!();

    sidecam_core::mdns::advertise(args.port, &host_ip, &hostname_str, &device_id);

    let mut vcam_init: Box<dyn sidecam_vcam::VirtualCamera> = if let Some(dev)=args.vcam_device.as_deref() {
        #[cfg(target_os="linux")]
        { if dev=="placeholder"{Box::new(sidecam_vcam::PlaceholderCamera::new("SideCam"))} else {Box::new(sidecam_vcam::linux::V4l2LoopbackCamera::new(dev))} }
        #[cfg(not(target_os="linux"))] { let _=dev; sidecam_vcam::create_default() }
    } else { sidecam_vcam::create_default() };
    let _ = vcam_init.open();
    let _ = vcam_init.push_frame(&sidecam_vcam::Frame::placeholder(1280,720));
    let vcam_shared = Arc::new(Mutex::new(vcam_init));
    info!("[vcam] device: {}", vcam_shared.lock().unwrap().name().to_string());

    let pairing_pub = pairing.clone();
    let pairing_admin = pairing.clone();
    let html_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/test-client.html");
    let html_content = std::fs::read_to_string(&html_path).unwrap_or_else(|_| "<h1>SideCam — test-client.html tidak ditemukan</h1>".into());
    let html_for_both = if !qr_svg.is_empty() {
        let inject = format!(r#"<div id="qr-inject" style="display:none">{}</div><script>window.SIDECAM_QR_SVG=document.getElementById('qr-inject').innerHTML;window.SIDECAM_PAIRING={};</script>"#, qr_svg, pairing_str);
        html_content.replace("</body>", &format!("{}</body>", inject))
    } else { html_content.clone() };

    let pairing_pub2 = pairing_pub.clone();
    let state_pub = AppState { pairing: pairing_pub.clone(), host: host_ip.clone(), vcam: vcam_shared.clone() };
    let html_pub = html_for_both.clone();
    let public_app = router(state_pub.clone())
        .route("/", axum::routing::get({
            let html=html_pub.clone(); move||{ let html=html.clone(); async move{ axum::response::Html(html) } }
        }))
        .route("/api/info", axum::routing::get({
            let p=pairing_pub2.clone(); move||{ let p=p.clone(); async move{
                axum::Json(serde_json::json!({"name":"SideCam","version":"0.6.0-split","devices": p.list_devices().len()}))
            }}
        }));

    let html_admin = html_for_both.clone();
    let qr_clone = qr_svg.clone();
    let pairing_for_admin = pairing_admin.clone();
    // admin_router already has /health
    let admin_app = admin_router(pairing_admin.clone(), vcam_shared.clone())
        .route("/", axum::routing::get({
            let html=html_admin.clone(); move||{ let html=html.clone(); async move{ axum::response::Html(html) } }
        }))
        .route("/qr.svg", axum::routing::get(move||{ let svg=qr_clone.clone(); async move{ ([("content-type","image/svg+xml")], svg) } }))
        .route("/api/devices", axum::routing::get({
            let p=pairing_for_admin.clone(); move||{ let p=p.clone(); async move{
                let devs: Vec<_>=p.list_devices().into_iter().map(|d| serde_json::json!({"device_id":d.device_id,"name":d.name})).collect();
                axum::Json(devs)
            }}
        }))
        .route("/api/devices/:id", axum::routing::delete({
            let p=pairing_for_admin.clone(); move|axum::extract::Path(id): axum::extract::Path<String>, headers: axum::http::HeaderMap|{
                let p=p.clone();
                async move{
                    if let Err(r)=check_admin_headers(&headers){ return r; }
                    if p.revoke(&id){ (axum::http::StatusCode::OK, "revoked".to_string()) } else { (axum::http::StatusCode::NOT_FOUND, "not found".to_string()) }
                }
            }
        }))
        .route("/api/approve", axum::routing::post({
            let p=pairing_admin.clone(); move|headers: axum::http::HeaderMap, axum::Json(body): axum::Json<serde_json::Value>|{
                let p=p.clone();
                async move{
                    if let Err(r)=check_admin_headers(&headers){ return r; }
                    // body: {device_id, approve: bool}
                    let _ = p; let _ = body;
                    (axum::http::StatusCode::OK, "ok".to_string())
                }
            }
        }));

    let use_tls = !args.no_tls;
    let pub_addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let admin_addr: SocketAddr = format!("127.0.0.1:{}", args.admin_port).parse()?;

    // spawn both listeners
    if use_tls {
        let cfg_pub = make_tls_config(&host_ip)?;
        let cfg_admin = make_tls_config("127.0.0.1")?;
        let pub_srv = axum_server::bind_rustls(pub_addr, cfg_pub);
        let adm_srv = axum_server::bind_rustls(admin_addr, cfg_admin);
        info!("[listen] public https {}  admin https {}", pub_addr, admin_addr);
        let h1 = tokio::spawn(async move{ if let Err(e)=pub_srv.serve(public_app.into_make_service()).await{ warn!("[public] error: {}",e);} });
        let h2 = tokio::spawn(async move{ if let Err(e)=adm_srv.serve(admin_app.into_make_service()).await{ warn!("[admin] error: {}",e);} });
        let want_preview = std::env::var("SIDECAM_PREVIEW").is_ok() || std::env::args().any(|a| a=="--preview");
        if args.headless || !want_preview {
            if !args.headless && !want_preview { println!("(preview mati default — --preview untuk jendela)"); }
            tokio::select!{ _=h1=>{}, _=h2=>{} }
        } else {
            let srv_h = tokio::spawn(async move{ tokio::select!{ _=h1=>{}, _=h2=>{} } });
            run_preview_loop(srv_h).await;
        }
    } else {
        let l1 = tokio::net::TcpListener::bind(pub_addr).await?;
        let l2 = tokio::net::TcpListener::bind(admin_addr).await?;
        info!("[listen] public http {}  admin http {}", pub_addr, admin_addr);
        let h1 = tokio::spawn(async move{ if let Err(e)=axum::serve(l1, public_app).await{ warn!("[public] error: {}",e);} });
        let h2 = tokio::spawn(async move{ if let Err(e)=axum::serve(l2, admin_app).await{ warn!("[admin] error: {}",e);} });
        let want_preview = std::env::var("SIDECAM_PREVIEW").is_ok() || std::env::args().any(|a| a=="--preview");
        if args.headless || !want_preview {
            if !args.headless && !want_preview { println!("(preview mati default — --preview untuk jendela)"); }
            tokio::select!{ _=h1=>{}, _=h2=>{} }
        } else {
            let srv_h = tokio::spawn(async move{ tokio::select!{ _=h1=>{}, _=h2=>{} } });
            run_preview_loop(srv_h).await;
        }
    }
    Ok(())
}

fn check_admin_headers(headers: &axum::http::HeaderMap) -> Result<(), (axum::http::StatusCode, String)> {
    // Host must be 127.0.0.1:port or localhost:port
    if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
        let h = host.to_ascii_lowercase();
        let ok = h.starts_with("127.0.0.1:") || h.starts_with("localhost:") || h=="127.0.0.1" || h=="localhost";
        if !ok { return Err((axum::http::StatusCode::FORBIDDEN, "Host not allowed".into())); }
    }
    // Origin check for mutating requests — reject if Origin is LAN IP
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let o = origin.to_ascii_lowercase();
        // allow missing origin, allow localhost/127.0.0.1 origin, deny others
        if !o.is_empty() && !o.contains("127.0.0.1") && !o.contains("localhost") {
            return Err((axum::http::StatusCode::FORBIDDEN, "Origin not allowed".into()));
        }
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
async fn run_preview_loop(server_handle:tokio::task::JoinHandle<()>){
    let frames:std::sync::Arc<Mutex<u64>>=std::sync::Arc::new(Mutex::new(0)); let start=std::time::Instant::now();
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
