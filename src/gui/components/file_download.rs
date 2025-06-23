use std::path::PathBuf;

use iced::{widget::{button, column, container, row, text}, Alignment, Element, Length, Task};

use crate::gui::{components::FileContainer, fonts::lucide, stylefns::{rounded_box_md, rounded_button_secondary, text_muted, PAD_MD, PAD_SM, SPACE_SM}};

#[derive(Debug, Clone)]
pub enum Message {
    PickPathForFile(FileContainer),
    SaveFile(FileContainer, PathBuf),
}

pub enum Action {
    None,
    Run(Task<Message>),
}

fn file_size(size: usize) -> String {
    let size_f = size as f64;
    if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.2} KB", size_f / 1024.0)
    } else {
        format!("{:.2} MB", size_f / 1024.0 / 1024.0)
    }
}

pub fn download_view<PMsg: Clone + 'static>(file: Option<&FileContainer>, message: impl Fn(Message) -> PMsg) -> Element<PMsg> {
    container(
        row![
            button(lucide::arrow_down_to_line(32))
                .style(rounded_button_secondary)
                .padding(PAD_MD)
                .on_press_maybe(file.map(|f| message(Message::PickPathForFile(f.clone())))),

            if let Some(file) = file {
                Element::from(
                    column![
                        text(file.name.clone()).size(14),
                        text(file_size(file.content.len())).size(12).style(text_muted),
                    ]
                    .align_x(Alignment::Start)
                    .spacing(SPACE_SM)
                    .padding([PAD_SM, PAD_MD]),
                )
            } else {
                text("Export not ready")
                    .size(14)
                    .width(Length::Fill)
                    .align_x(Alignment::Center)
                    .into()
            }
        ]
        .align_y(Alignment::Center)
        .spacing(SPACE_SM),
    )
    .style(rounded_box_md)
    .width(400)
    .into()
}

pub fn update(message: Message) -> Action {
    match message {
        Message::PickPathForFile(export) => {
            Action::Run(Task::future(
                rfd::AsyncFileDialog::new()
                    .set_file_name(&export.name)
                    .add_filter(&export.ext.description, &export.ext.extensions)
                    .save_file()
            ).and_then(move |file| Task::done(
                Message::SaveFile(export.clone(), file.path().to_path_buf())
            )))
        }
        Message::SaveFile(export, path) => {
            if let Err(e) = std::fs::write(&path, export.content) {
                eprintln!("Failed to save file: {}", e);
            }

            Action::None
        }
    }
}
