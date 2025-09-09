use std::{cell::RefCell, path::PathBuf, rc::Rc};

use raxis::{
    layout::{
        helpers::{center, row, ElementAlignmentExt},
        model::{
            Alignment2D, Border, BorderRadius, BoxAmount, Color, Direction, Element, FloatingConfig, HorizontalAlignment, Offset2D, Sizing,
            StrokeLineCap, StrokeLineJoin, VerticalAlignment,
        },
    },
    runtime::task::Task,
    svg_path, w_id,
    widgets::{
        button::Button,
        mouse_area::{MouseArea, MouseAreaEvent},
        svg::ViewBox,
        svg_path::SvgPath,
        text::{ParagraphAlignment, Text, TextAlignment},
        widget,
    },
    HookManager,
};

use crate::{
    rgui::{
        FileContainer, FileExtensions, BACKGROUND_LIGHT, BACKGROUND_STRONG, BORDER_COLOR, BORDER_RADIUS, PAD_MD, PAD_SM, PRIMARY_COLOR,
        SPACE_MD, SPACE_SM, SUCCESS_COLOR, TEXT_MUTED,
    },
    scopefns::Also,
};

#[derive(Debug, Clone)]
pub enum Message {
    PickPathForFile(FileContainer),
    SaveFile(FileContainer, PathBuf),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
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
fn download_arrow_icon<M>(stroke: Color) -> Element<M> {
    SvgPath::new(
        svg_path!("M12 15V3M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5"),
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

pub fn download_view<PMsg: Send + Clone + std::fmt::Debug + 'static>(
    file: Option<&FileContainer>,
    out_of_date: bool,
    hook: &mut HookManager<PMsg>,
) -> Element<PMsg> {
    let mut hook = hook.instance(w_id!());
    let hover_state = hook.use_hook(|| Rc::new(RefCell::new(false)));

    let download_button_id = w_id!();
    let download_button = Button::new()
        .with_bg_color(PRIMARY_COLOR)
        .with_border_radius(BORDER_RADIUS)
        .with_border(1.0, Color::from(0x00000033))
        // .with_no_border()
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
            download_button_id,
            download_arrow_icon(if file.is_some() { Color::WHITE } else { Color::from(0x0000003F) }),
        );

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
            .with_background_color(Color::from(0xFFF3CDFF)) // Light yellow warning background
            .with_padding(PAD_SM)
            .with_border_radius(BORDER_RADIUS)
            .with_border(Color::from(0xF0AD4EFF)) // Warning border color
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
            })
            .with_z_index(10),
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
        .with_padding(PAD_SM)
        .with_child_gap(SPACE_SM)
        .with_vertical_alignment(VerticalAlignment::Center)
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
        background_color: Some(Color::from(BACKGROUND_LIGHT)),
        border_radius: Some(BorderRadius::all(BORDER_RADIUS)),
        // padding: BoxAmount::all(PAD_SM),
        border: Some(Border {
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
