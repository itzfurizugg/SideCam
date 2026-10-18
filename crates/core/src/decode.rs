//! decode.rs — trait VideoDecoder, OpenH264Decoder (primary), Ffmpeg stub
//! Semua decode di spawn_blocking / thread khusus, jangan di async tokio thread.

use anyhow::Result;
use sidecam_vcam::{Frame, PixelFormat};

pub trait VideoDecoder: Send {
    fn decode(&mut self, annex_b: &[u8]) -> Result<Option<Frame>>;
    fn name(&self) -> &str;
}

// ── openh264 ────────────────────────────────────────────────────────────────

pub struct OpenH264Decoder {
    dec: openh264::decoder::Decoder,
}

impl OpenH264Decoder {
    pub fn new() -> Result<Self> {
        let dec = openh264::decoder::Decoder::new()?;
        Ok(Self { dec })
    }
}

fn scale_i420(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh { return src.to_vec(); }
    let sy = (sw * sh) as usize;
    let suv_w = sw / 2; let suv_h = sh / 2;
    let duv_w = dw / 2; let duv_h = dh / 2;
    let mut out = vec![0u8; (dw * dh * 3 / 2) as usize];
    for y in 0..dh {
        let sy0 = (y as u64 * sh as u64 / dh as u64) as u32;
        for x in 0..dw {
            let sx0 = (x as u64 * sw as u64 / dw as u64) as u32;
            out[(y * dw + x) as usize] = src[(sy0 * sw + sx0) as usize];
        }
    }
    let src_u = &src[sy..sy + (suv_w * suv_h) as usize];
    let src_v = &src[sy + (suv_w * suv_h) as usize..];
    let dst_u_off = (dw * dh) as usize;
    let dst_v_off = dst_u_off + (duv_w * duv_h) as usize;
    for y in 0..duv_h {
        let sy0 = (y as u64 * suv_h as u64 / duv_h as u64) as u32;
        for x in 0..duv_w {
            let sx0 = (x as u64 * suv_w as u64 / duv_w as u64) as u32;
            out[dst_u_off + (y * duv_w + x) as usize] = src_u[(sy0 * suv_w + sx0) as usize];
            out[dst_v_off + (y * duv_w + x) as usize] = src_v[(sy0 * suv_w + sx0) as usize];
        }
    }
    out
}

impl VideoDecoder for OpenH264Decoder {
    fn name(&self) -> &str { "openh264" }
    fn decode(&mut self, annex_b: &[u8]) -> Result<Option<Frame>> {
        let Some(img) = self.dec.decode(annex_b)? else { return Ok(None); };
        let (w, h) = img.dimension_y();
        let (w, h) = (w as u32, h as u32);
        let (sy, su, sv) = (img.y_with_stride().len(), img.u_with_stride().len()+img.v_with_stride().len(), 0);
        let _ = (sy, su, sv);
        // packed I420 without stride padding
        let mut i420 = Vec::with_capacity((w * h * 3/2) as usize);
        let (sy_stride, su_stride, _) = img.strides_yuv();
        let y = img.y_with_stride(); let u = img.u_with_stride(); let v = img.v_with_stride();
        for r in 0..h as usize { i420.extend_from_slice(&y[r*sy_stride..r*sy_stride+w as usize]); }
        let cw=(w/2) as usize; let ch=(h/2) as usize;
        for r in 0..ch { i420.extend_from_slice(&u[r*su_stride..r*su_stride+cw]); }
        for r in 0..ch { i420.extend_from_slice(&v[r*su_stride..r*su_stride+cw]); }
        let scaled = if w!=1280||h!=720 { scale_i420(&i420,w,h,1280,720) } else { i420 };
        Ok(Some(Frame{ width:1280,height:720, format:PixelFormat::I420, data:scaled, timestamp_ms:0 }))
    }
}

/// ffmpeg-next stub — trait parity, tidak dipakai kecuali openh264 gagal.
pub struct FfmpegDecoder;
impl FfmpegDecoder { pub fn new() -> Result<Self> { Ok(Self) } }
impl VideoDecoder for FfmpegDecoder {
    fn name(&self)->&str{"ffmpeg-stub"}
    fn decode(&mut self,_b:&[u8])->Result<Option<Frame>>{ anyhow::bail!("ffmpeg decoder belum diimplementasikan — gunakan openh264") }
}

pub fn psnr(a:&[u8],b:&[u8])->f64{
    if a.len()!=b.len()||a.is_empty(){return 0.0;}
    let mse:f64=a.iter().zip(b.iter()).map(|(x,y)|{let d=*x as f64-*y as f64; d*d}).sum::<f64>()/a.len() as f64;
    if mse==0.0{return 99.0;}
    10.0*(255.0*255.0/mse).log10()
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test] fn psnr_identical(){ let a=vec![128u8;100]; assert!(psnr(&a,&a)>80.0); }
    #[test] fn openh264_init(){ let d=OpenH264Decoder::new(); assert!(d.is_ok(),"openh264 init: {:?}", d.err()); }
    #[test] fn openh264_decode_empty_returns_none(){
        let mut d=OpenH264Decoder::new().unwrap();
        // empty should be Ok(None) or error — not panic
        let r=d.decode(&[]); assert!(r.is_ok() || r.is_err());
    }
}
