//! The pieces the window is built from: plates that tear like tickets, tags that sit
//! on a plate's edge, pill and round buttons, the medallion track, and the mahjong tile.
//!
//! The look follows the game's own interface: big panels sharp at two corners and generously
//! round at the other two, everything small a pill or a circle, light catching an edge rather
//! than outlines, a line offset outside a panel for an accent, neutral chrome with one accent
//! color, gold for progress, sentence case.

use rune::{Color, Cx, El, Icon, LineCap, Path, Shape, Tx, col, el, icon, row, text};

use super::format::sentence;
use crate::gui::state::Ui;
use crate::gui::theme::{self, Theme};

/// The corners of the card that leads its screen: sharp at the top left and bottom right,
/// generous at the other two, as the game rounds its big panels. Kept to one card a screen so it
/// stays special
pub const CORNERS: [f32; 4] = [0.0, 26.0, 0.0, 26.0];
/// Every other card's corners: evenly rounded
pub const PLAIN: [f32; 4] = [14.0; 4];
/// How far a plate's tag sits in from its left edge
const TAG_INSET: f32 = 18.0;
/// How far the accent line runs outside a plate
const OFFSET: f32 = 7.0;

/// Light catching the top of glass: white over dark glass, stronger over light
fn sheen(t: &Theme) -> Color {
    Color::WHITE.alpha(if t.dark { 0.07 } else { 0.55 })
}

/// A panel: light catches its top edge, and it sits on a soft shadow
pub fn plate(key: &str, cx: &Cx<Ui>, t: &Theme) -> El {
    ticket(key, &[], cx, t)
}

/// A [`plate`] with the leading card's [`CORNERS`], for the one card that heads its screen
pub fn lead_plate(key: &str, cx: &Cx<Ui>, t: &Theme) -> El {
    cut_ticket(key, &[], None, CORNERS, cx, t)
}

/// The column the plates stack in, with a line running outside it: down the left of every plate
/// from a sparkle by the first, round the last one's corner, and dots off its far corner. It ties
/// the plates together as one set. The last plate has [`PLAIN`] corners
pub fn plate_stack(key: &str, cx: &Cx<Ui>, t: &Theme) -> El {
    let stack = col().key(key);
    match cx.layout_size(key) {
        Some([w, h]) => stack.shape(accent_lines(w, h, t)),
        None => stack,
    }
}

/// A [`plate`] torn by the [`perforation`]s keyed `tears`: where each meets the plate's edge, a
/// half circle is punched out of it, and the window shows through as through a ticket's notch.
///
/// The plate's face is a layer under its content, and the holes are punched in that layer, not
/// through the content
pub fn ticket(key: &str, tears: &[&str], cx: &Cx<Ui>, t: &Theme) -> El {
    ticket_on(key, tears, None, cx, t)
}

/// A ticket with `surface` over its glass and under what it holds, cut to its corners and
/// notches
pub fn ticket_on(key: &str, tears: &[&str], surface: Option<El>, cx: &Cx<Ui>, t: &Theme) -> El {
    cut_ticket(key, tears, surface, PLAIN, cx, t)
}

fn cut_ticket(key: &str, tears: &[&str], surface: Option<El>, corners: [f32; 4], cx: &Cx<Ui>, t: &Theme) -> El {
    let rect = cx.layout(key);
    let mut holes = Vec::new();
    if let Some([x, y, w, h]) = rect {
        for tear in tears {
            let Some([tx, ty, tw, th]) = cx.layout(tear) else { continue };
            if tw >= th {
                let cy = ty - y + th / 2.0;
                holes.extend([[0.0, cy], [w, cy]]);
            } else {
                let cx_ = tx - x + tw / 2.0;
                holes.extend([[cx_, 0.0], [cx_, h]]);
            }
        }
    }
    let mut tint = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .corners(corners)
        .fill(t.glass)
        .linear_gradient(std::f32::consts::PI, [(0.0, sheen(t)), (0.35, sheen(t).alpha(0.0))])
        .border(1.0, Color::WHITE.alpha(if t.dark { 0.05 } else { 0.35 }));
    let surface = surface.map(|surface| {
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .clip_corners(corners)
            .child(surface.w_full().h_full())
            .cutout_circles(holes.iter().map(|&hole| (hole, NOTCH - 0.5, NOTCH)))
            .inert()
    });
    tint = tint.cutout_circles(holes.iter().map(|&hole| (hole, NOTCH - 0.5, NOTCH)));

    col()
        .key(key)
        .corners(corners)
        .shadow(10.0, 30.0, Color::BLACK.alpha(if t.dark { 0.35 } else { 0.12 }))
        .child(tint.inert())
        .children(surface)
}

/// A line offset outside a stack of plates, down its left side and round the last one's bottom
/// corner, with a sparkle where it starts and a few dots off the far corner
fn accent_lines(w: f32, h: f32, t: &Theme) -> Shape {
    let ink = t.text.alpha(if t.dark { 0.26 } else { 0.3 });
    let r = PLAIN[3] + OFFSET;
    let top = 28.0_f32.min(h / 3.0);
    let path = Path::new()
        .move_to([-OFFSET, top])
        .line_to([-OFFSET, h + OFFSET - r])
        .quad_to([-OFFSET, h + OFFSET], [-OFFSET + r, h + OFFSET])
        .line_to([(w * 0.42).max(r + 20.0), h + OFFSET]);
    let mut shape = Shape::new().stroke_path(&path, 1.0, LineCap::Round, ink);
    // A four point sparkle on the line's start
    let (sx, sy, s) = (-OFFSET, top - 7.0, 5.0);
    let sparkle: Vec<[f32; 2]> = (0..8)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_4 * i as f32;
            let r = if i % 2 == 0 { s } else { s * 0.22 };
            [sx + r * a.sin(), sy - r * a.cos()]
        })
        .collect();
    shape = shape.fill(sparkle, t.text.alpha(if t.dark { 0.45 } else { 0.5 }));
    // Dots off the bottom right corner
    for row in 0..2 {
        for col in 0..3 {
            let (x, y) = (w - 6.0 - col as f32 * 7.0, h + OFFSET + 3.0 + row as f32 * 7.0);
            shape = shape.line([x, y], [x + 0.01, y], 2.0, LineCap::Round, ink);
        }
    }
    shape
}

/// A tag on a plate's top edge: a sharp strip of color, as the game tags its cards. Put it first
/// in the plate
pub fn tab(key: &str, label: &str, tint: Color, t: &Theme) -> El {
    row()
        .key(key)
        .h(24.0)
        .pl(12.0)
        .pr(12.0)
        .center_y()
        .mt(-12.0)
        .ml(TAG_INSET)
        .self_start()
        .fill(tint)
        .child(text(sentence(label)).font(theme::TAG).color(t.on_accent))
}

/// Perforations, where a plate tears like a ticket: a dotted line, and where it meets the plate's
/// edges the [`ticket`] it's in punches a half circle
const NOTCH: f32 = 7.0;
const DASH: f32 = 3.0;
const GAP: f32 = 5.0;

fn dashes(key: &str, across: bool, cx: &Cx<Ui>, t: &Theme) -> El {
    let line = el().key(key);
    let line = if across {
        line.w_full().h(NOTCH * 2.0)
    } else {
        line.h_full().w(NOTCH * 2.0)
    };
    let Some([w, h]) = cx.layout_size(key) else { return line };
    let length = if across { w } else { h };
    let ink = t.text.alpha(0.2);
    let mut shape = Shape::new();
    let mut at = NOTCH + GAP;
    while at + DASH <= length - NOTCH - GAP {
        shape = if across {
            shape.line([at, NOTCH], [at + DASH, NOTCH], 1.5, LineCap::Round, ink)
        } else {
            shape.line([NOTCH, at], [NOTCH, at + DASH], 1.5, LineCap::Round, ink)
        };
        at += DASH + GAP;
    }
    line.shape(shape)
}

/// A perforation across a plate, edge to edge, keyed so its [`ticket`] can punch its ends: put it
/// in the plate's column with no padding around it
pub fn perforation(key: &str, cx: &Cx<Ui>, t: &Theme) -> El {
    dashes(key, true, cx, t)
}

/// How a pill button paints
#[derive(Clone, Copy, PartialEq)]
pub enum Pill {
    /// The thing to do next: cream, dark text, a fine ring inside, on a soft shadow
    Primary,
    /// A wash of the text color
    Secondary,
    /// Just text in the accent
    Quiet,
}

/// How tall a pill is; its radius is half of it
const PILL_HEIGHT: f32 = 34.0;

/// A rounded button, painted as `kind`. It's enabled and has no glyph unless told otherwise;
/// [`PillButton::build`] makes it
pub fn pill<'a>(key: &'a str, label: &'a str, kind: Pill) -> PillButton<'a> {
    PillButton {
        key,
        label,
        kind,
        glyph: None,
        enabled: true,
    }
}

/// A [`pill`] being made
pub struct PillButton<'a> {
    key: &'a str,
    label: &'a str,
    kind: Pill,
    glyph: Option<&'static Icon>,
    enabled: bool,
}

impl PillButton<'_> {
    /// An icon before the label
    pub fn glyph(mut self, glyph: &'static Icon) -> Self {
        self.glyph = Some(glyph);
        self
    }

    /// Without `enabled` it dims and ignores presses
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn build(self, t: &Theme, on_press: impl Fn(&mut Ui, &mut Tx) + 'static) -> El {
        self.build_for(t, on_press)
    }

    /// [`build`](Self::build) for a window whose state isn't the app's, like the test controller
    pub fn build_for<S: 'static>(self, t: &Theme, on_press: impl Fn(&mut S, &mut Tx) + 'static) -> El {
        let PillButton {
            key,
            label,
            kind,
            glyph,
            enabled,
        } = self;
        let (fill, ink, hover) = match kind {
            Pill::Primary => (t.primary, t.on_primary, t.on_primary.alpha(0.06)),
            Pill::Secondary => (wash(t), t.text, t.text.alpha(0.06)),
            Pill::Quiet => (Color::TRANSPARENT, t.accent, t.accent.alpha(0.1)),
        };
        let mut pill = row()
            .key(key)
            .role("button")
            .label(label)
            .h(PILL_HEIGHT)
            .px(if kind == Pill::Quiet { 12.0 } else { 20.0 })
            .gap(7.0)
            .center()
            .radius(PILL_HEIGHT / 2.0)
            .fill(fill);
        if kind == Pill::Primary {
            pill = pill.shadow(3.0, 12.0, Color::BLACK.alpha(if t.dark { 0.35 } else { 0.15 })).child(
                el().pin(Some(3.0), Some(3.0), Some(3.0), Some(3.0))
                    .radius(14.0)
                    .border(1.0, t.on_primary.alpha(0.22))
                    .inert(),
            );
        }
        pill = pill
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
}

/// A wash of the text color: the fill of a [`Pill::Secondary`], and of chips that look like one
fn wash(t: &Theme) -> Color {
    t.text.alpha(0.08)
}

/// A text field's key handler that does `submit` on Enter
pub fn submit_on_enter(submit: fn(&mut Ui)) -> impl Fn(&mut Ui, &rune::KeyEvent, &mut Tx) -> bool + 'static {
    move |s: &mut Ui, ev: &rune::KeyEvent, _: &mut Tx| {
        if ev.key == rune::Key::Enter {
            submit(s);
            return true;
        }
        false
    }
}

/// A round icon button
pub fn ring_button(key: &str, glyph: &'static Icon, name: &str, active: bool, t: &Theme, on_press: impl Fn(&mut Ui) + 'static) -> El {
    el().key(key)
        .role("button")
        .label(name)
        .help(name)
        .expanded(active)
        .focusable(true)
        .size(36.0, 36.0)
        .radius(18.0)
        .center()
        .fill(if active { t.accent.alpha(0.2) } else { t.text.alpha(0.08) })
        .hover(t.text.alpha(0.08))
        .cursor(rune::Cursor::Pointer)
        .on_press(on_press)
        .child(icon(glyph, 16.0).color(if active { t.accent } else { t.text }))
}

/// A small entrance: an icon, a label and a chevron in a pill, like the game's side entries
pub fn entry(key: &str, glyph: &'static Icon, label: &str, t: &Theme) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(38.0)
        .pl(14.0)
        .pr(12.0)
        .gap(10.0)
        .center_y()
        .radius(19.0)
        .fill(t.text.alpha(0.08))
        .hover(t.text.alpha(0.06))
        .child(icon(glyph, 16.0).color(t.accent))
        .child(text(label).font(theme::LABEL).color(t.text))
        .child(icon(&super::icons::CHEVRON, 14.0).color(t.muted))
}

/// A label over a group, in sentence case, quieter than a title
pub fn kicker(content: &str, color: Color) -> El {
    text(sentence(content)).font(theme::GROUP).color(color)
}

/// The dot between two [`kicker`]s on a line
pub fn dot_sep(t: &Theme) -> El {
    el().size(3.0, 3.0).radius(1.5).fill(t.faint)
}

/// A divider: a hairline of light that fades out at both ends
pub fn rule(t: &Theme) -> El {
    let line = t.text.alpha(if t.dark { 0.1 } else { 0.12 });
    el().w_full().h(1.0).linear_gradient(std::f32::consts::FRAC_PI_2, [
        (0.0, line.alpha(0.0)),
        (0.15, line),
        (0.85, line),
        (1.0, line.alpha(0.0)),
    ])
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

/// Steps as medallions on a gold line, each labeled underneath: done ones gold with a check,
/// the current one glowing in the accent, the rest dim
pub fn node_track(steps: &[(&'static str, Node)], t: &Theme) -> El {
    const SIZE: f32 = 30.0;
    let mut track = row().key("steps").w_full().items_start();
    for (i, (label, state)) in steps.iter().enumerate() {
        if i > 0 {
            let done = steps[i - 1].1 == Node::Done && *state != Node::Todo;
            track = track.child(track_link(done, SIZE, t).mx(-30.0));
        }
        let medallion = medallion(SIZE);
        let medallion = match state {
            Node::Done => check_medallion(SIZE, t).shadow(0.0, 10.0, t.gold.alpha(0.35)),
            Node::Active => medallion
                .fill(t.panel)
                .border(2.0, t.accent)
                .shadow(0.0, 18.0, t.accent.alpha(0.55))
                .child(el().size(10.0, 10.0).radius(5.0).fill(t.accent).cycle(PULSE)),
            Node::Todo => medallion.fill(t.panel).border(2.0, t.text.alpha(0.16)),
            Node::Blocked => medallion
                .fill(t.panel)
                .border(2.0, t.bad)
                .child(icon(&super::icons::ALERT, 14.0).color(t.bad)),
        };
        let ink = match state {
            Node::Active => t.text,
            Node::Blocked => t.bad,
            Node::Done => t.muted,
            Node::Todo => t.faint,
        };
        track = track.child(
            col()
                .key(*label)
                .w(96.0)
                .gap(8.0)
                .items_center()
                .child(medallion)
                .child(kicker(label, ink)),
        );
    }
    track
}

/// A round marker on a track, `size` across, its content centered
pub fn medallion(size: f32) -> El {
    el().size(size, size).radius(size / 2.0).center()
}

/// A gold [`medallion`] with a check, for a step or a copy that's done
pub fn check_medallion(size: f32, t: &Theme) -> El {
    medallion(size)
        .fill(t.gold)
        .child(icon(&super::icons::CHECK, size / 2.0).color(t.on_gold))
}

/// The line on a track between two [`medallion`]s `size` across, gold once it's `done`. Give it a
/// negative margin across to reach into the medallions either side; it goes under them
pub fn track_link(done: bool, size: f32, t: &Theme) -> El {
    el().grow(1.0)
        .h(3.0)
        .radius(1.5)
        .mt(size / 2.0 - 1.5)
        .paint_order(-1)
        .fill(if done { t.gold } else { t.text.alpha(0.14) })
}

/// How a status reads
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    /// Nothing to say
    Quiet,
    Good,
    Bad,
}

impl Tone {
    /// The color of its dot
    pub fn dot(self, t: &Theme) -> Color {
        match self {
            Tone::Quiet => t.faint,
            Tone::Good => t.good,
            Tone::Bad => t.bad,
        }
    }

    /// The color of its words: a quiet one's muted rather than faint, so it still reads
    pub fn ink(self, t: &Theme) -> Color {
        match self {
            Tone::Quiet => t.muted,
            tone => tone.dot(t),
        }
    }
}

/// A status's dot in `tint`, `size` across, or while `busy` a spinner in the accent
pub fn status_dot(tint: Color, busy: bool, size: f32, t: &Theme) -> El {
    if busy {
        rune_kit::Spinner::new().size(12.0).color(t.accent).build(&t.kit)
    } else {
        el().size(size, size).radius(size / 2.0).fill(tint)
    }
}

/// An icon in a double ring, like the nodes of the game's circuits, a title and what it means, and
/// its action past a divider. It has no padding of its own
pub fn option_row(
    key: &'static str,
    glyph: &'static Icon,
    title: &'static str,
    detail: El,
    action: impl Into<Option<El>>,
    t: &Theme,
) -> El {
    let medallion = medallion(38.0).border(1.0, t.accent_line).child(
        medallion(30.0)
            .fill(t.well)
            .border(1.0, t.line)
            .child(icon(glyph, 15.0).color(t.accent)),
    );
    let line = row().key(key).w_full().gap(14.0).center_y().child(medallion).child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(3.0)
            .child(text(title).font(theme::TITLE).color(t.text))
            .child(detail),
    );
    match action.into() {
        Some(action) => line.child(el().w(1.0).h(34.0).fill(t.line)).child(action),
        None => line,
    }
}

/// The red dragon on the app's tile, the one color that stays the same in every theme
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
