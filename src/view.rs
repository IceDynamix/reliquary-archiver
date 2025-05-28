use std::sync::Arc;

use futures::{channel::oneshot, lock::Mutex, StreamExt};
use iced::{alignment::Vertical, border, font, padding, widget::{self, button, column, mouse_area, row, svg, text, Button, Text}, window::icon, Background, Border, Color, Element, Font, Task, Theme};
use reliquary_archiver::export::fribbels::OptimizerExporter;
use tracing::info;

use crate::{websocket::start_websocket_server, worker::{self, archiver_worker}};

const LOGO: &[u8] = include_bytes!("../assets/icon256.png");

const HELP_ARROW: &[u8] = include_bytes!("../assets/arrow_up.svg");

#[derive(Default)]
pub struct RootState {
    exporter: Arc<Mutex<OptimizerExporter>>,
    ws_status: WebSocketStatus,

    worker_sender: Option<worker::WorkerHandle>,
}

#[derive(Debug, Clone, Default)]
pub enum WebSocketStatus {
    #[default]
    Pending,
    Running { port: u16 },
    Failed { error: String },
}

#[derive(Debug, Clone)]
pub enum RootMessage {
    WorkerEvent(worker::WorkerEvent),

    GoToLink(String),

    WSStatus(WebSocketStatus),
}

trait FontSettings {
    fn weight(self, weight: font::Weight) -> Self;
    fn styled(self, style: font::Style) -> Self;
}

impl FontSettings for Font {
    fn weight(mut self, weight: font::Weight) -> Self {
        self.weight = weight;
        self
    }

    fn styled(mut self, style: font::Style) -> Self {
        self.style = style;
        self
    }
}

fn inter() -> Font {
    Font::with_name("Inter 18pt")
}

fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        background: None,
        text_color: palette.secondary.base.text,
        border: border::rounded(8),
        ..button::Style::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(palette.secondary.base.color)),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(palette.secondary.strong.color)),
            ..base
        },
        button::Status::Active | button::Status::Disabled => base,
    }
}

fn branded_svg(brand_color: Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |theme: &Theme, _status: svg::Status| {
        let palette = theme.extended_palette();
        svg::Style {
            color: if palette.is_dark {
                Some(palette.background.base.text)
            } else {
                Some(brand_color)
            }
        }
    }
}

fn social_button(icon: svg::Handle, brand_color: Color, link: String) -> Element<'static, RootMessage> {
    let button = Button::new(
        svg(icon)
            .width(48)
            .height(48)
            .style(branded_svg(brand_color))
    )
        .padding(4)
        .style(ghost_button)
        .on_press(RootMessage::GoToLink(link));

    mouse_area(button).interaction(iced::mouse::Interaction::Pointer).into()
}

fn github_button() -> Element<'static, RootMessage> {
    social_button(
        svg::Handle::from_memory(include_bytes!("../assets/github.svg")), 
        Color::from_rgb8(0x18, 0x17, 0x17),
        "https://github.com/IceDynamix/reliquary-archiver".to_string()
    )
}

fn discord_button() -> Element<'static, RootMessage> {
    social_button(
        svg::Handle::from_memory(include_bytes!("../assets/discord.svg")), 
        Color::from_rgb8(0x58, 0x65, 0xF2),
        "https://discord.gg/EbZXfRDQpu".to_string()
    )
}

pub fn view(state: &RootState) -> Element<RootMessage> {
    let help_text = text("have questions or issues?").size(16).font(inter().styled(font::Style::Italic).weight(font::Weight::Semibold));

    let help_arrow = svg(svg::Handle::from_memory(HELP_ARROW))
        .width(44)
        .height(44)
        .style(|theme: &Theme, _| svg::Style { color: Some(theme.extended_palette().background.base.text) });

    let icon_row = row![
        github_button(),
        discord_button(),
        help_arrow
    ].align_y(Vertical::Bottom);

    let github_box = column![
        icon_row,
        help_text,
    ];

    let ws_status = match &state.ws_status {
        WebSocketStatus::Pending => text("starting server..."),
        WebSocketStatus::Running { port } => text(format!("ws://localhost:{}", port)),
        WebSocketStatus::Failed { error } => text(format!("failed to start server: {}", error)).style(text::danger),
    }
        .size(12);
    
    column![
        github_box,
        widget::bottom(ws_status)
    ]
        .padding(10)
        .spacing(10)
        .into()
}

pub fn update(state: &mut RootState, message: RootMessage) -> Task<RootMessage> {
    // match message {
    // }
    info!("update: {:?}", message);

    match message {
        RootMessage::GoToLink(link) => {
            open::that(link).unwrap();
        }
        RootMessage::WorkerEvent(worker::WorkerEvent::Ready(sender)) => {
            state.worker_sender = Some(sender);
        }
        RootMessage::WorkerEvent(worker::WorkerEvent::Event(_)) => {
            // TODO: handle event
        }
        RootMessage::WSStatus(status) => {
            state.ws_status = status;
        }
    }

    Task::none()
}

pub fn run() -> iced::Result {
    iced::application(|| {
            let state = RootState::default();
            let exporter = state.exporter.clone();

            (
                state, 
                Task::batch(vec![
                    Task::run(archiver_worker(exporter.clone()), |e| RootMessage::WorkerEvent(e)),
                    Task::future(start_websocket_server(53313, exporter.clone())).map(|e| {
                        match e {
                            Ok(()) => RootMessage::WSStatus(WebSocketStatus::Running { port: 53313 }),
                            Err(e) => RootMessage::WSStatus(WebSocketStatus::Failed { error: e })
                        }
                    }),
                ])
            )
        }, 
        update,
        view,
    )
        .window(iced::window::Settings { 
            icon: Some(icon::from_file_data(LOGO, None).expect("Failed to load icon")),
            ..Default::default()
        })
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-400-Regular.ttf"))
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-400-Italic.ttf"))
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-500-Medium.ttf"))
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-500-MediumItalic.ttf"))
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-600-SemiBold.ttf"))
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-600-SemiBoldItalic.ttf"))
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-700-Bold.ttf"))
        .font(include_bytes!("../assets/fonts/inter/Inter_18pt-700-BoldItalic.ttf"))
        .default_font(Font::with_name("Inter 18pt"))
        .theme(|_state| Theme::Light)
        .run()
}
