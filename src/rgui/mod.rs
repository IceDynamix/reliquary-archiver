use raxis::{
    column,
    layout::{
        helpers::{center, container, ElementAlignmentExt, Rule},
        model::{Element, HorizontalAlignment, Sizing, VerticalAlignment},
    },
    row,
    runtime::task::Task,
    w_id,
    widgets::{
        button::Button,
        text::{self, Text},
    },
    HookManager,
};

pub enum Message {}

#[derive(Default)]
pub struct State {}

pub const PAD_SM: u32 = 4;
pub const PAD_MD: u32 = 8;
pub const PAD_LG: u32 = 16;

pub const SPACE_SM: u32 = 4;
pub const SPACE_MD: u32 = 8;
pub const SPACE_LG: u32 = 16;

pub const BORDER_RADIUS: f32 = 8.0;

fn upload_bar() -> Element<Message> {
    row![Button::new()
        .with_border_radius(BORDER_RADIUS)
        .as_element(w_id!(), Text::new("Upload .pcap").with_font_size(16.0))
        .with_padding(PAD_MD)]
}

fn main_area() -> Element<Message> {
    // Text::new("hello world").as_element().with_background_color(0xFF0000FF)
    column![
        Text::new("Waiting for login...").with_font_size(24.0),
        Text::new("Please log into the game. If you are already in-game, you must log out and log back in.").with_font_size(16.0),
        //
        Rule::horizontal(),
        //
        Text::new("Alternatively, if you have a packet capture file, you can upload it.").with_font_size(16.0),
        upload_bar(),
    ]
    .align_x(HorizontalAlignment::Center)
}

fn view(_state: &State, hook: HookManager<Message>) -> Element<Message> {
    // let main_area = column![container(main_area()).with_horizontal_alignment(HorizontalAlignment::Center)]
    //     .with_width(Sizing::grow())
    //     .with_height(Sizing::grow())
    //     .with_vertical_alignment(VerticalAlignment::Center);
    let main_area = center(main_area());
    let status_bar = row![
        Text::new("WS Status").as_element().with_width(Sizing::grow()),
        Text::new("Server Status")
    ]
    .with_width(Sizing::grow());

    column![main_area, status_bar]
        .with_width(Sizing::grow())
        .with_height(Sizing::grow())
}

fn update(_state: &mut State, _message: Message) -> Option<Task<Message>> {
    None
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    raxis::runtime::run_event_loop(view, update, State::default(), |_| None)?;

    Ok(())
}
