//! The image pipeline (docs/phase1/conventions.md §8, docs/phase1/spec/auth.md §2.5): every
//! picture we store, an upload or a fetched Open Library cover, is decoded and re-encoded here.
//!
//! 1. Sniff the magic bytes (JPEG, PNG, GIF, WebP); anything else is refused.
//! 2. Read the size from the header and refuse more than 25,000,000 pixels before decoding.
//! 3. Decode (the first frame of a GIF or an animated WebP) under the decoder's memory limit.
//! 4. Apply the EXIF orientation, then composite any alpha onto white.
//! 5. Avatars: centre-crop to a square and resize to 256×256 (Lanczos3). Covers: `l` at most
//!    800 px wide, `s` at most 240 px wide (never upscaled).
//! 6. Encode lossy WebP at quality 82.
//!
//! Metadata is dropped by construction: only pixels reach the encoder, so EXIF (GPS included),
//! ICC and XMP never come out. The functions are synchronous and CPU-bound; callers run them on
//! the blocking pool (`MediaService::process`).

use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits, RgbImage};
use sha2::{Digest, Sha256};

use super::color;
use crate::error::msg;

/// Largest accepted upload (50 MiB).
pub const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
/// Largest accepted picture, counted before decoding (about 5000×5000).
pub const MAX_SOURCE_PIXELS: u64 = 25_000_000;
/// Fewer bytes than this cannot be a picture we accept.
const MIN_BYTES: usize = 24;
/// Edge of the square avatar.
pub const AVATAR_SIZE: u32 = 256;
/// Widest large cover (`l`).
pub const COVER_LARGE_WIDTH: u32 = 800;
/// Widest cover thumbnail (`s`).
pub const COVER_SMALL_WIDTH: u32 = 240;
/// WebP cannot store an edge longer than this.
const WEBP_MAX_EDGE: u32 = 16_383;
/// Lossy WebP quality.
pub const WEBP_QUALITY: f32 = 82.0;
/// What the decoder may allocate for one picture: 25 Mpx of 16-bit RGBA fit.
const DECODE_MAX_ALLOC: u64 = 512 * 1024 * 1024;

/// Why a picture was refused. Each maps to its legacy detail string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PictureError {
    /// Over [`MAX_UPLOAD_BYTES`].
    TooLarge,
    /// Not JPEG, PNG, GIF or WebP by its magic bytes, or too short.
    BadType,
    /// The magic bytes fit, but the picture does not decode.
    NotAPicture,
    /// Over [`MAX_SOURCE_PIXELS`], or the decoder's memory limit.
    TooManyPixels,
    /// The WebP encoder failed (a server error, not the picture's fault).
    Encode(String),
}

impl PictureError {
    /// The 400 detail of an upload refused for this reason; `None` for [`PictureError::Encode`].
    pub fn detail(&self) -> Option<&'static str> {
        match self {
            PictureError::TooLarge => Some(msg::PICTURE_TOO_LARGE),
            PictureError::BadType => Some(msg::PICTURE_BAD_TYPE),
            PictureError::NotAPicture => Some(msg::PICTURE_NOT_AN_IMAGE),
            PictureError::TooManyPixels => Some(msg::PICTURE_TOO_MANY_PIXELS),
            PictureError::Encode(_) => None,
        }
    }
}

/// What a picture becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureKind {
    /// `a`: 256×256.
    Avatar,
    /// `l` and `s`.
    Cover,
}

impl PictureKind {
    /// `media.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            PictureKind::Avatar => "avatar",
            PictureKind::Cover => "cover",
        }
    }

    /// The variants a stored picture of this kind has, largest first.
    pub fn variants(self) -> &'static [&'static str] {
        match self {
            PictureKind::Avatar => &["a"],
            PictureKind::Cover => &["l", "s"],
        }
    }
}

/// A converted picture, ready for the store and `media::shared::register`.
#[derive(Debug, Clone)]
pub struct Processed {
    /// `(variant, WebP bytes)`, largest first.
    pub variants: Vec<(&'static str, Vec<u8>)>,
    /// Size of the largest variant.
    pub width: u32,
    pub height: u32,
    /// SHA-256 of the bytes as received.
    pub sha256: [u8; 32],
    /// Covers: the spine colour (`""` when none is usable). Avatars: `None`.
    pub dominant_color: Option<String>,
}

impl Processed {
    /// Bytes of every variant together (`media.byte_size`).
    pub fn byte_size(&self) -> i64 {
        self.variants
            .iter()
            .map(|(_, bytes)| bytes.len() as i64)
            .sum()
    }
}

/// SHA-256 of `data` (`media.sha256`).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// The format the magic bytes announce, when it is one we accept.
pub fn sniff(data: &[u8]) -> Option<ImageFormat> {
    if data.starts_with(b"\xff\xd8\xff") {
        Some(ImageFormat::Jpeg)
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

/// Steps 1 to 4: the upright RGB pixels of `data`, alpha composited onto white.
pub fn decode(data: &[u8]) -> Result<RgbImage, PictureError> {
    if data.len() > MAX_UPLOAD_BYTES {
        return Err(PictureError::TooLarge);
    }
    if data.len() < MIN_BYTES {
        return Err(PictureError::BadType);
    }
    let format = sniff(data).ok_or(PictureError::BadType)?;
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(DECODE_MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(PictureError::NotAPicture);
    }
    if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err(PictureError::TooManyPixels);
    }
    // A damaged EXIF block only loses the rotation, never the picture.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    image.apply_orientation(orientation);
    Ok(onto_white(image))
}

fn decode_error(error: ImageError) -> PictureError {
    match error {
        ImageError::Limits(_) => PictureError::TooManyPixels,
        _ => PictureError::NotAPicture,
    }
}

/// RGB pixels; transparent ones are composited onto an opaque white background.
fn onto_white(image: DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.into_rgb8();
    }
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut rgb = RgbImage::new(width, height);
    for (out, pixel) in rgb.pixels_mut().zip(rgba.pixels()) {
        let alpha = u32::from(pixel[3]);
        for channel in 0..3 {
            let value = u32::from(pixel[channel]) * alpha + 255 * (255 - alpha);
            out[channel] = ((value + 127) / 255) as u8;
        }
    }
    rgb
}

/// The centred square of `image`, resized to [`AVATAR_SIZE`] (Lanczos3).
pub fn avatar(image: &RgbImage) -> RgbImage {
    let (width, height) = image.dimensions();
    let side = width.min(height);
    let left = (width - side) / 2;
    let top = (height - side) / 2;
    let square = imageops::crop_imm(image, left, top, side, side).to_image();
    imageops::resize(&square, AVATAR_SIZE, AVATAR_SIZE, FilterType::Lanczos3)
}

/// `image` scaled down (never up) to at most `max_width` wide and at most the WebP edge limit
/// tall, keeping its aspect ratio.
pub fn fit(image: &RgbImage, max_width: u32) -> RgbImage {
    let (width, height) = image.dimensions();
    let scale = (f64::from(max_width) / f64::from(width))
        .min(f64::from(WEBP_MAX_EDGE) / f64::from(height))
        .min(1.0);
    if scale >= 1.0 {
        return image.clone();
    }
    let new_width = ((f64::from(width) * scale).round() as u32).clamp(1, max_width);
    let new_height = ((f64::from(height) * scale).round() as u32).clamp(1, WEBP_MAX_EDGE);
    imageops::resize(image, new_width, new_height, FilterType::Lanczos3)
}

/// Lossy WebP of `image` at [`WEBP_QUALITY`]; carries no metadata.
pub fn encode_webp(image: &RgbImage) -> Result<Vec<u8>, PictureError> {
    let (width, height) = image.dimensions();
    webp::Encoder::from_rgb(image.as_raw(), width, height)
        .encode_simple(false, WEBP_QUALITY)
        .map(|memory| memory.to_vec())
        .map_err(|error| PictureError::Encode(format!("{error:?}")))
}

/// The whole pipeline for one received picture.
pub fn process(kind: PictureKind, data: &[u8]) -> Result<Processed, PictureError> {
    let hash = sha256(data);
    let image = decode(data)?;
    match kind {
        PictureKind::Avatar => {
            let square = avatar(&image);
            Ok(Processed {
                variants: vec![("a", encode_webp(&square)?)],
                width: square.width(),
                height: square.height(),
                sha256: hash,
                dominant_color: None,
            })
        }
        PictureKind::Cover => {
            let large = fit(&image, COVER_LARGE_WIDTH);
            let small = fit(&large, COVER_SMALL_WIDTH);
            let dominant = color::dominant_color_of(&large).unwrap_or_default();
            Ok(Processed {
                variants: vec![("l", encode_webp(&large)?), ("s", encode_webp(&small)?)],
                width: large.width(),
                height: large.height(),
                sha256: hash,
                dominant_color: Some(dominant),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use image::codecs::png::PngEncoder;
    use image::{ImageEncoder, Rgb, RgbaImage};

    use super::*;

    fn png(image: &RgbImage) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    #[test]
    fn sniffs_the_four_formats_only() {
        assert_eq!(sniff(b"\xff\xd8\xff\xe0rest"), Some(ImageFormat::Jpeg));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some(ImageFormat::Png));
        assert_eq!(sniff(b"GIF87a..."), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"GIF89a..."), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::WebP));
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(
            sniff(b"<svg xmlns='http://www.w3.org/2000/svg'></svg>"),
            None
        );
        assert_eq!(sniff(b"BM\0\0"), None);
    }

    #[test]
    fn short_and_unknown_data_is_the_wrong_type() {
        assert_eq!(decode(b"\xff\xd8\xff"), Err(PictureError::BadType));
        assert_eq!(decode(&[b'x'; 100]), Err(PictureError::BadType));
    }

    #[test]
    fn a_damaged_picture_is_not_a_picture() {
        let mut data = png(&RgbImage::from_pixel(10, 10, Rgb([1, 2, 3])));
        data.truncate(40);
        assert_eq!(decode(&data), Err(PictureError::NotAPicture));
    }

    #[test]
    fn transparency_becomes_white() {
        let mut rgba = RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        rgba.put_pixel(0, 0, image::Rgba([200, 0, 0, 255]));
        rgba.put_pixel(1, 0, image::Rgba([0, 0, 0, 128]));
        let rgb = onto_white(DynamicImage::ImageRgba8(rgba));
        assert_eq!(rgb.get_pixel(0, 0), &Rgb([200, 0, 0]));
        assert_eq!(rgb.get_pixel(3, 3), &Rgb([255, 255, 255]));
        assert_eq!(rgb.get_pixel(1, 0), &Rgb([127, 127, 127]));
    }

    #[test]
    fn avatars_are_centre_cropped_squares() {
        // A wide picture: red, green, red thirds. The square is the green middle.
        let mut image = RgbImage::from_pixel(300, 100, Rgb([255, 0, 0]));
        for x in 100..200 {
            for y in 0..100 {
                image.put_pixel(x, y, Rgb([0, 255, 0]));
            }
        }
        let square = avatar(&image);
        assert_eq!(square.dimensions(), (256, 256));
        let centre = square.get_pixel(128, 128);
        assert!(centre[1] > 200 && centre[0] < 50, "{centre:?}");
    }

    #[test]
    fn covers_are_scaled_down_never_up() {
        let big = RgbImage::new(1600, 2400);
        assert_eq!(fit(&big, COVER_LARGE_WIDTH).dimensions(), (800, 1200));
        assert_eq!(fit(&big, COVER_SMALL_WIDTH).dimensions(), (240, 360));
        let small = RgbImage::new(100, 150);
        assert_eq!(fit(&small, COVER_LARGE_WIDTH).dimensions(), (100, 150));
        // A strip taller than WebP allows is scaled to fit.
        let strip = RgbImage::new(10, 20_000);
        assert_eq!(fit(&strip, COVER_LARGE_WIDTH).dimensions(), (8, 16_383));
    }

    #[test]
    fn a_cover_has_two_webp_variants_and_a_colour() {
        let data = png(&RgbImage::from_pixel(400, 600, Rgb([30, 90, 160])));
        let processed = process(PictureKind::Cover, &data).unwrap();
        let names: Vec<_> = processed.variants.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["l", "s"]);
        assert_eq!((processed.width, processed.height), (400, 600));
        for (_, bytes) in &processed.variants {
            assert_eq!(sniff(bytes), Some(ImageFormat::WebP));
        }
        let small = image::load_from_memory(&processed.variants[1].1).unwrap();
        assert_eq!((small.width(), small.height()), (240, 360));
        assert_eq!(processed.sha256, sha256(&data));
        assert!(
            processed
                .dominant_color
                .as_deref()
                .unwrap()
                .starts_with('#')
        );
        assert_eq!(
            processed.byte_size(),
            (processed.variants[0].1.len() + processed.variants[1].1.len()) as i64
        );
    }
}
