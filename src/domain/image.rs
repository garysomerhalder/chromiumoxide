//! Image comparison for `toHaveScreenshot`: a port of the `pixelmatch`
//! comparator Playwright uses (YIQ colour distance, anti-aliasing detection)
//! and of Playwright's `compareImages` (size padding, diff thresholds, message).

use crate::domain::error::{Error, Result};

/// An RGBA image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major RGBA.
    pub rgba: Vec<u8>,
}

/// Playwright's comparison options.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CompareOptions {
    /// Per-pixel colour distance tolerance, 0..=1 (default 0.2).
    pub threshold: Option<f64>,
    /// How many pixels may differ.
    pub max_diff_pixels: Option<u64>,
    /// Which share of the pixels may differ, 0..=1.
    pub max_diff_pixel_ratio: Option<f64>,
}

/// Why two images do not match, with a diff image.
#[derive(Debug, Clone, PartialEq)]
pub struct Mismatch {
    pub message: String,
    pub diff: Image,
}

/// Checks option ranges like Playwright's `SnapshotHelper`.
pub fn validate(o: &CompareOptions) -> Result<()> {
    if o.max_diff_pixel_ratio
        .is_some_and(|r| !(0.0..=1.0).contains(&r))
    {
        return Err(Error::InvalidArgument(
            "`maxDiffPixelRatio` option value must be between 0 and 1".into(),
        ));
    }
    if o.threshold.is_some_and(|t| !(0.0..=1.0).contains(&t)) {
        return Err(Error::InvalidArgument(
            "`threshold` option value must be between 0 and 1".into(),
        ));
    }
    Ok(())
}

const AA_COLOR: [u8; 3] = [255, 255, 0];
const DIFF_COLOR: [u8; 3] = [255, 0, 0];
const ALPHA: f64 = 0.1;

/// `pixelmatch(img1, img2, output, width, height, { threshold })`: the number
/// of different pixels, drawing the diff into `output` when given.
pub fn pixelmatch(
    img1: &[u8],
    img2: &[u8],
    mut output: Option<&mut [u8]>,
    width: u32,
    height: u32,
    threshold: f64,
) -> u64 {
    let (w, h) = (width as usize, height as usize);
    if img1 == img2 {
        if let Some(out) = output.as_deref_mut() {
            for i in 0..w * h {
                draw_gray_pixel(img1, i * 4, out);
            }
        }
        return 0;
    }
    let max_delta = 35215.0 * threshold * threshold;
    let mut diff = 0;
    for y in 0..h {
        for x in 0..w {
            let pos = (y * w + x) * 4;
            let delta = color_delta(img1, img2, pos, pos, false);
            if delta.abs() > max_delta {
                if antialiased(img1, x, y, w, h, img2) || antialiased(img2, x, y, w, h, img1) {
                    if let Some(out) = output.as_deref_mut() {
                        draw_pixel(out, pos, AA_COLOR);
                    }
                } else {
                    if let Some(out) = output.as_deref_mut() {
                        draw_pixel(out, pos, DIFF_COLOR);
                    }
                    diff += 1;
                }
            } else if let Some(out) = output.as_deref_mut() {
                draw_gray_pixel(img1, pos, out);
            }
        }
    }
    diff
}

fn antialiased(img: &[u8], x1: usize, y1: usize, w: usize, h: usize, img2: &[u8]) -> bool {
    let x0 = x1.saturating_sub(1);
    let y0 = y1.saturating_sub(1);
    let x2 = (x1 + 1).min(w - 1);
    let y2 = (y1 + 1).min(h - 1);
    let pos = (y1 * w + x1) * 4;
    let mut zeroes = u32::from(x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2);
    let (mut min, mut max) = (0.0f64, 0.0f64);
    let (mut min_at, mut max_at) = ((0, 0), (0, 0));
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            let delta = color_delta(img, img, pos, (y * w + x) * 4, true);
            if delta == 0.0 {
                zeroes += 1;
                if zeroes > 2 {
                    return false;
                }
            } else if delta < min {
                min = delta;
                min_at = (x, y);
            } else if delta > max {
                max = delta;
                max_at = (x, y);
            }
        }
    }
    if min == 0.0 || max == 0.0 {
        return false;
    }
    (has_many_siblings(img, min_at, w, h) && has_many_siblings(img2, min_at, w, h))
        || (has_many_siblings(img, max_at, w, h) && has_many_siblings(img2, max_at, w, h))
}

fn has_many_siblings(img: &[u8], (x1, y1): (usize, usize), w: usize, h: usize) -> bool {
    let x0 = x1.saturating_sub(1);
    let y0 = y1.saturating_sub(1);
    let x2 = (x1 + 1).min(w - 1);
    let y2 = (y1 + 1).min(h - 1);
    let pos = (y1 * w + x1) * 4;
    let mut zeroes = u32::from(x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2);
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            let pos2 = (y * w + x) * 4;
            if img[pos..pos + 4] == img[pos2..pos2 + 4] {
                zeroes += 1;
            }
            if zeroes > 2 {
                return true;
            }
        }
    }
    false
}

fn color_delta(img1: &[u8], img2: &[u8], k: usize, m: usize, y_only: bool) -> f64 {
    let px = |img: &[u8], i: usize| {
        let (r, g, b, a) = (
            img[i] as f64,
            img[i + 1] as f64,
            img[i + 2] as f64,
            img[i + 3] as f64,
        );
        if a < 255.0 {
            let a = a / 255.0;
            (blend(r, a), blend(g, a), blend(b, a))
        } else {
            (r, g, b)
        }
    };
    if img1[k..k + 4] == img2[m..m + 4] {
        return 0.0;
    }
    let (r1, g1, b1) = px(img1, k);
    let (r2, g2, b2) = px(img2, m);
    let y1 = rgb2y(r1, g1, b1);
    let y2 = rgb2y(r2, g2, b2);
    let y = y1 - y2;
    if y_only {
        return y;
    }
    let i = rgb2i(r1, g1, b1) - rgb2i(r2, g2, b2);
    let q = rgb2q(r1, g1, b1) - rgb2q(r2, g2, b2);
    let delta = 0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q;
    if y1 > y2 { -delta } else { delta }
}

fn rgb2y(r: f64, g: f64, b: f64) -> f64 {
    r * 0.29889531 + g * 0.58662247 + b * 0.11448223
}
fn rgb2i(r: f64, g: f64, b: f64) -> f64 {
    r * 0.59597799 - g * 0.2741761 - b * 0.32180189
}
fn rgb2q(r: f64, g: f64, b: f64) -> f64 {
    r * 0.21147017 - g * 0.52261711 + b * 0.31114694
}
fn blend(c: f64, a: f64) -> f64 {
    255.0 + (c - 255.0) * a
}

fn draw_pixel(out: &mut [u8], pos: usize, [r, g, b]: [u8; 3]) {
    out[pos..pos + 4].copy_from_slice(&[r, g, b, 255]);
}

fn draw_gray_pixel(img: &[u8], i: usize, out: &mut [u8]) {
    let y = rgb2y(img[i] as f64, img[i + 1] as f64, img[i + 2] as f64);
    // Like a Uint8Array store: truncated.
    let v = blend(y, ALPHA * img[i + 3] as f64 / 255.0) as u8;
    draw_pixel(out, i, [v, v, v]);
}

/// Playwright's `padImageToSize`: transparent pixels to the right and below.
fn pad(image: &Image, width: u32, height: u32) -> Vec<u8> {
    if image.width == width && image.height == height {
        return image.rgba.clone();
    }
    let mut out = vec![0; (width * height * 4) as usize];
    for y in 0..image.height {
        let from = (y * image.width * 4) as usize;
        let to = (y * width * 4) as usize;
        let len = (image.width * 4) as usize;
        out[to..to + len].copy_from_slice(&image.rgba[from..from + len]);
    }
    out
}

/// Playwright's `compareImages`: `None` when `actual` matches `expected`.
pub fn compare_images(actual: &Image, expected: &Image, o: &CompareOptions) -> Option<Mismatch> {
    let width = expected.width.max(actual.width);
    let height = expected.height.max(actual.height);
    let mut sizes = String::new();
    if expected.width != actual.width || expected.height != actual.height {
        sizes = format!(
            "Expected an image {}px by {}px, received {}px by {}px. ",
            expected.width, expected.height, actual.width, actual.height
        );
    }
    let e = pad(expected, width, height);
    let a = pad(actual, width, height);
    let mut diff = vec![0; (width * height * 4) as usize];
    let count = pixelmatch(
        &e,
        &a,
        Some(&mut diff),
        width,
        height,
        o.threshold.unwrap_or(0.2),
    );
    let total = expected.width as f64 * expected.height as f64;
    let by_ratio = o.max_diff_pixel_ratio.map(|r| total * r);
    let allowed = match (o.max_diff_pixels.map(|p| p as f64), by_ratio) {
        (Some(p), Some(r)) => p.min(r),
        (p, r) => p.or(r).unwrap_or(0.0),
    };
    let ratio = (count as f64 / total * 100.0).ceil() / 100.0;
    let pixels = if count as f64 > allowed {
        format!("{count} pixels (ratio {ratio:.2} of all image pixels) are different.")
    } else {
        String::new()
    };
    if pixels.is_empty() && sizes.is_empty() {
        return None;
    }
    Some(Mismatch {
        message: sizes + &pixels,
        diff: Image {
            width,
            height,
            rgba: diff,
        },
    })
}

/// Decodes a PNG into RGBA.
pub fn decode_png(bytes: &[u8]) -> Result<Image> {
    let bad = || Error::InvalidArgument("Could not decode expected image as PNG.".into());
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|_| bad())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(bad)?];
    let info = reader.next_frame(&mut buf).map_err(|_| bad())?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(bad()),
    };
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Encodes RGBA as PNG.
pub fn encode_png(image: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("in-memory PNG header");
        writer
            .write_image_data(&image.rgba)
            .expect("RGBA data matches the size");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Image {
        Image {
            width: w,
            height: h,
            rgba: (0..w * h).flat_map(|_| c).collect(),
        }
    }

    fn set(img: &mut Image, x: u32, y: u32, c: [u8; 4]) {
        let i = ((y * img.width + x) * 4) as usize;
        img.rgba[i..i + 4].copy_from_slice(&c);
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const GRAY: [u8; 4] = [128, 128, 128, 255];

    #[test]
    fn identical_images_match() {
        let a = solid(4, 4, RED);
        assert_eq!(pixelmatch(&a.rgba, &a.rgba, None, 4, 4, 0.2), 0);
        assert_eq!(compare_images(&a, &a, &CompareOptions::default()), None);
    }

    #[test]
    fn counts_every_different_pixel() {
        let a = solid(4, 4, RED);
        let b = solid(4, 4, BLUE);
        let mut out = vec![0; 64];
        assert_eq!(pixelmatch(&a.rgba, &b.rgba, Some(&mut out), 4, 4, 0.2), 16);
        assert_eq!(&out[..4], &[255, 0, 0, 255], "diff colour is red");
        let m = compare_images(&b, &a, &CompareOptions::default()).unwrap();
        assert_eq!(
            m.message,
            "16 pixels (ratio 1.00 of all image pixels) are different."
        );
        assert_eq!((m.diff.width, m.diff.height), (4, 4));
    }

    #[test]
    fn one_isolated_pixel_counts_and_ratio_rounds_up() {
        let a = solid(5, 5, WHITE);
        let mut b = a.clone();
        set(&mut b, 2, 2, BLACK);
        assert_eq!(pixelmatch(&a.rgba, &b.rgba, None, 5, 5, 0.2), 1);
        let m = compare_images(&b, &a, &CompareOptions::default()).unwrap();
        assert_eq!(
            m.message,
            "1 pixels (ratio 0.04 of all image pixels) are different."
        );
        let mut big = solid(30, 30, WHITE);
        set(&mut big, 1, 1, BLACK);
        let m = compare_images(&big, &solid(30, 30, WHITE), &CompareOptions::default()).unwrap();
        assert_eq!(
            m.message, "1 pixels (ratio 0.01 of all image pixels) are different.",
            "1/900 rounds up to 0.01"
        );
    }

    #[test]
    fn unchanged_pixels_are_drawn_faded_gray() {
        let a = solid(5, 5, WHITE);
        let mut b = a.clone();
        set(&mut b, 2, 2, BLACK);
        let mut out = vec![0; 100];
        pixelmatch(&a.rgba, &b.rgba, Some(&mut out), 5, 5, 0.2);
        // blend(rgb2y(255,255,255) = 255, 0.1) = 255
        assert_eq!(&out[..4], &[255, 255, 255, 255]);
        let r = solid(1, 1, RED);
        let mut out = vec![0; 4];
        pixelmatch(&r.rgba, &r.rgba, Some(&mut out), 1, 1, 0.2);
        // 255 + (76.218 - 255) * 0.1 = 237.12 -> 237
        assert_eq!(out, vec![237, 237, 237, 255]);
    }

    #[test]
    fn threshold_tolerates_small_colour_changes() {
        let a = solid(3, 3, [255, 0, 0, 255]);
        let b = solid(3, 3, [250, 0, 0, 255]);
        assert_eq!(pixelmatch(&a.rgba, &b.rgba, None, 3, 3, 0.2), 0);
        assert_eq!(pixelmatch(&a.rgba, &b.rgba, None, 3, 3, 0.0), 9);
    }

    #[test]
    fn anti_aliased_pixels_are_ignored() {
        // A gray edge pixel between a black and a white column.
        let mut a = solid(3, 5, GRAY);
        for y in 0..5 {
            set(&mut a, 0, y, BLACK);
            set(&mut a, 2, y, WHITE);
        }
        let mut b = a.clone();
        set(&mut b, 1, 2, BLACK);
        let mut out = vec![0; 60];
        assert_eq!(pixelmatch(&a.rgba, &b.rgba, Some(&mut out), 3, 5, 0.2), 0);
        let i = (2 * 3 + 1) * 4;
        assert_eq!(&out[i..i + 4], &[255, 255, 0, 255], "drawn in aaColor");
    }

    #[test]
    fn transparent_pixels_blend_with_white() {
        let a = solid(2, 2, [0, 0, 0, 0]);
        let b = solid(2, 2, WHITE);
        assert_eq!(pixelmatch(&a.rgba, &b.rgba, None, 2, 2, 0.0), 0);
    }

    #[test]
    fn allowed_differences_follow_max_diff_options() {
        let a = solid(10, 10, WHITE);
        let mut b = a.clone();
        for x in 0..10 {
            set(&mut b, x, 0, BLACK);
        }
        let opts = |px, ratio| CompareOptions {
            max_diff_pixels: px,
            max_diff_pixel_ratio: ratio,
            ..Default::default()
        };
        assert!(compare_images(&b, &a, &opts(Some(10), None)).is_none());
        assert!(compare_images(&b, &a, &opts(Some(9), None)).is_some());
        assert!(compare_images(&b, &a, &opts(None, Some(0.1))).is_none());
        assert!(compare_images(&b, &a, &opts(None, Some(0.09))).is_some());
        // Both given: the stricter wins.
        assert!(compare_images(&b, &a, &opts(Some(100), Some(0.05))).is_some());
    }

    #[test]
    fn size_mismatch_pads_with_transparency_and_reports_sizes() {
        let expected = solid(2, 2, WHITE);
        let actual = solid(3, 2, WHITE);
        let m = compare_images(&actual, &expected, &CompareOptions::default()).unwrap();
        assert_eq!(
            m.message,
            "Expected an image 2px by 2px, received 3px by 2px. "
        );
        assert_eq!((m.diff.width, m.diff.height), (3, 2));
        let actual = solid(3, 2, BLACK);
        let m = compare_images(&actual, &expected, &CompareOptions::default()).unwrap();
        assert!(
            m.message.starts_with(
                "Expected an image 2px by 2px, received 3px by 2px. 6 pixels (ratio 1.50"
            ),
            "{}",
            m.message
        );
    }

    #[test]
    fn options_are_validated() {
        let bad = CompareOptions {
            max_diff_pixel_ratio: Some(1.5),
            ..Default::default()
        };
        assert_eq!(
            validate(&bad).unwrap_err().to_string(),
            Error::InvalidArgument(
                "`maxDiffPixelRatio` option value must be between 0 and 1".into()
            )
            .to_string()
        );
        assert!(validate(&CompareOptions::default()).is_ok());
    }

    #[test]
    fn matches_playwright_on_random_images() {
        // Same generator and images as a run of Playwright 1.63's pixelmatch,
        // which returned 115 different pixels and this output checksum.
        let mut seed: u64 = 7;
        let mut rnd = || {
            seed = (seed * 48271) % 2_147_483_647;
            seed
        };
        let (w, h) = (17u32, 13u32);
        let n = (w * h * 4) as usize;
        let (mut x, mut y) = (vec![0u8; n], vec![0u8; n]);
        for i in 0..n {
            x[i] = (rnd() % 256) as u8;
            y[i] = if i % 4 == 3 {
                255
            } else if rnd() % 3 == 0 {
                (rnd() % 256) as u8
            } else {
                x[i]
            };
        }
        for i in (3..n).step_by(4) {
            x[i] = if rnd() % 2 == 1 {
                255
            } else {
                (rnd() % 256) as u8
            };
        }
        let mut out = vec![0u8; n];
        assert_eq!(pixelmatch(&x, &y, Some(&mut out), w, h, 0.2), 115);
        let sum = out
            .iter()
            .fold(0u64, |s, v| (s * 31 + *v as u64) % 1_000_000_007);
        assert_eq!(sum, 845_922_178);
    }

    #[test]
    fn png_round_trips() {
        let mut img = solid(3, 2, RED);
        set(&mut img, 1, 1, [1, 2, 3, 4]);
        let bytes = encode_png(&img);
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(decode_png(&bytes).unwrap(), img);
        assert_eq!(
            decode_png(b"nope").unwrap_err().to_string(),
            Error::InvalidArgument("Could not decode expected image as PNG.".into()).to_string()
        );
    }
}
