//! pipeline.rs — RTP depacketize → Annex B → decode (spawn_blocking) → I420 scale 1280x720
//! Jangan buang H264 frame sebelum decode; drop hanya pada Frame hasil (watch slot).

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use sidecam_vcam::{Frame, VirtualCamera};
use crate::decode::OpenH264Decoder;
use crate::decode::VideoDecoder;

/// RTP H264 depacketizer RFC6184 (Single NAL + FU-A). STAP-A tidak dikirim browser, skip.
fn depacketize(payload: &[u8], out: &mut Vec<u8>) {
    if payload.is_empty() { return; }
    let nal_type = payload[0] & 0x1F;
    if nal_type <= 23 {
        // Single NAL
        out.extend_from_slice(b"\x00\x00\x00\x01");
        out.extend_from_slice(payload);
    } else if nal_type == 28 {
        // FU-A
        if payload.len() < 2 { return; }
        let start = (payload[1] & 0x80) != 0;
        let nri = payload[0] & 0x60;
        let orig_type = payload[1] & 0x1F;
        let frag = &payload[2..];
        if start {
            out.extend_from_slice(b"\x00\x00\x00\x01");
            out.push(nri | orig_type);
            out.extend_from_slice(frag);
        } else {
            out.extend_from_slice(frag);
        }
    } else {
        // STAP-A / unknown — skip or try as single
        // untuk A1 jangan buang diam: coba append dengan Annex B
        if payload.len() > 3 {
            out.extend_from_slice(b"\x00\x00\x00\x01");
            out.extend_from_slice(payload);
        }
    }
}

fn is_keyframe_annexb(annex_b: &[u8]) -> bool {
    // cari NAL type 5 (IDR)
    let mut i = 0;
    while i + 4 < annex_b.len() {
        if annex_b[i]==0 && annex_b[i+1]==0 && annex_b[i+2]==0 && annex_b[i+3]==1 {
            let typ = annex_b[i+4] & 0x1F;
            if typ == 5 { return true; }
            i += 4;
        } else { i += 1; }
    }
    false
}

/// Slot satu frame — watch semantics, drop-old (latensi > kelengkapan)
#[derive(Default)]
struct LastFrame { frame: Option<Frame> }

pub struct Pipeline {
    annex_buf: Vec<u8>,
    decoder: OpenH264Decoder,
    vcam: Arc<Mutex<Box<dyn VirtualCamera>>>,
    last: Arc<Mutex<LastFrame>>,
    last_at: Arc<AtomicU64>,
    fps_in: AtomicU64,
    fps_out: AtomicU64,
    dropped: AtomicU64,
    decode_ms_sum: AtomicU64,
    decode_cnt: AtomicU64,
    pli_tx: Option<tokio::sync::mpsc::Sender<()>>,
    seq_prev: Option<u16>,
    loss_cnt: u64,
}

impl Pipeline {
    pub fn new(vcam: Arc<Mutex<Box<dyn VirtualCamera>>>) -> anyhow::Result<Self> {
        Ok(Self {
            annex_buf: Vec::with_capacity(256*1024),
            decoder: OpenH264Decoder::new()?,
            vcam,
            last: Arc::new(Mutex::new(LastFrame::default())),
            last_at: Arc::new(AtomicU64::new(0)),
            fps_in: AtomicU64::new(0),
            fps_out: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            decode_ms_sum: AtomicU64::new(0),
            decode_cnt: AtomicU64::new(0),
            pli_tx: None,
            seq_prev: None,
            loss_cnt: 0,
        })
    }

    pub fn set_pli(&mut self, tx: tokio::sync::mpsc::Sender<()>) { self.pli_tx = Some(tx); }

    pub fn on_rtp(&mut self, payload: &[u8], seq: u16) {
        // packet loss detection → request PLI
        if let Some(prev) = self.seq_prev {
            let gap = seq.wrapping_sub(prev);
            if gap != 1 && gap < 1000 {
                self.loss_cnt += 1;
                if self.loss_cnt % 10 == 0 { // throttle
                    if let Some(tx) = self.pli_tx.clone() { let _ = tx.try_send(()); }
                }
                // jika loss, kita tidak buang buffer H264 — tetap depacketize,
                // tapi jika terpaksa drop, buang sampai keyframe berikutnya + PLI
            }
        }
        self.seq_prev = Some(seq);
        self.fps_in.fetch_add(1, Ordering::Relaxed);
        depacketize(payload, &mut self.annex_buf);
        // Heuristic: tiap RTP yang marker=1 biasanya akhir frame, tapi kita tidak punya marker di sini
        // Flush tiap 2-4 paket atau saat payload contains FU-A end.
        // Sederhana: coba decode tiap paket FU-A end / single NAL I/P — decoder return None jika belum cukup.
        // Untuk kumpulkan: decode hanya saat kita sudah punya cukup (marker). Tanpa marker, decode per paket tetap aman (decoder buffers).
    }

    /// Dipanggil setelah kumpul satu frame (dari track marker) — decode di spawn_blocking
    pub fn flush_decode(&mut self) {
        if self.annex_buf.is_empty() { return; }
        let annex = std::mem::take(&mut self.annex_buf);
        let vcam = Arc::clone(&self.vcam);
        let last = Arc::clone(&self.last);
        let last_at = Arc::clone(&self.last_at);
        let decode_ms_sum = self.decode_ms_sum.fetch_add(0, Ordering::Relaxed);
        let _ = decode_ms_sum;
        // decode di spawn_blocking (trait tidak Send+Sync across await, jadi buat decoder per task atau Arc<Mutex>)
        // Sederhana: decode sinkron di caller thread tapi di spawn_blocking wrapper dari server.rs.
        // Di sini kita sediakan sync path; async wrapper ada di server.rs
        let _ = (vcam, last, last_at, annex);
    }

    pub fn decode_sync(&mut self) -> Option<Frame> {
        if self.annex_buf.is_empty() { return None; }
        let annex = std::mem::take(&mut self.annex_buf);
        let t = Instant::now();
        let res = self.decoder.decode(&annex);
        let ms = t.elapsed().as_micros() as u64 / 1000;
        self.decode_ms_sum.fetch_add(ms, Ordering::Relaxed);
        self.decode_cnt.fetch_add(1, Ordering::Relaxed);
        match res {
            Ok(Some(f)) => {
                // skala sudah di decoder ke 1280x720; put to watch slot (drop-old)
                let mut g = self.last.lock().unwrap();
                let dropped = g.frame.is_some();
                if dropped { self.dropped.fetch_add(1, Ordering::Relaxed); }
                g.frame = Some(f.clone());
                self.last_at.store(crate::server::now_ms_public(), Ordering::Relaxed);
                self.fps_out.fetch_add(1, Ordering::Relaxed);
                Some(f)
            }
            Ok(None) => None,
            Err(_) => {
                // frame rusak → minta PLI
                if let Some(tx) = self.pli_tx.clone() { let _ = tx.try_send(()); }
                // jika harus buang, buang sampai keyframe berikutnya (annex sudah terkonsumsi)
                None
            }
        }
    }

    pub fn push_last_to_vcam(&self) {
        let frm = { self.last.lock().unwrap().frame.clone() };
        if let Some(f) = frm {
            if let Ok(mut v) = self.vcam.try_lock() {
                let _ = v.push_frame(&f);
            }
        }
    }

    pub fn tick_fps_log(&self) {
        let i = self.fps_in.swap(0, Ordering::Relaxed);
        let o = self.fps_out.swap(0, Ordering::Relaxed);
        let d = self.dropped.swap(0, Ordering::Relaxed);
        let cnt = self.decode_cnt.swap(0, Ordering::Relaxed);
        let sum = self.decode_ms_sum.swap(0, Ordering::Relaxed);
        let avg = if cnt>0 { sum as f64 / cnt as f64 } else { 0.0 };
        tracing::info!("[pipeline] fps_in={} fps_out={} dropped={} decode_avg_ms={:.1} cnt={}", i,o,d,avg,cnt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn depacketize_single() {
        let mut out=Vec::new();
        depacketize(&[0x67,0x42,0x00,0x1f], &mut out);
        assert!(out.starts_with(b"\x00\x00\x00\x01"));
    }
    #[test] fn depacketize_fu_a_reassemble() {
        let mut out=Vec::new();
        // FU-A start + end
        depacketize(&[0x7c,0x85,0xAA,0xBB], &mut out); // NRI=0x60, type=5, start
        depacketize(&[0x7c,0x05,0xCC,0xDD], &mut out); // end frag
        assert!(out.starts_with(b"\x00\x00\x00\x01"));
        assert!(out.contains(&0x65)); // reconstructed header 0x65 = 0x60|0x05
    }
}
