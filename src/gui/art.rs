//! Character and light cone art for the warp screen.
//!
//! Two sources:
//! - StarRailRes, for every character's and light cone's portrait and preview.
//! - The warp banner layers (hsr-warp-art): a limited banner's art as the game draws it, in layers
//!   cut from one canvas (backdrop, back, character, front), with where the game places each. Its
//!   `index.json` says what's there. Until it has a home online it's read from wherever
//!   `RQA_WARP_ART` points: a folder or a URL.
//!
//! Art is downloaded the first time it's shown and kept in the app's cache folder, so it isn't
//! shipped with the app and is only fetched once. Portraits and banner layers are kept decoded (as
//! QOI) at the size the game has them, since the warp screen draws them about as large.
//!
//! The view asks with [`ArtCache::get`], which answers at once: the image if it's there, `None`
//! while it's on its way. A worker thread downloads and decodes, then wakes the window, and
//! [`ArtCache::take_done`] (called from `poll`) hands the result to Rune.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use image::DynamicImage;
use rune::{Color, Image, ImageCache};

const BASE_URL: &str = "https://raw.githubusercontent.com/Mar-7th/StarRailRes/master";
/// Where the warp banner layers are, until they have a home online: a folder or a URL
const WARP_ART_ENV: &str = "RQA_WARP_ART";
/// Portraits taller than this are scaled down to it. Their source is 2048, which a high density
/// screen shows nearly at full size, so it's kept
const PORTRAIT_HEIGHT: u32 = 2048;

/// A piece of art, by the id of what it shows
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Art {
    /// A character's or light cone's full art
    Portrait(u32),
    /// A character's half-length art, or a light cone's card
    Preview(u32),
    /// An item's icon, like a pass or Stellar Jade
    Item(u32),
    /// A layer of a warp banner's art, by the banner's featured item
    Layer(u32, Layer),
    /// A warp's ticket, as the game's warp screen tabs it; selected or not
    Ticket(Ticket, bool),
    /// The motif a banner's panel tiles (`BannerArt::surface`), by the banner's featured item
    Pattern(u32),
}

/// Whose ticket
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ticket {
    /// A limited banner's, by its featured item
    Banner(u32),
    Standard,
    Departure,
}

impl Ticket {
    /// The ticket a warp shows: its banner's (the newest running), or the standing warp's own
    pub fn of(summary: &crate::warps::service::Summary, pool: crate::warps::Pool) -> Option<Ticket> {
        use crate::warps::Pool;
        match pool {
            Pool::Standard => Some(Ticket::Standard),
            Pool::Departure => Some(Ticket::Departure),
            _ => summary.running(pool).next()?.fives.first().copied().map(Ticket::Banner),
        }
    }
}

/// The layers of a warp banner's art, back to front
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// A blurred backdrop for the whole screen
    Backdrop,
    Back,
    /// The character or light cone
    Main,
    Front,
    /// The banner's ticket on the warp screen's tabs, and as it shows selected: whole pictures,
    /// not pieces of the canvas
    Tab,
    TabSelected,
}

impl Layer {
    fn from_name(name: &str) -> Option<Layer> {
        Some(match name {
            "backdrop" => Layer::Backdrop,
            "bg" => Layer::Back,
            "avatar" | "light_cone" => Layer::Main,
            "front" => Layer::Front,
            "tab" => Layer::Tab,
            "tab_selected" => Layer::TabSelected,
            _ => return None,
        })
    }
}

impl Art {
    fn path(self) -> String {
        let db = reliquary_archiver::export::database::get_database();
        let (kind, id) = match self {
            Art::Portrait(id) if db.is_light_cone(id) => ("image/light_cone_portrait", id),
            Art::Portrait(id) => ("image/character_portrait", id),
            Art::Preview(id) if db.is_light_cone(id) => ("image/light_cone_preview", id),
            Art::Preview(id) => ("image/character_preview", id),
            Art::Item(id) => ("icon/item", id),
            Art::Layer(id, layer) => return format!("layer/{id}/{layer:?}.png"),
            Art::Ticket(ticket, selected) => {
                return format!("ticket/{ticket:?}{}.png", if selected { "-selected" } else { "" });
            }
            Art::Pattern(id) => return format!("pattern/{id}.png"),
        };
        format!("{kind}/{id}.png")
    }

    /// The name of its file in the cache folder
    fn file_name(self, build: &str) -> String {
        let path = self.path().replace('/', "-");
        match self {
            Art::Portrait(_) => path.replace(".png", &format!("-{PORTRAIT_HEIGHT}.qoi")),
            Art::Layer(..) => format!("warp-{build}-{}", path.replace(".png", "-full.qoi")),
            Art::Ticket(..) | Art::Pattern(_) => format!("warp-{build}-{}", path.replace(['(', ')'], "")),
            _ => path,
        }
    }

    /// Scaled down and kept decoded, rather than kept as downloaded
    fn scaled(self) -> bool {
        matches!(self, Art::Portrait(_) | Art::Layer(..))
    }
}

/// A banner's art as the game lays it out
#[derive(Clone, Debug, PartialEq)]
pub struct BannerArt {
    pub layers: Vec<LayerArt>,
    /// What the game puts behind the art, when it's more than the art's own back layer
    pub surface: Option<Surface>,
}

/// The panel behind a banner's art in the game: a colour, a second one fading in towards the
/// bottom, and a small motif tiled over both (`Art::Pattern`). A light cone's banner has only
/// the motif, from the frame beside its art
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub fill: Option<Paint>,
    pub fade: Option<Paint>,
    pub pattern: Option<Pattern>,
}

/// A panel layer's colour: one, or a gradient's two (the game's runs between them at an angle,
/// most often top to bottom)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub start: Color,
    pub end: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    file: String,
    /// What the game draws it in, alpha included
    pub color: Color,
    /// One copy's size on the game's 1920x1080 reference screen
    pub tile: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerArt {
    pub layer: Layer,
    file: String,
    /// The canvas the banner's layers are cut from
    pub canvas: [f32; 2],
    /// This layer's piece of it: x, y, width, height from the top left
    pub rect: [f32; 4],
    /// Where the game draws the canvas on its 1920x1080 reference screen
    pub on_screen: Option<[f32; 4]>,
}

impl BannerArt {
    pub fn layer(&self, layer: Layer) -> Option<&LayerArt> {
        self.layers.iter().find(|l| l.layer == layer)
    }

    /// Whether the banner's art can be drawn in layers, as the game places them
    pub fn placed(&self) -> bool {
        self.layer(Layer::Main).is_some_and(|l| l.on_screen.is_some())
    }
}

/// What the warp banner art has, by featured item
#[derive(Debug, Default)]
struct Manifest {
    build: String,
    banners: HashMap<u32, BannerArt>,
    /// The standard and departure warps' tickets, plain and selected
    tickets: HashMap<Ticket, [Option<String>; 2]>,
}

impl Manifest {
    fn parse(bytes: &[u8]) -> Option<Manifest> {
        let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let build = json["game"]["build"].as_str().unwrap_or("0").to_string();
        let floats = |v: &serde_json::Value| -> Option<Vec<f32>> { v.as_array()?.iter().map(|x| x.as_f64().map(|x| x as f32)).collect() };
        let mut banners = HashMap::new();
        for kind in ["character", "light_cone"] {
            let Some(entries) = json["banners"][kind].as_object() else {
                continue;
            };
            for (id, entry) in entries {
                let Ok(id) = id.parse::<u32>() else { continue };
                let mut layers: Vec<LayerArt> = entry["layers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| {
                        let layer = Layer::from_name(l["layer"].as_str()?)?;
                        let file = l["file"].as_str()?.to_string();
                        let canvas = floats(&l["canvas"]).filter(|v| v.len() == 2).unwrap_or(vec![2048.0, 2048.0]);
                        let rect = floats(&l["rect"])
                            .filter(|v| v.len() == 4)
                            .unwrap_or(vec![0.0, 0.0, canvas[0], canvas[1]]);
                        let on_screen = floats(&l["canvas_on_screen"]).filter(|v| v.len() == 4 && v[2] > 0.0);
                        Some(LayerArt {
                            layer,
                            file,
                            canvas: [canvas[0], canvas[1]],
                            rect: [rect[0], rect[1], rect[2], rect[3]],
                            on_screen: on_screen.map(|v| [v[0], v[1], v[2], v[3]]),
                        })
                    })
                    .collect();
                // Without its main layer placed, a banner's art isn't worth drawing in layers, but
                // its tickets still are
                if !layers.iter().any(|l| l.layer == Layer::Main && l.on_screen.is_some()) {
                    layers.retain(|l| matches!(l.layer, Layer::Tab | Layer::TabSelected));
                }
                let surface = Surface::parse(entry);
                if !layers.is_empty() || surface.is_some() {
                    banners.insert(id, BannerArt { layers, surface });
                }
            }
        }
        let mut tickets = HashMap::new();
        for (name, ticket) in [("standard", Ticket::Standard), ("departure", Ticket::Departure)] {
            let tabs = &json["tabs"][name];
            let file = |layer: &str| tabs[layer].as_str().map(str::to_string);
            if tabs.is_object() {
                tickets.insert(ticket, [file("tab"), file("tab_selected")]);
            }
        }
        Some(Manifest { build, banners, tickets })
    }
}

impl Surface {
    /// The banner's panel, or for a light cone the frame's motif alone
    fn parse(entry: &serde_json::Value) -> Option<Surface> {
        let color = |v: &serde_json::Value| -> Option<Color> {
            let c: Vec<f32> = v.as_array()?.iter().map(|x| x.as_f64().map(|x| x as f32)).collect::<Option<_>>()?;
            (c.len() == 4).then(|| srgb([c[0], c[1], c[2]], c[3]))
        };
        let pattern = |v: &serde_json::Value| -> Option<Pattern> {
            let tile: Vec<f32> = v["tile"].as_array()?.iter().map(|x| x.as_f64().map(|x| x as f32)).collect::<Option<_>>()?;
            (tile.len() == 2 && tile[0] > 0.0 && tile[1] > 0.0).then_some(())?;
            Some(Pattern { file: v["pattern"].as_str()?.to_string(), color: color(&v["color"])?, tile: [tile[0], tile[1]] })
        };
        // A gradient's colours take the place of the layer's own
        let paint = |v: &serde_json::Value| -> Option<Paint> {
            let stops = v["gradient"]["colors"].as_array();
            match stops.map(|s| s.iter().map(color).collect::<Option<Vec<_>>>()) {
                Some(Some(stops)) if stops.len() == 2 => Some(Paint { start: stops[0], end: stops[1] }),
                _ => color(&v["color"]).map(|c| Paint { start: c, end: c }),
            }
        };
        let panel = &entry["panel"];
        let surface = if panel.is_object() {
            Surface {
                fill: paint(&panel["fill"]),
                fade: paint(&panel["fade"]),
                pattern: pattern(&panel["pattern"]).or_else(|| pattern(&entry["frame_pattern"])),
            }
        } else {
            Surface { fill: None, fade: None, pattern: pattern(&entry["frame_pattern"]) }
        };
        (surface.fill.is_some() || surface.pattern.is_some()).then_some(surface)
    }
}

/// A colour as the game's UI gives it, in sRGB
fn srgb(rgb: [f32; 3], alpha: f32) -> Color {
    let [r, g, b] = rgb.map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) });
    Color { r, g, b, a: alpha }
}

enum Slot {
    Loading,
    Ready(Image),
    /// Couldn't be had this run, it isn't asked for again until the app restarts
    Missing,
}

enum Fetched {
    Encoded(Vec<u8>),
    Rgba(u32, u32, Vec<u8>),
}

struct Done {
    art: Art,
    fetched: Option<Fetched>,
    tint: Option<[f32; 3]>,
}

enum Job {
    Manifest,
    Art(Art, String),
}

enum ManifestState {
    NotAsked,
    Loading,
    Ready(Arc<Manifest>),
    Missing,
}

pub struct ArtCache {
    images: ImageCache,
    slots: Mutex<HashMap<Art, Slot>>,
    tints: Mutex<HashMap<Art, Color>>,
    manifest: Mutex<ManifestState>,
    /// Where requests go, `None` to fetch on the spot (for snapshots, which draw one frame)
    jobs: Option<mpsc::Sender<Job>>,
    done: Arc<Mutex<Vec<Done>>>,
    manifest_done: Arc<Mutex<Option<Option<Manifest>>>>,
}

impl ArtCache {
    /// Fetches on a thread of its own, and wakes the window with `waker` when art arrives
    pub fn new(waker: rune::Waker) -> Self {
        let (jobs, requests) = mpsc::channel::<Job>();
        let done: Arc<Mutex<Vec<Done>>> = Arc::default();
        let manifest_done: Arc<Mutex<Option<Option<Manifest>>>> = Arc::default();
        let (finished, manifest_finished) = (done.clone(), manifest_done.clone());
        let wake = waker.clone();
        std::thread::Builder::new()
            .name("warp-art".into())
            .spawn(move || {
                let agent = agent();
                while let Ok(job) = requests.recv() {
                    match job {
                        Job::Manifest => {
                            *manifest_finished.lock().unwrap_or_else(PoisonError::into_inner) = Some(fetch_manifest(&agent));
                        }
                        Job::Art(art, source) => {
                            let (fetched, tint) = fetch(&agent, art, &source);
                            finished
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .push(Done { art, fetched, tint });
                        }
                    }
                    wake();
                }
            })
            .expect("couldn't start the art thread");
        Self::with(waker, Some(jobs), done, manifest_done)
    }

    /// Fetches when asked, for snapshots
    pub fn blocking(waker: rune::Waker) -> Self {
        Self::with(waker, None, Arc::default(), Arc::default())
    }

    fn with(
        waker: rune::Waker,
        jobs: Option<mpsc::Sender<Job>>,
        done: Arc<Mutex<Vec<Done>>>,
        manifest_done: Arc<Mutex<Option<Option<Manifest>>>>,
    ) -> Self {
        Self {
            images: ImageCache::new(waker),
            slots: Mutex::default(),
            tints: Mutex::default(),
            manifest: Mutex::new(ManifestState::NotAsked),
            jobs,
            done,
            manifest_done,
        }
    }

    /// How a banner featuring `item` lays its art out, when the banner art has it. `None` while
    /// that's being found out
    pub fn banner(&self, item: u32) -> Option<BannerArt> {
        self.manifest().and_then(|m| m.banners.get(&item).cloned())
    }

    fn manifest(&self) -> Option<Arc<Manifest>> {
        let mut state = self.manifest.lock().unwrap_or_else(PoisonError::into_inner);
        match &*state {
            ManifestState::Ready(m) => return Some(m.clone()),
            ManifestState::Loading | ManifestState::Missing => return None,
            ManifestState::NotAsked => {}
        }
        match &self.jobs {
            Some(jobs) => {
                *state = ManifestState::Loading;
                jobs.send(Job::Manifest).ok();
                None
            }
            None => {
                let manifest = fetch_manifest(&agent()).map(Arc::new);
                *state = manifest.clone().map_or(ManifestState::Missing, ManifestState::Ready);
                manifest
            }
        }
    }

    /// The art if it's here, `None` while it's on its way or when it couldn't be had
    pub fn get(&self, art: Art) -> Option<Image> {
        {
            let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            match slots.get(&art) {
                Some(Slot::Ready(image)) => return image.error().is_none().then(|| image.clone()),
                Some(Slot::Loading | Slot::Missing) => return None,
                None => {}
            }
        }
        // Layers and tickets come from where the manifest says, under the banner art's base
        let source = match art {
            Art::Layer(..) | Art::Ticket(..) | Art::Pattern(_) => {
                let manifest = self.manifest()?;
                let file = match art {
                    Art::Layer(item, layer) => manifest.banners.get(&item)?.layer(layer)?.file.clone(),
                    Art::Ticket(Ticket::Banner(item), selected) => {
                        let layer = if selected { Layer::TabSelected } else { Layer::Tab };
                        manifest.banners.get(&item)?.layer(layer)?.file.clone()
                    }
                    Art::Ticket(ticket, selected) => manifest.tickets.get(&ticket)?[usize::from(selected)].clone()?,
                    Art::Pattern(item) => manifest.banners.get(&item)?.surface.as_ref()?.pattern.as_ref()?.file.clone(),
                    _ => return None,
                };
                format!("{}\n{}\n{file}", manifest.build, warp_art_base()?)
            }
            _ => String::new(),
        };
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        match &self.jobs {
            Some(jobs) => {
                slots.insert(art, Slot::Loading);
                jobs.send(Job::Art(art, source)).ok();
                None
            }
            None => {
                let (fetched, tint) = fetch(&agent(), art, &source);
                let image = fetched.map(|fetched| self.install(fetched));
                if let Some(image) = &image {
                    let _ = image.wait();
                }
                if let Some(tint) = tint {
                    self.tints.lock().unwrap_or_else(PoisonError::into_inner).insert(art, color(tint));
                }
                slots.insert(art, image.clone().map_or(Slot::Missing, Slot::Ready));
                image
            }
        }
    }

    /// The colour of a banner's backdrop, once it's here: the banner's own accent
    pub fn tint(&self, item: u32) -> Option<Color> {
        self.tints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&Art::Layer(item, Layer::Backdrop))
            .copied()
    }

    /// Hands art the thread finished to Rune. True when there was any
    pub fn take_done(&self) -> bool {
        let mut changed = false;
        if let Some(manifest) = self.manifest_done.lock().unwrap_or_else(PoisonError::into_inner).take() {
            *self.manifest.lock().unwrap_or_else(PoisonError::into_inner) = match manifest {
                Some(manifest) => ManifestState::Ready(Arc::new(manifest)),
                None => ManifestState::Missing,
            };
            changed = true;
        }
        let done = std::mem::take(&mut *self.done.lock().unwrap_or_else(PoisonError::into_inner));
        if done.is_empty() {
            return changed;
        }
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        for Done { art, fetched, tint } in done {
            if let Some(tint) = tint {
                self.tints.lock().unwrap_or_else(PoisonError::into_inner).insert(art, color(tint));
            }
            let slot = match fetched {
                Some(fetched) => Slot::Ready(self.install(fetched)),
                None => Slot::Missing,
            };
            slots.insert(art, slot);
        }
        true
    }

    fn install(&self, fetched: Fetched) -> Image {
        match fetched {
            Fetched::Encoded(bytes) => self.images.load(bytes),
            Fetched::Rgba(width, height, rgba) => self.images.rgba(width, height, rgba),
        }
    }
}

/// An accent from a picture's colour: its hue, at a lightness and strength that read as an accent
/// on dark and light alike. A grey picture gives a soft gold
fn color(rgb: [f32; 3]) -> Color {
    let [_, a, bb, _] = srgb(rgb, 1.0).to_oklab();
    let chroma = (a * a + bb * bb).sqrt();
    if chroma < 0.02 {
        return rune::oklch(0.72, 0.1, 80.0);
    }
    rune::oklch(0.7, chroma.clamp(0.1, 0.17), bb.atan2(a).to_degrees())
}

/// Where the warp banner art is, when it's set
fn warp_art_base() -> Option<String> {
    std::env::var(WARP_ART_ENV)
        .ok()
        .filter(|base| !base.trim().is_empty())
        .map(|base| base.trim().trim_end_matches(['/', '\\']).to_string())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("reliquary-archiver/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn download(agent: &ureq::Agent, url: &str) -> Option<Vec<u8>> {
    let response = agent.get(url).call().inspect_err(|e| tracing::debug!(%e, url, "no art")).ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes).ok()?;
    Some(bytes)
}

/// A file under the warp banner art's base, which is a folder or a URL
fn read_warp_art(agent: &ureq::Agent, base: &str, file: &str) -> Option<Vec<u8>> {
    if base.starts_with("http://") || base.starts_with("https://") {
        download(agent, &format!("{base}/{file}"))
    } else {
        std::fs::read(PathBuf::from(base).join(file))
            .inspect_err(|e| tracing::debug!(%e, file, "no art"))
            .ok()
    }
}

/// The warp banner art's manifest, fresh, or as kept last time when it can't be had
fn fetch_manifest(agent: &ureq::Agent) -> Option<Manifest> {
    let kept = cache_dir().map(|dir| dir.join("warp-index.json"));
    let fresh = warp_art_base().and_then(|base| read_warp_art(agent, &base, "index.json"));
    let bytes = match fresh {
        Some(bytes) => {
            if let Some(path) = &kept {
                keep(path, &bytes);
            }
            bytes
        }
        None => std::fs::read(kept?).ok()?,
    };
    Manifest::parse(&bytes)
}

/// The art from the cache folder, or downloaded into it, and for a banner's backdrop its colour.
/// `source` is, for a banner layer, the manifest's build, the base and the file, a line each
fn fetch(agent: &ureq::Agent, art: Art, source: &str) -> (Option<Fetched>, Option<[f32; 3]>) {
    let mut lines = source.lines();
    let build = lines.next().unwrap_or("0");
    let cached = cache_dir().map(|dir| dir.join(art.file_name(build)));
    let backdrop = matches!(art, Art::Layer(_, Layer::Backdrop));
    if let Some(path) = &cached
        && let Ok(bytes) = std::fs::read(path)
    {
        if !art.scaled() {
            return (Some(Fetched::Encoded(bytes)), None);
        }
        if let Ok(image) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Qoi) {
            let tint = backdrop.then(|| mean_color(&image));
            return (Some(rgba(image)), tint);
        }
    }

    let bytes = match art {
        Art::Layer(..) | Art::Ticket(..) | Art::Pattern(_) => {
            let (Some(base), Some(file)) = (lines.next(), lines.next()) else {
                return (None, None);
            };
            read_warp_art(agent, base, file)
        }
        _ => download(agent, &format!("{BASE_URL}/{}", art.path())),
    };
    let Some(bytes) = bytes else {
        return (None, None);
    };

    let (fetched, keep_bytes, tint) = if art.scaled() {
        let Ok(image) = image::load_from_memory(&bytes) else {
            return (None, None);
        };
        // Banner layers are drawn as large as the window, they're kept as the game has them
        let scale = match art {
            Art::Layer(..) => 1.0,
            _ => (PORTRAIT_HEIGHT as f32 / image.height() as f32).min(1.0),
        };
        let image = if scale < 1.0 {
            let w = (image.width() as f32 * scale).round().max(1.0) as u32;
            let h = (image.height() as f32 * scale).round().max(1.0) as u32;
            image.thumbnail_exact(w, h)
        } else {
            image
        };
        let tint = backdrop.then(|| mean_color(&image));
        let mut qoi = Vec::new();
        let _ = image.write_to(&mut std::io::Cursor::new(&mut qoi), image::ImageFormat::Qoi);
        (rgba(image), qoi, tint)
    } else {
        (Fetched::Encoded(bytes.clone()), bytes, None)
    };
    if let Some(path) = cached
        && !keep_bytes.is_empty()
    {
        keep(&path, &keep_bytes);
    }
    (Some(fetched), tint)
}

/// Writes beside the file and moves it in place, so a half written file is never read
fn keep(path: &std::path::Path, bytes: &[u8]) {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&temp, bytes))
        .and_then(|()| std::fs::rename(&temp, path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        tracing::debug!(%e, "couldn't keep art");
    }
}

/// The image's colour, its saturated pixels counting most, as sRGB 0..1
fn mean_color(image: &DynamicImage) -> [f32; 3] {
    let small = image.thumbnail(32, 32).into_rgb8();
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for p in small.pixels() {
        let [r, g, b] = p.0.map(|c| c as f32 / 255.0);
        let saturation = r.max(g).max(b) - r.min(g).min(b);
        let w = 0.05 + saturation * saturation;
        sum[0] += r * w;
        sum[1] += g * w;
        sum[2] += b * w;
        weight += w;
    }
    sum.map(|c| c / weight.max(1e-6))
}

fn rgba(image: DynamicImage) -> Fetched {
    let rgba = image.into_rgba8();
    Fetched::Rgba(rgba.width(), rgba.height(), rgba.into_raw())
}

/// The app's cache folder: under the local app data folder on Windows, Caches on macOS, the XDG
/// cache folder elsewhere
fn cache_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("reliquary-archiver").join("Cache"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Caches").join("reliquary-archiver"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|dir| dir.join("reliquary-archiver"))
    };
    base.filter(|path| path.is_absolute()).map(|dir| dir.join("art"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_comes_from_the_folder_for_its_kind() {
        assert_eq!(Art::Portrait(1506).path(), "image/character_portrait/1506.png");
        assert_eq!(Art::Preview(23048).path(), "image/light_cone_preview/23048.png");
        assert_eq!(Art::Item(102).path(), "icon/item/102.png");
        assert_eq!(Art::Portrait(1506).file_name("1"), "image-character_portrait-1506-2048.qoi");
        assert_eq!(
            Art::Layer(1511, Layer::Main).file_name("20261029-0524"),
            "warp-20261029-0524-layer-1511-Main-full.qoi"
        );
    }

    #[test]
    fn tickets_are_kept_as_they_come() {
        assert_eq!(
            Art::Ticket(Ticket::Banner(1511), true).file_name("b"),
            "warp-b-ticket-Banner1511-selected.png"
        );
        assert_eq!(Art::Ticket(Ticket::Standard, false).file_name("b"), "warp-b-ticket-Standard.png");
    }

    #[test]
    fn the_banner_art_manifest_reads() {
        let json = br#"{"game":{"build":"b"},"banners":{"character":{"1511":{"layers":[
            {"layer":"backdrop","file":"characters/1511/backdrop.png"},
            {"layer":"avatar","file":"characters/1511/avatar.png","canvas":[2048,2048],"rect":[2,191,1958,1715],"canvas_on_screen":[585.8,-161.2,1382.4,1382.4]}
        ],"panel":{"fill":{"color":[0.16,0.11,0.41,1],"gradient":{"colors":[[0.75,0.01,0,1],[0.2,0,0,1]],"angle":-110}},
            "fade":{"color":[0.06,0.94,1,1]},
            "pattern":{"color":[0.06,0.04,0.16,0.05],"pattern":"patterns/decopatternpixeleater.png","tile":[69.33,61.33]}},
          "frame_pattern":{"color":[0,0,0,0.035],"pattern":"patterns/decopatternpixeleatersingle.png","tile":[26.67,55.76]}}},
        "light_cone":{"23048":{"layers":[{"layer":"bg","file":"light_cones/23048/bg.png"},
            {"layer":"tab","file":"light_cones/23048/tab.png"}],
          "frame_pattern":{"color":[0,0,0,0.035],"pattern":"patterns/decopatterncross.png","tile":[20,20]}},
          "23049":{"layers":[{"layer":"bg","file":"light_cones/23049/bg.png"}]}}},
        "tabs":{"standard":{"tab":"tabs/standard_tab.png","tab_selected":"tabs/standard_tab_selected.png"}}}"#;
        let manifest = Manifest::parse(json).unwrap();
        let banner = &manifest.banners[&1511];
        assert_eq!(banner.layer(Layer::Main).unwrap().rect, [2.0, 191.0, 1958.0, 1715.0]);
        assert!(banner.layer(Layer::Backdrop).unwrap().on_screen.is_none());
        // Without its main layer placed, only a banner's tickets are kept
        let cone = &manifest.banners[&23048];
        assert!(!cone.placed());
        assert!(cone.layer(Layer::Back).is_none() && cone.layer(Layer::Tab).is_some());
        // A character's panel: its colours in linear light, a gradient's in place of the layer's
        // own, and the panel's own motif
        let panel = banner.surface.as_ref().unwrap();
        let fill = panel.fill.unwrap();
        assert!((fill.start.r - 0.522).abs() < 0.01 && (fill.end.r - 0.033).abs() < 0.01, "{fill:?}");
        let fade = panel.fade.unwrap();
        assert_eq!(fade.start, fade.end);
        assert_eq!(panel.pattern.as_ref().unwrap().file, "patterns/decopatternpixeleater.png");
        assert_eq!(panel.pattern.as_ref().unwrap().tile, [69.33, 61.33]);
        // A light cone's has only the frame's motif; a banner with neither has none, and with
        // nothing else of use isn't kept
        let cone_panel = cone.surface.as_ref().unwrap();
        assert!(cone_panel.fill.is_none() && cone_panel.pattern.as_ref().unwrap().tile == [20.0, 20.0]);
        assert!(!manifest.banners.contains_key(&23049));
        assert_eq!(
            manifest.tickets[&Ticket::Standard][1].as_deref(),
            Some("tabs/standard_tab_selected.png")
        );
    }
}
