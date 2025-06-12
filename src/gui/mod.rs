use std::{sync::{Arc, LazyLock}, time::{Duration, Instant}};

use chrono::Local;
use futures::{lock::Mutex, SinkExt};
use iced::{alignment::Vertical, border, font, widget::{self, button, column, container, container::rounded_box, grid, horizontal_space, row, svg, text, Button}, window::icon, Alignment, Background, Color, Element, Font, Length, Subscription, Task, Theme};
use reliquary_archiver::export::fribbels::{OptimizerEvent, OptimizerExporter};
use stylefns::{rounded_box_md, rounded_button_primary, rounded_button_secondary, text_muted};
use tracing::info;
use widgets::spinner::spinner;
use fonts::{inter, lucide, FontSettings};

mod widgets;
mod fonts;
mod stylefns;

use crate::{websocket::start_websocket_server, worker::{self, archiver_worker}};

const LOGO: &[u8] = include_bytes!("../../assets/icon256.png");

#[derive(Debug, Clone)]
struct FileExtension {
    description: String,
    extensions: Vec<String>,
}

#[derive(Debug, Clone)]
struct FileContainer {
    name: String,
    content: String,
    ext: FileExtension,
}

#[derive(Default)]
pub struct RootState {
    exporter: Arc<Mutex<OptimizerExporter>>,
    ws_status: WebSocketStatus,

    worker_sender: Option<worker::WorkerHandle>,

    connected: bool,
    packets_received: usize,
    commands_received: usize,
    decryption_key_missing: usize,
    network_errors: usize,

    last_command_time: Option<Instant>,

    json_export: Option<FileContainer>,

    test: bool,
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

    CheckConnection(Instant),

    #[cfg(feature="pcap")]
    OpenPcapPicker,

    #[cfg(feature="pcap")]
    OpenPcap(Option<rfd::FileHandle>),

    DownloadFile(FileContainer),

    Test,
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

fn social_button(icon: svg::Handle, brand_color: Color, link: String) -> Button<'static, RootMessage> {
    Button::new(
        svg(icon)
            .width(48)
            .height(48)
            .style(branded_svg(brand_color))
    )
        .padding(4)
        .style(ghost_button)
        .on_press(RootMessage::GoToLink(link))
        .into()
}

fn github_button() -> Button<'static, RootMessage> {
    static GITHUB_LOGO: LazyLock<svg::Handle> = LazyLock::new(|| {
        svg::Handle::from_memory(include_bytes!("../../assets/github.svg"))
    });

    social_button(
        GITHUB_LOGO.clone(), 
        Color::from_rgb8(0x18, 0x17, 0x17),
        "https://github.com/IceDynamix/reliquary-archiver".to_string()
    )
}

fn discord_button() -> Button<'static, RootMessage> {
    static DISCORD_LOGO: LazyLock<svg::Handle> = LazyLock::new(|| {
        svg::Handle::from_memory(include_bytes!("../../assets/discord.svg"))
    });    
    
    social_button(
        DISCORD_LOGO.clone(), 
        Color::from_rgb8(0x58, 0x65, 0xF2),
        "https://discord.gg/EbZXfRDQpu".to_string()
    )
}

fn help_arrow() -> iced::widget::Svg<'static> {
    static HELP_ARROW: LazyLock<svg::Handle> = LazyLock::new(|| {
        svg::Handle::from_memory(include_bytes!("../../assets/arrow_up.svg"))
    });

    svg(HELP_ARROW.clone())
        .width(44)
        .height(44)
        .style(|theme: &Theme, _| svg::Style { color: Some(theme.extended_palette().background.base.text) })
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

fn download_view<'a>(file: Option<&FileContainer>) -> Element<'a, RootMessage> {
    container(
        row![
            button(lucide::arrow_down_to_line(32))
                .style(rounded_button_secondary)
                .padding(8)
                .on_press_maybe(file.map(|f| RootMessage::DownloadFile(f.clone()))),

            // horizontal_space(),

            if let Some(file) = file {
                Element::from(column![
                    text(file.name.clone()).size(14),
                    text(file_size(file.content.len())).size(12).style(text_muted),
                ]
                    .align_x(Alignment::Start)
                    .spacing(4)
                    .padding([4, 8])
                )
            } else {
                text("Export not ready")
                    .size(14)
                    .width(Length::Fill)
                    .align_x(Alignment::Center)
                    .into()
            }

            // horizontal_space(),
        ]
            .align_y(Alignment::Center)
            .spacing(4)
    )
        .style(rounded_box_md)
        .width(400)
        .into()
}

pub fn view(state: &RootState) -> Element<RootMessage> {
    let help_text = text("have questions or issues?").size(16).font(inter().styled(font::Style::Italic).weight(font::Weight::Semibold));

    let icon_row = row![
        github_button(),
        discord_button(),
        help_arrow(),
        // spinner().completed(state.test),
        // button(text("test")).on_press(RootMessage::Test)
    ]
        .align_y(Vertical::Bottom)
        .spacing(4);

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
    
    let mut content = vec![
        text("Waiting for login...").size(24).into(),
        text("Please log into the game. If you are already in-game, you must log out and log back in.").into(),
    ];

    content.push(row![
        button(
            text("Upload .pcap").align_y(Alignment::Center).height(Length::Fill)
        )
            .on_press(RootMessage::OpenPcapPicker)
            .style(rounded_button_primary)
            .padding([8, 16])
            .height(Length::Fill),
        download_view(state.json_export.as_ref()),
    ]
        .height(Length::Shrink)
        .spacing(10)
        .into()
    );

    // content.push();

    let content = column(content).align_x(Alignment::Center);

    let connection_status = if state.connected {
        text(format!("connected, {}/{} pkts/cmds received", state.packets_received, state.commands_received))
    } else {
        text("disconnected").style(text::danger)
    }.size(12);

    let footer = row![
        ws_status,
        widget::horizontal_space(),
        connection_status,
    ]
        .align_y(Vertical::Bottom)
        .spacing(4);

    Into::<Element<RootMessage>>::into(column![
        github_box,
        widget::center(content).padding(20),
        footer,
    ]
        .padding(10)
        .spacing(10))
        // .explain(Color::from_rgb8(0xFF, 0, 0))
}

pub fn update(state: &mut RootState, message: RootMessage) -> Task<RootMessage> {
    // info!("update: {:?}", message);

    match message {
        RootMessage::GoToLink(link) => {
            open::that(link).unwrap();
        }

        RootMessage::WorkerEvent(event) => handle_worker_event(state, event),

        RootMessage::CheckConnection(now) => {
            if let Some(last_command_time) = state.last_command_time {
                if now.duration_since(last_command_time) > Duration::from_secs(60) {
                    // Assume the connection is lost
                    state.connected = false;
                }
            }
        }

        RootMessage::WSStatus(status) => {
            state.ws_status = status;
        }

        #[cfg(feature="pcap")]
        RootMessage::OpenPcapPicker => {
            return Task::perform(
                rfd::AsyncFileDialog::new()
                    .set_title("Select a packet capture file")
                    .add_filter("Packet Captures", &["pcap", "pcapng"])
                    .pick_file(), 
                RootMessage::OpenPcap
            );
        }

        #[cfg(feature="pcap")]
        RootMessage::OpenPcap(file) => {
            if let Some(file) = file {
                let mut sender = state.worker_sender.as_ref().unwrap().clone();
                return Task::future(async move {
                    sender.send(worker::WorkerCommand::ProcessRecorded(file.path().to_path_buf())).await
                }).discard();
            }
        }

        RootMessage::DownloadFile(file) => {            
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&file.name)
                .add_filter(&file.ext.description, &file.ext.extensions)
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, file.content) {
                    eprintln!("Failed to save file: {}", e);
                }
            }
        }

        RootMessage::Test => {
            state.test = !state.test;
        }
    }

    Task::none()
}

fn handle_worker_event(state: &mut RootState, event: worker::WorkerEvent) {
    match event {
        worker::WorkerEvent::Ready(sender) => {
            state.worker_sender = Some(sender);
        }
        worker::WorkerEvent::Metric(metric) => {
            handle_sniffer_metric(state, metric);
        }
        worker::WorkerEvent::ExportEvent(event) => {
            match event {
                OptimizerEvent::InitialScan(scan) => {
                    state.json_export = Some(
                        FileContainer {
                            name: Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string(),
                            content: serde_json::to_string_pretty(&scan).unwrap(),
                            ext: FileExtension {
                                description: "JSON files".to_string(),
                                extensions: vec!["json".to_string()],
                            }
                        }
                    );
                }
                _ => {} // TODO: handle other events
            }
        }
    }
}

fn handle_sniffer_metric(state: &mut RootState, metric: worker::SnifferMetric) {
    match metric {
        worker::SnifferMetric::ConnectionEstablished => {
            state.connected = true;
        }
        worker::SnifferMetric::ConnectionDisconnected => {
            state.connected = false;
        }
        worker::SnifferMetric::NetworkPacketReceived => {
            state.packets_received += 1;
        }
        worker::SnifferMetric::GameCommandsReceived(commands) => {
            state.connected = true; // Must be connected to receive commands
            state.commands_received += commands;
            state.last_command_time = Some(Instant::now());
        }
        worker::SnifferMetric::DecryptionKeyMissing => {
            state.decryption_key_missing += 1;
        }
        worker::SnifferMetric::NetworkError => {
            state.network_errors += 1;
        }
    }
}

pub fn subscription(state: &RootState) -> Subscription<RootMessage> {
    if state.connected {
        iced::time::every(Duration::from_secs(60)).map(|now| RootMessage::CheckConnection(now))
    } else {
        Subscription::none()
    }
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
                            Ok(port) => RootMessage::WSStatus(WebSocketStatus::Running { port }),
                            Err(e) => RootMessage::WSStatus(WebSocketStatus::Failed { error: e })
                        }
                    }),
                ])
            )
        }, 
        update,
        view,
    )
        .title("Reliquary Archiver")
        .window(iced::window::Settings { 
            icon: Some(icon::from_file_data(LOGO, None).expect("Failed to load icon")),
            ..Default::default()
        })
        .subscription(subscription)
        .font(include_bytes!("../../assets/fonts/lucide.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-400-Regular.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-400-Italic.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-500-Medium.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-500-MediumItalic.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-600-SemiBold.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-600-SemiBoldItalic.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-700-Bold.ttf"))
        .font(include_bytes!("../../assets/fonts/inter/Inter_18pt-700-BoldItalic.ttf"))
        .default_font(Font::with_name("Inter 18pt"))
        .theme(|_state| Theme::Oxocarbon)
        .run()
}
