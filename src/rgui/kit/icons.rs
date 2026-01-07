//! SVG icon components.
//!
//! Provides reusable icon elements built from SVG paths.

use raxis::layout::model::{BoxAmount, Color, Element, StrokeLineCap, StrokeLineJoin};
use raxis::widgets::svg::Svg;
use raxis::widgets::svg::ViewBox;
use raxis::widgets::svg_path::{ColorChoice, SvgPath};
use raxis::widgets::button::Button;
use raxis::{svg, svg_path, w_id, SvgPathCommands};

use crate::rgui::theme::{BORDER_RADIUS, PAD_MD};
use crate::rgui::messages::RootMessage;

/// Creates a refresh/reload icon (circular arrows).
pub fn refresh_icon<M>() -> Element<M> {
    SvgPath::new(
        svg![svg_path!(
            "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8 M21 3v5h-5 M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16 M8 16H3v5"
        )],
        ViewBox::new(24.0, 24.0),
    )
    .with_size(32.0, 32.0)
    .with_stroke(Color::WHITE)
    .with_stroke_width(2.0)
    .with_stroke_cap(StrokeLineCap::Round)
    .with_stroke_join(StrokeLineJoin::Round)
    .as_element(w_id!())
    .with_padding(PAD_MD)
}

/// Creates a close/X icon.
pub fn x_icon<M>() -> Element<M> {
    SvgPath::new(svg![svg_path!("M18 6 6 18"), svg_path!("m6 6 12 12"),], ViewBox::new(24.0, 24.0))
        .with_size(16.0, 16.0)
        .with_stroke(ColorChoice::CurrentColor)
        .with_stroke_width(2.0)
        .with_stroke_cap(StrokeLineCap::Round)
        .with_stroke_join(StrokeLineJoin::Round)
        .as_element(w_id!())
        .with_padding(PAD_MD)
}

/// Creates a settings/cog icon.
pub fn cog_icon<M>() -> Element<M> {
    SvgPath::new(
        svg![
            svg_path!("M11 10.27 7 3.34"),
            svg_path!("m11 13.73-4 6.93"),
            svg_path!("M12 22v-2"),
            svg_path!("M12 2v2"),
            svg_path!("M14 12h8"),
            svg_path!("m17 20.66-1-1.73"),
            svg_path!("m17 3.34-1 1.73"),
            svg_path!("M2 12h2"),
            svg_path!("m20.66 17-1.73-1"),
            svg_path!("m20.66 7-1.73 1"),
            svg_path!("m3.34 17 1.73-1"),
            svg_path!("m3.34 7 1.73 1"),
            SvgPathCommands::Circle {
                cx: 12.0,
                cy: 12.0,
                r: 2.0
            },
            SvgPathCommands::Circle {
                cx: 12.0,
                cy: 12.0,
                r: 8.0
            },
        ],
        ViewBox::new(24.0, 24.0),
    )
    .with_size(32.0, 32.0)
    .with_stroke(ColorChoice::CurrentColor)
    .with_stroke_width(2.0)
    .with_stroke_cap(StrokeLineCap::Round)
    .with_stroke_join(StrokeLineJoin::Round)
    .as_element(w_id!())
    .with_padding(PAD_MD)
}

/// Creates a GitHub social button that opens the project repository.
pub fn github_button() -> Element<RootMessage> {
    Button::new()
        .with_bg_color(Color::from(0x181717FF))
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler(move |_, shell| {
            if let Err(e) = open::that("https://github.com/IceDynamix/reliquary-archiver") {
                tracing::error!("Failed to open GitHub link: {}", e);
            }
        })
        .as_element(
            w_id!(),
            Svg::new(include_str!("../../../assets/github.svg"))
                .with_size(32.0, 32.0)
                .with_recolor(Color::WHITE)
                .as_element(w_id!()),
        )
        .with_padding(PAD_MD)
}

/// Creates a Discord social button that opens the Discord server invite.
pub fn discord_button() -> Element<RootMessage> {
    Button::new()
        .with_bg_color(Color::from(0x5865F2FF))
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler(move |_, shell| {
            if let Err(e) = open::that("https://discord.gg/EbZXfRDQpu") {
                tracing::error!("Failed to open Discord link: {}", e);
            }
        })
        .as_element(
            w_id!(),
            Svg::new(include_str!("../../../assets/discord.svg"))
                .with_size(32.0, 32.0)
                .with_recolor(Color::WHITE)
                .as_element(w_id!()),
        )
        .with_padding(PAD_MD)
}
