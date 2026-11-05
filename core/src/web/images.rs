// SPDX-License-Identifier: AGPL-3.0-only
//! Pictures for Threema Web in the sizes Android sends (its `Protocol.java`): message thumbnails
//! at most 350 px with a 50 px preview inside the message, profile pictures at 48 px (lists) or
//! 512 px, and the 512 px thumbnail of an image the browser sends (like our own FilePreparer).
//! JPEG and PNG only; anything else, or a picture that doesn't decode, gives `None`.

use std::io::Cursor;

use image::{
    DynamicImage, ImageFormat, ImageReader, codecs::jpeg::JpegEncoder, imageops::FilterType,
};

pub const THUMBNAIL_MAX: u32 = 350;
pub const PREVIEW_MAX: u32 = 50;
pub const AVATAR_LOW_MAX: u32 = 48;
pub const AVATAR_HIGH_MAX: u32 = 512;
pub const SENT_THUMBNAIL_MAX: u32 = 512;

/// Pictures larger than this are not decoded (memory on a phone).
const MAX_PIXELS: u64 = 40_000_000;

fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if !matches!(reader.format(), Some(ImageFormat::Jpeg | ImageFormat::Png)) {
        return None;
    }
    let (width, height) = reader.into_dimensions().ok()?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

/// Fit within `max` × `max` (never enlarged).
fn fit(image: &DynamicImage, max: u32) -> DynamicImage {
    if image.width() <= max && image.height() <= max {
        return image.clone();
    }
    image.resize(max, max, FilterType::Triangle)
}

/// The size [`fit`] comes to, without scaling anything.
fn fitted_size(width: u32, height: u32, max: u32) -> (u32, u32) {
    if width <= max && height <= max {
        return (width, height);
    }
    let ratio = (f64::from(max) / f64::from(width)).min(f64::from(max) / f64::from(height));
    let side = |length: u32| ((f64::from(length) * ratio).round() as u32).max(1);
    (side(width), side(height))
}

/// JPEG (transparency flattened onto white, like FilePreparer's thumbnails).
fn jpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let rgba = image.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = u16::from(pixel[3]);
        let channel = |c: u8| ((u16::from(c) * alpha + 255 * (255 - alpha)) / 255) as u8;
        rgb.put_pixel(
            x,
            y,
            image::Rgb([channel(pixel[0]), channel(pixel[1]), channel(pixel[2])]),
        );
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

/// `bytes` within `max` px as JPEG.
#[must_use]
pub fn scaled_jpeg(bytes: &[u8], max: u32, quality: u8) -> Option<Vec<u8>> {
    jpeg(&fit(&decode(bytes)?, max), quality)
}

/// A message's thumbnail for the browser: its size within 350 px and a 50 px preview.
#[must_use]
pub fn thumbnail_info(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let image = decode(bytes)?;
    let (width, height) = fitted_size(image.width(), image.height(), THUMBNAIL_MAX);
    Some((width, height, jpeg(&fit(&image, PREVIEW_MAX), 50)?))
}

/// An image the browser sends: its size and our 512 px JPEG thumbnail; `None` if not a JPEG or
/// PNG we can read.
#[must_use]
pub fn sent_image(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let image = decode(bytes)?;
    Some((
        image.width(),
        image.height(),
        jpeg(&fit(&image, SENT_THUMBNAIL_MAX), 80)?,
    ))
}

/// A GIF's size from its header.
#[must_use]
pub fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !matches!(bytes.get(..6)?, b"GIF87a" | b"GIF89a") {
        return None;
    }
    let side = |at: usize| {
        Some(u32::from(u16::from_le_bytes([
            *bytes.get(at)?,
            *bytes.get(at + 1)?,
        ])))
    };
    Some((side(6)?, side(8)?)).filter(|(width, height)| *width > 0 && *height > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gifs_size_is_in_its_header() {
        let gif = b"GIF89a\x40\x01\xf0\x00rest";
        assert_eq!(gif_size(gif), Some((320, 240)));
        assert_eq!(gif_size(b"GIF89a\x40"), None);
        assert_eq!(gif_size(b"GIF89a\x00\x00\xf0\x00"), None);
        assert_eq!(gif_size(b"PNG"), None);
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([10, 200, 30, 128]),
        ));
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn the_size_comes_out_as_scaling_would() {
        for (width, height) in [
            (1000, 500),
            (500, 1000),
            (351, 350),
            (350, 350),
            (20, 10),
            (1000, 1),
            (3, 2000),
            (1279, 853),
            (853, 1279),
            (1024, 1023),
        ] {
            let image = DynamicImage::ImageRgb8(image::RgbImage::new(width, height));
            let scaled = fit(&image, THUMBNAIL_MAX);
            assert_eq!(
                fitted_size(width, height, THUMBNAIL_MAX),
                (scaled.width(), scaled.height()),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn sizes() {
        let picture = png(1000, 500);
        let (width, height, preview) = thumbnail_info(&picture).unwrap();
        assert_eq!((width, height), (350, 175));
        let preview = decode(&preview).unwrap();
        assert_eq!((preview.width(), preview.height()), (50, 25));
        let avatar = decode(&scaled_jpeg(&picture, AVATAR_LOW_MAX, 90).unwrap()).unwrap();
        assert_eq!(avatar.width(), 48);
        let (w, h, thumbnail) = sent_image(&png(20, 10)).unwrap();
        assert_eq!((w, h), (20, 10));
        assert_eq!(decode(&thumbnail).unwrap().width(), 20);
        assert!(scaled_jpeg(b"GIF89a....", 48, 90).is_none());
    }
}
