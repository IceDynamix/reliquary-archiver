use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use raxis::layout::helpers::spacer;
use raxis::layout::model::{
    Alignment, Border, BorderRadius, BoxAmount, Color, Element, ScrollBarSize, ScrollConfig, ScrollbarStyle, Sizing,
};
use raxis::runtime::font_manager::FontWeight;
use raxis::runtime::task::Task;
use raxis::widgets::button::Button;
use raxis::widgets::rule::horizontal_rule;
use raxis::widgets::slider::Slider;
use raxis::widgets::text::Text;
use raxis::widgets::text_input::TextInput;
use raxis::widgets::toggle::Toggle;
use raxis::widgets::widget;
use raxis::{column, row, use_animation, w_id, HookManager};
use tracing::info;

use crate::rgui::theme::{
    BORDER_RADIUS, BORDER_RADIUS_SM, CARD_BACKGROUND, OPAQUE_CARD_BACKGROUND, PAD_LG, PAD_MD, PAD_SM, PRIMARY_COLOR,
    SCROLLBAR_THUMB_COLOR, SCROLLBAR_TRACK_COLOR, SPACE_LG, SPACE_SM, TEXT_COLOR, TEXT_MUTED,
};
use crate::rgui::state::{ImageFit, RootState};
use crate::rgui::messages::{RootMessage, SettingsMessage, WebSocketMessage, WebSocketStatus, WindowMessage};
use crate::rgui::kit::modal::{modal_backdrop, ModalConfig, ModalPosition};
use crate::rgui::kit::togglegroup::{togglegroup, ToggleGroupConfig, ToggleOption};
use crate::rgui::kit::icons::x_icon;
use crate::rgui::components::update::UpdateMessage;
use crate::rgui::run_on_start::{RegistryError, set_run_on_start};
use crate::rgui::handlers::save_settings;

#[derive(Default, Clone)]
struct WebsocketConfigState {
    port_input: u16,
}

fn websocket_settings_section(state: &RootState, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let mut instance = hook.instance(w_id!());
    let config_state: Rc<RefCell<WebsocketConfigState>> = instance.use_state(|| { WebsocketConfigState {
        port_input: state.store.settings.ws_port,
    } });

    let text_input = row![
        Text::new("ws://0.0.0.0:")
            .as_element()
            .with_padding(BoxAmount::new(2.0, 2.0, 0.0, 0.0)),
        Element {
            id: Some(w_id!()),
            // 40 px wide = scrollbar
            width: Sizing::Fixed { px: 41.0 },
            height: Sizing::Fixed { px: 25.0 },
            background_color: Some(OPAQUE_CARD_BACKGROUND.deviate(0.1)),
            border_radius: Some(BorderRadius::all(8.0)),
            border: Some(Border {
                width: 1.0,
                color: OPAQUE_CARD_BACKGROUND.deviate(0.4),
                ..Default::default()
            }),
            color: Some(Color::WHITE),
            scroll: Some(ScrollConfig {
                horizontal: Some(true),
                sticky_right: Some(true),
                scrollbar_style: Some(ScrollbarStyle {
                    thumb_color: SCROLLBAR_THUMB_COLOR.lighten(0.3),
                    track_color: SCROLLBAR_TRACK_COLOR.lighten(0.3),
                    size: ScrollBarSize::ThinThick(4.0, 8.0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            children: vec![
                Element {
                    id: Some(w_id!()),
                    width: Sizing::grow(),
                    height: Sizing::grow(),
                    padding: BoxAmount::new(2.0, 4.0, 2.0, 4.0),
                    content: widget(TextInput::new()
                        .with_font_size(12.0)
                        .with_paragraph_alignment(raxis::widgets::text::ParagraphAlignment::Center)
                        .with_text(config_state.borrow_mut().port_input.to_string())
                        .with_text_input_handler({
                            let config_state = config_state.clone();
                            let ws_status = state.store.connection_stats.ws_status.clone();
                            move |text, shell| {
                                if let Ok(port) = text.parse::<u16>() {
                                    config_state.borrow_mut().port_input = port;
                                }
                            }
                        })
                    ),
                    wrap: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }.with_axis_align_content(Alignment::Center).with_cross_align_content(Alignment::Center),
        Text::new("/ws")
            .as_element()
            .with_padding(BoxAmount::new(2.0, 0.0 , 0.0, 2.0)),
    ];
    let header: Element<RootMessage> = Text::new("Configure Websocket port")
        .with_font_size(14.0)
        .as_element();

    let explainer_text = Text::new("Setting port to 0 will make windows assign you a port of its choosing.")
        .with_font_size(12.0)
        .with_color(TEXT_MUTED)
        .as_element()
        .with_padding(BoxAmount::top(5.0));

    let button = Button::new()
        .with_click_handler({
            let requested_port = config_state.borrow().port_input;
            let ws_status = state.store.connection_stats.ws_status.clone();
            move |_, s| {
                match ws_status {
                    WebSocketStatus::Running { port, client_count: _ } => {
                        if port == requested_port {
                            info!("Websocket server already running on requested port");
                        } else {
                            s.publish(RootMessage::WebSocket(WebSocketMessage::SendPort(requested_port)));
                        }
                    }
                    _ => s.publish(RootMessage::WebSocket(WebSocketMessage::SendPort(requested_port))),
                }
            }
        })
        .with_bg_color(PRIMARY_COLOR)
        .as_element(
            w_id!(),
            Text::new("Restart server")
                .with_color(Color::WHITE)
                .with_font_size(16.0)
                .with_font_weight(FontWeight::Medium)
                .as_element()
                .with_axis_align_self(Alignment::Center)
                .with_cross_align_self(Alignment::Center)
                .with_padding(BoxAmount::horizontal(PAD_MD))
        )
        .with_height(Sizing::grow())
        .with_border_radius(BORDER_RADIUS);

    column![
        row![
            column![
                header,
                text_input,
            ],
            spacer(),
            button
        ]
        .with_width(Sizing::grow()),
        explainer_text
    ]
    .with_width(Sizing::grow())
}

#[derive(Clone, PartialEq)]
enum SettingsModalPanel {
    Graphics,
    Update,
    Misc
}

struct SettingsModalState {
    active_panel: SettingsModalPanel
}

pub fn settings_modal(state: &RootState, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let mut instance = hook.instance(w_id!());

    let modal_state = instance.use_state(|| SettingsModalState {
        active_panel: SettingsModalPanel::Graphics
    });

    let opacity = use_animation(&mut instance, state.settings_open);
    let bg_opacity = use_animation(&mut instance, !state.opacity_slider_dragging);
    let opacity = opacity.interpolate(hook, 0.0, 1.0, Instant::now());
    let bg_opacity = bg_opacity.interpolate(hook, 0.0, 0.5, Instant::now());

    if !state.settings_open && opacity == 0.0 {
        return Element::default();
    }

    // Header
    let close_button = Button::new()
        .ghost()
        .with_border_radius(BorderRadius::all(BORDER_RADIUS_SM))
        .with_click_handler(|_, shell| shell.publish(RootMessage::Window(WindowMessage::ToggleMenu)))
        .as_element(w_id!(), x_icon());

    let header_section = row![
        Text::new("Settings").with_font_size(20.0).with_color(TEXT_COLOR).as_element(),
        spacer(),
        close_button
    ]
    .with_width(Sizing::grow());

    // Background image section
    let select_image_button = Button::new()
        .with_bg_color(PRIMARY_COLOR)
        .with_border_radius(BORDER_RADIUS_SM)
        .with_click_handler(move |_, shell| {
            shell.dispatch_task(Task::future(async {
                let file = rfd::AsyncFileDialog::new()
                    .add_filter("Image files", &["jpg", "jpeg", "png", "bmp", "gif", "webp"])
                    .set_title("Select background image")
                    .pick_file()
                    .await;
                RootMessage::Settings(SettingsMessage::BackgroundImageSelected(file.map(|f| f.path().to_path_buf())))
            }));
        })
        .as_element(
            w_id!(),
            Text::new("Select Image")
                .with_font_size(12.0)
                .with_color(Color::WHITE)
                .as_element()
                .with_padding(BoxAmount::new(PAD_SM, PAD_MD, PAD_SM, PAD_MD)),
        );

    let remove_image_button = Button::new()
        .with_border_radius(BorderRadius::all(BORDER_RADIUS_SM))
        .with_bg_color(Color::from_rgba(220.0 / 255.0, 38.0 / 255.0, 38.0 / 255.0, 1.0))
        .with_click_handler(|_, shell| shell.publish(RootMessage::Settings(SettingsMessage::RemoveBackgroundImage)))
        .as_element(
            w_id!(),
            Text::new("✕")
                .with_font_size(12.0)
                .with_color(Color::WHITE)
                .as_element()
                .with_padding(BoxAmount::new(PAD_SM, PAD_MD, PAD_SM, PAD_MD)),
        );

    let mut bg_image_row = row![
        Text::new("Background Image")
            .with_font_size(14.0)
            .with_color(TEXT_COLOR)
            .as_element(),
        spacer(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow())
    .with_cross_align_items(Alignment::Center);

    // Add remove button if background image is present
    if !state.store.settings.background_image.is_empty() {
        bg_image_row.push_child(remove_image_button);
    }
    bg_image_row.push_child(select_image_button);

    let bg_image_section = column![
        bg_image_row,
        Text::new(format!(
            "Current: {}",
            if state.store.settings.background_image.is_empty() {
                "None"
            } else {
                state.store.settings.background_image.as_str()
            }
        ))
        .with_font_size(12.0)
        .with_color(TEXT_MUTED)
        .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Image fit mode section
    let mut fit_mode_toggles = togglegroup(
        w_id!(),
        vec![
            ToggleOption::new(ImageFit::Fill, "Fill"),
            ToggleOption::new(ImageFit::Contain, "Contain"),
            ToggleOption::new(ImageFit::Cover, "Cover"),
            ToggleOption::new(ImageFit::ScaleDown, "Scale Down"),
            ToggleOption::new(ImageFit::None, "None"),
        ],
        &state.store.settings.image_fit,
        |fit| Some(RootMessage::Settings(SettingsMessage::ImageFitChanged(fit))),
        None
    )
    .with_width(Sizing::grow());

    fit_mode_toggles.children = fit_mode_toggles
        .children
        .into_iter()
        .map(|mut child| child.with_width(Sizing::grow()))
        .collect();

    let fit_mode_section = column![
        Text::new("Image Fit Mode").with_font_size(14.0).with_color(TEXT_COLOR).as_element(),
        fit_mode_toggles,
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Opacity slider section
    let opacity_slider = Slider::new(0.0, 1.0, state.store.settings.background_opacity)
        .with_step(0.01)
        .with_track_height(6.0)
        .with_thumb_size(18.0)
        .with_track_color(CARD_BACKGROUND.deviate(0.2))
        .with_filled_track_color(PRIMARY_COLOR)
        .with_thumb_color(Color::WHITE)
        .with_thumb_border_color(PRIMARY_COLOR)
        .with_value_change_handler(|value, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::OpacityChanged(value)));
        })
        .with_drag_handler(|is_dragging, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::OpacitySliderDrag(is_dragging)));
        })
        .as_element(w_id!())
        .with_width(Sizing::grow());

    let opacity_section = column![
        row![
            Text::new("Background Opacity")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            Text::new(format!("{:.0}%", state.store.settings.background_opacity * 100.0))
                .with_font_size(12.0)
                .with_color(TEXT_MUTED)
                .as_element(),
        ]
        .with_width(Sizing::grow())
        .with_child_gap(SPACE_SM)
        .with_cross_align_items(Alignment::Center),
        opacity_slider,
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Text shadow section
    let text_shadow_toggle = Toggle::new(state.store.settings.text_shadow_enabled)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::TextShadowToggled(enabled)));
        })
        .as_element(w_id!());

    let text_shadow_section = column![
        row![
            Text::new("Text Shadow").with_font_size(14.0).with_color(TEXT_COLOR).as_element(),
            spacer(),
            text_shadow_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Add a text-shadow to text for better readability")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let update_check_button = Button::new()
        .with_click_handler(|_, s| {
            s.publish(RootMessage::Update(UpdateMessage::PerformCheck));
        })
        .with_bg_color(PRIMARY_COLOR)
        .as_element(
            w_id!(),
            Text::new("Check for updates")
                .with_color(Color::WHITE)
                .with_font_size(14.0)
                .with_font_weight(FontWeight::Medium)
                .as_element()
                .with_axis_align_self(Alignment::Center)
                .with_cross_align_self(Alignment::Center)
                .with_padding(BoxAmount::all(8.0))
        )
        .with_height(Sizing::grow())
        .with_border_radius(BORDER_RADIUS);

    let update_unprompted_toggle = Toggle::new(state.store.settings.always_update)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::AlwaysUpdateToggled(enabled)));
        })
        .as_element(w_id!());

    let update_unprompted_section = column![
        row![
            Text::new("Update Automatically")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            update_unprompted_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("When an update is available, update automatically without waiting for confirmation")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Minimize to tray on close section
    let minimize_on_close_toggle = Toggle::new(state.store.settings.minimize_to_tray_on_close)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::MinimizeToTrayOnCloseToggled(enabled)));
        })
        .as_element(w_id!());

    let minimize_on_close_section = column![
        row![
            Text::new("Minimize to Tray on Close")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            minimize_on_close_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Hide to system tray instead of closing when clicking the X button")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Minimize to tray on minimize section
    let minimize_on_minimize_toggle = Toggle::new(state.store.settings.minimize_to_tray_on_minimize)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::MinimizeToTrayOnMinimizeToggled(enabled)));
        })
        .as_element(w_id!());

    let minimize_on_minimize_section = column![
        row![
            Text::new("Minimize to Tray on Minimize")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            minimize_on_minimize_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Hide to system tray instead of taskbar when minimizing")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let run_on_start_toggle = Toggle::new(state.store.settings.run_on_start)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::RunOnStartToggled(enabled)));
        })
        .as_element(w_id!());

    let run_on_start_section = column![
        row![
            Text::new("Run on startup")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            run_on_start_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Run automatically when your computer starts")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let start_minimized_toggle = Toggle::new(state.store.settings.start_minimized)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::Settings(SettingsMessage::StartMinimizedToggled(enabled)));
        })
        .as_element(w_id!());

    let start_minimized_section = column![
        row![
            Text::new("Start minimized")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            start_minimized_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("The app will launch already minimized to the taskbar/system tray")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let graphics_settings_content = column![
        bg_image_section,
        fit_mode_section,
        opacity_section,
        text_shadow_section
    ];

    let update_settings_content = column![
        update_unprompted_section,
        update_check_button
    ];

    let misc_settings_content = column![
        websocket_settings_section(state, hook),
        horizontal_rule(w_id!()),
        minimize_on_close_section,
        minimize_on_minimize_section,
        run_on_start_section,
        start_minimized_section
    ];

    let modal_state_clone = modal_state.clone();

    let mut panel_toggles = togglegroup(
        w_id!(),
        vec![
            ToggleOption::new(SettingsModalPanel::Graphics, "Graphics"),
            ToggleOption::new(SettingsModalPanel::Update, "Update"),
            ToggleOption::new(SettingsModalPanel::Misc, "Misc"),
        ],
        &modal_state_clone.borrow().active_panel,
        move |e| {
            modal_state.borrow_mut().active_panel = e;
            None
        },
        Some(ToggleGroupConfig {
            text_size: 20.0,
            ..Default::default()
        })
    ).with_width(Sizing::grow());

    panel_toggles.children = panel_toggles
        .children
        .into_iter()
        .map(|mut child| child.with_width(Sizing::grow()))
        .collect();

    let settings_content = column![
        header_section,
        panel_toggles,
        horizontal_rule(w_id!()),
        match modal_state_clone.borrow().active_panel {
            SettingsModalPanel::Graphics => graphics_settings_content,
            SettingsModalPanel::Update => update_settings_content,
            SettingsModalPanel::Misc => misc_settings_content
        }.with_child_gap(SPACE_LG).with_width(Sizing::grow())
    ]
    .with_child_gap(SPACE_LG)
    .with_width(Sizing::fixed(400.0))
    .with_padding(BoxAmount::all(PAD_LG));

    modal_backdrop(
        w_id!(),
        state.settings_open,
        hook,
        Some(ModalConfig {
            backdrop_visible: !state.opacity_slider_dragging,
            modal_position: ModalPosition {
                top: Some(40.0),
                ..Default::default()
            },
            ..Default::default()
        }),
        Some(RootMessage::Window(WindowMessage::ToggleMenu)),
        settings_content,
    )
}

// ============================================================================
// Settings Message Handler
// ============================================================================

/// Handle settings-related messages
pub fn handle_settings_message(
    state: &mut RootState,
    message: SettingsMessage,
) -> Option<Task<RootMessage>> {
    match message {
        SettingsMessage::Load(path) => {
            use raxis::runtime::task::{self, Task};
            use tracing::{info, error};
            
            info!("Loading settings from {}", path.display());
            if path.exists() {
                Some(Task::future(tokio::fs::read_to_string(path)).and_then(move |content| {
                    use crate::rgui::state::Settings;
                    use crate::rgui::run_on_start::registry_matches_settings;
                    
                    let mut settings: Settings = match serde_json::from_str::<Settings>(&content) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Failed to load settings: {}", e);
                            Settings::default()
                        }
                    };

                    let run_on_start = settings.run_on_start;
                    match registry_matches_settings(run_on_start) {
                        // settings are not guaranteed to match the registry
                        // e.g. user moves the exe after enabling/disabling run on start
                        // in case of mismatch, update the settings and delete registry key if appropriate
                        Ok(false) => settings.run_on_start = !run_on_start,
                        Ok(true) => {},
                        _ => {},
                    };
                    
                    // want to avoid having the app briefly flash up if set to start minimized 
                    // the app will therefore always start minimized and update display mode here as necessary
                    let display_task = if settings.start_minimized {
                        // TODO: Does this make sense or should it also consider onClose preference
                        if settings.minimize_to_tray_on_minimize {
                            Task::none()
                        } else {
                            task::minimize_window()
                        }
                    } else {
                        task::show_window()
                    };
                    
                    Task::batch(vec![
                        display_task,
                        Task::done(RootMessage::WebSocket(WebSocketMessage::SendPort(settings.ws_port))),
                        Task::done(RootMessage::Settings(SettingsMessage::Activate(settings)))
                    ])
                }))
            } else {
                None
            }
        }

        SettingsMessage::Activate(settings) => {
            state.store.settings = settings;
            None
        }

        SettingsMessage::Save => save_settings(state),

        SettingsMessage::BackgroundImageSelected(path) => {
            if let Some(path) = path {
                state.store.settings.background_image = path.to_string_lossy().to_string();
                tracing::info!("Background image changed to: {}", state.store.settings.background_image);
            }
            save_settings(state)
        }

        SettingsMessage::RemoveBackgroundImage => {
            state.store.settings.background_image = String::new();
            tracing::info!("Background image removed");
            save_settings(state)
        }

        SettingsMessage::ImageFitChanged(fit) => {
            state.store.settings.image_fit = fit;
            tracing::info!("Image fit mode changed to: {:?}", fit);
            save_settings(state)
        }

        SettingsMessage::OpacityChanged(opacity) => {
            state.store.settings.background_opacity = opacity;
            save_settings(state)
        }

        SettingsMessage::OpacitySliderDrag(is_dragging) => {
            state.opacity_slider_dragging = is_dragging;
            None
        }

        SettingsMessage::TextShadowToggled(enabled) => {
            state.store.settings.text_shadow_enabled = enabled;
            save_settings(state)
        }

        SettingsMessage::AlwaysUpdateToggled(enabled) => {
            state.store.settings.always_update = enabled;
            save_settings(state)
        }

        SettingsMessage::MinimizeToTrayOnCloseToggled(enabled) => {
            state.store.settings.minimize_to_tray_on_close = enabled;
            save_settings(state)
        }

        SettingsMessage::MinimizeToTrayOnMinimizeToggled(enabled) => {
            state.store.settings.minimize_to_tray_on_minimize = enabled;
            save_settings(state)
        }

        SettingsMessage::RunOnStartToggled(enabled) => {
            match set_run_on_start(enabled) {
                Ok(()) => {
                    state.store.settings.run_on_start = enabled;
                },
                Err(RegistryError::KeyCreationFailed) => {
                    tracing::warn!("Unable to create registry key!");
                },
                Err(RegistryError::PathUnobtainable) => {
                    tracing::warn!("Unable to get current exe path!");
                },
                Err(RegistryError::AddFailed) => {
                    tracing::warn!("Failed to add registry key!");
                },
                Err(RegistryError::RemoveFailed) => {
                    state.store.settings.run_on_start = false;
                    tracing::warn!("Failed to remove registry key!");
                },
            }
            save_settings(state)
        }

        SettingsMessage::StartMinimizedToggled(enabled) => {
            state.store.settings.start_minimized = enabled;
            save_settings(state)
        }
    }
}
