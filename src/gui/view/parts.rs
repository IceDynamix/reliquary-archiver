//! The pieces the window is built from: plates with cut corners and gold hairlines, tabs that sit
//! on a plate's edge, pill and ring buttons, the node track, and the astrolabe behind it all.
//!
//! Plates draw their outline from the size they laid out at last (`Cx::layout_size`), which Rune
//! settles before the first frame, so a plate never shows without its corners.

use rune::{Color, Cx, El, Icon, LineCap, Shape, Tx, col, el, icon, row, text};

use crate::gui::state::Ui;
use crate::gui::theme::{self, Theme};

/// How much a plate's corners are cut
const CUT: f32 = 12.0;
/// How far the tile set's back shows under a plate
const BACK: f32 = 4.0;

/// A panel with its top left and bottom right corners cut, edged with a hairline, a short accent
/// mark along the top edge, and the back of the tile set showing under its bottom edge
pub fn plate(key: &str, cx: &Cx<Ui>, t: &Theme) -> El {
    let plate = col().key(key);
    let Some([w, h]) = cx.layout_size(key) else {
        return plate.fill(t.panel);
    };
    let outline = vec![[CUT, 0.0], [w, 0.0], [w, h - CUT], [w - CUT, h], [0.0, h], [0.0, CUT]];
    let mark = 40.0_f32.min(w / 3.0);
    // The bottom edge, moved down by `top` and `bottom`: a strip of a layer under the face
    let strip = |top: f32, bottom: f32| {
        vec![
            [0.0, h + top],
            [w - CUT, h + top],
            [w, h - CUT + top],
            [w, h - CUT + bottom],
            [w - CUT, h + bottom],
            [0.0, h + bottom],
        ]
    };
    plate.shape(
        Shape::new()
            .fill(strip(0.0, BACK), t.tile_back)
            .fill(strip(BACK - 1.0, BACK), t.tile_back_edge)
            .fill(outline.clone(), t.panel)
            .stroke_with(outline, true, 1.0, LineCap::Butt, t.line)
            .line([CUT, 0.75], [CUT + mark, 0.75], 1.5, LineCap::Butt, t.accent)
            .line([w - CUT - mark, h - 0.75], [w - CUT, h - 0.75], 1.5, LineCap::Butt, t.accent),
    )
}

/// A label on a plate's top edge, cut at its right end, like a file tab. Put it first in the plate
pub fn tab(key: &str, label: &str, tint: Color, cx: &Cx<Ui>, t: &Theme) -> El {
    let tab = row()
        .key(key)
        .h(22.0)
        .pl(12.0)
        .pr(20.0)
        .center_y()
        .mt(-11.0)
        .ml(CUT + 8.0)
        .self_start();
    let tab = match cx.layout_size(key) {
        Some([w, h]) => tab.shape(Shape::new().fill(vec![[0.0, 0.0], [w - 8.0, 0.0], [w, h], [0.0, h]], tint)),
        None => tab.fill(tint),
    };
    tab.child(text(label.to_uppercase()).font(theme::KICKER).color(t.on_accent))
}

/// How a pill button paints
#[derive(Clone, Copy, PartialEq)]
pub enum Pill {
    /// The thing to do next: cream, dark text
    Primary,
    /// Dark with a hairline edge
    Secondary,
    /// Just gold text
    Quiet,
}

/// A rounded button. Without `enabled` it dims and ignores presses
pub fn pill(
    key: &str,
    label: &str,
    glyph: Option<&'static Icon>,
    kind: Pill,
    enabled: bool,
    t: &Theme,
    on_press: impl Fn(&mut Ui, &mut Tx) + 'static,
) -> El {
    pill_for(key, label, glyph, kind, enabled, t, on_press)
}

/// [`pill`] for a window whose state isn't the app's, like the test controller
pub fn pill_for<S: 'static>(
    key: &str,
    label: &str,
    glyph: Option<&'static Icon>,
    kind: Pill,
    enabled: bool,
    t: &Theme,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let (fill, edge, ink, hover) = match kind {
        Pill::Primary => (t.primary, t.primary, t.on_primary, t.on_primary.alpha(0.1)),
        Pill::Secondary => (t.well, t.accent_line, t.text, t.accent.alpha(0.12)),
        Pill::Quiet => (Color::TRANSPARENT, Color::TRANSPARENT, t.accent, t.accent.alpha(0.1)),
    };
    let mut pill = row()
        .key(key)
        .role("button")
        .label(label)
        .h(32.0)
        .px(if kind == Pill::Quiet { 12.0 } else { 18.0 })
        .gap(7.0)
        .center()
        .radius(16.0)
        .fill(fill)
        .border(1.0, edge)
        .children(glyph.map(|glyph| icon(glyph, 15.0).color(ink)))
        .child(text(label).font(theme::BUTTON).color(ink));
    if enabled {
        pill = pill
            .focusable(true)
            .hover(hover)
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(on_press);
    } else {
        pill = pill.disabled(true).opacity(0.38);
    }
    pill
}

/// A round icon button inside a hairline ring
pub fn ring_button(key: &str, glyph: &'static Icon, name: &str, active: bool, t: &Theme, on_press: impl Fn(&mut Ui) + 'static) -> El {
    el().key(key)
        .role("button")
        .label(name)
        .help(name)
        .expanded(active)
        .focusable(true)
        .size(34.0, 34.0)
        .radius(17.0)
        .center()
        .fill(if active { t.accent.alpha(0.16) } else { t.well })
        .border(1.0, if active { t.accent } else { t.accent_line })
        .hover(t.accent.alpha(0.14))
        .cursor(rune::Cursor::Pointer)
        .on_press(on_press)
        .child(icon(glyph, 16.0).color(if active { t.accent } else { t.text }))
}

/// Small capitals, the way the game labels things
pub fn kicker(content: &str, color: Color) -> El {
    text(content.to_uppercase()).font(theme::KICKER).color(color)
}

/// A gold rule that fades out at its far end
pub fn rule(t: &Theme) -> El {
    el().w_full()
        .h(1.0)
        .linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, t.accent_line), (1.0, t.accent_line.alpha(0.0))])
}

/// How far a step on the node track got
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Node {
    Done,
    Active,
    Todo,
    Blocked,
}

/// A slow breath for the node that waits on the game
const PULSE: rune::Cycle = rune::Cycle {
    period: 1.8,
    phase: 0.0,
    stops: &[(0.0, 1.0), (0.5, 0.35), (1.0, 1.0)],
    still: 1.0,
    fps: 30.0,
};

/// Steps as nodes on a gold line, each labelled underneath
pub fn node_track(steps: &[(&'static str, Node)], t: &Theme) -> El {
    // Steps are tiles: face down until the game gets there, then turned over
    const FACE: (f32, f32) = (24.0, 31.0);
    let mut track = row().key("steps").w_full().items_start();
    for (i, (label, state)) in steps.iter().enumerate() {
        if i > 0 {
            let done = steps[i - 1].1 == Node::Done && *state != Node::Todo;
            track = track.child(
                el().grow(1.0)
                    .h(1.0)
                    .mt(FACE.1 / 2.0)
                    .mx(-22.0)
                    .fill(if done { t.accent } else { t.line }),
            );
        }
        let face_up = TileLook::face_up(t);
        let tile = match state {
            Node::Done => tile(FACE.0, FACE.1, face_up, Some(icon(&super::icons::CHECK, 15.0).color(t.face_ink))),
            Node::Active => tile(
                FACE.0,
                FACE.1,
                face_up,
                Some(el().size(8.0, 8.0).radius(4.0).fill(t.face_ink).cycle(PULSE)),
            )
            .shadow(0.0, 16.0, t.accent.alpha(0.5)),
            Node::Todo => tile(FACE.0, FACE.1, TileLook::face_down(t.tile_back, t.tile_back_edge, t), None),
            Node::Blocked => tile(FACE.0, FACE.1, face_up, Some(icon(&super::icons::ALERT, 14.0).color(t.face_bad))),
        };
        let ink = match state {
            Node::Active => t.accent,
            Node::Blocked => t.bad,
            Node::Done => t.muted,
            Node::Todo => t.faint,
        };
        track = track.child(
            col()
                .key(*label)
                .w(84.0)
                .gap(6.0)
                .items_center()
                .child(tile)
                .child(kicker(label, ink)),
        );
    }
    track
}

/// The red dragon on the app's tile, the one colour that stays the same in every theme
pub fn dragon() -> Color {
    rune::oklch(0.66, 0.22, 8.0)
}

/// How a tile paints
pub struct TileLook {
    /// The side facing up, light at the top and `shade` at the bottom
    pub top: Color,
    pub shade: Color,
    pub edge: Color,
    /// What shows under it: the back of a face up tile, the face of a face down one
    pub under: Color,
}

impl TileLook {
    /// Face up: bone over the set's back
    pub fn face_up(t: &Theme) -> TileLook {
        TileLook {
            top: t.bone,
            shade: t.bone_shade,
            edge: if t.dark { Color::TRANSPARENT } else { t.line },
            under: t.tile_back,
        }
    }

    /// Face down: the set's back, with a sliver of bone underneath
    pub fn face_down(back: Color, back_edge: Color, t: &Theme) -> TileLook {
        TileLook {
            top: back,
            shade: back_edge,
            edge: Color::TRANSPARENT,
            under: t.bone_shade,
        }
    }
}

/// A tile's side, `w` by `h` and `y` points down
fn side(w: f32, h: f32, y: f32) -> Vec<[f32; 2]> {
    vec![[0.0, y], [w, y], [w, y + h], [0.0, y + h]]
}

/// How much of a tile shows under its top side
fn tile_depth(h: f32) -> f32 {
    (h * 0.12).max(2.0)
}

/// A mahjong tile standing up, `w` by `h` for its top side, with what's under it showing below.
/// Square cornered: a slab, where the plates around it are cut
pub fn tile(w: f32, h: f32, look: TileLook, content: Option<El>) -> El {
    let depth = tile_depth(h);
    // The bottom of the top side, where the light falls off
    let fall = h * 0.14;
    let mut shape = Shape::new()
        .fill(side(w, h, depth), look.under)
        .fill(side(w, h, 0.0), look.top)
        .fill(side(w, fall, h - fall), look.shade);
    if look.edge.a > 0.0 {
        shape = shape.stroke_with(side(w, h, 0.0), true, 1.0, LineCap::Butt, look.edge);
    }
    el().size(w, h + depth)
        .shape(shape)
        .child(el().size(w, h).center().children(content))
}

/// What a sheet is called and how it closes
pub struct Sheet {
    pub key: &'static str,
    pub kicker: &'static str,
    pub title: &'static str,
    pub width: f32,
    pub close: fn(&mut Ui),
}

/// A plate over a dimmed window, with a kicker and title, closed by its ring button, Escape or a
/// click beside it
pub fn sheet(sheet: Sheet, body: El, cx: &Cx<Ui>, t: &Theme, layer: &mut Vec<El>) {
    let Sheet {
        key,
        kicker: kicker_text,
        title,
        width,
        close,
    } = sheet;
    let header = row()
        .w_full()
        .px(22.0)
        .pt(18.0)
        .pb(14.0)
        .gap(12.0)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(kicker(kicker_text, t.accent))
                .child(text(title).font(theme::HEADLINE).color(t.text)),
        )
        .child(ring_button(&format!("{key}-close"), &super::icons::CLOSE, "Close", false, t, close));
    let panel = plate(&format!("{key}-sheet"), cx, t)
        .w_full()
        .max_w(width)
        .role("dialog")
        .label(title)
        .modal(true, close)
        .child(header)
        .child(el().w_full().px(22.0).child(rule(t)))
        .child(body);

    layer.push(
        el().key(format!("{key}-scrim"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(t.kit.c.scrim)
            .a11y_hidden(),
    );
    layer.push(
        el().key(format!("{key}-layer"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .padding(18.0)
            .center()
            .child(panel),
    );
}
