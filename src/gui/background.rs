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

/// Images are scaled down until they just cover this many pixels
const COVER: (u32, u32) = (2560, 1600);
/// Where the scaled copies are kept, inside the app's data folder
const FOLDER: &str = "backgrounds";

/// An image ready to hand to Rune: straight-alpha sRGB rows
pub struct Prepared {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

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
    let rgba = image.into_rgba8();
    Prepared {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
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
}
