use std::path::Path;

use font8x8::UnicodeFonts;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, Pixel, Rgb, Rgba, RgbaImage};

/// Render a simple LCD frame (background + optional icon + optional text) to JPEG bytes.
///
/// This is intentionally “dumb but reliable” for MVP:
/// - background: either a solid RGB or a default dark gray
/// - icon: optional image from disk; resized to fit and centered
/// - text: optional single-line text rendered with an 8x8 bitmap font
pub fn render_lcd_jpeg(
    width: u32,
    height: u32,
    background_rgb: Option<[u8; 3]>,
    icon_path: Option<&Path>,
    text: Option<&str>,
) -> anyhow::Result<Vec<u8>> {
    let bg = background_rgb.unwrap_or([16, 16, 18]);

    let mut frame: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([bg[0], bg[1], bg[2], 255]));

    if let Some(path) = icon_path {
        if let Ok(img) = image::open(path) {
            overlay_icon(&mut frame, &img);
        }
    }

    if let Some(t) = text {
        draw_text_bottom_center(&mut frame, t, Rgba([235, 235, 240, 255]));
    }

    // JPEG has no alpha, so flatten to RGB.
    let mut rgb = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(width, height);
    for (x, y, px) in frame.enumerate_pixels() {
        let c = px.to_rgb();
        rgb.put_pixel(x, y, c);
    }

    let mut out = Vec::new();
    let mut enc = JpegEncoder::new_with_quality(&mut out, 90);
    enc.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ColorType::Rgb8.into(),
    )?;
    Ok(out)
}

fn overlay_icon(dst: &mut RgbaImage, icon: &DynamicImage) {
    let (w, h) = dst.dimensions();
    if w == 0 || h == 0 {
        return;
    }

    let max_w = (w as f32 * 0.70).max(1.0) as u32;
    let max_h = (h as f32 * 0.60).max(1.0) as u32;

    let icon_rgba = icon.to_rgba8();
    let (iw, ih) = icon_rgba.dimensions();
    if iw == 0 || ih == 0 {
        return;
    }

    // Fit within max box, preserve aspect.
    let scale = (max_w as f32 / iw as f32).min(max_h as f32 / ih as f32);
    let rw = ((iw as f32 * scale).max(1.0)).round() as u32;
    let rh = ((ih as f32 * scale).max(1.0)).round() as u32;

    let resized = image::imageops::resize(&icon_rgba, rw, rh, FilterType::Lanczos3);

    // Center slightly above vertical center so bottom text has room.
    let ox = (w.saturating_sub(rw)) / 2;
    let oy = (h.saturating_sub(rh)) / 2;
    alpha_blit(dst, &resized, ox, oy);
}

fn draw_text_bottom_center(img: &mut RgbaImage, text: &str, color: Rgba<u8>) {
    // Use 8x8 font, scale up for readability.
    let scale: u32 = ((img.height() as f32 / 72.0).clamp(1.0, 3.0)).round() as u32;
    let char_w = 8 * scale;
    let char_h = 8 * scale;
    let padding = 4 * scale;

    let printable: Vec<char> = text.chars().take(20).collect();
    if printable.is_empty() {
        return;
    }

    let text_w = (printable.len() as u32) * char_w;
    let x0 = (img.width().saturating_sub(text_w)) / 2;
    let y0 = img.height().saturating_sub(char_h + padding);

    // Slight shadow for contrast.
    let shadow = Rgba([0, 0, 0, 180]);
    draw_text_at(img, x0 + scale, y0 + scale, &printable, scale, shadow);
    draw_text_at(img, x0, y0, &printable, scale, color);
}

fn draw_text_at(
    img: &mut RgbaImage,
    x0: u32,
    y0: u32,
    text: &[char],
    scale: u32,
    color: Rgba<u8>,
) {
    for (i, ch) in text.iter().enumerate() {
        let x = x0 + (i as u32) * 8 * scale;
        draw_char(img, x, y0, *ch, scale, color);
    }
}

fn draw_char(img: &mut RgbaImage, x0: u32, y0: u32, ch: char, scale: u32, color: Rgba<u8>) {
    // `font8x8` supports a large subset of Unicode; fallback to '?'.
    let glyph = font8x8::BASIC_FONTS.get(ch).or_else(|| font8x8::BASIC_FONTS.get('?'));
    let Some(rows) = glyph else {
        return;
    };

    for (row, bits) in rows.iter().copied().enumerate() {
        for col in 0..8 {
            if (bits >> col) & 1 == 0 {
                continue;
            }
            let px = x0 + (7 - col) as u32 * scale;
            let py = y0 + row as u32 * scale;
            fill_rect(img, px, py, scale, scale, color);
        }
    }
}

fn fill_rect(img: &mut RgbaImage, x0: u32, y0: u32, w: u32, h: u32, c: Rgba<u8>) {
    for y in y0..y0.saturating_add(h) {
        if y >= img.height() {
            break;
        }
        for x in x0..x0.saturating_add(w) {
            if x >= img.width() {
                break;
            }
            img.put_pixel(x, y, c);
        }
    }
}

fn alpha_blit(dst: &mut RgbaImage, src: &RgbaImage, ox: u32, oy: u32) {
    for (sx, sy, spx) in src.enumerate_pixels() {
        let dx = ox + sx;
        let dy = oy + sy;
        if dx >= dst.width() || dy >= dst.height() {
            continue;
        }

        let sa = spx.0[3] as u16;
        if sa == 0 {
            continue;
        }

        let dp = dst.get_pixel(dx, dy);
        let da = dp.0[3] as u16;

        let inv_sa = 255u16.saturating_sub(sa);

        let out = [
            blend_channel(spx.0[0], dp.0[0], sa, inv_sa),
            blend_channel(spx.0[1], dp.0[1], sa, inv_sa),
            blend_channel(spx.0[2], dp.0[2], sa, inv_sa),
            ((sa + (da * inv_sa) / 255).min(255)) as u8,
        ];
        dst.put_pixel(dx, dy, Rgba(out));
    }
}

fn blend_channel(src: u8, dst: u8, sa: u16, inv_sa: u16) -> u8 {
    let s = src as u16;
    let d = dst as u16;
    (((s * sa) + (d * inv_sa)) / 255).min(255) as u8
}


