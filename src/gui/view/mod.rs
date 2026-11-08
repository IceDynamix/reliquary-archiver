//! Draws the window: a header, where capturing stands, the two ways to get the data into the
//! optimizer, and a footer for capture files and help. Settings and logs open over it.

use rune::{Cx, El, SurfaceFit, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};

use super::icons;
use super::settings::ImageFit;
use super::state::{Notice, Panel, Ui, Update};
use super::theme::{self, Theme};

mod delivery;
mod logs;
mod settings;
mod status;

const GITHUB_URL: &str = "https://github.com/IceDynamix/reliquary-archiver";
const RELEASES_URL: &str = "https://github.com/IceDynamix/reliquary-archiver/releases/latest";
const DISCORD_URL: &str = "https://discord.gg/EbZXfRDQpu";
/// Space between the window's edge and its content
const GUTTER: f32 = 16.0;
/// The cards don't grow wider than this, in a wide window they sit where the user anchored them
const CARD_WIDTH: f32 = 480.0;

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let t = Theme::new(cx.system_dark(), s.background_image().is_some());
    let background = background(s, cx, &t);
    let mut layer = Vec::new();

    let cards = col()
        .key("cards")
        .w_full()
        .max_w(CARD_WIDTH)
        .gap(12.0)
        .children(update_banner(s, &t))
        .child(status::view(s, &t))
        .child(delivery::view(s, &t));

    // The space between header and footer, with the cards placed in it as the settings say
    let (column, row) = s.settings.card_anchor.cell();
    let stage = col().key("stage").w_full().grow(1.0).min_h(0.0);
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
        .gap(12.0)
        .child(header(s, &t))
        .child(stage.child(cards))
        .child(footer(s, &t));
    let page = el().key("page").w_full().h_full().fill(t.bg).children(background).child(content);

    settings::dialog(s, &t, &mut layer);
    logs::dialog(s, &t, &mut layer);
    if let Some(notice) = &s.notice {
        layer.push(notice_toast(notice, &t));
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
        let (from, to) = (t.bg.alpha(0.85), t.bg.alpha(0.0));
        let (fill_top, fill_bottom) = if top { (from, to) } else { (to, from) };
        let pinned = if top {
            el().pin(Some(0.0), Some(0.0), Some(0.0), None)
        } else {
            el().pin(Some(0.0), None, Some(0.0), Some(0.0))
        };
        pinned.h(80.0).gradient(fill_top, fill_bottom)
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
    let k = &t.kit;
    let panel_button = |key: &'static str, glyph: &'static rune::Icon, name: &'static str, panel: Panel| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::Ghost)
            .label(name)
            .help(name)
            .expanded(s.panel == Some(panel))
            .on_press(move |s: &mut Ui| s.toggle_panel(panel))
            .build(k)
            .key(key)
    };

    row()
        .key("header")
        .w_full()
        .center_y()
        .gap(8.0)
        .child(
            row()
                .grow(1.0)
                .gap(8.0)
                .center_y()
                .child(text("Reliquary Archiver").font(theme::TITLE).color(t.text))
                .child(text(concat!("v", env!("CARGO_PKG_VERSION"))).font(theme::SMALL).color(t.faint)),
        )
        .child(panel_button("open-logs", &icons::LOGS, "Logs", Panel::Logs))
        .child(panel_button("open-settings", &icons::SETTINGS, "Settings", Panel::Settings))
}

fn update_banner(s: &Ui, t: &Theme) -> Option<El> {
    let k = &t.kit;
    let (title, actions) = match &s.update {
        Update::Available { version } if !s.update_dismissed => (
            format!("Version {version} is available"),
            vec![
                Button::new("Later")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut Ui| s.update_dismissed = true)
                    .build(k)
                    .key("update-later"),
                Button::new("Update")
                    .variant(ButtonVariant::Contrast)
                    .on_press(|s: &mut Ui| s.install_update())
                    .build(k)
                    .key("update-now"),
            ],
        ),
        Update::Installing => (
            "Installing the update, the app restarts by itself".to_string(),
            vec![rune_kit::Spinner::new().size(16.0).color(t.accent).build(k)],
        ),
        _ => return None,
    };

    Some(
        row()
            .key("update-banner")
            .w_full()
            .h(44.0)
            .pl(12.0)
            .pr(6.0)
            .gap(8.0)
            .center_y()
            .radius(10.0)
            .fill(t.accent_soft)
            .child(icon(&icons::ARROW_UP, 16.0).color(t.accent))
            .child(text(title).font(theme::LABEL).color(t.text).grow(1.0).truncate())
            .children(actions),
    )
}

fn footer(s: &Ui, t: &Theme) -> El {
    let k = &t.kit;
    let open_file = if s.reading_file {
        Button::new("Reading capture file…")
            .variant(ButtonVariant::GhostMuted)
            .disabled(true)
    } else {
        Button::new("Open capture file…")
            .variant(ButtonVariant::GhostMuted)
            .icon(&icons::FOLDER)
            .help("Read a recording made with Wireshark or pktmon instead of capturing live")
            .on_press_tx(|s: &mut Ui, tx: &mut Tx| s.open_capture_file(tx))
    };
    let link = |key: &'static str, glyph: &'static rune::Icon, name: &'static str, url: &'static str| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(name)
            .help(name)
            .on_press(move |_: &mut Ui| open_url(url))
            .build(k)
            .key(key)
    };

    row()
        .key("footer")
        .w_full()
        .center_y()
        .gap(4.0)
        .child(open_file.build(k).key("open-capture-file"))
        .child(el().grow(1.0))
        .child(link("github", &icons::GITHUB, "GitHub", GITHUB_URL))
        .child(link("discord", &icons::DISCORD, "Questions? Ask on Discord", DISCORD_URL))
}

fn notice_toast(notice: &Notice, t: &Theme) -> El {
    let k = &t.kit;
    let (glyph, tint) = if notice.error {
        (&icons::ALERT, t.bad)
    } else {
        (&icons::CHECK, t.good)
    };
    let mut toast = rune_kit::Toast::new(format!("notice-{}", notice.id), notice.title.clone())
        .lead(icon(glyph, 16.0).color(tint))
        .on_close(|s: &mut Ui| s.notice = None);
    if let Some(detail) = &notice.detail {
        toast = toast.description(detail.clone());
    }
    if let Some(file) = notice.file.clone() {
        toast = toast.action(k, "Show in folder", false, move |_: &mut Ui, _: &mut Tx| {
            super::services::reveal(&file)
        });
    }
    el().key("notice")
        .pin(Some(GUTTER), None, Some(GUTTER), Some(56.0))
        .child(toast.build(k).w_full())
}

fn open_url(url: &str) {
    if let Err(e) = open::that(url) {
        tracing::error!(%e, "failed to open {}", url);
    }
}

/// A rounded surface that groups related content
fn card(t: &Theme) -> El {
    let card = col().w_full().radius(12.0).fill(t.card).border(1.0, t.card_border);
    if t.frosted { card.backdrop_blur(24.0) } else { card }
}
