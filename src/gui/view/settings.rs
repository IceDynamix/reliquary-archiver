//! The settings sheet. Every change applies and saves at once, except the port, which is only
//! saved once the server could open it.

use rune::{Color, Cx, Drag, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{ControlSize, SegmentedControl, Switch, TextInput};

use super::parts::{Pill, Sheet, TileLook, kicker, pill, sheet, submit_on_enter, tile};
use crate::gui::background::Grab;
use crate::gui::background::preview::Stage;
use crate::gui::settings::{Accent, Anchor, BackgroundFill, ImageFit, ThemeMode};
use crate::gui::state::{Panel, Ui, Update};
use crate::gui::theme::{self, Theme};

pub fn dialog(s: &Ui, cx: &Cx<Ui>, t: &Theme, layer: &mut Vec<El>) {
    if s.panel != Some(Panel::Settings) {
        return;
    }
    let page = super::window_size(cx);
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
            s.background.trial = None;
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

/// A setting: its name, and under it what it means if there's more to say, on the left; its
/// control on the right
fn setting_row(label: &str, detail: Option<String>, control: El, t: &Theme) -> El {
    row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(label).font(theme::BODY).color(t.text).wrap())
                .children(detail.map(|detail| note(detail, t))),
        )
        .child(control)
}

/// A setting that's on or off: a [`setting_row`] with a switch
fn toggle(key: &'static str, label: &'static str, description: Option<&str>, on: bool, toggle: fn(&mut Ui), t: &Theme) -> El {
    let switch = Switch::new(on)
        .label(label)
        .hide_label()
        .control_size(ControlSize::Small)
        .on_toggle(toggle)
        .build(&t.kit)
        .key("switch");
    setting_row(label, description.map(str::to_string), switch, t).key(key)
}

/// A line of small print under a setting
fn note(content: impl Into<String>, t: &Theme) -> El {
    text(content).font(theme::SMALL).color(t.faint).wrap()
}

/// A [`note`] that something went wrong
fn error_note(content: impl Into<String>, t: &Theme) -> El {
    text(content).font(theme::SMALL).color(t.bad).wrap()
}

fn live_sync(s: &Ui, t: &Theme) -> El {
    let k = &t.kit;
    let field = TextInput::new(&s.port.edit)
        .label("Port")
        .placeholder("23313")
        .width(Some(120.0))
        .invalid(s.port.error.is_some())
        .on_edit(|s: &mut Ui| &mut s.port.edit)
        .on_key(submit_on_enter(Ui::apply_port))
        .build(k)
        .key("port");
    let can_apply = s.port_edited() && s.port.pending.is_none();
    let apply = pill("apply-port", "Apply", Pill::Secondary)
        .enabled(can_apply)
        .build(t, |s, _| s.apply_port());

    let note = match (&s.port.error, s.port.pending) {
        (Some(error), _) => error_note(error.clone(), t),
        (None, Some(port)) => note(format!("Switching to port {port}…"), t),
        (None, None) => note("Port 0 picks a free port.", t),
    };

    section("live-sync-settings", "Live sync", t)
        .child(setting_row("Port", None, row().gap(8.0).center_y().child(field).child(apply), t))
        .child(note)
}

fn window(s: &Ui, t: &Theme) -> El {
    let mut section = section("window-settings", "Window", t);

    if s.has_tray {
        section = section.child(toggle(
            "close-to-tray",
            "Close to tray",
            Some("Live sync keeps running"),
            s.settings.close_to_tray,
            |s| s.settings_mut(|s| s.close_to_tray = !s.close_to_tray),
            t,
        ));
    }

    section = section.child(toggle(
        "start-minimized",
        "Start minimized",
        None,
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
            section = section.child(error_note(format!("Couldn't update: {error}"), t));
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
    section("appearance-settings", "Appearance", t)
        .child(setting_row(
            "Theme",
            None,
            SegmentedControl::new(&labels, mode)
                .label("Theme")
                .fit()
                .on_pick(|s: &mut Ui, i, _| s.settings_mut(|s| s.theme = MODES[i].0))
                .build(&t.kit)
                .key("theme-mode"),
            t,
        ))
        .child(setting_row(
            "Accent",
            Some(s.settings.accent.label().into()),
            accent_picker(s.settings.accent, t),
            t,
        ))
        .child(setting_row(
            "Card position",
            Some(s.settings.card_anchor.label().into()),
            anchor_picker(s.settings.card_anchor, t),
            t,
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
        // The cards jump to the picked spot rather than fading across
        .instant_changes()
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
        return section.child(note("Shown faded behind the window.", t));
    }
    let error = s.background.error.clone().or_else(|| {
        s.background
            .loaded
            .as_ref()
            .and_then(|(_, image)| image.error())
            .map(|e| e.to_string())
    });
    if s.background.loading.is_some() {
        section = section.child(note("Loading…", t));
    }
    if let Some(error) = error {
        section = section.child(error_note(format!("Couldn't load that image: {error}"), t));
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
        .child(setting_row(
            "Fit",
            None,
            SegmentedControl::new(&labels, fit)
                .label("Background fit")
                .fit()
                .on_pick(|s: &mut Ui, i, _| s.set_image_fit(FITS[i].0))
                .build(&t.kit)
                .key("background-fit"),
            t,
        ))
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
    let image = s.background.image();
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
        row_ = row_.child(pill("remove-background", "Remove", Pill::Quiet).build(t, |s, _| s.remove_background()));
    }
    row_.child(
        pill(
            "choose-background",
            if chosen { "Change…" } else { "Choose image…" },
            Pill::Secondary,
        )
        .build(t, |s: &mut Ui, tx: &mut Tx| s.choose_background(tx)),
    )
}

/// Where the image sits, as a preview to place it on. Dragging the image moves it; dragging the
/// pin picks the point that holds still when the window resizes, the image staying put; dragging
/// the frame's corner tries the window at another size; and Ctrl + scroll or a pinch zooms
fn placement(s: &Ui, cx: &Cx<Ui>, room: f32, t: &Theme) -> Option<El> {
    let image = s.background.image()?;
    let scale = cx.env().scale;
    let own = image.size().map(|[w, h]| [w as f32 / scale, h as f32 / scale])?;
    let stage = Stage::new(room, super::window_size(cx), own);
    let k = stage.k;
    let window = stage.window(s);
    let [ox, oy] = stage.origin(window);
    let frame = window.map(|v| v * k);

    let mut wallpaper = super::wallpaper_for(s, image, window, scale);
    let scaled = |r: [f32; 4]| r.map(|v| v * k);
    let rect = scaled(wallpaper.rect);
    wallpaper.rect = rect;
    wallpaper.cover = scaled(wallpaper.cover);
    wallpaper.window = frame;
    wallpaper.scale = k;
    wallpaper.reach = k;

    // The whole image, dim, then the window over it as it shows
    let beyond = fixed()
        .pin(Some(ox + rect[0]), Some(oy + rect[1]), None, None)
        .size(rect[2], rect[3])
        .opacity(0.3)
        .pixels(image.pixels());
    // Where the pin is: dragged out past the window's edges, where it's held; else where it's kept
    let out = match s.background.grab {
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
    let shown = fixed()
        .pin(Some(ox), Some(oy), None, None)
        .size(frame[0], frame[1])
        .clip_rounded(0.0)
        .fill(t.bg)
        .child(fixed().pin(Some(0.0), Some(0.0), None, None).child(wallpaper.build()))
        // At where in the window the pin goes, once it's let go
        .children(crosshairs([at[0].clamp(0.0, frame[0]), at[1].clamp(0.0, frame[1])]));
    let outline = fixed()
        .pin(Some(ox - 1.0), Some(oy - 1.0), None, None)
        .size(frame[0] + 2.0, frame[1] + 2.0)
        .radius(2.0)
        .border(1.5, t.accent);
    // The pinned point on the image, when the image's edges keep it from reaching the pin
    let anchor = s.settings.background_anchor;
    let point = [ox + rect[0] + anchor[0] * rect[2], oy + rect[1] + anchor[1] * rect[3]];
    let held = (out.is_none() && (point[0] - ox - at[0]).hypot(point[1] - oy - at[1]) > 3.0).then(|| {
        fixed()
            .key("background-point")
            .pin(Some(point[0] - 4.0), Some(point[1] - 4.0), None, None)
            .size(8.0, 8.0)
            .radius(4.0)
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
        .child(beyond)
        .child(shown)
        .child(outline)
        .children(held)
        .child(pin_marker([ox + at[0], oy + at[1]], s.settings.background_target.is_some(), t))
        .child(trial_handle([ox + frame[0], oy + frame[1]], t));
    let surface = stage_events(surface, stage, s);

    let mut actions = row().gap(4.0).center_y();
    if s.settings.background_target.is_some() {
        actions = actions.child(pill("background-unpin", "Unpin", Pill::Quiet).build(t, move |s, _| stage.unpin(s)));
    }
    if s.background.trial.is_some() {
        actions = actions.child(pill("background-untry", "Back to this window", Pill::Quiet).build(t, |s, _| s.background.trial = None));
    }
    let hint = if s.settings.background_target.is_some() {
        "Drag to move. Drag the pin to change the anchor point. Resize from the corner to preview."
    } else {
        "Drag to move. Drag the pin to set an anchor point. Resize from the corner to preview."
    };

    let mut block = col().key("background-placement").w_full().gap(8.0).child(surface).child(
        row()
            .w_full()
            .gap(12.0)
            .items_start()
            .child(note(hint, t).grow(1.0).min_w(0.0))
            .child(actions),
    );
    // Stretched to the window and not zoomed, it's the same wherever it's placed
    if s.settings.image_fit == ImageFit::Fill && s.settings.background_zoom == 1.0 {
        block = block.child(note("Zoom to reposition a stretched image.", t));
    }
    Some(block)
}

/// A piece of the placement preview. None of it springs from where it was, as Rune's elements do,
/// or it would trail behind the image the wallpaper puts at once
fn fixed() -> El {
    el().inert().motion(rune::Spring::NONE)
}

/// Faint crosshairs across the window through `at`, so the pin reads against any image
fn crosshairs(at: [f32; 2]) -> [El; 2] {
    let hair = Color::WHITE.alpha(0.35);
    [
        fixed().pin(Some(at[0]), Some(0.0), None, Some(0.0)).w(1.0).fill(hair),
        fixed().pin(Some(0.0), Some(at[1]), Some(0.0), None).h(1.0).fill(hair),
    ]
}

/// The pin at `at` on the preview: a ring, with a dot in it once it's `targeted`, with a place of
/// its own in the window
fn pin_marker(at: [f32; 2], targeted: bool, t: &Theme) -> El {
    const PIN: f32 = 16.0;
    let pin = fixed()
        .key("background-pin")
        .pin(Some(at[0] - PIN / 2.0), Some(at[1] - PIN / 2.0), None, None)
        .size(PIN, PIN)
        .radius(PIN / 2.0)
        .center()
        .border(2.0, Color::WHITE)
        .shadow(0.0, 4.0, Color::BLACK.alpha(0.6));
    pin.children(targeted.then(|| el().size(6.0, 6.0).radius(3.0).fill(t.accent)))
}

/// The handle on the window's corner, at `corner` on the preview, to try another size by
fn trial_handle(corner: [f32; 2], t: &Theme) -> El {
    const HANDLE: f32 = 10.0;
    fixed()
        .key("background-trial-handle")
        .pin(Some(corner[0] - HANDLE / 2.0), Some(corner[1] - HANDLE / 2.0), None, None)
        .size(HANDLE, HANDLE)
        .radius(2.0)
        .fill(t.accent)
        .border(1.0, Color::WHITE)
}

/// The preview's `surface` taking the pointer, pinches and arrow keys to `stage`
fn stage_events(surface: El, stage: Stage, s: &Ui) -> El {
    surface
        // What a press would take, as the pointer moves over it, or what the drag has
        .cursor(match s.background.grab.or(s.background.hover) {
            Some(Grab::Pan(_)) if s.background.grab.is_some() => rune::Cursor::Grabbing,
            Some(Grab::Pin { .. }) => rune::Cursor::Pointer,
            Some(Grab::Resize) => rune::Cursor::ResizeNwse,
            _ => rune::Cursor::Grab,
        })
        .on_pointer_move_local(move |s: &mut Ui, p: rune::PointerEvent, _: &mut Tx| {
            let hover = Some(stage.grab_at(s, [p.x, p.y]));
            if s.background.hover != hover {
                s.background.hover = hover;
            }
        })
        .on_drag(move |s: &mut Ui, drag: &mut Drag, _: &mut Tx| stage.drag(s, drag))
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
            stage.nudge(s, by);
            true
        })
}

/// What fills the window around an image smaller than it: each choice as a tile showing the image
/// with that fill around it
fn fill_picker(s: &Ui, room: f32, t: &Theme) -> Option<El> {
    const PER_ROW: usize = 4;
    const GAP: f32 = 8.0;
    const TILE_H: [f32; 2] = [58.0, 80.0];
    let image = s.background.image()?;
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
            edges: s.background.edges,
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
                        "Not visible at this fit. Try Fit, Shrink, Original, or zoom out."
                    } else {
                        "Fills the space around the image."
                    },
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
        Update::Available { version: latest } => (format!("Version {latest} available (you have {version})"), false),
        Update::Installing => ("Installing…".to_string(), false),
        Update::Failed(e) => (format!("Couldn't update: {e}"), true),
    };
    let action = match &s.update {
        Update::Available { .. } => pill("update-action", "Update", Pill::Primary).build(t, |s, _| s.install_update()),
        update => {
            let busy = matches!(update, Update::Checking | Update::Installing);
            pill("update-action", "Check now", Pill::Secondary)
                .enabled(!busy)
                .build(t, |s, _| s.check_for_update())
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
    if let Some(edges) = s.background.edges {
        swatches.extend(edges.map(|c| c.alpha(1.0)));
    }
    let mut picker = rune_kit::ColorPicker::new(s.background_color(t.bg))
        .label("Color around the image")
        .size(room, 132.0)
        .swatches(swatches)
        .on_change(|s: &mut Ui, hsv, _| s.set_background_color(hsv))
        .hex_field(&s.background.hex, |s: &mut Ui| &mut s.background.hex);
    if s.background.sample.is_some() {
        picker = picker.eyedropper(s.background.dropper, |s: &mut Ui, _| s.background.dropper = !s.background.dropper);
    }
    col()
        .key("background-color")
        .pt(6.0)
        .gap(12.0)
        .children(s.background.dropper.then(|| dropper(s, room, t)).flatten())
        .child(picker.build(&t.kit))
}

/// The image to pick a color from with the eyedropper, as wide as the sheet: pressing picks the
/// color under the pointer, dragging picks as it goes, and letting go puts the eyedropper away
fn dropper(s: &Ui, room: f32, t: &Theme) -> Option<El> {
    let image = s.background.image()?;
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
                s.background.dropper = false;
            }
        });
    Some(
        col()
            .w_full()
            .gap(6.0)
            .items_center()
            .child(surface)
            .child(note("Click to pick a color.", t)),
    )
}
