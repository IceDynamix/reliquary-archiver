//! Draws the window: the logotype and its controls along the top, the status and delivery plates
//! anchored where the user put them, and capture files and help along the bottom. Settings and
//! logs open over it as sheets.

use parts::{Pill, pill, plate, ring_button};
use rune::{Cx, El, Tx, col, el, icon, row, text};

use super::settings::{ImageFit, ThemeMode};
use super::state::{Notice, Panel, Screen, Ui, Update};
use super::theme::{self, Theme};
use super::{background, icons};

mod delivery;
mod logs;
pub mod parts;
mod settings;
mod sky;
mod status;
pub(super) mod wallpaper;
mod warp_screen;
mod warps;

const GITHUB_URL: &str = "https://github.com/IceDynamix/reliquary-archiver";
const RELEASES_URL: &str = "https://github.com/IceDynamix/reliquary-archiver/releases/latest";
const DISCORD_URL: &str = "https://discord.gg/EbZXfRDQpu";
/// Space between the window's edge and its content
const GUTTER: f32 = 18.0;
/// The plates don't grow wider than this, in a wide window they sit where the user anchored them
const CARD_WIDTH: f32 = 480.0;

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let dark = match s.settings.theme {
        ThemeMode::System => cx.system_dark(),
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
    };
    let t = Theme::new(dark, s.settings.accent, s.background_image().is_some());
    let mut layer = Vec::new();

    let cards = parts::plate_stack("cards", cx, &t)
        .w_full()
        .max_w(CARD_WIDTH)
        .gap(22.0)
        .children(update_banner(s, cx, &t))
        .child(status::view(s, cx, &t))
        .child(delivery::view(s, cx, &t))
        .child(warps::plate_view(s, cx, &t));

    // The space between header and footer, with the plates placed in it as the settings say
    let (column, row) = s.settings.card_anchor.cell();
    let stage = col().key("stage").w_full().grow(1.0).min_h(0.0).pt(6.0);
    let stage = match row {
        0 => stage.justify_start(),
        1 => stage.justify_center(),
        _ => stage.justify_end(),
    };
    let stage = match column {
        0 => stage.items_start(),
        1 => stage.items_center(),
        _ => stage.items_end(),
    };

    let content = col()
        .key("content")
        .w_full()
        .h_full()
        .padding(GUTTER)
        .gap(14.0)
        .child(header(s, &t))
        .child(stage.child(cards))
        .child(footer(s, &t));

    // A warm light from the top right, like a nebula over the dark
    let mut page = el()
        .key("page")
        .w_full()
        .h_full()
        .fill(t.bg)
        .radial_gradient([1.0, 0.0], [620.0, 520.0], [(0.0, t.accent.alpha(0.1)), (1.0, t.accent.alpha(0.0))]);
    page = match background(s, cx, &t) {
        Some(background) => page.child(background),
        None => {
            let [w, h] = cx.layout_size("page").unwrap_or([460.0, 700.0]);
            page.child(sky::sky(w, h, &t))
        }
    };
    // The warp screen takes the window over, inside the page so it can read the window's size
    let page = if s.screen == Screen::Warps {
        page.child(warp_screen::screen(s, cx, &t).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)))
    } else {
        page.child(content)
    };

    settings::dialog(s, cx, &t, &mut layer);
    logs::dialog(s, cx, &t, &mut layer);
    if let Some(notice) = &s.notice {
        layer.push(notice_band(notice, cx, &t));
    }

    rune_kit::root(page, layer)
}

/// The background as the settings place it in a window `window` points large, on a display of
/// `scale` pixels to the point
pub(super) fn wallpaper_for<'a>(s: &Ui, image: &'a rune::Image, window: [f32; 2], scale: f32) -> Option<wallpaper::Wallpaper<'a>> {
    // The image's own size is one image pixel per device pixel; a vector image without one fills
    let own = image.size().map_or(window, |[w, h]| [w as f32 / scale, h as f32 / scale]);
    let settings = &s.settings;
    let place = |fit, zoom| background::place(fit, zoom, own, window, settings.background_anchor, settings.background_target);
    // On whole device pixels, so the image isn't softened by sitting between them
    let rect = place(settings.image_fit, settings.background_zoom);
    let snap = |v: f32| (v * scale).round() / scale;
    let rect = [snap(rect[0]), snap(rect[1]), snap(rect[2]), snap(rect[3])];
    Some(wallpaper::Wallpaper {
        pixels: image.pixels(),
        rect,
        // The blur behind covers the window however the image is zoomed
        cover: place(ImageFit::Cover, 1.0),
        window,
        fill: settings.background_fill,
        edges: s.background_edges,
        color: s.background_color(rune::Color::BLACK).color(),
        scale: 1.0,
        reach: 1.0,
    })
}

/// The user's image behind everything, where they placed it and with what they picked around it,
/// faded to the opacity they picked
fn background(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> Option<El> {
    let image = s.background_image()?;
    let window = cx.layout_size("page").unwrap_or([460.0, 700.0]);
    let scale = cx.env().scale;
    let wallpaper = wallpaper_for(s, image, window, scale)?;
    let layer = el()
        .pin(Some(0.0), Some(0.0), None, None)
        .opacity(s.settings.background_opacity)
        .child(wallpaper.build());

    // The header and footer sit right on the image, fading its edges into the window keeps them readable
    let edge = |top: bool| {
        let (from, to) = (t.bg.alpha(0.9), t.bg.alpha(0.0));
        let (fill_top, fill_bottom) = if top { (from, to) } else { (to, from) };
        let pinned = if top {
            el().pin(Some(0.0), Some(0.0), Some(0.0), None)
        } else {
            el().pin(Some(0.0), None, Some(0.0), Some(0.0))
        };
        pinned.h(96.0).gradient(fill_top, fill_bottom)
    };
    Some(
        el().key("background")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .inert()
            .child(layer)
            .child(edge(true))
            .child(edge(false)),
    )
}

fn header(s: &Ui, t: &Theme) -> El {
    let panel_button = |key: &'static str, glyph: &'static rune::Icon, name: &'static str, panel: Panel| {
        ring_button(key, glyph, name, s.panel == Some(panel), t, move |s: &mut Ui| s.toggle_panel(panel))
    };

    let logotype = row()
        .grow(1.0)
        .gap(10.0)
        .center_y()
        .child(parts::tile(
            24.0,
            31.0,
            parts::TileLook::face_up(t),
            Some(icon(&icons::ZHONG, 17.0).color(parts::dragon())),
        ))
        .child(
            col()
                .gap(1.0)
                .child(text("Reliquary Archiver").font(theme::TITLE).color(t.text))
                .child(
                    text(concat!("Version ", env!("CARGO_PKG_VERSION")))
                        .font(theme::SMALL)
                        .color(t.muted),
                ),
        );

    row()
        .key("header")
        .w_full()
        .center_y()
        .gap(8.0)
        .child(logotype)
        .child(panel_button("open-logs", &icons::LOGS, "Logs", Panel::Logs))
        .child(panel_button("open-settings", &icons::SETTINGS, "Settings", Panel::Settings))
}

fn update_banner(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> Option<El> {
    let (title, actions) = match &s.update {
        Update::Available { version } if !s.update_dismissed => (format!("Version {version} is ready to install"), vec![
            pill("update-later", "Later", None, Pill::Quiet, true, t, |s, _| {
                s.update_dismissed = true
            }),
            pill("update-now", "Update", Some(&icons::ARROW_UP), Pill::Primary, true, t, |s, _| {
                s.install_update()
            }),
        ]),
        Update::Installing => ("Installing, the archiver restarts by itself".to_string(), vec![
            rune_kit::Spinner::new().size(16.0).color(t.accent).build(&t.kit),
        ]),
        _ => return None,
    };

    Some(
        plate("update-banner", cx, t)
            .w_full()
            .child(parts::tab("update-tab", "Update", t.accent, cx, t))
            .child(
                row()
                    .w_full()
                    .pl(18.0)
                    .pr(12.0)
                    .pt(8.0)
                    .pb(12.0)
                    .gap(8.0)
                    .center_y()
                    .child(text(title).font(theme::LABEL).color(t.text).grow(1.0).min_w(0.0).truncate())
                    .children(actions),
            ),
    )
}

fn footer(s: &Ui, t: &Theme) -> El {
    // A small plate like the game's side entrances: icon, label, chevron
    let label = if s.reading_file {
        "Reading capture file…"
    } else {
        "Open capture file"
    };
    let mut open_file = parts::entry("open-capture-file", &icons::FOLDER, label, t)
        .help("Read a recording made with Wireshark or pktmon instead of capturing live");
    if s.reading_file {
        open_file = open_file.disabled(true).opacity(0.5);
    } else {
        open_file = open_file
            .focusable(true)
            .hover(t.accent.alpha(0.1))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(|s: &mut Ui, tx: &mut Tx| s.open_capture_file(tx));
    }

    let link = |key: &'static str, glyph: &'static rune::Icon, name: &'static str, url: &'static str| {
        ring_button(key, glyph, name, false, t, move |_| open_url(url))
    };

    row()
        .key("footer")
        .w_full()
        .center_y()
        .gap(8.0)
        .child(open_file)
        .child(el().grow(1.0))
        .child(link("github", &icons::GITHUB, "GitHub", GITHUB_URL))
        .child(link("discord", &icons::DISCORD, "Questions? Ask on Discord", DISCORD_URL))
}

/// What just happened, in a band across the bottom of the window
fn notice_band(notice: &Notice, cx: &Cx<Ui>, t: &Theme) -> El {
    let (glyph, tint) = if notice.error {
        (&icons::ALERT, t.bad)
    } else {
        (&icons::CHECK, t.good)
    };
    let mut text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(notice.title.clone()).font(theme::TITLE).color(t.text));
    if let Some(detail) = &notice.detail {
        text_col = text_col.child(text(detail.clone()).font(theme::SMALL).color(t.muted).wrap());
    }
    let mut band = row()
        .w_full()
        .padding(14.0)
        .gap(12.0)
        .center_y()
        .child(icon(glyph, 18.0).color(tint))
        .child(text_col);
    if let Some(file) = notice.file.clone() {
        band = band.child(pill(
            "reveal-file",
            "Show in folder",
            None,
            Pill::Secondary,
            true,
            t,
            move |_, _| super::services::reveal(&file),
        ));
    }
    band = band.child(ring_button("dismiss-notice", &icons::CLOSE, "Dismiss", false, t, |s| {
        s.notice = None
    }));

    el().key("notice-layer")
        .pin(Some(GUTTER), None, Some(GUTTER), Some(64.0))
        .items_center()
        .child(
            plate(&format!("notice-{}", notice.id), cx, t)
                .w_full()
                .max_w(CARD_WIDTH)
                .child(band),
        )
}

fn open_url(url: &str) {
    if let Err(e) = open::that(url) {
        tracing::error!(%e, "failed to open {}", url);
    }
}
