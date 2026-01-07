use raxis::layout::helpers::{center, container, spacer};
use raxis::layout::model::{
    Alignment, BorderRadius, BoxAmount, Element, ScrollBarSize, ScrollConfig, ScrollbarStyle, Sizing,
};
use raxis::widgets::button::Button;
use raxis::widgets::image::Image;
use raxis::widgets::rule::Rule;
use raxis::widgets::text::{ParagraphAlignment, Text, TextAlignment};
use raxis::widgets::titlebar_controls::titlebar_controls;
use raxis::{column, row, w_id, HookManager};
use tracing::level_filters::LevelFilter;

use crate::rgui::theme::{
    maybe_text_shadow, BORDER_COLOR, BORDER_RADIUS, BORDER_RADIUS_SM, CARD_BACKGROUND, DANGER_COLOR, PAD_LG, PAD_MD, PAD_SM,
    SCROLLBAR_THUMB_COLOR, SCROLLBAR_TRACK_COLOR, SPACE_MD, SUCCESS_COLOR, TEXT_COLOR, TEXT_MUTED,
};
use crate::rgui::state::{ImageFit, RootState, Screen};
use crate::rgui::messages::{LogMessage, RootMessage, WebSocketStatus, WindowMessage};
use crate::rgui::kit::icons::{cog_icon, discord_button, github_button};
use crate::rgui::kit::togglegroup::{togglegroup, ToggleOption};
use crate::rgui::components::log_view::log_view;
use crate::rgui::components::settings_modal::settings_modal;
use crate::rgui::components::update::{update_modal, UpdateState};
use crate::scopefns::Also;

// Main view function
pub fn view(state: &RootState, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let text_shadow_enabled = state.store.settings.text_shadow_enabled;

    let help_text = maybe_text_shadow(
        Text::new("have questions or issues?")
            .with_font_size(16.0)
            .italic()
            .with_color(TEXT_MUTED),
        text_shadow_enabled,
    );

    let social_buttons = row![github_button(), discord_button()].with_child_gap(SPACE_MD);

    let header = column![social_buttons, help_text]
        .with_child_gap(SPACE_MD / 2.0)
        .with_padding(BoxAmount::all(PAD_LG).apply(|p| p.bottom = PAD_SM))
        .with_width(Sizing::grow())
        .with_height(Sizing::Grow { min: 0.0, max: 182.0 }); // Size of footer + log view; this ensures the main content is as centered as possible

    let menu_button = Button::new()
        .ghost()
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler(|_, s| s.publish(RootMessage::Window(WindowMessage::ToggleMenu)))
        .as_element(w_id!(), cog_icon());

    let header = row![
        header,
        spacer(),
        column![
            titlebar_controls(hook),
            container(menu_button).with_padding(BoxAmount::all(PAD_MD)),
        ].with_cross_align_items(Alignment::End)
    ]
    .with_width(Sizing::grow());

    let ws_status_text = match &state.store.connection_stats.ws_status {
        WebSocketStatus::Pending => "starting server...".to_string(),
        WebSocketStatus::Running { port, client_count } => {
            if *client_count > 0 {
                format!(
                    "ws://localhost:{}/ws ({} client{})",
                    port,
                    client_count,
                    if *client_count == 1 { "" } else { "s" }
                )
            } else {
                format!("ws://localhost:{}/ws (no clients)", port)
            }
        }
        WebSocketStatus::Failed { error } => format!("failed to start server: {}", error),
    };

    let ws_status = maybe_text_shadow(
        Text::new(ws_status_text)
            .with_font_size(12.0)
            .with_color(match &state.store.connection_stats.ws_status {
                WebSocketStatus::Failed { .. } => DANGER_COLOR,
                _ => TEXT_MUTED,
            }),
        text_shadow_enabled,
    )
    .as_element()
    .with_id(w_id!());

    let content = match &state.screen {
        Screen::Waiting(screen) => screen.view(&state.store, hook),
        Screen::Active(screen) => screen.view(&state.store, hook),
    };

    let connection_status_text = if state.store.connection_stats.connected {
        format!(
            "connected, {}/{} pkts/cmds received",
            state.store.connection_stats.packets_received, state.store.connection_stats.commands_received
        )
    } else {
        "disconnected".to_string()
    };

    let connection_status = maybe_text_shadow(
        Text::new(connection_status_text)
            .with_font_size(12.0)
            .with_color(if state.store.connection_stats.connected {
                SUCCESS_COLOR
            } else {
                DANGER_COLOR
            }),
        text_shadow_enabled,
    );

    let level_group = togglegroup(
        w_id!(),
        vec![
            ToggleOption::new(LevelFilter::INFO, "Info"),
            ToggleOption::new(LevelFilter::DEBUG, "Debug"),
            ToggleOption::new(LevelFilter::TRACE, "Trace"),
        ],
        &state.store.log_level,
        |value| Some(RootMessage::Log(LogMessage::LevelChanged(value))),
        None
    );

    let footer = column![
        row![
            level_group,
            spacer(),
            Button::new()
                .with_bg_color(CARD_BACKGROUND)
                .with_border(1.0, BORDER_COLOR)
                .with_border_radius(BORDER_RADIUS_SM)
                .with_click_handler(|_, shell| shell.publish(RootMessage::Log(LogMessage::Export)))
                .as_element(w_id!(), Text::new("Export").with_color(TEXT_COLOR).with_font_size(10.0))
                .with_padding(PAD_SM)
        ]
        .with_width(Sizing::grow())
        .with_padding(BoxAmount::bottom(PAD_SM)),
        log_view(hook),
        row![
            ws_status,
            if matches!(state.store.update_state, Some(UpdateState::Checking)) {
                container(
                    maybe_text_shadow(
                        Text::new("Checking for updates...")
                            .with_font_size(12.0)
                            .with_color(TEXT_MUTED)
                            .with_text_alignment(TextAlignment::Center),
                        text_shadow_enabled,
                    )
                    .as_element()
                ).with_axis_align_content(Alignment::Center)
            } else {
                Element::default()
            },
            container(connection_status).with_axis_align_content(Alignment::End),
        ]
        .map_children(|e| e.with_width(Sizing::grow()))
        .with_child_gap(SPACE_MD)
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::End)
        .with_padding(PAD_MD)
    ]
    .with_width(Sizing::grow())
    .with_height(Sizing::fit())
    .with_padding(PAD_MD);

    row![
        column![header, center(content).with_padding(PAD_MD), footer]
            .with_id(w_id!())
            .with_color(TEXT_COLOR)
            .with_width(Sizing::grow())
            .with_height(Sizing::grow())
            .with_child_gap(SPACE_MD)
            .with_scroll(ScrollConfig {
                vertical: Some(true),
                // Safe area padding for the window controls
                safe_area_padding: Some(BoxAmount::all(4.0).apply(|p| p.top = 34.0)),
                scrollbar_style: Some(ScrollbarStyle {
                    thumb_color: SCROLLBAR_THUMB_COLOR,
                    track_color: SCROLLBAR_TRACK_COLOR,
                    track_radius: BorderRadius::all(4.0),
                    size: ScrollBarSize::ThinThick(8.0, 12.0),
                ..Default::default()
                }),
                ..Default::default()
            }),
        settings_modal(state, hook),
        update_modal(
            state.store.update_state.as_ref(),
            hook,
            RootMessage::Update,
        )
    ]
    .with_id(w_id!())
    .with_color(TEXT_COLOR)
    .apply(|e| {
        if !state.store.settings.background_image.is_empty() {
            e.with_widget(
                Image::new(state.store.settings.background_image.clone())
                    .with_opacity(state.store.settings.background_opacity)
                    .with_fit(match state.store.settings.image_fit {
                        ImageFit::Fill => raxis::widgets::image::ImageFit::Fill,
                        ImageFit::Contain => raxis::widgets::image::ImageFit::Contain,
                        ImageFit::Cover => raxis::widgets::image::ImageFit::Cover,
                        ImageFit::ScaleDown => raxis::widgets::image::ImageFit::ScaleDown,
                        ImageFit::None => raxis::widgets::image::ImageFit::None,
                    }),
            )
        } else {
            e
        }
    })
    .with_width(Sizing::grow())
    .with_height(Sizing::grow())
}
