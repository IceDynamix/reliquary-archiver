//! The pieces the window is built from: plates with cut corners and gold hairlines, tabs that sit
//! on a plate's edge, pill and ring buttons, the node track, and the astrolabe behind it all.
//!
//! Plates draw their outline from the size they laid out at last (`Cx::layout_size`), which Rune
//! settles before the first frame, so a plate never shows without its corners.

use rune::{Color, Cx, El, Icon, LineCap, Shape, Tx, canvas, col, el, icon, row, text};

use crate::gui::state::Ui;
use crate::gui::theme::{self, Theme};

/// How much a plate's corners are cut
const CUT: f32 = 12.0;

/// A panel with its top left and bottom right corners cut, edged with a hairline, and a short
/// gold mark along the top edge
pub fn plate(key: &str, cx: &Cx<Ui>, t: &Theme) -> El {
    let plate = col().key(key);
    let Some([w, h]) = cx.layout_size(key) else {
        return plate.fill(t.panel);
    };
    let outline = vec![[CUT, 0.0], [w, 0.0], [w, h - CUT], [w - CUT, h], [0.0, h], [0.0, CUT]];
    let mark = 40.0_f32.min(w / 3.0);
    plate.shape(
        Shape::new()
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
    const SIZE: f32 = 30.0;
    let mut track = row().key("steps").w_full().items_start();
    for (i, (label, state)) in steps.iter().enumerate() {
        if i > 0 {
            let done = steps[i - 1].1 == Node::Done && *state != Node::Todo;
            track = track.child(
                el().grow(1.0)
                    .h(1.0)
                    .mt(SIZE / 2.0)
                    .mx(-14.0)
                    .fill(if done { t.accent } else { t.line }),
            );
        }
        let (ring, inner): (Color, El) = match state {
            Node::Done => (t.accent, icon(&super::icons::CHECK, 15.0).color(t.on_accent)),
            Node::Active => (t.accent, el().size(10.0, 10.0).radius(5.0).fill(t.accent).cycle(PULSE)),
            Node::Todo => (t.line, el().size(6.0, 6.0).radius(3.0).fill(t.line)),
            Node::Blocked => (t.bad, icon(&super::icons::ALERT, 14.0).color(t.bad)),
        };
        let mut dot = el()
            .size(SIZE, SIZE)
            .radius(SIZE / 2.0)
            .center()
            .fill(if *state == Node::Done { t.accent } else { t.well })
            .border(if *state == Node::Active { 1.5 } else { 1.0 }, ring)
            .child(inner);
        if *state == Node::Active {
            dot = dot.shadow(0.0, 14.0, t.accent.alpha(0.55));
        }
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
                .gap(7.0)
                .items_center()
                .child(dot)
                .child(kicker(label, ink)),
        );
    }
    track
}

/// Rings, ticks and spokes in faint gold, behind the window's content
pub fn astrolabe(size: f32, t: &Theme) -> El {
    let c = size / 2.0;
    let ink = t.accent.alpha(0.075);
    let at = |r: f32, deg: f32| {
        let a = deg.to_radians();
        [c + r * a.cos(), c + r * a.sin()]
    };
    let circle = |r: f32| (0..=120).map(|i| at(r, i as f32 * 3.0)).collect::<Vec<_>>();

    let mut shape = Shape::new();
    for (r, width) in [(c - 2.0, 1.0), (c * 0.82, 1.0), (c * 0.8, 0.6), (c * 0.46, 1.0), (c * 0.2, 1.0)] {
        shape = shape.stroke(circle(r), width, ink);
    }
    for i in 0..72 {
        let deg = i as f32 * 5.0;
        let inner = if i % 6 == 0 { c * 0.7 } else { c * 0.76 };
        shape = shape.line(at(inner, deg), at(c * 0.8, deg), 0.8, LineCap::Butt, ink);
    }
    // An eight pointed star in the middle
    let star: Vec<[f32; 2]> = (0..16)
        .map(|i| at(if i % 2 == 0 { c * 0.44 } else { c * 0.13 }, i as f32 * 22.5 - 90.0))
        .collect();
    shape = shape.stroke_with(star, true, 1.0, LineCap::Butt, ink);
    canvas(shape).size(size, size).inert().a11y_hidden()
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
