//! Tooltip components with theme support.
//!
//! Provides hover-activated tooltips that can be attached to any element.
//! Supports multiple visual themes (warning, info, error, success) and
//! configurable positioning.

use std::cell::RefCell;
use std::rc::Rc;

use raxis::layout::model::{Alignment, Alignment2D, Color, Element, FloatingConfig, Offset2D};
use raxis::util::unique::combine_id;
use raxis::widgets::mouse_area::{MouseArea, MouseAreaEvent};
use raxis::widgets::text::{ParagraphAlignment, Text};
use raxis::{w_id, HookManager};

use crate::rgui::theme::{BORDER_RADIUS, PAD_SM};

/// Visual theme variants for tooltips.
#[derive(Debug, Clone)]
pub enum TooltipTheme {
    Warning,
    Info,
    Error,
    Success,
    Custom {
        background_color: Color,
        text_color: Color,
        border_color: Option<Color>,
    },
}

impl TooltipTheme {
    fn colors(&self) -> (Color, Color, Option<Color>) {
        match self {
            TooltipTheme::Warning => (
                Color::from(0x856404FF),       // Dark yellow background
                Color::from(0xFFF3CDFF),       // Light yellow text
                Some(Color::from(0xF0AD4E8F)), // Warning border
            ),
            TooltipTheme::Info => (
                Color::from(0x0C5460FF),       // Dark blue background
                Color::from(0xD1ECF1FF),       // Light blue text
                Some(Color::from(0x17A2B88F)), // Info border
            ),
            TooltipTheme::Error => (
                Color::from(0x721C24FF),       // Dark red background
                Color::from(0xF8D7DAFF),       // Light red text
                Some(Color::from(0xDC35458F)), // Error border
            ),
            TooltipTheme::Success => (
                Color::from(0x155724FF),       // Dark green background
                Color::from(0xD4EDDDFF),       // Light green text
                Some(Color::from(0x28A7458F)), // Success border
            ),
            TooltipTheme::Custom {
                background_color,
                text_color,
                border_color,
            } => (*background_color, *text_color, *border_color),
        }
    }
}

/// Controls where the tooltip appears relative to its anchor element.
#[derive(Debug, Clone)]
pub struct TooltipPosition {
    pub offset: Offset2D,
    pub anchor: Alignment2D<Alignment, Alignment>,
    pub align: Alignment2D<Alignment, Alignment>,
}

impl Default for TooltipPosition {
    fn default() -> Self {
        Self {
            offset: Offset2D {
                x: Some(0.0),
                y: Some(-4.0),
            },
            anchor: Alignment2D {
                x: Some(Alignment::Center),
                y: Some(Alignment::Start),
            },
            align: Alignment2D {
                x: Some(Alignment::Center),
                y: Some(Alignment::End),
            },
        }
    }
}

/// Full configuration for tooltip behavior and appearance.
#[derive(Debug, Clone)]
pub struct TooltipConfig {
    pub theme: TooltipTheme,
    pub position: TooltipPosition,
    pub z_index: i32,
    pub font_size: f32,
}

impl Default for TooltipConfig {
    fn default() -> Self {
        Self {
            theme: TooltipTheme::Info,
            position: TooltipPosition::default(),
            z_index: 10,
            font_size: 12.0,
        }
    }
}

/// Wraps an element with hover-activated tooltip functionality.
///
/// The tooltip appears when the user hovers over the content and
/// disappears when they move away.
pub fn tooltip_wrapper<PMsg: Send + Clone + std::fmt::Debug + 'static>(
    content: Element<PMsg>,
    tooltip_text: String,
    anchor_id: u64,
    show_condition: bool,
    config: TooltipConfig,
    hook: &mut HookManager<PMsg>,
) -> Element<PMsg> {
    let mut hook = hook.instance(combine_id(anchor_id, w_id!()));
    let hover_state = hook.use_hook(|| Rc::new(RefCell::new(false)));

    if !show_condition {
        return content;
    }

    let wrapped_content = MouseArea::new({
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
    })
    .as_element(combine_id(anchor_id, w_id!()), content);

    let mut elements = vec![wrapped_content];

    if *hover_state.borrow() {
        let tooltip = create_tooltip(tooltip_text, anchor_id, config);
        elements.push(tooltip);
    }

    Element {
        children: elements,
        ..Default::default()
    }
}

/// Creates a floating tooltip element for use in custom layouts.
pub fn create_tooltip<PMsg: Send + Clone + std::fmt::Debug + 'static>(
    text: String,
    anchor_id: u64,
    config: TooltipConfig,
) -> Element<PMsg> {
    let (background_color, text_color, border_color) = config.theme.colors();

    let mut tooltip = Element {
        children: vec![Text::new(text)
            .with_font_size(config.font_size)
            .with_color(text_color)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .into()],
        ..Default::default()
    }
    .with_id(combine_id(anchor_id, w_id!()))
    .with_background_color(background_color)
    .with_padding(PAD_SM)
    .with_border_radius(BORDER_RADIUS)
    .with_floating(FloatingConfig {
        offset: Some(config.position.offset),
        anchor_id: Some(anchor_id),
        anchor: Some(config.position.anchor),
        align: Some(config.position.align),
    })
    .with_z_index(config.z_index);

    if let Some(border_color) = border_color {
        tooltip = tooltip.with_border(border_color);
    }

    tooltip
}

macro_rules! tooltip_helper {
    ($name:ident, $theme:ident) => {
        /// Helper function to create a tooltip with a specific theme
        pub fn $name<PMsg: Send + Clone + std::fmt::Debug + 'static>(
            content: Element<PMsg>,
            tooltip_text: String,
            show_condition: bool,
            hook: &mut HookManager<PMsg>,
        ) -> Element<PMsg> {
            let anchor_id = content.id.expect("Tooltip content must have an ID");
            tooltip_wrapper(
                content,
                tooltip_text,
                anchor_id,
                show_condition,
                TooltipConfig {
                    theme: TooltipTheme::$theme,
                    ..Default::default()
                },
                hook,
            )
        }
    };
}

tooltip_helper!(warning_tooltip, Warning);
tooltip_helper!(info_tooltip, Info);
tooltip_helper!(error_tooltip, Error);
tooltip_helper!(success_tooltip, Success);
