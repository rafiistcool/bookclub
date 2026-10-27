//! The spine colour of a cover: a port of the legacy `spine_color.dominant_color`
//! (docs/phase1/spec/shelf.md §2.8 and §6 risk 17).
//!
//! 1. Drop an 8% inset (publisher borders and scan edges).
//! 2. Shrink to fit 48×48 (never up).
//! 3. Quantise to 6 colours by median cut (hand-written after Pillow's `MEDIANCUT`: the box with
//!    the most pixels is split first, along its widest luma-weighted channel, at the median;
//!    each pixel then counts for the nearest palette colour).
//! 4. Score each colour `(count / total) * (0.35 + HSV saturation)`: area decides, but a vivid
//!    colour beats an equally large grey one. The first highest score wins.
//! 5. Clamp the HLS lightness to [0.17, 0.86], so the spine's shading stays visible.

use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::{ImageDecoder, ImageReader, RgbImage};

/// Larger cover files are not sampled.
pub const MAX_COVER_BYTES: usize = 1_500_000;
/// Larger pictures are not sampled (checked before decoding).
pub const MAX_COVER_PIXELS: u64 = 4_000_000;
/// Open Library answers a missing cover with a 1×1 pixel; anything this thin is no cover.
pub const MIN_COVER_EDGE: u32 = 8;
const MIN_LIGHTNESS: f64 = 0.17;
const MAX_LIGHTNESS: f64 = 0.86;
const INSET: f64 = 0.08;
const THUMBNAIL: u32 = 48;
const PALETTE_SIZE: usize = 6;

/// The cover's most characteristic colour as `#rrggbb`, or `None` when `data` is no usable
/// picture: empty, over 1,500,000 bytes, undecodable, under 8 px on a side or over 4,000,000
/// pixels.
pub fn dominant_color(data: &[u8]) -> Option<String> {
    if data.is_empty() || data.len() > MAX_COVER_BYTES {
        return None;
    }
    let decoder = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let (width, height) = decoder.dimensions();
    if width.min(height) < MIN_COVER_EDGE || u64::from(width) * u64::from(height) > MAX_COVER_PIXELS
    {
        return None;
    }
    let image = image::DynamicImage::from_decoder(decoder).ok()?;
    dominant_color_of(&image.into_rgb8())
}

/// [`dominant_color`] of decoded pixels (a picture already converted by the pipeline). `None`
/// under 8 px on a side.
pub fn dominant_color_of(image: &RgbImage) -> Option<String> {
    let (width, height) = image.dimensions();
    if width.min(height) < MIN_COVER_EDGE {
        return None;
    }
    // int() truncation, as in Python.
    let inset_x = (f64::from(width) * INSET) as u32;
    let inset_y = (f64::from(height) * INSET) as u32;
    let cropped = imageops::crop_imm(
        image,
        inset_x,
        inset_y,
        width - 2 * inset_x,
        height - 2 * inset_y,
    )
    .to_image();
    let (thumb_width, thumb_height) = thumbnail_size(cropped.width(), cropped.height(), THUMBNAIL);
    let thumb = if (thumb_width, thumb_height) == cropped.dimensions() {
        cropped
    } else {
        // Pillow's reducing gap of 2: an integer box reduction first, then bicubic.
        let factor_x = (cropped.width() / thumb_width / 2).max(1);
        let factor_y = (cropped.height() / thumb_height / 2).max(1);
        let reduced = if factor_x > 1 || factor_y > 1 {
            box_reduce(&cropped, factor_x, factor_y)
        } else {
            cropped
        };
        imageops::resize(&reduced, thumb_width, thumb_height, FilterType::CatmullRom)
    };
    let pixels: Vec<[u8; 3]> = thumb.pixels().map(|pixel| pixel.0).collect();
    let total = pixels.len() as f64;

    let mut best: Option<(f64, [f64; 3])> = None;
    for (color, count) in quantize(&pixels, PALETTE_SIZE) {
        let rgb = color.map(|channel| f64::from(channel) / 255.0);
        let (_, saturation, _) = rgb_to_hsv(rgb);
        let score = (count as f64 / total) * (0.35 + saturation);
        if best.is_none_or(|(top, _)| score > top) {
            best = Some((score, rgb));
        }
    }
    let (_, rgb) = best?;
    let (hue, lightness, saturation) = rgb_to_hls(rgb);
    let lightness = lightness.clamp(MIN_LIGHTNESS, MAX_LIGHTNESS);
    let [red, green, blue] = hls_to_rgb(hue, lightness, saturation)
        .map(|channel| (channel * 255.0).round_ties_even().clamp(0.0, 255.0) as u8);
    Some(format!("#{red:02x}{green:02x}{blue:02x}"))
}

/// Pillow's `Image.reduce`: each output pixel is the mean of a `fx`×`fy` block (a partial
/// block at the right or bottom edge averages the pixels it has).
fn box_reduce(image: &RgbImage, fx: u32, fy: u32) -> RgbImage {
    let (width, height) = image.dimensions();
    let out_width = width.div_ceil(fx);
    let out_height = height.div_ceil(fy);
    RgbImage::from_fn(out_width, out_height, |ox, oy| {
        let mut sums = [0u32; 3];
        let mut count = 0u32;
        for y in oy * fy..((oy + 1) * fy).min(height) {
            for x in ox * fx..((ox + 1) * fx).min(width) {
                let pixel = image.get_pixel(x, y);
                for channel in 0..3 {
                    sums[channel] += u32::from(pixel[channel]);
                }
                count += 1;
            }
        }
        image::Rgb(sums.map(|sum| ((sum + count / 2) / count) as u8))
    })
}

/// Pillow's `thumbnail` size: fit within `max`×`max`, keep the aspect ratio, never upscale;
/// the rounding picks whichever of floor and ceil keeps the ratio closest.
fn thumbnail_size(width: u32, height: u32, max: u32) -> (u32, u32) {
    if max >= width && max >= height {
        return (width, height);
    }
    let aspect = f64::from(width) / f64::from(height);
    let target = f64::from(max);
    let closest = |number: f64, error: &dyn Fn(f64) -> f64| -> u32 {
        let (low, high) = (number.floor(), number.ceil());
        let pick = if error(high) < error(low) { high } else { low };
        (pick as u32).max(1)
    };
    if 1.0 >= aspect {
        // Taller than wide: the height is `max`.
        let w = closest(target * aspect, &|n| (aspect - n / target).abs());
        (w, max)
    } else {
        let h = closest(target / aspect, &|n| {
            if n == 0.0 {
                0.0
            } else {
                (aspect - target / n).abs()
            }
        });
        (max, h)
    }
}

/// A box of the median cut: distinct colours with their pixel counts.
struct ColorBox {
    colors: Vec<([u8; 3], u32)>,
    count: u64,
}

impl ColorBox {
    fn new(colors: Vec<([u8; 3], u32)>) -> Self {
        let count = colors.iter().map(|(_, n)| u64::from(*n)).sum();
        Self { colors, count }
    }

    fn range(&self, channel: usize) -> (u8, u8) {
        let mut low = u8::MAX;
        let mut high = u8::MIN;
        for (color, _) in &self.colors {
            low = low.min(color[channel]);
            high = high.max(color[channel]);
        }
        (low, high)
    }

    /// A box with one colour cannot be split.
    fn splittable(&self) -> bool {
        self.colors.len() > 1
    }

    /// Splits along the channel with the widest luma-weighted range at the pixel median; the
    /// lower half comes first.
    fn split(mut self) -> (ColorBox, ColorBox) {
        const WEIGHTS: [u32; 3] = [77, 150, 29];
        let mut axis = 0;
        let mut widest = 0;
        for (channel, weight) in WEIGHTS.iter().enumerate() {
            let (low, high) = self.range(channel);
            let width = u32::from(high - low) * weight;
            if width > widest {
                widest = width;
                axis = channel;
            }
        }
        self.colors.sort_unstable_by_key(|(color, _)| {
            (color[axis], color[(axis + 1) % 3], color[(axis + 2) % 3])
        });
        // Pillow's `splitlists`: colours go left until they hold more than half the pixels;
        // the colours that share the last one's value go left too.
        let mut seen = 0u64;
        let mut cut = self.colors[0].0[axis];
        for (color, count) in &self.colors {
            seen += u64::from(*count);
            cut = color[axis];
            if seen * 2 > self.count {
                break;
            }
        }
        let top = self.colors[self.colors.len() - 1].0[axis];
        if cut == top {
            // The median is the highest value: cut just below it, so both halves have pixels.
            cut = self
                .colors
                .iter()
                .map(|(color, _)| color[axis])
                .filter(|value| *value < top)
                .max()
                .unwrap_or(top);
        }
        let (low, high): (Vec<_>, Vec<_>) = self
            .colors
            .into_iter()
            .partition(|(color, _)| color[axis] <= cut);
        (ColorBox::new(low), ColorBox::new(high))
    }

    /// The pixel-weighted mean colour, rounded half up.
    fn mean(&self) -> [u8; 3] {
        let mut sums = [0u64; 3];
        for (color, count) in &self.colors {
            for channel in 0..3 {
                sums[channel] += u64::from(color[channel]) * u64::from(*count);
            }
        }
        sums.map(|sum| (sum as f64 / self.count as f64 + 0.5).floor().min(255.0) as u8)
    }
}

/// Median-cut quantisation to at most `size` colours: the palette in box order with the number
/// of pixels nearest to each colour (colours no pixel is nearest to are left out).
fn quantize(pixels: &[[u8; 3]], size: usize) -> Vec<([u8; 3], u64)> {
    let mut histogram: Vec<([u8; 3], u32)> = Vec::new();
    {
        let mut sorted = pixels.to_vec();
        sorted.sort_unstable();
        for color in sorted {
            match histogram.last_mut() {
                Some((last, count)) if *last == color => *count += 1,
                _ => histogram.push((color, 1)),
            }
        }
    }
    if histogram.is_empty() {
        return Vec::new();
    }
    // Leaves in tree order: a split box is replaced by its two halves, lower half first.
    let mut boxes = vec![ColorBox::new(histogram)];
    while boxes.len() < size {
        let candidate = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.splittable())
            .max_by(|(i, a), (j, b)| a.count.cmp(&b.count).then(j.cmp(i)))
            .map(|(index, _)| index);
        let Some(index) = candidate else {
            break;
        };
        let (low, high) = boxes.remove(index).split();
        boxes.insert(index, high);
        boxes.insert(index, low);
    }

    let palette: Vec<[u8; 3]> = boxes.iter().map(ColorBox::mean).collect();
    let mut counts = vec![0u64; palette.len()];
    for (own, color_box) in boxes.iter().enumerate() {
        for (color, count) in &color_box.colors {
            // The pixel's own box first; another colour wins only when strictly closer.
            let mut best = own;
            let mut best_distance = distance(&palette[own], color);
            for (index, entry) in palette.iter().enumerate() {
                let d = distance(entry, color);
                if d < best_distance {
                    best = index;
                    best_distance = d;
                }
            }
            counts[best] += u64::from(*count);
        }
    }
    palette
        .into_iter()
        .zip(counts)
        .filter(|(_, count)| *count > 0)
        .collect()
}

fn distance(a: &[u8; 3], b: &[u8; 3]) -> u32 {
    (0..3)
        .map(|channel| {
            let d = i32::from(a[channel]) - i32::from(b[channel]);
            (d * d) as u32
        })
        .sum()
}

/// Python's `colorsys.rgb_to_hsv`.
fn rgb_to_hsv([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max == min {
        return (0.0, 0.0, max);
    }
    let saturation = (max - min) / max;
    (hue(r, g, b, max, max - min), saturation, max)
}

/// The hue part of `colorsys`, in [0, 1).
fn hue(r: f64, g: f64, b: f64, max: f64, range: f64) -> f64 {
    let rc = (max - r) / range;
    let gc = (max - g) / range;
    let bc = (max - b) / range;
    let h = if r == max {
        bc - gc
    } else if g == max {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    (h / 6.0).rem_euclid(1.0)
}

/// Python's `colorsys.rgb_to_hls` (3.12+, gh-106498).
fn rgb_to_hls([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sum = max + min;
    let range = max - min;
    let lightness = sum / 2.0;
    if min == max {
        return (0.0, lightness, 0.0);
    }
    let saturation = if lightness <= 0.5 {
        range / sum
    } else {
        range / (2.0 - max - min)
    };
    (hue(r, g, b, max, range), lightness, saturation)
}

/// Python's `colorsys.hls_to_rgb`.
fn hls_to_rgb(hue: f64, lightness: f64, saturation: f64) -> [f64; 3] {
    if saturation == 0.0 {
        return [lightness; 3];
    }
    let m2 = if lightness <= 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let m1 = 2.0 * lightness - m2;
    [
        channel(m1, m2, hue + 1.0 / 3.0),
        channel(m1, m2, hue),
        channel(m1, m2, hue - 1.0 / 3.0),
    ]
}

fn channel(m1: f64, m2: f64, hue: f64) -> f64 {
    let hue = hue.rem_euclid(1.0);
    if hue < 1.0 / 6.0 {
        m1 + (m2 - m1) * hue * 6.0
    } else if hue < 0.5 {
        m2
    } else if hue < 2.0 / 3.0 {
        m1 + (m2 - m1) * (2.0 / 3.0 - hue) * 6.0
    } else {
        m1
    }
}

#[cfg(test)]
mod tests {
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder, Rgb};

    use super::*;

    /// The legacy test helper: a JPEG of `color`, optionally inside a border.
    fn jpeg(color: [u8; 3], size: (u32, u32), border: Option<[u8; 3]>) -> Vec<u8> {
        let mut image = RgbImage::from_pixel(size.0, size.1, Rgb(border.unwrap_or(color)));
        if border.is_some() {
            for x in 12..size.0.saturating_sub(12) {
                for y in 18..size.1.saturating_sub(18) {
                    image.put_pixel(x, y, Rgb(color));
                }
            }
        }
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 92)
            .write_image(image.as_raw(), size.0, size.1, ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    fn rgb(value: &str) -> [i32; 3] {
        [1, 3, 5].map(|i| i32::from_str_radix(&value[i..i + 2], 16).unwrap())
    }

    /// parity: test_spine_color.py::test_dominant_color_reads_the_cover_and_ignores_its_border
    #[test]
    fn reads_the_cover_and_ignores_its_border() {
        let color =
            dominant_color(&jpeg([30, 90, 160], (120, 180), Some([255, 255, 255]))).unwrap();
        let [red, green, blue] = rgb(&color);
        assert!(blue > green && green > red, "{color}");
        assert!((blue - 160).abs() < 20, "{color}");
    }

    /// parity: test_spine_color.py::test_dominant_color_prefers_a_vivid_colour_over_a_slightly_larger_pale_one
    #[test]
    fn prefers_a_vivid_colour_over_a_slightly_larger_pale_one() {
        // A light grey panel covers a little more of the cover than the deep green one.
        let mut image = RgbImage::from_pixel(120, 180, Rgb([205, 205, 200]));
        for x in 0..120 {
            for y in 0..80 {
                image.put_pixel(x, y, Rgb([20, 95, 40]));
            }
        }
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(image.as_raw(), 120, 180, ExtendedColorType::Rgb8)
            .unwrap();
        let color = dominant_color(&png).unwrap();
        let [red, green, blue] = rgb(&color);
        assert!(green > red + 40 && green > blue + 30, "{color}");
    }

    /// parity: test_spine_color.py::test_dominant_color_keeps_extremes_printable_and_rejects_junk
    #[test]
    fn keeps_extremes_printable_and_rejects_junk() {
        // Pure black and white still leave room for the spine's shading.
        assert!(rgb(&dominant_color(&jpeg([0, 0, 0], (120, 180), None)).unwrap())[0] > 30);
        assert!(rgb(&dominant_color(&jpeg([255, 255, 255], (120, 180), None)).unwrap())[0] < 235);
        assert_eq!(dominant_color(b""), None);
        assert_eq!(dominant_color(b"not an image at all, just some text"), None);
        // Open Library's 1×1 "no cover" pixel is not a colour.
        assert_eq!(dominant_color(&jpeg([200, 20, 20], (1, 1), None)), None);
    }

    #[test]
    fn oversized_files_and_pictures_are_not_sampled() {
        let mut big = jpeg([200, 20, 20], (120, 180), None);
        big.resize(MAX_COVER_BYTES + 1, 0);
        assert_eq!(dominant_color(&big), None);
        // 2001×2000 pixels is over the 4,000,000 limit; only the header is read.
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&vec![0; 2001 * 2000], 2001, 2000, ExtendedColorType::L8)
            .unwrap();
        assert_eq!(dominant_color(&png), None);
    }

    #[test]
    fn thumbnails_round_like_pillow() {
        // The legacy cover after its 8% inset: 102×152 → 32×48.
        assert_eq!(thumbnail_size(102, 152, 48), (32, 48));
        assert_eq!(thumbnail_size(152, 102, 48), (48, 32));
        assert_eq!(thumbnail_size(40, 30, 48), (40, 30));
        assert_eq!(thumbnail_size(480, 4, 48), (48, 1));
    }

    #[test]
    fn colorsys_round_trips() {
        let (h, l, s) = rgb_to_hls([30.0 / 255.0, 90.0 / 255.0, 160.0 / 255.0]);
        let back = hls_to_rgb(h, l, s).map(|c| (c * 255.0).round_ties_even() as i32);
        assert_eq!(back, [30, 90, 160]);
        let (_, saturation, _) = rgb_to_hsv([0.8, 0.8, 0.8]);
        assert_eq!(saturation, 0.0);
    }

    #[test]
    fn median_cut_keeps_distinct_colours_apart() {
        let mut pixels = vec![[200, 20, 20]; 30];
        pixels.extend(vec![[20, 20, 200]; 10]);
        let palette = quantize(&pixels, 6);
        assert_eq!(palette, vec![([20, 20, 200], 10), ([200, 20, 20], 30)]);
    }
}
