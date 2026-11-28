//! The desktop window.
//!
//! The window answers one question: is my account's data in, and how do I get it into the
//! optimizer? It leads with where capturing stands (waiting for a login, reading an account, or
//! ready), says what to do when something is in the way, and offers the two ways out: live sync
//! over the WebSocket server, or saving an export file.
//!
//! - [`state`]: what the window shows and how events change it, as plain data
//! - [`services`]: the work that leaves the UI thread (files, the server, updates)
//! - [`view`]: draws the state with Rune, in the colors and type of [`theme`] with [`icons`]
//! - [`worker`]: reads packets and reports what happened
//! - [`settings`]: the settings and their file
//! - [`background`]: the background image, read and scaled off the UI thread
//! - [`art`]: character and light cone art for the warp screen, downloaded and kept
//! - `run_on_start`: starting with Windows
//! - [`controller`]: a second window that puts the app into any state, for trying the views
//!
//! Every screen can be rendered without a window or a capture: `--ui --snap out.png --scene ready`
//! (see [`scenes`]).

use std::cell::RefCell;
use std::pin::pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::lock::Mutex;
use reliquary_archiver::export::Exporter;
use rune::{App, AppHandle, AppSpec, Tray, TrayEvent, TrayIcon, TrayItem, TraySpec, Tx};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tracing::info;

use crate::accounts::MultiAccountManager;
use crate::capture::Tap;
use crate::warps::item;
use crate::warps::service::{Command as WarpCommand, WarpService};
use crate::websocket::{PortCommand, start_dynamic_websocket_server};

mod art;
mod background;
mod controller;
mod icons;
#[cfg(windows)]
mod run_on_start;
mod scenes;
mod services;
mod settings;
mod state;
mod theme;
mod view;
mod worker;

#[cfg(test)]
mod tests;

use services::Services;
use state::{Inventory, Panel, Ui};
use worker::WorkerEvent;

/// An sRGB-encoded channel, 0 to 1, in linear light
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

const TITLE: &str = "Reliquary Archiver";

thread_local! {
    /// The app's window, to save where it is when it closes. Rune runs every window hook on the UI
    /// thread, the thread this lives on
    static WINDOW: RefCell<Option<rune::WindowHandle>> = const { RefCell::new(None) };
}

pub struct Options {
    /// Capturing needs administrator rights the app doesn't have, so it won't start
    pub needs_admin: bool,
    /// Updates were turned off from the command line
    pub no_update: bool,
    /// Flags for Rune's own tooling, such as `--snap` for headless snapshots
    pub ui_args: Vec<String>,
    /// Open the test controller instead of capturing (see [`controller`])
    pub test_controller: bool,
}

/// Shows the window and captures from `tap` until the app quits
pub fn run(tap: Tap, options: Options) {
    let spec = AppSpec {
        min_size: Some((420.0, 620.0)),
        scenes: scenes::SCENES,
        poll: Some(|s: &mut Ui, _, tx| {
            // Each time the window comes on screen or goes off it, poll runs. Snapshots draw
            // without a window on screen, with everything held
            if rune::tooling::headless_run().is_none() {
                s.window_visible(tx.window_visible(), tx);
            }
            let expired = s.expire(Instant::now());
            let art = s.art_ready();
            let background = s.background.ready(&s.settings.background_image);
            // Art and backgrounds arriving come in on their transitions. Rune lands a change no
            // input caused at once unless it's told otherwise
            if art || background {
                tx.animated();
            }
            background || expired || art
        }),
        setup: Some(|s: &mut Ui, waker| {
            // Snapshots draw a single frame, the art and the background have to be there for it
            let headless = rune::tooling::headless_run().is_some();
            s.background.images = Some(rune::ImageCache::new(waker.clone()));
            s.art = Some(if headless {
                art::ArtCache::blocking(waker.clone())
            } else {
                art::ArtCache::new(waker.clone())
            });
            s.background.loader = Some(rune::Latest::new(waker));
            if headless {
                s.background.load_now(&s.settings.background_image);
            } else {
                s.background.load(&s.settings.background_image);
            }
        }),
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        ..AppSpec::new(TITLE, (540.0, 800.0), || Ui::new(settings::Settings::default()), view::view)
    };

    let mut app = App::new(spec).args(options.ui_args.clone()).keep_running();
    let handle = app.handle();

    if options.test_controller {
        let mut app = app.init(controller::mock_ui).on_close(|_, tx| tx.quit()).on_window(set_window_icon);
        app.window(controller::window(handle));
        app.run();
        return;
    }

    let (settings, settings_file) = settings::load();
    let rt = tokio::runtime::Handle::current();

    // Open where the window was last time. Rune's tooling (snapshots, scripts) draws at its own
    // size, the same on every machine
    let frame = settings.window.filter(|_| options.ui_args.is_empty());
    if let Some(frame) = frame {
        app = app
            .window_options(rune::WindowOptions {
                position: Some(frame.position),
                physical_size: Some(frame.size),
                ..Default::default()
            })
            .maximized(frame.maximized);
    }

    // Starting minimized hides the window in the tray if there is one to bring it back from,
    // otherwise it opens minimized
    let start_in_tray = settings.start_minimized && settings.close_to_tray;
    let minimize = settings.start_minimized && !start_in_tray;
    if start_in_tray {
        app = app.windowless();
    }
    if frame.is_some() || minimize {
        let handle = handle.clone();
        app = app.on_ready(move |window| {
            if frame.is_some() {
                keep_on_screen(window, &handle);
            }
            if minimize {
                handle.minimize();
            }
        });
    }

    app.init(move || {
        // Snapshots and scripts draw the scenes from the default settings, with nothing running
        // behind them, so they look the same on every machine
        if rune::tooling::headless_run().is_some() {
            return Ui::new(settings::Settings::default());
        }
        let mut ui = Ui::new(settings.clone());
        ui.needs_admin = options.needs_admin;
        // Started in the tray, the warp screen's art is only fetched ahead until the window shows
        ui.visible = !start_in_tray;

        let tray = tray(handle.clone());
        if start_in_tray && tray.is_none() {
            handle.open();
        }
        ui.has_tray = tray.is_some();
        let services = Services::start(handle, services::Start {
            rt,
            tap,
            settings_file,
            ws_port: settings.ws_port,
            tray,
            needs_admin: options.needs_admin,
            no_update: options.no_update,
        });
        ui.updates_supported = services.can_update();
        ui.services = Some(services);
        ui.check_for_update();
        ui
    })
    .on_close(|s: &mut Ui, tx: &mut Tx| {
        remember_frame(s);
        if !(s.settings.close_to_tray && s.has_tray) {
            tx.quit();
        }
    })
    // Resting, the window lets go of the warp screen's art it doesn't need
    .on_rest(|s: &mut Ui, _| {
        s.rest_warp_art();
        false
    })
    .on_quit(|s: &mut Ui, _| {
        remember_frame(s);
        if let Some(services) = &s.services {
            services.flush_settings(&s.settings);
        }
    })
    .on_window(|window| {
        set_window_icon(window);
        WINDOW.set(Some(window.clone()));
    })
    .run();
}

/// Keeps where the window is in the settings, to open it there next time. Does nothing once it
/// has closed (to the tray), when where it was is already kept
fn remember_frame(s: &mut Ui) {
    let Some(placement) = WINDOW.with_borrow(|window| window.as_ref().and_then(|window| window.placement())) else {
        return;
    };
    if let Some(frame) = settings::WindowFrame::of(&placement, s.settings.window) {
        s.settings_mut(|settings| settings.window = Some(frame));
    }
}

/// How much of a window's top edge has to show on its display for it to stay where it opened, in
/// pixels: enough to drag it by
const SHOWN_TOP: i32 = 120;
/// How far above its display's bottom a window's top has to be for it to stay, in pixels
const TOP_ABOVE_BOTTOM: i32 = 40;

/// Brings a window opened where it was last time back onto a display, for when that display is
/// gone or moved: centered on the display nearest to it if too little of its top edge, where it's
/// dragged from, shows
fn keep_on_screen(window: &rune::WindowHandle, handle: &AppHandle<Ui>) {
    let Some(placement) = window.placement() else { return };
    let Some(((dx, dy), (dw, dh))) = placement.display else { return };
    let (x, y) = placement.position;
    let width = placement.content_size.0 as i32;
    let shown = (x + width).min(dx + dw as i32) - x.max(dx);
    let top_inside = y >= dy && y < dy + dh as i32 - TOP_ABOVE_BOTTOM;
    if shown < SHOWN_TOP || !top_inside {
        handle.update(|_, tx| tx.center_window());
    }
}

/// Hands what the capture finds to the window
async fn forward_worker(app: AppHandle<Ui>, manager: Arc<Mutex<MultiAccountManager>>, tap: Tap, warps: WarpService) {
    use crate::session::SessionEvent;

    let mut events = pin!(worker::archiver_worker(manager.clone(), tap));
    // Every packet is reported, the window only needs to hear about the first one
    let mut traffic = false;

    while let Some(event) = events.next().await {
        match event {
            WorkerEvent::PacketReceived => {
                if !traffic {
                    traffic = true;
                    app.update(|s, _| s.traffic_seen());
                }
            }
            WorkerEvent::CaptureFailed(message) => {
                traffic = false;
                app.update(move |s, tx| s.capture_failed(message, tx));
            }
            WorkerEvent::Session(SessionEvent::Warp(event)) => warps.send(WarpCommand::Event(event)),
            WorkerEvent::Session(event) => app.update_if(move |s, _| s.session_event(&event)),
            WorkerEvent::AccountLoggedIn { uid } => {
                app.update(move |s, _| s.account_logged_in(uid));
                refresh_inventory(&app, &manager, uid).await;
            }
            WorkerEvent::AccountChanged { uid } => refresh_inventory(&app, &manager, uid).await,
        }
    }
}

/// Counts what the account holds, once all of it arrived
async fn refresh_inventory(app: &AppHandle<Ui>, manager: &Mutex<MultiAccountManager>, uid: u32) {
    let manager = manager.lock().await;
    let Some(exporter) = manager.account_exporter(uid) else {
        return;
    };
    if !exporter.is_initialized() {
        return;
    }
    let material = |id: u32| exporter.materials().get(&id).map_or(0, |m| m.count);
    let inventory = Inventory {
        relics: exporter.relics().len(),
        light_cones: exporter.light_cones().len(),
        characters: exporter.characters().len(),
        materials: exporter.materials().len(),
        funds: state::Funds {
            stellar_jade: exporter.gacha_funds().stellar_jade,
            passes: material(item::PASS),
            special_passes: material(item::SPECIAL_PASS),
            undying_starlight: material(item::UNDYING_STARLIGHT),
        },
    };
    let mut roster = state::Roster::default();
    for character in exporter.characters().values() {
        roster.eidolons.insert(character.id, character.eidolon);
    }
    for cone in exporter.light_cones().values() {
        let best = roster.superimpositions.entry(cone.id).or_insert(0);
        *best = (*best).max(cone.superimposition);
    }
    app.update_if(move |s, _| s.inventory_changed(uid, inventory) | s.roster_changed(uid, roster));
}

/// Runs the WebSocket server and reports its port and clients to the window
async fn forward_server(
    app: AppHandle<Ui>,
    manager: Arc<Mutex<MultiAccountManager>>,
    port_rx: watch::Receiver<PortCommand>,
    selected_rx: watch::Receiver<Option<u32>>,
) {
    let (ports, clients) = start_dynamic_websocket_server(WatchStream::new(port_rx), manager, selected_rx);

    let client_app = app.clone();
    tokio::spawn(async move {
        let mut clients = pin!(clients);
        while let Some(count) = clients.next().await {
            client_app.update(move |s, _| s.sync_clients(count));
        }
    });

    let mut ports = pin!(ports);
    while let Some(port) = ports.next().await {
        match port {
            Ok(port) => app.update(move |s, _| s.sync_opened(port)),
            Err(e) => {
                info!("could not start WebSocket server: {}", e);
                app.update(move |s, _| s.sync_failed(e));
            }
        }
    }
}

/// Keeps the logs panel current while it's open
async fn forward_logs(app: AppHandle<Ui>) {
    loop {
        crate::logging::wait_for_new_lines().await;
        // Lines come in bursts, one redraw is enough for each
        tokio::time::sleep(Duration::from_millis(200)).await;
        app.update_if(|s, _| {
            let open = s.panel == Some(Panel::Logs);
            if open {
                s.refresh_log_tail();
            }
            open
        });
    }
}

/// The tray icon, which brings the window back after it was closed to the tray
fn tray(app: AppHandle<Ui>) -> Option<Tray> {
    let spec = TraySpec::new("reliquary-archiver", TITLE, tray_icon()?).tooltip(TITLE).menu(vec![
        TrayItem::action("show", "Show window"),
        TrayItem::Separator,
        TrayItem::action("quit", "Quit"),
    ]);

    let tray = Tray::new(spec, move |event| match event {
        TrayEvent::Activate { token: Some(token) } => app.activate(token),
        TrayEvent::Menu(id) if id == "quit" => app.quit(),
        _ => app.open(),
    });

    tray.inspect_err(|e| info!(%e, "no system tray")).ok()
}

fn tray_icon() -> Option<TrayIcon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../../assets/icon32.png")));
    let mut reader = decoder.read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    pixels.truncate(frame.buffer_size());
    Some(TrayIcon::rgba(frame.width, frame.height, pixels))
}

/// Gives the window the icon embedded in the executable (see build.rs)
fn set_window_icon(window: &rune::WindowHandle) {
    #[cfg(windows)]
    {
        use rune::raw_window_handle::RawWindowHandle;
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON, SendMessageW,
            WM_SETICON,
        };
        use windows::core::PCWSTR;

        let hwnd = window.with_raw_handle(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as _)),
            _ => None,
        });
        let Ok(Some(hwnd)) = hwnd else {
            return;
        };

        unsafe {
            let Ok(module) = GetModuleHandleW(None) else {
                return;
            };
            // winres embeds the icon as resource 1
            let resource = PCWSTR(1 as _);
            for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
                let size = GetSystemMetrics(metric);
                if let Ok(icon) = LoadImageW(Some(module.into()), resource, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
                    SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(icon.0 as isize)));
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}
