// SPDX-License-Identifier: AGPL-3.0-only
//! Pictures for Threema Web in the sizes Android sends (its `Protocol.java`): message thumbnails
//! at most 350 px with a 50 px preview inside the message, profile pictures at 48 px (lists) or
//! 512 px, and a picture the browser sends as media, made like our own FilePreparer's.
//! Decoding is JPEG and PNG only; anything else, or a picture that doesn't decode, gives `None`.

use std::io::Cursor;

use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, codecs::jpeg::JpegEncoder,
    imageops::FilterType, metadata::Orientation,
};
use zeroize::Zeroizing;

pub const THUMBNAIL_MAX: u32 = 350;
pub const PREVIEW_MAX: u32 = 50;
pub const AVATAR_LOW_MAX: u32 = 48;
pub const AVATAR_HIGH_MAX: u32 = 512;
pub const SENT_THUMBNAIL_MAX: u32 = 512;
/// A picture sent as media: Android's and our FilePreparer's default.
pub const SENT_MAX: u32 = 1600;

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
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    Some(image)
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

/// What a picture goes as, once `sent_picture` has made it fit to leave.
pub struct SentPicture {
    pub media_type: &'static str,
    pub size: Option<(u32, u32)>,
    pub thumbnail: Option<Vec<u8>>,
}

/// A picture that says it is one, and whose metadata can't be vouched for.
#[derive(Debug, PartialEq, Eq)]
pub struct PictureKeepsMetadata;

/// Makes a picture sent as media (`claimed` its media type) fit to leave, in place: what a file
/// says about itself stays behind. JPEG and PNG are made anew like FilePreparer's: upright, at most
/// `SENT_MAX`, JPEG 85 or PNG if transparent, with our 512 px thumbnail. A WebP loses all but its
/// picture and animation. A GIF goes as it is, as on the phone and on Android. `None`: another kind
/// of file, whose bytes are untouched. Takes long, for a blocking thread.
pub fn sent_picture(
    data: &mut Zeroizing<Vec<u8>>,
    claimed: &str,
) -> Result<Option<SentPicture>, PictureKeepsMetadata> {
    match image::guess_format(data).ok() {
        Some(ImageFormat::Jpeg | ImageFormat::Png) => {
            let image = decode(data).ok_or(PictureKeepsMetadata)?;
            let shown = fit(&image, SENT_MAX);
            let transparent = shown.color().has_alpha();
            let encoded = if transparent {
                let mut out = Vec::new();
                shown
                    .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
                    .map_err(|_| PictureKeepsMetadata)?;
                out
            } else {
                jpeg(&shown, 85).ok_or(PictureKeepsMetadata)?
            };
            let thumbnail =
                jpeg(&fit(&image, SENT_THUMBNAIL_MAX), 80).ok_or(PictureKeepsMetadata)?;
            *data = Zeroizing::new(encoded);
            Ok(Some(SentPicture {
                media_type: if transparent {
                    "image/png"
                } else {
                    "image/jpeg"
                },
                size: Some((shown.width(), shown.height())),
                thumbnail: Some(thumbnail),
            }))
        }
        Some(ImageFormat::WebP) => {
            let size = crate::webp::clean(data).map_err(|_| PictureKeepsMetadata)?;
            Ok(Some(SentPicture {
                media_type: "image/webp",
                size,
                thumbnail: None,
            }))
        }
        Some(ImageFormat::Gif) => Ok(Some(SentPicture {
            media_type: "image/gif",
            size: gif_size(data),
            thumbnail: None,
        })),
        // It says it is one of these, and it isn't.
        _ if matches!(
            claimed,
            "image/jpeg" | "image/jpg" | "image/png" | "image/webp" | "image/gif"
        ) =>
        {
            Err(PictureKeepsMetadata)
        }
        _ => Ok(None),
    }
}

/// A GIF's size from its header.
fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
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

/// Pictures for other modules' tests.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;

    /// What a file says of itself in the pictures here, to look for in what leaves.
    pub(crate) const SECRET: &[u8] = b"GPS-SECRET";

    /// A JPEG of 20 x 10 that its EXIF says to turn by 90 degrees.
    pub(crate) fn jpeg_with_exif() -> Vec<u8> {
        let plain = jpeg(
            &DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                20,
                10,
                image::Rgb([200, 30, 30]),
            )),
            90,
        )
        .unwrap();
        // One entry: orientation (0x0112), a short, 6.
        let exif = [
            b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".as_slice(),
            SECRET,
        ]
        .concat();
        let length = u16::try_from(exif.len() + 2).unwrap().to_be_bytes();
        [&plain[..2], &[0xff, 0xe1], &length, &exif, &plain[2..]].concat()
    }

    fn crc(data: &[u8]) -> u32 {
        !data.iter().fold(u32::MAX, |crc, byte| {
            (0..8).fold(crc ^ u32::from(*byte), |crc, _| {
                (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg())
            })
        })
    }

    pub(crate) fn png_chunk(name: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let body = [name.as_slice(), content].concat();
        [
            &u32::try_from(content.len()).unwrap().to_be_bytes()[..],
            &body,
            &crc(&body).to_be_bytes(),
        ]
        .concat()
    }

    /// A PNG of 20 x 10, transparent or not, with a text chunk.
    pub(crate) fn png_with_text(transparent: bool) -> Vec<u8> {
        let image = if transparent {
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                20,
                10,
                image::Rgba([10, 200, 30, 128]),
            ))
        } else {
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                20,
                10,
                image::Rgb([10, 200, 30]),
            ))
        };
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        // After the signature and the header chunk.
        let text = png_chunk(b"tEXt", &[b"Comment\0", SECRET].concat());
        [&out[..33], &text, &out[33..]].concat()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fixture::{SECRET, jpeg_with_exif, png_chunk, png_with_text},
        *,
    };

    fn contains(data: &[u8], part: &[u8]) -> bool {
        data.windows(part.len()).any(|window| window == part)
    }

    #[test]
    fn a_jpeg_leaves_upright_and_without_its_exif() {
        let source = jpeg_with_exif();
        assert!(contains(&source, SECRET));
        let mut data = Zeroizing::new(source);
        let sent = sent_picture(&mut data, "image/jpeg").unwrap().unwrap();
        assert!(!contains(&data, SECRET));
        assert!(!contains(&data, b"Exif"));
        assert_eq!(sent.media_type, "image/jpeg");
        assert_eq!(sent.size, Some((10, 20)), "turned as its EXIF said");
        let again = decode(&data).unwrap();
        assert_eq!((again.width(), again.height()), (10, 20));
        let thumbnail = decode(&sent.thumbnail.unwrap()).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (10, 20));
    }

    #[test]
    fn a_png_leaves_without_its_text() {
        for (transparent, media_type) in [(true, "image/png"), (false, "image/jpeg")] {
            let mut data = Zeroizing::new(png_with_text(transparent));
            assert!(contains(&data, SECRET));
            let sent = sent_picture(&mut data, "image/png").unwrap().unwrap();
            assert!(!contains(&data, SECRET));
            assert_eq!(sent.media_type, media_type);
            assert_eq!(sent.size, Some((20, 10)));
            assert!(decode(&data).is_some());
        }
    }

    #[test]
    fn a_large_picture_is_scaled() {
        let mut data = Zeroizing::new(png(2000, 1000));
        let sent = sent_picture(&mut data, "image/png").unwrap().unwrap();
        assert_eq!(sent.size, Some((1600, 800)));
        let thumbnail = decode(&sent.thumbnail.unwrap()).unwrap();
        assert_eq!(thumbnail.width(), SENT_THUMBNAIL_MAX);
    }

    #[test]
    fn a_webp_loses_its_metadata_and_a_gif_goes_as_it_is() {
        let mut data = Zeroizing::new(crate::webp::fixture::still());
        let sent = sent_picture(&mut data, "image/webp").unwrap().unwrap();
        assert!(!contains(&data, b"GPS here"));
        assert_eq!(sent.media_type, "image/webp");
        assert_eq!(sent.size, Some((320, 240)));
        assert!(sent.thumbnail.is_none());

        let gif = b"GIF89a\x40\x01\xf0\x00rest";
        let mut data = Zeroizing::new(gif.to_vec());
        let sent = sent_picture(&mut data, "image/gif").unwrap().unwrap();
        assert_eq!(data.as_slice(), gif);
        assert_eq!(sent.size, Some((320, 240)));
    }

    #[test]
    fn what_cant_be_vouched_for_is_refused() {
        // A PNG over the limit: its header is all it takes.
        let mut header = [0; 13];
        header[..4].copy_from_slice(&10_000_u32.to_be_bytes());
        header[4..8].copy_from_slice(&10_000_u32.to_be_bytes());
        header[8] = 8;
        header[9] = 2;
        let too_large = [
            b"\x89PNG\r\n\x1a\n".as_slice(),
            &png_chunk(b"IHDR", &header),
            &png_chunk(b"tEXt", &[b"Comment\0", SECRET].concat()),
        ]
        .concat();
        let damaged = [&jpeg_with_exif()[..40], b"cut"].concat();
        let cut_webp = crate::webp::fixture::still()[..20].to_vec();
        for (why, source, claimed) in [
            ("too large", too_large, "image/png"),
            ("damaged", damaged, "image/jpeg"),
            ("cut off", cut_webp, "image/webp"),
            ("no JPEG", b"text".to_vec(), "image/jpeg"),
            ("no PNG", b"text".to_vec(), "image/png"),
            ("no WebP", b"text".to_vec(), "image/webp"),
            ("no GIF", b"text".to_vec(), "image/gif"),
        ] {
            let mut data = Zeroizing::new(source.clone());
            assert!(sent_picture(&mut data, claimed).is_err(), "{why}: not sent");
            assert_eq!(*data, source, "{why}");
        }
        // Another kind of file altogether.
        let mut data = Zeroizing::new(b"II*\0 a tiff".to_vec());
        assert!(matches!(sent_picture(&mut data, "image/tiff"), Ok(None)));
    }

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
        assert!(scaled_jpeg(b"GIF89a....", 48, 90).is_none());
    }
}
