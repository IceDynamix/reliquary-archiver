use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use chrono::Local;
use fonts::{FontSettings, inter, lucide};
use futures::channel::oneshot;
use futures::SinkExt;
use futures::lock::Mutex;
use iced::alignment::Vertical;
use iced::widget::{self, button, column, container, grid, horizontal_rule, horizontal_space, row, rule, svg, text, toggler, Button};
use iced::window::icon;
use iced::{border, font, Alignment, Background, Color, Element, Font, Length, Padding, Subscription, Task, Theme};
use reliquary_archiver::export::fribbels::{Export, OptimizerEvent, OptimizerExporter};
use stylefns::{rounded_box_md, rounded_button_primary, rounded_button_secondary, text_muted};
use tracing::info;
use widgets::spinner::spinner;

mod components;
mod fonts;
mod screens;
mod stylefns;
mod utils;
mod widgets;

use crate::gui::components::file_download::download_view;
use crate::gui::components::{FileContainer, FileExtensions};
use crate::gui::stylefns::{ghost_button, PAD_LG, PAD_MD, PAD_SM, SPACE_MD, SPACE_SM};
use crate::scopefns::Also;
use crate::websocket::start_websocket_server;
use crate::worker::{self, archiver_worker};

const LOGO: &[u8] = include_bytes!("../../assets/icon256.png");

#[derive(Default)]
pub struct Store {
    json_export: Option<FileContainer>,
    export_out_of_date: bool,
    connection_stats: StatsStore,
    export_stats: ExportStats,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct ExportStats {
    relics: usize,
    characters: usize,
    light_cones: usize,
    materials: usize,
}

impl ExportStats {
    pub fn new(exporter: &OptimizerExporter) -> Self {
        Self {
            relics: exporter.relics.len(),
            characters: exporter.characters.len(),
            light_cones: exporter.light_cones.len(),
            materials: exporter.materials.len(),
        }
    }
}

#[derive(Default)]
pub struct StatsStore {
    ws_status: WebSocketStatus,

    connected: bool,
    connection_active: bool,
    packets_received: usize,
    commands_received: usize,
    decryption_key_missing: usize,
    network_errors: usize,

    last_packet_time: Option<Instant>,
    last_command_time: Option<Instant>,
}

#[derive(Default)]
pub struct RootState {
    exporter: Arc<Mutex<OptimizerExporter>>,
    worker_sender: Option<worker::WorkerHandle>,

    store: Store,

    screen: Screen,

    explain: bool,
}

#[derive(Debug)]
enum Screen {
    Waiting(screens::waiting::WaitingScreen),
    Active(screens::active::ActiveScreen),
}

impl Default for Screen {
    fn default() -> Self {
        Self::Waiting(screens::waiting::WaitingScreen::new())
    }
}

#[derive(Debug, Clone, Default)]
pub enum WebSocketStatus {
    #[default]
    Pending,
    Running {
        port: u16,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone)]
pub enum RootMessage {
    ToggleExplain(bool),

    ExportStats(ExportStats),
    NewExport(Export),

    WorkerEvent(worker::WorkerEvent),

    GoToLink(String),

    WSStatus(WebSocketStatus),

    CheckConnection(Instant),

    ActiveScreen(screens::active::Message),
    WaitingScreen(screens::waiting::Message),
}

fn branded_svg(brand_color: Color) -> impl Fn(&Theme, svg::Status) -> svg::Style {
    move |theme: &Theme, _status: svg::Status| {
        let palette = theme.extended_palette();
        svg::Style {
            color: if palette.is_dark {
                Some(palette.background.base.text)
            } else {
                Some(brand_color)
            },
        }
    }
}

fn social_button(icon: svg::Handle, brand_color: Color, link: String) -> Button<'static, RootMessage> {
    Button::new(svg(icon).width(48).height(48).style(branded_svg(brand_color)))
        .padding(PAD_SM)
        .style(ghost_button)
        .on_press(RootMessage::GoToLink(link))
        .into()
}

fn github_button() -> Button<'static, RootMessage> {
    static GITHUB_LOGO: LazyLock<svg::Handle> = LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../../assets/github.svg")));

    social_button(
        GITHUB_LOGO.clone(),
        Color::from_rgb8(0x18, 0x17, 0x17),
        "https://github.com/IceDynamix/reliquary-archiver".to_string(),
    )
}

fn discord_button() -> Button<'static, RootMessage> {
    static DISCORD_LOGO: LazyLock<svg::Handle> = LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../../assets/discord.svg")));

    social_button(
        DISCORD_LOGO.clone(),
        Color::from_rgb8(0x58, 0x65, 0xF2),
        "https://discord.gg/EbZXfRDQpu".to_string(),
    )
}

fn help_arrow() -> iced::widget::Svg<'static> {
    static HELP_ARROW: LazyLock<svg::Handle> = LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../../assets/arrow_up.svg")));

    svg(HELP_ARROW.clone()).width(44).height(44).style(|theme: &Theme, _| svg::Style {
        color: Some(theme.extended_palette().background.base.text),
    })
}

pub fn view(state: &RootState) -> Element<RootMessage> {
    let help_text = text("have questions or issues?")
        .size(16)
        .font(inter().styled(font::Style::Italic).weight(font::Weight::Semibold));

    let icon_row = row![
        github_button(),
        discord_button(),
        help_arrow(),
    ]
    .align_y(Vertical::Bottom)
    .spacing(SPACE_SM);

    let github_box = column![icon_row, help_text,];

    let header = row![
        github_box, 
        horizontal_space(),
        row![
            text("explain").size(12),
            toggler(state.explain).on_toggle(|value| RootMessage::ToggleExplain(value)),
        ].align_y(Alignment::Center).spacing(SPACE_SM),
    ];

    let ws_status = match &state.store.connection_stats.ws_status {
        WebSocketStatus::Pending => text("starting server..."),
        WebSocketStatus::Running { port } => text(format!("ws://localhost:{}/ws", port)),
        WebSocketStatus::Failed { error } => text(format!("failed to start server: {}", error)).style(text::danger),
    }
    .size(12);

    let content = match &state.screen {
        Screen::Waiting(screen) => screen.view(&state.store).map(RootMessage::WaitingScreen),
        Screen::Active(screen) => screen.view(&state.store).map(RootMessage::ActiveScreen),
    };

    let connection_status = if state.store.connection_stats.connected {
        text(format!(
            "connected, {}/{} pkts/cmds received",
            state.store.connection_stats.packets_received, state.store.connection_stats.commands_received
        ))
    } else {
        text("disconnected").style(text::danger)
    }
    .size(12);

    let footer = row![ws_status, widget::horizontal_space(), connection_status,]
        .align_y(Vertical::Bottom)
        .spacing(SPACE_SM);

    let el = Into::<Element<RootMessage>>::into(
        column![header, widget::center(content), footer,]
            .padding(PAD_MD)
            .spacing(SPACE_MD),
    );

    if state.explain {
        el.explain(Color::from_rgb8(0xFF, 0, 0))
    } else {
        el
    }
}

pub enum ScreenAction<Message> {
    None,
    Run(Task<Message>),
    RefreshExport,

    #[cfg(feature = "pcap")]
    ProcessCapture(PathBuf),
}

impl<Message: Send + 'static> ScreenAction<Message> {
    pub fn run(self, state: &mut RootState, wrapper: impl Send + Fn(Message) -> RootMessage + 'static) -> Task<RootMessage> {
        match self {
            Self::None => Task::none(),
            Self::Run(task) => task.map(wrapper),
            Self::RefreshExport => {
                if let Some(sender) = state.worker_sender.as_ref() {
                    let mut sender = sender.clone();
                    Task::future(async move {
                        let (tx, rx) = oneshot::channel();
                        sender.send(worker::WorkerCommand::MakeExport(tx)).await;
                        rx.await.unwrap()
                    }).and_then(|e| Task::done(RootMessage::NewExport(e)))
                } else {
                    Task::none()
                }
            }
            
            #[cfg(feature = "pcap")]
            Self::ProcessCapture(path) => {
                if let Some(sender) = state.worker_sender.as_ref() {
                    let mut sender = sender.clone();
                    Task::future(async move { sender.send(worker::WorkerCommand::ProcessRecorded(path)).await })
                        .discard()
                } else {
                    Task::none()
                }
            }
        }
    }
}

pub fn update(state: &mut RootState, message: RootMessage) -> Task<RootMessage> {
    macro_rules! handle_screen {
        ($screen:ident, $message:ident) => {
            if let Screen::$screen(screen) = &mut state.screen {
                screen.update($message).run(state, RootMessage::${concat($screen, Screen)})
            } else {
                Task::none()
            }
        };
    }

    match message {
        RootMessage::ToggleExplain(value) => {
            state.explain = value;
            Task::none()
        }

        RootMessage::NewExport(export) => {
            state.store.json_export = Some(FileContainer {
                name: Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string(),
                content: serde_json::to_string_pretty(&export).unwrap(),
                ext: FileExtensions::of("JSON files", &["json"]),
            });
            state.store.export_out_of_date = false;
            Task::none()
        }
        RootMessage::ExportStats(stats) => {
            state.store.export_stats = stats;
            Task::none()
        }

        RootMessage::GoToLink(link) => {
            open::that(link).unwrap();

            Task::none()
        }

        RootMessage::WorkerEvent(event) => handle_worker_event(state, event),

        RootMessage::CheckConnection(now) => {
            if let Some(last_packet_time) = state.store.connection_stats.last_packet_time {
                if now.duration_since(last_packet_time) > Duration::from_secs(60) {
                    // Assume the connection is lost
                    state.store.connection_stats.connected = false;
                    state.store.connection_stats.connection_active = false;
                    state.store.connection_stats.packets_received = 0;
                    state.store.connection_stats.commands_received = 0;
                }
            }

            if let Some(last_command_time) = state.store.connection_stats.last_command_time {
                if now.duration_since(last_command_time) > Duration::from_secs(60) {
                    // Assume the connection is lost
                    state.store.connection_stats.connection_active = false;
                }
            }

            Task::none()
        }

        RootMessage::WSStatus(status) => {
            state.store.connection_stats.ws_status = status;

            Task::none()
        }

        RootMessage::WaitingScreen(message) => handle_screen!(Waiting, message),
        RootMessage::ActiveScreen(message) => handle_screen!(Active, message),
    }.also(|_| {
        // Handle connection transitions
        let is_connected = state.store.connection_stats.connection_active;
        let is_waiting = matches!(&state.screen, Screen::Waiting(_));
        if is_connected && is_waiting {
            state.screen = Screen::Active(screens::active::ActiveScreen::new());
        } else if !is_connected && !is_waiting {
            state.screen = Screen::Waiting(screens::waiting::WaitingScreen::new());
        }
    })
}

fn handle_worker_event(state: &mut RootState, event: worker::WorkerEvent) -> Task<RootMessage> {
    match event {
        worker::WorkerEvent::Ready(sender) => {
            state.worker_sender = Some(sender);
        }
        worker::WorkerEvent::Metric(metric) => {
            handle_sniffer_metric(state, metric);
        }
        worker::WorkerEvent::ExportEvent(event) => {
            // For now we assume any exporter event means the export is out of date
            state.store.export_out_of_date = true;

            let task = match event {
                OptimizerEvent::InitialScan(scan) => {
                    // let json = serde_json::to_string_pretty(&scan).unwrap();
                    // state.store.json_export = Some(FileContainer {
                    //     name: Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string(),
                    //     content: json,
                    //     ext: FileExtensions::of("JSON files", &["json"]),
                    // });
                    Task::done(RootMessage::NewExport(scan))
                }
                _ => Task::none(), // TODO: handle other events
            };

            let exporter = state.exporter.clone();
            return Task::batch([task, Task::future(async move {
                let mut exporter = exporter.lock().await;
                let stats = ExportStats::new(&exporter);

                RootMessage::ExportStats(stats)
            })]);
        }
    }

    Task::none()
}

fn handle_sniffer_metric(state: &mut RootState, metric: worker::SnifferMetric) {
    let stats = &mut state.store.connection_stats;

    match metric {
        worker::SnifferMetric::ConnectionEstablished => {
            stats.connected = true;
        }
        worker::SnifferMetric::ConnectionDisconnected => {
            stats.connected = false;
            stats.connection_active = false;
            stats.packets_received = 0;
            stats.commands_received = 0;
        }
        worker::SnifferMetric::NetworkPacketReceived => {
            stats.connected = true;
            stats.last_packet_time = Some(Instant::now());
            stats.packets_received += 1;
        }
        worker::SnifferMetric::GameCommandsReceived(commands) => {
            if commands > 0 {
                // Must be connected to receive commands
                stats.connected = true;
                stats.connection_active = true;

                stats.commands_received += commands;
                stats.last_command_time = Some(Instant::now());
            }
        }
        worker::SnifferMetric::DecryptionKeyMissing => {
            stats.decryption_key_missing += 1;
        }
        worker::SnifferMetric::NetworkError => {
            stats.network_errors += 1;
        }
    }
}

pub fn subscription(state: &RootState) -> Subscription<RootMessage> {
    if state.store.connection_stats.connected {
        iced::time::every(Duration::from_secs(60)).map(|now| RootMessage::CheckConnection(now))
    } else {
        Subscription::none()
    }
}

pub fn run() -> iced::Result {
    iced::application(
        || {
            let state = RootState::default();
            let exporter = state.exporter.clone();

            (
                state,
                Task::batch(vec![
                    Task::run(archiver_worker(exporter.clone()), |e| RootMessage::WorkerEvent(e)),
                    Task::future(start_websocket_server(53313, exporter.clone())).map(|e| match e {
                        Ok(port) => RootMessage::WSStatus(WebSocketStatus::Running { port }),
                        Err(e) => RootMessage::WSStatus(WebSocketStatus::Failed { error: e }),
                    }),
                ]),
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
    .window_size([720.0, 580.0])
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
    .theme(|_state| match Theme::default() {
        Theme::Light => Theme::CatppuccinLatte,
        Theme::Dark => Theme::CatppuccinMocha,
        _ => unreachable!(),
    })
    .run()
}
