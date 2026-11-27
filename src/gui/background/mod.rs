//! The background image: getting it ready to draw, and what the window keeps of it.
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
use rune::{Image, ImageCache, TextEdit};

use super::settings::ImageFit;

pub mod preview;

/// Images are scaled down until they just cover this many pixels
const COVER: (u32, u32) = (2560, 1600);
/// Where the scaled copies are kept, inside the app's data folder
const FOLDER: &str = "backgrounds";

/// An image ready to hand to Rune: straight-alpha sRGB rows
pub struct Prepared {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The average color along each edge: top, right, bottom, left
    pub edges: [rune::Color; 4],
    /// A small copy, to pick colors from with the eyedropper
    pub sample: std::sync::Arc<Sample>,
}

/// How wide or tall [`Sample`] is at most, in pixels: far finer than a pointer picks
const SAMPLE: u32 = 512;

/// A small copy of the background image, straight-alpha sRGB rows, to pick colors from
#[derive(Debug)]
pub struct Sample {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Sample {
    /// The color at `at` (0 to 1 across and down), averaged over the pixels around it so a single
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

/// The size, in points, vector images are loaded at. Rune draws them at the size they're shown
/// at, which only needs the size
const VECTOR_SIZE: f32 = 1024.0;

/// The background image as the window has it, and what the settings' preview of it is doing
pub struct Background {
    /// Decodes images off the UI thread, set up once the window's waker exists
    pub images: Option<ImageCache>,
    /// Gets images ready off the UI thread, set up with the window
    pub loader: Option<rune::Latest<(String, Result<Prepared, String>)>>,
    /// The image and the path it was loaded from
    pub loaded: Option<(String, Image)>,
    /// The image is still decoding: until it's done its size isn't known, so the view is built
    /// again when it is (see [`Background::ready`])
    pub decoding: bool,
    /// The path the loader is working on
    pub loading: Option<String>,
    /// Why the image couldn't be read
    pub error: Option<String>,
    /// The average color along each edge of the image (top, right, bottom, left), for
    /// [`BackgroundFill::EdgeColor`](super::settings::BackgroundFill::EdgeColor); none for vector
    /// images
    pub edges: Option<[rune::Color; 4]>,
    /// A small copy of the image, to pick colors from; not for vector images
    pub sample: Option<std::sync::Arc<Sample>>,
    /// The eyedropper is on: pressing on the image picks the color around it
    pub dropper: bool,
    /// What a drag on the settings' preview is doing, picked where it was pressed
    pub grab: Option<Grab>,
    /// What a press on the settings' preview would take, where the pointer is over it
    pub hover: Option<Grab>,
    /// The window size the settings' preview tries in place of the window's own, to see how the
    /// image holds up when the window resizes
    pub trial: Option<[f32; 2]>,
    /// The color around the image as it's being picked: its hue kept through grays, which have none
    pub hsv: Option<rune_kit::Hsv>,
    /// That color's hex code, typed or kept in step with the picker
    pub hex: TextEdit,
}

/// What a drag on the settings' background preview does, picked by where it was pressed
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    /// Moves the image, which was this far from the window's top-left when pressed
    Pan([f32; 2]),
    /// Moves the point that holds still as the window resizes, leaving the image where it is:
    /// `from` the window's top-left, as it was when pressed. Dragged past the window's edges,
    /// the pin is `out` there, in the window's points, until it's let go
    Pin { from: [f32; 2], out: Option<[f32; 2]> },
    /// Tries another window size
    Resize,
}

impl Background {
    /// Nothing loaded, with `color` (`#rrggbb`) picked for around the image
    pub fn new(color: Option<&str>) -> Self {
        Self {
            images: None,
            loader: None,
            loaded: None,
            decoding: false,
            loading: None,
            error: None,
            edges: None,
            sample: None,
            dropper: false,
            grab: None,
            hover: None,
            trial: None,
            hsv: None,
            hex: TextEdit::new(color.unwrap_or_default()),
        }
    }

    /// Starts loading the image at `path`, if it isn't loaded or on its way. An empty path clears it
    pub fn load(&mut self, path: &str) {
        if path.is_empty() {
            self.clear();
            return;
        }
        if self.loaded.as_ref().is_some_and(|(loaded, _)| loaded == path) || self.loading.as_deref() == Some(path) {
            return;
        }
        let (Some(_), Some(loader)) = (&self.images, &self.loader) else {
            return;
        };
        self.error = None;
        if is_raster(path) {
            self.loading = Some(path.to_string());
            loader.request(path.to_string(), |path| {
                let prepared = prepare(&path);
                (path, prepared)
            });
        } else if let Some(image) = self.install_vector(path) {
            self.show(path.to_string(), image);
        }
    }

    /// Loads the image at `path` on this thread and waits for it, for snapshots, which draw a
    /// single frame
    pub fn load_now(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        let image = if is_raster(path) {
            match prepare(path) {
                Ok(prepared) => self.install_prepared(prepared),
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        } else {
            self.install_vector(path)
        };
        let Some(image) = image else {
            return;
        };
        let _ = image.wait();
        self.loaded = Some((path.to_string(), image));
    }

    /// Takes an image the loader finished for `path`, or notices the image it handed over
    /// finished decoding (it wakes the window then, but the view has to be built again to see its
    /// size). True when that changed anything
    pub fn ready(&mut self, path: &str) -> bool {
        let taken = self.take(path);
        let decoded = self.decoding && self.loaded.as_ref().is_none_or(|(_, image)| image.is_ready());
        if decoded {
            self.decoding = false;
        }
        taken || decoded
    }

    /// Takes an image the loader finished, if it's for `path`. True when that changed anything
    fn take(&mut self, path: &str) -> bool {
        let Some((taken, prepared)) = self.loader.as_ref().and_then(|loader| loader.take()) else {
            return false;
        };
        if self.loading.as_ref() == Some(&taken) {
            self.loading = None;
        }
        // Another image was picked meanwhile
        if taken != path {
            return false;
        }
        match prepared {
            Ok(prepared) => {
                if let Some(image) = self.install_prepared(prepared) {
                    self.show(taken, image);
                }
            }
            Err(e) => {
                tracing::warn!(%e, "couldn't load the background");
                self.clear();
                self.error = Some(e);
            }
        }
        true
    }

    /// Hands a prepared image to Rune, keeping its edge colors and sample
    fn install_prepared(&mut self, prepared: Prepared) -> Option<Image> {
        let image = self.images.as_ref()?.rgba(prepared.width, prepared.height, prepared.rgba);
        self.edges = Some(prepared.edges);
        self.sample = Some(prepared.sample);
        Some(image)
    }

    /// Has Rune load the vector image at `path`, which has no edge colors or sample to pick from
    fn install_vector(&mut self, path: &str) -> Option<Image> {
        let image = self.images.as_ref()?.file(path, VECTOR_SIZE, 1.0);
        self.edges = None;
        self.sample = None;
        self.dropper = false;
        Some(image)
    }

    /// Shows `image`, loaded from `path`, once it's decoded
    fn show(&mut self, path: String, image: Image) {
        self.loaded = Some((path, image));
        self.decoding = true;
    }

    /// Forgets the image and everything taken from it
    pub fn clear(&mut self) {
        self.loaded = None;
        self.loading = None;
        self.error = None;
        self.edges = None;
        self.sample = None;
        self.dropper = false;
    }

    /// The image behind the window, when there is one that decoded
    pub fn image(&self) -> Option<&Image> {
        self.loaded.as_ref().map(|(_, image)| image).filter(|image| image.error().is_none())
    }

    /// The image's darkest edge color
    pub fn darkest_edge(&self) -> Option<rune_kit::Hsv> {
        self.edges?
            .into_iter()
            .min_by(|a, b| a.luminance().total_cmp(&b.luminance()))
            .map(rune_kit::Hsv::of)
    }

    /// The color around `at` (0 to 1 across and down) of the image, as the eyedropper picks it
    pub fn color_at(&self, at: [f32; 2]) -> Option<rune_kit::Hsv> {
        let hex = self.sample.as_ref()?.hex_at(at)?;
        rune_kit::Hsv::from_hex(&hex)
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

/// The average color of a strip along each edge of straight-alpha sRGB rows (top, right, bottom,
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
                    sum[c] += f64::from(super::srgb_to_linear(f32::from(px[c]) / 255.0)) * a;
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

/// How far the background image zooms from its fit, smallest and largest
/// ([`super::settings::Settings::background_zoom`])
pub const ZOOM: [f32; 2] = [0.5, 4.0];

/// Where the background image goes in a window `window` points large: `[x, y, w, h]` in points.
/// `image` is its own size in points, which `fit` scales, and then `zoom` (1 for as it fits). Then
/// in each direction the image's `anchor` point (0 to 1 across and down it) goes to the window's
/// `target` point, as near as the image allows: one larger than the window never pulls away from
/// its edges, one smaller never slides out of it. Without a target, the anchor goes to the same
/// place in the window as it is in the image, as CSS's `object-position` puts it
pub fn place(fit: ImageFit, zoom: f32, image: [f32; 2], window: [f32; 2], anchor: [f32; 2], target: Option<[f32; 2]>) -> [f32; 4] {
    let [iw, ih] = image;
    let [ww, wh] = window;
    if iw <= 0.0 || ih <= 0.0 {
        return [0.0, 0.0, ww, wh];
    }
    let scaled = |k: f32| [iw * k, ih * k];
    let contain = scaled((ww / iw).min(wh / ih));
    let fitted = match fit {
        ImageFit::Fill => window,
        ImageFit::Contain => contain,
        ImageFit::Cover => scaled((ww / iw).max(wh / ih)),
        ImageFit::None => image,
        ImageFit::ScaleDown if iw > ww || ih > wh => contain,
        ImageFit::ScaleDown => image,
    };
    let zoom = zoom.clamp(ZOOM[0], ZOOM[1]);
    let size = fitted.map(|side| side * zoom);
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
    let folder = crate::dirs::config_dir()?.join(FOLDER);
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
    let written = crate::dirs::write_atomic(at, |temp| {
        DynamicImage::ImageRgba8(image)
            .save_with_format(temp, image::ImageFormat::Qoi)
            .map_err(std::io::Error::other)
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
    const CENTER: [f32; 2] = [0.5, 0.5];

    #[test]
    fn centered_it_sits_where_it_always_has() {
        // A wide image covering the window is cut evenly left and right
        assert_eq!(place(ImageFit::Cover, 1.0, [1600.0, 600.0], WINDOW, CENTER, Some(CENTER)), [
            -400.0, 0.0, 1600.0, 600.0
        ]);
        // Contained, it's centered top to bottom
        assert_eq!(place(ImageFit::Contain, 1.0, [1600.0, 600.0], WINDOW, CENTER, Some(CENTER)), [
            0.0, 150.0, 800.0, 300.0
        ]);
        assert_eq!(place(ImageFit::Fill, 1.0, [1600.0, 600.0], WINDOW, [0.0, 0.0], None), [
            0.0, 0.0, 800.0, 600.0
        ]);
    }

    #[test]
    fn the_anchor_goes_to_the_target_as_near_as_the_image_allows() {
        // The image's point at 60% (960) at the window's left third (266.7)
        let [x, ..] = place(ImageFit::Cover, 1.0, [1600.0, 600.0], WINDOW, [0.6, 0.5], Some([1.0 / 3.0, 0.5]));
        assert!((x - (800.0 / 3.0 - 960.0)).abs() < 0.01);
        // At the window's left edge it would pull the image's right edge in: it stops there
        let [x, ..] = place(ImageFit::Cover, 1.0, [1600.0, 600.0], WINDOW, [0.9, 0.5], Some([0.0, 0.5]));
        assert_eq!(x, -800.0);
        // A smaller image doesn't slide out of the window
        let [_, y, ..] = place(ImageFit::Contain, 1.0, [1600.0, 600.0], WINDOW, [0.5, 0.0], Some([0.5, 1.0]));
        assert_eq!(y, 300.0);
    }

    #[test]
    fn without_a_target_the_anchor_lines_up_as_css_does() {
        // object-position 25% 100%: the image's quarter at the window's quarter, bottoms together
        assert_eq!(place(ImageFit::Cover, 1.0, [1600.0, 600.0], WINDOW, [0.25, 1.0], None), [
            -200.0, 0.0, 1600.0, 600.0
        ]);
        assert_eq!(place(ImageFit::None, 1.0, [200.0, 100.0], WINDOW, [1.0, 1.0], None), [
            600.0, 500.0, 200.0, 100.0
        ]);
    }

    #[test]
    fn zoom_grows_the_fit_about_the_anchor() {
        // Covering, twice as large: the centers stay together
        assert_eq!(place(ImageFit::Cover, 2.0, [1600.0, 600.0], WINDOW, CENTER, Some(CENTER)), [
            -1200.0, -300.0, 3200.0, 1200.0
        ]);
        // Zoomed in on the top left corner, it stays in the window's top left
        assert_eq!(place(ImageFit::Fill, 2.0, [1600.0, 600.0], WINDOW, [0.0, 0.0], None), [
            0.0, 0.0, 1600.0, 1200.0
        ]);
        // Zoomed out, the fit shrinks and the anchor goes where it would
        assert_eq!(place(ImageFit::Contain, 0.5, [1600.0, 600.0], WINDOW, CENTER, Some(CENTER)), [
            200.0, 225.0, 400.0, 150.0
        ]);
        // Past the range, it stops at its ends
        assert_eq!(place(ImageFit::Fill, 100.0, [1.0, 1.0], WINDOW, CENTER, None)[2..], [
            3200.0, 2400.0
        ]);
    }

    #[test]
    fn scale_down_shrinks_only_what_is_too_big() {
        assert_eq!(place(ImageFit::ScaleDown, 1.0, [200.0, 100.0], WINDOW, CENTER, None)[2..], [
            200.0, 100.0
        ]);
        assert_eq!(place(ImageFit::ScaleDown, 1.0, [1600.0, 600.0], WINDOW, CENTER, None)[2..], [
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

    #[test]
    fn a_picked_color_averages_the_pixels_around_it() {
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
