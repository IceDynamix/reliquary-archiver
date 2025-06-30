use std::path::PathBuf;

use futures::SinkExt;
use iced::widget::{self, button, column, container, horizontal_rule, horizontal_space, row, text, Rule, Space};
use iced::{Alignment, Element, Length, Padding, Task};

use crate::gui::components::file_download::{self, download_view};
use crate::gui::components::{FileContainer, FileExtensions, file_picker};
use crate::gui::fonts::lucide::refresh_cw;
use crate::gui::stylefns::{
    PAD_LG, PAD_MD, PAD_SM, SPACE_LG, SPACE_MD, SPACE_SM, rounded_box_md, rounded_button_primary,
};
use crate::gui::widgets::dashed::DashedRule;
use crate::gui::widgets::spinner::spinner;
use crate::gui::{RootState, ScreenAction, Store};
use crate::worker;

#[derive(Debug)]
pub struct ActiveScreen;

#[derive(Debug, Clone)]
pub enum Message {
    DownloadExport(file_download::Message),
    RefreshExport,
}

fn stat(value: usize, label: &str) -> Element<Message> {
    row![
        spinner().size(32.0).completed(value > 0),
        // horizontal_space(),
        DashedRule::horizontal(1),
        text(format!("found {} {}", value, label))
    ]
    .align_y(Alignment::Center)
    .padding(PAD_SM)
    .spacing(SPACE_MD)
    .into()
}

impl ActiveScreen {
    pub fn new() -> Self {
        Self
    }

    pub fn update(&mut self, message: Message) -> ScreenAction<Message> {
        match message {
            Message::DownloadExport(message) => match file_download::update(message) {
                file_download::Action::None => ScreenAction::None,
                file_download::Action::Run(task) => ScreenAction::Run(task.map(Message::DownloadExport)),
            },
            Message::RefreshExport => ScreenAction::RefreshExport,
        }
    }

    pub fn view<'a>(&'a self, store: &'a Store) -> Element<'a, Message> {
        let mut content = column![text("Connected!").size(24),]
            .push(
                column![
                    stat(store.export_stats.relics, "relics"),
                    stat(store.export_stats.characters, "characters"),
                    stat(store.export_stats.light_cones, "light cones"),
                    stat(store.export_stats.materials, "materials"),
                ]
                .align_x(Alignment::Start)
                .width(Length::Fill),
            )
            .push(Space::with_height(SPACE_LG))
            .push(text("Reliquary Archiver will continue to capture data in the background."))
            .push(text("You can now enable 'Live Import' in the optimizer."))
            .push(Rule::horizontal(SPACE_LG * 2))
            .push(text("If you wish to import manually, you can download a JSON export below."))
            .push(Space::with_height(SPACE_LG))
            .push(
                row![
                    button(refresh_cw(32))
                        .on_press_maybe(store.json_export.as_ref().map(|_| Message::RefreshExport))
                        .padding(PAD_MD)
                        .style(rounded_button_primary),
                    download_view(store.json_export.as_ref(), Message::DownloadExport, store.export_out_of_date)
                ]
                .spacing(SPACE_MD),
            )
            ;

        widget::center(content.width(Length::Shrink).align_x(Alignment::Center)).padding(PAD_LG).into()
    }
}
