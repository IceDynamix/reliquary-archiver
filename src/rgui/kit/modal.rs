//! Shared modal backdrop component

use std::time::Instant;

use raxis::layout::model::{Alignment, Border, BorderRadius, BoxAmount, Color, Direction, Element, FloatingConfig, Sizing};
use raxis::util::unique::{combine_id, WidgetId};
use raxis::widgets::button::Button;
use raxis::widgets::widget;
use raxis::{use_animation, w_id, HookManager};

use crate::rgui::theme::{BORDER_COLOR, BORDER_RADIUS, OPAQUE_CARD_BACKGROUND, SHADOW_XL};

/// Configuration for the modal backdrop
pub struct ModalConfig {
    /// Target background opacity when fully visible (default 0.5)
    pub bg_opacity_target: f32,
    /// Whether the backdrop should be visible (for cases like slider dragging where
    /// you want to hide the backdrop temporarily). Default true.
    pub backdrop_visible: bool,
    /// Where you want the modal to appear (defaults to center of screen)
    pub modal_position: ModalPosition,
}

#[derive(Default)]
pub struct ModalPosition {
    pub top: Option<f32>,
    pub left: Option<f32>,
}

impl Default for ModalConfig {
    fn default() -> Self {
        Self {
            bg_opacity_target: 0.5,
            backdrop_visible: true,
            modal_position: ModalPosition::default(),
        }
    }
}

/// Creates a modal backdrop with animated fade-in/out.
///
/// Returns an empty element if the modal should not be rendered (closed and fully faded out).
///
/// # Arguments
/// * `id` - Unique widget ID for the modal (use `w_id!()` at callsite)
/// * `visible` - Whether the modal should be visible
/// * `hook` - The hook manager for animations
/// * `config` - Optional configuration for the modal
/// * `on_backdrop_click` - Message to publish when the backdrop is clicked (for dismissing)
/// * `content` - The content to display inside the modal card
pub fn modal_backdrop<M: Clone + Send + 'static>(
    id: WidgetId,
    visible: bool,
    hook: &mut HookManager<M>,
    config: Option<ModalConfig>,
    on_backdrop_click: Option<M>,
    content: Element<M>,
) -> Element<M> {
    let config = config.unwrap_or_default();

    let mut instance = hook.instance(id);
    let opacity = use_animation(&mut instance, visible);
    let bg_opacity_anim = use_animation(&mut instance, config.backdrop_visible);
    let opacity = opacity.interpolate(hook, 0.0, 1.0, Instant::now());
    let bg_opacity_factor = bg_opacity_anim.interpolate(hook, 0.0, 1.0, Instant::now());

    // Don't render if closed and fully faded out
    if !visible && opacity == 0.0 {
        return Element::default();
    }

    let bg_opacity = opacity * config.bg_opacity_target * bg_opacity_factor;

    // Backdrop button - either handles click or is just a clear blocker
    let backdrop_button = match on_backdrop_click {
        Some(msg) => Button::new().clear().with_click_handler(move |_, s| s.publish(msg.clone())),
        None => Button::new().clear(),
    };

    Element {
        id: Some(combine_id(id, w_id!())),
        width: Sizing::percent(1.0),
        height: Sizing::percent(1.0),
        opacity: Some(opacity),
        background_color: Some(Color::from_rgba(0.0, 0.0, 0.0, bg_opacity)),
        floating: Some(FloatingConfig { ..Default::default() }),

        content: widget(backdrop_button),

        children: vec![position(
            Element {
                id: Some(combine_id(id, w_id!())),
                background_color: Some(OPAQUE_CARD_BACKGROUND),
                border_radius: Some(BorderRadius::all(BORDER_RADIUS)),
                border: Some(Border {
                    width: 1.0,
                    color: BORDER_COLOR,
                    ..Default::default()
                }),
                drop_shadows: vec![SHADOW_XL],

                content: widget(Button::new().clear()),

                children: vec![content],
                ..Default::default()
            },
            config.modal_position,
        )],

        ..Default::default()
    }
}

fn position<Message>(content: impl Into<Element<Message>>, position: ModalPosition) -> Element<Message> {
    Element {
        direction: Direction::TopToBottom,
        width: Sizing::grow(),
        children: vec![Element {
            direction: Direction::LeftToRight,
            children: vec![content.into()],
            axis_align_content: Alignment::Center,
            width: if let Some(left) = position.left {
                Sizing::default()
            } else {
                Sizing::grow()
            },
            padding: if let Some(left) = position.left {
                BoxAmount::left(left)
            } else {
                BoxAmount::default()
            },
            ..Default::default()
        }],
        axis_align_content: Alignment::Center,
        height: if let Some(top) = position.top {
            Sizing::default()
        } else {
            Sizing::grow()
        },
        padding: if let Some(top) = position.top {
            BoxAmount::top(top)
        } else {
            BoxAmount::default()
        },
        ..Default::default()
    }
}
