//! File download component.
//!
//! Provides a download button card that displays file information and
//! allows the user to save the export to disk.

use std::path::PathBuf;

use raxis::layout::helpers::{center, row};
use raxis::layout::model::{
    Alignment, BackdropFilter, Border, BorderRadius, BoxAmount, Color, Direction, Element, Sizing, StrokeLineCap, StrokeLineJoin,
};
use raxis::runtime::task::Task;
use raxis::widgets::button::Button;
use raxis::widgets::svg::ViewBox;
use raxis::widgets::svg_path::SvgPath;
use raxis::widgets::text::{ParagraphAlignment, Text, TextAlignment};
use raxis::widgets::widget;
use raxis::{svg, svg_path, w_id, HookManager};

use crate::rgui::kit::tooltip::{error_tooltip, info_tooltip, success_tooltip, warning_tooltip};
use crate::rgui::state::{FileContainer, FileExtensions};
use crate::rgui::theme::{
    BORDER_COLOR, BORDER_RADIUS, CARD_BACKGROUND, PAD_MD, PAD_SM, PRIMARY_COLOR, SPACE_MD, SPACE_SM, SUCCESS_COLOR, TEXT_MUTED,
};
use crate::scopefns::Also;

/// Messages for file download operations.
#[derive(Debug, Clone)]
pub enum Message {
    PickPathForFile(FileContainer),
    SaveFile(FileContainer, PathBuf),
}

/// Result of processing a file download message.
#[derive(Debug)]
pub enum Action {
    /// No action needed
    None,
    /// Run an async task
    Run(Task<Message>),
}

/// Formats a file size in bytes to a human-readable string.
fn format_file_size(size: usize) -> String {
    let size_f = size as f64;
    if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.2} KB", size_f / 1024.0)
    } else {
        format!("{:.2} MB", size_f / 1024.0 / 1024.0)
    }
}

/// Creates a download arrow icon with the specified stroke color.
fn download_arrow_icon<M>(stroke: Color) -> Element<M> {
    SvgPath::new(
        svg![svg_path!("M12 15V3M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5")],
        ViewBox::new(24.0, 24.0),
    )
    .with_size(32.0, 32.0)
    .with_stroke(stroke)
    .with_stroke_width(2.0)
    .with_stroke_cap(StrokeLineCap::Round)
    .with_stroke_join(StrokeLineJoin::Round)
    .as_element(w_id!())
    .with_padding(PAD_MD)
}

/// Renders the file download card component.
///
/// Displays a download button and file information. Shows a warning tooltip
/// if the export is out of date.
///
/// # Arguments
/// * `file` - The file container with export data, or None if not ready
/// * `out_of_date` - Whether the export needs to be refreshed
/// * `hook` - The hook manager for widget state
pub fn download_view<PMsg: Send + Clone + std::fmt::Debug + 'static>(
    file: Option<&FileContainer>,
    out_of_date: bool,
    hook: &mut HookManager<PMsg>,
) -> Element<PMsg> {
    let download_button = Button::new()
        .with_bg_color(PRIMARY_COLOR)
        .with_border_radius(BORDER_RADIUS)
        .with_border(1.0, Color::from(0x00000033))
        .enabled(file.is_some())
        .with_click_handler({
            let file = file.map(|f| f.clone());
            move |_, shell| {
                if let Some(ref file) = file {
                    shell.dispatch_task(
                        Task::future(
                            rfd::AsyncFileDialog::new()
                                .set_file_name(&file.name)
                                .add_filter(&file.ext.description, &file.ext.extensions)
                                .save_file(),
                        )
                        .and_consume({
                            let file = file.clone();
                            move |picked_file| {
                                if let Err(e) = std::fs::write(&picked_file.path().to_path_buf(), &file.content) {
                                    eprintln!("Failed to save file: {}", e);
                                }
                            }
                        }),
                    );
                }
            }
        })
        .as_element(
            w_id!(),
            download_arrow_icon(if file.is_some() { Color::WHITE } else { Color::from(0x0000003F) }),
        );

    // Create tooltip-wrapped button elements
    let download_button = warning_tooltip(
        download_button,
        "Export is out of date. Please refresh.".to_string(),
        out_of_date,
        hook,
    );

    let file_info = if let Some(file) = file {
        Element {
            children: vec![
                Text::new(file.name.clone()).with_font_size(14.0).into(),
                Text::new(format_file_size(file.content.len()))
                    .with_font_size(12.0)
                    .with_color(TEXT_MUTED)
                    .into(),
            ],
            ..Default::default()
        }
        .with_id(w_id!())
        .with_direction(Direction::TopToBottom)
        .with_width(Sizing::grow())
        .with_height(Sizing::fit())
        .with_padding(PAD_SM)
        .with_child_gap(SPACE_SM)
        .with_axis_align_content(Alignment::Center)
    } else {
        Text::new("Export not ready")
            .with_font_size(14.0)
            .with_color(TEXT_MUTED)
            .with_text_alignment(TextAlignment::Center)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .as_element()
            .with_width(Sizing::grow())
            .with_height(Sizing::grow())
    };

    Element {
        id: Some(w_id!()),
        width: Sizing::fixed(400.0),
        background_color: Some(Color::from(CARD_BACKGROUND)),
        backdrop_filter: Some(BackdropFilter::blur(10.0)),
        border_radius: Some(BorderRadius::all(BORDER_RADIUS)),
        border: Some(Border {
            color: BORDER_COLOR,
            ..Default::default()
        }),
        child_gap: SPACE_SM,
        cross_align_items: Alignment::Center,
        // children: {
        //     let mut children = button_elements;
        //     children.push(file_info);
        //     children
        // },
        children: vec![download_button, file_info],
        snap: true,
        ..Default::default()
    }
}
