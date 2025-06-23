use iced::{widget::{button, text, Button}, Alignment, Element, Length, Task};

use crate::gui::{components::FileExtensions, stylefns::{rounded_button_primary, PAD_LG, PAD_MD}};

#[derive(Debug, Clone)]
pub enum Message {
    OpenPicker(FileExtensions),
    FilePicked(Option<rfd::FileHandle>),
}

pub enum Action {
    Run(Task<Message>),
    FilePicked(Option<rfd::FileHandle>),
}

pub fn view<M>(label: &str, extensions: FileExtensions, message: impl Fn(Message) -> M) -> Button<M> {
    button(text(label.to_string()).align_y(Alignment::Center).height(Length::Fill))
        .style(rounded_button_primary)
        .padding([PAD_MD, PAD_LG])
        .on_press(message(Message::OpenPicker(extensions)))
}

pub fn update(message: Message) -> Action {
    match message {
        Message::OpenPicker(extensions) => Action::Run(
            Task::perform(rfd::AsyncFileDialog::new()
                .add_filter(extensions.description.clone(), &extensions.extensions)
                .pick_file(), Message::FilePicked)
        ),

        Message::FilePicked(file) => Action::FilePicked(file),
    }
}
