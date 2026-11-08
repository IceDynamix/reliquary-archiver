//! Waiting screen implementation.
//!
//! Displayed when the application is waiting for a game connection.
//! Shows instructions and optionally allows importing a pcap file.

use raxis::layout::model::{Alignment, BackdropFilter, BorderRadius, BoxAmount, Element, Sizing};
use raxis::runtime::task::Task;
use raxis::widgets::button::Button;
use raxis::widgets::rule::Rule;
use raxis::widgets::text::{ParagraphAlignment, Text};
use raxis::{HookManager, column, row, w_id};

use crate::capture::Replay;
use crate::rgui::components::file_download::download_view;
use crate::rgui::messages::{ExportMessage, RootMessage};
use crate::rgui::state::Store;
use crate::rgui::theme::{BORDER_COLOR, BORDER_RADIUS, PAD_LG, PAD_MD, PRIMARY_COLOR, SPACE_MD, SPACE_SM, TEXT_MUTED, maybe_text_shadow};

/// Renders the waiting screen with instructions and the capture file upload.
pub fn view(store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let text_shadow_enabled = store.settings.text_shadow_enabled;

    let upload_button = Button::new()
        .with_bg_color(PRIMARY_COLOR)
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler(move |_, shell| {
            // Open file picker for capture files
            shell.dispatch_task(
                Task::future(async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("Packet Capture", Replay::file_extensions().as_slice())
                        .set_title("Select packet capture file")
                        .pick_file()
                        .await
                })
                .and_then(|file| Task::done(RootMessage::Export(ExportMessage::LoadCaptureFile(file.path().to_path_buf())))),
            );
        })
        .as_element(
            w_id!(),
            Text::new("Upload capture")
                .with_font_size(16.0)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .with_color(raxis::layout::model::Color::WHITE)
                .with_word_wrap(false)
                .as_element()
                .with_padding(BoxAmount::new(PAD_MD, PAD_LG, PAD_MD, PAD_LG))
                .with_height(Sizing::grow()),
        )
        .with_backdrop_filter(BackdropFilter::blur(10.0))
        .with_height(Sizing::grow())
        .with_snap(true);

    let download_section = download_view(store.json_export.as_ref(), store.export_out_of_date, hook);

    let upload_bar = row![upload_button, download_section]
        .with_child_gap(SPACE_MD)
        .with_padding(BoxAmount::all(PAD_MD));

    column![
        maybe_text_shadow(
            Text::new("Waiting for login...")
                .with_font_size(24.0)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            text_shadow_enabled
        ),
        maybe_text_shadow(
            Text::new("Please log into the game. If you are already in-game, you must log out and log back in.")
                .with_font_size(16.0)
                .with_color(TEXT_MUTED)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            text_shadow_enabled
        )
        .as_element()
        .with_padding(BoxAmount::horizontal(PAD_LG)),
        Rule::horizontal()
            .with_color(BORDER_COLOR)
            .as_element(w_id!())
            .with_padding(BoxAmount::vertical(PAD_LG)),
        maybe_text_shadow(
            Text::new("Alternatively, if you have a packet capture file, you can upload it.")
                .with_font_size(16.0)
                .with_color(TEXT_MUTED)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            text_shadow_enabled
        )
        .as_element()
        .with_padding(BoxAmount::horizontal(PAD_LG)),
        upload_bar,
    ]
    .with_child_gap(SPACE_SM)
    .with_cross_align_items(Alignment::Center)
    .with_padding(BoxAmount::all(PAD_LG * 2.0))
    .with_border_radius(BorderRadius::all(BORDER_RADIUS))
}
