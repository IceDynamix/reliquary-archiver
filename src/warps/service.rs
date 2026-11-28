//! Runs the warp database on a thread of its own: keeps what the capture sees, syncs with the
//! game's history when there's an authkey to do it with, and reports how each account stands.
//!
//! Live pulls only get their ids once the history has them, which can take an hour. While any are
//! waiting and the authkey still works, the service asks the history again every few minutes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use reliquary_archiver::export::database::get_database;
use tracing::{error, info, warn};

use super::api::{self, ApiError, Client, HistoryKey};
use super::reconcile::{LiveKey, group_live, group_requests};
use super::srs::{self, Profile};
use super::stats::{PoolStats, RecentPull, pool_stats, recorded_outcomes};
use super::store::{AccountState, KeyOrigin, Store, StoredKey};
use super::{Pool, Pull, WarpEvent, csv, source};

/// The game's history is filled in an hour at a time, the hour a pull's id names: its pulls show
/// up once that hour is over and the batch that publishes it ran. This is how long after the hour
/// that takes
const PUBLISH_DELAY: i64 = 45 * 60;
/// How often to look again for pulls that are later than that
const LATE_RECHECK: i64 = 10 * 60;
/// How long to keep looking that often, after that it's hourly
const LATE_FOR: i64 = 3 * 3600;
/// How long after it's made the history surely has a pull: published past its hour, and late for
/// as long as it's looked for often. A live pull without an id by then is dropped
const STALE_AFTER: i64 = 3600 + PUBLISH_DELAY + LATE_FOR;
/// How long to wait after a sync failed before trying again
const RETRY_AFTER_FAILURE: i64 = 10 * 60;
/// Where a new authkey comes from, for the notices about an old one
const NEW_KEY_HINT: &str = "Open Warp → View Details in the game for a new one";
/// Authkeys stop working after about a day, there's no use asking with an older one
const KEY_LIFETIME: i64 = 24 * 3600;
/// A banner end this far off (Unix seconds, in 2096) means it doesn't end, as 0 does
const NEVER_ENDS: i64 = 4_000_000_000;

/// Who asked for a sync
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncKind {
    /// The user: looks at every pool and always says how it went
    Asked,
    /// The schedule: looks only at the pools with live pulls waiting, and says nothing when
    /// nothing changed
    Timed,
}

/// When to look in the history next for a live pull made at `pulled`, at `now`, both in Unix seconds
fn next_look(pulled: i64, now: i64) -> i64 {
    let expected = pulled.div_euclid(3600) * 3600 + 3600 + PUBLISH_DELAY;
    if now < expected {
        expected
    } else if now < expected + LATE_FOR {
        now + LATE_RECHECK
    } else {
        now + 3600
    }
}

pub enum Command {
    Event(WarpEvent),
    /// Fetch the account's history now
    Sync(u32),
    /// Look for an authkey in the game's web cache
    FindCachedKey,
    /// An authkey from a link the user pasted
    UseLink(String),
    /// Import a Star Rail Station backup into the account, from the named profile or the one that fits
    ImportSrs {
        path: PathBuf,
        uid: u32,
        profile: Option<String>,
    },
    /// Write the account's whole history to a CSV file
    ExportCsv {
        path: PathBuf,
        uid: u32,
    },
    /// Import the pulls in a CSV file into the account
    ImportCsv {
        path: PathBuf,
        uid: u32,
    },
    /// Report the account
    Show(u32),
}

/// What the service tells the window
#[derive(Clone, Debug)]
pub enum Update {
    /// The database couldn't be opened, nothing is kept this run
    Unavailable(String),
    /// Accounts with warps kept, the latest first
    Accounts(Vec<u32>),
    Summary(Box<Summary>),
    /// What's running, `None` when it's done
    Busy(Option<String>),
    Notice {
        title: String,
        detail: Option<String>,
        tone: Tone,
    },
    /// The backup has several profiles and none clearly is the account
    ChooseProfile {
        path: PathBuf,
        uid: u32,
        profiles: Vec<ProfileChoice>,
    },
}

/// Whether a notice says something went wrong
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Error,
}

impl Tone {
    pub fn is_error(self) -> bool {
        self == Tone::Error
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileChoice {
    pub key: String,
    pub name: String,
    pub pulls: usize,
    /// Unix seconds
    pub latest: Option<i64>,
    /// Its pull ids end in the account's uid
    pub fits: bool,
}

/// How an account's warps stand
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub uid: u32,
    /// Every pool with pulls, and character, light cone and standard always
    pub pools: Vec<PoolStats>,
    pub total: usize,
    /// Live pulls waiting for the history
    pub unconfirmed: usize,
    /// Requests the kept pulls were made in
    pub requests: usize,
    /// The account's warp request counter as of the latest pull seen live
    pub request_counter: Option<u32>,
    pub key: Option<KeyState>,
    pub last_sync: Option<i64>,
    pub last_error: Option<String>,
    /// When the history is next looked at for the live pulls waiting, in Unix seconds
    pub next_look: Option<i64>,
    /// Names of the items the summary mentions
    pub names: HashMap<u32, String>,
    /// The banners running now, each warp's featured one first ([`featured_first`])
    pub banners: Vec<RunningBanner>,
}

/// A banner running now
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningBanner {
    pub gacha_id: u32,
    pub pool: Pool,
    /// Its featured 5 and 4 stars
    pub fives: Vec<u32>,
    pub fours: Vec<u32>,
    /// When it started and when it ends, in Unix seconds; none when it doesn't end
    pub begin: i64,
    pub end: Option<i64>,
}

impl Summary {
    /// Requests the account made that the history doesn't have, from the request counter. The
    /// counter numbers requests from 0, so the latest one seen is one less than the requests made
    pub fn missing_requests(&self) -> Option<usize> {
        let made = self.request_counter? as usize + 1;
        Some(made.saturating_sub(self.requests))
    }

    pub fn pool(&self, pool: Pool) -> Option<&PoolStats> {
        self.pools.iter().find(|p| p.pool == pool)
    }

    /// The banners of a pool running now, the featured one first
    pub fn running(&self, pool: Pool) -> impl Iterator<Item = &RunningBanner> {
        self.banners.iter().filter(move |b| b.pool == pool)
    }

    pub fn name(&self, item_id: u32) -> String {
        self.names.get(&item_id).cloned().unwrap_or_else(|| item_name(item_id))
    }

    /// The pulls seen live from `since` on, in Unix seconds, of those each pool keeps in
    /// [`PoolStats::recent`]
    pub fn session(&self, since: i64) -> impl Iterator<Item = &RecentPull> {
        self.pools
            .iter()
            .flat_map(|stats| &stats.recent)
            .filter(move |recent| recent.pull.sources & source::LIVE != 0 && recent.pull.time >= since)
    }
}

/// What the game's data calls an item, or its id when it doesn't know it
pub fn item_name(item_id: u32) -> String {
    get_database().warp_item_name(item_id).unwrap_or_else(|| format!("#{item_id}"))
}

/// The order running banners are shown in, each warp's featured one first. The game's own list
/// doesn't say which that is. A version's headline banner runs the whole version while each half's
/// reruns end with their half, and among those ending together it has the lowest id: so the one
/// ending last, then the lowest id. Banners that don't end, the collaborations', by the newest
/// collaboration, then the lowest id, as the game lists them
pub fn featured_first(a: &RunningBanner, b: &RunningBanner) -> std::cmp::Ordering {
    match (a.end, b.end) {
        (Some(x), Some(y)) => y.cmp(&x).then(a.gacha_id.cmp(&b.gacha_id)),
        (None, None) => b.begin.cmp(&a.begin).then(a.gacha_id.cmp(&b.gacha_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyState {
    pub origin: KeyOrigin,
    /// Unix seconds
    pub obtained_at: i64,
    pub expired: bool,
}

impl KeyState {
    /// Whether it's worth asking the history with
    pub fn usable(&self, now: i64) -> bool {
        !self.expired && now - self.obtained_at < KEY_LIFETIME
    }
}

/// The handle the rest of the app talks to the service through
#[derive(Clone)]
pub struct WarpService {
    tx: mpsc::Sender<Command>,
}

impl WarpService {
    /// Starts the service on its own thread. `report` hears every [`Update`]
    pub fn start(path: Option<PathBuf>, report: impl Fn(Update) + Send + 'static) -> WarpService {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("warps".into())
            .spawn(move || {
                let Some(path) = path else {
                    error!("there's no folder to keep the warp database in");
                    report(Update::Unavailable("There's no folder to keep it in".into()));
                    return;
                };
                match Store::open(&path) {
                    Ok(store) => {
                        info!("keeping warps in {}", path.display());
                        Worker::new(store, Box::new(report)).run(rx)
                    }
                    Err(e) => {
                        error!(%e, "couldn't open the warp database");
                        report(Update::Unavailable(format!("Couldn't open {}: {e}", path.display())));
                    }
                }
            })
            .expect("couldn't start the warp thread");
        WarpService { tx }
    }

    pub fn send(&self, command: Command) {
        self.tx.send(command).ok();
    }
}

struct Worker {
    store: Store,
    report: Box<dyn Fn(Update) + Send>,
    /// When to look for the account's live pulls in the history next
    due: HashMap<u32, Instant>,
    /// Accounts whose last full sync failed, the next one is full again
    full_pending: HashSet<u32>,
}

impl Worker {
    fn new(store: Store, report: Box<dyn Fn(Update) + Send>) -> Self {
        Worker {
            store,
            report,
            due: HashMap::new(),
            full_pending: HashSet::new(),
        }
    }

    fn run(mut self, rx: mpsc::Receiver<Command>) {
        self.report_accounts();
        // Live pulls left waiting last time get looked for again
        if let Ok(accounts) = self.store.accounts() {
            for uid in accounts {
                self.schedule(uid);
            }
        }

        loop {
            let wait = self
                .due
                .values()
                .min()
                .map_or(Duration::from_secs(3600), |at| at.saturating_duration_since(Instant::now()));
            match rx.recv_timeout(wait) {
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => {
                    let now = Instant::now();
                    let due: Vec<u32> = self.due.iter().filter(|(_, at)| **at <= now).map(|(uid, _)| *uid).collect();
                    for uid in due {
                        self.due.remove(&uid);
                        self.sync(uid, SyncKind::Timed);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn handle(&mut self, command: Command) {
        // The app shows these as busy from the moment it sends them, so however they end (an
        // expired key, no key in the cache, a link without one) it has to hear they're done
        let ends_busy = matches!(command, Command::Sync(_) | Command::FindCachedKey | Command::UseLink(_));
        self.handle_command(command);
        if ends_busy {
            self.busy(None);
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Event(WarpEvent::Pulled(request)) => {
                let db = get_database();
                match self.store.record_live(&request, |item| db.warp_item_rarity(item).unwrap_or(3)) {
                    Ok(true) => {
                        info!(
                            uid = request.uid,
                            gacha_id = request.gacha_id,
                            pulls = request.items.len(),
                            "kept a warp"
                        );
                        self.schedule(request.uid);
                        self.report_accounts();
                        self.show(request.uid);
                    }
                    Ok(false) => {}
                    Err(e) => error!(%e, "couldn't keep a warp"),
                }
            }
            Command::Event(WarpEvent::Authkey { uid, authkey }) => {
                info!(uid, "the game handed out a warp history authkey");
                let key = HistoryKey::for_account(uid, authkey);
                self.use_key(uid, &key, KeyOrigin::Capture);
            }
            Command::Event(WarpEvent::Banners { banners }) => {
                if let Err(e) = self.store.save_banners(&banners) {
                    warn!(%e, "couldn't keep the banners");
                }
            }
            Command::Sync(uid) => self.sync(uid, SyncKind::Asked),
            Command::FindCachedKey => match super::authkey::from_game_cache() {
                Ok(key) => self.adopt_key(key, KeyOrigin::Cache),
                Err(e) => self.notice("No authkey found", Some(e), Tone::Error),
            },
            Command::UseLink(link) => match super::authkey::from_url(&link) {
                Some(key) => self.adopt_key(key, KeyOrigin::Link),
                None => self.notice(
                    "That isn't a warp history link",
                    Some("It needs to have an authkey in it".into()),
                    Tone::Error,
                ),
            },
            Command::ImportSrs { path, uid, profile } => self.import_srs(path, uid, profile),
            Command::ExportCsv { path, uid } => self.export_csv(&path, uid),
            Command::ImportCsv { path, uid } => self.import_csv(&path, uid),
            Command::Show(uid) => self.show(uid),
        }
    }

    /// A key whose account isn't known yet: find out whose it is from the history
    fn adopt_key(&mut self, key: HistoryKey, origin: KeyOrigin) {
        self.busy(Some("Checking the authkey…".into()));
        let result = api::probe(&key);
        self.busy(None);
        match result {
            Ok(Some(uid)) => self.use_key(uid, &key, origin),
            Ok(None) => self.notice("No warps on this account", None, Tone::Info),
            Err(ApiError::Expired) => self.notice("That authkey has expired", Some(NEW_KEY_HINT.into()), Tone::Error),
            Err(e) => self.notice("The authkey didn't work", Some(e.to_string()), Tone::Error),
        }
    }

    fn use_key(&mut self, uid: u32, key: &HistoryKey, origin: KeyOrigin) {
        if let Err(e) = self.store.set_authkey(uid, key, origin, now()) {
            error!(%e, "couldn't keep the authkey");
            return;
        }
        self.report_accounts();
        self.sync(uid, SyncKind::Asked);
    }

    /// Plans the next look for the account's live pulls: once the hour of the oldest one waiting
    /// has been published, or not at all when none are waiting or the key no longer works
    fn schedule(&mut self, uid: u32) {
        let now = now();
        match next_look_for(&self.store, uid, now) {
            Ok(Some(at)) => {
                info!(uid, in_minutes = (at - now) / 60, "looking for the live pulls in the history later");
                self.due.insert(uid, Instant::now() + Duration::from_secs((at - now).max(5) as u64));
            }
            Ok(None) => {
                self.due.remove(&uid);
            }
            Err(e) => {
                warn!(%e, uid, "couldn't plan when to look for the live pulls");
                self.due.remove(&uid);
            }
        }
    }

    /// Tries a failed sync again in a while, as long as the key works
    fn retry_later(&mut self, uid: u32) -> bool {
        if !matches!(key_usable(&self.store, uid, now()), Ok(true)) {
            return false;
        }
        let at = Instant::now() + Duration::from_secs(RETRY_AFTER_FAILURE as u64);
        let due = self.due.entry(uid).or_insert(at);
        *due = (*due).min(at);
        true
    }

    /// Brings the account up to date with the game's history
    fn sync(&mut self, uid: u32, kind: SyncKind) {
        let quiet = kind == SyncKind::Timed;
        let key = match self.store.authkey(uid) {
            Ok(Some(key)) if !key.expired => key,
            _ => {
                if !quiet {
                    self.notice(
                        "No authkey to sync with",
                        Some("Open Warp → View Details in the game while the archiver runs".into()),
                        Tone::Error,
                    );
                }
                return;
            }
        };
        let full = !quiet || self.full_pending.contains(&uid);
        let pools: Vec<Pool> = if full {
            Pool::ALL.to_vec()
        } else {
            self.store.unconfirmed_pools(uid).unwrap_or_else(|e| {
                warn!(%e, uid, "couldn't read which pools have live pulls waiting");
                Vec::new()
            })
        };
        if pools.is_empty() {
            self.schedule(uid);
            return;
        }

        let (fetched, failure) = self.fetch(uid, key.key, &pools, quiet);

        if let Some(other) = fetched.iter().find(|pull| pull.uid != uid) {
            warn!(uid, other = other.uid, "the authkey is another account's");
            self.invalidate_key(uid);
            let detail = format!("It belongs to UID {}", other.uid);
            self.notice("That authkey belongs to another account", Some(detail), Tone::Error);
            self.report_accounts();
            self.show(uid);
            return;
        }

        match failure {
            Some(error) => self.sync_failed(uid, error, &fetched, full, quiet),
            None => self.sync_succeeded(uid, &fetched, &pools, full, quiet),
        }
    }

    /// Fetches the pools' new pulls, up to the first pool that fails, and why it failed. `quiet`
    /// leaves the busy state alone: a timed sync didn't set it, so it isn't its to clear
    fn fetch(&self, uid: u32, key: HistoryKey, pools: &[Pool], quiet: bool) -> (Vec<Pull>, Option<ApiError>) {
        let known = self.store.history_ids(uid).unwrap_or_else(|e| {
            warn!(%e, uid, "couldn't read the pulls already synced, fetching them all");
            HashSet::new()
        });
        let client = Client::new(key);
        let mut fetched: Vec<Pull> = Vec::new();
        let mut failure = None;
        for (i, &pool) in pools.iter().enumerate() {
            if i > 0 {
                client.pause();
            }
            let label = pool.label();
            let before = fetched.len();
            let result = client.pool(
                pool,
                |id| known.contains(&id),
                |count| {
                    if !quiet {
                        (self.report)(Update::Busy(Some(format!("Syncing {label} warps… {}", before + count))));
                    }
                },
            );
            match result {
                Ok(pulls) => fetched.extend(pulls),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if !quiet {
            self.busy(None);
        }
        (fetched, failure)
    }

    fn sync_failed(&mut self, uid: u32, error: ApiError, fetched: &[Pull], full: bool, quiet: bool) {
        warn!(uid, %error, "couldn't sync warps");
        // The pools fetched before it failed are whole, keep them. The one it failed in isn't:
        // keeping its newest pulls would let the next sync stop at them, short of the rest
        if !fetched.is_empty() {
            match self.store.merge_history(uid, fetched, source::API) {
                Ok(merged) => info!(
                    uid,
                    added = merged.added,
                    confirmed = merged.confirmed,
                    "kept what synced before the failure"
                ),
                Err(e) => error!(%e, "couldn't keep the synced warps"),
            }
        }
        self.mark_synced(uid, Some(&error.to_string()));
        match error {
            ApiError::Expired => {
                self.invalidate_key(uid);
                self.notice("The authkey has expired", Some(NEW_KEY_HINT.into()), Tone::Error);
            }
            ApiError::Rejected(_) => {
                self.invalidate_key(uid);
                self.notice("The authkey was refused", Some(error.to_string()), Tone::Error);
            }
            _ => {
                // Slowed down or offline: the key still works, try again in a while
                if full {
                    self.full_pending.insert(uid);
                }
                let retry = self.retry_later(uid);
                if !quiet {
                    let detail = if retry {
                        format!("{error}. What synced so far is kept, the rest is tried again in a few minutes.")
                    } else {
                        error.to_string()
                    };
                    self.notice("Couldn't sync warps", Some(detail), Tone::Error);
                }
            }
        }
        self.show(uid);
    }

    fn sync_succeeded(&mut self, uid: u32, fetched: &[Pull], pools: &[Pool], full: bool, quiet: bool) {
        let merged = self.store.merge_history(uid, fetched, source::API);
        match self.store.drop_stale_live(uid, pools, now() - STALE_AFTER) {
            Ok(0) => {}
            Ok(dropped) => warn!(uid, dropped, "dropped live pulls the history never recognized"),
            Err(e) => warn!(%e, uid, "couldn't drop stale live pulls"),
        }
        self.mark_synced(uid, None);
        if full {
            self.full_pending.remove(&uid);
        }
        let waiting = self.store.unconfirmed(uid).unwrap_or(0);
        self.schedule(uid);
        match merged {
            Ok(merged) => {
                info!(uid, added = merged.added, confirmed = merged.confirmed, waiting, "synced warps");
                if !quiet || merged.added + merged.confirmed > 0 {
                    let mut parts = Vec::new();
                    if merged.added > 0 {
                        parts.push(format!("{} new pulls", merged.added));
                    }
                    if merged.confirmed > 0 {
                        parts.push(format!("{} live pulls confirmed", merged.confirmed));
                    }
                    if waiting > 0 {
                        parts.push(format!("{waiting} still waiting for the history"));
                    }
                    let detail = if parts.is_empty() {
                        "Already up to date".to_string()
                    } else {
                        parts.join(", ")
                    };
                    self.notice("Warps synced", Some(detail), Tone::Info);
                }
            }
            Err(e) => {
                error!(%e, "couldn't keep the synced warps");
                self.notice("Couldn't keep the synced warps", Some(e.to_string()), Tone::Error);
            }
        }
        self.report_accounts();
        self.show(uid);
    }

    fn mark_synced(&mut self, uid: u32, error: Option<&str>) {
        if let Err(e) = self.store.set_synced(uid, now(), error) {
            warn!(%e, uid, "couldn't note the sync");
        }
    }

    /// Stops using the account's key: it expired, was refused, or is another account's
    fn invalidate_key(&mut self, uid: u32) {
        if let Err(e) = self.store.expire_authkey(uid) {
            warn!(%e, uid, "couldn't mark the authkey expired");
        }
        self.due.remove(&uid);
        self.full_pending.remove(&uid);
    }

    fn import_srs(&mut self, path: PathBuf, uid: u32, profile: Option<String>) {
        self.busy(Some("Reading the backup…".into()));
        let profiles = srs::read(&path);
        self.busy(None);
        let profiles = match profiles {
            Ok(profiles) if !profiles.is_empty() => profiles,
            Ok(_) => return self.notice("The backup has no warps in it", None, Tone::Error),
            Err(e) => return self.notice("Couldn't import that file", Some(e), Tone::Error),
        };

        let chosen: Option<&Profile> = match &profile {
            Some(key) => profiles.iter().find(|p| &p.key == key),
            None => {
                let fitting: Vec<&Profile> = profiles.iter().filter(|p| p.fits(uid)).collect();
                match fitting.as_slice() {
                    [only] => Some(*only),
                    _ => None,
                }
            }
        };
        let Some(chosen) = chosen else {
            let profiles = profiles
                .iter()
                .map(|p| ProfileChoice {
                    key: p.key.clone(),
                    name: p.name.clone(),
                    pulls: p.pulls.len(),
                    latest: p.latest(),
                    fits: p.fits(uid),
                })
                .collect();
            (self.report)(Update::ChooseProfile { path, uid, profiles });
            return;
        };
        if !chosen.fits(uid) {
            let detail = format!("Pull IDs don't match UID {uid}");
            return self.notice("That profile is another account's", Some(detail), Tone::Error);
        }

        let merged = self.store.merge_history(uid, &chosen.pulls, source::SRS);
        if let Err(e) = self.store.add_rate_ups(&chosen.rate_ups) {
            warn!(%e, "couldn't keep what the backup tells about banners");
        }
        match merged {
            Ok(merged) => {
                let known = chosen.pulls.len() - merged.added - merged.confirmed;
                let detail = format!("{} new pulls, {known} already kept", merged.added + merged.confirmed);
                info!(
                    uid,
                    profile = chosen.key,
                    added = merged.added,
                    "imported a Star Rail Station backup"
                );
                self.notice("Imported from Star Rail Station", Some(detail), Tone::Info);
            }
            Err(e) => self.notice("Couldn't import the backup", Some(e.to_string()), Tone::Error),
        }
        self.report_accounts();
        self.show(uid);
    }

    fn export_csv(&mut self, path: &std::path::Path, uid: u32) {
        let mut pulls = match self.store.pulls(uid) {
            Ok(pulls) => pulls,
            Err(e) => return self.notice("Couldn't read the warps to export", Some(e.to_string()), Tone::Error),
        };
        pulls.sort_by_key(Pull::sort_key);
        let db = get_database();
        let rate_ups = self.store.rate_ups().unwrap_or_else(|e| {
            warn!(%e, "couldn't read what banners feature, exporting without outcomes");
            Default::default()
        });
        let complete = self.store.account(uid).is_ok_and(|account| history_complete(&account, &pulls));
        let outcomes = recorded_outcomes(&pulls, &rate_ups, complete, |item| db.warp_item_rarity(item));
        let text = csv::write(&pulls, &outcomes, |item| db.warp_item_name(item));
        match std::fs::write(path, text) {
            Ok(()) => {
                info!(uid, pulls = pulls.len(), "exported the warps as CSV");
                let detail = format!("{} pulls to {}", pulls.len(), path.display());
                self.notice("Exported the warp history", Some(detail), Tone::Info);
            }
            Err(e) => self.notice("Couldn't write the CSV", Some(format!("{}: {e}", path.display())), Tone::Error),
        }
    }

    fn import_csv(&mut self, path: &std::path::Path, uid: u32) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => return self.notice("Couldn't import that file", Some(format!("{}: {e}", path.display())), Tone::Error),
        };
        let db = get_database();
        let read = match csv::read(&text, |item| db.warp_item_rarity(item)) {
            Ok(read) => read,
            Err(e) => {
                return self.notice(
                    "Couldn't import that file",
                    Some(format!("It isn't a warp history: {e}")),
                    Tone::Error,
                );
            }
        };
        if read.pulls.is_empty() {
            return self.notice("The file has no warps with ids in it", None, Tone::Error);
        }
        // Ids end in the account's last two digits, a file of another account's pulls is turned away
        if let Some(pull) = read.pulls.iter().find(|p| !super::id_belongs_to(p.id.unwrap_or_default(), uid)) {
            let detail = format!("Pull {} doesn't match UID {uid}", pull.id.unwrap_or_default());
            return self.notice("Those are another account's warps", Some(detail), Tone::Error);
        }
        let merged = self.store.merge_history(uid, &read.pulls, source::CSV);
        if let Err(e) = self.store.add_rate_ups(&read.rate_ups) {
            warn!(%e, "couldn't keep what the CSV tells about banners");
        }
        match merged {
            Ok(merged) => {
                let known = read.pulls.len() - merged.added - merged.confirmed;
                let mut detail = format!("{} new pulls, {known} already kept", merged.added + merged.confirmed);
                if read.without_id > 0 {
                    detail.push_str(&format!(
                        ", {} without an id skipped (the game's history fills those in)",
                        read.without_id
                    ));
                }
                info!(uid, added = merged.added, "imported warps from a CSV");
                self.notice("Imported the warp history", Some(detail), Tone::Info);
            }
            Err(e) => self.notice("Couldn't import the CSV", Some(e.to_string()), Tone::Error),
        }
        self.report_accounts();
        self.show(uid);
    }

    fn show(&self, uid: u32) {
        match summary(&self.store, uid, now()) {
            Ok(summary) => (self.report)(Update::Summary(Box::new(summary))),
            Err(e) => error!(%e, "couldn't read the warps"),
        }
    }

    fn report_accounts(&self) {
        if let Ok(accounts) = self.store.accounts() {
            (self.report)(Update::Accounts(accounts));
        }
    }

    fn busy(&self, what: Option<String>) {
        (self.report)(Update::Busy(what));
    }

    fn notice(&self, title: &str, detail: Option<String>, tone: Tone) {
        (self.report)(Update::Notice {
            title: title.into(),
            detail,
            tone,
        });
    }
}

/// The requests the account's pulls were made in: the history's, and the live ones still waiting
fn requests_kept(pulls: &[Pull]) -> usize {
    let waiting = group_live(
        pulls
            .iter()
            .filter(|p| p.id.is_none())
            .map(|p| LiveKey::new(p.gacha_id, p.time, p.request_no)),
    );
    group_requests(pulls).len() + waiting.len()
}

/// Whether the pulls reach back to the account's first warp: as many requests as its counter says
/// it made
fn history_complete(account: &AccountState, pulls: &[Pull]) -> bool {
    account
        .request_counter
        .is_some_and(|counter| requests_kept(pulls) > counter as usize)
}

/// How the account stands at `now`, in Unix seconds
fn summary(store: &Store, uid: u32, now: i64) -> rusqlite::Result<Summary> {
    let pulls = store.pulls(uid)?;
    let rate_ups = store.rate_ups()?;
    let refs: Vec<&Pull> = pulls.iter().collect();
    let db = get_database();
    let account = store.account(uid)?;
    let requests = requests_kept(&pulls);
    let complete = history_complete(&account, &pulls);

    let pools: Vec<PoolStats> = Pool::ALL
        .iter()
        .filter(|&&pool| matches!(pool, Pool::Character | Pool::LightCone | Pool::Standard) || pulls.iter().any(|p| p.pool == pool))
        .map(|&pool| pool_stats(pool, &refs, &rate_ups, complete, |item| db.warp_item_rarity(item)))
        .collect();

    let mut names = HashMap::new();
    for stats in &pools {
        let items = stats
            .fives
            .iter()
            .map(|h| h.item_id)
            .chain(stats.recent.iter().map(|recent| recent.pull.item_id));
        for item in items {
            names.entry(item).or_insert_with(|| item_name(item));
        }
    }

    // The banners running now, the standard one aside: it features its whole pool
    let mut banners: Vec<RunningBanner> = store
        .running_banners(now)?
        .into_iter()
        .filter_map(|banner| {
            let pool = Pool::from_gacha_id(banner.gacha_id)?;
            if !pool.has_rate_up() {
                return None;
            }
            let rarity = |r: u8| {
                banner
                    .rate_up
                    .iter()
                    .copied()
                    .filter(move |&item| db.warp_item_rarity(item) == Some(r))
            };
            Some(RunningBanner {
                gacha_id: banner.gacha_id,
                pool,
                fives: rarity(5).collect(),
                fours: rarity(4).collect(),
                begin: banner.begin,
                end: (banner.end > 0 && banner.end < NEVER_ENDS).then_some(banner.end),
            })
        })
        .filter(|banner| !banner.fives.is_empty())
        .collect();
    banners.sort_by(featured_first);
    for &item in banners.iter().flat_map(|b| b.fives.iter().chain(&b.fours)) {
        names.entry(item).or_insert_with(|| item_name(item));
    }

    let key = store.authkey(uid)?.map(|key| key_state(&key));
    Ok(Summary {
        next_look: next_look_for(store, uid, now)?,
        uid,
        total: pulls.len(),
        unconfirmed: pulls.iter().filter(|p| p.id.is_none()).count(),
        requests,
        request_counter: account.request_counter,
        key,
        last_sync: account.last_sync,
        last_error: account.last_error,
        pools,
        names,
        banners,
    })
}

/// When to look in the history next for the account's live pulls, at `now`: none when none are
/// waiting or the key no longer works
fn next_look_for(store: &Store, uid: u32, now: i64) -> rusqlite::Result<Option<i64>> {
    let Some(pulled) = store.oldest_unconfirmed(uid)? else {
        return Ok(None);
    };
    Ok(key_usable(store, uid, now)?.then(|| next_look(pulled, now)))
}

/// Whether the account's key is worth asking the history with at `now`
fn key_usable(store: &Store, uid: u32, now: i64) -> rusqlite::Result<bool> {
    Ok(store.authkey(uid)?.is_some_and(|key| key_state(&key).usable(now)))
}

/// What the window would show for an account of the database at `path`, for snapshots of real data
pub fn summary_of(path: &std::path::Path, uid: Option<u32>) -> Option<Summary> {
    let store = Store::open(path).ok()?;
    let uid = uid.or_else(|| store.accounts().ok()?.first().copied())?;
    summary(&store, uid, now()).ok()
}

fn key_state(key: &StoredKey) -> KeyState {
    KeyState {
        origin: key.origin,
        obtained_at: key.obtained_at,
        expired: key.expired,
    }
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    #[test]
    fn the_version_headline_is_featured_through_both_halves() {
        use super::{RunningBanner, featured_first};
        use crate::warps::Pool;
        // 4.7 as the game describes it: Aeon Aha (2142) for the whole version, three reruns each
        // half, and the collaborations', which don't end
        let (half, version) = (Some(1_796_230_740), Some(1_797_969_600));
        let (first, second, collab, collab2) = (1_794_349_800, 1_796_230_800, 1_752_253_200, 1_784_912_400);
        let banner = |gacha_id, pool, five, begin, end| RunningBanner {
            gacha_id,
            pool,
            fives: vec![five],
            fours: Vec::new(),
            begin,
            end,
        };
        let featured = |mut banners: Vec<RunningBanner>, pool| {
            banners.sort_by(featured_first);
            banners.into_iter().find(|b| b.pool == pool).map(|b| b.gacha_id)
        };
        let first_half = vec![
            banner(2145, Pool::Character, 1502, first, half),
            banner(2144, Pool::Character, 1501, first, half),
            banner(2143, Pool::Character, 1506, first, half),
            banner(2142, Pool::Character, 1511, first, version),
            banner(3145, Pool::LightCone, 23054, first, half),
            banner(3142, Pool::LightCone, 23065, first, version),
        ];
        assert_eq!(featured(first_half.clone(), Pool::Character), Some(2142));
        assert_eq!(featured(first_half, Pool::LightCone), Some(3142));
        let second_half = vec![
            banner(2142, Pool::Character, 1511, first, version),
            banner(2146, Pool::Character, 1308, second, version),
            banner(2147, Pool::Character, 1213, second, version),
            banner(2148, Pool::Character, 1204, second, version),
        ];
        assert_eq!(featured(second_half, Pool::Character), Some(2142));
        let collaborations = vec![
            banner(5001, Pool::CollabCharacter, 1014, collab, None),
            banner(5002, Pool::CollabCharacter, 1015, collab, None),
            banner(5004, Pool::CollabCharacter, 1509, collab2, None),
            banner(5003, Pool::CollabCharacter, 1508, collab2, None),
        ];
        // The newest collaboration's, as the game lists them: Rin Tohsaka before Gilgamesh
        assert_eq!(featured(collaborations, Pool::CollabCharacter), Some(5003));
    }

    #[test]
    fn live_pulls_are_looked_for_once_their_hour_is_published() {
        // A pull at 08:05 is in the 08:00 hour, published 09:45
        let pulled = 8 * 3600 + 5 * 60;
        assert_eq!(super::next_look(pulled, pulled + 60), 9 * 3600 + 45 * 60);
        // Late: every ten minutes for three hours, then hourly
        assert_eq!(super::next_look(pulled, 10 * 3600), 10 * 3600 + 600);
        assert_eq!(super::next_look(pulled, 13 * 3600), 14 * 3600);
    }

    use super::*;
    use crate::warps::LiveRequest;
    use crate::warps::stats::{Featured, RateUps};

    #[test]
    fn a_sync_that_cant_start_still_ends_busy() {
        let busy = Arc::new(Mutex::new(Vec::new()));
        let heard = busy.clone();
        let mut worker = Worker::new(
            Store::open_in_memory().unwrap(),
            Box::new(move |update| {
                if let Update::Busy(what) = update {
                    heard.lock().unwrap().push(what);
                }
            }),
        );
        // No authkey, as after one expired: the sync stops before it asks the history
        worker.handle(Command::Sync(801234567));
        worker.handle(Command::UseLink("https://example.com/no-key-here".into()));
        assert_eq!(*busy.lock().unwrap(), vec![None, None]);
    }

    #[test]
    fn an_exported_csv_imports_into_another_database_and_again_adds_nothing() {
        const UID: u32 = 801234567;
        let notices = Arc::new(Mutex::new(Vec::new()));
        let worker = |notices: &Arc<Mutex<Vec<(String, bool)>>>| {
            let heard = notices.clone();
            Worker::new(
                Store::open_in_memory().unwrap(),
                Box::new(move |update| {
                    if let Update::Notice { title, detail, tone } = update {
                        heard
                            .lock()
                            .unwrap()
                            .push((format!("{title}: {}", detail.unwrap_or_default()), tone.is_error()));
                    }
                }),
            )
        };
        let pull = |n: i64, item_id: u32, rarity: u8| Pull {
            id: Some(1_700_000_000_000_000_067 + n * 100),
            uid: UID,
            pool: Pool::Character,
            gacha_id: 2142,
            item_id,
            rarity,
            time: 1_700_000_000 + n,
            request_no: None,
            batch_index: 0,
            sources: source::API,
        };
        let mut from = worker(&notices);
        from.store
            .merge_history(UID, &[pull(0, 20000, 3), pull(1, 1006, 5)], source::API)
            .unwrap();
        let path = std::env::temp_dir().join(format!("rqa-warps-{}.csv", std::process::id()));
        from.handle(Command::ExportCsv {
            path: path.clone(),
            uid: UID,
        });

        let mut into = worker(&notices);
        into.handle(Command::ImportCsv {
            path: path.clone(),
            uid: UID,
        });
        into.handle(Command::ImportCsv {
            path: path.clone(),
            uid: UID,
        });
        // Another account's warps are turned away
        into.handle(Command::ImportCsv {
            path: path.clone(),
            uid: UID + 1,
        });
        let _ = std::fs::remove_file(&path);

        let mut kept = into.store.pulls(UID).unwrap();
        kept.sort_by_key(Pull::sort_key);
        assert_eq!(kept.iter().map(|p| (p.id, p.item_id, p.rarity, p.time)).collect::<Vec<_>>(), [
            (pull(0, 0, 0).id, 20000, 3, 1_700_000_000),
            (pull(1, 0, 0).id, 1006, 5, 1_700_000_001)
        ]);
        assert!(kept.iter().all(|p| p.sources == source::CSV));
        let notices = notices.lock().unwrap();
        let said: Vec<&str> = notices.iter().map(|(n, _)| n.as_str()).collect();
        assert!(said[0].starts_with("Exported the warp history: 2 pulls to "));
        assert_eq!(said[1..3], [
            "Imported the warp history: 2 new pulls, 0 already kept",
            "Imported the warp history: 0 new pulls, 2 already kept"
        ]);
        assert!(notices[3].1 && notices[3].0.starts_with("Those are another account's warps"));
    }

    #[test]
    fn outcomes_known_from_banners_survive_a_csv_round_trip_and_guesses_do_not() {
        const UID: u32 = 801234567;
        let pull = |n: i64, gacha_id: u32, item_id: u32, rarity: u8| Pull {
            id: Some(1_700_000_000_000_000_067 + n * 100),
            uid: UID,
            pool: Pool::Character,
            gacha_id,
            item_id,
            rarity,
            time: 1_700_000_000 + n,
            request_no: None,
            batch_index: 0,
            sources: source::API,
        };
        let mut from = Worker::new(Store::open_in_memory().unwrap(), Box::new(|_| {}));
        let pulls = [
            pull(0, 2142, 1003, 5),
            pull(1, 2142, 1105, 4),
            pull(2, 2142, 1006, 5),
            pull(3, 2142, 20000, 3),
            pull(4, 2143, 1102, 5),
        ];
        from.store.merge_history(UID, &pulls, source::API).unwrap();
        // The history reaches back to the account's first warp, so its first 50/50s are known
        from.store.set_request_counter(UID, 0).unwrap();
        let mut known = RateUps::new();
        known.insert(2142, Featured {
            items: [1006, 1105].into_iter().collect(),
            not: [1003].into_iter().collect(),
        });
        from.store.add_rate_ups(&known).unwrap();
        let path = std::env::temp_dir().join(format!("rqa-warps-outcomes-{}.csv", std::process::id()));
        from.handle(Command::ExportCsv {
            path: path.clone(),
            uid: UID,
        });
        let text = std::fs::read_to_string(&path).unwrap();
        let results: Vec<&str> = text.lines().skip(1).map(|row| row.rsplit(',').next().unwrap()).collect();
        // 1102 is only taken for featured by not being a standard 5 star
        assert_eq!(results, ["lost", "won", "guaranteed", "", ""]);

        let mut into = Worker::new(Store::open_in_memory().unwrap(), Box::new(|_| {}));
        into.handle(Command::ImportCsv {
            path: path.clone(),
            uid: UID,
        });
        let _ = std::fs::remove_file(&path);
        assert_eq!(into.store.rate_ups().unwrap(), known);
        let outcomes = |store: &Store| {
            let mut pulls = store.pulls(UID).unwrap();
            pulls.sort_by_key(Pull::sort_key);
            let refs: Vec<&Pull> = pulls.iter().collect();
            let db = get_database();
            let stats = pool_stats(Pool::Character, &refs, &store.rate_ups().unwrap(), true, |item| {
                db.warp_item_rarity(item)
            });
            stats.recent.iter().map(|r| r.outcome).collect::<Vec<_>>()
        };
        assert_eq!(outcomes(&into.store), outcomes(&from.store));
    }

    #[test]
    fn live_requests_without_a_counter_count_apart_on_different_banners() {
        const UID: u32 = 801234567;
        let mut store = Store::open_in_memory().unwrap();
        for (gacha_id, items) in [(1001, vec![20000]), (2142, vec![20000, 20001])] {
            let request = LiveRequest {
                uid: UID,
                gacha_id,
                request_no: None,
                items,
                captured_at: 1_700_000_000_000,
            };
            store.record_live(&request, |_| 3).unwrap();
        }
        let summary = summary(&store, UID, 1_700_000_100).unwrap();
        assert_eq!((summary.total, summary.requests), (3, 2));
        assert_eq!(summary.session(1_700_000_000).count(), 3);
        assert_eq!(summary.session(1_700_000_001).count(), 0);
    }
}
