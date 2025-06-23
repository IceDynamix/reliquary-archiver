use std::path::PathBuf;

use futures::SinkExt;
use iced::widget::{self, button, column, container, horizontal_rule, row, text};
use iced::{Alignment, Element, Length, Padding, Task};

use crate::gui::components::file_download::{self, download_view};
use crate::gui::components::{file_picker, FileContainer, FileExtensions};
use crate::gui::stylefns::{rounded_box_md, rounded_button_primary, PAD_LG, PAD_MD, SPACE_LG, SPACE_MD};
use crate::gui::{RootState, Store};
use crate::worker;

#[derive(Debug)]
pub struct WaitingScreen;

#[derive(Debug, Clone)]
pub enum Message {
    UploadPcap(file_picker::Message),
    DownloadExport(file_download::Message),
}

pub enum Action {
    None,
    Run(Task<Message>),
    ProcessCapture(PathBuf),
}

impl WaitingScreen {
    pub fn new() -> Self {
        Self
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::UploadPcap(message) => match file_picker::update(message) {
                file_picker::Action::Run(task) => Action::Run(task.map(Message::UploadPcap)),
                file_picker::Action::FilePicked(None) => Action::None,
                file_picker::Action::FilePicked(Some(file)) => Action::ProcessCapture(file.path().to_path_buf()),
            },
            
            Message::DownloadExport(message) => match file_download::update(message) {
                file_download::Action::None => Action::None,
                file_download::Action::Run(task) => Action::Run(task.map(Message::DownloadExport)),
            },
        }
    }

    pub fn view<'a>(&'a self, store: &'a Store) -> Element<'a, Message> {
        let mut content = vec![
            text("Waiting for login...").size(24).into(),
            text("Please log into the game. If you are already in-game, you must log out and log back in.").into(),
        ];

        content.push(horizontal_rule(SPACE_LG * 2).into());

        content.push(
            container(text("Alternatively, if you have a packet capture file, you can upload it."))
                .padding(Padding::ZERO.bottom(SPACE_MD))
                .into(),
        );

        content.push(
            row![
                file_picker::view(
                    "Upload .pcap", 
                    FileExtensions::of("PCAP files", &["pcap", "pcapng"]),
                    Message::UploadPcap
                ).height(Length::Fill),
                download_view(store.json_export.as_ref(), Message::DownloadExport),
            ]
            .height(Length::Shrink)
            .spacing(SPACE_MD)
            .into(),
        );

        widget::center(column(content).align_x(Alignment::Center))
            .padding(PAD_LG)
            .into()
    }
}
