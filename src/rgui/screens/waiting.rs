use std::path::PathBuf;

use raxis::layout::model::{Alignment, BorderRadius, BoxAmount, Element, Sizing};
use raxis::widgets::button::Button;
use raxis::widgets::rule::Rule;
use raxis::widgets::text::{ParagraphAlignment, Text};
use raxis::layout::model::BackdropFilter;
use raxis::{column, row, w_id, HookManager};
use raxis::runtime::task::Task;

use crate::rgui::theme::{
    maybe_text_shadow, BORDER_COLOR, BORDER_RADIUS, PAD_LG, PAD_MD, PRIMARY_COLOR, SPACE_MD, SPACE_SM, TEXT_MUTED,
};
use crate::rgui::state::{Store, WaitingScreen};
use crate::rgui::messages::{RootMessage, WaitingMessage, ScreenAction};
use crate::rgui::components::file_download::download_view;

impl WaitingScreen {
    pub fn view(&self, store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
        self.waiting_view(store, hook)
    }

    pub fn update(&mut self, message: WaitingMessage) -> ScreenAction<WaitingMessage> {
        match message {
            WaitingMessage::PcapFileSelected(Some(path)) => {
                tracing::info!("Processing pcap file: {:?}", path);
                #[cfg(feature = "pcap")]
                {
                    ScreenAction::ProcessCapture(path)
                }
                #[cfg(not(feature = "pcap"))]
                {
                    tracing::warn!("PCAP feature not enabled");
                    ScreenAction::None
                }
            }
            WaitingMessage::PcapFileSelected(None) => {
                tracing::info!("No file selected");
                ScreenAction::None
            }
        }
    }

    fn waiting_view(&self, store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
        let text_shadow_enabled = store.settings.text_shadow_enabled;

        let upload_button = Button::new()
            .with_bg_color(PRIMARY_COLOR)
            .with_border_radius(BORDER_RADIUS)
            .with_click_handler(move |_, shell| {
                // Open file picker for .pcap files
                shell.dispatch_task(Task::future(async {
                    let file = rfd::AsyncFileDialog::new()
                        .add_filter("Packet Capture", &["pcap", "pcapng", "etl"])
                        .set_title("Select packet capture file")
                        .pick_file()
                        .await;

                    RootMessage::WaitingScreen(WaitingMessage::PcapFileSelected(file.map(|f| f.path().to_path_buf())))
                }));
            })
            .as_element(
                w_id!(),
                Text::new("Upload .pcap")
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
}
