//! Named states for rendering the window headlessly, with nothing captured or running:
//!
//! ```sh
//! reliquary-archiver --ui --list-scenes
//! reliquary-archiver --ui --snap ready.png --scene ready [--theme light]
//! ```

use std::time::{Duration, Instant};

use super::settings::{Accent, Anchor, BackgroundFill, ImageFit};
use super::state::{Account, CaptureError, Inventory, LiveSync, Notice, Panel, Ui, Update};

/// A name and what it changes in a fresh window
pub type Scene = (&'static str, fn(&mut Ui));

pub const SCENES: &[Scene] = &[
    ("waiting", |s| running(s, 0)),
    ("reading", |s| {
        running(s, 0);
        s.capture.connected = true;
        logged_in(s, UID, None);
    }),
    ("ready", |s| {
        running(s, 0);
        logged_in(s, UID, Some(INVENTORY));
    }),
    ("synced", linked),
    ("accounts", |s| {
        running(s, 2);
        logged_in(
            s,
            700_514_288,
            Some(Inventory {
                relics: 412,
                light_cones: 96,
                characters: 31,
                materials: 140,
            }),
        );
        logged_in(s, UID, Some(INVENTORY));
    }),
    ("stalled", |s| {
        running(s, 1);
        logged_in(s, UID, None);
        s.capture.stalled = true;
    }),
    ("stalled-ready", |s| {
        linked(s);
        s.capture.stalled = true;
    }),
    ("unsupported", |s| {
        running(s, 0);
        s.updates_supported = true;
        s.capture.unsupported = true;
    }),
    ("capture-error", |s| {
        running(s, 0);
        s.capture.error = Some(CaptureError {
            message: CAPTURE_ERROR.into(),
            at: Instant::now(),
        });
    }),
    ("needs-admin", |s| {
        running(s, 0);
        s.needs_admin = true;
    }),
    ("sync-failed", |s| {
        s.sync = LiveSync::Failed("Address already in use".into());
    }),
    ("update", |s| {
        running(s, 0);
        logged_in(s, UID, Some(INVENTORY));
        s.update = Update::Available { version: "0.20.0".into() };
    }),
    ("installing", |s| {
        s.sync = LiveSync::Closed;
        s.update = Update::Installing;
    }),
    ("settings", |s| {
        running(s, 0);
        s.has_tray = true;
        s.updates_supported = true;
        s.update = Update::UpToDate;
        s.panel = Some(Panel::Settings);
    }),
    ("port-error", |s| {
        running(s, 0);
        s.has_tray = true;
        s.panel = Some(Panel::Settings);
        s.port.edit.replace_text("80");
        s.port.error = Some("Port 80 is in use. Close other copies or choose another port.".into());
    }),
    ("logs", |s| {
        running(s, 0);
        s.panel = Some(Panel::Logs);
        s.log_tail = LOG.lines().map(str::to_string).collect();
    }),
    ("background", |s| {
        linked(s);
        background(s, 0.45);
    }),
    ("background-waiting", |s| {
        running(s, 0);
        background(s, 0.45);
    }),
    ("background-settings", |s| {
        running(s, 0);
        s.has_tray = true;
        background(s, 0.3);
        s.panel = Some(Panel::Settings);
    }),
    // The background contained, with a fill around it: the one `SCENE_FILL` numbers (as
    // `BackgroundFill::ALL`), Blur without
    ("background-fill", |s| {
        linked(s);
        background(s, 0.9);
        s.settings.image_fit = ImageFit::Contain;
        let fill = std::env::var("SCENE_FILL").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1);
        s.settings.background_fill = BackgroundFill::ALL[fill.min(BackgroundFill::ALL.len() - 1)];
        s.settings.background_anchor = [0.5, 0.5];
        s.settings.background_target = Some([0.3, 0.5]);
    }),
    // The background's position and fill in the settings, over the background they set
    ("background-position", |s| {
        linked(s);
        background(s, 0.6);
        s.settings.image_fit = ImageFit::Contain;
        s.settings.background_fill = BackgroundFill::MirrorBlur;
        s.settings.background_anchor = [0.3, 0.6];
        s.settings.background_target = Some([0.5, 0.2]);
        s.panel = Some(Panel::Settings);
    }),
    // The background covering the window, zoomed in on a point to the right of its middle
    ("background-zoom", |s| {
        linked(s);
        background(s, 0.6);
        s.settings.image_fit = ImageFit::Cover;
        s.settings.background_zoom = 1.6;
        s.settings.background_anchor = [0.7, 0.4];
        s.settings.background_target = None;
        s.panel = Some(Panel::Settings);
    }),
    // The color around the background picked in the settings
    ("background-color", background_color),
    // The eyedropper out, to pick the color around the background from the image
    ("background-dropper", |s| {
        background_color(s);
        s.background.dropper = true;
    }),
    ("anchored", |s| {
        linked(s);
        background(s, 0.5);
        s.settings.card_anchor = Anchor::BottomRight;
    }),
    ("anchored-settings", |s| {
        linked(s);
        background(s, 0.5);
        s.settings.card_anchor = Anchor::Left;
        s.panel = Some(Panel::Settings);
    }),
    ("appearance", |s| {
        linked(s);
        s.settings.accent = Accent::Sapphire;
        s.panel = Some(Panel::Settings);
    }),
    ("gold", |s| gem(s, Accent::Gold)),
    ("silver", |s| gem(s, Accent::Silver)),
    ("ruby", |s| gem(s, Accent::Ruby)),
    ("amethyst", |s| gem(s, Accent::Amethyst)),
    ("emerald", |s| gem(s, Accent::Emerald)),
    ("sapphire", |s| gem(s, Accent::Sapphire)),
    ("saved", |s| {
        linked(s);
        s.notice = Some(Notice {
            id: 0,
            title: "Export saved".into(),
            detail: Some("archive_output-2026-11-07T21-14-03.json".into()),
            error: false,
            file: Some("archive_output-2026-11-07T21-14-03.json".into()),
            until: Instant::now() + Duration::from_secs(60),
        });
    }),
];

/// The made-up account the scenes log in
pub(super) const UID: u32 = 801_234_567;
/// What it holds
pub(super) const INVENTORY: Inventory = Inventory {
    relics: 1532,
    light_cones: 218,
    characters: 54,
    materials: 311,
};
/// Why capturing failed, in the scenes that show it
pub(super) const CAPTURE_ERROR: &str = "could not open capture source: wpcap.dll was not found";

const LOG: &str = "\
2026-11-07T21:12:40.120Z  INFO reliquary_archiver::gui::settings: Loaded settings from C:\\Users\\Player\\AppData\\Local\\reliquary-archiver\\settings.json
2026-11-07T21:12:40.311Z  INFO reliquary_archiver::websocket: Starting WebSocket server on 0.0.0.0:23313
2026-11-07T21:12:40.402Z  INFO reliquary_archiver::capture: Discovered new device: pktmon
2026-11-07T21:12:40.402Z  INFO reliquary_archiver::gui::worker: instructions: go to main menu screen and go to the \"Click to Start\" screen
2026-11-07T21:13:02.877Z  INFO reliquary_archiver::accounts: discovered account uid=801234567
2026-11-07T21:13:04.019Z  INFO reliquary_archiver::export::fribbels::exporter: initialized relics=1532 light_cones=218 characters=54";

fn running(s: &mut Ui, clients: usize) {
    s.capture.traffic = true;
    s.sync = LiveSync::Running { port: 23313, clients };
}

/// The made-up account read, with an optimizer linked
fn linked(s: &mut Ui) {
    running(s, 1);
    logged_in(s, UID, Some(INVENTORY));
}

/// A stand-in for a user's own background
fn background(s: &mut Ui, opacity: f32) {
    s.settings.background_image = background_path();
    s.settings.background_opacity = opacity;
}

/// The image `SCENE_BACKGROUND` names, else the splash image in the repository
pub(super) fn background_path() -> String {
    std::env::var("SCENE_BACKGROUND").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/hsr_hyperdrive.jpg").into())
}

/// The background contained, the color around it picked in the settings
fn background_color(s: &mut Ui) {
    linked(s);
    background(s, 0.6);
    s.settings.image_fit = ImageFit::Contain;
    s.settings.background_fill = BackgroundFill::Color;
    s.settings.background_color = Some("#2a1f4d".into());
    s.background.hex.replace_text("#2a1f4d");
    s.panel = Some(Panel::Settings);
}

/// An accent while an account is read, so face up and face down tiles both show
fn gem(s: &mut Ui, accent: Accent) {
    running(s, 1);
    logged_in(s, UID, None);
    s.settings.accent = accent;
}

fn logged_in(s: &mut Ui, uid: u32, inventory: Option<Inventory>) {
    s.capture.connected = true;
    s.accounts.push(Account { uid, inventory });
    s.selected = Some(uid);
}
