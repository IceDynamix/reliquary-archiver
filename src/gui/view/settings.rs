//! The settings sheet. Every change applies and saves at once, except the port, which is only
//! saved once the server could open it.

use rune::{Color, Cx, Drag, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{ControlSize, SegmentedControl, Switch, TextInput};

use super::parts::{Pill, Sheet, TileLook, kicker, pill, sheet, tile};
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
        .max_h(440.0)
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

/// A face down tile from each accent's set, its back in that set's colour. The chosen one stands
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
    let k = &t.kit;
    let path = &s.settings.background_image;
    let chosen = !path.is_empty();
    let name = std::path::Path::new(path)
        .file_name()
        .map_or_else(|| "None".to_string(), |name| name.to_string_lossy().into_owned());

    let mut picker = row().gap(8.0).center_y().child(
        text(name)
            .key("background-name")
            .font(theme::BODY)
            .color(if chosen { t.text } else { t.faint })
            .truncate()
            .grow(1.0)
            .min_w(0.0),
    );
    if chosen {
        picker = picker.child(pill("remove-background", "Remove", None, Pill::Quiet, true, t, |s, _| {
            s.remove_background()
        }));
    }
    picker = picker.child(pill(
        "choose-background",
        if chosen { "Change…" } else { "Choose image…" },
        None,
        Pill::Secondary,
        true,
        t,
        |s: &mut Ui, tx: &mut Tx| s.choose_background(tx),
    ));

    let mut section = section("background-settings", "Background", t).child(picker);
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
                        .build(k)
                        .key("background-fit"),
                ),
        )
        .children(position(s, cx, t))
        .child(fill_picker(s, t))
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(text("Opacity").font(theme::BODY).color(t.text).w(64.0))
                .child(slider(opacity, t).grow(1.0))
                .child(
                    text(format!("{}%", (opacity * 100.0).round()))
                        .key("opacity-value")
                        .font(theme::LABEL)
                        .color(t.muted)
                        .w(36.0),
                ),
        )
}

/// A horizontal slider over 0 to 1, dragged with the pointer or stepped with the arrow keys
fn slider(value: f32, t: &Theme) -> El {
    const KNOB: f32 = 14.0;
    let set = |s: &mut Ui, drag: &mut Drag, _: &mut Tx| {
        let usable = (drag.w - KNOB).max(1.0);
        s.set_background_opacity((drag.x - KNOB / 2.0) / usable);
    };
    let track = |fill| el().h(4.0).radius(2.0).fill(fill);
    row()
        .key("opacity-slider")
        .role("slider")
        .label("Background opacity")
        .range_value(f64::from(value), 0.0, 1.0)
        .focusable(true)
        .h(24.0)
        .min_w(0.0)
        .center_y()
        .cursor(rune::Cursor::Pointer)
        .on_drag(set)
        .on_step(move |s: &mut Ui, delta, _| s.set_background_opacity(s.settings.background_opacity + delta as f32 * 0.05))
        .child(track(t.accent).w_frac(value))
        .child(
            el().size(KNOB, KNOB)
                .radius(KNOB / 2.0)
                .fill(t.primary)
                .border(2.0, t.accent)
                .shadow(0.0, 8.0, t.accent.alpha(0.45))
                .mx(-KNOB / 2.0),
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

/// How tall the position pickers' image and window are
const PICKER_HEIGHT: f32 = 104.0;
/// How far an arrow key moves a point
const POINT_STEP: f32 = 0.05;

/// Where the image sits: the point of it that matters, picked on the image, and where that point
/// goes, picked on a small window that shows the result, fill and all
fn position(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> Option<El> {
    let image = s.background_image()?;
    let [iw, ih] = image.size().map(|[w, h]| [w as f32, h as f32])?;
    let window = cx.layout_size("page").unwrap_or([460.0, 700.0]);
    let stretched = s.settings.image_fit == ImageFit::Fill;
    let anchor = s.settings.background_anchor;
    let follows = s.settings.background_target.is_none();
    let target = s.settings.background_target.unwrap_or(anchor);

    // The image, its point marked
    let image_w = (PICKER_HEIGHT * iw / ih).min(200.0);
    let image_h = image_w * ih / iw;
    let picture = point_picker(
        "background-anchor",
        "Point in the image",
        [image_w, image_h],
        anchor,
        t,
        |s: &mut Ui, at| s.set_background_anchor(at),
        |s: &Ui| s.settings.background_anchor,
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert().pixels(image.pixels()),
    );

    // The window, small, with the image where it'll go and the fill around it
    let k = PICKER_HEIGHT / window[1];
    let frame = [window[0] * k, PICKER_HEIGHT];
    let mut shown = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert().fill(t.bg);
    if let Some(mut wallpaper) = super::wallpaper_for(s, image, window, cx.env().scale) {
        let scaled = |r: [f32; 4]| r.map(|v| v * k);
        wallpaper.rect = scaled(wallpaper.rect);
        wallpaper.cover = scaled(wallpaper.cover);
        wallpaper.window = frame;
        wallpaper.scale = k;
        shown = shown.child(wallpaper.build());
    }
    let preview = point_picker(
        "background-target",
        "Where it goes in the window",
        frame,
        target,
        t,
        |s: &mut Ui, at| s.set_background_target(Some(at)),
        |s: &Ui| s.settings.background_target.unwrap_or(s.settings.background_anchor),
        shown,
    );

    let labelled = |picker: El, label: &'static str| {
        col()
            .gap(6.0)
            .items_center()
            .child(picker)
            .child(text(label).font(theme::SMALL).color(t.muted))
    };
    let pickers = row()
        .w_full()
        .gap(16.0)
        .items_end()
        .child(labelled(picture, "The part that matters"))
        .child(labelled(preview, "Where it goes"));

    let mut section = col()
        .key("background-position")
        .w_full()
        .gap(10.0)
        .child(text("Position").font(theme::BODY).color(t.text))
        .child(pickers)
        .child(toggle(
            "background-follows",
            "Same place in the window as in the image",
            None,
            follows,
            |s| {
                let next = match s.settings.background_target {
                    Some(_) => None,
                    None => Some(s.settings.background_anchor),
                };
                s.set_background_target(next)
            },
            t,
        ));
    if stretched {
        section = section.opacity(0.45).child(note(
            "Stretched to the window, the whole image shows wherever it's placed.",
            false,
            t,
        ));
    }
    Some(section)
}

/// A surface `size` points large to pick a point on, by dragging or with the arrow keys: `content`
/// in it, and the point at `at` (0 to 1 across and down) marked over that
fn point_picker(
    key: &'static str,
    label: &'static str,
    size: [f32; 2],
    at: [f32; 2],
    t: &Theme,
    set: fn(&mut Ui, [f32; 2]),
    get: fn(&Ui) -> [f32; 2],
    content: El,
) -> El {
    const MARK: f32 = 14.0;
    let mark = el()
        .key(format!("{key}-mark"))
        .pin(Some(at[0] * size[0] - MARK / 2.0), Some(at[1] * size[1] - MARK / 2.0), None, None)
        .size(MARK, MARK)
        .radius(MARK / 2.0)
        .inert()
        .border(2.0, Color::WHITE)
        .shadow(0.0, 4.0, Color::BLACK.alpha(0.6))
        .child(
            el().pin(Some(MARK / 2.0 - 2.0), Some(MARK / 2.0 - 2.0), None, None)
                .size(4.0, 4.0)
                .radius(2.0)
                .fill(t.accent),
        );
    el().key(key)
        .size(size[0], size[1])
        .radius(6.0)
        .clip_rounded(6.0)
        .border(1.0, t.accent_line)
        .role("slider")
        .label(label)
        .help(format!("{}% across, {}% down", (at[0] * 100.0).round(), (at[1] * 100.0).round()))
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
        .child(mark)
}

/// What fills the window around an image smaller than it, as chips to pick from
fn fill_picker(s: &Ui, t: &Theme) -> El {
    let covers = matches!(s.settings.image_fit, ImageFit::Cover | ImageFit::Fill);
    let chips = BackgroundFill::ALL.into_iter().map(|fill| {
        let on = fill == s.settings.background_fill;
        el().key(format!("fill-{}", fill.label().to_lowercase().replace([' ', ','], "-")))
            .h(28.0)
            .px(12.0)
            .center()
            .radius(14.0)
            .role("radio")
            .label(fill.label())
            .checked(on)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .fill(if on { t.accent.alpha(0.18) } else { t.well })
            .border(1.0, if on { t.accent } else { t.line })
            .hover(t.accent.alpha(0.12))
            .on_press(move |s: &mut Ui| s.set_background_fill(fill))
            .child(text(fill.label()).font(theme::LABEL).color(if on { t.text } else { t.muted }))
    });
    let mut picker = col()
        .key("background-fill")
        .w_full()
        .gap(8.0)
        .child(text("Around the image").font(theme::BODY).color(t.text))
        .child(
            row()
                .role("radiogroup")
                .label("Around the image")
                .focus_group()
                .focus_follows()
                .w_full()
                .flex_wrap()
                .gap(6.0)
                .children(chips),
        );
    if covers {
        picker = picker.opacity(0.45).child(note(
            "The image covers the window at this fit, there's nothing around it.",
            false,
            t,
        ));
    }
    picker
}
