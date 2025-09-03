use std::{cell::RefCell, path::PathBuf, rc::Rc};

use raxis::{
    layout::{
        helpers::{row, ElementAlignmentExt, Rule},
        model::{
            Alignment2D, Border, BorderRadius, BoxAmount, Direction, Element, FloatingConfig, HorizontalAlignment, Offset2D, Sizing,
            VerticalAlignment,
        },
    },
    runtime::task::Task,
    svg_path, w_id,
    widgets::{
        button::Button,
        mouse_area::{MouseArea, MouseAreaEvent},
        svg::ViewBox,
        svg_path::SvgPath,
        text::{ParagraphAlignment, Text},
        widget, Color,
    },
    HookManager,
};

use crate::{
    rgui::{
        FileContainer, FileExtensions, BACKGROUND_LIGHT, BORDER_COLOR, BORDER_RADIUS, PAD_MD, PAD_SM, PRIMARY_COLOR, SPACE_MD, SPACE_SM,
        SUCCESS_COLOR, TEXT_MUTED,
    },
    scopefns::Also,
};

#[derive(Debug, Clone)]
pub enum FileDownloadMessage {
    PickPathForFile(FileContainer),
    SaveFile(FileContainer, PathBuf),
}

pub enum FileDownloadAction {
    None,
    Run(Task<FileDownloadMessage>),
}

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

// Download arrow icon SVG path
fn download_arrow_icon<M>() -> Element<M> {
    SvgPath::new(
        svg_path!("M12 3v13m0 0-4-4m4 4 4-4M2 17l.621 2.485A2 2 0 0 0 4.561 21h14.878a2 2 0 0 0 1.94-1.515L22 17"),
        ViewBox::new(24.0, 24.0),
    )
    .with_size(20.0, 20.0)
    .with_stroke(Color::WHITE)
    .with_stroke_width(2.0)
    .as_element(w_id!())
}

pub fn download_view<PMsg: Send + Clone + std::fmt::Debug + 'static>(
    file: Option<&FileContainer>,
    message_mapper: impl Fn(FileDownloadMessage) -> PMsg + Send + Clone + 'static,
    out_of_date: bool,
    hook: &mut HookManager<PMsg>,
) -> Element<PMsg> {
    let mut hook = hook.instance(w_id!());
    let hover_state = hook.use_hook(|| Rc::new(RefCell::new(false)));

    let download_button_id = w_id!();
    let download_button = Button::new()
        .with_bg_color(PRIMARY_COLOR)
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler({
            let file = file.map(|f| f.clone());
            let message_mapper = message_mapper.clone();
            move |_, shell| {
                if let Some(ref file) = file {
                    shell.publish(message_mapper(FileDownloadMessage::PickPathForFile(file.clone())));
                }
            }
        })
        .as_element(download_button_id, download_arrow_icon());

    let button_with_hover = (MouseArea::new({
        let hover_state = hover_state.clone();
        move |event, _shell| match event {
            MouseAreaEvent::MouseEntered { .. } => {
                *hover_state.borrow_mut() = true;
                None
            }
            MouseAreaEvent::MouseLeft { .. } => {
                *hover_state.borrow_mut() = false;
                None
            }
            _ => None,
        }
    }))
    .as_element(w_id!(), download_button);

    let tooltip = if out_of_date && *hover_state.borrow() {
        Some(
            Element {
                children: vec![Text::new("Export is out of date. Please refresh.")
                    .with_font_size(12.0)
                    .with_color(Color::from(0x856404FF))
                    .with_paragraph_alignment(ParagraphAlignment::Center)
                    .into()],
                ..Default::default()
            }
            .with_id(w_id!())
            .with_width(Sizing::fit())
            .with_height(Sizing::fit())
            .with_background_color(Color::from(0xFFF3CDFF)) // Light yellow warning background
            .with_padding(BoxAmount::all(PAD_SM))
            .with_border_radius(BorderRadius::all(BORDER_RADIUS))
            .with_border(Border {
                width: 1.0,
                color: Color::from(0xF0AD4EFF), // Warning border color
                ..Default::default()
            })
            .with_floating(FloatingConfig {
                offset: Some(Offset2D {
                    x: Some(0.0),
                    y: Some(-4.0),
                }),
                anchor_id: Some(download_button_id),
                anchor: Some(Alignment2D {
                    x: Some(HorizontalAlignment::Center),
                    y: Some(VerticalAlignment::Top),
                }),
                align: Some(Alignment2D {
                    x: Some(HorizontalAlignment::Center),
                    y: Some(VerticalAlignment::Bottom),
                }),
                ..Default::default()
            }),
        )
    } else {
        None
    };

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
        .with_padding(BoxAmount::new(PAD_SM, PAD_MD, PAD_SM, PAD_MD))
        .with_child_gap(SPACE_SM)
        .with_vertical_alignment(VerticalAlignment::Center)
    } else {
        Text::new("Export not ready")
            .with_font_size(14.0)
            .with_color(TEXT_MUTED)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .into()
    };

    Element {
        id: Some(w_id!()),
        direction: Direction::LeftToRight,
        width: Sizing::fixed(400.0),
        height: Sizing::fit(),
        background_color: Some(Color::from(BACKGROUND_LIGHT)),
        padding: BoxAmount::all(PAD_MD),
        border_radius: Some(BorderRadius::all(BORDER_RADIUS)),
        border: Some(Border {
            width: 1.0,
            color: BORDER_COLOR,
            ..Default::default()
        }),
        child_gap: SPACE_SM,
        vertical_alignment: VerticalAlignment::Center,
        children: {
            let mut children = vec![button_with_hover, file_info];
            if let Some(tooltip) = tooltip {
                children.push(tooltip);
            }
            children
        },
        ..Default::default()
    }
}

pub fn update(message: FileDownloadMessage) -> FileDownloadAction {
    match message {
        FileDownloadMessage::PickPathForFile(export) => {
            FileDownloadAction::Run(Task::future(async move {
                let file_dialog = rfd::AsyncFileDialog::new()
                    .set_file_name(&export.name)
                    .add_filter(&export.ext.description, &export.ext.extensions)
                    .save_file()
                    .await;

                if let Some(file) = file_dialog {
                    FileDownloadMessage::SaveFile(export, file.path().to_path_buf())
                } else {
                    // User cancelled - we could add a separate message for this
                    FileDownloadMessage::PickPathForFile(export) // Return same message to indicate no action
                }
            }))
        }
        FileDownloadMessage::SaveFile(export, path) => {
            if let Err(e) = std::fs::write(&path, &export.content) {
                tracing::error!("Failed to save file to {:?}: {}", path, e);
            } else {
                tracing::info!("Successfully saved file to {:?}", path);
            }
            FileDownloadAction::None
        }
    }
}
