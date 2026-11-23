//! Getting a background image ready to draw.
//!
//! Rune decodes and mip maps an image file at its full size, and a wallpaper is often several
//! times larger than any window it's shown in: a 5760×3240 JPEG took over ten seconds before it
//! showed in a debug build. So the image is decoded here instead, off the UI thread, scaled down
//! to what a window can show, and kept in the app's data folder at that size. Later launches read
//! the small copy and skip the large decode altogether.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{DynamicImage, ImageDecoder, ImageReader};

use super::settings::ImageFit;

/// Images are scaled down until they just cover this many pixels
const COVER: (u32, u32) = (2560, 1600);
/// Where the scaled copies are kept, inside the app's data folder
const FOLDER: &str = "backgrounds";

/// An image ready to hand to Rune: straight-alpha sRGB rows
pub struct Prepared {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The average colour along each edge: top, right, bottom, left
    pub edges: [rune::Color; 4],
    /// A small copy, to pick colours from with the eyedropper
    pub sample: std::sync::Arc<Sample>,
}

/// How wide or tall [`Sample`] is at most, in pixels: far finer than a pointer picks
const SAMPLE: u32 = 512;

/// A small copy of the background image, straight-alpha sRGB rows, to pick colours from
#[derive(Debug)]
pub struct Sample {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Sample {
    /// The colour at `at` (0 to 1 across and down), averaged over the pixels around it so a single
    /// speck of noise doesn't decide it, as `#rrggbb`
    pub fn hex_at(&self, at: [f32; 2]) -> Option<String> {
        let (w, h) = (self.width as i64, self.height as i64);
        if w == 0 || h == 0 {
            return None;
        }
        let x = ((at[0].clamp(0.0, 1.0) * w as f32) as i64).min(w - 1);
        let y = ((at[1].clamp(0.0, 1.0) * h as f32) as i64).min(h - 1);
        let mut sum = [0u32; 3];
        let mut n = 0;
        for yy in (y - 1).max(0)..=(y + 1).min(h - 1) {
            for xx in (x - 1).max(0)..=(x + 1).min(w - 1) {
                let i = ((yy * w + xx) * 4) as usize;
                for c in 0..3 {
                    sum[c] += u32::from(self.rgba[i + c]);
                }
                n += 1;
            }
        }
        let [r, g, b] = sum.map(|v| (v + n / 2) / n);
        Some(format!("#{r:02x}{g:02x}{b:02x}"))
    }
}

/// How deep a strip along each edge [`Prepared::edges`] averages, as a share of the image
const EDGE_STRIP: f32 = 0.02;

/// Whether the image at `path` goes through this module, rather than Rune drawing it at the size
/// it's shown at, as it does for vector images
pub fn is_raster(path: &str) -> bool {
    !matches!(
        Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("svg" | "svgz")
    )
}

/// Reads the image at `path`, from the scaled copy kept earlier if there is one
pub fn prepare(path: &str) -> Result<Prepared, String> {
    let started = Instant::now();
    let ms = |since: Instant| since.elapsed().as_secs_f64() * 1000.0;
    let cached = cache_path(Path::new(path));
    if let Some(cached) = &cached
        && let Ok(image) = image::open(cached)
    {
        let prepared = into_prepared(image);
        tracing::debug!(ms = ms(started), "background read from its scaled copy");
        return Ok(prepared);
    }

    let image = decode(Path::new(path))?;
    let decoded = Instant::now();
    let (width, height) = (image.width(), image.height());
    let image = scaled(image);
    let prepared = into_prepared(image);
    tracing::debug!(
        width,
        height,
        decode = decoded.duration_since(started).as_secs_f64() * 1000.0,
        scale = ms(decoded),
        "background decoded and scaled"
    );
    // The image shows without waiting for its copy to be written
    if let Some(cached) = cached {
        let copy = image::RgbaImage::from_raw(prepared.width, prepared.height, prepared.rgba.clone());
        std::thread::spawn(move || {
            if let Some(copy) = copy {
                keep(copy, &cached);
            }
        });
    }
    Ok(prepared)
}

/// Decodes the file, turned upright by its EXIF orientation as a phone photo needs
fn decode(path: &Path) -> Result<DynamicImage, String> {
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let reader = ImageReader::open(path)
        .map_err(|e| error(&e))?
        .with_guessed_format()
        .map_err(|e| error(&e))?;
    let mut decoder = reader.into_decoder().map_err(|e| error(&e))?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| error(&e))?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// The image scaled down to just cover [`COVER`], keeping its shape
fn scaled(image: DynamicImage) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    let scale = (COVER.0 as f32 / width as f32).max(COVER.1 as f32 / height as f32);
    if scale >= 1.0 {
        return image;
    }
    let (w, h) = (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    );
    image.thumbnail_exact(w, h)
}

fn into_prepared(image: DynamicImage) -> Prepared {
    let small = image.thumbnail(SAMPLE, SAMPLE).into_rgba8();
    let sample = std::sync::Arc::new(Sample {
        width: small.width(),
        height: small.height(),
        rgba: small.into_raw(),
    });
    let rgba = image.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let rgba = rgba.into_raw();
    Prepared {
        width,
        height,
        edges: edge_colors(width, height, &rgba),
        rgba,
        sample,
    }
}

/// The average colour of a strip along each edge of straight-alpha sRGB rows (top, right, bottom,
/// left), averaged in linear light, as the eye mixes them
fn edge_colors(width: u32, height: u32, rgba: &[u8]) -> [rune::Color; 4] {
    let (w, h) = (width as usize, height as usize);
    let deep = |side: usize| ((side as f32 * EDGE_STRIP).ceil() as usize).clamp(1, side.max(1));
    let (across, down) = (deep(w), deep(h));
    let average = |xs: std::ops::Range<usize>, ys: std::ops::Range<usize>| {
        let mut sum = [0.0f64; 4];
        let mut n = 0.0f64;
        for y in ys {
            for x in xs.clone() {
                let i = (y * w + x) * 4;
                let Some(px) = rgba.get(i..i + 4) else { continue };
                let a = f64::from(px[3]) / 255.0;
                for c in 0..3 {
                    sum[c] += srgb_to_linear(f64::from(px[c]) / 255.0) * a;
                }
                sum[3] += a;
                n += 1.0;
            }
        }
        if sum[3] <= 0.0 {
            return rune::Color::TRANSPARENT;
        }
        rune::Color {
            r: (sum[0] / sum[3]) as f32,
            g: (sum[1] / sum[3]) as f32,
            b: (sum[2] / sum[3]) as f32,
            a: (sum[3] / n) as f32,
        }
    };
    [
        average(0..w, 0..down),
        average(w.saturating_sub(across)..w, 0..h),
        average(0..w, h.saturating_sub(down)..h),
        average(0..across, 0..h),
    ]
}

/// An sRGB-encoded channel in linear light
fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Where the background image goes in a window `window` points large: `[x, y, w, h]` in points.
/// `image` is its own size in points, which `fit` scales. Then in each direction the image's
/// `anchor` point (0 to 1 across and down it) goes to the window's `target` point, as near as the
/// image allows: one larger than the window never pulls away from its edges, one smaller never
/// slides out of it. Without a target, the anchor goes to the same place in the window as it is in
/// the image, as CSS's `object-position` puts it
pub fn place(fit: ImageFit, image: [f32; 2], window: [f32; 2], anchor: [f32; 2], target: Option<[f32; 2]>) -> [f32; 4] {
    let [iw, ih] = image;
    let [ww, wh] = window;
    if iw <= 0.0 || ih <= 0.0 || fit == ImageFit::Fill {
        return [0.0, 0.0, ww, wh];
    }
    let scaled = |k: f32| [iw * k, ih * k];
    let contain = scaled((ww / iw).min(wh / ih));
    let size = match fit {
        ImageFit::Fill => [ww, wh],
        ImageFit::Contain => contain,
        ImageFit::Cover => scaled((ww / iw).max(wh / ih)),
        ImageFit::None => image,
        ImageFit::ScaleDown if iw > ww || ih > wh => contain,
        ImageFit::ScaleDown => image,
    };
    let target = target.unwrap_or(anchor);
    let at = |n: usize| {
        let slack = window[n] - size[n];
        (target[n] * window[n] - anchor[n] * size[n]).clamp(slack.min(0.0), slack.max(0.0))
    };
    [at(0), at(1), size[0], size[1]]
}

/// Where the scaled copy of the image at `path` is kept. It names the file's path, size and
/// modification time, so an edited or replaced image is read again
fn cache_path(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok()?.hash(&mut hasher);
    COVER.hash(&mut hasher);
    let folder = super::settings::data_dir()?.join(FOLDER);
    Some(folder.join(format!("{:016x}.qoi", hasher.finish())))
}

/// Keeps the scaled copy, in place of any copy of an earlier background
fn keep(image: image::RgbaImage, at: &Path) {
    let Some(folder) = at.parent() else {
        return;
    };
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let written = std::fs::create_dir_all(folder).map_err(|e| e.to_string()).and_then(|()| {
        // Written beside it and moved in place, so a half written copy is never read
        let temp = at.with_extension(format!("qoi.{}.tmp", std::process::id()));
        DynamicImage::ImageRgba8(image)
            .save_with_format(&temp, image::ImageFormat::Qoi)
            .map_err(|e| e.to_string())?;
        std::fs::rename(&temp, at).map_err(|e| e.to_string())
    });
    if let Err(e) = written {
        tracing::warn!(%e, "couldn't keep a scaled copy of the background");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_images_are_scaled_down_to_cover_the_window_and_small_ones_kept() {
        let big = scaled(DynamicImage::new_rgba8(5760, 3240));
        assert_eq!((big.width(), big.height()), (2844, 1600));
        let tall = scaled(DynamicImage::new_rgba8(4000, 12000));
        assert_eq!((tall.width(), tall.height()), (2560, 7680));
        let small = scaled(DynamicImage::new_rgba8(1200, 800));
        assert_eq!((small.width(), small.height()), (1200, 800));
    }

    #[test]
    fn vector_images_are_left_to_rune() {
        assert!(!is_raster("C:\\\\art\\\\tile.SVG"));
        assert!(is_raster("wallpaper.jpg"));
        assert!(is_raster("no extension"));
    }

    const WINDOW: [f32; 2] = [800.0, 600.0];
    const CENTRE: [f32; 2] = [0.5, 0.5];

    #[test]
    fn centred_it_sits_where_it_always_has() {
        // A wide image covering the window is cut evenly left and right
        assert_eq!(place(ImageFit::Cover, [1600.0, 600.0], WINDOW, CENTRE, Some(CENTRE)), [
            -400.0, 0.0, 1600.0, 600.0
        ]);
        // Contained, it's centred top to bottom
        assert_eq!(place(ImageFit::Contain, [1600.0, 600.0], WINDOW, CENTRE, Some(CENTRE)), [
            0.0, 150.0, 800.0, 300.0
        ]);
        assert_eq!(place(ImageFit::Fill, [1600.0, 600.0], WINDOW, [0.0, 0.0], None), [
            0.0, 0.0, 800.0, 600.0
        ]);
    }

    #[test]
    fn the_anchor_goes_to_the_target_as_near_as_the_image_allows() {
        // The image's point at 60% (960) at the window's left third (266.7)
        let [x, ..] = place(ImageFit::Cover, [1600.0, 600.0], WINDOW, [0.6, 0.5], Some([1.0 / 3.0, 0.5]));
        assert!((x - (800.0 / 3.0 - 960.0)).abs() < 0.01);
        // At the window's left edge it would pull the image's right edge in: it stops there
        let [x, ..] = place(ImageFit::Cover, [1600.0, 600.0], WINDOW, [0.9, 0.5], Some([0.0, 0.5]));
        assert_eq!(x, -800.0);
        // A smaller image doesn't slide out of the window
        let [_, y, ..] = place(ImageFit::Contain, [1600.0, 600.0], WINDOW, [0.5, 0.0], Some([0.5, 1.0]));
        assert_eq!(y, 300.0);
    }

    #[test]
    fn without_a_target_the_anchor_lines_up_as_css_does() {
        // object-position 25% 100%: the image's quarter at the window's quarter, bottoms together
        assert_eq!(place(ImageFit::Cover, [1600.0, 600.0], WINDOW, [0.25, 1.0], None), [
            -200.0, 0.0, 1600.0, 600.0
        ]);
        assert_eq!(place(ImageFit::None, [200.0, 100.0], WINDOW, [1.0, 1.0], None), [
            600.0, 500.0, 200.0, 100.0
        ]);
    }

    #[test]
    fn scale_down_shrinks_only_what_is_too_big() {
        assert_eq!(place(ImageFit::ScaleDown, [200.0, 100.0], WINDOW, CENTRE, None)[2..], [
            200.0, 100.0
        ]);
        assert_eq!(place(ImageFit::ScaleDown, [1600.0, 600.0], WINDOW, CENTRE, None)[2..], [
            800.0, 300.0
        ]);
    }

    #[test]
    fn edges_average_their_own_strip() {
        // 4 by 4: the top row red, the rest blue
        let mut rgba = Vec::new();
        for y in 0..4 {
            for _ in 0..4 {
                rgba.extend_from_slice(if y == 0 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let [top, right, bottom, _] = edge_colors(4, 4, &rgba);
        assert_eq!((top.r, top.b), (1.0, 0.0));
        assert_eq!((bottom.r, bottom.b), (0.0, 1.0));
        assert!((right.r - 0.25).abs() < 1e-6, "a quarter of the right edge is red");
    }
}

#[cfg(test)]
mod sample_tests {
    use super::*;

    #[test]
    fn a_picked_colour_averages_the_pixels_around_it() {
        // 3 by 1: red, red, blue. The middle averages all three; the left edge only its two
        let sample = Sample {
            width: 3,
            height: 1,
            rgba: vec![255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255],
        };
        assert_eq!(sample.hex_at([0.5, 0.5]).as_deref(), Some("#aa0055"));
        assert_eq!(sample.hex_at([0.0, 0.0]).as_deref(), Some("#ff0000"));
        assert_eq!(sample.hex_at([1.0, 1.0]).as_deref(), Some("#800080"));
    }
}
