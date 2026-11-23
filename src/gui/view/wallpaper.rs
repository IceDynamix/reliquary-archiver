//! The background image where the settings place it (see [`background::place`]), and what fills
//! the window around it when it's smaller than the window: drawn at the window's size behind
//! everything, and scaled down in the settings to preview it.
//!
//! [`background::place`]: crate::gui::background::place

use rune::{Color, El, Pixels, Wrap, el};

use crate::gui::settings::BackgroundFill;

/// How blurred the image behind is for [`BackgroundFill::Blur`], in points
const BLUR: f32 = 28.0;
/// [`BackgroundFill::MirrorBlur`]: the reflections blurred this much far from the image, and
/// sharper in bands nearer it, each reaching this far past the image (points) with this blur,
/// fading out over the outer part of its reach
const MIRROR_FAR: f32 = 32.0;
const MIRROR_BANDS: [(f32, f32); 3] = [(380.0, 16.0), (200.0, 7.0), (90.0, 2.0)];
const BAND_FADE: f32 = 0.75;

/// What draws the image and its fill
pub struct Wallpaper<'a> {
    pub pixels: &'a Pixels,
    /// Where the image goes, `[x, y, w, h]` in points from the window's top-left
    pub rect: [f32; 4],
    /// Where the image covering the whole window would go, for [`BackgroundFill::Blur`]
    pub cover: [f32; 4],
    pub window: [f32; 2],
    pub fill: BackgroundFill,
    /// The image's edge colors, top, right, bottom, left
    pub edges: Option<[Color; 4]>,
    /// What [`BackgroundFill::Color`] fills with
    pub color: Color,
    /// Drawn this much smaller than the window it stands for, which blurs scale with
    pub scale: f32,
    /// What the blurred mirror's bands reach scales with: `scale`, but less in a preview too
    /// small for them to show at that
    pub reach: f32,
}

impl Wallpaper<'_> {
    /// The image and its fill over the whole of a window-sized element
    pub fn build(self) -> El {
        let [x, y, w, h] = self.rect;
        let [ww, wh] = self.window;
        let mut layer = el().w(ww).h(wh).clip_rounded(0.0);
        // Nothing to fill when the image reaches every edge
        let covers = x <= 0.5 && y <= 0.5 && x + w >= ww - 0.5 && y + h >= wh - 0.5;
        if !covers {
            layer = self.fill(layer);
        }
        layer.child(el().pin(Some(x), Some(y), None, None).size(w, h).pixels(self.pixels))
    }

    fn fill(&self, layer: El) -> El {
        let k = self.scale;
        match self.fill {
            BackgroundFill::None => layer,
            BackgroundFill::Blur => {
                // Reaching past the window by three blurs, so its edges don't fade to clear
                let over = 3.0 * BLUR * k;
                let [x, y, w, h] = self.cover;
                layer.child(
                    el().pin(Some(x - over), Some(y - over), None, None)
                        .size(w + 2.0 * over, h + 2.0 * over)
                        .pixels(self.pixels)
                        .blur(BLUR * k),
                )
            }
            BackgroundFill::Color => layer.child(el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(self.color)),
            BackgroundFill::EdgeColor => {
                let Some([top, right, bottom, left]) = self.edges else {
                    return layer;
                };
                let [x, y, w, h] = self.rect;
                let [ww, wh] = self.window;
                let block = |at: [f32; 4], color: Color| {
                    el().pin(Some(at[0]), Some(at[1]), None, None)
                        .size(at[2].max(0.0), at[3].max(0.0))
                        .fill(color)
                };
                layer
                    .child(block([0.0, 0.0, x, wh], left))
                    .child(block([x + w, 0.0, ww - x - w, wh], right))
                    .child(block([x, 0.0, w, y], top))
                    .child(block([x, y + h, w, wh - y - h], bottom))
            }
            BackgroundFill::Extend => layer.child(self.wrapped(Wrap::Extend, [0.0, 0.0], 0.0)),
            BackgroundFill::Mirror => layer.child(self.wrapped(Wrap::Mirror, [0.0, 0.0], 0.0)),
            BackgroundFill::Repeat => layer.child(self.wrapped(Wrap::Repeat, [0.0, 0.0], 0.0)),
            BackgroundFill::MirrorBlur => {
                // The blurriest reflections over the whole window, then sharper ones in bands
                // nearer the image, each fading out towards its own edge so the blur grows
                // smoothly with the distance from the image
                let mut layer = layer.child(self.wrapped(Wrap::Mirror, [0.0, 0.0], MIRROR_FAR * k));
                let [x, y, w, h] = self.rect;
                for (reach, blur) in MIRROR_BANDS {
                    let reach = reach * self.reach;
                    let band = [x - reach, y - reach, w + 2.0 * reach, h + 2.0 * reach];
                    let fade = reach * BAND_FADE;
                    layer = layer.child(
                        el().pin(Some(band[0]), Some(band[1]), None, None)
                            .size(band[2], band[3])
                            .clip_fade(fade, fade, fade, fade)
                            .child(self.wrapped(Wrap::Mirror, [band[0], band[1]], blur * k)),
                    );
                }
                layer
            }
        }
    }

    /// The image wrapped across the window (`El::pixels_wrap`, our Rune patch: anomalyco/rune#1432), in an element whose parent starts at `origin` in the
    /// window, blurred by `blur` (and reaching past the window by three blurs, so its edges don't
    /// fade to clear)
    fn wrapped(&self, wrap: Wrap, origin: [f32; 2], blur: f32) -> El {
        let over = 3.0 * blur;
        let [x, y, w, h] = self.rect;
        let [ww, wh] = self.window;
        let at = [-origin[0] - over, -origin[1] - over];
        el().pin(Some(at[0]), Some(at[1]), None, None)
            .size(ww + 2.0 * over, wh + 2.0 * over)
            .pixels_wrap(self.pixels, [x + over, y + over, w, h], wrap)
            .blur(blur)
    }
}
