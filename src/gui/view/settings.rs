//! The settings sheet. Every change applies and saves at once, except the port, which is only
//! saved once the server could open it.

use rune::{Color, Cx, Drag, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{ControlSize, SegmentedControl, Switch, TextInput};

use super::parts::{Pill, Sheet, TileLook, kicker, pill, sheet, tile};
use crate::gui::settings::{Accent, Anchor, BackgroundFill, ImageFit, ThemeMode};
use crate::gui::state::{Grab, Panel, Ui, Update};
use crate::gui::theme::{self, Theme};

pub fn dialog(s: &Ui, cx: &Cx<Ui>, t: &Theme, layer: &mut Vec<El>) {
    if s.panel != Some(Panel::Settings) {
        return;
    }
    let page = cx.layout_size("page").unwrap_or([460.0, 700.0]);
    // As tall as the window leaves room for under the sheet's title
    let tall = (page[1] - 170.0).clamp(320.0, 900.0);
    let scrolling = |body: El| {
        body.min_h(0.0)
            .max_h(tall)
            .pt(16.0)
            .scroll(ScrollSpec { stick_bottom: false })
            .scroll_edge_fade(12.0)
    };
    let mut rest = vec![appearance(s, t), live_sync(s, t), window(s, t)];
    if s.updates_supported {
        rest.push(updates(s, t));
    }

    // A wide window has the background on the right, with room for its preview, and the rest on
    // the left; a narrow one has them all in one column, the background after the appearance
    let (width, body) = if page[0] >= WIDE {
        let width = (page[0] - 36.0).min(1040.0);
        let room = width - LEFT - 1.0 - 44.0;
        let left = scrolling(col().key("settings-general").w(LEFT).children(rest));
        let right = scrolling(
            col()
                .key("settings-background")
                .grow(1.0)
                .min_w(0.0)
                .child(background(s, cx, room, t)),
        );
        let divider = el().w(1.0).fill(t.line);
        (
            width,
            row()
                .key("settings-body")
                .w_full()
                .items_stretch()
                .child(left)
                .child(divider)
                .child(right),
        )
    } else {
        let mut rest = rest.into_iter();
        let body = col()
            .key("settings-body")
            .w_full()
            .children(rest.next())
            .child(background(s, cx, NARROW - 44.0, t))
            .children(rest);
        (NARROW, scrolling(body))
    };
    let spec = Sheet {
        key: "settings",
        kicker: "Archiver",
        title: "Settings",
        width,
        close: |s| {
            s.panel = None;
            s.background_trial = None;
        },
    };
    sheet(spec, body, cx, t, layer);
}

/// The narrowest window the sheet has the background beside the other settings in
const WIDE: f32 = 900.0;
/// The sheet's width in a narrower window
const NARROW: f32 = 420.0;
/// The width of the other settings beside the background
const LEFT: f32 = 400.0;

fn section(key: &'static str, title: &'static str, t: &Theme) -> El {
    col().key(key).w_full().px(22.0).pb(22.0).gap(12.0).child(kicker(title, t.accent))
}

/// A setting that's on or off: what it does on the left, its switch on the right
fn toggle(key: &'static str, label: &'static str, description: Option<&str>, on: bool, toggle: fn(&mut Ui), t: &Theme) -> El {
    let switch = Switch::new(on)
        .label(label)
        .hide_label()
        .control_size(ControlSize::Small)
        .on_toggle(toggle)
        .build(&t.kit)
        .key("switch");
    let mut text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(label).font(theme::BODY).color(t.text).wrap());
    if let Some(description) = description {
        text_col = text_col.child(text(description.to_string()).font(theme::SMALL).color(t.faint).wrap());
    }
    row().key(key).w_full().gap(12.0).center_y().child(text_col).child(switch)
}

fn note(content: impl Into<String>, error: bool, t: &Theme) -> El {
    text(content).font(theme::SMALL).color(if error { t.bad } else { t.faint }).wrap()
}

fn live_sync(s: &Ui, t: &Theme) -> El {
    let k = &t.kit;
    let field = TextInput::new(&s.port_edit)
        .label("Port")
        .placeholder("23313")
        .width(Some(120.0))
        .invalid(s.port_error.is_some())
        .on_edit(|s: &mut Ui| &mut s.port_edit)
        .on_key(|s: &mut Ui, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Enter {
                s.apply_port();
                return true;
            }
            false
        })
        .build(k)
        .key("port");
    let can_apply = s.port_edited() && s.pending_port.is_none();
    let apply = pill("apply-port", "Apply", None, Pill::Secondary, can_apply, t, |s, _| s.apply_port());

    let note = match (&s.port_error, s.pending_port) {
        (Some(error), _) => note(error.clone(), true, t),
        (None, Some(port)) => note(format!("Moving the server to port {port}…"), false, t),
        (None, None) => note(
            "The optimizer connects to ws://localhost:<port>/ws. Port 0 picks any free port.",
            false,
            t,
        ),
    };

    section("live-sync-settings", "Live sync", t)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(text("Port").font(theme::BODY).color(t.text).grow(1.0))
                .child(field)
                .child(apply),
        )
        .child(note)
}

fn window(s: &Ui, t: &Theme) -> El {
    let mut section = section("window-settings", "Window", t);

    if s.has_tray {
        section = section.child(toggle(
            "close-to-tray",
            "Keep running in the tray when closed",
            Some("Live sync keeps working while the window is closed"),
            s.settings.close_to_tray,
            |s| s.settings_mut(|s| s.close_to_tray = !s.close_to_tray),
            t,
        ));
    }

    let start_hidden = if s.has_tray && s.settings.close_to_tray {
        "Starts in the tray"
    } else {
        "Starts with the window minimized"
    };
    section = section.child(toggle(
        "start-minimized",
        "Start minimized",
        Some(start_hidden),
        s.settings.start_minimized,
        |s| s.settings_mut(|s| s.start_minimized = !s.start_minimized),
        t,
    ));

    if cfg!(windows) {
        section = section.child(toggle(
            "run-on-start",
            "Start with Windows",
            None,
            s.settings.run_on_start,
            |s| s.set_run_on_start(!s.settings.run_on_start),
            t,
        ));
        if let Some(error) = &s.run_on_start_error {
            section = section.child(note(format!("Couldn't change that: {error}"), true, t));
        }
    }
    section
}

/// The fit modes in the order the settings offer them, with their labels
const FITS: [(ImageFit, &str); 5] = [
    (ImageFit::Cover, "Cover"),
    (ImageFit::Contain, "Fit"),
    (ImageFit::Fill, "Stretch"),
    (ImageFit::ScaleDown, "Shrink"),
    (ImageFit::None, "Original"),
];

/// The theme modes in the order the settings offer them, with their labels
const MODES: [(ThemeMode, &str); 3] = [
    (ThemeMode::System, "System"),
    (ThemeMode::Dark, "Dark"),
    (ThemeMode::Light, "Light"),
];

fn appearance(s: &Ui, t: &Theme) -> El {
    let labels = MODES.map(|(_, label)| label);
    let mode = MODES.iter().position(|(mode, _)| *mode == s.settings.theme);
    let setting = |label: &'static str, detail: String, control: El| {
        row()
            .w_full()
            .gap(16.0)
            .center_y()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(label).font(theme::BODY).color(t.text))
                    .child(note(detail, false, t)),
            )
            .child(control)
    };

    section("appearance-settings", "Appearance", t)
        .child(setting(
            "Theme",
            "Light or dark".into(),
            SegmentedControl::new(&labels, mode)
                .label("Theme")
                .fit()
                .on_pick(|s: &mut Ui, i, _| s.settings_mut(|s| s.theme = MODES[i].0))
                .build(&t.kit)
                .key("theme-mode"),
        ))
        .child(setting(
            "Accent",
            s.settings.accent.label().into(),
            accent_picker(s.settings.accent, t),
        ))
        .child(setting(
            "Card position",
            format!("{}, for when the window is bigger than the cards", s.settings.card_anchor.label()),
            anchor_picker(s.settings.card_anchor, t),
        ))
}

/// A face down tile from each accent's set, its back in that set's color. The chosen one stands
/// a little taller, with a mark under it
fn accent_picker(current: Accent, t: &Theme) -> El {
    let swatch = |accent: Accent| {
        let selected = accent == current;
        let (back, edge) = theme::tile_back(accent, t.dark);
        let look = TileLook::face_down(back, edge, t);
        col()
            .key(format!("accent-{}", accent.label().to_lowercase()))
            .w(26.0)
            .h(40.0)
            .gap(4.0)
            .items_center()
            .justify_end()
            .role("radio")
            .label(accent.label())
            .help(accent.label())
            .checked(selected)
            .focusable(true)
            .radius(4.0)
            .hover(t.text.alpha(0.08))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut Ui| s.settings_mut(|s| s.accent = accent))
            .child(if selected {
                tile(18.0, 23.0, look, None)
            } else {
                tile(16.0, 20.0, look, None)
            })
            .child(
                el().size(12.0, 2.0)
                    .radius(1.0)
                    .fill(if selected { t.text } else { Color::TRANSPARENT }),
            )
    };
    row()
        .key("accent-picker")
        .role("radiogroup")
        .label("Accent")
        .focus_group()
        .focus_follows()
        .gap(2.0)
        .items_end()
        .children(Accent::ALL.into_iter().map(swatch))
}

/// A small window with nine spots to put the cards in. The chosen spot shows the cards as a
/// tile, the others are dots to click
fn anchor_picker(current: Anchor, t: &Theme) -> El {
    const CELL: (f32, f32) = (30.0, 22.0);
    let spot = |anchor: Anchor| {
        let selected = anchor == current;
        // The cards, as the tile they are
        let mark = if selected {
            tile(12.0, 15.0, TileLook::face_up(t), None).shadow(0.0, 8.0, t.accent.alpha(0.5))
        } else {
            el().size(5.0, 5.0).radius(2.5).fill(t.faint)
        };
        let (column, row) = anchor.cell();
        el().key(format!("anchor-{column}-{row}"))
            .size(CELL.0, CELL.1)
            .center()
            .radius(4.0)
            .role("radio")
            .label(anchor.label())
            .checked(selected)
            .focusable(true)
            .hover(t.accent.alpha(0.12))
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut Ui| s.settings_mut(|s| s.card_anchor = anchor))
            .child(mark)
    };
    let rows = Anchor::ALL
        .chunks(3)
        .map(|anchors| row().gap(2.0).children(anchors.iter().copied().map(spot)));

    col()
        .key("card-anchor")
        .role("radiogroup")
        .label("Card position")
        .focus_group()
        .focus_follows()
        .padding(4.0)
        .gap(2.0)
        .radius(4.0)
        .fill(t.well)
        .border(1.0, t.accent_line)
        .children(rows)
}

fn background(s: &Ui, cx: &Cx<Ui>, room: f32, t: &Theme) -> El {
    let path = &s.settings.background_image;
    let chosen = !path.is_empty();
    let mut section = section("background-settings", "Background", t).child(image_row(s, t));
    if !chosen {
        return section.child(note(
            "An image of your choice behind the window, faded so the text stays easy to read.",
            false,
            t,
        ));
    }
    let error = s
        .background_error
        .clone()
        .or_else(|| s.background.as_ref().and_then(|(_, image)| image.error()).map(|e| e.to_string()));
    if s.background_loading.is_some() {
        section = section.child(note("Loading the image…", false, t));
    }
    if let Some(error) = error {
        section = section.child(note(format!("Couldn't load that image: {error}"), true, t));
    }

    let labels = FITS.map(|(_, label)| label);
    let fit = FITS.iter().position(|(fit, _)| *fit == s.settings.image_fit);
    let opacity = s.settings.background_opacity;
    let zoom = s.settings.background_zoom;
    let zoom = slider_row(
        "zoom",
        "Zoom",
        zoom_along(zoom),
        percent(zoom),
        |s, along| s.set_background_zoom(zoom_at(along)),
        // A step is a tenth of the image's fit
        |s, delta| s.set_background_zoom(s.settings.background_zoom + delta * 0.1),
        t,
    );
    let opacity = slider_row(
        "opacity",
        "Opacity",
        opacity,
        percent(opacity),
        |s, along| s.set_background_opacity(along),
        |s, delta| s.set_background_opacity(s.settings.background_opacity + delta * 0.05),
        t,
    );
    // Side by side when there's room for both
    let sliders = if room >= 520.0 {
        row()
            .w_full()
            .gap(28.0)
            .child(zoom.grow(1.0).min_w(0.0))
            .child(opacity.grow(1.0).min_w(0.0))
    } else {
        col().w_full().gap(12.0).child(zoom).child(opacity)
    };
    section
        .children(placement(s, cx, room, t))
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(text("Fit").font(theme::BODY).color(t.text).grow(1.0))
                .child(
                    SegmentedControl::new(&labels, fit)
                        .label("Background fit")
                        .fit()
                        .on_pick(|s: &mut Ui, i, _| s.set_image_fit(FITS[i].0))
                        .build(&t.kit)
                        .key("background-fit"),
                ),
        )
        .child(sliders)
        .children(fill_picker(s, room, t))
}

/// A setting's name, its [`slider`], and its value
fn slider_row(
    key: &str,
    name: &str,
    along: f32,
    value: String,
    set: impl Fn(&mut Ui, f32) + 'static,
    step: impl Fn(&mut Ui, f32) + 'static,
    t: &Theme,
) -> El {
    row()
        .gap(12.0)
        .center_y()
        .child(text(name).font(theme::BODY).color(t.text).w(64.0))
        .child(slider(key, &format!("Background {}", name.to_lowercase()), along, set, step, t).grow(1.0))
        .child(text(value).key(format!("{key}-value")).font(theme::LABEL).color(t.muted).w(44.0))
}

fn percent(share: f32) -> String {
    format!("{}%", (share * 100.0).round())
}

/// How far along the zoom slider `zoom` is. It's even in doublings, so 50% to 100% takes as
/// much of it as 100% to 200%
fn zoom_along(zoom: f32) -> f32 {
    let [least, most] = crate::gui::background::ZOOM.map(f32::log2);
    ((zoom.max(f32::MIN_POSITIVE).log2() - least) / (most - least)).clamp(0.0, 1.0)
}

/// The zoom at `along` of the way along its slider
fn zoom_at(along: f32) -> f32 {
    let [least, most] = crate::gui::background::ZOOM.map(f32::log2);
    (least + along * (most - least)).exp2()
}

/// The image chosen, as a thumbnail with its name and size, and the buttons to change it
fn image_row(s: &Ui, t: &Theme) -> El {
    const THUMB: [f32; 2] = [56.0, 38.0];
    let path = &s.settings.background_image;
    let chosen = !path.is_empty();
    let name = std::path::Path::new(path)
        .file_name()
        .map_or_else(|| "No image".to_string(), |name| name.to_string_lossy().into_owned());
    let image = s.background_image();
    let mut thumb = el()
        .key("background-thumb")
        .size(THUMB[0], THUMB[1])
        .radius(5.0)
        .clip_rounded(5.0)
        .fill(t.well)
        .border(1.0, t.line);
    if let Some(image) = image {
        thumb = thumb.child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .pixels_fit(image.pixels(), rune::SurfaceFit::Fill),
        );
    }
    let size = image.and_then(|image| image.size()).map(|[w, h]| format!("{w} × {h}"));
    let mut row_ = row().w_full().gap(12.0).center_y().child(thumb).child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .child(
                text(name)
                    .key("background-name")
                    .font(theme::BODY)
                    .color(if chosen { t.text } else { t.faint })
                    .truncate(),
            )
            .children(size.map(|size| text(size).font(theme::SMALL).color(t.faint))),
    );
    if chosen {
        row_ = row_.child(pill("remove-background", "Remove", None, Pill::Quiet, true, t, |s, _| {
            s.remove_background()
        }));
    }
    row_.child(pill(
        "choose-background",
        if chosen { "Change…" } else { "Choose image…" },
        None,
        Pill::Secondary,
        true,
        t,
        |s: &mut Ui, tx: &mut Tx| s.choose_background(tx),
    ))
}

/// How far an arrow key moves the image, as a share of the window
const POINT_STEP: f32 = 0.05;
/// How much of the image the preview shows past the window's edges, as a share of the window's
/// longer side
const MARGIN: f32 = 0.15;
/// The shortest and tallest the preview stands
const STAGE_HEIGHT: [f32; 2] = [200.0, 300.0];
/// How near a press has to be to the pin or the frame's corner to take it, in points
const REACH: f32 = 14.0;

/// The preview of where the image sits: the window drawn small, with the image placed in it and
/// what fills around it, and the rest of the image dim past its edges
#[derive(Clone, Copy)]
struct Stage {
    size: [f32; 2],
    /// Preview points to a window point
    k: f32,
    /// The window's own size
    real: [f32; 2],
    /// The image's own size, in points
    own: [f32; 2],
}

impl Stage {
    fn new(room: f32, real: [f32; 2], own: [f32; 2]) -> Self {
        let margin = real[0].max(real[1]) * MARGIN;
        let span = real.map(|v| v + 2.0 * margin);
        let height = (room * span[1] / span[0]).clamp(STAGE_HEIGHT[0], STAGE_HEIGHT[1]);
        let k = (room / span[0]).min(height / span[1]);
        Stage {
            size: [room, height],
            k,
            real,
            own,
        }
    }

    /// The window the preview shows: the one tried, else the window's own
    fn window(&self, s: &Ui) -> [f32; 2] {
        s.background_trial.unwrap_or(self.real)
    }

    /// Where the image goes in that window, `[x, y, w, h]` in its points
    fn rect(&self, s: &Ui) -> [f32; 4] {
        let settings = &s.settings;
        crate::gui::background::place(
            settings.image_fit,
            settings.background_zoom,
            self.own,
            self.window(s),
            settings.background_anchor,
            settings.background_target,
        )
    }

    /// Where the pin is, 0 to 1 across and down the window
    fn pin(&self, s: &Ui) -> [f32; 2] {
        s.settings.background_target.unwrap_or(s.settings.background_anchor)
    }

    /// Where a window `window` large has its top-left drawn: in the middle of the preview, so a
    /// size tried grows and shrinks about it
    fn origin(&self, window: [f32; 2]) -> [f32; 2] {
        [0, 1].map(|n| (self.size[n] - window[n] * self.k) / 2.0)
    }

    /// A point on the preview, in the points of a window `window` large
    fn in_window(&self, window: [f32; 2], at: [f32; 2]) -> [f32; 2] {
        let origin = self.origin(window);
        [0, 1].map(|n| (at[n] - origin[n]) / self.k)
    }

    /// A point in a window `window` large, on the preview
    fn on_stage(&self, window: [f32; 2], at: [f32; 2]) -> [f32; 2] {
        let origin = self.origin(window);
        [0, 1].map(|n| origin[n] + at[n] * self.k)
    }

    /// What a press at `at` on the preview takes: the pin, the frame's corner, else the image
    fn grab_at(&self, s: &Ui, at: [f32; 2]) -> Grab {
        let window = self.window(s);
        let near = |p: [f32; 2]| {
            let p = self.on_stage(window, p);
            (p[0] - at[0]).hypot(p[1] - at[1]) <= REACH
        };
        let pin = self.pin(s);
        let [x, y, ..] = self.rect(s);
        if near([pin[0] * window[0], pin[1] * window[1]]) {
            Grab::Pin { from: [x, y], out: None }
        } else if near(window) {
            Grab::Resize
        } else {
            Grab::Pan([x, y])
        }
    }

    /// Puts the image `to` from the window's top-left, as near as it goes. With the pin where its
    /// point is in the image, the point moves; with a pin of its own, the pin moves with the image
    fn pan(&self, s: &mut Ui, to: [f32; 2]) {
        let window = self.window(s);
        let [_, _, w, h] = self.rect(s);
        let size = [w, h];
        let anchor = s.settings.background_anchor;
        if s.settings.background_target.is_some() {
            let target = [0, 1].map(|n| ((to[n] + anchor[n] * size[n]) / window[n]).clamp(0.0, 1.0));
            s.set_background_target(Some(target));
        } else {
            s.set_background_anchor(follow(anchor, to, window, size));
        }
    }

    /// Pins the image, `from` the window's top-left, at its point under the preview's point
    /// `at`, as near as the image has one. Over the window, the image stays exactly where it is:
    /// the pin goes to a whole percent of the window, as it's kept, and its point on the image is
    /// worked out from that, unrounded, rather than from where the image is now, so nothing drifts
    /// as it moves. Past the window's edges, the pin waits there, as handed back, until `released`,
    /// when it goes to the nearest place in the window and the image moves to keep the point under
    /// it
    fn repin(&self, s: &mut Ui, from: [f32; 2], at: [f32; 2], released: bool) -> Option<[f32; 2]> {
        let window = self.window(s);
        let [_, _, w, h] = self.rect(s);
        let size = [w.max(1.0), h.max(1.0)];
        // Over the image, and the part of it the preview shows
        let [seen_from, seen_to] = [[0.0, 0.0], self.size].map(|p| self.in_window(window, p));
        let at = self.in_window(window, at);
        let at = [0, 1].map(|n| at[n].clamp(from[n].max(seen_from[n]), (from[n] + size[n]).min(seen_to[n])));
        let inside = (0..2).all(|n| (0.0..=window[n]).contains(&at[n]));
        if !inside && !released {
            return Some(at);
        }
        let mut target = [0.0; 2];
        let mut anchor = [0.0; 2];
        for n in 0..2 {
            let to = (at[n].clamp(0.0, window[n]) / window[n] * 100.0).round() / 100.0;
            if inside {
                // Rounded to where the image still has a point
                let (least, most) = (from[n].max(0.0), (from[n] + size[n]).min(window[n]));
                let on = (to * window[n]).clamp(least, most);
                target[n] = on / window[n];
                anchor[n] = ((on - from[n]) / size[n]).clamp(0.0, 1.0);
            } else {
                target[n] = to;
                anchor[n] = ((at[n] - from[n]) / size[n]).clamp(0.0, 1.0);
            }
        }
        s.set_background_pin(anchor, Some(target));
        None
    }

    /// Puts the pin back where its point is in the image, the image staying where it is
    fn unpin(&self, s: &mut Ui) {
        let window = self.window(s);
        let [x, y, w, h] = self.rect(s);
        let anchor = follow(s.settings.background_anchor, [x, y], window, [w, h]);
        s.set_background_pin(anchor, None);
    }

    /// Tries the window at the size, about the preview's middle, with a corner at its point `at`
    fn resize(&self, s: &mut Ui, at: [f32; 2]) {
        s.background_trial = Some([0, 1].map(|n| {
            let size = 2.0 * (at[n] - self.size[n] / 2.0).abs() / self.k;
            let most = ((self.size[n] - 8.0) / self.k).max(self.real[n]);
            size.clamp(self.real[n] * 0.35, most)
        }));
    }
}

/// The point of an image `size` large that puts it `to` from the window's top-left when it goes
/// to the same place in the window, as CSS's `object-position` has it. Along a side as long as
/// the window's, where it goes doesn't matter, so that stays as `anchor` has it
fn follow(anchor: [f32; 2], to: [f32; 2], window: [f32; 2], size: [f32; 2]) -> [f32; 2] {
    [0, 1].map(|n| {
        let slack = window[n] - size[n];
        if slack.abs() < 0.5 {
            anchor[n]
        } else {
            (to[n] / slack).clamp(0.0, 1.0)
        }
    })
}

/// Where the image sits, as a preview to place it on. Dragging the image moves it; dragging the
/// pin picks the point that holds still when the window resizes, the image staying put; dragging
/// the frame's corner tries the window at another size; and Ctrl + scroll or a pinch zooms
fn placement(s: &Ui, cx: &Cx<Ui>, room: f32, t: &Theme) -> Option<El> {
    const PIN: f32 = 16.0;
    const HANDLE: f32 = 10.0;
    let image = s.background_image()?;
    let scale = cx.env().scale;
    let [iw, ih] = image.size().map(|[w, h]| [w as f32 / scale, h as f32 / scale])?;
    let real = cx.layout_size("page").unwrap_or([460.0, 700.0]);
    let stage = Stage::new(room, real, [iw, ih]);
    let k = stage.k;
    let window = stage.window(s);
    let [ox, oy] = stage.origin(window);
    let frame = window.map(|v| v * k);

    let mut wallpaper = super::wallpaper_for(s, image, window, scale)?;
    let scaled = |r: [f32; 4]| r.map(|v| v * k);
    let rect = scaled(wallpaper.rect);
    wallpaper.rect = rect;
    wallpaper.cover = scaled(wallpaper.cover);
    wallpaper.window = frame;
    wallpaper.scale = k;
    wallpaper.reach = k;

    // The whole image, dim, then the window over it as it shows. None of it springs from where it
    // was, as Rune's elements do, or it would trail behind the image the wallpaper puts at once
    let beyond = el()
        .pin(Some(ox + rect[0]), Some(oy + rect[1]), None, None)
        .size(rect[2], rect[3])
        .inert()
        .motion(rune::Spring::NONE)
        .opacity(0.3)
        .pixels(image.pixels());
    // Where the pin is: dragged out past the window's edges, where it's held; else where it's kept
    let out = match s.background_grab {
        Some(Grab::Pin { out, .. }) => out,
        _ => None,
    };
    let at = match out {
        Some(out) => out.map(|v| v * k),
        None => {
            let pinned = stage.pin(s);
            [pinned[0] * frame[0], pinned[1] * frame[1]]
        }
    };
    // The crosshairs at where in the window the pin goes, once it's let go
    let snap = [at[0].clamp(0.0, frame[0]), at[1].clamp(0.0, frame[1])];
    // Faint crosshairs across the window through the pin, so it reads against any image
    let hair = Color::WHITE.alpha(0.35);
    let shown = el()
        .pin(Some(ox), Some(oy), None, None)
        .size(frame[0], frame[1])
        .inert()
        .motion(rune::Spring::NONE)
        .clip_rounded(0.0)
        .fill(t.bg)
        .child(
            el().pin(Some(0.0), Some(0.0), None, None)
                .motion(rune::Spring::NONE)
                .child(wallpaper.build()),
        )
        .child(
            el().pin(Some(snap[0]), Some(0.0), None, Some(0.0))
                .w(1.0)
                .fill(hair)
                .motion(rune::Spring::NONE),
        )
        .child(
            el().pin(Some(0.0), Some(snap[1]), Some(0.0), None)
                .h(1.0)
                .fill(hair)
                .motion(rune::Spring::NONE),
        );
    let outline = el()
        .pin(Some(ox - 1.0), Some(oy - 1.0), None, None)
        .size(frame[0] + 2.0, frame[1] + 2.0)
        .inert()
        .motion(rune::Spring::NONE)
        .radius(2.0)
        .border(1.5, t.accent);
    let handle = el()
        .key("background-trial-handle")
        .pin(Some(ox + frame[0] - HANDLE / 2.0), Some(oy + frame[1] - HANDLE / 2.0), None, None)
        .size(HANDLE, HANDLE)
        .radius(2.0)
        .inert()
        .motion(rune::Spring::NONE)
        .fill(t.accent)
        .border(1.0, Color::WHITE);
    // The pin: a ring, with a dot in it once it has a place of its own in the window
    let mut pin = el()
        .key("background-pin")
        .pin(Some(ox + at[0] - PIN / 2.0), Some(oy + at[1] - PIN / 2.0), None, None)
        .size(PIN, PIN)
        .radius(PIN / 2.0)
        .center()
        .inert()
        .motion(rune::Spring::NONE)
        .border(2.0, Color::WHITE)
        .shadow(0.0, 4.0, Color::BLACK.alpha(0.6));
    if s.settings.background_target.is_some() {
        pin = pin.child(el().size(6.0, 6.0).radius(3.0).fill(t.accent));
    }
    // The pinned point on the image, when the image's edges keep it from reaching the pin
    let anchor = s.settings.background_anchor;
    let point = [ox + rect[0] + anchor[0] * rect[2], oy + rect[1] + anchor[1] * rect[3]];
    let held = (out.is_none() && (point[0] - ox - at[0]).hypot(point[1] - oy - at[1]) > 3.0).then(|| {
        el().key("background-point")
            .pin(Some(point[0] - 4.0), Some(point[1] - 4.0), None, None)
            .size(8.0, 8.0)
            .radius(4.0)
            .inert()
            .motion(rune::Spring::NONE)
            .fill(Color::WHITE.alpha(0.8))
    });

    let surface = el()
        .key("background-stage")
        .size(stage.size[0], stage.size[1])
        .radius(8.0)
        .clip_rounded(8.0)
        .fill(t.well)
        .border(1.0, t.line)
        .role("slider")
        .label("Where the image sits in the window")
        .focusable(true)
        // What a press would take, as the pointer moves over it, or what the drag has
        .cursor(match s.background_grab.or(s.background_hover) {
            Some(Grab::Pan(_)) if s.background_grab.is_some() => rune::Cursor::Grabbing,
            Some(Grab::Pin { .. }) => rune::Cursor::Pointer,
            Some(Grab::Resize) => rune::Cursor::ResizeNwse,
            _ => rune::Cursor::Grab,
        })
        .on_pointer_move_local(move |s: &mut Ui, p: rune::PointerEvent, _: &mut Tx| {
            let hover = Some(stage.grab_at(s, [p.x, p.y]));
            if s.background_hover != hover {
                s.background_hover = hover;
            }
        })
        .on_drag(move |s: &mut Ui, drag: &mut Drag, _: &mut Tx| {
            if drag.phase == rune::Phase::Down {
                s.background_grab = Some(stage.grab_at(s, [drag.start_x, drag.start_y]));
            }
            match s.background_grab {
                Some(Grab::Pan(from)) => stage.pan(s, [from[0] + drag.dx / k, from[1] + drag.dy / k]),
                Some(Grab::Pin { from, .. }) => {
                    let released = drag.phase == rune::Phase::Up;
                    let out = stage.repin(s, from, [drag.x, drag.y], released);
                    s.background_grab = Some(Grab::Pin { from, out });
                }
                Some(Grab::Resize) => stage.resize(s, [drag.x, drag.y]),
                None => {}
            }
            if drag.phase == rune::Phase::Up {
                s.background_grab = None;
            }
        })
        .on_magnify(|s: &mut Ui, ev: rune::MagnifyEvent, _: &mut Tx| {
            let zoom = s.settings.background_zoom;
            let next = zoom * (1.0 + ev.delta);
            // A pinch comes in steps too small for a whole percent, so each moves one at least
            let next = if (next - zoom).abs() < 0.01 {
                zoom + 0.01f32.copysign(ev.delta)
            } else {
                next
            };
            s.set_background_zoom(next);
            true
        })
        .on_key(move |s: &mut Ui, ev: &rune::KeyEvent, _: &mut Tx| {
            let by = match ev.key {
                rune::Key::Left => [-1.0, 0.0],
                rune::Key::Right => [1.0, 0.0],
                rune::Key::Up => [0.0, -1.0],
                rune::Key::Down => [0.0, 1.0],
                _ => return false,
            };
            let window = stage.window(s);
            let [x, y, ..] = stage.rect(s);
            stage.pan(s, [x + by[0] * POINT_STEP * window[0], y + by[1] * POINT_STEP * window[1]]);
            true
        })
        .child(beyond)
        .child(shown)
        .child(outline)
        .children(held)
        .child(pin)
        .child(handle);

    let mut actions = row().gap(4.0).center_y();
    if s.settings.background_target.is_some() {
        actions = actions.child(pill("background-unpin", "Unpin", None, Pill::Quiet, true, t, move |s, _| {
            stage.unpin(s)
        }));
    }
    if s.background_trial.is_some() {
        actions = actions.child(pill(
            "background-untry",
            "Back to this window",
            None,
            Pill::Quiet,
            true,
            t,
            |s, _| s.background_trial = None,
        ));
    }
    let hint = if s.settings.background_target.is_some() {
        "Drag the image to move it. The pin holds its point there as the window resizes; drag it to \
         pick another. Drag the window's corner to try a size."
    } else {
        "Drag the image to move it. Drag the pin to hold a point in place as the window resizes. \
         Drag the window's corner to try a size."
    };

    let mut block = col().key("background-placement").w_full().gap(8.0).child(surface).child(
        row()
            .w_full()
            .gap(12.0)
            .items_start()
            .child(note(hint, false, t).grow(1.0).min_w(0.0))
            .child(actions),
    );
    // Stretched to the window and not zoomed, it's the same wherever it's placed
    if s.settings.image_fit == ImageFit::Fill && s.settings.background_zoom == 1.0 {
        block = block.child(note(
            "Stretched to the window, the whole image shows wherever it's placed. Zoom in or out to place it.",
            false,
            t,
        ));
    }
    Some(block)
}

/// What fills the window around an image smaller than it: each choice as a tile showing the image
/// with that fill around it
fn fill_picker(s: &Ui, room: f32, t: &Theme) -> Option<El> {
    const PER_ROW: usize = 4;
    const GAP: f32 = 8.0;
    const TILE_H: [f32; 2] = [58.0, 80.0];
    let image = s.background_image()?;
    let [iw, ih] = image.size().map(|[w, h]| [w as f32, h as f32])?;
    // Zoomed out, even a covering fit leaves room around the image
    let covers = matches!(s.settings.image_fit, ImageFit::Cover | ImageFit::Fill) && s.settings.background_zoom >= 1.0;
    let tile_w = (room - GAP * (PER_ROW - 1) as f32) / PER_ROW as f32;
    // Taller as they widen, short of crowding out what follows
    let tile_h = (tile_w * 0.62).clamp(TILE_H[0], TILE_H[1]);
    // In each tile the image small in the middle, so there's room around it to fill
    let inner = (tile_w * 0.56 / iw).min(tile_h * 0.56 / ih);
    let (w, h) = (iw * inner, ih * inner);
    let rect = [(tile_w - w) / 2.0, (tile_h - h) / 2.0, w, h];
    let cover = {
        let k = (tile_w / iw).max(tile_h / ih);
        [(tile_w - iw * k) / 2.0, (tile_h - ih * k) / 2.0, iw * k, ih * k]
    };

    let tile = |fill: BackgroundFill| {
        let on = fill == s.settings.background_fill;
        let picture = super::wallpaper::Wallpaper {
            pixels: image.pixels(),
            rect,
            cover,
            window: [tile_w, tile_h],
            fill,
            edges: s.background_edges,
            color: s.background_color(t.bg).color(),
            // Blurs as strong as in a window a few times the tile's size, and the blurred
            // mirror's bands close enough to show in the room around the image, so the fills
            // read apart
            scale: 0.25,
            reach: 0.05,
        }
        .build();
        col()
            .key(format!("fill-{}", fill.label().to_lowercase().replace([' ', ','], "-")))
            .w(tile_w)
            .gap(5.0)
            .role("radio")
            .label(fill.label())
            .checked(on)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut Ui| s.set_background_fill(fill))
            .child(
                el().size(tile_w, tile_h)
                    .radius(6.0)
                    .clip_rounded(6.0)
                    .fill(t.bg)
                    .border(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.line })
                    .hover(t.accent.alpha(0.12))
                    .child(el().pin(Some(0.0), Some(0.0), None, None).inert().child(picture)),
            )
            .child(
                text(fill.label())
                    .font(theme::SMALL)
                    .color(if on { t.text } else { t.muted })
                    .truncate(),
            )
    };
    let rows = BackgroundFill::ALL
        .chunks(PER_ROW)
        .map(|fills| row().gap(GAP).children(fills.iter().copied().map(tile)));

    let mut block = col()
        .key("background-fill")
        .w_full()
        .gap(8.0)
        .child(
            col()
                .gap(2.0)
                .child(text("Around the image").font(theme::BODY).color(t.text))
                .child(note(
                    if covers {
                        "The image covers the window at this fit, so there's nothing around it. Fit, Shrink and Original \
                         leave room, as does zooming out."
                    } else {
                        "What fills the window where the image doesn't reach."
                    },
                    false,
                    t,
                )),
        )
        .child(
            col()
                .role("radiogroup")
                .label("Around the image")
                .focus_group()
                .focus_follows()
                .gap(10.0)
                .children(rows),
        );
    if s.settings.background_fill == BackgroundFill::Color && !covers {
        block = block.child(color_picker(s, room, t));
    }
    if covers {
        block = block.opacity(0.55);
    }
    Some(block)
}

/// A horizontal slider, its knob `value` of the way along (0 to 1), dragged with the pointer, which
/// hands `set` where it is along it, or stepped with the arrow keys, which hand `step` the step
fn slider(
    key: &str,
    label: &str,
    value: f32,
    set: impl Fn(&mut Ui, f32) + 'static,
    step: impl Fn(&mut Ui, f32) + 'static,
    t: &Theme,
) -> El {
    const KNOB: f32 = 14.0;
    let set = move |s: &mut Ui, drag: &mut Drag, _: &mut Tx| {
        let usable = (drag.w - KNOB).max(1.0);
        set(s, ((drag.x - KNOB / 2.0) / usable).clamp(0.0, 1.0));
    };
    let track = |fill| el().h(4.0).radius(2.0).fill(fill);
    row()
        .key(format!("{key}-slider"))
        .role("slider")
        .label(label)
        .range_value(f64::from(value), 0.0, 1.0)
        .focusable(true)
        .h(24.0)
        .min_w(0.0)
        // Half a knob of room each side, outside where the track was: the knob at either end
        // reaches past the track, and a press on any of it is the slider's
        .px(KNOB / 2.0)
        .mx(-KNOB / 2.0)
        .center_y()
        .cursor(rune::Cursor::Pointer)
        .on_drag(set)
        .on_step(move |s: &mut Ui, delta, _| step(s, delta as f32))
        .child(track(t.accent).w_frac(value))
        .child(
            el().key(format!("{key}-knob"))
                .size(KNOB, KNOB)
                .radius(KNOB / 2.0)
                .fill(t.primary)
                .border(2.0, t.accent)
                .shadow(0.0, 8.0, t.accent.alpha(0.45))
                .mx(-KNOB / 2.0)
                // Over the rest of the track, which comes after it
                .paint_order(1),
        )
        .child(track(t.line).grow(1.0))
}

fn updates(s: &Ui, t: &Theme) -> El {
    let version = env!("CARGO_PKG_VERSION");
    let (status, error) = match &s.update {
        Update::Idle => (format!("Version {version}"), false),
        Update::Checking => ("Checking for updates…".to_string(), false),
        Update::UpToDate => (format!("Version {version} is the latest"), false),
        Update::Available { version: latest } => (format!("Version {latest} is available, you have {version}"), false),
        Update::Installing => ("Installing…".to_string(), false),
        Update::Failed(e) => (format!("Couldn't update: {e}"), true),
    };
    let action = match &s.update {
        Update::Available { .. } => pill("update-action", "Update", None, Pill::Primary, true, t, |s, _| s.install_update()),
        update => {
            let busy = matches!(update, Update::Checking | Update::Installing);
            pill("update-action", "Check now", None, Pill::Secondary, !busy, t, |s, _| {
                s.check_for_update()
            })
        }
    };

    section("update-settings", "Updates", t)
        .child(toggle(
            "always-update",
            "Install updates automatically",
            None,
            s.settings.always_update,
            |s| s.settings_mut(|s| s.always_update = !s.always_update),
            t,
        ))
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    text(status)
                        .key("update-status")
                        .font(theme::SMALL)
                        .color(if error { t.bad } else { t.muted })
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(action),
        )
}

/// The color around the image: picked on the picker (`rune_kit::ColorPicker`, our Rune patch:
/// anomalyco/rune#1433), typed as a code, or one of the image's own edge colors, black, white or
/// the window's own
fn color_picker(s: &Ui, room: f32, t: &Theme) -> El {
    let mut swatches = vec![t.bg, Color::BLACK, Color::WHITE];
    if let Some(edges) = s.background_edges {
        swatches.extend(edges.map(|c| c.alpha(1.0)));
    }
    let mut picker = rune_kit::ColorPicker::new(s.background_color(t.bg))
        .label("Color around the image")
        .size(room, 132.0)
        .swatches(swatches)
        .on_change(|s: &mut Ui, hsv, _| s.set_background_color(hsv))
        .hex_field(&s.background_hex, |s: &mut Ui| &mut s.background_hex);
    if s.background_sample.is_some() {
        picker = picker.eyedropper(s.background_dropper, |s: &mut Ui, _| s.background_dropper = !s.background_dropper);
    }
    col()
        .key("background-color")
        .pt(6.0)
        .gap(12.0)
        .children(s.background_dropper.then(|| dropper(s, room, t)).flatten())
        .child(picker.build(&t.kit))
}

/// The image to pick a color from with the eyedropper, as wide as the sheet: pressing picks the
/// color under the pointer, dragging picks as it goes, and letting go puts the eyedropper away
fn dropper(s: &Ui, room: f32, t: &Theme) -> Option<El> {
    let image = s.background_image()?;
    let [iw, ih] = image.size().map(|[w, h]| [w as f32, h as f32])?;
    let height = (room * ih / iw).min(240.0);
    let width = height * iw / ih;
    let surface = el()
        .key("background-dropper")
        .size(width, height)
        .radius(6.0)
        .clip_rounded(6.0)
        .border(2.0, t.accent)
        .role("button")
        .label("Pick a color from the image")
        .cursor(rune::Cursor::Crosshair)
        .pixels(image.pixels())
        .on_drag(|s: &mut Ui, drag: &mut Drag, _: &mut Tx| {
            s.pick_background_color([drag.x / drag.w.max(1.0), drag.y / drag.h.max(1.0)]);
            if drag.phase == rune::Phase::Up {
                s.background_dropper = false;
            }
        });
    Some(
        col()
            .w_full()
            .gap(6.0)
            .items_center()
            .child(surface)
            .child(note("Press on the image to pick its color there.", false, t)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::settings::Settings;

    /// A 1200 × 800 window with a 1920 × 1080 image covering it, zoomed in a little
    fn staged() -> (Ui, Stage) {
        let mut ui = Ui::new(Settings::default());
        ui.settings.image_fit = ImageFit::Cover;
        ui.settings.background_zoom = 1.3;
        ui.settings.background_anchor = [0.37, 0.61];
        (ui, Stage::new(600.0, [1200.0, 800.0], [1920.0, 1080.0]))
    }

    /// Pins the image at the window's point `at`, dragging there from where it is and letting go
    /// there if `released`
    fn drag_pin(ui: &mut Ui, stage: &Stage, from: [f32; 2], at: [f32; 2], released: bool) -> Option<[f32; 2]> {
        let at = stage.on_stage(stage.window(ui), at);
        stage.repin(ui, from, at, released)
    }

    fn assert_still(before: [f32; 4], after: [f32; 4]) {
        for n in 0..4 {
            assert!((after[n] - before[n]).abs() < 0.01, "moved to {after:?} from {before:?}");
        }
    }

    #[test]
    fn moving_the_pin_leaves_the_image_where_it_is() {
        let (mut ui, stage) = staged();
        let before = stage.rect(&ui);
        let from = [before[0], before[1]];
        // Wandering over the window and past its edges, a fraction of a point at a time
        for i in 0..400 {
            let i = i as f32;
            drag_pin(&mut ui, &stage, from, [i * 3.7 - 50.0, 900.0 - i * 2.3], false);
            assert_still(before, stage.rect(&ui));
        }
        assert!(ui.settings.background_target.is_some());
    }

    #[test]
    fn a_pin_let_go_past_the_window_goes_to_its_edge_with_its_point() {
        let (mut ui, stage) = staged();
        // Contained and zoomed in, the image reaches past the window's left edge, with more of it
        // the preview shows there
        ui.settings.image_fit = ImageFit::Contain;
        ui.settings.background_zoom = 1.2;
        ui.settings.background_anchor = [0.5, 0.5];
        ui.settings.background_target = Some([0.5, 0.5]);
        let before = stage.rect(&ui);
        let from = [before[0], before[1]];
        let past = [-40.0, 300.0];
        let out = drag_pin(&mut ui, &stage, from, past, false).expect("held past the edge");
        assert!(
            (out[0] - past[0]).abs() < 0.01 && (out[1] - past[1]).abs() < 0.01,
            "held at {out:?}"
        );
        assert_still(before, stage.rect(&ui));

        assert_eq!(drag_pin(&mut ui, &stage, from, past, true), None);
        let target = ui.settings.background_target.unwrap();
        assert_eq!(target[0], 0.0);
        let anchor = ui.settings.background_anchor;
        assert!((anchor[0] - (past[0] - from[0]) / before[2]).abs() < 1e-3);
        // The point the pin was on is at the window's left edge, as near as the image goes
        let after = stage.rect(&ui);
        assert!(after[0] > from[0]);
    }

    #[test]
    fn unpinning_leaves_the_image_where_it_is() {
        let (mut ui, stage) = staged();
        let before = stage.rect(&ui);
        drag_pin(&mut ui, &stage, [before[0], before[1]], [300.0, 200.0], true);
        stage.unpin(&mut ui);
        assert_eq!(ui.settings.background_target, None);
        assert_still(before, stage.rect(&ui));
    }

    #[test]
    fn a_size_tried_stays_in_the_middle() {
        let (mut ui, stage) = staged();
        let middle = stage.size.map(|v| v / 2.0);
        stage.resize(&mut ui, [middle[0] + 100.0, middle[1] + 50.0]);
        let window = stage.window(&ui);
        let origin = stage.origin(window);
        let corner = stage.on_stage(window, window);
        assert!((origin[0] + corner[0] - stage.size[0]).abs() < 0.01);
        assert!((origin[1] + corner[1] - stage.size[1]).abs() < 0.01);
        assert!((corner[0] - middle[0] - 100.0).abs() < 0.01);
    }
}
