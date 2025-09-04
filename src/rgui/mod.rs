use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use chrono::Local;
use futures::channel::oneshot;
use futures::lock::Mutex;
use futures::sink::SinkExt;
use raxis::{
    column,
    layout::{
        helpers::{center, container, row, ElementAlignmentExt, Rule},
        model::{Border, BorderRadius, BoxAmount, Direction, Element, HorizontalAlignment, Sizing, VerticalAlignment},
    },
    row,
    runtime::task::Task,
    w_id,
    widgets::{
        button::Button,
        svg::ViewBox,
        svg_path::SvgPath,
        text::{self, ParagraphAlignment, Text},
        widget, Color,
    },
    HookManager,
};
use reliquary_archiver::export::fribbels::{Export, OptimizerEvent, OptimizerExporter};
use tokio::sync::broadcast;
use tracing::info;

use crate::rgui::components::file_download::{self, download_view};
use crate::scopefns::Also;
use crate::websocket::start_websocket_server;
use crate::worker::{self, archiver_worker};

mod components;

// Constants
pub const PAD_SM: f32 = 4.0;
pub const PAD_MD: f32 = 8.0;
pub const PAD_LG: f32 = 16.0;

pub const SPACE_SM: f32 = 4.0;
pub const SPACE_MD: f32 = 8.0;
pub const SPACE_LG: f32 = 16.0;

pub const BORDER_RADIUS: f32 = 8.0;

// Color constants
const BACKGROUND_LIGHT: u32 = 0xF5F5F5FF;
const TEXT_MUTED: Color = Color {
    r: 0.6,
    g: 0.6,
    b: 0.6,
    a: 1.0,
};
const BORDER_COLOR: Color = Color {
    r: 0.85,
    g: 0.85,
    b: 0.85,
    a: 1.0,
};
const DANGER_COLOR: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.2,
    a: 1.0,
};
const SUCCESS_COLOR: Color = Color {
    r: 0.2,
    g: 0.8,
    b: 0.2,
    a: 1.0,
};
const PRIMARY_COLOR: Color = Color {
    r: 0.2,
    g: 0.6,
    b: 1.0,
    a: 1.0,
};

#[derive(Debug, Clone)]
pub struct FileExtensions {
    pub description: String,
    pub extensions: Vec<String>,
}

impl FileExtensions {
    pub fn of(description: &str, extensions: &[&str]) -> Self {
        Self {
            description: description.to_string(),
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileContainer {
    pub name: String,
    pub content: String,
    pub ext: FileExtensions,
}

// State Management
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
}

#[derive(Debug)]
enum Screen {
    Waiting(WaitingScreen),
    Active(ActiveScreen),
}

impl Default for Screen {
    fn default() -> Self {
        Self::Waiting(WaitingScreen::new())
    }
}

#[derive(Default, Debug)]
pub struct WaitingScreen {
    // Waiting screen specific state
}

impl WaitingScreen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn view(&self, store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
        self.waiting_view(store)
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

    fn waiting_view(&self, _store: &Store) -> Element<RootMessage> {
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
            .as_element(w_id!(), Text::new("Upload .pcap").with_font_size(16.0));

        let upload_bar = row![upload_button]
            .with_child_gap(SPACE_MD)
            .with_horizontal_alignment(HorizontalAlignment::Center);

        column![
            Text::new("Waiting for login...")
                .with_font_size(24.0)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            Text::new("Please log into the game. If you are already in-game, you must log out and log back in.")
                .with_font_size(16.0)
                .with_color(TEXT_MUTED)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            Rule::horizontal().with_color(BORDER_COLOR),
            Text::new("Alternatively, if you have a packet capture file, you can upload it.")
                .with_font_size(16.0)
                .with_color(TEXT_MUTED)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            upload_bar,
        ]
        .with_child_gap(SPACE_LG)
        .with_horizontal_alignment(HorizontalAlignment::Center)
        .with_vertical_alignment(VerticalAlignment::Center)
    }
}

#[derive(Default, Debug)]
pub struct ActiveScreen {
    // Active screen specific state
}

impl ActiveScreen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn view(&self, store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
        self.active_view(store, hook)
    }

    pub fn update(&mut self, _message: ActiveMessage) -> ScreenAction<ActiveMessage> {
        ScreenAction::None
    }

    fn active_view(&self, store: &Store, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
        let stats_display = column![
            Text::new(format!("Relics: {}", store.export_stats.relics)).with_font_size(16.0),
            Text::new(format!("Characters: {}", store.export_stats.characters)).with_font_size(16.0),
            Text::new(format!("Light Cones: {}", store.export_stats.light_cones)).with_font_size(16.0),
            Text::new(format!("Materials: {}", store.export_stats.materials)).with_font_size(16.0),
        ]
        .with_child_gap(SPACE_SM)
        .with_padding(BoxAmount::all(PAD_MD))
        .with_background_color(Color::from(BACKGROUND_LIGHT))
        .with_border_radius(BorderRadius::all(BORDER_RADIUS));

        let export_button = Button::new()
            .with_bg_color(SUCCESS_COLOR)
            .with_border_radius(BORDER_RADIUS)
            .with_click_handler(move |_, shell| {
                shell.publish(RootMessage::RefreshExport);
            })
            .as_element(w_id!(), Text::new("Refresh Export").with_font_size(16.0));

        let download_section = download_view(
            store.json_export.as_ref(),
            // store.export_out_of_date,
            true,
            hook,
        );

        column![
            Text::new("Connection Active!")
                .with_font_size(24.0)
                .with_color(SUCCESS_COLOR)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            stats_display,
            row![export_button, download_section]
                .with_child_gap(SPACE_MD)
                .with_horizontal_alignment(HorizontalAlignment::Center),
        ]
        .with_child_gap(SPACE_LG)
        .with_horizontal_alignment(HorizontalAlignment::Center)
        .with_vertical_alignment(VerticalAlignment::Center)
    }
}

#[derive(Debug, Clone, Default)]
pub enum WebSocketStatus {
    #[default]
    Pending,
    Running {
        port: u16,
        client_count: usize,
    },
    Failed {
        error: String,
    },
}

// Messages
#[derive(Debug, Clone)]
pub enum RootMessage {
    ExportStats(ExportStats),
    NewExport(Export),
    WorkerEvent(worker::WorkerEvent),
    GoToLink(String),
    WSStatus(WebSocketStatus),
    CheckConnection(Instant),
    ActiveScreen(ActiveMessage),
    WaitingScreen(WaitingMessage),
    RefreshExport,
}

#[derive(Debug, Clone)]
pub enum WaitingMessage {
    PcapFileSelected(Option<PathBuf>),
}

#[derive(Debug, Clone)]
pub enum ActiveMessage {
    // Active screen messages
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
                    })
                    .and_then(|e| Task::done(RootMessage::NewExport(e)))
                } else {
                    Task::none()
                }
            }
            #[cfg(feature = "pcap")]
            Self::ProcessCapture(path) => {
                if let Some(sender) = state.worker_sender.as_ref() {
                    let mut sender = sender.clone();
                    Task::future(async move { sender.send(worker::WorkerCommand::ProcessRecorded(path)).await }).discard()
                } else {
                    Task::none()
                }
            }
        }
    }
}

// Helper functions for social buttons
fn github_button() -> Element<RootMessage> {
    Button::new()
        .with_bg_color(Color::from(0x181717FF))
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler(move |_, shell| {
            if let Err(e) = open::that("https://github.com/IceDynamix/reliquary-archiver") {
                tracing::error!("Failed to open GitHub link: {}", e);
            }
        })
        .as_element(w_id!(), Text::new("GitHub").with_font_size(14.0).with_color(Color::WHITE))
}

fn discord_button() -> Element<RootMessage> {
    Button::new()
        .with_bg_color(Color::from(0x5865F2FF))
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler(move |_, shell| {
            if let Err(e) = open::that("https://discord.gg/EbZXfRDQpu") {
                tracing::error!("Failed to open Discord link: {}", e);
            }
        })
        .as_element(w_id!(), Text::new("Discord").with_font_size(14.0).with_color(Color::WHITE))
}

// Main view function
pub fn view(state: &RootState, mut hook: HookManager<RootMessage>) -> Element<RootMessage> {
    let help_text = Text::new("have questions or issues?").with_font_size(16.0).with_color(TEXT_MUTED);

    let social_buttons = row![github_button(), discord_button()]
        .with_child_gap(SPACE_SM)
        .with_vertical_alignment(VerticalAlignment::Center);

    let header = row![
        column![social_buttons, help_text].with_child_gap(SPACE_SM),
        Element::default().with_width(Sizing::grow()), // spacer
    ]
    .with_width(Sizing::grow())
    .with_vertical_alignment(VerticalAlignment::Center);

    let ws_status_text = match &state.store.connection_stats.ws_status {
        WebSocketStatus::Pending => "starting server...".to_string(),
        WebSocketStatus::Running { port, client_count } => {
            if *client_count > 0 {
                format!(
                    "ws://localhost:{}/ws ({} client{})",
                    port,
                    client_count,
                    if *client_count == 1 { "" } else { "s" }
                )
            } else {
                format!("ws://localhost:{}/ws (no clients)", port)
            }
        }
        WebSocketStatus::Failed { error } => format!("failed to start server: {}", error),
    };

    let ws_status = Text::new(ws_status_text)
        .with_font_size(12.0)
        .with_color(match &state.store.connection_stats.ws_status {
            WebSocketStatus::Failed { .. } => DANGER_COLOR,
            _ => TEXT_MUTED,
        })
        .as_element()
        .with_id(w_id!());

    let content = match &state.screen {
        Screen::Waiting(screen) => screen.view(&state.store, &mut hook),
        Screen::Active(screen) => screen.view(&state.store, &mut hook),
    };

    let connection_status_text = if state.store.connection_stats.connected {
        format!(
            "connected, {}/{} pkts/cmds received",
            state.store.connection_stats.packets_received, state.store.connection_stats.commands_received
        )
    } else {
        "disconnected".to_string()
    };

    let connection_status = Text::new(connection_status_text)
        .with_font_size(12.0)
        .with_color(if state.store.connection_stats.connected {
            SUCCESS_COLOR
        } else {
            DANGER_COLOR
        });

    let footer = row![
        ws_status,
        Element::default().with_width(Sizing::grow()), // spacer
        connection_status,
    ]
    .with_width(Sizing::grow())
    .with_vertical_alignment(VerticalAlignment::Bottom);

    column![header, center(content), footer]
        .with_width(Sizing::grow())
        .with_height(Sizing::grow())
        .with_padding(BoxAmount::all(PAD_MD))
        .with_child_gap(SPACE_MD)
}

pub fn update(state: &mut RootState, message: RootMessage) -> Option<Task<RootMessage>> {
    macro_rules! handle_screen {
        ($screen:ident, $screen_message:ident, $message:ident) => {
            if let Screen::$screen(screen) = &mut state.screen {
                Some(screen.update($message).run(state, RootMessage::$screen_message))
            } else {
                None
            }
        };
    }

    match message {
        RootMessage::NewExport(export) => {
            state.store.json_export = Some(FileContainer {
                name: Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string(),
                content: serde_json::to_string_pretty(&export).unwrap(),
                ext: FileExtensions::of("JSON files", &["json"]),
            });
            state.store.export_out_of_date = false;
            None
        }
        RootMessage::ExportStats(stats) => {
            state.store.export_stats = stats;
            None
        }

        RootMessage::GoToLink(link) => {
            if let Err(e) = open::that(link) {
                tracing::error!("Failed to open link: {}", e);
            }
            None
        }

        RootMessage::WorkerEvent(event) => Some(handle_worker_event(state, event)),

        RootMessage::CheckConnection(now) => {
            if let Some(last_packet_time) = state.store.connection_stats.last_packet_time {
                if now.duration_since(last_packet_time) > Duration::from_secs(60) {
                    state.store.connection_stats.connected = false;
                    state.store.connection_stats.connection_active = false;
                    state.store.connection_stats.packets_received = 0;
                    state.store.connection_stats.commands_received = 0;
                }
            }

            if let Some(last_command_time) = state.store.connection_stats.last_command_time {
                if now.duration_since(last_command_time) > Duration::from_secs(60) {
                    state.store.connection_stats.connection_active = false;
                }
            }

            None
        }

        RootMessage::WSStatus(status) => {
            state.store.connection_stats.ws_status = status;
            None
        }

        RootMessage::WaitingScreen(message) => handle_screen!(Waiting, WaitingScreen, message),
        RootMessage::ActiveScreen(message) => handle_screen!(Active, ActiveScreen, message),

        RootMessage::RefreshExport => {
            if let Some(sender) = state.worker_sender.as_ref() {
                let mut sender = sender.clone();
                Some(
                    Task::future(async move {
                        let (tx, rx) = oneshot::channel();
                        sender.send(worker::WorkerCommand::MakeExport(tx)).await;
                        rx.await.unwrap()
                    })
                    .and_then(|e| Task::done(RootMessage::NewExport(e))),
                )
            } else {
                None
            }
        }
    }
    .also(|_| {
        // Handle connection transitions
        let is_connected = state.store.connection_stats.connection_active;
        let is_waiting = matches!(&state.screen, Screen::Waiting(_));
        if is_connected && is_waiting {
            state.screen = Screen::Active(ActiveScreen::new());
        } else if !is_connected && !is_waiting {
            state.screen = Screen::Waiting(WaitingScreen::new());
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
            state.store.export_out_of_date = true;

            let task = match event {
                OptimizerEvent::InitialScan(scan) => Task::done(RootMessage::NewExport(scan)),
                _ => Task::none(),
            };

            let exporter = state.exporter.clone();
            return Task::batch([
                task,
                Task::future(async move {
                    let mut exporter = exporter.lock().await;
                    let stats = ExportStats::new(&exporter);
                    RootMessage::ExportStats(stats)
                }),
            ]);
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

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let state = RootState::default();
    let exporter = state.exporter.clone();

    raxis::runtime::run_event_loop(view, update, state, move |_state| {
        Some(Task::batch(vec![
            Task::run(archiver_worker(exporter.clone()), |e| RootMessage::WorkerEvent(e)),
            Task::future(start_websocket_server(53313, exporter.clone()))
                .then(|e| match e {
                    Err(e) => Task::done(WebSocketStatus::Failed { error: e }),
                    Ok((port, client_count_stream)) => Task::done(WebSocketStatus::Running { port, client_count: 0 })
                        .chain(Task::stream(client_count_stream).map(move |client_count| WebSocketStatus::Running { port, client_count })),
                })
                .map(|e| RootMessage::WSStatus(e)),
        ]))
    })?;

    Ok(())
}
