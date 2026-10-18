//! crates/vcam — trait VirtualCamera
//! Abstraksi kamera virtual. Kode Windows isolasi di balik trait.
//!
//! Konsep Rust:
//! - trait + Send: trait sbg interface, Send agar aman kirim antar thread tokio
//! - ownership: Frame owns Vec<u8>, push_frame pinjam &Frame (& = borrow)
//! - cfg: kompilasi kondisional per OS, zero-cost

use anyhow::{anyhow, Result};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    I420,
    Yuyv,
    Rgb24,
    Rgba,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    pub timestamp_ms: u64,
}

impl Frame {
    pub fn new(width: u32, height: u32, format: PixelFormat, data: Vec<u8>) -> Self {
        Self { width, height, format, data, timestamp_ms: 0 }
    }
    /// Frame placeholder saat HP terputus (FR-9)
    pub fn placeholder(width: u32, height: u32) -> Self {
        // ponytail: gradient sederhana, upgrade ke teks "HP terputus" di M2
        let mut data = vec![0u8; (width * height * 3) as usize];
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 3) as usize;
                data[i] = ((x * 255) / width) as u8;
                data[i + 1] = ((y * 255) / height) as u8;
                data[i + 2] = 128;
            }
        }
        Self { width, height, format: PixelFormat::Rgb24, data, timestamp_ms: 0 }
    }
}

/// Trait inti. Send agar bisa dipindah antar task tokio.
pub trait VirtualCamera: Send {
    fn open(&mut self) -> Result<()>;
    fn push_frame(&mut self, frame: &Frame) -> Result<()>;
    fn close(&mut self) -> Result<()>;
    fn is_open(&self) -> bool;
    fn name(&self) -> &str;
}

// ── Placeholder ──────────────────────────────────────────────────────────

pub struct PlaceholderCamera {
    name: String,
    open: bool,
    frames_pushed: u64,
}

impl PlaceholderCamera {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), open: false, frames_pushed: 0 }
    }
    pub fn frames_pushed(&self) -> u64 { self.frames_pushed }
}

impl VirtualCamera for PlaceholderCamera {
    fn open(&mut self) -> Result<()> {
        self.open = true;
        info!("[vcam] placeholder opened: {}", self.name);
        Ok(())
    }
    fn push_frame(&mut self, frame: &Frame) -> Result<()> {
        if !self.open { return Err(anyhow!("placeholder not open")); }
        self.frames_pushed += 1;
        if self.frames_pushed % 90 == 0 {
            info!("[vcam] placeholder frame #{} {}x{} {:?}", self.frames_pushed, frame.width, frame.height, frame.format);
        }
        Ok(())
    }
    fn close(&mut self) -> Result<()> { self.open = false; Ok(()) }
    fn is_open(&self) -> bool { self.open }
    fn name(&self) -> &str { &self.name }
}

// ── Linux v4l2loopback ───────────────────────────────────────────────────

#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;

    pub struct V4l2LoopbackCamera {
        device: String,
        file: Option<File>,
    }

    impl V4l2LoopbackCamera {
        pub fn new(device: impl Into<String>) -> Self {
            Self { device: device.into(), file: None }
        }
        pub fn device_path(&self) -> &str { &self.device }
    }

    impl VirtualCamera for V4l2LoopbackCamera {
        fn open(&mut self) -> Result<()> {
            let file = OpenOptions::new().write(true).open(&self.device)
                .map_err(|e| anyhow!("buka {} gagal: {} (buat: sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=SideCam exclusive_caps=1)", self.device, e))?;
            info!("[vcam] v4l2loopback opened {}", self.device);
            self.file = Some(file);
            Ok(())
        }
        fn push_frame(&mut self, frame: &Frame) -> Result<()> {
            let f = self.file.as_mut().ok_or_else(|| anyhow!("v4l2loopback not open"))?;
            if frame.format == PixelFormat::I420 {
                let expected = (frame.width * frame.height * 3 / 2) as usize;
                if frame.data.len() != expected {
                    warn!("[vcam] I420 size mismatch {} vs {}", frame.data.len(), expected);
                }
            }
            f.write_all(&frame.data)?;
            Ok(())
        }
        fn close(&mut self) -> Result<()> { self.file = None; Ok(()) }
        fn is_open(&self) -> bool { self.file.is_some() }
        fn name(&self) -> &str { &self.device }
    }
}

// ── Windows DirectShow ───────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub mod windows {
    use super::*;
    // ponytail: FFI ke native/vcam-windows DLL, real impl di M2
    pub struct DirectShowCamera { name: String, open: bool }
    impl DirectShowCamera {
        pub fn new(name: impl Into<String>) -> Self { Self { name: name.into(), open: false } }
    }
    impl VirtualCamera for DirectShowCamera {
        fn open(&mut self) -> Result<()> {
            // TODO M2: LoadLibrary + sidecam_vcam_open()
            self.open = true;
            info!("[vcam] DirectShow stub opened {}", self.name);
            Ok(())
        }
        fn push_frame(&mut self, _frame: &Frame) -> Result<()> {
            if !self.open { return Err(anyhow!("DirectShow not open")); }
            Ok(())
        }
        fn close(&mut self) -> Result<()> { self.open = false; Ok(()) }
        fn is_open(&self) -> bool { self.open }
        fn name(&self) -> &str { &self.name }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod windows {
    use super::*;
    pub struct DirectShowCamera { name: String }
    impl DirectShowCamera {
        pub fn new(name: impl Into<String>) -> Self { Self { name: name.into() } }
    }
    impl VirtualCamera for DirectShowCamera {
        fn open(&mut self) -> Result<()> { Err(anyhow!("DirectShow hanya di Windows")) }
        fn push_frame(&mut self, _: &Frame) -> Result<()> { Err(anyhow!("DirectShow hanya di Windows")) }
        fn close(&mut self) -> Result<()> { Ok(()) }
        fn is_open(&self) -> bool { false }
        fn name(&self) -> &str { &self.name }
    }
}

/// Factory — pilih implementasi sesuai OS.
pub fn create_default() -> Box<dyn VirtualCamera> {
    #[cfg(target_os = "linux")]
    {
        let dev = "/dev/video10";
        if std::path::Path::new(dev).exists() {
            return Box::new(linux::V4l2LoopbackCamera::new(dev));
        }
        warn!("[vcam] {} tidak ada, pakai Placeholder (buat: sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=SideCam exclusive_caps=1)", dev);
    }
    #[cfg(target_os = "windows")]
    {
        return Box::new(windows::DirectShowCamera::new("SideCam"));
    }
    Box::new(PlaceholderCamera::new("SideCam (placeholder)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholder_push() {
        let mut cam = PlaceholderCamera::new("test");
        cam.open().unwrap();
        cam.push_frame(&Frame::placeholder(640, 480)).unwrap();
        assert_eq!(cam.frames_pushed(), 1);
        cam.close().unwrap();
        assert!(!cam.is_open());
    }
    #[test]
    fn factory_creates() {
        let cam = create_default();
        assert!(!cam.name().is_empty());
    }
}
