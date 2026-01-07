use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::env;
use std::ops::{Deref, Range};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use async_stream::stream;
use chrono::Local;
use futures::channel::oneshot;
use futures::lock::Mutex;
use futures::sink::SinkExt;
use raxis::gfx::color::Oklch;
use raxis::layout::helpers::spacer;
use raxis::layout::model::{
    Alignment2D, BackdropFilter, Color, DropShadow, FloatingConfig, ScrollBarSize, ScrollConfig, ScrollbarStyle, StrokeLineCap, StrokeLineJoin, TextShadow
};
use raxis::runtime::font_manager::{FontIdentifier, FontWeight};
use raxis::runtime::scroll::ScrollPosition;
use raxis::runtime::task::{hide_window, show_window, ClipboardAction, WindowMode};
use raxis::runtime::vkey::VKey;
use raxis::runtime::window::builder::InitialDisplay;
use raxis::runtime::{task, Backdrop};
use raxis::util::str::StableString;
use raxis::util::unique::combine_id;
use raxis::widgets::image::Image;
use raxis::widgets::mouse_area::{MouseArea, MouseAreaEvent};
use raxis::widgets::rule::{horizontal_rule, Rule};
use raxis::widgets::slider::Slider;
use raxis::widgets::svg::Svg;
use raxis::widgets::svg_path::ColorChoice;
use raxis::widgets::text::TextAlignment;
use raxis::widgets::text_input::TextInput;
use raxis::widgets::toggle::Toggle;
use raxis::widgets::titlebar_controls::titlebar_controls;
use raxis::widgets::Widget;
use raxis::{
    column,
    layout::{
        helpers::{center, container, row},
        model::{Alignment, Border, BorderRadius, BoxAmount, Direction, Element, Sizing},
    },
    row,
    runtime::task::Task,
    w_id,
    widgets::{
        button::Button,
        svg::ViewBox,
        svg_path::SvgPath,
        text::{self, ParagraphAlignment, Text},
        widget,
    },
    HookManager,
};
use raxis::{
    svg, svg_path, use_animation, ContextMenuItem, SvgPathCommands, SystemCommand, SystemCommandResponse, TrayEvent, TrayIconConfig,
};
use reliquary_archiver::export::fribbels::{Export, OptimizerEvent, OptimizerExporter};
use tokio::sync::watch::{self, Sender};
use tokio_stream::wrappers::WatchStream;
use tracing::{error, info};
use tracing::level_filters::LevelFilter;

use crate::rgui::components::file_download::{self, download_view};
use crate::rgui::components::modal::{ModalConfig, ModalPosition, modal_backdrop};
use crate::rgui::components::togglegroup::{ToggleGroupConfig, ToggleOption, togglegroup};
use crate::rgui::components::update::{self, UpdateState, UpdateMessage, update_modal};
use crate::scopefns::Also;
use crate::websocket::{PortSource, start_websocket_server};
use crate::worker::{self, archiver_worker};
use crate::{LOG_BUFFER, LOG_NOTIFY, VEC_LAYER_HANDLE};
use run_on_start::{RegistryError, registry_matches_settings, set_run_on_start};

mod components;
mod run_on_start;

// Constants
pub const PAD_SM: f32 = 4.0;
pub const PAD_MD: f32 = 8.0;
pub const PAD_LG: f32 = 16.0;

pub const SPACE_SM: f32 = 4.0;
pub const SPACE_MD: f32 = 8.0;
pub const SPACE_LG: f32 = 16.0;

pub const BORDER_RADIUS: f32 = 8.0;
pub const BORDER_RADIUS_SM: f32 = 4.0;

// Color constants
const CARD_BACKGROUND: Color = Color::from_oklch(Oklch::deg(0.17, 0.006, 285.885, 0.6));
const SCROLLBAR_THUMB_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.6));
const SCROLLBAR_TRACK_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.2));

const OPAQUE_CARD_BACKGROUND: Color = Color::from_oklch(Oklch::deg(0.17, 0.006, 285.885, 1.0));

const TEXT_MUTED: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.6,
};
const TEXT_COLOR: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.9,
};
const TEXT_ON_LIGHT_COLOR: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.9,
};
const BORDER_COLOR: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.1,
};
const DANGER_COLOR: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.2,
    a: 0.6,
};
const SUCCESS_COLOR: Color = Color {
    r: 0.2,
    g: 0.8,
    b: 0.2,
    a: 0.6,
};
const PRIMARY_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.6));
const SELECTION_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.3));
const SELECTION_HOVER_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.4));

const SHADOW_XS: DropShadow = DropShadow {
    offset_y: 1.0,
    blur_radius: 2.0,
    color: Color::from_hex(0x0000000D),
    ..DropShadow::default()
};

const SHADOW_SM: DropShadow = DropShadow {
    offset_y: 1.0,
    blur_radius: 3.0,
    color: Color::from_hex(0x0000001A),
    ..DropShadow::default()
};

const SHADOW_XL: DropShadow = DropShadow {
    offset_y: 20.0,
    blur_radius: 25.0,
    spread_radius: -5.0,
    color: Color::from_hex(0x0000008A),
    ..DropShadow::default()
};

const TEXT_SHADOW_4PX: TextShadow = TextShadow {
    offset_x: 0.0,
    offset_y: 0.0,
    blur_radius: 4.0,
    color: Color::BLACK,
};

const TEXT_SHADOW_2PX: TextShadow = TextShadow {
    offset_x: 0.0,
    offset_y: 0.0,
    blur_radius: 2.0,
    color: Color::BLACK,
};

// Helper function to conditionally apply text shadow to Text widgets
fn maybe_text_shadow(text: Text, enabled: bool) -> Text {
    if enabled {
        // text.with_text_shadows(vec![TEXT_SHADOW_4PX, TEXT_SHADOW_2PX])
        text.with_text_shadows(vec![
            TextShadow {
                offset_x: -1.0,
                offset_y: -1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
            TextShadow {
                offset_x: 1.0,
                offset_y: 1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
            TextShadow {
                offset_x: -1.0,
                offset_y: 1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
            TextShadow {
                offset_x: 1.0,
                offset_y: -1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
        ])
    } else {
        text
    }
}

// fn maybe_text_shadow(text: Text, enabled: bool) -> Text {
//     if enabled {
//         text.with_text_shadows(vec![
//             TextShadow {
//                 color: Color::WHITE.scale_alpha(0.5),
//                 ..TEXT_SHADOW_4PX
//             },
//             TextShadow {
//                 color: Color::WHITE.scale_alpha(0.5),
//                 ..TEXT_SHADOW_2PX
//             },
//         ])
//     } else {
//         text
//     }
// }

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
pub struct Store {
    json_export: Option<FileContainer>,
    export_out_of_date: bool,
    connection_stats: StatsStore,
    export_stats: ExportStats,

    log_level: LevelFilter,
    settings: Settings,
    update_state: Option<UpdateState>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub enum ImageFit {
    /// Stretch to fill the container (default)
    #[default]
    Fill,
    /// Scale to fit inside the container while maintaining aspect ratio
    Contain,
    /// Scale to cover the container while maintaining aspect ratio (may crop)
    Cover,
    /// Like contain but never scale up beyond intrinsic size
    ScaleDown,
    /// Display at intrinsic size with no scaling
    None,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    background_image: String,
    image_fit: ImageFit,
    background_opacity: f32,
    text_shadow_enabled: bool,
    always_update: bool,
    minimize_to_tray_on_close: bool,
    minimize_to_tray_on_minimize: bool,
    run_on_start: bool,
    start_minimized: bool,
    ws_port: u16
}

impl Default for Store {
    fn default() -> Self {
        Self {
            json_export: None,
            export_out_of_date: false,
            connection_stats: StatsStore::default(),
            export_stats: ExportStats::default(),
            log_level: LevelFilter::INFO, // TODO: This is not right
            settings: Settings::default(),
            update_state: None,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_image: "".to_string(),
            image_fit: ImageFit::Cover,
            background_opacity: 0.12,
            text_shadow_enabled: false,
            always_update: false,
            minimize_to_tray_on_close: false,
            minimize_to_tray_on_minimize: false,
            run_on_start: false,
            start_minimized: false,
            ws_port: 23313
        }
    }
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
    settings_open: bool,
    opacity_slider_dragging: bool,
    ws_port_sender: Option<Sender<u16>>,
}

impl RootState {
    pub fn with_port_sender(self: Self, port_sender: Sender<u16>) -> Self {
        Self {
            ws_port_sender: Some(port_sender),
            ..self
        }
    }
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
                    .with_color(Color::WHITE)
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

#[derive(Default, Debug)]
pub struct ActiveScreen {
    // Active screen specific state
}

fn stat_line(label: &'static str, value: usize, text_shadow_enabled: bool) -> Element<RootMessage> {
    column![
        maybe_text_shadow(Text::new(label).with_font_size(16.0).with_color(TEXT_MUTED), text_shadow_enabled),
        maybe_text_shadow(
            Text::new(value.to_string())
                .with_font_size(24.0)
                .with_assisted_id(combine_id(w_id!(), label)),
            text_shadow_enabled
        )
    ]
    .with_child_gap(SPACE_MD)
    .with_cross_align_items(Alignment::Center)
    .with_width(Sizing::grow())
}

fn refresh_icon<M>() -> Element<M> {
    SvgPath::new(
        svg![svg_path!(
            "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8 M21 3v5h-5 M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16 M8 16H3v5"
        )],
        ViewBox::new(24.0, 24.0),
    )
    .with_size(32.0, 32.0)
    .with_stroke(Color::WHITE)
    .with_stroke_width(2.0)
    .with_stroke_cap(StrokeLineCap::Round)
    .with_stroke_join(StrokeLineJoin::Round)
    .as_element(w_id!())
    .with_padding(PAD_MD)
}

fn x_icon<M>() -> Element<M> {
    SvgPath::new(svg![svg_path!("M18 6 6 18"), svg_path!("m6 6 12 12"),], ViewBox::new(24.0, 24.0))
        .with_size(16.0, 16.0)
        .with_stroke(ColorChoice::CurrentColor)
        .with_stroke_width(2.0)
        .with_stroke_cap(StrokeLineCap::Round)
        .with_stroke_join(StrokeLineJoin::Round)
        .as_element(w_id!())
        .with_padding(PAD_MD)
}

fn cog_icon<M>() -> Element<M> {
    SvgPath::new(
        svg![
            svg_path!("M11 10.27 7 3.34"),
            svg_path!("m11 13.73-4 6.93"),
            svg_path!("M12 22v-2"),
            svg_path!("M12 2v2"),
            svg_path!("M14 12h8"),
            svg_path!("m17 20.66-1-1.73"),
            svg_path!("m17 3.34-1 1.73"),
            svg_path!("M2 12h2"),
            svg_path!("m20.66 17-1.73-1"),
            svg_path!("m20.66 7-1.73 1"),
            svg_path!("m3.34 17 1.73-1"),
            svg_path!("m3.34 7 1.73 1"),
            SvgPathCommands::Circle {
                cx: 12.0,
                cy: 12.0,
                r: 2.0
            },
            SvgPathCommands::Circle {
                cx: 12.0,
                cy: 12.0,
                r: 8.0
            },
        ],
        ViewBox::new(24.0, 24.0),
    )
    .with_size(32.0, 32.0)
    .with_stroke(ColorChoice::CurrentColor)
    .with_stroke_width(2.0)
    .with_stroke_cap(StrokeLineCap::Round)
    .with_stroke_join(StrokeLineJoin::Round)
    .as_element(w_id!())
    .with_padding(PAD_MD)
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
        let text_shadow_enabled = store.settings.text_shadow_enabled;

        let stats_display = row![
            stat_line("Relics", store.export_stats.relics, text_shadow_enabled),
            stat_line("Characters", store.export_stats.characters, text_shadow_enabled),
            stat_line("Light Cones", store.export_stats.light_cones, text_shadow_enabled),
            stat_line("Materials", store.export_stats.materials, text_shadow_enabled),
        ]
        .with_width(Sizing::grow())
        .with_child_gap(SPACE_LG);

        let refresh_button = Button::new()
            .with_bg_color(SUCCESS_COLOR)
            .with_border_radius(BORDER_RADIUS)
            .with_drop_shadow(SHADOW_SM)
            .with_click_handler(move |_, shell| {
                shell.publish(RootMessage::RefreshExport);
            })
            .as_element(
                w_id!(),
                refresh_icon(),
            )
            .with_backdrop_filter(BackdropFilter::blur(10.0))
            .with_snap(true);

        let download_section = download_view(store.json_export.as_ref(), store.export_out_of_date, hook).with_drop_shadow(SHADOW_SM);

        let action_bar = row![refresh_button, download_section]
            .with_child_gap(SPACE_LG)
            .with_axis_align_content(Alignment::Center)
            .with_padding(BoxAmount::all(PAD_MD));

        column![
            maybe_text_shadow(
                Text::new("Connected!")
                    .with_font_size(24.0)
                    .with_color(TEXT_COLOR)
                    .with_paragraph_alignment(ParagraphAlignment::Center),
                text_shadow_enabled
            )
            .as_element()
            .with_padding(BoxAmount::all(PAD_MD)),
            stats_display,
            Rule::horizontal()
                .with_color(BORDER_COLOR)
                .as_element(w_id!())
                .with_padding(BoxAmount::vertical(PAD_LG)),
            action_bar,
        ]
        .with_child_gap(SPACE_LG)
        .with_cross_align_items(Alignment::Center)
        .with_padding(BoxAmount::all(PAD_LG * 2.0))
        .with_border_radius(BorderRadius::all(BORDER_RADIUS))
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
    SendWSPort(u16),
    NotifyInvalidWSPort(String),
    WSPortChanged(u16),
    WSClientCountChanged(usize),
    CheckConnection(Instant),
    ActiveScreen(ActiveMessage),
    WaitingScreen(WaitingMessage),
    RefreshExport,
    TriggerRender,
    LogLevelChanged(LevelFilter),
    ExportLog,
    ToggleMenu,
    BackgroundImageSelected(Option<PathBuf>),
    RemoveBackgroundImage,
    ImageFitChanged(ImageFit),
    OpacityChanged(f32),
    OpacitySliderDrag(bool),
    TextShadowToggled(bool),
    AlwaysUpdateToggled(bool),
    MinimizeToTrayOnCloseToggled(bool),
    MinimizeToTrayOnMinimizeToggled(bool),
    RunOnStartToggled(bool),
    StartminimizedToggled(bool),
    HideWindow,
    ShowWindow,
    ContextMenuShow,
    ContextMenuMinimize,
    ContextMenuQuit,
    ContextMenuCancelled,
    LoadSettings(PathBuf),
    ActivateSettings(Settings),
    SaveSettings,
    // Update messages
    Update(UpdateMessage),
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
            Self::ProcessCapture(path) => Task::future(async move {
                use reliquary::network::GameSniffer;
                use reliquary_archiver::export::{
                    database::{get_database, Database},
                    Exporter,
                };

                use crate::capture_from_pcap;

                tokio::task::spawn_blocking(move || {
                    let sniffer = GameSniffer::new().set_initial_keys(get_database().keys.clone());
                    let exporter = OptimizerExporter::new();

                    capture_from_pcap(exporter, sniffer, path)
                })
                .await
                .expect("Failed to process pcap")
            })
            .and_then(|e| Task::done(RootMessage::NewExport(e))),
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
        .as_element(
            w_id!(),
            Svg::new(include_str!("../../assets/github.svg"))
                .with_size(32.0, 32.0)
                .with_recolor(Color::WHITE)
                .as_element(w_id!()),
        )
        .with_padding(PAD_MD)
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
        .as_element(
            w_id!(),
            Svg::new(include_str!("../../assets/discord.svg"))
                .with_size(32.0, 32.0)
                .with_recolor(Color::WHITE)
                .as_element(w_id!()),
        )
        .with_padding(PAD_MD)
}

fn short_size(size: usize) -> String {
    let size_f = size as f64;
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.2} KB", size_f / 1024.0)
    } else {
        format!("{:.2} MB", size_f / 1024.0 / 1024.0)
    }
}

struct InvalidateOnBoundsChanged<Message, E: Fn(&raxis::widgets::Event, &mut raxis::Shell<Message>) -> Option<Task<Message>>> {
    _marker: std::marker::PhantomData<Message>,
    event_listener: E,
}
impl<Message, E: Fn(&raxis::widgets::Event, &mut raxis::Shell<Message>) -> Option<Task<Message>>> std::fmt::Debug
    for InvalidateOnBoundsChanged<Message, E>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvalidateOnBoundsChanged").finish()
    }
}

struct InvalidateOnBoundsChangedState {
    prev_bounds: raxis::widgets::Bounds,
}
impl<Message, E: Fn(&raxis::widgets::Event, &mut raxis::Shell<Message>) -> Option<Task<Message>>> Widget<Message>
    for InvalidateOnBoundsChanged<Message, E>
{
    fn state(&self, arenas: &raxis::layout::UIArenas, device_resources: &raxis::runtime::DeviceResources) -> raxis::widgets::State {
        Some(Box::new(InvalidateOnBoundsChangedState {
            prev_bounds: raxis::widgets::Bounds::default(),
        }))
    }

    fn paint(
        &mut self,
        arenas: &raxis::layout::UIArenas,
        instance: &mut raxis::widgets::Instance,
        shell: &mut raxis::Shell<Message>,
        recorder: &mut raxis::gfx::command_recorder::CommandRecorder,
        style: raxis::layout::model::ElementStyle,
        bounds: raxis::widgets::Bounds,
        now: Instant,
    ) {
        // Nothing to do
    }

    fn update(
        &mut self,
        arenas: &mut raxis::layout::UIArenas,
        instance: &mut raxis::widgets::Instance,
        hwnd: windows::Win32::Foundation::HWND,
        shell: &mut raxis::Shell<Message>,
        event: &raxis::widgets::Event,
        bounds: raxis::widgets::Bounds,
    ) {
        if matches!(event, raxis::widgets::Event::Redraw { .. }) {
            let state = raxis::with_state!(mut instance as InvalidateOnBoundsChangedState);
            if state.prev_bounds != bounds {
                shell.request_redraw(hwnd, raxis::RedrawRequest::Immediate);
                state.prev_bounds = bounds;
            }
        }

        if let Some(task) = (self.event_listener)(event, shell) {
            shell.dispatch_task(task);
        }
    }
}

fn log_view(hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let container_id = w_id!();

    let mut state = hook.instance(container_id);
    let show_more = state.use_hook(|| Rc::new(RefCell::new(HashSet::<usize>::new()))).clone();
    let max_content_width = state.use_hook(|| Rc::new(Cell::new(0.0f32))).clone();
    let max_line_length = state.use_hook(|| Rc::new(Cell::new(0usize))).clone();
    let prev_item_count = state.use_hook(|| Rc::new(Cell::new(0usize))).clone();

    // Selection state
    let selection_state = state.use_hook(|| Rc::new(RefCell::new(Option::<Range<usize>>::None))).clone();
    let drag_start = state.use_hook(|| Rc::new(Cell::new(Option::<usize>::None))).clone();
    let is_dragging = state.use_hook(|| Rc::new(Cell::new(false))).clone();

    let lines = LOG_BUFFER.lock().unwrap();

    let total_items = lines.len();
    if total_items != prev_item_count.replace(total_items) {
        hook.invalidate_layout();
    }

    let line_height_no_gap = 12.0;
    let gap = 0.0;
    let padding = BoxAmount::new(8.0, 16.0, 16.0, 8.0);
    let buffer_items_per_side = 2usize;

    let truncate_threshold = 3000;

    let line_height = line_height_no_gap + gap;

    let container_dims = hook.scroll_state_manager.get_container_dimensions(container_id);

    let content_dims = hook.scroll_state_manager.get_previous_content_dimensions(container_id);

    max_content_width.replace(max_content_width.get().max(content_dims.0));

    let visible_items = (container_dims.1 / line_height).ceil() as usize + buffer_items_per_side * 2;

    let ScrollPosition { x: _scroll_x, y: scroll_y } = hook.scroll_state_manager.get_scroll_position(container_id);

    let pre_scroll_items = (((scroll_y + gap - padding.top) / line_height).floor() as usize).saturating_sub(buffer_items_per_side);
    let post_scroll_items = total_items.saturating_sub(pre_scroll_items).saturating_sub(visible_items).max(0);

    Element {
        id: Some(container_id),
        direction: Direction::TopToBottom,
        width: Sizing::grow(),
        height: Sizing::fixed(150.0),
        scroll: Some(ScrollConfig {
            horizontal: Some(true),
            vertical: Some(true),
            sticky_bottom: Some(true),
            scrollbar_style: Some(ScrollbarStyle {
                thumb_color: SCROLLBAR_THUMB_COLOR,
                track_color: SCROLLBAR_TRACK_COLOR,
                size: ScrollBarSize::ThinThick(8.0, 12.0),
                ..Default::default()
            }),
            ..Default::default()
        }),
        background_color: Some(CARD_BACKGROUND),
        backdrop_filter: Some(BackdropFilter::blur(10.0)),
        border: Some(Border {
            width: 1.0,
            color: BORDER_COLOR, //Color::from(0x000000FF),
            ..Default::default()
        }),
        border_radius: Some(BorderRadius::all(BORDER_RADIUS_SM)),
        child_gap: gap,
        padding,
        content: widget(InvalidateOnBoundsChanged {
            _marker: std::marker::PhantomData,
            event_listener: {
                let selection_state = selection_state.clone();
                let is_dragging = is_dragging.clone();
                let drag_start = drag_start.clone();
                move |e, shell| match e {
                    raxis::widgets::Event::KeyDown { key, modifiers } => {
                        if matches!(key, VKey::C | VKey::X) && modifiers.ctrl {
                            if let Some(selection_range) = selection_state.borrow_mut().take() {
                                let lines = LOG_BUFFER.lock().unwrap();
                                let selected_lines = lines[selection_range.start..selection_range.end]
                                    .iter()
                                    .map(|line| line.clone())
                                    .collect::<Vec<_>>();

                                return Some(task::effect(task::Action::Clipboard(ClipboardAction::Set(
                                    selected_lines.join("\n"),
                                ))));
                            }
                        }
                        None
                    }
                    raxis::widgets::Event::MouseButtonDown {
                        x,
                        y,
                        click_count,
                        modifiers,
                    } => {
                        shell.capture_event(container_id);
                        None
                    }

                    raxis::widgets::Event::MouseButtonUp { .. } => {
                        is_dragging.set(false);
                        drag_start.set(None);
                        None
                    }
                    _ => None,
                }
            },
        }),
        children: {
            // DWrite runs into precision issues with really long text (it only uses f32)
            // So we have to calculate the width manually with a f64
            // Obviously won't work with special glyphs but what are you gonna do? /shrug
            const MONO_CHAR_WIDTH: f64 = 6.02411;

            // let mut max_line_length = max_line_length.borrow_mut();

            let mut text_children = (pre_scroll_items..(pre_scroll_items + visible_items).min(total_items))
                .map(|i| {
                    // Determine if this line is selected
                    let is_selected = if let Some(selection_range) = selection_state.borrow().as_ref() {
                        selection_range.contains(&i)
                    } else {
                        false
                    };

                    let line_element = if lines[i].len() > truncate_threshold && !show_more.borrow().contains(&i) {
                        max_line_length.replace(max_line_length.get().max(truncate_threshold));

                        Element {
                            id: Some(combine_id(w_id!(), i % visible_items)),
                            height: Sizing::fixed(line_height_no_gap),
                            background_color: if is_selected { Some(SELECTION_COLOR) } else { None },
                            children: vec![
                                Text::new(lines[i][0..truncate_threshold].to_string())
                                    .with_word_wrap(false)
                                    .with_font_family(FontIdentifier::System("Lucida Console".to_string()))
                                    .with_assisted_width((MONO_CHAR_WIDTH * truncate_threshold as f64) as f32)
                                    .with_font_size(10.0)
                                    .with_paragraph_alignment(ParagraphAlignment::Center)
                                    .as_element()
                                    .with_id(combine_id(w_id!(), i % visible_items))
                                    .with_height(Sizing::fixed(line_height_no_gap))
                                    .with_background_color(if is_selected { SELECTION_COLOR } else { Color::TRANSPARENT }),
                                Button::new()
                                    .with_click_handler({
                                        let show_more = show_more.clone();
                                        move |_, _| {
                                            show_more.borrow_mut().insert(i);
                                        }
                                    })
                                    .as_element(
                                        combine_id(w_id!(), i % visible_items),
                                        Text::new(format!("Show more ({})", short_size(lines[i].len())))
                                            .with_font_size(8.0)
                                            .with_color(TEXT_ON_LIGHT_COLOR)
                                            .with_assisted_id(combine_id(w_id!(), i % visible_items)),
                                    ),
                            ],

                            ..Default::default()
                        }
                    } else {
                        max_line_length.replace(max_line_length.get().max(lines[i].len()));

                        Text::new(lines[i].to_string())
                            .with_word_wrap(false)
                            .with_font_family(FontIdentifier::System("Lucida Console".to_string()))
                            .with_font_size(10.0)
                            .with_assisted_width((MONO_CHAR_WIDTH * lines[i].len() as f64) as f32)
                            .with_paragraph_alignment(ParagraphAlignment::Center)
                            .as_element()
                            .with_id(combine_id(w_id!(), i % visible_items))
                            .with_height(Sizing::fixed(line_height_no_gap))
                            .with_background_color(if is_selected { SELECTION_COLOR } else { Color::TRANSPARENT })
                    };

                    // Wrap the line with MouseArea for selection
                    MouseArea::new({
                        let selection_state = selection_state.clone();
                        let drag_start = drag_start.clone();
                        let is_dragging = is_dragging.clone();
                        let line_index = i;

                        move |event, shell| {
                            match event {
                                MouseAreaEvent::MouseButtonDown { modifiers, .. } => {
                                    if modifiers.ctrl {
                                        // Toggle selection for this line
                                        let mut selection = selection_state.borrow_mut();
                                        match selection.as_mut() {
                                            Some(range) => {
                                                if range.contains(&line_index) {
                                                    // Remove from selection - this is complex with ranges
                                                    // For now, just clear selection if clicking on selected line
                                                    *selection = None;
                                                } else {
                                                    // Expand selection to include this line
                                                    let new_start = range.start.min(line_index);
                                                    let new_end = range.end.max(line_index + 1);
                                                    *selection = Some(new_start..new_end);
                                                }
                                            }
                                            None => {
                                                *selection = Some(line_index..line_index + 1);
                                            }
                                        }
                                    } else if modifiers.shift {
                                        // Extend selection from existing start to this line
                                        let mut selection = selection_state.borrow_mut();
                                        if let Some(existing_range) = selection.as_ref() {
                                            let start = existing_range.start.min(line_index);
                                            let end = existing_range.end.max(line_index + 1);
                                            *selection = Some(start..end);
                                        } else {
                                            *selection = Some(line_index..line_index + 1);
                                        }
                                    } else {
                                        // Start new selection
                                        *selection_state.borrow_mut() = Some(line_index..line_index + 1);
                                        drag_start.set(Some(line_index));
                                        is_dragging.set(true);
                                    }
                                }
                                MouseAreaEvent::MouseMove { inside, .. } => {
                                    if inside && is_dragging.get() {
                                        if let Some(start_line) = drag_start.get() {
                                            let start = start_line.min(line_index);
                                            let end = start_line.max(line_index) + 1;
                                            *selection_state.borrow_mut() = Some(start..end);
                                        }
                                    }
                                }
                                MouseAreaEvent::MouseButtonUp { .. } => {
                                    is_dragging.set(false);
                                    drag_start.set(None);
                                }
                                _ => {}
                            };

                            Some(RootMessage::TriggerRender)
                        }
                    })
                    .as_element(combine_id(container_id, i % visible_items), line_element)
                })
                .collect();

            let keep_width =
                ((max_line_length.get() as f64 * MONO_CHAR_WIDTH) as f32).max(max_content_width.get() - padding.left - padding.right);

            let mut children = vec![];
            if pre_scroll_items > 0 {
                children.push(Element {
                    id: Some(w_id!()),
                    width: Sizing::fixed(keep_width),
                    height: Sizing::fixed(line_height * pre_scroll_items as f32 - gap),
                    ..Default::default()
                });
            }

            children.append(&mut text_children);

            if post_scroll_items > 0 {
                children.push(Element {
                    id: Some(w_id!()),
                    width: Sizing::fixed(keep_width),
                    height: Sizing::fixed(line_height * post_scroll_items as f32 - gap),
                    ..Default::default()
                });
            }
            children
        },
        ..Default::default()
    }
}

#[derive(Default, Clone)]
struct WebsocketConfigState {
    port_input: u16,
}

fn websocket_settings_section(state: &RootState, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let mut instance = hook.instance(w_id!());
    let config_state: Rc<RefCell<WebsocketConfigState>> = instance.use_state(|| { WebsocketConfigState {
        port_input: state.store.settings.ws_port,
    } });

    let text_input = row![
        Text::new("ws://0.0.0.0:")
            .as_element()
            .with_padding(BoxAmount::new(2.0, 2.0, 0.0, 0.0)),
        Element {
            id: Some(w_id!()),
            // 40 px wide = scrollbar
            width: Sizing::Fixed { px: 41.0 },
            height: Sizing::Fixed { px: 25.0 },
            background_color: Some(OPAQUE_CARD_BACKGROUND.deviate(0.1)),
            border_radius: Some(BorderRadius::all(8.0)),
            border: Some(Border {
                width: 1.0,
                color: OPAQUE_CARD_BACKGROUND.deviate(0.4),
                ..Default::default()
            }),
            color: Some(Color::WHITE),
            scroll: Some(ScrollConfig {
                horizontal: Some(true),
                sticky_right: Some(true),
                scrollbar_style: Some(ScrollbarStyle {
                    thumb_color: SCROLLBAR_THUMB_COLOR.lighten(0.3),
                    track_color: SCROLLBAR_TRACK_COLOR.lighten(0.3),
                    size: ScrollBarSize::ThinThick(4.0, 8.0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            children: vec![
                Element {
                    id: Some(w_id!()),
                    width: Sizing::grow(),
                    height: Sizing::grow(),
                    padding: BoxAmount::new(2.0, 4.0, 2.0, 4.0),
                    content: widget(TextInput::new()
                        .with_font_size(12.0)
                        .with_paragraph_alignment(ParagraphAlignment::Center)
                        .with_text(config_state.borrow_mut().port_input.to_string())
                        .with_text_input_handler({
                            let config_state = config_state.clone();
                            let ws_status = state.store.connection_stats.ws_status.clone();
                            move |text, shell| {
                                if let Ok(port) = text.parse::<u16>() {
                                    config_state.borrow_mut().port_input = port;
                                }
                            }
                        })
                    ),
                    wrap: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }.with_axis_align_content(Alignment::Center).with_cross_align_content(Alignment::Center),
        Text::new("/ws")
            .as_element()
            .with_padding(BoxAmount::new(2.0, 0.0 , 0.0, 2.0)),
    ];
    let header: Element<RootMessage> = Text::new("Configure Websocket port")
        .with_font_size(14.0)
        .as_element();

    let explainer_text = Text::new("Setting port to 0 will make windows assign you a port of its choosing.")
        .with_font_size(12.0)
        .with_color(TEXT_MUTED)
        .as_element()
        .with_padding(BoxAmount::top(5.0));

    let button = Button::new()
        .with_click_handler({
            let requested_port = config_state.borrow().port_input;
            let ws_status = state.store.connection_stats.ws_status.clone();
            move |_, s| {
                match ws_status {
                    WebSocketStatus::Running { port, client_count: _ } => {
                        if port == requested_port {
                            info!("Websocket server already running on requested port");
                        } else {
                            s.publish(RootMessage::SendWSPort(requested_port));
                        }
                    }
                    _ => s.publish(RootMessage::SendWSPort(requested_port)),
                }
            }
        })
        .with_bg_color(PRIMARY_COLOR)
        .as_element(
            w_id!(),
            Text::new("Restart server")
                .with_color(Color::WHITE)
                .with_font_size(16.0)
                .with_font_weight(FontWeight::Medium)
                .as_element()
                .with_axis_align_self(Alignment::Center)
                .with_cross_align_self(Alignment::Center)
                .with_padding(BoxAmount::horizontal(PAD_MD))
        )
        .with_height(Sizing::grow())
        .with_border_radius(BORDER_RADIUS);

    column![
        row![
            column![
                header,
                text_input,
            ],
            spacer(),
            button
        ]
        .with_width(Sizing::grow()),
        explainer_text
    ]
    .with_width(Sizing::grow())
}

#[derive(Clone, PartialEq)]
enum SettingsModalPanel {
    Graphics,
    Update,
    Misc
}
struct SettingsModalState {
    active_panel: SettingsModalPanel
}

fn settings_modal(state: &RootState, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let mut instance = hook.instance(w_id!());

    let modal_state = instance.use_state(|| SettingsModalState {
        active_panel: SettingsModalPanel::Graphics
    });

    let opacity = use_animation(&mut instance, state.settings_open);
    let bg_opacity = use_animation(&mut instance, !state.opacity_slider_dragging);
    let opacity = opacity.interpolate(hook, 0.0, 1.0, Instant::now());
    let bg_opacity = bg_opacity.interpolate(hook, 0.0, 0.5, Instant::now());

    if !state.settings_open && opacity == 0.0 {
        return Element::default();
    }

    // Header
    let close_button = Button::new()
        .ghost()
        .with_border_radius(BorderRadius::all(BORDER_RADIUS_SM))
        .with_click_handler(|_, shell| shell.publish(RootMessage::ToggleMenu))
        .as_element(w_id!(), x_icon());

    let header_section = row![
        Text::new("Settings").with_font_size(20.0).with_color(TEXT_COLOR).as_element(),
        spacer(),
        close_button
    ]
    .with_width(Sizing::grow());

    // Background image section
    let select_image_button = Button::new()
        .with_bg_color(PRIMARY_COLOR)
        .with_border_radius(BORDER_RADIUS_SM)
        .with_click_handler(move |_, shell| {
            shell.dispatch_task(Task::future(async {
                let file = rfd::AsyncFileDialog::new()
                    .add_filter("Image files", &["jpg", "jpeg", "png", "bmp", "gif", "webp"])
                    .set_title("Select background image")
                    .pick_file()
                    .await;
                RootMessage::BackgroundImageSelected(file.map(|f| f.path().to_path_buf()))
            }));
        })
        .as_element(
            w_id!(),
            Text::new("Select Image")
                .with_font_size(12.0)
                .with_color(Color::WHITE)
                .as_element()
                .with_padding(BoxAmount::new(PAD_SM, PAD_MD, PAD_SM, PAD_MD)),
        );

    let remove_image_button = Button::new()
        .with_border_radius(BorderRadius::all(BORDER_RADIUS_SM))
        .with_bg_color(Color::from_rgba(220.0 / 255.0, 38.0 / 255.0, 38.0 / 255.0, 1.0))
        .with_click_handler(|_, shell| shell.publish(RootMessage::RemoveBackgroundImage))
        .as_element(
            w_id!(),
            Text::new("✕")
                .with_font_size(12.0)
                .with_color(Color::WHITE)
                .as_element()
                .with_padding(BoxAmount::new(PAD_SM, PAD_MD, PAD_SM, PAD_MD)),
        );

    let mut bg_image_row = row![
        Text::new("Background Image")
            .with_font_size(14.0)
            .with_color(TEXT_COLOR)
            .as_element(),
        spacer(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow())
    .with_cross_align_items(Alignment::Center);

    // Add remove button if background image is present
    if !state.store.settings.background_image.is_empty() {
        bg_image_row.push_child(remove_image_button);
    }
    bg_image_row.push_child(select_image_button);

    let bg_image_section = column![
        bg_image_row,
        Text::new(format!(
            "Current: {}",
            if state.store.settings.background_image.is_empty() {
                "None"
            } else {
                state.store.settings.background_image.as_str()
            }
        ))
        .with_font_size(12.0)
        .with_color(TEXT_MUTED)
        .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Image fit mode section
    let mut fit_mode_toggles = togglegroup(
        w_id!(),
        vec![
            ToggleOption::new(ImageFit::Fill, "Fill"),
            ToggleOption::new(ImageFit::Contain, "Contain"),
            ToggleOption::new(ImageFit::Cover, "Cover"),
            ToggleOption::new(ImageFit::ScaleDown, "Scale Down"),
            ToggleOption::new(ImageFit::None, "None"),
        ],
        &state.store.settings.image_fit,
        |fit| Some(RootMessage::ImageFitChanged(fit)),
        None
    )
    .with_width(Sizing::grow());

    fit_mode_toggles.children = fit_mode_toggles
        .children
        .into_iter()
        .map(|mut child| child.with_width(Sizing::grow()))
        .collect();

    let fit_mode_section = column![
        Text::new("Image Fit Mode").with_font_size(14.0).with_color(TEXT_COLOR).as_element(),
        fit_mode_toggles,
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Opacity slider section
    let opacity_slider = Slider::new(0.0, 1.0, state.store.settings.background_opacity)
        .with_step(0.01)
        .with_track_height(6.0)
        .with_thumb_size(18.0)
        .with_track_color(CARD_BACKGROUND.deviate(0.2))
        .with_filled_track_color(PRIMARY_COLOR)
        .with_thumb_color(Color::WHITE)
        .with_thumb_border_color(PRIMARY_COLOR)
        .with_value_change_handler(|value, _, shell| {
            shell.publish(RootMessage::OpacityChanged(value));
        })
        .with_drag_handler(|is_dragging, _, shell| {
            shell.publish(RootMessage::OpacitySliderDrag(is_dragging));
        })
        .as_element(w_id!())
        .with_width(Sizing::grow());

    let opacity_section = column![
        row![
            Text::new("Background Opacity")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            Text::new(format!("{:.0}%", state.store.settings.background_opacity * 100.0))
                .with_font_size(12.0)
                .with_color(TEXT_MUTED)
                .as_element(),
        ]
        .with_width(Sizing::grow())
        .with_child_gap(SPACE_SM)
        .with_cross_align_items(Alignment::Center),
        opacity_slider,
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Text shadow section
    let text_shadow_toggle = Toggle::new(state.store.settings.text_shadow_enabled)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::TextShadowToggled(enabled));
        })
        .as_element(w_id!());

    let text_shadow_section = column![
        row![
            Text::new("Text Shadow").with_font_size(14.0).with_color(TEXT_COLOR).as_element(),
            spacer(),
            text_shadow_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Add a text-shadow to text for better readability")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let update_check_button = Button::new()
        .with_click_handler(|_, s| {
            s.publish(RootMessage::Update(UpdateMessage::PerformCheck));
        })
        .with_bg_color(PRIMARY_COLOR)
        .as_element(
            w_id!(),
            Text::new("Check for updates")
                .with_color(Color::WHITE)
                .with_font_size(14.0)
                .with_font_weight(FontWeight::Medium)
                .as_element()
                .with_axis_align_self(Alignment::Center)
                .with_cross_align_self(Alignment::Center)
                .with_padding(BoxAmount::all(8.0))
        )
        .with_height(Sizing::grow())
        .with_border_radius(BORDER_RADIUS);

    let update_unprompted_toggle = Toggle::new(state.store.settings.always_update)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::AlwaysUpdateToggled(enabled));
        })
        .as_element(w_id!());

    let update_unprompted_section = column![
        row![
            Text::new("Update Automatically")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            update_unprompted_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("When an update is available, update automatically without waiting for confirmation")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());;

    // Minimize to tray on close section
    let minimize_on_close_toggle = Toggle::new(state.store.settings.minimize_to_tray_on_close)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::MinimizeToTrayOnCloseToggled(enabled));
        })
        .as_element(w_id!());

    let minimize_on_close_section = column![
        row![
            Text::new("Minimize to Tray on Close")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            minimize_on_close_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Hide to system tray instead of closing when clicking the X button")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    // Minimize to tray on minimize section
    let minimize_on_minimize_toggle = Toggle::new(state.store.settings.minimize_to_tray_on_minimize)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::MinimizeToTrayOnMinimizeToggled(enabled));
        })
        .as_element(w_id!());

    let minimize_on_minimize_section = column![
        row![
            Text::new("Minimize to Tray on Minimize")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            minimize_on_minimize_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Hide to system tray instead of taskbar when minimizing")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let run_on_start_toggle = Toggle::new(state.store.settings.run_on_start)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::RunOnStartToggled(enabled));
        })
        .as_element(w_id!());

    let run_on_start_section = column![
        row![
            Text::new("Run on startup")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            run_on_start_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("Run automatically when your computer starts")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let start_minimized_toggle = Toggle::new(state.store.settings.start_minimized)
        .with_track_colors(CARD_BACKGROUND.deviate(0.2), PRIMARY_COLOR)
        .with_toggle_handler(|enabled, _, shell| {
            shell.publish(RootMessage::StartminimizedToggled(enabled));
        })
        .as_element(w_id!());

    let start_minimized_section = column![
        row![
            Text::new("Start minimized")
                .with_font_size(14.0)
                .with_color(TEXT_COLOR)
                .as_element(),
            spacer(),
            start_minimized_toggle
        ]
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::Center),
        Text::new("The app will launch already minimized to the taskbar/system tray")
            .with_font_size(11.0)
            .with_color(TEXT_MUTED)
            .as_element(),
    ]
    .with_child_gap(SPACE_SM)
    .with_width(Sizing::grow());

    let graphics_settings_content = column![
        bg_image_section,
        fit_mode_section,
        opacity_section,
        text_shadow_section
    ];

    let update_settings_content = column![
        update_unprompted_section,
        update_check_button
    ];

    let misc_settings_content = column![
        websocket_settings_section(state, hook),
        horizontal_rule(w_id!()),
        minimize_on_close_section,
        minimize_on_minimize_section,
        run_on_start_section,
        start_minimized_section
    ];

    let modal_state_clone = modal_state.clone();

    let mut panel_toggles = togglegroup(
        w_id!(),
        vec![
            ToggleOption::new(SettingsModalPanel::Graphics, "Graphics"),
            ToggleOption::new(SettingsModalPanel::Update, "Update"),
            ToggleOption::new(SettingsModalPanel::Misc, "Misc"),
        ],
        &modal_state_clone.borrow().active_panel,
        move |e| {
            modal_state.borrow_mut().active_panel = e;
            None
        },
        Some(ToggleGroupConfig {
            text_size: 20.0,
            ..Default::default()
        })
    ).with_width(Sizing::grow());

    panel_toggles.children = panel_toggles
        .children
        .into_iter()
        .map(|mut child| child.with_width(Sizing::grow()))
        .collect();

    let settings_content = column![
        header_section,
        panel_toggles,
        horizontal_rule(w_id!()),
        match modal_state_clone.borrow().active_panel {
            SettingsModalPanel::Graphics => graphics_settings_content,
            SettingsModalPanel::Update => update_settings_content,
            SettingsModalPanel::Misc => misc_settings_content
        }.with_child_gap(SPACE_LG).with_width(Sizing::grow())
    ]
    .with_child_gap(SPACE_LG)
    .with_width(Sizing::fixed(400.0))
    .with_padding(BoxAmount::all(PAD_LG));

    modal_backdrop(
        w_id!(),
        state.settings_open,
        hook,
        Some(ModalConfig {
            backdrop_visible: !state.opacity_slider_dragging,
            modal_position: ModalPosition {
                top: Some(40.0),
                ..Default::default()
            },
            ..Default::default()
        }),
        Some(RootMessage::ToggleMenu),
        settings_content,
    )
}

// Main view function
pub fn view(state: &RootState, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let text_shadow_enabled = state.store.settings.text_shadow_enabled;

    let help_text = maybe_text_shadow(
        Text::new("have questions or issues?")
            .with_font_size(16.0)
            .italic()
            .with_color(TEXT_MUTED),
        text_shadow_enabled,
    );

    let social_buttons = row![github_button(), discord_button()].with_child_gap(SPACE_MD);

    let header = column![social_buttons, help_text]
        .with_child_gap(SPACE_SM)
        .with_padding(BoxAmount::all(PAD_LG).apply(|p| p.bottom = PAD_SM))
        .with_width(Sizing::grow())
        .with_height(Sizing::Grow { min: 0.0, max: 182.0 }); // Size of footer + log view; this ensures the main content is as centered as possible

    let menu_button = Button::new()
        .ghost()
        .with_border_radius(BORDER_RADIUS)
        .with_click_handler(|_, s| s.publish(RootMessage::ToggleMenu))
        .as_element(w_id!(), cog_icon());

    let header = row![
        header,
        spacer(),
        column![
            titlebar_controls(hook),
            container(menu_button).with_padding(BoxAmount::all(PAD_MD)),
        ].with_cross_align_items(Alignment::End)
    ]
    .with_width(Sizing::grow());

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

    let ws_status = maybe_text_shadow(
        Text::new(ws_status_text)
            .with_font_size(12.0)
            .with_color(match &state.store.connection_stats.ws_status {
                WebSocketStatus::Failed { .. } => DANGER_COLOR,
                _ => TEXT_MUTED,
            }),
        text_shadow_enabled,
    )
    .as_element()
    .with_id(w_id!());

    let content = match &state.screen {
        Screen::Waiting(screen) => screen.view(&state.store, hook),
        Screen::Active(screen) => screen.view(&state.store, hook),
    };

    let connection_status_text = if state.store.connection_stats.connected {
        format!(
            "connected, {}/{} pkts/cmds received",
            state.store.connection_stats.packets_received, state.store.connection_stats.commands_received
        )
    } else {
        "disconnected".to_string()
    };

    let connection_status = maybe_text_shadow(
        Text::new(connection_status_text)
            .with_font_size(12.0)
            .with_color(if state.store.connection_stats.connected {
                SUCCESS_COLOR
            } else {
                DANGER_COLOR
            }),
        text_shadow_enabled,
    );

    let level_group = togglegroup(
        w_id!(),
        vec![
            ToggleOption::new(LevelFilter::INFO, "Info"),
            ToggleOption::new(LevelFilter::DEBUG, "Debug"),
            ToggleOption::new(LevelFilter::TRACE, "Trace"),
            // ToggleOption::new(LevelFilter::WARN, "Warn"),
            // ToggleOption::new(LevelFilter::ERROR, "Error"),
        ],
        // &VEC_LAYER_HANDLE
        //     .lock()
        //     .unwrap()
        //     .as_ref()
        //     .map(|h| h.with_current(|f| f.max_level_hint()).unwrap_or_default())
        //     .flatten()
        //     .unwrap_or(LevelFilter::INFO),
        // &LevelFilter::INFO,
        &state.store.log_level,
        |value| Some(RootMessage::LogLevelChanged(value)),
        None
    );

    let footer = column![
        row![
            level_group,
            spacer(),
            Button::new()
                .with_bg_color(CARD_BACKGROUND)
                .with_border(1.0, BORDER_COLOR)
                .with_border_radius(BORDER_RADIUS_SM)
                .with_click_handler(|_, shell| shell.publish(RootMessage::ExportLog))
                .as_element(w_id!(), Text::new("Export").with_color(TEXT_COLOR).with_font_size(10.0))
                .with_padding(PAD_SM)
        ]
        .with_width(Sizing::grow())
        .with_padding(BoxAmount::bottom(PAD_SM)),
        log_view(hook),
        row![
            ws_status,
            if matches!(state.store.update_state, Some(UpdateState::Checking)) {
                container(
                    maybe_text_shadow(
                        Text::new("Checking for updates...")
                            .with_font_size(12.0)
                            .with_color(TEXT_MUTED)
                            .with_text_alignment(TextAlignment::Center),
                        text_shadow_enabled,
                    )
                    .as_element()
                ).with_axis_align_content(Alignment::Center)
            } else {
                Element::default()
            },
            container(connection_status).with_axis_align_content(Alignment::End),
        ]
        .map_children(|e| e.with_width(Sizing::grow()))
        .with_child_gap(SPACE_MD)
        .with_width(Sizing::grow())
        .with_cross_align_items(Alignment::End)
        .with_padding(PAD_MD)
    ]
    .with_width(Sizing::grow())
    .with_height(Sizing::fit())
    .with_padding(PAD_MD);

    // let modal = Element {
    //     // background_color: Some(Color::from(0x00000033)),
    //     floating: Some(FloatingConfig {
    //         anchor: Some(Alignment2D {
    //             x: Some(Alignment::Center),
    //             y: Some(Alignment::Center),
    //         }),
    //         align: Some(Alignment2D {
    //             x: Some(Alignment::Center),
    //             y: Some(Alignment::Center),
    //         }),
    //         ..Default::default()
    //     }),
    //     width: Sizing::percent(100.0),
    //     height: Sizing::percent(100.0),
    //     ..Default::default()
    // };

    row![
        column![header, center(content).with_padding(PAD_MD), footer]
            .with_id(w_id!())
            .with_color(TEXT_COLOR)
            .with_width(Sizing::grow())
            .with_height(Sizing::grow())
            .with_child_gap(SPACE_MD)
            .with_scroll(ScrollConfig {
                vertical: Some(true),
                // Safe area padding for the window controls
                safe_area_padding: Some(BoxAmount::all(4.0).apply(|p| p.top = 34.0)),
                scrollbar_style: Some(ScrollbarStyle {
                    thumb_color: SCROLLBAR_THUMB_COLOR,
                    track_color: SCROLLBAR_TRACK_COLOR,
                    track_radius: BorderRadius::all(4.0),
                    size: ScrollBarSize::ThinThick(8.0, 12.0),
                ..Default::default()
                }),
                ..Default::default()
            }),
        settings_modal(state, hook),
        update_modal(
            state.store.update_state.as_ref(),
            hook,
            RootMessage::Update,
        )
    ]
    .with_id(w_id!())
    .with_color(TEXT_COLOR)
    .apply(|e| {
        if !state.store.settings.background_image.is_empty() {
            e.with_widget(
                Image::new(state.store.settings.background_image.clone())
                    .with_opacity(state.store.settings.background_opacity)
                    .with_fit(match state.store.settings.image_fit {
                        ImageFit::Fill => raxis::widgets::image::ImageFit::Fill,
                        ImageFit::Contain => raxis::widgets::image::ImageFit::Contain,
                        ImageFit::Cover => raxis::widgets::image::ImageFit::Cover,
                        ImageFit::ScaleDown => raxis::widgets::image::ImageFit::ScaleDown,
                        ImageFit::None => raxis::widgets::image::ImageFit::None,
                    }),
            )
        } else {
            e
        }
    })
    .with_width(Sizing::grow())
    .with_height(Sizing::grow())
}

fn get_settings_path(appdata: PathBuf) -> PathBuf {
    appdata.join("reliquary-archiver").join("settings.json")
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

    fn save_settings(state: &RootState) -> Option<Task<RootMessage>> {
        let settings = state.store.settings.clone();
        Some(
            task::get_local_app_data()
                .and_then(move |path| {
                    let settings = settings.clone();
                    Task::future(async move {
                        let path = get_settings_path(path);
                        tokio::fs::create_dir_all(path.parent().unwrap().to_owned()).await;
                        tokio::fs::write(path, serde_json::to_string(&settings).unwrap()).await;
                    })
                })
                .discard(),
        )
    };

    match message {
        RootMessage::TriggerRender => None, // Just update the view

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

        RootMessage::SendWSPort(port) => {
            if let Some(ref sender) = state.ws_port_sender {
                sender.send(port);
            };
            // modify settings but don't save yet to minimize odds of saving on a bad port
            state.store.settings.ws_port = port;
            None
        }
        // Save settings when we hear the update rather than when we request a change to avoid saving a bad port
        RootMessage::WSPortChanged(port) => {
            state.store.connection_stats.ws_status = WebSocketStatus::Running { port, client_count: 0 };
            Some(Task::done(RootMessage::SaveSettings))
        },

        RootMessage::WSClientCountChanged(client_count) => {
            if let WebSocketStatus::Running { port, client_count: old_client_count } = state.store.connection_stats.ws_status {
                state.store.connection_stats.ws_status = WebSocketStatus::Running { port, client_count };
            }
            None
        },

        RootMessage::NotifyInvalidWSPort(err) => {
            tracing::info!("Unable to start websocket server on desired port. e={}", err);
            state.store.connection_stats.ws_status = WebSocketStatus::Failed { error: err };
            None
        },

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

        RootMessage::LogLevelChanged(level) => {
            state.store.log_level = level;

            if let Some(handle) = VEC_LAYER_HANDLE.lock().unwrap().as_ref() {
                handle(level);
            }

            None
        }

        RootMessage::ExportLog => Some(
            Task::future(async move {
                if let Some(mut file) = rfd::AsyncFileDialog::new().set_file_name("log.txt").save_file().await {
                    let lines = LOG_BUFFER.lock().unwrap().join("\n");
                    file.write(lines.as_bytes()).await;
                    info!("Exported log to {}", file.path().display());
                }
            })
            .discard(),
        ),

        RootMessage::ToggleMenu => {
            state.settings_open = !state.settings_open;
            None
        }

        RootMessage::BackgroundImageSelected(path) => {
            if let Some(path) = path {
                state.store.settings.background_image = path.to_string_lossy().to_string();
                tracing::info!("Background image changed to: {}", state.store.settings.background_image);
            }
            save_settings(state)
        }

        RootMessage::RemoveBackgroundImage => {
            state.store.settings.background_image = String::new();
            tracing::info!("Background image removed");
            save_settings(state)
        }

        RootMessage::ImageFitChanged(fit) => {
            state.store.settings.image_fit = fit;
            tracing::info!("Image fit mode changed to: {:?}", fit);
            save_settings(state)
        }

        RootMessage::OpacityChanged(opacity) => {
            state.store.settings.background_opacity = opacity;
            save_settings(state)
        }

        RootMessage::OpacitySliderDrag(is_dragging) => {
            state.opacity_slider_dragging = is_dragging;
            None
        }

        RootMessage::TextShadowToggled(enabled) => {
            state.store.settings.text_shadow_enabled = enabled;
            save_settings(state)
        }

        RootMessage::AlwaysUpdateToggled(enabled) => {
            state.store.settings.always_update = enabled;
            save_settings(state)
        }

        RootMessage::MinimizeToTrayOnCloseToggled(enabled) => {
            state.store.settings.minimize_to_tray_on_close = enabled;
            save_settings(state)
        }

        RootMessage::MinimizeToTrayOnMinimizeToggled(enabled) => {
            state.store.settings.minimize_to_tray_on_minimize = enabled;
            save_settings(state)
        }

        RootMessage::RunOnStartToggled(enabled) => {
            match set_run_on_start(enabled) {
                Ok(()) => {
                    state.store.settings.run_on_start = enabled;
                },
                Err(RegistryError::KeyCreationFailed) => {
                    tracing::warn!("Unable to create registry key!");
                },
                Err(RegistryError::PathUnobtainable) => {
                    tracing::warn!("Unable to get current exe path!");
                },
                Err(RegistryError::AddFailed) => {
                    tracing::warn!("Failed to add registry key!");
                },
                Err(RegistryError::RemoveFailed) => {
                    state.store.settings.run_on_start = false;
                    tracing::warn!("Failed to remove registry key!");
                },
            }
            save_settings(state)
        },

        RootMessage::StartminimizedToggled(enabled) => {
            state.store.settings.start_minimized = enabled;
            save_settings(state)
        }

        RootMessage::HideWindow => Some(task::hide_window()),

        RootMessage::ShowWindow => Some(task::show_window()),

        RootMessage::ContextMenuShow => Some(task::show_window()),

        RootMessage::ContextMenuMinimize => Some(task::hide_window()),

        RootMessage::ContextMenuQuit => Some(task::exit_application()),

        RootMessage::ContextMenuCancelled => None,

        RootMessage::LoadSettings(path) => {
            info!("Loading settings from {}", path.display());
            if path.exists() {
                Some(Task::future(tokio::fs::read_to_string(path)).and_then(move |content| {
                    let mut settings: Settings = match serde_json::from_str::<Settings>(&content) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Failed to load settings: {}", e);
                            Settings::default()
                        }
                    };

                    let run_on_start = settings.run_on_start;
                    let test = match registry_matches_settings(run_on_start) {
                        // settings are not guaranteed to match the registry
                        // e.g. user moves the exe after enabling/disabling run on start
                        // in case of mismatch, update the settings and delete registry key if appropriate
                        Ok(false) => settings.run_on_start = !run_on_start,
                        Ok(true) => {},
                        _ => {},
                    };
                    // want to avoid having the app briefly flash up if set to start minimized 
                    // the app will therefore always start minimized and update display mode here as necessary
                    let display_task = if settings.start_minimized {
                        // TODO: Does this make sense or should it also consider onClose preference
                        if settings.minimize_to_tray_on_minimize {
                            Task::none()
                        } else {
                            task::minimize_window()
                        }
                    } else {
                        task::show_window()
                    };
                    Task::batch(vec![
                        display_task,
                        Task::done(RootMessage::SendWSPort(settings.ws_port)),
                        Task::done(RootMessage::ActivateSettings(settings))
                    ])
                }))
            } else {
                None
            }
        }

        RootMessage::SaveSettings => save_settings(state),

        RootMessage::ActivateSettings(settings) => {
            state.store.settings = settings;
            None
        }

        // Update messages
        RootMessage::Update(msg) => {
            match update::handle_message(msg, &mut state.store.update_state, state.store.settings.always_update) {
                update::HandleResult::None => None,
                update::HandleResult::Task(t) => Some(t.map(RootMessage::Update)),
                update::HandleResult::ExitForRestart => return Some(task::exit_application()),
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
    let (port_tx, port_rx) = watch::channel::<u16>(0);
    let state = RootState::default().with_port_sender(port_tx);
    let exporter = state.exporter.clone();

    let app = raxis::Application::new(state, view, update, move |state| {

        Some(Task::batch(vec![
            task::get_local_app_data().and_then(|path| {
                Task::done(RootMessage::LoadSettings(get_settings_path(path)))
            }),
            // technically calling PerformCheck here is a data race (should be .chain() to RootMessage::ActivateSettings)
            // however its a case of loading+parsing ~300 bytes of json vs a round trip web request
            Task::done(RootMessage::Update(update::UpdateMessage::PerformCheck)),
            Task::run(archiver_worker(exporter.clone()), |e| RootMessage::WorkerEvent(e)),
            Task::future(start_websocket_server(
                PortSource::Dynamic(WatchStream::from_changes(port_rx.clone())),
                exporter.clone(),
                ))
                .then(|e| match e {
                    Err(e) => Task::done(RootMessage::WSStatus(WebSocketStatus::Failed { error: e })),
                    Ok((port_stream, client_count_stream)) => {
                        Task::done(RootMessage::WSStatus(WebSocketStatus::Running { port: 0, client_count: 0 }))
                            .chain(Task::batch(vec![
                                Task::stream(client_count_stream).map(move |client_count| {
                                    RootMessage::WSClientCountChanged(client_count)
                                }),
                                Task::stream(port_stream).map(move |port| {
                                    match port {
                                        Ok(port) => RootMessage::WSPortChanged(port),
                                        Err(e) => RootMessage::NotifyInvalidWSPort(e)
                                    }
                                })
                            ]))
                    },
                }),
            Task::stream(stream! {
                loop {
                    LOG_NOTIFY.notified().await;
                    yield RootMessage::TriggerRender;
                }
            }),
        ])
    )})
    .with_title("Reliquary Archiver")
    .with_icons(Some(1))
    .with_tray_icon(TrayIconConfig {
        icon_resource: Some(1),
        tooltip: Some("Reliquary Archiver".to_string()),
    })
    .with_tray_event_handler(|state, event| match event {
        TrayEvent::LeftClick | TrayEvent::LeftDoubleClick => Some(task::get_window_mode().then({
            // TODO: Does this make sense or should it also consider onClose preference (re)
            let minimize_to_tray = state.store.settings.minimize_to_tray_on_minimize;
            move |mode| {
                match mode {
                    WindowMode::Hidden => task::show_window(),
                    WindowMode::Windowed => if minimize_to_tray {
                        task::hide_window()
                    } else {
                        task::minimize_window()
                    },
                    WindowMode::Minimized => task::restore_window()
                }
            }
        })),
        TrayEvent::RightClick => Some(task::get_window_mode().then(|mode| {
            task::show_context_menu(
                vec![
                    if mode == WindowMode::Hidden {
                        ContextMenuItem::new(RootMessage::ContextMenuShow, "Show Window")
                    } else {
                        ContextMenuItem::new(RootMessage::ContextMenuMinimize, "Minimize to Tray")
                    },
                    ContextMenuItem::separator(),
                    ContextMenuItem::new(RootMessage::ContextMenuQuit, "Quit"),
                ],
                RootMessage::ContextMenuCancelled,
            )
        })),
    })
    .with_syscommand_handler(|state, command| match command {
        SystemCommand::Close => {
            if state.store.settings.minimize_to_tray_on_close {
                return SystemCommandResponse::PreventWith(RootMessage::HideWindow);
            }
            SystemCommandResponse::Allow
        }
        SystemCommand::Minimize => {
            if state.store.settings.minimize_to_tray_on_minimize {
                return SystemCommandResponse::PreventWith(RootMessage::HideWindow);
            }
            SystemCommandResponse::Allow
        }
        SystemCommand::Maximize | SystemCommand::Restore => SystemCommandResponse::Allow,
        _ => SystemCommandResponse::Allow,
    })
    .replace_titlebar()
    .with_backdrop(Backdrop::MicaAlt)
    .with_window_size(960, 760)
    .with_initial_display(InitialDisplay::Hidden);

    app.run()?;

    Ok(())
}