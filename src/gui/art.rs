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
//! while it's on its way. Worker threads download, several at once, and decode, a few at once,
//! then wake the window, and [`ArtCache::take_done`] (called from `poll`) hands the result to Rune.
//!
//! Decoded, the warp screen's art is hundreds of MB, so only what's on screen or about to be is
//! held. The rest is fetched ahead into the cache folder with [`ArtCache::warm`], from where it
//! reads in a moment, and [`ArtCache::release`] lets go of what's no longer needed.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use image::DynamicImage;
use rune::{Color, Image, ImageCache};
use serde_json::Value;

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
        let height = if matches!(self, Art::Portrait(_)) {
            format!("-{PORTRAIT_HEIGHT}")
        } else {
            String::new()
        };
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

    /// A banner's entry in the manifest, `None` when nothing of it is of use
    fn parse(entry: &Value) -> Option<BannerArt> {
        let mut layers: Vec<LayerArt> = entry["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(LayerArt::parse)
            .collect();
        // Without its main layer placed, a banner's art isn't worth drawing in layers, but its
        // tickets still are
        if !layers.iter().any(|l| l.layer == Layer::Main && l.on_screen.is_some()) {
            layers.retain(|l| matches!(l.layer, Layer::Tab | Layer::TabSelected));
        }
        let surface = Surface::parse(entry);
        (!layers.is_empty() || surface.is_some()).then_some(BannerArt { layers, surface })
    }
}

impl LayerArt {
    /// A layer's entry in a banner's
    fn parse(l: &Value) -> Option<LayerArt> {
        let canvas = numbers(&l["canvas"]).unwrap_or([2048.0, 2048.0]);
        let weight = &l["weight"];
        Some(LayerArt {
            layer: Layer::from_name(l["layer"].as_str()?)?,
            file: string(&l["file"])?,
            canvas,
            rect: numbers(&l["rect"]).unwrap_or([0.0, 0.0, canvas[0], canvas[1]]),
            on_screen: numbers::<4>(&l["canvas_on_screen"]).filter(|v| v[2] > 0.0),
            weight: numbers(&weight["centre"]).zip(rect(&weight["core"])).map(|(center, core)| Weight {
                center,
                core,
                person: rect(&weight["person"]),
                head: rect(&weight["head"]),
            }),
        })
    }
}

/// `N` numbers, from a JSON array of exactly that many
fn numbers<const N: usize>(v: &Value) -> Option<[f32; N]> {
    let floats: Vec<f32> = v.as_array()?.iter().map(|x| x.as_f64().map(|x| x as f32)).collect::<Option<_>>()?;
    floats.try_into().ok()
}

/// A box, x, y, width and height, that has an area
fn rect(v: &Value) -> Option<[f32; 4]> {
    numbers::<4>(v).filter(|r| r[2] > 0.0 && r[3] > 0.0)
}

fn string(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// The entries of a JSON object keyed by id, skipping keys that aren't one
fn by_id(v: &Value) -> impl Iterator<Item = (u32, &Value)> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(id, entry)| Some((id.parse().ok()?, entry)))
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

    /// The names everything it has is kept under in the cache folder ([`Art::cache_name`])
    fn cache_names(&self) -> HashSet<String> {
        let own = self.own.values().flat_map(|own| [&own.portrait, &own.preview, &own.icon]).flatten();
        let banners = self.banners.values().flat_map(|banner| {
            let pattern = banner.surface.as_ref().and_then(|s| s.pattern.as_ref()).map(|p| &p.file);
            banner.layers.iter().map(|l| &l.file).chain(pattern)
        });
        let tickets = self.tickets.values().flatten().flatten();
        own.chain(banners)
            .chain(tickets)
            .chain(self.items.values())
            // A portrait's copy is named for its height, anything else's isn't: either may be kept
            .flat_map(|file| {
                let version = self.version(file);
                [Art::Portrait(0), Art::Item(0)].map(|art| art.cache_name(file, version))
            })
            .collect()
    }

    fn parse(bytes: &[u8]) -> Option<Manifest> {
        let json: Value = serde_json::from_slice(bytes).ok()?;
        let build = json["game"]["build"].as_str().unwrap_or("0").to_string();
        let banners = ["character", "light_cone", "collaboration_character", "collaboration_light_cone"]
            .into_iter()
            .flat_map(|kind| by_id(&json["banners"][kind]))
            .filter_map(|(id, entry)| Some((id, BannerArt::parse(entry)?)))
            .collect();
        let mut tickets = HashMap::new();
        for (name, ticket) in [("standard", Ticket::Standard), ("departure", Ticket::Departure)] {
            let tabs = &json["tabs"][name];
            if tabs.is_object() {
                tickets.insert(ticket, [string(&tabs["tab"]), string(&tabs["tab_selected"])]);
            }
        }
        let own = ["character", "light_cone"]
            .into_iter()
            .flat_map(|kind| by_id(&json["art"][kind]))
            .map(|(id, slots)| {
                (id, OwnArt {
                    portrait: string(&slots["portrait"]),
                    preview: string(&slots["preview"]),
                    icon: string(&slots["icon"]),
                })
            })
            .collect();
        let items = by_id(&json["art"]["item"])
            .filter_map(|(id, slots)| Some((id, string(&slots["icon"])?)))
            .collect();
        let hashes = json["files"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(file, about)| Some((file.clone(), string(&about["sha256"])?)))
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
    fn parse(entry: &Value) -> Option<Surface> {
        let panel = &entry["panel"];
        let frame_pattern = || Pattern::parse(&entry["frame_pattern"]);
        let surface = if panel.is_object() {
            Surface {
                fill: Paint::parse(&panel["fill"]),
                fade: Paint::parse(&panel["fade"]),
                pattern: Pattern::parse(&panel["pattern"]).or_else(frame_pattern),
            }
        } else {
            Surface {
                fill: None,
                fade: None,
                pattern: frame_pattern(),
            }
        };
        (surface.fill.is_some() || surface.pattern.is_some()).then_some(surface)
    }
}

impl Paint {
    /// A panel layer's paint: a gradient's colors take the place of the layer's own
    fn parse(v: &Value) -> Option<Paint> {
        let stops = v["gradient"]["colors"].as_array();
        match stops.map(|s| s.iter().map(game_color).collect::<Option<Vec<_>>>()) {
            Some(Some(stops)) if stops.len() == 2 => Some(Paint {
                start: stops[0],
                end: stops[1],
            }),
            _ => game_color(&v["color"]).map(|c| Paint { start: c, end: c }),
        }
    }
}

impl Pattern {
    fn parse(v: &Value) -> Option<Pattern> {
        let tile = numbers::<2>(&v["tile"]).filter(|[w, h]| *w > 0.0 && *h > 0.0)?;
        Some(Pattern {
            file: string(&v["pattern"])?,
            color: game_color(&v["color"])?,
            tile,
        })
    }
}

/// A color from the manifest, sRGB and alpha
fn game_color(v: &Value) -> Option<Color> {
    numbers::<4>(v).map(|[r, g, b, a]| srgb([r, g, b], a))
}

/// A color as the game's UI gives it, in sRGB
fn srgb(rgb: [f32; 3], alpha: f32) -> Color {
    let [r, g, b] = rgb.map(super::srgb_to_linear);
    Color { r, g, b, a: alpha }
}

enum Slot {
    Loading,
    /// Handed to Rune, which may still be preparing it to draw ([`Image::is_ready`])
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

/// Where a piece of art is read from
struct Source {
    /// The file's version, as the manifest has it ([`Manifest::version`])
    version: String,
    /// The art's base, a folder or a URL
    base: String,
    file: String,
}

enum Job {
    Manifest,
    Art(Art, Source),
    /// Into the cache folder only, for later
    Warm(Art, Source),
}

/// The art threads' work: what the window waits on first, then what's fetched ahead
#[derive(Default)]
struct Queue {
    jobs: Mutex<Jobs>,
    added: Condvar,
}

#[derive(Default)]
struct Jobs {
    now: VecDeque<Job>,
    later: VecDeque<Job>,
    /// The cache is gone, the threads end
    closed: bool,
}

impl Queue {
    fn push(&self, job: Job) {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        match job {
            Job::Warm(..) => jobs.later.push_back(job),
            _ => jobs.now.push_back(job),
        }
        drop(jobs);
        self.added.notify_one();
    }

    /// The next job, in the order asked, waiting for one. `None` once the cache is gone
    fn pop(&self) -> Option<Job> {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if jobs.closed {
                return None;
            }
            if let Some(job) = jobs.now.pop_front().or_else(|| jobs.later.pop_front()) {
                return Some(job);
            }
            jobs = self.added.wait(jobs).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn close(&self) {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner).closed = true;
        self.added.notify_all();
    }
}

/// What an art thread finished
enum Finished {
    /// The manifest, `None` when it couldn't be had
    Manifest(Option<Manifest>),
    Art(Done),
}

enum ManifestState {
    NotAsked,
    Loading,
    Ready(Arc<Manifest>),
    Missing,
}

/// Art for the window. Asked for and handed to Rune on the UI thread; only `done` is shared with
/// the art threads
pub struct ArtCache {
    images: ImageCache,
    slots: RefCell<HashMap<Art, Slot>>,
    tints: RefCell<HashMap<Art, Color>>,
    manifest: RefCell<ManifestState>,
    /// Where requests go, `None` to fetch on the spot (for snapshots, which draw one frame)
    jobs: Option<Arc<Queue>>,
    /// Art asked to be fetched ahead this run, which isn't asked for again
    warmed: RefCell<HashSet<Art>>,
    /// Art handed to Rune that it's still preparing, which the window hears of once it's ready
    preparing: RefCell<Vec<Image>>,
    /// What the art threads finished, for [`ArtCache::take_done`]
    done: Arc<Mutex<Vec<Finished>>>,
}

impl Drop for ArtCache {
    fn drop(&mut self) {
        if let Some(jobs) = &self.jobs {
            jobs.close();
        }
    }
}

impl ArtCache {
    /// Fetches on threads of its own, `DOWNLOADS` at once and decoding a few of those at a time
    /// (`DECODING`), and wakes the window with `waker` when art arrives
    pub fn new(waker: rune::Waker) -> Self {
        let jobs = Arc::new(Queue::default());
        let done: Arc<Mutex<Vec<Finished>>> = Arc::default();
        let agent = agent();
        for n in 0..DOWNLOADS {
            let (jobs, done) = (jobs.clone(), done.clone());
            let (wake, agent) = (waker.clone(), agent.clone());
            std::thread::Builder::new()
                .name(format!("warp-art-{n}"))
                .spawn(move || {
                    // The queue is held only while taking from it, so the others wait for work,
                    // not for this one's download. Jobs start in the order they're asked, what
                    // the window waits on before what's fetched ahead
                    while let Some(job) = jobs.pop() {
                        let finished = match job {
                            Job::Manifest => Finished::Manifest(fetch_manifest(&agent)),
                            Job::Art(art, source) => {
                                let (fetched, tint) = fetch(&agent, art, &source, Want::Pixels);
                                Finished::Art(Done { art, fetched, tint })
                            }
                            Job::Warm(art, source) => {
                                fetch(&agent, art, &source, Want::Kept);
                                continue;
                            }
                        };
                        done.lock().unwrap_or_else(PoisonError::into_inner).push(finished);
                        wake();
                    }
                })
                .expect("couldn't start an art thread");
        }
        Self::with(waker, Some(jobs), done)
    }

    /// Fetches when asked, for snapshots
    pub fn blocking(waker: rune::Waker) -> Self {
        Self::with(waker, None, Arc::default())
    }

    fn with(waker: rune::Waker, jobs: Option<Arc<Queue>>, done: Arc<Mutex<Vec<Finished>>>) -> Self {
        Self {
            images: ImageCache::new(waker),
            slots: RefCell::default(),
            tints: RefCell::default(),
            manifest: RefCell::new(ManifestState::NotAsked),
            jobs,
            warmed: RefCell::default(),
            preparing: RefCell::default(),
            done,
        }
    }

    /// How a banner featuring `item` lays its art out, when the banner art has it. `None` while
    /// that's being found out
    pub fn banner(&self, item: u32) -> Option<BannerArt> {
        self.with_banner(item, BannerArt::clone)
    }

    /// What `look` makes of how a banner featuring `item` lays its art out, as [`ArtCache::banner`]
    /// without a copy of it
    pub fn with_banner<T>(&self, item: u32, look: impl FnOnce(&BannerArt) -> T) -> Option<T> {
        self.manifest()?.banners.get(&item).map(look)
    }

    fn manifest(&self) -> Option<Arc<Manifest>> {
        let mut state = self.manifest.borrow_mut();
        match &*state {
            ManifestState::Ready(m) => return Some(m.clone()),
            ManifestState::Loading | ManifestState::Missing => return None,
            ManifestState::NotAsked => {}
        }
        match &self.jobs {
            Some(jobs) => {
                *state = ManifestState::Loading;
                jobs.push(Job::Manifest);
                None
            }
            None => {
                let manifest = fetch_manifest(&agent()).map(Arc::new);
                *state = manifest.clone().map_or(ManifestState::Missing, ManifestState::Ready);
                manifest
            }
        }
    }

    /// The art if it's here and ready to draw, `None` while it's on its way or when it couldn't be
    /// had. Asks for it when it isn't
    pub fn get(&self, art: Art) -> Option<Image> {
        {
            let slots = self.slots.borrow();
            match slots.get(&art) {
                Some(Slot::Ready(image)) => return (image.is_ready() && image.error().is_none()).then(|| image.clone()),
                Some(Slot::Loading | Slot::Missing) => return None,
                None => {}
            }
        }
        let source = self.source(art)?;
        let mut slots = self.slots.borrow_mut();
        match &self.jobs {
            Some(jobs) => {
                slots.insert(art, Slot::Loading);
                jobs.push(Job::Art(art, source));
                None
            }
            None => {
                let (fetched, tint) = fetch(&agent(), art, &source, Want::Pixels);
                let image = fetched.map(|fetched| self.install(fetched));
                if let Some(image) = &image {
                    let _ = image.wait();
                }
                if let Some(tint) = tint {
                    self.tints.borrow_mut().insert(art, accent_of_rgb(tint));
                }
                slots.insert(art, image.clone().map_or(Slot::Missing, Slot::Ready));
                image
            }
        }
    }

    /// Whether the art is on its way: asked for and not yet ready to draw. Art that couldn't be
    /// had isn't coming
    pub fn coming(&self, art: Art) -> bool {
        match self.slots.borrow().get(&art) {
            Some(Slot::Loading) => true,
            Some(Slot::Ready(image)) => !image.is_ready(),
            Some(Slot::Missing) | None => false,
        }
    }

    /// Fetches the art into the cache folder without keeping it, so that asking for it later
    /// reads it from there rather than waiting on a download and a JPEG XL decode. Runs after
    /// everything [`ArtCache::get`] asked for
    pub fn warm(&self, art: Art) {
        let Some(jobs) = &self.jobs else { return };
        if self.slots.borrow().contains_key(&art) || self.warmed.borrow().contains(&art) {
            return;
        }
        let Some(source) = self.source(art) else { return };
        self.warmed.borrow_mut().insert(art);
        jobs.push(Job::Warm(art, source));
    }

    /// Lets go of the art `keep` doesn't want, and Rune of its copies once nothing draws them.
    /// What's still on its way is let go of when it arrives. Asking for it again reads it from the
    /// cache folder
    pub fn release(&self, keep: impl Fn(&Art) -> bool) {
        let mut slots = self.slots.borrow_mut();
        let mut warmed = self.warmed.borrow_mut();
        slots.retain(|art, slot| {
            let kept = keep(art) || matches!(slot, Slot::Missing);
            if !kept {
                // Its download is in the cache folder, or on its way there
                warmed.insert(*art);
            }
            kept
        });
        let held: HashSet<rune::ImageId> = slots
            .values()
            .filter_map(|slot| match slot {
                Slot::Ready(image) => Some(image.id()),
                _ => None,
            })
            .collect();
        self.preparing.borrow_mut().retain(|image| held.contains(&image.id()));
    }

    /// Where the art is read from, as the manifest says
    fn source(&self, art: Art) -> Option<Source> {
        let manifest = self.manifest()?;
        let file = art.file(&manifest)?;
        Some(Source {
            version: manifest.version(file).to_string(),
            base: warp_art_base(),
            file: file.to_string(),
        })
    }

    /// The color of a banner's backdrop, once it's here: the banner's own accent
    pub fn tint(&self, item: u32) -> Option<Color> {
        self.tints.borrow().get(&Art::Layer(item, Layer::Backdrop)).copied()
    }

    /// Hands what the threads finished to Rune. True when there was any, or when art Rune was
    /// preparing became ready to draw
    pub fn take_done(&self) -> bool {
        let prepared = {
            let mut preparing = self.preparing.borrow_mut();
            let before = preparing.len();
            preparing.retain(|image| !image.is_ready());
            preparing.len() < before
        };
        let done = std::mem::take(&mut *self.done.lock().unwrap_or_else(PoisonError::into_inner));
        if done.is_empty() {
            return prepared;
        }
        let (mut slots, mut tints) = (self.slots.borrow_mut(), self.tints.borrow_mut());
        for finished in done {
            match finished {
                Finished::Manifest(manifest) => {
                    *self.manifest.borrow_mut() = manifest.map_or(ManifestState::Missing, |m| ManifestState::Ready(Arc::new(m)));
                }
                Finished::Art(Done { art, fetched, tint }) => {
                    if let Some(tint) = tint {
                        tints.insert(art, accent_of_rgb(tint));
                    }
                    // Released while it loaded
                    if !matches!(slots.get(&art), Some(Slot::Loading)) {
                        continue;
                    }
                    let slot = match fetched {
                        Some(fetched) => {
                            let image = self.install(fetched);
                            if !image.is_ready() {
                                self.preparing.borrow_mut().push(image.clone());
                            }
                            Slot::Ready(image)
                        }
                        None => Slot::Missing,
                    };
                    slots.insert(art, slot);
                }
            }
        }
        true
    }

    fn install(&self, Fetched { width, height, rgba }: Fetched) -> Image {
        self.images.rgba(width, height, rgba)
    }
}

/// An accent from a picture's color: its hue, at a lightness and strength that read as an accent
/// on dark and light alike. A gray picture gives a soft gold
fn accent_of_rgb(rgb: [f32; 3]) -> Color {
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
        .map_or(WARP_ART_URL.to_string(), |base| {
            base.trim().trim_end_matches(['/', '\\']).to_string()
        })
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
    let is_fresh = fresh.is_some();
    let bytes = match fresh {
        Some(bytes) => {
            if let Some(path) = &kept {
                keep(path, &bytes);
            }
            bytes
        }
        None => std::fs::read(kept?).ok()?,
    };
    let manifest = Manifest::parse(&bytes)?;
    // Art from `RQA_WARP_ART` shares the cache folder: its manifest doesn't name the bucket's art
    if is_fresh
        && warp_art_base() == WARP_ART_URL
        && let Some(dir) = cache_dir()
    {
        prune(&dir, &manifest);
    }
    Some(manifest)
}

/// Removes the art in the cache folder that `manifest` doesn't name: older versions of what it
/// has, and what it no longer has
fn prune(dir: &std::path::Path, manifest: &Manifest) {
    let wanted = manifest.cache_names();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with("warp-") && name.ends_with(".qoi") && !wanted.contains(name) {
            if std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        tracing::debug!(removed, "removed old art from the cache");
    }
}

/// What [`fetch`] is for
#[derive(Clone, Copy, PartialEq)]
enum Want {
    /// The art's pixels, to draw
    Pixels,
    /// Only the art in the cache folder: nothing is read when it's there already
    Kept,
}

/// The art from the cache folder, or downloaded into it, and for a banner's backdrop its color.
/// For [`Want::Kept`], only into the cache folder, and nothing comes back
fn fetch(agent: &ureq::Agent, art: Art, source: &Source, want: Want) -> (Option<Fetched>, Option<[f32; 3]>) {
    let Source { version, base, file } = source;
    let (version, file) = (version.as_str(), file.as_str());
    let cached = cache_dir().map(|dir| dir.join(art.cache_name(file, version)));
    let backdrop = matches!(art, Art::Layer(_, Layer::Backdrop));
    if want == Want::Kept && cached.as_ref().is_some_and(|path| path.exists()) {
        return (None, None);
    }
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
    if let Some(path) = cached {
        let mut qoi = Vec::new();
        if image.write_to(&mut std::io::Cursor::new(&mut qoi), image::ImageFormat::Qoi).is_ok() {
            keep(&path, &qoi);
        }
    }
    if want == Want::Kept {
        return (None, None);
    }
    let tint = backdrop.then(|| mean_color(&image));
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

fn keep(path: &std::path::Path, bytes: &[u8]) {
    if let Err(e) = crate::dirs::write_atomic(path, |temp| std::fs::write(temp, bytes)) {
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

/// Where downloaded art is kept
fn cache_dir() -> Option<PathBuf> {
    crate::dirs::cache_dir().map(|dir| dir.join("art"))
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
        assert_eq!(
            file(Art::Ticket(Ticket::Banner(1511), true)),
            Some("characters/1511/tab_selected.jxl")
        );
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
    fn only_art_the_manifest_names_is_kept() {
        let manifest = Manifest::parse(MANIFEST).unwrap();
        let dir = std::env::temp_dir().join(format!("rqa-art-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let named = Art::Portrait(1511).cache_name("characters/1511/portrait.jxl", manifest.version("characters/1511/portrait.jxl"));
        let layer = Art::Layer(1511, Layer::Main).cache_name("characters/1511/avatar.jxl", "b");
        for name in [
            named.as_str(),
            layer.as_str(),
            "warp-0ld0ld0ld0ld0ld0-characters-1511-portrait-2048.qoi",
            "other.qoi",
        ] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        prune(&dir, &manifest);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        std::fs::remove_dir_all(&dir).unwrap();
        // An older version goes; what isn't the art's own is left alone
        let mut kept = vec!["other.qoi".to_string(), layer, named];
        kept.sort();
        assert_eq!(left, kept);
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
            0xff, 0x0a, 0x08, 0x10, 0xb0, 0x12, 0x08, 0x00, 0x10, 0x00, 0x74, 0x00, 0x4b, 0x18, 0x8b, 0x15, 0xc2, 0x09, 0xaa, 0xb7, 0x1f,
            0x34, 0x17, 0x90, 0x01, 0x60, 0xfc, 0x47, 0x44, 0x3f, 0x9e, 0xde, 0x6d, 0x22, 0xc1, 0x8a, 0xbd, 0x77, 0x98, 0x2a, 0x03,
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
