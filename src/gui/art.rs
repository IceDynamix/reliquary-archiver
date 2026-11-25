//! Character and light cone art for the warp screen, from hsr-warp-art: every character's and
//! light cone's portrait, preview and icon, the passes' icons, and each limited banner's art as
//! the game draws it, in layers cut from one canvas (backdrop, back, character, front), with where
//! the game places each. Its `index.json` says what's there, and each file's SHA-256. It's read
//! from its R2 bucket, or from wherever `RQA_WARP_ART` points: a folder or a URL.
//!
//! Art is downloaded the first time it's shown and kept in the app's cache folder, so it isn't
//! shipped with the app and is only fetched again when its hash changes. It comes as JPEG XL and is
//! kept decoded (as QOI) at the size the game has it, since the warp screen draws it about as large.
//!
//! The view asks with [`ArtCache::get`], which answers at once: the image if it's there, `None`
//! while it's on its way. Worker threads download, several at once, and decode, a few at once, then
//! wake the window, and
//! [`ArtCache::take_done`] (called from `poll`) hands the result to Rune.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, mpsc};
use std::time::Duration;

use image::DynamicImage;
use rune::{Color, Image, ImageCache};

/// Where the art is: hsr-warp-art's files, in an R2 bucket behind Cloudflare's cache
const WARP_ART_URL: &str = "https://sparxie.anemew.me";
/// Where to read the art from instead, for trying a new extraction: a folder or a URL
const WARP_ART_ENV: &str = "RQA_WARP_ART";
/// Downloads at once. They mostly wait on the network, and the art's edge cache answers quickly,
/// so several run side by side; decoding what they bring is limited apart (`DECODING`)
const DOWNLOADS: usize = 12;
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
    /// A character's round head, or a light cone's small card
    Icon(u32),
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
    /// The ticket a warp shows: the banner of the 5 star `picked` for it when that's running,
    /// else its newest running banner's, or the standing warp's own
    pub fn of(summary: &crate::warps::service::Summary, pool: crate::warps::Pool, picked: Option<u32>) -> Option<Ticket> {
        use crate::warps::Pool;
        match pool {
            Pool::Standard => Some(Ticket::Standard),
            Pool::Departure => Some(Ticket::Departure),
            _ => summary
                .running(pool)
                .find_map(|b| picked.filter(|item| b.fives.contains(item)))
                .or_else(|| summary.running(pool).next()?.fives.first().copied())
                .map(Ticket::Banner),
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
    /// A second front, over the first: some collab banners have two
    Front2,
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
            "front2" => Layer::Front2,
            "tab" => Layer::Tab,
            "tab_selected" => Layer::TabSelected,
            _ => return None,
        })
    }
}

impl Art {
    /// Its file in the art, as the manifest has it
    fn file(self, manifest: &Manifest) -> Option<&str> {
        let own = |id: u32| manifest.own.get(&id);
        let file = match self {
            Art::Portrait(id) => own(id)?.portrait.as_ref(),
            Art::Preview(id) => own(id)?.preview.as_ref(),
            Art::Icon(id) => own(id)?.icon.as_ref(),
            Art::Item(id) => manifest.items.get(&id),
            Art::Layer(item, layer) => Some(&manifest.banners.get(&item)?.layer(layer)?.file),
            Art::Ticket(Ticket::Banner(item), selected) => {
                let layer = if selected { Layer::TabSelected } else { Layer::Tab };
                Some(&manifest.banners.get(&item)?.layer(layer)?.file)
            }
            Art::Ticket(ticket, selected) => manifest.tickets.get(&ticket)?[usize::from(selected)].as_ref(),
            Art::Pattern(item) => Some(&manifest.banners.get(&item)?.surface.as_ref()?.pattern.as_ref()?.file),
        };
        file.map(String::as_str)
    }

    /// The name `file` is kept under in the cache folder, decoded: with its version, so a new one
    /// is fetched rather than the old one read
    fn cache_name(self, file: &str, version: &str) -> String {
        // Both come from the manifest: nothing in them may reach outside the cache folder
        let safe = |s: &str| s.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "-");
        let (stem, version) = (safe(file.rsplit_once('.').map_or(file, |(stem, _)| stem)), safe(version));
        // Portraits are kept scaled down to this
        let height = if matches!(self, Art::Portrait(_)) { format!("-{PORTRAIT_HEIGHT}") } else { String::new() };
        format!("warp-{version}-{stem}{height}.qoi")
    }
}

/// A character's or light cone's own art, not a banner's
#[derive(Clone, Debug, Default, PartialEq)]
struct OwnArt {
    portrait: Option<String>,
    preview: Option<String>,
    icon: Option<String>,
}

/// A banner's art as the game lays it out
#[derive(Clone, Debug, PartialEq)]
pub struct BannerArt {
    pub layers: Vec<LayerArt>,
    /// What the game puts behind the art, when it's more than the art's own back layer
    pub surface: Option<Surface>,
}

/// The panel behind a banner's art in the game: a color, a second one fading in towards the
/// bottom, and a small motif tiled over both (`Art::Pattern`). A light cone's banner has only
/// the motif, from the frame beside its art
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub fill: Option<Paint>,
    pub fade: Option<Paint>,
    pub pattern: Option<Pattern>,
}

/// A panel layer's color: one, or a gradient's two (the game's runs between them at an angle,
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
    /// Where the layer's visual weight sits, on its canvas like `rect`
    pub weight: Option<Weight>,
}

/// Where a character's visual weight sits, measured by hsr-warp-art from its opacity
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weight {
    /// The center of its opacity
    pub center: [f32; 2],
    /// The box holding most of it, without the props and effects that reach out from the figure
    pub core: [f32; 4],
    /// The character themselves, as an anime person detector found them: without the scenery and
    /// effects drawn in the same layer, which opacity can't tell from them
    pub person: Option<[f32; 4]>,
    /// Their head, which stays much the same size however they're posed
    pub head: Option<[f32; 4]>,
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

/// What the art has
#[derive(Debug, Default)]
struct Manifest {
    build: String,
    /// By featured item
    banners: HashMap<u32, BannerArt>,
    /// The standard and departure warps' tickets, plain and selected
    tickets: HashMap<Ticket, [Option<String>; 2]>,
    /// Every character's and light cone's own art, by id
    own: HashMap<u32, OwnArt>,
    /// Items' icons, by id
    items: HashMap<u32, String>,
    /// Each file's SHA-256
    hashes: HashMap<String, String>,
}

impl Manifest {
    /// What a file's copy in the cache is kept as: a short form of its hash, or without one the
    /// game build the art is from
    fn version(&self, file: &str) -> &str {
        self.hashes.get(file).map_or(&self.build, |hash| hash.get(..16).unwrap_or(hash))
    }

    fn parse(bytes: &[u8]) -> Option<Manifest> {
        let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let build = json["game"]["build"].as_str().unwrap_or("0").to_string();
        let floats = |v: &serde_json::Value| -> Option<Vec<f32>> { v.as_array()?.iter().map(|x| x.as_f64().map(|x| x as f32)).collect() };
        let mut banners = HashMap::new();
        for kind in ["character", "light_cone", "collaboration_character", "collaboration_light_cone"] {
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
                            weight: floats(&l["weight"]["centre"])
                                .filter(|v| v.len() == 2)
                                .zip(floats(&l["weight"]["core"]).filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0))
                                .map(|(c, k)| Weight {
                                    center: [c[0], c[1]],
                                    core: [k[0], k[1], k[2], k[3]],
                                    person: floats(&l["weight"]["person"])
                                        .filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0)
                                        .map(|v| [v[0], v[1], v[2], v[3]]),
                                    head: floats(&l["weight"]["head"])
                                        .filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0)
                                        .map(|v| [v[0], v[1], v[2], v[3]]),
                                }),
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
        let mut own = HashMap::new();
        for kind in ["character", "light_cone"] {
            for (id, slots) in json["art"][kind].as_object().into_iter().flatten() {
                let Ok(id) = id.parse::<u32>() else { continue };
                let file = |slot: &str| slots[slot].as_str().map(str::to_string);
                own.insert(id, OwnArt {
                    portrait: file("portrait"),
                    preview: file("preview"),
                    icon: file("icon"),
                });
            }
        }
        let items = json["art"]["item"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(id, slots)| Some((id.parse::<u32>().ok()?, slots["icon"].as_str()?.to_string())))
            .collect();
        let hashes = json["files"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(file, about)| Some((file.clone(), about["sha256"].as_str()?.to_string())))
            .collect();
        Some(Manifest {
            build,
            banners,
            tickets,
            own,
            items,
            hashes,
        })
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
            let tile: Vec<f32> = v["tile"]
                .as_array()?
                .iter()
                .map(|x| x.as_f64().map(|x| x as f32))
                .collect::<Option<_>>()?;
            (tile.len() == 2 && tile[0] > 0.0 && tile[1] > 0.0).then_some(())?;
            Some(Pattern {
                file: v["pattern"].as_str()?.to_string(),
                color: color(&v["color"])?,
                tile: [tile[0], tile[1]],
            })
        };
        // A gradient's colors take the place of the layer's own
        let paint = |v: &serde_json::Value| -> Option<Paint> {
            let stops = v["gradient"]["colors"].as_array();
            match stops.map(|s| s.iter().map(color).collect::<Option<Vec<_>>>()) {
                Some(Some(stops)) if stops.len() == 2 => Some(Paint {
                    start: stops[0],
                    end: stops[1],
                }),
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
            Surface {
                fill: None,
                fade: None,
                pattern: pattern(&entry["frame_pattern"]),
            }
        };
        (surface.fill.is_some() || surface.pattern.is_some()).then_some(surface)
    }
}

/// A color as the game's UI gives it, in sRGB
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

/// Decoded art
struct Fetched {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
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
    /// Fetches on threads of its own, `DOWNLOADS` at once and decoding a few of those at a time
    /// (`DECODING`), and wakes the window with `waker` when art arrives
    pub fn new(waker: rune::Waker) -> Self {
        let (jobs, requests) = mpsc::channel::<Job>();
        let requests = Arc::new(Mutex::new(requests));
        let done: Arc<Mutex<Vec<Done>>> = Arc::default();
        let manifest_done: Arc<Mutex<Option<Option<Manifest>>>> = Arc::default();
        let agent = agent();
        for n in 0..DOWNLOADS {
            let (requests, finished, manifest_finished) = (requests.clone(), done.clone(), manifest_done.clone());
            let (wake, agent) = (waker.clone(), agent.clone());
            std::thread::Builder::new()
                .name(format!("warp-art-{n}"))
                .spawn(move || {
                    loop {
                        // The queue is held only while taking from it, so the others wait for
                        // work, not for this one's download. Jobs start in the order they're asked
                        let job = requests.lock().unwrap_or_else(PoisonError::into_inner).recv();
                        let Ok(job) = job else { break };
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
                .expect("couldn't start an art thread");
        }
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
        // From where the manifest says, under the art's base
        let manifest = self.manifest()?;
        let file = art.file(&manifest)?;
        let source = format!("{}\n{}\n{file}", manifest.version(file), warp_art_base());
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

    /// The color of a banner's backdrop, once it's here: the banner's own accent
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

    fn install(&self, Fetched { width, height, rgba }: Fetched) -> Image {
        self.images.rgba(width, height, rgba)
    }
}

/// An accent from a picture's color: its hue, at a lightness and strength that read as an accent
/// on dark and light alike. A gray picture gives a soft gold
fn color(rgb: [f32; 3]) -> Color {
    let [_, a, bb, _] = srgb(rgb, 1.0).to_oklab();
    let chroma = (a * a + bb * bb).sqrt();
    if chroma < 0.02 {
        return rune::oklch(0.72, 0.1, 80.0);
    }
    rune::oklch(0.7, chroma.clamp(0.1, 0.17), bb.atan2(a).to_degrees())
}

/// Where the art is: `RQA_WARP_ART` when it's set, else its R2 bucket
fn warp_art_base() -> String {
    std::env::var(WARP_ART_ENV)
        .ok()
        .filter(|base| !base.trim().is_empty())
        .map_or(WARP_ART_URL.to_string(), |base| base.trim().trim_end_matches(['/', '\\']).to_string())
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

/// A file under the art's base, which is a folder or a URL. From a URL it's asked for with its
/// `version`, when it has one: the bucket's edge cache keeps files a year, and a new version is
/// then a new address rather than a stale copy
fn read_warp_art(agent: &ureq::Agent, base: &str, file: &str, version: Option<&str>) -> Option<Vec<u8>> {
    if base.starts_with("http://") || base.starts_with("https://") {
        match version {
            Some(version) => download(agent, &format!("{base}/{file}?v={version}")),
            None => download(agent, &format!("{base}/{file}")),
        }
    } else {
        std::fs::read(PathBuf::from(base).join(file))
            .inspect_err(|e| tracing::debug!(%e, file, "no art"))
            .ok()
    }
}

/// The warp banner art's manifest, fresh, or as kept last time when it can't be had
fn fetch_manifest(agent: &ureq::Agent) -> Option<Manifest> {
    let kept = cache_dir().map(|dir| dir.join("warp-index.json"));
    // Never cached at the edge, it's how a new version is found out
    let fresh = read_warp_art(agent, &warp_art_base(), "index.json", None);
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

/// The art from the cache folder, or downloaded into it, and for a banner's backdrop its color.
/// `source` is the file's version, the art's base and the file, a line each
fn fetch(agent: &ureq::Agent, art: Art, source: &str) -> (Option<Fetched>, Option<[f32; 3]>) {
    let mut lines = source.lines();
    let (Some(version), Some(base), Some(file)) = (lines.next(), lines.next(), lines.next()) else {
        return (None, None);
    };
    let cached = cache_dir().map(|dir| dir.join(art.cache_name(file, version)));
    let backdrop = matches!(art, Art::Layer(_, Layer::Backdrop));
    if let Some(path) = &cached
        && let Ok(bytes) = std::fs::read(path)
    {
        let _slot = DECODING.take();
        if let Ok(image) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Qoi) {
            let tint = backdrop.then(|| mean_color(&image));
            return (Some(rgba(image)), tint);
        }
    }

    // The version is a hash or a build: hex and dashes, safe in a query
    let version = version.chars().all(|c| c.is_ascii_alphanumeric() || c == '-').then_some(version);
    let Some(bytes) = read_warp_art(agent, base, file, version) else {
        return (None, None);
    };
    // Downloads run side by side freely; from here on it's held until the decoded copy is kept
    let _slot = DECODING.take();
    let Some(image) = decode(&bytes) else {
        return (None, None);
    };
    drop(bytes);
    // Portraits are scaled down to a height the screen can use; the rest are kept as the game has
    // them, banner layers since they're drawn as large as the window
    let scale = match art {
        Art::Portrait(_) => (PORTRAIT_HEIGHT as f32 / image.height() as f32).min(1.0),
        _ => 1.0,
    };
    let image = if scale < 1.0 {
        let w = (image.width() as f32 * scale).round().max(1.0) as u32;
        let h = (image.height() as f32 * scale).round().max(1.0) as u32;
        image.thumbnail_exact(w, h)
    } else {
        image
    };
    let tint = backdrop.then(|| mean_color(&image));
    if let Some(path) = cached {
        let mut qoi = Vec::new();
        if image.write_to(&mut std::io::Cursor::new(&mut qoi), image::ImageFormat::Qoi).is_ok() {
            keep(&path, &qoi);
        }
    }
    (Some(rgba(image)), tint)
}

/// Decodes at once, all art threads together. A banner layer being decoded holds its pixels and
/// the decoder's working copies, tens of MB, and each takes a core: a few at a time, one fewer
/// than the cores so the window keeps one, keeps a cold start's memory and the UI in check
static DECODING: DecodeSlots = DecodeSlots::new();

/// A count of slots taken, up to `decode_slots`
struct DecodeSlots {
    taken: Mutex<usize>,
    freed: Condvar,
}

impl DecodeSlots {
    const fn new() -> Self {
        Self {
            taken: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    /// Waits for a free slot and holds it until the returned guard is dropped
    fn take(&'static self) -> DecodeSlot {
        self.take_of(decode_slots())
    }

    fn take_of(&'static self, limit: usize) -> DecodeSlot {
        let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        while *taken >= limit {
            taken = self.freed.wait(taken).unwrap_or_else(PoisonError::into_inner);
        }
        *taken += 1;
        DecodeSlot(self)
    }
}

struct DecodeSlot(&'static DecodeSlots);

impl Drop for DecodeSlot {
    fn drop(&mut self) {
        *self.0.taken.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
        self.0.freed.notify_one();
    }
}

fn decode_slots() -> usize {
    static SLOTS: OnceLock<usize> = OnceLock::new();
    *SLOTS.get_or_init(|| {
        std::thread::available_parallelism()
            .map_or(2, |cores| cores.get().saturating_sub(1))
            .clamp(2, 8)
    })
}

/// A picture from the art: JPEG XL, or anything else the image crate reads
fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    // A bare codestream, or one in its container
    let jxl = bytes.starts_with(&[0xff, 0x0a]) || bytes.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n");
    if !jxl {
        return image::load_from_memory(bytes)
            .inspect_err(|e| tracing::debug!(%e, "couldn't decode art"))
            .ok();
    }
    let mut image = jxl_oxide::JxlImage::builder()
        .read(bytes)
        .inspect_err(|e| tracing::debug!(%e, "couldn't read art"))
        .ok()?;
    // Lossy art is kept in the XYB color space, it's turned back into what the screen shows
    image.request_color_encoding(jxl_oxide::EnumColourEncoding::srgb(jxl_oxide::RenderingIntent::Relative));
    let render = image
        .render_frame(0)
        .inspect_err(|e| tracing::debug!(%e, "couldn't decode art"))
        .ok()?;
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels() as usize);
    let mut samples = vec![0u8; width as usize * height as usize * channels];
    stream.write_to_buffer(&mut samples);
    let rgba = match channels {
        4 => samples,
        3 => samples.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        2 => samples.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        1 => samples.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        _ => return None,
    };
    image::RgbaImage::from_raw(width, height, rgba).map(DynamicImage::ImageRgba8)
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

/// The image's color, its saturated pixels counting most, as sRGB 0..1
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
    Fetched {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
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

    const MANIFEST: &[u8] = br#"{"game":{"build":"b"},"banners":{"character":{"1511":{"layers":[
            {"layer":"backdrop","file":"characters/1511/backdrop.jxl"},
            {"layer":"avatar","file":"characters/1511/avatar.jxl","canvas":[2048,2048],"rect":[2,191,1958,1715],"canvas_on_screen":[585.8,-161.2,1382.4,1382.4]},
            {"layer":"tab_selected","file":"characters/1511/tab_selected.jxl"}
        ],"panel":{"fill":{"color":[0.16,0.11,0.41,1],"gradient":{"colors":[[0.75,0.01,0,1],[0.2,0,0,1]],"angle":-110}},
            "fade":{"color":[0.06,0.94,1,1]},
            "pattern":{"color":[0.06,0.04,0.16,0.05],"pattern":"patterns/decopatternpixeleater.jxl","tile":[69.33,61.33]}},
          "frame_pattern":{"color":[0,0,0,0.035],"pattern":"patterns/decopatternpixeleatersingle.jxl","tile":[26.67,55.76]}}},
        "light_cone":{"23048":{"layers":[{"layer":"bg","file":"light_cones/23048/bg.jxl"},
            {"layer":"tab","file":"light_cones/23048/tab.jxl"}],
          "frame_pattern":{"color":[0,0,0,0.035],"pattern":"patterns/decopatterncross.jxl","tile":[20,20]}},
          "23049":{"layers":[{"layer":"bg","file":"light_cones/23049/bg.jxl"}]}}},
        "tabs":{"standard":{"tab":"tabs/standard_tab.jxl","tab_selected":"tabs/standard_tab_selected.jxl"}},
        "art":{"character":{"1511":{"portrait":"characters/1511/portrait.jxl","preview":"characters/1511/preview.jxl","icon":"characters/1511/icon.jxl"}},
            "light_cone":{"23048":{"portrait":"light_cones/23048/portrait.jxl","preview":"light_cones/23048/preview.jxl","icon":"light_cones/23048/icon.jxl"}},
            "item":{"102":{"icon":"items/102.jxl"}}},
        "files":{"characters/1511/portrait.jxl":{"bytes":1,"sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
            "items/102.jxl":{"bytes":1,"sha256":"../../evil"}}}"#;

    #[test]
    fn art_comes_from_where_the_manifest_says() {
        let manifest = Manifest::parse(MANIFEST).unwrap();
        let file = |art: Art| art.file(&manifest);
        assert_eq!(file(Art::Portrait(1511)), Some("characters/1511/portrait.jxl"));
        assert_eq!(file(Art::Preview(23048)), Some("light_cones/23048/preview.jxl"));
        assert_eq!(file(Art::Icon(1511)), Some("characters/1511/icon.jxl"));
        assert_eq!(file(Art::Icon(23048)), Some("light_cones/23048/icon.jxl"));
        assert_eq!(file(Art::Item(102)), Some("items/102.jxl"));
        assert_eq!(file(Art::Item(101)), None);
        assert_eq!(file(Art::Layer(1511, Layer::Main)), Some("characters/1511/avatar.jxl"));
        assert_eq!(file(Art::Ticket(Ticket::Banner(1511), true)), Some("characters/1511/tab_selected.jxl"));
        assert_eq!(file(Art::Ticket(Ticket::Standard, false)), Some("tabs/standard_tab.jxl"));
        assert_eq!(file(Art::Pattern(1511)), Some("patterns/decopatternpixeleater.jxl"));
    }

    #[test]
    fn art_is_cached_by_its_hash() {
        let manifest = Manifest::parse(MANIFEST).unwrap();
        let name = |art: Art| {
            let file = art.file(&manifest).unwrap();
            art.cache_name(file, manifest.version(file))
        };
        assert_eq!(name(Art::Portrait(1511)), "warp-0123456789abcdef-characters-1511-portrait-2048.qoi");
        // Without a hash, by the game build
        assert_eq!(name(Art::Layer(1511, Layer::Main)), "warp-b-characters-1511-avatar.qoi");
        // Nothing from the manifest reaches outside the cache folder
        assert_eq!(name(Art::Item(102)), "warp-------evil-items-102.qoi");
    }

    #[test]
    fn decodes_wait_for_a_free_slot() {
        static SLOTS: DecodeSlots = DecodeSlots::new();
        let (now, most) = (Arc::new(Mutex::new(0usize)), Arc::new(Mutex::new(0usize)));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (now, most) = (now.clone(), most.clone());
                std::thread::spawn(move || {
                    let _slot = SLOTS.take_of(2);
                    let mut n = now.lock().unwrap();
                    *n += 1;
                    let mut m = most.lock().unwrap();
                    *m = (*m).max(*n);
                    drop((n, m));
                    std::thread::sleep(Duration::from_millis(20));
                    *now.lock().unwrap() -= 1;
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(*most.lock().unwrap(), 2);
        assert_eq!(*SLOTS.taken.lock().unwrap(), 0);
        assert!((2..=8).contains(&decode_slots()));
    }

    #[test]
    fn jpeg_xl_decodes() {
        // 2x2, lossless: red, half transparent green, transparent, white
        const TINY: &[u8] = &[
            0xff, 0x0a, 0x08, 0x10, 0xb0, 0x12, 0x08, 0x00, 0x10, 0x00, 0x74, 0x00, 0x4b, 0x18, 0x8b, 0x15, 0xc2, 0x09,
            0xaa, 0xb7, 0x1f, 0x34, 0x17, 0x90, 0x01, 0x60, 0xfc, 0x47, 0x44, 0x3f, 0x9e, 0xde, 0x6d, 0x22, 0xc1, 0x8a,
            0xbd, 0x77, 0x98, 0x2a, 0x03,
        ];
        let image = decode(TINY).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 255, 0, 128]);
        assert_eq!(image.get_pixel(0, 1).0[3], 0);
        assert_eq!(image.get_pixel(1, 1).0, [255, 255, 255, 255]);
    }

    #[test]
    fn the_banner_art_manifest_reads() {
        let manifest = Manifest::parse(MANIFEST).unwrap();
        let banner = &manifest.banners[&1511];
        assert_eq!(banner.layer(Layer::Main).unwrap().rect, [2.0, 191.0, 1958.0, 1715.0]);
        assert!(banner.layer(Layer::Backdrop).unwrap().on_screen.is_none());
        // Without its main layer placed, only a banner's tickets are kept
        let cone = &manifest.banners[&23048];
        assert!(!cone.placed());
        assert!(cone.layer(Layer::Back).is_none() && cone.layer(Layer::Tab).is_some());
        // A character's panel: its colors in linear light, a gradient's in place of the layer's
        // own, and the panel's own motif
        let panel = banner.surface.as_ref().unwrap();
        let fill = panel.fill.unwrap();
        assert!((fill.start.r - 0.522).abs() < 0.01 && (fill.end.r - 0.033).abs() < 0.01, "{fill:?}");
        let fade = panel.fade.unwrap();
        assert_eq!(fade.start, fade.end);
        assert_eq!(panel.pattern.as_ref().unwrap().file, "patterns/decopatternpixeleater.jxl");
        assert_eq!(panel.pattern.as_ref().unwrap().tile, [69.33, 61.33]);
        // A light cone's has only the frame's motif; a banner with neither has none, and with
        // nothing else of use isn't kept
        let cone_panel = cone.surface.as_ref().unwrap();
        assert!(cone_panel.fill.is_none() && cone_panel.pattern.as_ref().unwrap().tile == [20.0, 20.0]);
        assert!(!manifest.banners.contains_key(&23049));
        assert_eq!(
            manifest.tickets[&Ticket::Standard][1].as_deref(),
            Some("tabs/standard_tab_selected.jxl")
        );
    }
}
