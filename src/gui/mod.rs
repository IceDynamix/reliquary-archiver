//! The desktop window.
//!
//! The window answers one question: is my account's data in, and how do I get it into the
//! optimizer? It leads with where capturing stands (waiting for a login, reading an account, or
//! ready), says what to do when something is in the way, and offers the two ways out: live sync
//! over the WebSocket server, or saving an export file.
//!
//! - [`state`]: what the window shows and how events change it, as plain data
//! - [`services`]: the work that leaves the UI thread (files, the server, updates)
//! - [`view`]: draws the state with Rune
//! - [`worker`]: reads packets and reports what happened
//! - [`settings`]: the settings and their file
//!
//! Every screen can be rendered without a window or a capture: `--ui --snap out.png --scene ready`
//! (see [`scenes`]).

use std::pin::pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::lock::Mutex;
use reliquary_archiver::export::Exporter;
use rune::{App, AppHandle, AppSpec, Tray, TrayEvent, TrayIcon, TrayItem, TraySpec, Tx};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tracing::{info, warn};

use crate::accounts::MultiAccountManager;
use crate::capture::Tap;
use crate::websocket::{PortCommand, PortSource, start_websocket_server};

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

const TITLE: &str = "Reliquary Archiver";

pub struct Options {
    /// Capturing needs administrator rights the app doesn't have, so it won't start
    pub needs_admin: bool,
    /// Updates were turned off from the command line
    pub no_update: bool,
    /// Flags for Rune's own tooling, such as `--snap` for headless snapshots
    pub ui_args: Vec<String>,
}

/// Shows the window and captures from `tap` until the app quits
pub fn run(tap: Tap, options: Options) {
    let (settings, settings_file) = settings::load();
    let rt = tokio::runtime::Handle::current();

    let spec = AppSpec {
        min_size: Some((400.0, 540.0)),
        scenes: scenes::SCENES,
        poll: Some(|s: &mut Ui, _, _| s.expire(Instant::now())),
        setup: Some(|s: &mut Ui, waker| {
            s.images = Some(rune::ImageCache::new(waker));
            s.load_background();
            // Snapshots draw a single frame, the background has to be there for it
            if rune::tooling::headless_run().is_some()
                && let Some((_, image)) = &s.background
            {
                let _ = image.wait();
            }
        }),
        // Inter for the interface, the system's monospace font for logs and addresses
        families: ("Inter", "ui-monospace"),
        ..AppSpec::new(TITLE, (440.0, 600.0), || Ui::new(settings::Settings::default()), view::view)
    };

    let mut app = App::new(spec).args(options.ui_args.clone()).keep_running();
    let handle = app.handle();

    // Starting minimized hides the window in the tray if there is one to bring it back from,
    // otherwise it opens minimized
    let start_in_tray = settings.start_minimized && settings.close_to_tray;
    if start_in_tray {
        app = app.windowless();
    } else if settings.start_minimized {
        let handle = handle.clone();
        app = app.on_ready(move |_| handle.minimize());
    }

    app.init(move || {
        let mut ui = Ui::new(settings.clone());
        ui.needs_admin = options.needs_admin;
        // Snapshots and scripts draw the scenes, with nothing running behind them
        if rune::tooling::headless_run().is_some() {
            return ui;
        }

        let tray = tray(handle.clone());
        if start_in_tray && tray.is_none() {
            handle.open();
        }
        ui.has_tray = tray.is_some();
        let services = start(rt, handle, tap, settings, settings_file, &options, tray);
        ui.updates_supported = services.can_update();
        ui.services = Some(services);
        ui.check_for_update();
        ui
    })
    .on_close(|s: &mut Ui, tx: &mut Tx| {
        if !(s.settings.close_to_tray && s.has_tray) {
            tx.quit();
        }
    })
    .on_quit(|s: &mut Ui, _| {
        if let Some(services) = &s.services {
            services.save_settings(&s.settings);
            services.settings_file.flush();
        }
    })
    .on_window(set_window_icon)
    .run();
}

/// Starts capturing, the WebSocket server and everything that feeds the window
fn start(
    rt: tokio::runtime::Handle,
    app: AppHandle<Ui>,
    tap: Tap,
    settings: settings::Settings,
    settings_file: settings::SettingsFile,
    options: &Options,
    tray: Option<Tray>,
) -> Services {
    let manager = Arc::new(Mutex::new(MultiAccountManager::new()));
    let (selected_tx, selected_rx) = watch::channel(None);
    let (port_tx, port_rx) = watch::channel(PortCommand::Open(settings.ws_port));

    rt.spawn(settings_file.writer());
    rt.spawn(forward_server(app.clone(), manager.clone(), port_rx, selected_rx));
    if options.needs_admin {
        warn!("not capturing, administrator rights are needed");
    } else {
        rt.spawn(forward_worker(app.clone(), manager.clone(), tap));
    }
    rt.spawn(forward_logs(app.clone()));

    Services {
        rt,
        app,
        manager,
        selected_tx,
        port_tx,
        settings_file,
        no_update: options.no_update || std::env::var("NO_SELF_UPDATE").is_ok_and(|v| v == "1"),
        _tray: tray,
    }
}

/// Hands what the capture finds to the window
async fn forward_worker(app: AppHandle<Ui>, manager: Arc<Mutex<MultiAccountManager>>, tap: Tap) {
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
            WorkerEvent::Session(
                SessionEvent::PacketsDecoded(0)
                | SessionEvent::DecodeError
                | SessionEvent::AccountSeen { .. }
                | SessionEvent::LoginFinished,
            ) => {}
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
    let Some(exporter) = manager.lock().await.get_account_exporter(uid) else {
        return;
    };
    let exporter = exporter.lock().await;
    if !exporter.is_initialized() {
        return;
    }
    let inventory = Inventory {
        relics: exporter.relics().len(),
        light_cones: exporter.light_cones().len(),
        characters: exporter.characters().len(),
        materials: exporter.materials().len(),
    };
    app.update_if(move |s, _| s.inventory_changed(uid, inventory));
}

/// Runs the WebSocket server and reports its port and clients to the window
async fn forward_server(
    app: AppHandle<Ui>,
    manager: Arc<Mutex<MultiAccountManager>>,
    port_rx: watch::Receiver<PortCommand>,
    selected_rx: watch::Receiver<Option<u32>>,
) {
    let (ports, clients) = match start_websocket_server(PortSource::Dynamic(WatchStream::new(port_rx)), manager, selected_rx).await {
        Ok(streams) => streams,
        Err(e) => {
            app.update(move |s, _| s.sync_failed(e));
            return;
        }
    };

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
