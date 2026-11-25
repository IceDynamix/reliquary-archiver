//! The settings sheet. Every change applies and saves at once, except the port, which is only
//! saved once the server could open it.

use rune::{Color, Cx, Drag, El, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{ControlSize, SegmentedControl, Switch, TextInput};

use super::parts::{Pill, Sheet, TileLook, kicker, pill, sheet, tile};
use crate::gui::icons;
use crate::gui::settings::{Accent, Anchor, BackgroundFill, ImageFit, ThemeMode};
use crate::gui::state::{Panel, Ui, Update};
use crate::gui::theme::{self, Theme};

pub fn dialog(s: &Ui, cx: &Cx<Ui>, t: &Theme, layer: &mut Vec<El>) {
    if s.panel != Some(Panel::Settings) {
        return;
    }
    let mut body = col()
        .key("settings-body")
        .w_full()
        // As tall as the window leaves room for under the sheet's title
        .max_h(cx.layout_size("page").map_or(440.0, |[_, h]| (h - 170.0).clamp(320.0, 720.0)))
        .min_h(0.0)
        .pt(16.0)
        .child(appearance(s, t))
        .child(background(s, cx, t))
        .child(live_sync(s, t))
        .child(window(s, t));
    if s.updates_supported {
        body = body.child(updates(s, t));
    }
    let body = body.scroll(ScrollSpec { stick_bottom: false }).scroll_edge_fade(12.0);
    let spec = Sheet {
        key: "settings",
        kicker: "Archiver",
        title: "Settings",
        width: 420.0,
        close: |s| s.panel = None,
    };
    sheet(spec, body, cx, t, layer);
}

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

fn background(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
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
    section
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
        .child(slider_row(
            "zoom",
            "Zoom",
            zoom_along(zoom),
            percent(zoom),
            |s, along| s.set_background_zoom(zoom_at(along)),
            // A step is a tenth of the image's fit
            |s, delta| s.set_background_zoom(s.settings.background_zoom + delta * 0.1),
            t,
        ))
        .children(placement(s, cx, t))
        .children(fill_picker(s, t))
        .child(slider_row(
            "opacity",
            "Opacity",
            opacity,
            percent(opacity),
            |s, along| s.set_background_opacity(along),
            |s, delta| s.set_background_opacity(s.settings.background_opacity + delta * 0.05),
            t,
        ))
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

/// The room the background controls have across the settings sheet
const ROOM: f32 = 376.0;
/// The tallest the placement pickers stand
const PICKER_HEIGHT: f32 = 118.0;
/// How far an arrow key moves a point
const POINT_STEP: f32 = 0.05;

/// Where the image sits: the point of it that matters, picked on the image, and where in the
/// window that point goes, picked on a small window showing the result, fill and all. Between
/// them a link: linked, the point goes to the same place in the window as it is in the image
fn placement(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> Option<El> {
    const LINK: f32 = 30.0;
    const GAP: f32 = 10.0;
    let image = s.background_image()?;
    let [iw, ih] = image.size().map(|[w, h]| [w as f32, h as f32])?;
    let window = cx.layout_size("page").unwrap_or([460.0, 700.0]);
    let anchor = s.settings.background_anchor;
    let linked = s.settings.background_target.is_none();
    let target = s.settings.background_target.unwrap_or(anchor);

    // Both as tall as fits across the sheet with the link between them
    let (image_aspect, window_aspect) = (iw / ih, window[0] / window[1]);
    let height = PICKER_HEIGHT.min((ROOM - LINK - 2.0 * GAP) / (image_aspect + window_aspect));
    let picture_size = [height * image_aspect, height];
    let frame = [height * window_aspect, height];

    let picture = point_picker(
        "background-anchor",
        "The part of the image that matters",
        picture_size,
        anchor,
        false,
        t,
        |s: &mut Ui, at| s.set_background_anchor(at),
        |s: &Ui| s.settings.background_anchor,
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert().pixels(image.pixels()),
    );

    let k = height / window[1];
    let mut shown = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert().fill(t.bg);
    if let Some(mut wallpaper) = super::wallpaper_for(s, image, window, cx.env().scale) {
        let scaled = |r: [f32; 4]| r.map(|v| v * k);
        wallpaper.rect = scaled(wallpaper.rect);
        wallpaper.cover = scaled(wallpaper.cover);
        wallpaper.window = frame;
        wallpaper.scale = k;
        wallpaper.reach = k;
        shown = shown.child(wallpaper.build());
    }
    let preview = point_picker(
        "background-target",
        "Where in the window it goes",
        frame,
        target,
        linked,
        t,
        |s: &mut Ui, at| s.set_background_target(Some(at)),
        |s: &Ui| s.settings.background_target.unwrap_or(s.settings.background_anchor),
        shown,
    );

    let link = el()
        .key("background-link")
        .size(LINK, LINK)
        .radius(LINK / 2.0)
        .center()
        .role("switch")
        .label("Put it in the same place in the window as in the image")
        .help(if linked {
            "Linked: the point goes to the same place in the window as in the image"
        } else {
            "Link: put the point in the same place in the window as in the image"
        })
        .checked(linked)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .fill(if linked { t.accent.alpha(0.2) } else { t.well })
        .border(1.0, if linked { t.accent } else { t.line })
        .hover(t.accent.alpha(0.14))
        .on_press(|s: &mut Ui| {
            let next = match s.settings.background_target {
                Some(_) => None,
                None => Some(s.settings.background_anchor),
            };
            s.set_background_target(next)
        })
        .child(icon(&icons::LINK, 15.0).color(if linked { t.accent } else { t.muted }));

    // The window's caption lines up with the window's right edge, as the image's does with its left
    let caption = |kicker_text: &'static str, words: String, width: f32, end: bool| {
        let caption = col().w(width).gap(1.0);
        let caption = if end { caption.items_end() } else { caption };
        caption
            .child(kicker(kicker_text, t.muted))
            .child(text(words).font(theme::SMALL).color(t.faint).truncate())
    };
    let percent = |[x, y]: [f32; 2]| format!("{}% across, {}% down", (x * 100.0).round(), (y * 100.0).round());
    let goes = if linked {
        "Same place as in the image".to_string()
    } else {
        percent(target)
    };

    let mut block = col()
        .key("background-placement")
        .w_full()
        .gap(8.0)
        .child(col().gap(2.0).child(text("Position").font(theme::BODY).color(t.text)).child(note(
            "Pick the part of the image to keep in view, then where in the window it goes.",
            false,
            t,
        )))
        // The image at the left edge, the window at the right, the link centered between them
        .child(
            row()
                .w_full()
                .gap(GAP)
                .center_y()
                .justify_between()
                .child(picture)
                .child(link)
                .child(preview),
        )
        .child(
            row()
                .w_full()
                .gap(GAP)
                .justify_between()
                .child(caption("Image", percent(anchor), picture_size[0], false))
                .child(caption("Window", goes, frame[0].max(120.0), true)),
        );
    // Stretched to the window and not zoomed, it's the same wherever it's placed
    if s.settings.image_fit == ImageFit::Fill && s.settings.background_zoom == 1.0 {
        block = block.opacity(0.45).child(note(
            "Stretched to the window, the whole image shows wherever it's placed. Zoom in or out to place it.",
            false,
            t,
        ));
    }
    Some(block)
}

/// A surface `size` points large to pick a point on, by dragging or with the arrow keys: `content`
/// in it, and the point at `at` (0 to 1 across and down) marked over that, hollow when it only
/// follows another point
#[allow(clippy::too_many_arguments)]
fn point_picker(
    key: &'static str,
    label: &'static str,
    size: [f32; 2],
    at: [f32; 2],
    following: bool,
    t: &Theme,
    set: fn(&mut Ui, [f32; 2]),
    get: fn(&Ui) -> [f32; 2],
    content: El,
) -> El {
    const MARK: f32 = 16.0;
    // Crosshairs across the whole surface, faint, so the point reads against any image
    let hair = Color::WHITE.alpha(0.35);
    let cross = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .inert()
        .child(el().pin(Some(at[0] * size[0]), Some(0.0), None, Some(0.0)).w(1.0).fill(hair))
        .child(el().pin(Some(0.0), Some(at[1] * size[1]), Some(0.0), None).h(1.0).fill(hair));
    let mut mark = el()
        .key(format!("{key}-mark"))
        .pin(Some(at[0] * size[0] - MARK / 2.0), Some(at[1] * size[1] - MARK / 2.0), None, None)
        .size(MARK, MARK)
        .radius(MARK / 2.0)
        .center()
        .inert()
        .border(2.0, Color::WHITE)
        .shadow(0.0, 4.0, Color::BLACK.alpha(0.6));
    if !following {
        mark = mark.child(el().size(6.0, 6.0).radius(3.0).fill(t.accent));
    }
    el().key(key)
        .size(size[0], size[1])
        .radius(6.0)
        .clip_rounded(6.0)
        .border(1.0, t.accent_line)
        .role("slider")
        .label(label)
        .help(label)
        .focusable(true)
        .cursor(rune::Cursor::Crosshair)
        .on_drag(move |s: &mut Ui, drag: &mut Drag, _: &mut Tx| {
            set(s, [drag.x / drag.w.max(1.0), drag.y / drag.h.max(1.0)]);
        })
        .on_key(move |s: &mut Ui, ev: &rune::KeyEvent, _: &mut Tx| {
            let [x, y] = get(s);
            let to = match ev.key {
                rune::Key::Left => [x - POINT_STEP, y],
                rune::Key::Right => [x + POINT_STEP, y],
                rune::Key::Up => [x, y - POINT_STEP],
                rune::Key::Down => [x, y + POINT_STEP],
                _ => return false,
            };
            set(s, to);
            true
        })
        .child(content)
        .child(cross)
        .child(mark)
}

/// What fills the window around an image smaller than it: each choice as a tile showing the image
/// with that fill around it
fn fill_picker(s: &Ui, t: &Theme) -> Option<El> {
    const PER_ROW: usize = 4;
    const GAP: f32 = 8.0;
    const TILE_H: f32 = 58.0;
    let image = s.background_image()?;
    let [iw, ih] = image.size().map(|[w, h]| [w as f32, h as f32])?;
    // Zoomed out, even a covering fit leaves room around the image
    let covers = matches!(s.settings.image_fit, ImageFit::Cover | ImageFit::Fill) && s.settings.background_zoom >= 1.0;
    let tile_w = (ROOM - GAP * (PER_ROW - 1) as f32) / PER_ROW as f32;
    // In each tile the image small in the middle, so there's room around it to fill
    let inner = (tile_w * 0.56 / iw).min(TILE_H * 0.56 / ih);
    let (w, h) = (iw * inner, ih * inner);
    let rect = [(tile_w - w) / 2.0, (TILE_H - h) / 2.0, w, h];
    let cover = {
        let k = (tile_w / iw).max(TILE_H / ih);
        [(tile_w - iw * k) / 2.0, (TILE_H - ih * k) / 2.0, iw * k, ih * k]
    };

    let tile = |fill: BackgroundFill| {
        let on = fill == s.settings.background_fill;
        let picture = super::wallpaper::Wallpaper {
            pixels: image.pixels(),
            rect,
            cover,
            window: [tile_w, TILE_H],
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
                el().size(tile_w, TILE_H)
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
        block = block.child(color_picker(s, t));
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
fn color_picker(s: &Ui, t: &Theme) -> El {
    let mut swatches = vec![t.bg, Color::BLACK, Color::WHITE];
    if let Some(edges) = s.background_edges {
        swatches.extend(edges.map(|c| c.alpha(1.0)));
    }
    let mut picker = rune_kit::ColorPicker::new(s.background_color(t.bg))
        .label("Color around the image")
        .size(ROOM, 132.0)
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
        .children(s.background_dropper.then(|| dropper(s, t)).flatten())
        .child(picker.build(&t.kit))
}

/// The image to pick a color from with the eyedropper, as wide as the sheet: pressing picks the
/// color under the pointer, dragging picks as it goes, and letting go puts the eyedropper away
fn dropper(s: &Ui, t: &Theme) -> Option<El> {
    let image = s.background_image()?;
    let [iw, ih] = image.size().map(|[w, h]| [w as f32, h as f32])?;
    let height = (ROOM * ih / iw).min(240.0);
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
