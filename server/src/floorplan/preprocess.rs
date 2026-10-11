//! Preprocessing: decode, perspective rectification / border trim, scaling, binarisation.

use anyhow::Context;
use image::{imageops::FilterType, DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use imageproc::geometric_transformations::{warp_into, Interpolation, Projection};

pub const MAX_SIDE: u32 = 2048;
/// Small images are upscaled so thin and thick strokes separate better.
const MIN_SIDE: u32 = 1400;

/// Binary image, `true` = ink.
#[derive(Clone, Debug)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub d: Vec<bool>,
}

impl Mask {
    pub fn new(w: usize, h: usize) -> Self {
        Mask { w, h, d: vec![false; w * h] }
    }
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> bool {
        self.d[y * self.w + x]
    }
    /// Out-of-bounds = background.
    #[inline]
    pub fn get(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.d[y as usize * self.w + x as usize]
    }
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: bool) {
        self.d[y * self.w + x] = v;
    }
    pub fn transpose(&self) -> Mask {
        let mut t = Mask::new(self.h, self.w);
        for y in 0..self.h {
            for x in 0..self.w {
                t.d[x * self.h + y] = self.d[y * self.w + x];
            }
        }
        t
    }
    pub fn from_gray(g: &GrayImage, thr: u8) -> Mask {
        let (w, h) = (g.width() as usize, g.height() as usize);
        Mask { w, h, d: g.as_raw().iter().map(|&v| v < thr).collect() }
    }
    pub fn to_gray(&self) -> GrayImage {
        GrayImage::from_raw(self.w as u32, self.h as u32, self.d.iter().map(|&b| if b { 255 } else { 0 }).collect()).unwrap()
    }
}

pub struct Prepared {
    /// Rectified colour image (≤ 2048 px).
    pub rgb: RgbImage,
    pub gray: GrayImage,
    /// Contrast-stretched + Otsu binarised.
    pub ink: Mask,
    /// Looser threshold (anything clearly darker than paper): thin, light symbol lines
    /// (window panes, door leaves) for opening classification.
    pub light: Mask,
    pub png: Vec<u8>,
    /// Without corners: rectified px = (source px − src_offset) · src_scale.
    pub src_offset: [f64; 2],
    pub src_scale: f64,
    /// Prepared pixels per source pixel (resolution only; survives rotation, approximate for perspective).
    pub px_per_src: f64,
}

pub fn encode_png(img: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).context("PNG kodieren")?;
    Ok(out.into_inner())
}

pub fn preprocess(bytes: &[u8], corners: Option<[[f64; 2]; 4]>) -> anyhow::Result<Prepared> {
    let img = image::load_from_memory(bytes).context("Bild konnte nicht gelesen werden (PNG oder JPEG erwartet)")?;
    let rgb = flatten_alpha(&img);
    let (rgb, src_offset, src_scale, px_per_src) = match corners {
        Some(c) => {
            let d = |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
            let w = d(c[0], c[1]).max(d(c[3], c[2]));
            let r = rectify(&rgb, c)?;
            let k = r.width() as f64 / w.max(1.0);
            (r, [0.0, 0.0], f64::NAN, k)
        }
        None => {
            let (t, off) = trim_borders(&rgb);
            let w0 = t.width() as f64;
            let r = resize_limits(t);
            let k = r.width() as f64 / w0;
            (r, off, k, k)
        }
    };
    let mut p = from_rgb(rgb, src_offset, src_scale)?;
    p.px_per_src = px_per_src;
    Ok(p)
}

pub fn from_rgb(rgb: RgbImage, src_offset: [f64; 2], src_scale: f64) -> anyhow::Result<Prepared> {
    let gray = DynamicImage::ImageRgb8(rgb.clone()).to_luma8();
    let gray = stretch(&gray);
    let ink = binarize(&gray);
    let light = Mask::from_gray(&gray, 215);
    let png = encode_png(&DynamicImage::ImageRgb8(rgb.clone()))?;
    let px_per_src = if src_scale.is_finite() { src_scale } else { 1.0 };
    Ok(Prepared { rgb, gray, ink, light, png, src_offset, src_scale, px_per_src })
}

/// Rotate the rectified image by `deg` (clockwise on screen) about its centre, white fill.
/// The source→rectified transform is no longer a pure offset/scale afterwards (src_scale = NaN).
pub fn rotated(p: &Prepared, deg: f64) -> anyhow::Result<Prepared> {
    let img = imageproc::geometric_transformations::rotate_about_center(
        &p.rgb,
        deg.to_radians() as f32,
        Interpolation::Bilinear,
        Rgb([255, 255, 255]),
    );
    let mut out = from_rgb(img, [0.0, 0.0], f64::NAN)?;
    out.px_per_src = p.px_per_src;
    Ok(out)
}

/// Transparent pixels → white.
fn flatten_alpha(img: &DynamicImage) -> RgbImage {
    let rgba = img.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as f32 / 255.0;
        let c = |v: u8| (v as f32 * a + 255.0 * (1.0 - a)).round() as u8;
        out.put_pixel(x, y, Rgb([c(p[0]), c(p[1]), c(p[2])]));
    }
    out
}

fn scale_for(w: u32, h: u32) -> f64 {
    let m = w.max(h) as f64;
    if m > MAX_SIDE as f64 {
        MAX_SIDE as f64 / m
    } else if m < MIN_SIDE as f64 {
        (MIN_SIDE as f64 / m).min(2.0)
    } else {
        1.0
    }
}

fn resize_limits(img: RgbImage) -> RgbImage {
    let s = scale_for(img.width(), img.height());
    if (s - 1.0).abs() < 1e-9 {
        return img;
    }
    let nw = ((img.width() as f64 * s).round() as u32).max(1);
    let nh = ((img.height() as f64 * s).round() as u32).max(1);
    let filter = if s < 1.0 { FilterType::Triangle } else { FilterType::CatmullRom };
    image::imageops::resize(&img, nw, nh, filter)
}

/// Perspective-warp the quad (TL, TR, BR, BL) to an axis-aligned rectangle.
pub fn rectify(img: &RgbImage, c: [[f64; 2]; 4]) -> anyhow::Result<RgbImage> {
    let d = |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let w = d(c[0], c[1]).max(d(c[3], c[2]));
    let h = d(c[0], c[3]).max(d(c[1], c[2]));
    anyhow::ensure!(w >= 16.0 && h >= 16.0, "Eckpunkte ergeben ein zu kleines Rechteck");
    let s = scale_for(w.round() as u32, h.round() as u32);
    let (ow, oh) = ((w * s).round().max(1.0) as u32, (h * s).round().max(1.0) as u32);
    let from = c.map(|p| (p[0] as f32, p[1] as f32));
    let to = [(0.0, 0.0), (ow as f32, 0.0), (ow as f32, oh as f32), (0.0, oh as f32)];
    let proj = Projection::from_control_points(from, to).context("Eckpunkte ungültig (degeneriertes Viereck)")?;
    let mut out = RgbImage::new(ow, oh);
    warp_into(img, &proj, Interpolation::Bilinear, Rgb([255, 255, 255]), &mut out);
    Ok(out)
}

/// Crop near-white borders (keeps a small margin). Returns the crop and its offset.
fn trim_borders(img: &RgbImage) -> (RgbImage, [f64; 2]) {
    let (w, h) = img.dimensions();
    let dark = |x: u32, y: u32| {
        let p = img.get_pixel(x, y);
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 3 * 200
    };
    let row_dark = |y: u32| (0..w).filter(|&x| dark(x, y)).count();
    let col_dark = |x: u32| (0..h).filter(|&y| dark(x, y)).count();
    let min_row = (w as usize / 500).max(1);
    let min_col = (h as usize / 500).max(1);
    let y0 = (0..h).find(|&y| row_dark(y) >= min_row);
    let Some(y0) = y0 else { return (img.clone(), [0.0, 0.0]) };
    let y1 = (0..h).rev().find(|&y| row_dark(y) >= min_row).unwrap_or(h - 1);
    let x0 = (0..w).find(|&x| col_dark(x) >= min_col).unwrap_or(0);
    let x1 = (0..w).rev().find(|&x| col_dark(x) >= min_col).unwrap_or(w - 1);
    let margin = (w.max(h) / 100).max(4);
    let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (x1, y1) = ((x1 + margin).min(w - 1), (y1 + margin).min(h - 1));
    if x1 <= x0 + 16 || y1 <= y0 + 16 {
        return (img.clone(), [0.0, 0.0]);
    }
    (image::imageops::crop_imm(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image(), [x0 as f64, y0 as f64])
}

/// Linear contrast stretch between the 0.5 % and 99.5 % percentiles.
fn stretch(g: &GrayImage) -> GrayImage {
    let mut hist = [0usize; 256];
    for p in g.pixels() {
        hist[p[0] as usize] += 1;
    }
    let n = (g.width() * g.height()) as usize;
    let pct = |q: f64| {
        let target = (n as f64 * q) as usize;
        let mut acc = 0;
        for (i, c) in hist.iter().enumerate() {
            acc += c;
            if acc > target {
                return i as u8;
            }
        }
        255
    };
    let (lo, hi) = (pct(0.005), pct(0.995));
    if hi <= lo.saturating_add(10) {
        return g.clone();
    }
    let mut out = g.clone();
    for p in out.pixels_mut() {
        let v = (p[0] as f64 - lo as f64) / (hi as f64 - lo as f64) * 255.0;
        *p = Luma([v.clamp(0.0, 255.0) as u8]);
    }
    out
}

fn binarize(g: &GrayImage) -> Mask {
    let otsu = imageproc::contrast::otsu_level(g);
    // Mid-grey wall fills should count as ink; very light hatching not.
    let thr = otsu.clamp(110, 200).max(160);
    Mask::from_gray(g, thr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectify_keeps_square() {
        let mut img = RgbImage::from_pixel(400, 300, Rgb([255, 255, 255]));
        for y in 100..120 {
            for x in 50..350 {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        let out = rectify(&img, [[50.0, 50.0], [350.0, 50.0], [350.0, 250.0], [50.0, 250.0]]).unwrap();
        // 300x200 quad → upscaled by MIN_SIDE rule (×2 max)
        assert_eq!(out.width() * 200, out.height() * 300);
        let s = out.width() as f64 / 300.0;
        let p = out.get_pixel((150.0 * s) as u32, (60.0 * s) as u32);
        assert!(p[0] < 50, "{p:?}");
    }

    #[test]
    fn preprocess_png_roundtrip() {
        let mut img = RgbImage::from_pixel(1500, 1000, Rgb([250, 250, 250]));
        for y in 400..420 {
            for x in 100..1400 {
                img.put_pixel(x, y, Rgb([20, 20, 20]));
            }
        }
        let png = encode_png(&DynamicImage::ImageRgb8(img)).unwrap();
        let p = preprocess(&png, None).unwrap();
        assert!(p.ink.d.iter().any(|&b| b));
        assert!(p.rgb.width() <= MAX_SIDE);
    }
}
