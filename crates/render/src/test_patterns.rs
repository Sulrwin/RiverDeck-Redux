use image::codecs::jpeg::JpegEncoder;
use image::{ImageBuffer, Rgb};

/// Generate a solid-color JPEG image (RGB) for quick device bring-up.
pub fn solid_color_jpeg(width: u32, height: u32, rgb: [u8; 3]) -> anyhow::Result<Vec<u8>> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(width, height, Rgb(rgb));

    let mut out = Vec::new();
    let mut enc = JpegEncoder::new_with_quality(&mut out, 90);
    enc.encode(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ColorType::Rgb8.into(),
    )?;

    Ok(out)
}
