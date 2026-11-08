//! Named states for rendering the window headlessly, with nothing captured or running:
//!
//! ```sh
//! reliquary-archiver --ui --list-scenes
//! reliquary-archiver --ui --snap ready.png --scene ready [--theme light]
//! ```

use std::time::Instant;

use super::settings::{Accent, Anchor};
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
    ("synced", |s| {
        running(s, 1);
        logged_in(s, UID, Some(INVENTORY));
    }),
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
        running(s, 1);
        logged_in(s, UID, Some(INVENTORY));
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
            message: "could not open capture source: wpcap.dll was not found".into(),
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
        s.port_edit.replace_text("80");
        s.port_error = Some("Port 80 is already in use. Close other copies of the archiver, or choose another port.".into());
    }),
    ("logs", |s| {
        running(s, 0);
        s.panel = Some(Panel::Logs);
        s.log_tail = LOG.lines().map(str::to_string).collect();
    }),
    ("background", |s| {
        running(s, 1);
        logged_in(s, UID, Some(INVENTORY));
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
    ("anchored", |s| {
        running(s, 1);
        logged_in(s, UID, Some(INVENTORY));
        background(s, 0.5);
        s.settings.card_anchor = Anchor::BottomRight;
    }),
    ("anchored-settings", |s| {
        running(s, 1);
        logged_in(s, UID, Some(INVENTORY));
        background(s, 0.5);
        s.settings.card_anchor = Anchor::Left;
        s.panel = Some(Panel::Settings);
    }),
    ("appearance", |s| {
        running(s, 1);
        logged_in(s, UID, Some(INVENTORY));
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
        running(s, 1);
        logged_in(s, UID, Some(INVENTORY));
        s.notice = Some(Notice {
            id: 0,
            title: "Export saved".into(),
            detail: Some("archive_output-2026-11-07T21-14-03.json".into()),
            error: false,
            file: Some("archive_output-2026-11-07T21-14-03.json".into()),
            until: Instant::now() + std::time::Duration::from_secs(60),
        });
    }),
];

const UID: u32 = 801_234_567;
const INVENTORY: Inventory = Inventory {
    relics: 1532,
    light_cones: 218,
    characters: 54,
    materials: 311,
};

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

/// A stand-in for a user's own background: the image `SCENE_BACKGROUND` names, else the splash
/// image in the repository
fn background(s: &mut Ui, opacity: f32) {
    s.settings.background_image =
        std::env::var("SCENE_BACKGROUND").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/hsr_hyperdrive.jpg").into());
    s.settings.background_opacity = opacity;
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
