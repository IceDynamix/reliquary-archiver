use std::cell::{Cell, RefCell};
use std::collections::HashSet;
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
use raxis::layout::model::{Alignment2D, Color, DropShadow, FloatingConfig, ScrollConfig, StrokeLineCap, StrokeLineJoin};
use raxis::runtime::font_manager::FontIdentifier;
use raxis::runtime::scroll::ScrollPosition;
use raxis::runtime::Backdrop;
use raxis::svg_path;
use raxis::util::str::StableString;
use raxis::util::unique::combine_id;
use raxis::widgets::rule::{horizontal_rule, Rule};
use raxis::widgets::svg::Svg;
use raxis::widgets::Widget;
use raxis::{
    column,
    layout::{
        helpers::{center, container, row, ElementAlignmentExt},
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
        widget,
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
use crate::{LOG_BUFFER, LOG_NOTIFY};

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
const CARD_BACKGROUND: Color = Color::from_oklch(Oklch::deg(0.17, 0.006, 285.885, 0.6));
const SCROLLBAR_THUMB_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.6));
const SCROLLBAR_TRACK_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.2));

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
            .with_height(Sizing::grow())
            .with_snap(true);

        let download_section = download_view(store.json_export.as_ref(), store.export_out_of_date, hook);

        let upload_bar = row![upload_button, download_section]
            .with_child_gap(SPACE_MD)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_padding(BoxAmount::all(PAD_MD));

        column![
            Text::new("Waiting for login...")
                .with_font_size(24.0)
                .with_paragraph_alignment(ParagraphAlignment::Center),
            Text::new("Please log into the game. If you are already in-game, you must log out and log back in.")
                .with_font_size(16.0)
                .with_color(TEXT_MUTED)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .as_element()
                .with_padding(BoxAmount::horizontal(PAD_LG)),
            Rule::horizontal()
                .with_color(BORDER_COLOR)
                .as_element(w_id!())
                .with_padding(BoxAmount::vertical(PAD_LG)),
            Text::new("Alternatively, if you have a packet capture file, you can upload it.")
                .with_font_size(16.0)
                .with_color(TEXT_MUTED)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .as_element()
                .with_padding(BoxAmount::horizontal(PAD_LG)),
            upload_bar,
        ]
        .with_child_gap(SPACE_SM)
        .with_horizontal_alignment(HorizontalAlignment::Center)
        .with_vertical_alignment(VerticalAlignment::Center)
        .with_padding(BoxAmount::all(PAD_LG * 2.0))
        .with_border_radius(BorderRadius::all(BORDER_RADIUS))
        .align_x(HorizontalAlignment::Center)
    }
}

#[derive(Default, Debug)]
pub struct ActiveScreen {
    // Active screen specific state
}

fn stat_line(label: &'static str, value: usize) -> Element<RootMessage> {
    row![
        Text::new(label).with_font_size(16.0),
        Rule::horizontal()
            .with_custom_dashes(&[5.0, 5.0], 0.0)
            .with_color(BORDER_COLOR)
            .as_element(combine_id(w_id!(), label)),
        Text::new(value.to_string()).with_font_size(16.0)
    ]
    .with_child_gap(SPACE_MD)
    .with_width(Sizing::grow())
    .align_y(VerticalAlignment::Center)
}

fn refresh_icon<M>() -> Element<M> {
    SvgPath::new(
        svg_path!(
            "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8 M21 3v5h-5 M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16 M8 16H3v5"
        ),
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
            stat_line("Relics", store.export_stats.relics),
            stat_line("Characters", store.export_stats.characters),
            stat_line("Light Cones", store.export_stats.light_cones),
            stat_line("Materials", store.export_stats.materials),
        ]
        .with_width(Sizing::grow())
        .with_child_gap(SPACE_MD);

        let refresh_button = Button::new()
            .with_bg_color(SUCCESS_COLOR)
            .with_border_radius(BORDER_RADIUS)
            .with_drop_shadow(SHADOW_SM)
            .with_click_handler(move |_, shell| {
                shell.publish(RootMessage::RefreshExport);
            })
            .as_element(
                w_id!(),
                // Text::new("Refresh Export")
                //     .with_font_size(16.0)
                //     .as_element()
                //     .with_padding(BoxAmount::all(PAD_MD)),
                refresh_icon(),
            )
            .with_snap(true);

        let download_section = download_view(store.json_export.as_ref(), store.export_out_of_date, hook).with_drop_shadow(SHADOW_SM);

        let action_bar = row![refresh_button, download_section]
            .with_child_gap(SPACE_LG)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_padding(BoxAmount::all(PAD_MD))
            .align_y(VerticalAlignment::Center);

        column![
            Text::new("Connected!")
                .with_font_size(24.0)
                .with_color(SUCCESS_COLOR)
                .with_paragraph_alignment(ParagraphAlignment::Center)
                .as_element()
                .with_padding(BoxAmount::all(PAD_MD)),
            stats_display,
            action_bar,
        ]
        .with_child_gap(SPACE_LG)
        .with_horizontal_alignment(HorizontalAlignment::Center)
        .with_vertical_alignment(VerticalAlignment::Center)
        .with_padding(BoxAmount::all(PAD_LG * 2.0))
        .with_border_radius(BorderRadius::all(BORDER_RADIUS))
        .align_x(HorizontalAlignment::Center)
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
    LogUpdate,
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

#[derive(Debug)]
struct InvalidateOnBoundsChanged;
struct InvalidateOnBoundsChangedState {
    prev_bounds: raxis::widgets::Bounds,
}
impl<Message> Widget<Message> for InvalidateOnBoundsChanged {
    fn state(&self, arenas: &raxis::layout::UIArenas, device_resources: &raxis::runtime::DeviceResources) -> raxis::widgets::State {
        Some(Box::new(InvalidateOnBoundsChangedState {
            prev_bounds: raxis::widgets::Bounds::default(),
        }))
    }

    fn paint(
        &mut self,
        arenas: &raxis::layout::UIArenas,
        instance: &mut raxis::widgets::Instance,
        shell: &raxis::Shell<Message>,
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
    }
}

fn log_view(hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let container_id = w_id!();

    let mut state = hook.instance(container_id);
    let show_more = state.use_hook(|| Rc::new(RefCell::new(HashSet::<usize>::new()))).clone();
    let max_content_width = state.use_hook(|| Rc::new(Cell::new(0.0f32))).clone();
    let max_line_length = state.use_hook(|| Rc::new(Cell::new(0usize))).clone();
    let prev_item_count = state.use_hook(|| Rc::new(Cell::new(0usize))).clone();

    let lines = LOG_BUFFER.lock().unwrap();

    let total_items = lines.len();
    if total_items != prev_item_count.replace(total_items) {
        hook.invalidate_layout();
    }

    let line_height_no_gap = 10.0;
    let gap = 2.0;
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
            scrollbar_thumb_color: Some(SCROLLBAR_THUMB_COLOR),
            scrollbar_track_color: Some(SCROLLBAR_TRACK_COLOR),
            scrollbar_size: Some(12.0),
            ..Default::default()
        }),
        background_color: Some(CARD_BACKGROUND),
        border: Some(Border {
            width: 1.0,
            color: BORDER_COLOR, //Color::from(0x000000FF),
            ..Default::default()
        }),
        child_gap: gap,
        padding,
        content: widget(InvalidateOnBoundsChanged),
        children: {
            // DWrite runs into precision issues with really long text (it only uses f32)
            // So we have to calculate the width manually with a f64
            // Obviously won't work with special glyphs but what are you gonna do? /shrug
            const MONO_CHAR_WIDTH: f64 = 6.02411;

            // let mut max_line_length = max_line_length.borrow_mut();

            let mut text_children = (pre_scroll_items..(pre_scroll_items + visible_items).min(total_items))
                .map(|i| {
                    if lines[i].len() > truncate_threshold && !show_more.borrow().contains(&i) {
                        max_line_length.replace(max_line_length.get().max(truncate_threshold));

                        Element {
                            id: Some(combine_id(w_id!(), i % visible_items)),
                            height: Sizing::fixed(line_height_no_gap),
                            children: vec![
                                Text::new(lines[i][0..truncate_threshold].to_string())
                                    .with_word_wrap(false)
                                    .with_font_family(FontIdentifier::System("Lucida Console".to_string()))
                                    .with_assisted_width((MONO_CHAR_WIDTH * truncate_threshold as f64) as f32)
                                    .with_font_size(10.0)
                                    .as_element()
                                    .with_id(combine_id(w_id!(), i % visible_items))
                                    .with_height(Sizing::fixed(line_height_no_gap)),
                                Button::new()
                                    .with_click_handler({
                                        let show_more = show_more.clone();
                                        move |_, _| {
                                            show_more.borrow_mut().insert(i);
                                        }
                                    })
                                    .as_element(
                                        combine_id(w_id!(), i % visible_items),
                                        Text::new(format!("Show more ({})", short_size(lines[i].len()))).with_font_size(8.0),
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
                            .as_element()
                            .with_id(combine_id(w_id!(), i % visible_items))
                            .with_height(Sizing::fixed(line_height_no_gap))
                    }
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

// Main view function
pub fn view(state: &RootState, hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let help_text = Text::new("have questions or issues?")
        .with_font_size(16.0)
        .italic()
        .with_color(TEXT_MUTED);

    let social_buttons = row![github_button(), discord_button()]
        .with_child_gap(SPACE_MD)
        .with_vertical_alignment(VerticalAlignment::Center);

    let header = column![social_buttons, help_text]
        .with_child_gap(SPACE_SM)
        .with_padding(BoxAmount::all(PAD_LG).apply(|p| p.bottom = PAD_SM))
        .with_height(Sizing::Grow { min: 0.0, max: 182.0 }); // Size of footer + log view; this ensures the main content is as centered as possible

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

    let connection_status = Text::new(connection_status_text)
        .with_font_size(12.0)
        .with_color(if state.store.connection_stats.connected {
            SUCCESS_COLOR
        } else {
            DANGER_COLOR
        });

    let footer = column![
        log_view(hook),
        row![
            ws_status,
            Element::default().with_width(Sizing::grow()), // spacer
            connection_status,
        ]
        .with_width(Sizing::grow())
        .with_vertical_alignment(VerticalAlignment::Bottom)
        .align_y(VerticalAlignment::Bottom)
        .with_padding(PAD_MD)
    ]
    .with_width(Sizing::grow())
    .with_height(Sizing::fit());

    let modal = Element {
        // background_color: Some(Color::from(0x00000033)),
        floating: Some(FloatingConfig {
            anchor: Some(Alignment2D {
                x: Some(HorizontalAlignment::Center),
                y: Some(VerticalAlignment::Center),
            }),
            align: Some(Alignment2D {
                x: Some(HorizontalAlignment::Center),
                y: Some(VerticalAlignment::Center),
            }),
            ..Default::default()
        }),
        width: Sizing::percent(100.0),
        height: Sizing::percent(100.0),
        ..Default::default()
    };

    column![header, center(content), footer, modal]
        .with_id(w_id!())
        .with_color(TEXT_COLOR)
        .with_width(Sizing::grow())
        .with_height(Sizing::grow())
        .with_padding(PAD_MD)
        .with_child_gap(SPACE_MD)
        .with_scroll(ScrollConfig {
            vertical: Some(true),
            // Safe area padding for the window controls
            safe_area_padding: Some(BoxAmount::all(0.0).apply(|p| p.top = 30.0)),
            scrollbar_thumb_color: Some(SCROLLBAR_THUMB_COLOR),
            scrollbar_track_color: Some(SCROLLBAR_TRACK_COLOR),
            scrollbar_size: Some(12.0),
            ..Default::default()
        })
    // .with_background_color(Color::from_hex(0xF1F5EDFF))
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
        RootMessage::LogUpdate => None, // Just update the view

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

    raxis::Application::new(state, view, update, move |_state| {
        Some(Task::batch(vec![
            Task::run(archiver_worker(exporter.clone()), |e| RootMessage::WorkerEvent(e)),
            Task::future(start_websocket_server(53313, exporter.clone()))
                .then(|e| match e {
                    Err(e) => Task::done(WebSocketStatus::Failed { error: e }),
                    Ok((port, client_count_stream)) => Task::done(WebSocketStatus::Running { port, client_count: 0 })
                        .chain(Task::stream(client_count_stream).map(move |client_count| WebSocketStatus::Running { port, client_count })),
                })
                .map(|e| RootMessage::WSStatus(e)),
            Task::stream(stream! {
                loop {
                    LOG_NOTIFY.notified().await;
                    yield RootMessage::LogUpdate;
                }
            }),
        ]))
    })
    .with_title("Reliquary Archiver")
    .replace_titlebar()
    .with_backdrop(Backdrop::MicaAlt)
    .with_window_size(960, 760)
    .run()?;

    Ok(())
}
