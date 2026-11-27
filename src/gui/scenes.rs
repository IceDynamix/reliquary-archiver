//! Named states for rendering the window headlessly, with nothing captured or running:
//!
//! ```sh
//! reliquary-archiver --ui --list-scenes
//! reliquary-archiver --ui --snap ready.png --scene ready [--theme light]
//! ```

use std::time::{Duration, Instant};

use super::settings::{Accent, Anchor, BackgroundFill, ImageFit};
use super::state::{Account, CaptureError, Funds, Inventory, LiveSync, Notice, Panel, ProfilePick, Roster, Screen, Ui, Update, WarpPage};
use crate::warps::Pool;
use crate::warps::service::{KeyState, ProfileChoice, RunningBanner, Summary};
use crate::warps::stats::{Featured, RateUps, pool_stats};

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
                funds: INVENTORY.funds,
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
    ("warps", |s| {
        linked(s);
        warps(s, true);
    }),
    // The warp screen turned to its history page
    ("warp-history", |s| warp_screen(s, WarpPage::History)),
    ("warp-history-lc", |s| {
        warp_screen(s, WarpPage::History);
        s.warps.pool = Pool::LightCone;
    }),
    // Turned further, to where the history comes from
    ("warp-import-export", |s| warp_screen(s, WarpPage::ImportExport)),
    // An account with no warps kept yet
    ("warps-new", |s| {
        running(s, 0);
        logged_in(s, UID, Some(INVENTORY));
        s.warps.accounts = vec![UID];
        s.warps.summaries.insert(UID, empty_summary());
        s.screen = Screen::Warps;
    }),
    ("warps-syncing", |s| {
        linked(s);
        warps(s, false);
        s.warps.busy = Some("Syncing Character warps… 120".into());
    }),
    ("warp-screen", |s| warp_screen(s, WarpPage::Banner)),
    // Another of the running banners picked, one the account owns
    ("warp-screen-picked", |s| {
        warp_screen(s, WarpPage::Banner);
        s.warps.featured.insert(Pool::Character, 1506);
    }),
    // A character's banner art on its own: the one `WARP_HERO` names, as a running banner, picked
    ("warp-screen-hero", |s| {
        warp_screen(s, WarpPage::Banner);
        let Some(hero) = std::env::var("WARP_HERO").ok().and_then(|id| id.parse::<u32>().ok()) else {
            return;
        };
        if let Some(summary) = s.warps.summaries.get_mut(&UID) {
            let mut banner = summary.banners[0].clone();
            banner.gacha_id = 9999;
            banner.fives = vec![hero];
            summary.banners.push(banner);
        }
        s.warps.featured.insert(Pool::Character, hero);
    }),
    // Not logged in this run: no funds or roster, so no budget
    ("warp-screen-offline", |s| {
        running(s, 1);
        warps(s, true);
        s.warps.accounts = vec![UID];
        s.screen = Screen::Warps;
    }),
    ("warp-screen-lc", |s| {
        warp_screen(s, WarpPage::Banner);
        s.warps.pool = Pool::LightCone;
    }),
    // The warp screen with a real database, the one `WARPS_DB` names
    ("warp-screen-db", |s| from_db(s, WarpPage::Banner)),
    // The history page of a real database, the one `WARPS_DB` names
    ("warp-history-db", |s| from_db(s, WarpPage::History)),
    ("warps-pick", |s| {
        linked(s);
        warps(s, false);
        s.screen = Screen::Warps;
        s.warps.page = WarpPage::ImportExport;
        s.warps.choose = Some(ProfilePick {
            path: "starrailstation-backup.dat".into(),
            uid: UID,
            profiles: vec![
                ProfileChoice {
                    key: "1".into(),
                    name: "Default".into(),
                    pulls: 521,
                    latest: Some(1_685_664_600),
                    fits: true,
                },
                ProfileChoice {
                    key: "2".into(),
                    name: "slate".into(),
                    pulls: 7399,
                    latest: Some(1_794_472_323),
                    fits: true,
                },
            ],
        });
    }),
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
    funds: Funds {
        stellar_jade: 6420,
        passes: 12,
        special_passes: 37,
        undying_starlight: 592,
    },
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

/// The made-up account's warps on the warp screen, turned to `page`
fn warp_screen(s: &mut Ui, page: WarpPage) {
    linked(s);
    warps(s, true);
    roster(s);
    s.screen = Screen::Warps;
    s.warps.page = page;
}

/// The warp screen with a real database, the one `WARPS_DB` names, turned to `page` and to the
/// pool whose gacha type `WARPS_POOL` names
fn from_db(s: &mut Ui, page: WarpPage) {
    running(s, 1);
    let Some(summary) = std::env::var_os("WARPS_DB").and_then(|path| crate::warps::service::summary_of(path.as_ref(), None)) else {
        return;
    };
    logged_in(s, summary.uid, Some(INVENTORY));
    s.warps.accounts = vec![summary.uid];
    s.warps.summaries.insert(summary.uid, summary);
    s.screen = Screen::Warps;
    s.warps.page = page;
    if let Ok(pool) = std::env::var("WARPS_POOL") {
        let gacha_type = pool.parse().unwrap_or(Pool::Character.gacha_type());
        s.warps.pool = Pool::from_gacha_type(gacha_type).unwrap_or(Pool::Character);
    }
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

/// What the made-up account owns
fn roster(s: &mut Ui) {
    let roster = Roster {
        eidolons: [(1506, 0), (1308, 2), (1501, 1)].into_iter().collect(),
        superimpositions: [(23048, 1)].into_iter().collect(),
    };
    s.warps.rosters.insert(UID, roster);
}

fn logged_in(s: &mut Ui, uid: u32, inventory: Option<Inventory>) {
    s.capture.connected = true;
    s.accounts.push(Account { uid, inventory });
    s.selected = Some(uid);
}

/// A made-up account's warps: a few hundred pulls on each pool, with 5 stars at believable pity,
/// the last requests only seen live. `synced` gives it a fresh authkey and sync
fn warps(s: &mut Ui, synced: bool) {
    use crate::warps::{Pull, source};

    let now = jiff::Timestamp::now().as_second();
    // (pool, banner, featured 5 star, standard 5 star, pulls, 5 star pity in order)
    let plans: [(Pool, u32, u32, u32, &[u32]); 5] = [
        (Pool::Character, 2142, 1506, 1003, &[78, 64, 81, 23, 76, 90, 71, 58, 49]),
        (Pool::LightCone, 3116, 23048, 23004, &[66, 71, 12, 64, 58]),
        (Pool::Standard, 1001, 1209, 1107, &[85, 74, 79]),
        (Pool::CollabCharacter, 4142, 1014, 1003, &[71]),
        (Pool::CollabLightCone, 5142, 23045, 23004, &[]),
    ];
    let fours = [1214, 1110, 1109, 21002, 21008, 1105];
    let threes = [20000, 20001, 20002, 20005, 20010, 20011, 20015];

    let mut pulls = Vec::new();
    let mut rate_ups = RateUps::new();
    let mut id = 1_700_000_000_000_000_086_i64;
    for (pool, banner, featured, standard, pities) in plans {
        rate_ups.insert(banner, Featured {
            items: [featured, 1214, 1110, 1109].into_iter().collect(),
            not: [standard].into_iter().collect(),
        });
        let mut n = 0u32;
        let mut lost_next = false;
        for (k, &pity) in pities.iter().enumerate() {
            for p in 1..=pity {
                n += 1;
                let (item, rarity) = if p == pity {
                    // Every third 50/50 is lost, the one after is guaranteed
                    let lost = pool != Pool::Standard && !lost_next && k % 3 == 1;
                    lost_next = lost;
                    (if lost || pool == Pool::Standard { standard } else { featured }, 5)
                } else if n % 9 == 0 {
                    (fours[(n as usize / 9) % fours.len()], 4)
                } else {
                    (threes[(n as usize * 7) % threes.len()], 3)
                };
                id += 100;
                pulls.push(Pull {
                    id: Some(id),
                    uid: UID,
                    pool,
                    gacha_id: banner,
                    item_id: item,
                    rarity,
                    time: now - 400 * 86400 + i64::from(n) * 3000 + pool as i64,
                    request_no: None,
                    batch_index: 0,
                    sources: source::API,
                });
            }
        }
        // A few more since the last 5 star, the latest two only seen live
        for extra in 1..=12u32 {
            n += 1;
            let live = pool == Pool::Character && extra > 10;
            let (item, rarity) = if extra == 7 {
                (fours[extra as usize % fours.len()], 4)
            } else {
                (threes[(n as usize * 7) % threes.len()], 3)
            };
            id += 100;
            pulls.push(Pull {
                id: (!live).then_some(id),
                uid: UID,
                pool,
                gacha_id: banner,
                item_id: item,
                rarity,
                time: now - 2000 + i64::from(extra) * 60,
                request_no: live.then_some(1570 + extra),
                batch_index: 0,
                sources: if live { source::LIVE } else { source::API },
            });
        }
    }

    let refs: Vec<&Pull> = pulls.iter().collect();
    let pools: Vec<_> = plans
        .iter()
        .map(|plan| plan.0)
        .map(|pool| {
            pool_stats(pool, &refs, &rate_ups, |item| {
                reliquary_archiver::export::database::get_database().warp_item_rarity(item)
            })
        })
        .collect();
    let db = reliquary_archiver::export::database::get_database();
    let names = pulls
        .iter()
        .map(|p| (p.item_id, db.warp_item_name(p.item_id).unwrap_or_else(|| format!("#{}", p.item_id))))
        .collect();
    let unconfirmed = pulls.iter().filter(|p| p.id.is_none()).count();
    let mut summary = Summary {
        uid: UID,
        total: pulls.len(),
        unconfirmed,
        requests: pulls.len() - 2,
        request_counter: Some(pulls.len() as u32 + 5),
        key: Some(KeyState {
            origin: crate::warps::store::KeyOrigin::Capture,
            obtained_at: if synced { now - 1800 } else { now - 3 * 86400 },
            expired: !synced,
        }),
        last_sync: Some(now - 240),
        last_error: None,
        next_look: Some(now + 1500),
        pools,
        names,
        banners: running_banners(now),
    };
    for banner in &summary.banners {
        for &item in banner.fives.iter().chain(&banner.fours) {
            summary
                .names
                .entry(item)
                .or_insert_with(|| db.warp_item_name(item).unwrap_or_else(|| format!("#{item}")));
        }
    }
    s.warps.accounts = vec![UID];
    s.warps.summaries.insert(UID, summary);
}

/// An account that's never been synced and has nothing kept
pub(super) fn empty_summary() -> Summary {
    Summary {
        uid: UID,
        pools: [Pool::Character, Pool::LightCone, Pool::Standard]
            .into_iter()
            .map(|pool| pool_stats(pool, &[], &RateUps::new(), |_| None))
            .collect(),
        total: 0,
        unconfirmed: 0,
        requests: 0,
        request_counter: None,
        key: None,
        last_sync: None,
        last_error: None,
        next_look: None,
        names: Default::default(),
        banners: Vec::new(),
    }
}

/// The banners of 4.7's first half, as the game describes them
fn running_banners(now: i64) -> Vec<RunningBanner> {
    let banner = |gacha_id, pool, five, fours: &[u32]| RunningBanner {
        gacha_id,
        pool,
        fives: vec![five],
        fours: fours.to_vec(),
        begin: now - 14 * 86400,
        end: Some(now + 30 * 86400 + 23 * 3600),
    };
    vec![
        banner(2142, Pool::Character, 1511, &[1009, 1110, 1207]),
        banner(2143, Pool::Character, 1506, &[1009, 1110, 1207]),
        banner(2144, Pool::Character, 1501, &[1009, 1110, 1207]),
        banner(3142, Pool::LightCone, 23065, &[21016, 21020, 21047]),
        banner(3143, Pool::LightCone, 23057, &[21016, 21020, 21047]),
        banner(4142, Pool::CollabCharacter, 1014, &[]),
        banner(4143, Pool::CollabCharacter, 1015, &[]),
        banner(5142, Pool::CollabLightCone, 23045, &[]),
    ]
}
