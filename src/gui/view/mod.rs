//! Draws the window: the logotype and its controls along the top, the status and delivery plates
//! anchored where the user put them, and capture files and help along the bottom. Settings and
//! logs open over it as sheets.

use rune::{Cx, El, SurfaceFit, Tx, col, el, icon, row, text};

use super::icons;
use super::settings::{ImageFit, ThemeMode};
use super::state::{LiveSync, Notice, Panel, Ui, Update};
use super::theme::{self, Theme};
use parts::{Pill, kicker, pill, plate, ring_button};

mod delivery;
mod logs;
pub mod parts;
mod settings;
mod status;

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

    let cards = col()
        .key("cards")
        .w_full()
        .max_w(CARD_WIDTH)
        .gap(22.0)
        .children(update_banner(s, cx, &t))
        .child(status::view(s, cx, &t))
        .child(delivery::view(s, cx, &t));

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
        .child(footer(s, cx, &t));

    // A warm light from the top right, like a nebula over the dark
    let mut page = el().key("page").w_full().h_full().fill(t.bg).radial_gradient(
        [1.0, 0.0],
        [620.0, 520.0],
        [(0.0, t.accent.alpha(0.1)), (1.0, t.accent.alpha(0.0))],
    );
    page = match background(s, cx, &t) {
        Some(background) => page.child(background),
        None => page.child(el().absolute(-250.0, 90.0).child(parts::astrolabe(560.0, &t))),
    };
    let page = page.child(content);

    settings::dialog(s, cx, &t, &mut layer);
    logs::dialog(s, cx, &t, &mut layer);
    if let Some(notice) = &s.notice {
        layer.push(notice_band(notice, cx, &t));
    }

    rune_kit::root(page, layer)
}

/// The user's image behind everything, faded to the opacity they picked
fn background(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> Option<El> {
    let image = s.background_image()?;
    let pixels = image.pixels();
    let layer = el().w_full().h_full().opacity(s.settings.background_opacity);

    let layer = match s.settings.image_fit {
        ImageFit::Fill => layer.pixels(pixels),
        ImageFit::Contain => layer.pixels_fit(pixels, SurfaceFit::Fit),
        ImageFit::Cover => layer.pixels_fit(pixels, SurfaceFit::Fill),
        fit @ (ImageFit::None | ImageFit::ScaleDown) => {
            // The image's own size is one image pixel per device pixel
            let [width, height] = image.size()?;
            let scale = cx.env().scale;
            let (width, height) = (width as f32 / scale, height as f32 / scale);
            let too_big = cx
                .layout_size("page")
                .is_none_or(|[window_width, window_height]| width > window_width || height > window_height);
            if fit == ImageFit::ScaleDown && too_big {
                layer.pixels_fit(pixels, SurfaceFit::Fit)
            } else {
                layer.center().child(el().size(width, height).pixels(pixels))
            }
        }
    };

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
        .child(icon(&icons::EMBLEM, 30.0).color(t.accent))
        .child(
            col()
                .gap(1.0)
                .child(kicker(concat!("Reliquary · v", env!("CARGO_PKG_VERSION")), t.muted))
                .child(text("Archiver").font(theme::TITLE).color(t.text)),
        );

    row()
        .key("header")
        .w_full()
        .center_y()
        .gap(8.0)
        .child(logotype)
        .child(sync_counter(s, t))
        .child(panel_button("open-logs", &icons::LOGS, "Logs", Panel::Logs))
        .child(panel_button("open-settings", &icons::SETTINGS, "Settings", Panel::Settings))
}

/// How many optimizers listen, in a counter like the game's top bar
fn sync_counter(s: &Ui, t: &Theme) -> El {
    let (label, tint) = match &s.sync {
        LiveSync::Running { clients: 0, .. } => ("Not linked".to_string(), t.faint),
        LiveSync::Running { clients, .. } => (format!("{clients} linked"), t.good),
        LiveSync::Starting => ("Starting".to_string(), t.faint),
        LiveSync::Failed(_) => ("Sync off".to_string(), t.bad),
        LiveSync::Closed => ("Paused".to_string(), t.faint),
    };
    row()
        .key("sync-counter")
        .h(28.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(14.0)
        .fill(t.well)
        .border(1.0, t.line)
        .mr(4.0)
        .help("Optimizers receiving live sync")
        .child(el().size(6.0, 6.0).radius(3.0).fill(tint))
        .child(text(label).font(theme::LABEL).color(if tint == t.faint { t.muted } else { tint }))
}

fn update_banner(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> Option<El> {
    let (title, actions) = match &s.update {
        Update::Available { version } if !s.update_dismissed => (
            format!("Version {version} is ready to install"),
            vec![
                pill("update-later", "Later", None, Pill::Quiet, true, t, |s, _| {
                    s.update_dismissed = true
                }),
                pill("update-now", "Update", Some(&icons::ARROW_UP), Pill::Primary, true, t, |s, _| {
                    s.install_update()
                }),
            ],
        ),
        Update::Installing => (
            "Installing, the archiver restarts by itself".to_string(),
            vec![rune_kit::Spinner::new().size(16.0).color(t.accent).build(&t.kit)],
        ),
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

fn footer(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
    // A small plate like the game's side entrances: icon, label, chevron
    let label = if s.reading_file {
        "Reading capture file…"
    } else {
        "Open capture file"
    };
    let mut open_file = plate("open-capture-file", cx, t)
        .role("button")
        .label(label)
        .help("Read a recording made with Wireshark or pktmon instead of capturing live")
        .child(
            row()
                .h(38.0)
                .pl(14.0)
                .pr(12.0)
                .gap(10.0)
                .center_y()
                .child(icon(&icons::FOLDER, 16.0).color(t.accent))
                .child(text(label).font(theme::LABEL).color(t.text))
                .child(icon(&icons::CHEVRON, 14.0).color(t.accent)),
        );
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
