//! mdns.rs — iklan _sidecam._tcp.local (PRD 8.1)
use std::collections::HashMap;
use tracing::{info, warn};

pub fn advertise(port: u16, host_ip: &str, device_name: &str, device_id: &str) {
    let host_ip = host_ip.to_string();
    let device_name = device_name.to_string();
    let device_id = device_id.to_string();
    std::thread::spawn(move || {
        let Ok(mdns) = mdns_sd::ServiceDaemon::new() else { warn!("[mdns] daemon gagal"); return; };
        let service_type = "_sidecam._tcp.local.";
        let instance = format!("SideCam-{}", &device_id[..8.min(device_id.len())]);
        let host = format!("{}.local.", device_name.replace(' ', "-").to_lowercase());
        let mut props: HashMap<String, String> = HashMap::new();
        props.insert("v".into(), "1".into());
        props.insert("name".into(), device_name.clone());
        props.insert("id".into(), device_id.clone());
        let Ok(svc) = mdns_sd::ServiceInfo::new(service_type, &instance, &host, &host_ip, port, props) else {
            warn!("[mdns] ServiceInfo gagal"); return;
        };
        if let Err(e) = mdns.register(svc) { warn!("[mdns] register gagal: {}", e); return; }
        info!("[mdns] iklan {} @ {}:{}", service_type, host_ip, port);
        // keep thread alive while daemon runs
        loop { std::thread::sleep(std::time::Duration::from_secs(60)); }
    });
}

pub fn pairing_qr_svg(json: &str) -> String {
    // ponytail: SVG tanpa dep image, langsung embed di HTML. Upgrade M4 ke canvas/PNG jika perlu binary QR.
    let Ok(code) = qrcode::QrCode::new(json.as_bytes()) else { return String::new(); };
    code.render::<qrcode::render::svg::Color<'_>>().min_dimensions(240,240).build()
}
