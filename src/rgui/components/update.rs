//! Update modal and related UI components

use raxis::layout::helpers::spacer;
use raxis::layout::model::{Alignment, BoxAmount, Color, Element, Sizing};
use raxis::runtime::task::Task;
use raxis::widgets::button::Button;
use raxis::widgets::text::{ParagraphAlignment, Text, TextAlignment};
use raxis::{column, row, w_id, HookManager};
use tracing::info;

use crate::rgui::kit::modal::modal_backdrop;
use crate::rgui::theme::{
    BORDER_RADIUS, CARD_BACKGROUND,
    PAD_LG, PAD_MD, SPACE_LG, SPACE_MD, SPACE_SM,
    SUCCESS_COLOR, TEXT_COLOR, TEXT_MUTED, BORDER_COLOR,
};

/// Update state for the GUI
#[derive(Debug, Clone, Default)]
pub enum UpdateState {
    #[default]
    Checking,
    Available {
        current_version: String,
        latest_version: String,
    },
    Updating,
    UpToDate,
    Failed(String),
}

/// Messages related to updates
#[derive(Debug, Clone)]
pub enum UpdateMessage {
    PerformCheck,
    CheckResult(Result<Option<(String, String)>, String>),
    Confirm,
    Dismiss,
    Complete(Result<(), String>),
}

/// Result of handling an update message
pub enum HandleResult {
    /// No task to run
    None,
    /// Run this task (produces UpdateMessage)
    Task(Task<UpdateMessage>),
    /// Update completed successfully, exit the application
    ExitForRestart,
}

/// Handle update messages and return the new state and optional task
pub fn handle_message(
    msg: UpdateMessage,
    current_state: &mut Option<UpdateState>,
    skip_consent: bool
) -> HandleResult {
    match msg {
        UpdateMessage::PerformCheck => {
            *current_state = Some(UpdateState::Checking);
            HandleResult::Task(check_for_updates_task())
        }

        UpdateMessage::CheckResult(result) => {
            match result {
                Ok(Some((current, latest))) => {
                    if (skip_consent) {
                        info!("Update available! Updating immediately");
                        return HandleResult::Task(Task::done(UpdateMessage::Confirm));
                    } else {
                        info!("Update available: {} -> {}", current, latest);
                        *current_state = Some(UpdateState::Available {
                            current_version: current,
                            latest_version: latest,
                        });
                    }
                }
                Ok(None) => {
                    info!("Already up-to-date");
                    *current_state = Some(UpdateState::UpToDate);
                }
                Err(e) => {
                    tracing::error!("Failed to check for updates: {}", e);
                    *current_state = Some(UpdateState::Failed(e));
                }
            }
            HandleResult::None
        }

        UpdateMessage::Confirm => {
            *current_state = Some(UpdateState::Updating);
            HandleResult::Task(Task::future(async move {
                let result = tokio::task::spawn_blocking(|| {
                    crate::update::update_noninteractive()
                }).await;
                
                match result {
                    Ok(Ok(true)) => UpdateMessage::Complete(Ok(())),
                    Ok(Ok(false)) => UpdateMessage::Complete(Err("Update reported no changes".to_string())),
                    Ok(Err(e)) => UpdateMessage::Complete(Err(e.to_string())),
                    Err(e) => UpdateMessage::Complete(Err(e.to_string())),
                }
            }))
        }

        UpdateMessage::Dismiss => {
            *current_state = None;
            HandleResult::None
        }

        UpdateMessage::Complete(result) => {
            match result {
                Ok(()) => {
                    info!("Update completed successfully, restarting...");
                    crate::update::request_spawn_after_exit();
                    HandleResult::ExitForRestart
                }
                Err(e) => {
                    tracing::error!("Update failed: {}", e);
                    *current_state = Some(UpdateState::Failed(e));
                    HandleResult::None
                }
            }
        }
    }
}

/// Create the update check task for startup
pub fn check_for_updates_task() -> Task<UpdateMessage> {
    Task::future(async move {
        let result = tokio::task::spawn_blocking(|| {
            crate::update::check_for_update()
                .map(|opt| opt.map(|info| (info.current_version, info.latest_version)))
                .map_err(|e| e.to_string())
        }).await;
        
        match result {
            Ok(inner) => UpdateMessage::CheckResult(inner),
            Err(e) => UpdateMessage::CheckResult(Err(e.to_string())),
        }
    })
}

/// Render the update modal
pub fn update_modal<M: Clone + Send + 'static>(
    update_state: Option<&UpdateState>,
    hook: &mut HookManager<M>,
    map: impl Fn(UpdateMessage) -> M + Clone + 'static,
) -> Element<M> {
    let update_state = match update_state {
        Some(s) => s,
        None => return Element::default(),
    };

    // Only show modal for Available or Updating state
    let (current_version, latest_version, is_updating) = match update_state {
        UpdateState::Available { current_version, latest_version } => (current_version.clone(), latest_version.clone(), false),
        UpdateState::Updating => (String::new(), String::new(), true),
        _ => return Element::default(),
    };

    // Header with icon
    let (header_icon, header_text) = if is_updating {
        ("⏳", "Updating...")
    } else {
        ("🎉", "Update Available!")
    };

    let header_section = column![
        Text::new(header_icon)
            .with_font_size(48.0)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .as_element(),
        Text::new(header_text)
            .with_font_size(24.0)
            .with_color(TEXT_COLOR)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .as_element(),
    ]
    .with_child_gap(SPACE_MD)
    .with_cross_align_items(Alignment::Center)
    .with_width(Sizing::grow());

    // Version info (only show when not updating)
    let version_info = if is_updating {
        Text::new("Please wait while the update is being installed...")
            .with_font_size(14.0)
            .with_color(TEXT_MUTED)
            .with_paragraph_alignment(ParagraphAlignment::Center)
            .as_element()
            .with_width(Sizing::grow())
            .with_padding(BoxAmount::vertical(PAD_MD))
    } else {
        column![
            row![
                Text::new("Current version:")
                    .with_font_size(14.0)
                    .with_color(TEXT_MUTED)
                    .as_element(),
                spacer(),
                Text::new(current_version.clone())
                    .with_font_size(14.0)
                    .with_color(TEXT_COLOR)
                    .as_element(),
            ]
            .with_width(Sizing::grow()),
            row![
                Text::new("New version:")
                    .with_font_size(14.0)
                    .with_color(TEXT_MUTED)
                    .as_element(),
                spacer(),
                Text::new(latest_version.clone())
                    .with_font_size(14.0)
                    .with_color(SUCCESS_COLOR)
                    .as_element(),
            ]
            .with_width(Sizing::grow()),
        ]
        .with_child_gap(SPACE_SM)
        .with_width(Sizing::grow())
        .with_padding(BoxAmount::vertical(PAD_MD))
    };

    // Buttons (only shown when not updating)
    let buttons: Element<M> = if !is_updating {
        let update_button = Button::new()
            .with_bg_color(SUCCESS_COLOR)
            .with_border_radius(BORDER_RADIUS)
            .with_click_handler({
                let map = map.clone();
                move |_, shell| shell.publish(map(UpdateMessage::Confirm))
            })
            .as_element(
                w_id!(),
                Text::new("Update Now")
                    .with_font_size(14.0)
                    .with_color(Color::WHITE)
                    .with_paragraph_alignment(ParagraphAlignment::Center)
                    .with_text_alignment(TextAlignment::Center)
                    .as_element()
                    .with_width(Sizing::grow())
                    .with_padding(BoxAmount::new(PAD_MD, PAD_LG, PAD_MD, PAD_LG)),
            )
            .with_width(Sizing::grow());

        let dismiss_button = Button::new()
            .with_bg_color(CARD_BACKGROUND)
            .with_border(1.0, BORDER_COLOR)
            .with_border_radius(BORDER_RADIUS)
            .with_click_handler({
                let map = map.clone();
                move |_, shell| shell.publish(map(UpdateMessage::Dismiss))
            })
            .as_element(
                w_id!(),
                Text::new("Later")
                    .with_font_size(14.0)
                    .with_color(TEXT_MUTED)
                    .with_paragraph_alignment(ParagraphAlignment::Center)
                    .with_text_alignment(TextAlignment::Center)
                    .as_element()
                    .with_width(Sizing::grow())
                    .with_padding(BoxAmount::new(PAD_MD, PAD_LG, PAD_MD, PAD_LG)),
            )
            .with_width(Sizing::grow());

        row![update_button, dismiss_button]
            .with_child_gap(SPACE_MD)
            .with_width(Sizing::grow())
    } else {
        Element::default()
    };

    let modal_content = if is_updating {
        column![
            header_section,
            version_info,
        ]
    } else {
        column![
            header_section,
            version_info,
            buttons,
        ]
    }
    .with_child_gap(SPACE_LG)
    .with_width(Sizing::fixed(350.0))
    .with_padding(BoxAmount::all(PAD_LG * 1.5));

    modal_backdrop(w_id!(), true, hook, None, None, modal_content)
}
