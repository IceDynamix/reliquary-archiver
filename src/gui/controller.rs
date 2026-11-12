//! A test controller: run with `--test-controller` and a second window opens beside the app
//! with buttons that put it into every state it can show. Nothing is captured, no server runs
//! and the settings file is neither read nor written, so the app shows only what the buttons
//! say.
//!
//! The buttons send [`Act`]s, which change the app's state the way real events would. The
//! named scenes are the same ones headless snapshots use (see [`super::scenes`]).

use std::time::Instant;

use rune::{App, AppHandle, AppSpec, Cx, El, ScrollSpec, Tx, WindowSpec, col, row, text};

use super::scenes::SCENES;
use super::settings::{Accent, Anchor, Settings, ThemeMode};
use super::state::{Account, CaptureError, Inventory, LiveSync, Panel, Ui, Update};
use super::theme::{self, Theme};
use super::view::parts::{Pill, kicker, pill_for};

/// The accounts the controller logs in, in order
const UIDS: [u32; 3] = [801_234_567, 700_514_288, 602_118_903];
const INVENTORY: Inventory = Inventory {
    relics: 1532,
    light_cones: 218,
    characters: 54,
    materials: 311,
};

/// Something the controller does to the app
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    /// Back to a fresh window, keeping the appearance
    Reset,
    /// One of the named scenes, from a fresh window
    Scene(&'static str),

    Waiting,
    LogIn,
    Ready,
    AnotherAccount,
    /// One more of everything, to watch the counts change
    Grow,

    NeedsAdmin(bool),
    CaptureError(bool),
    Unsupported(bool),
    Stalled(bool),
    ReadingFile(bool),

    SyncStarting,
    SyncClients(usize),
    SyncFailed,
    SyncPaused,

    UpdateNone,
    UpdateChecking,
    UpdateAvailable,
    UpdateInstalling,
    UpdateFailed,
    UpdateCurrent,

    NoticeSaved,
    NoticeError,
    NoticeNone,

    Open(Option<Panel>),

    Theme(ThemeMode),
    Accent(Accent),
    NextAnchor,
    Background(bool),
}

/// The app's first state under the controller: defaults, with everything the controller can
/// show switched on
pub fn mock_ui() -> Ui {
    let mut ui = Ui::new(Settings::default());
    ui.has_tray = true;
    ui.updates_supported = true;
    ui
}

/// Applies `act` to the app's state
pub fn apply(act: Act, s: &mut Ui, tx: &mut Tx) {
    match act {
        Act::Reset => reset(s),
        Act::Scene(name) => {
            reset(s);
            if let Some((_, scene)) = SCENES.iter().find(|(scene, _)| *scene == name) {
                scene(s);
            }
        }

        Act::Waiting => {
            s.accounts.clear();
            s.selected = None;
            s.capture.connected = false;
        }
        Act::LogIn => {
            s.capture.traffic = true;
            s.capture.connected = true;
            let uid = s.selected.unwrap_or(UIDS[0]);
            s.account_logged_in(uid);
        }
        Act::Ready => {
            if s.selected.is_none() {
                apply(Act::LogIn, s, tx);
            }
            if let Some(uid) = s.selected {
                s.inventory_changed(uid, INVENTORY);
            }
        }
        Act::AnotherAccount => {
            let next = UIDS.iter().find(|uid| !s.accounts.iter().any(|a| a.uid == **uid));
            if let Some(&uid) = next {
                s.capture.connected = true;
                s.accounts.push(Account {
                    uid,
                    inventory: Some(INVENTORY),
                });
                s.select(uid);
            }
        }
        Act::Grow => {
            if let Some(Account {
                uid,
                inventory: Some(inventory),
            }) = s.selected_account().cloned()
            {
                let grown = Inventory {
                    relics: inventory.relics + 1,
                    light_cones: inventory.light_cones + 1,
                    characters: inventory.characters + 1,
                    materials: inventory.materials + 1,
                };
                s.inventory_changed(uid, grown);
            }
        }

        Act::NeedsAdmin(on) => s.needs_admin = on,
        Act::CaptureError(on) => {
            s.capture.error = on.then(|| CaptureError {
                message: "could not open capture source: wpcap.dll was not found".into(),
                // Never expires while the controller shows it
                at: Instant::now() + std::time::Duration::from_secs(3600),
            });
        }
        Act::Unsupported(on) => s.capture.unsupported = on,
        Act::Stalled(on) => s.capture.stalled = on,
        Act::ReadingFile(on) => s.reading_file = on,

        Act::SyncStarting => s.sync = LiveSync::Starting,
        Act::SyncClients(clients) => {
            s.sync = LiveSync::Running {
                port: s.settings.ws_port,
                clients,
            }
        }
        Act::SyncFailed => {
            s.sync = LiveSync::Starting;
            s.sync_failed("Address already in use".into());
        }
        Act::SyncPaused => s.sync = LiveSync::Closed,

        Act::UpdateNone => s.update = Update::Idle,
        Act::UpdateChecking => s.update = Update::Checking,
        Act::UpdateAvailable => {
            s.update = Update::Available { version: "0.20.0".into() };
            s.update_dismissed = false;
        }
        Act::UpdateInstalling => s.update = Update::Installing,
        Act::UpdateFailed => s.update = Update::Failed("the release has no asset for this build".into()),
        Act::UpdateCurrent => s.update = Update::UpToDate,

        Act::NoticeSaved => {
            let file = std::env::temp_dir().join("archive_output-2026-11-07T21-14-03.json");
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned());
            s.notify("Export saved", name, false, Some(file), tx);
        }
        Act::NoticeError => s.notify(
            "Couldn't save the export",
            Some("Access is denied. (os error 5)".into()),
            true,
            None,
            tx,
        ),
        Act::NoticeNone => s.notice = None,

        Act::Open(panel) => match panel {
            Some(panel) if s.panel != Some(panel) => s.toggle_panel(panel),
            Some(_) => {}
            None => s.panel = None,
        },

        Act::Theme(mode) => s.settings.theme = mode,
        Act::Accent(accent) => s.settings.accent = accent,
        Act::NextAnchor => {
            let at = Anchor::ALL.iter().position(|a| *a == s.settings.card_anchor).unwrap_or(0);
            s.settings.card_anchor = Anchor::ALL[(at + 1) % Anchor::ALL.len()];
        }
        Act::Background(on) => {
            s.settings.background_image = if on { background_path() } else { String::new() };
        }
    }
    s.load_background();
}

/// Everything that happened goes, the appearance stays
fn reset(s: &mut Ui) {
    let settings = Settings {
        theme: s.settings.theme,
        accent: s.settings.accent,
        card_anchor: s.settings.card_anchor,
        background_image: s.settings.background_image.clone(),
        background_opacity: s.settings.background_opacity,
        image_fit: s.settings.image_fit,
        ..Settings::default()
    };
    let images = s.images.take();
    let background = s.background.take();
    let loader = s.background_loader.take();
    let loading = s.background_loading.take();
    *s = mock_ui();
    s.settings = settings;
    s.images = images;
    s.background = background;
    s.background_loader = loader;
    s.background_loading = loading;
}

/// The image `SCENE_BACKGROUND` names, else the splash image in the repository
fn background_path() -> String {
    std::env::var("SCENE_BACKGROUND").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/hsr_hyperdrive.jpg").into())
}

/// The controller's own state: the app it drives
pub struct Controller {
    app: AppHandle<Ui>,
}

/// The controller's window, driving the app behind `app`
pub fn window(app: AppHandle<Ui>) -> WindowSpec<Controller> {
    let spec = AppSpec {
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        min_size: Some((340.0, 400.0)),
        ..AppSpec::new(
            "Test controller",
            (380.0, 760.0),
            || unreachable!("the controller starts from init"),
            view,
        )
    };
    WindowSpec::from_app("controller", App::new(spec).init(move || Controller { app }))
}

fn view(_: &Controller, cx: &mut Cx<Controller>) -> El {
    let t = Theme::new(cx.system_dark(), Accent::Silver, false);
    let mut sections = col().key("controller").w_full().padding(16.0).gap(16.0).child(
        col()
            .gap(2.0)
            .child(kicker("Reliquary Archiver", t.accent))
            .child(text("Test controller").font(theme::HEADLINE).color(t.text))
            .child(
                text("Puts the app into any state. Nothing is captured, served or saved.")
                    .font(theme::SMALL)
                    .color(t.muted)
                    .wrap(),
            ),
    );

    let scenes: Vec<(String, Act)> = SCENES.iter().map(|(name, _)| ((*name).to_string(), Act::Scene(name))).collect();
    sections = sections.child(section("Scenes", &scenes, &t));
    for (title, acts) in SECTIONS {
        let acts: Vec<(String, Act)> = acts.iter().map(|(label, act)| ((*label).to_string(), *act)).collect();
        sections = sections.child(section(title, &acts, &t));
    }

    col()
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(sections.scroll(ScrollSpec { stick_bottom: false }).grow(1.0).min_h(0.0))
}

/// The controls apart from the scenes, by what they change
const SECTIONS: &[(&str, &[(&str, Act)])] = &[
    ("Reset", &[("Fresh window", Act::Reset)]),
    (
        "Account",
        &[
            ("Waiting", Act::Waiting),
            ("Log in", Act::LogIn),
            ("Ready", Act::Ready),
            ("+ account", Act::AnotherAccount),
            ("Grow counts", Act::Grow),
        ],
    ),
    (
        "Problems",
        &[
            ("Needs admin", Act::NeedsAdmin(true)),
            ("Capture error", Act::CaptureError(true)),
            ("Unsupported", Act::Unsupported(true)),
            ("Interrupted", Act::Stalled(true)),
            ("Reading file", Act::ReadingFile(true)),
            ("Clear admin", Act::NeedsAdmin(false)),
            ("Clear error", Act::CaptureError(false)),
            ("Clear unsupported", Act::Unsupported(false)),
            ("Clear interrupted", Act::Stalled(false)),
            ("Done reading", Act::ReadingFile(false)),
        ],
    ),
    (
        "Live sync",
        &[
            ("Starting", Act::SyncStarting),
            ("No optimizer", Act::SyncClients(0)),
            ("1 linked", Act::SyncClients(1)),
            ("3 linked", Act::SyncClients(3)),
            ("Port in use", Act::SyncFailed),
            ("Paused", Act::SyncPaused),
        ],
    ),
    (
        "Updates",
        &[
            ("None", Act::UpdateNone),
            ("Checking", Act::UpdateChecking),
            ("Available", Act::UpdateAvailable),
            ("Installing", Act::UpdateInstalling),
            ("Failed", Act::UpdateFailed),
            ("Up to date", Act::UpdateCurrent),
        ],
    ),
    (
        "Notices",
        &[
            ("Saved", Act::NoticeSaved),
            ("Error", Act::NoticeError),
            ("Dismiss", Act::NoticeNone),
        ],
    ),
    (
        "Panels",
        &[
            ("Settings", Act::Open(Some(Panel::Settings))),
            ("Logs", Act::Open(Some(Panel::Logs))),
            ("Close", Act::Open(None)),
        ],
    ),
    (
        "Appearance",
        &[
            ("System", Act::Theme(ThemeMode::System)),
            ("Dark", Act::Theme(ThemeMode::Dark)),
            ("Light", Act::Theme(ThemeMode::Light)),
            ("Gold", Act::Accent(Accent::Gold)),
            ("Silver", Act::Accent(Accent::Silver)),
            ("Ruby", Act::Accent(Accent::Ruby)),
            ("Amethyst", Act::Accent(Accent::Amethyst)),
            ("Emerald", Act::Accent(Accent::Emerald)),
            ("Sapphire", Act::Accent(Accent::Sapphire)),
            ("Next position", Act::NextAnchor),
            ("Background on", Act::Background(true)),
            ("Background off", Act::Background(false)),
        ],
    ),
];

fn section(title: &str, acts: &[(String, Act)], t: &Theme) -> El {
    let buttons = acts.iter().map(|(label, act)| {
        let act = *act;
        let key = format!("act-{}", label.to_lowercase().replace(' ', "-"));
        pill_for(&key, label, None, Pill::Secondary, true, t, move |c: &mut Controller, _| {
            c.app.update(move |s, tx| apply(act, s, tx));
        })
        .h(28.0)
        .px(12.0)
    });
    col()
        .key(format!("section-{}", title.to_lowercase().replace(' ', "-")))
        .w_full()
        .gap(8.0)
        .child(kicker(title, t.muted))
        .child(row().w_full().gap(6.0).flex_wrap().children(buttons))
}

#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use super::*;
    use crate::gui::state::Stage;

    /// Runs `acts` against a fresh app, the way the controller would
    fn after(acts: &[Act]) -> Ui {
        let mut rt = Runtime::new(
            mock_ui(),
            crate::gui::view::view,
            (460.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for act in acts {
            let act = *act;
            rt.update(move |s, tx| apply(act, s, tx));
            rt.frame(1.0 / 60.0);
        }
        std::mem::replace(&mut rt.state, mock_ui())
    }

    #[test]
    fn every_control_draws_the_window() {
        let mut acts: Vec<Act> = SECTIONS.iter().flat_map(|(_, acts)| acts.iter().map(|(_, act)| *act)).collect();
        acts.extend(SCENES.iter().map(|(name, _)| Act::Scene(name)));
        for act in acts {
            let ui = after(&[act]);
            let _ = ui.stage();
        }
    }

    #[test]
    fn the_account_controls_walk_through_the_stages() {
        assert_eq!(after(&[Act::LogIn]).stage(), Stage::Reading { uid: UIDS[0] });
        assert_eq!(
            after(&[Act::Ready]).stage(),
            Stage::Ready {
                uid: UIDS[0],
                inventory: INVENTORY
            }
        );
        let two = after(&[Act::Ready, Act::AnotherAccount]);
        assert_eq!(two.accounts.len(), 2);
        assert_eq!(two.selected, Some(UIDS[1]));
        assert_eq!(after(&[Act::Ready, Act::Waiting]).stage(), Stage::Waiting);
    }

    #[test]
    fn growing_adds_one_of_everything() {
        let ui = after(&[Act::Ready, Act::Grow]);
        let Stage::Ready { inventory, .. } = ui.stage() else {
            panic!("not ready")
        };
        assert_eq!(inventory.relics, INVENTORY.relics + 1);
    }

    #[test]
    fn a_reset_keeps_the_appearance() {
        let ui = after(&[Act::Accent(Accent::Ruby), Act::Theme(ThemeMode::Light), Act::Ready, Act::Reset]);
        assert_eq!(ui.stage(), Stage::Waiting);
        assert_eq!(ui.settings.accent, Accent::Ruby);
        assert_eq!(ui.settings.theme, ThemeMode::Light);
    }

    #[test]
    fn a_scene_starts_from_a_fresh_window() {
        let ui = after(&[Act::NeedsAdmin(true), Act::Scene("ready")]);
        assert!(!ui.needs_admin);
        assert!(matches!(ui.stage(), Stage::Ready { .. }));
    }
}
