use raxis::{
    layout::model::{Border, BorderRadius, Color, Direction::LeftToRight, Element, HorizontalAlignment, Sizing, VerticalAlignment},
    util::unique::combine_id,
    w_id,
    widgets::{button::Button, text::Text},
    HookManager,
};

use crate::rgui::{
    BORDER_COLOR, BORDER_RADIUS, BORDER_RADIUS_SM, CARD_BACKGROUND, PAD_MD, PAD_SM, PRIMARY_COLOR, SPACE_SM, TEXT_COLOR, TEXT_MUTED,
};

/// Configuration for a single toggle button in the group
#[derive(Debug, Clone)]
pub struct ToggleOption<T: Clone> {
    pub value: T,
    pub label: String,
}

impl<T: Clone> ToggleOption<T> {
    pub fn new(value: T, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

/// Configuration for the toggle group appearance
#[derive(Debug, Clone)]
pub struct ToggleGroupConfig {
    pub active_color: Color,
    pub inactive_color: Color,
    pub active_text_color: Color,
    pub inactive_text_color: Color,
    pub text_size: f32,
    pub border_color: Color,
    pub border_width: f32,
    pub border_radius: f32,
    pub button_padding: f32,
    pub button_gap: f32,
}

impl Default for ToggleGroupConfig {
    fn default() -> Self {
        Self {
            active_color: PRIMARY_COLOR,
            inactive_color: CARD_BACKGROUND,
            active_text_color: TEXT_COLOR,
            inactive_text_color: TEXT_MUTED,
            text_size: 10.0,
            border_color: BORDER_COLOR,
            border_width: 1.0,
            border_radius: BORDER_RADIUS_SM,
            button_padding: PAD_SM,
            button_gap: SPACE_SM,
        }
    }
}

/// ```
// pub fn togglegroup<T, PMsg, F>(
//     options: Vec<ToggleOption<T>>,
//     active_value: &T,
//     on_change: F,
//     config: Option<ToggleGroupConfig>,
//     hook: &mut HookManager<PMsg>,
// ) -> Element<PMsg>
// where
//     T: Clone + PartialEq + 'static,
//     PMsg: Send + Clone + std::fmt::Debug + 'static,
//     F: Fn(T) -> PMsg + Clone + 'static,
// {
//     let config = config.unwrap_or_default();
//     let border_radius = config.border_radius;

//     togglegroup_custom(
//         options,
//         active_value,
//         on_change,
//         move |button, is_first, is_last| {
//             let radius = if is_first {
//                 BorderRadius::left(border_radius)
//             } else if is_last {
//                 BorderRadius::right(border_radius)
//             } else {
//                 BorderRadius::all(0.0)
//             };

//             button
//                 .with_bg_color(config.active_color)
//                 .with_border_radius(radius)
//                 .with_border(config.border_width, config.border_color)
//         },
//         move |button, is_first, is_last| {
//             let radius = if is_first {
//                 BorderRadius::left(border_radius)
//             } else if is_last {
//                 BorderRadius::right(border_radius)
//             } else {
//                 BorderRadius::all(0.0)
//             };

//             button.with_bg_color(config.inactive_color)
//         },
//         config.button_gap,
//         hook,
//     )
// }

/// Creates a toggle group with multiple buttons where only one can be active at a time
///
/// # Arguments
/// * `options` - List of toggle options to display
/// * `active_value` - The currently active value
/// * `on_change` - Callback function that receives the new selected value
/// * `config` - Optional configuration for appearance
/// * `hook` - Hook manager for state management
///
/// # Example
/// ```ignore
/// let options = vec![
///     ToggleOption::new("option1", "Option 1"),
///     ToggleOption::new("option2", "Option 2"),
///     ToggleOption::new("option3", "Option 3"),
/// ];
///
/// let element = togglegroup(
///     options,
///     current_selection,
///     |new_value| Message::SelectionChanged(new_value),
///     None,
///     hook,
/// );
pub fn togglegroup<T, PMsg, F>(options: Vec<ToggleOption<T>>, active_value: &T, on_change: F) -> Element<PMsg>
where
    T: Clone + PartialEq + 'static,
    PMsg: Send + Clone + std::fmt::Debug + 'static,
    F: Fn(T) -> Option<PMsg> + Clone + 'static,
{
    let base_id = w_id!();
    let total_count = options.len();

    let config = ToggleGroupConfig::default();

    let buttons: Vec<Element<PMsg>> = options
        .into_iter()
        .enumerate()
        .map(|(idx, option)| {
            let is_active = &option.value == active_value;
            let is_first = idx == 0;
            let is_last = idx == total_count - 1;
            let on_change = on_change.clone();
            let value = option.value.clone();

            let mut button = Button::new();

            button = if is_active {
                button.with_bg_color(config.active_color)
            } else {
                button.with_bg_color(config.inactive_color)
            };

            let radius = if is_first {
                BorderRadius::left(config.border_radius)
            } else if is_last {
                BorderRadius::right(config.border_radius)
            } else {
                BorderRadius::all(0.0)
            };

            let text_color = if is_active {
                config.active_text_color
            } else {
                config.inactive_text_color
            };

            button
                .with_border_radius(radius)
                .with_border(config.border_width, config.border_color)
                .with_click_handler(move |_, shell| {
                    if let Some(msg) = on_change(value.clone()) {
                        shell.publish(msg);
                    }
                })
                .as_element(
                    combine_id(base_id, idx as u64),
                    Text::new(option.label).with_color(text_color).with_font_size(config.text_size),
                )
                .with_padding(config.button_padding)
        })
        .collect();

    Element {
        children: buttons,
        direction: LeftToRight,
        ..Default::default()
    }
}
