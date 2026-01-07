//! Active screen implementation.
//!
//! Displayed when actively connected to the game and capturing data.
//! Shows capture statistics and provides export functionality.

use raxis::layout::model::{Alignment, BackdropFilter, BorderRadius, BoxAmount, Color, Element, Sizing};
use raxis::util::unique::combine_id;
use raxis::widgets::button::Button;
use raxis::widgets::rule::Rule;
use raxis::widgets::text::{ParagraphAlignment, Text};
use raxis::{column, row, w_id, HookManager};

use crate::rgui::components::file_download::download_view;
use crate::rgui::kit::icons::refresh_icon;
use crate::rgui::messages::{ActiveMessage, ExportMessage, RootMessage, ScreenAction};
use crate::rgui::state::{ActiveScreen, Store};
use crate::rgui::theme::{
    maybe_text_shadow, BORDER_COLOR, BORDER_RADIUS, PAD_LG, PAD_MD, SHADOW_SM, SPACE_LG, SPACE_MD, SUCCESS_COLOR, TEXT_COLOR, TEXT_MUTED,
};

/// Renders a single statistic line (label + value).
fn stat_line(label: &'static str, value: usize, text_shadow_enabled: bool) -> Element<RootMessage> {
    column![
        maybe_text_shadow(Text::new(label).with_font_size(16.0).with_color(TEXT_MUTED), text_shadow_enabled),
        maybe_text_shadow(
            Text::new(value.to_string())
                .with_font_size(24.0)
                .with_assisted_id(combine_id(w_id!(), label)),
            text_shadow_enabled
        )
    ]
    .with_child_gap(SPACE_MD)
    .with_cross_align_items(Alignment::Center)
    .with_width(Sizing::grow())
}

impl ActiveScreen {
    /// Renders the active screen view.
    pub fn view(&self, store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
        self.active_view(store, hook)
    }

    /// Handles active screen messages.
    pub fn update(&mut self, _message: ActiveMessage) -> ScreenAction<ActiveMessage> {
        ScreenAction::None
    }

    /// Renders the main active view content with stats and export controls.
    fn active_view(&self, store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
        let text_shadow_enabled = store.settings.text_shadow_enabled;

        let stats_display = row![
            stat_line("Relics", store.export_stats.relics, text_shadow_enabled),
            stat_line("Characters", store.export_stats.characters, text_shadow_enabled),
            stat_line("Light Cones", store.export_stats.light_cones, text_shadow_enabled),
            stat_line("Materials", store.export_stats.materials, text_shadow_enabled),
        ]
        .with_width(Sizing::grow())
        .with_child_gap(SPACE_LG);

        let refresh_button = Button::new()
            .with_bg_color(SUCCESS_COLOR)
            .with_border_radius(BORDER_RADIUS)
            .with_drop_shadow(SHADOW_SM)
            .with_click_handler(move |_, shell| {
                shell.publish(RootMessage::Export(ExportMessage::Refresh));
            })
            .as_element(w_id!(), refresh_icon())
            .with_backdrop_filter(BackdropFilter::blur(10.0))
            .with_snap(true);

        let download_section = download_view(store.json_export.as_ref(), store.export_out_of_date, hook).with_drop_shadow(SHADOW_SM);

        let action_bar = row![refresh_button, download_section]
            .with_child_gap(SPACE_LG)
            .with_axis_align_content(Alignment::Center)
            .with_padding(BoxAmount::all(PAD_MD));

        column![
            maybe_text_shadow(
                Text::new("Connected!")
                    .with_font_size(24.0)
                    .with_color(TEXT_COLOR)
                    .with_paragraph_alignment(ParagraphAlignment::Center),
                text_shadow_enabled
            )
            .as_element()
            .with_padding(BoxAmount::all(PAD_MD)),
            stats_display,
            Rule::horizontal()
                .with_color(BORDER_COLOR)
                .as_element(w_id!())
                .with_padding(BoxAmount::vertical(PAD_LG)),
            action_bar,
        ]
        .with_child_gap(SPACE_LG)
        .with_cross_align_items(Alignment::Center)
        .with_padding(BoxAmount::all(PAD_LG * 2.0))
        .with_border_radius(BorderRadius::all(BORDER_RADIUS))
    }
}
