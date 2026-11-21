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
use super::reconcile::group_requests;
use super::srs::{self, Profile};
use super::stats::{PoolStats, pool_stats};
use super::store::{Store, StoredKey};
use super::{Pool, Pull, WarpEvent, source};

/// The game's history is filled in an hour at a time, the hour a pull's id names: its pulls show
/// up once that hour is over and the batch that publishes it ran. This is how long after the hour
/// that takes
const PUBLISH_DELAY: i64 = 45 * 60;
/// How often to look again for pulls that are later than that
const LATE_RECHECK: i64 = 10 * 60;
/// How long to keep looking that often, after that it's hourly
const LATE_FOR: i64 = 3 * 3600;
/// How long to wait after a sync failed before trying again
const RETRY_AFTER_FAILURE: i64 = 10 * 60;
/// Authkeys stop working after about a day, there's no use asking with an older one
const KEY_LIFETIME: i64 = 24 * 3600;

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
        error: bool,
    },
    /// The backup has several profiles and none clearly is the account
    ChooseProfile {
        path: PathBuf,
        uid: u32,
        profiles: Vec<ProfileChoice>,
    },
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
    /// The banners running now, newest first
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
    /// When it ends, in Unix seconds, none when it doesn't
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

    /// The banners of a pool running now, newest first
    pub fn running(&self, pool: Pool) -> impl Iterator<Item = &RunningBanner> {
        self.banners.iter().filter(move |b| b.pool == pool)
    }

    pub fn name(&self, item_id: u32) -> String {
        self.names.get(&item_id).cloned().unwrap_or_else(|| format!("#{item_id}"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyState {
    /// capture, cache or link
    pub origin: String,
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
                let store = match &path {
                    Some(path) => Store::open(path),
                    None => Err(rusqlite::Error::InvalidPath(PathBuf::new())),
                };
                match store {
                    Ok(store) => {
                        if let Some(path) = &path {
                            info!("keeping warps in {}", path.display());
                        }
                        Worker::new(store, Box::new(report)).run(rx)
                    }
                    Err(e) => {
                        error!(%e, "couldn't open the warp database");
                        report(Update::Unavailable(match path {
                            Some(path) => format!("Couldn't open {}: {e}", path.display()),
                            None => "There's no folder to keep it in".into(),
                        }));
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
                        self.sync(uid, true);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn handle(&mut self, command: Command) {
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
                self.use_key(uid, &key, "capture");
            }
            Command::Event(WarpEvent::Banners { banners }) => {
                if let Err(e) = self.store.save_banners(&banners) {
                    warn!(%e, "couldn't keep the banners");
                }
            }
            Command::Sync(uid) => self.sync(uid, false),
            Command::FindCachedKey => match super::authkey::from_game_cache() {
                Ok(key) => self.adopt_key(key, "cache"),
                Err(e) => self.notice("No authkey found", Some(e), true),
            },
            Command::UseLink(link) => match super::authkey::from_url(&link) {
                Some(key) => self.adopt_key(key, "link"),
                None => self.notice(
                    "That isn't a warp history link",
                    Some("It needs to have an authkey in it".into()),
                    true,
                ),
            },
            Command::ImportSrs { path, uid, profile } => self.import_srs(path, uid, profile),
            Command::Show(uid) => self.show(uid),
        }
    }

    /// A key whose account isn't known yet: find out whose it is from the history
    fn adopt_key(&mut self, key: HistoryKey, origin: &str) {
        self.busy(Some("Checking the authkey…".into()));
        let result = api::probe(&key);
        self.busy(None);
        match result {
            Ok(Some(uid)) => self.use_key(uid, &key, origin),
            Ok(None) => self.notice("The authkey works, but its account has no warps", None, false),
            Err(ApiError::Expired) => self.notice(
                "That authkey has expired",
                Some("Open Warp → View Details in the game for a new one".into()),
                true,
            ),
            Err(e) => self.notice("The authkey didn't work", Some(e.to_string()), true),
        }
    }

    fn use_key(&mut self, uid: u32, key: &HistoryKey, origin: &str) {
        if let Err(e) = self.store.set_authkey(uid, key, origin, now()) {
            error!(%e, "couldn't keep the authkey");
            return;
        }
        self.report_accounts();
        self.sync(uid, false);
    }

    /// Plans the next look for the account's live pulls: once the hour of the oldest one waiting
    /// has been published, or not at all when none are waiting or the key no longer works
    fn schedule(&mut self, uid: u32) {
        let oldest = self.store.oldest_unconfirmed(uid).ok().flatten();
        match oldest {
            Some(pulled) if self.key_usable(uid) => {
                let now = now();
                let at = next_look(pulled, now);
                info!(uid, in_minutes = (at - now) / 60, "looking for the live pulls in the history later");
                self.due.insert(uid, Instant::now() + Duration::from_secs((at - now).max(5) as u64));
            }
            _ => {
                self.due.remove(&uid);
            }
        }
    }

    /// Tries a failed sync again in a while, as long as the key works
    fn retry_later(&mut self, uid: u32) -> bool {
        if !self.key_usable(uid) {
            return false;
        }
        let at = Instant::now() + Duration::from_secs(RETRY_AFTER_FAILURE as u64);
        let due = self.due.entry(uid).or_insert(at);
        *due = (*due).min(at);
        true
    }

    fn key_usable(&self, uid: u32) -> bool {
        self.store
            .authkey(uid)
            .ok()
            .flatten()
            .is_some_and(|key| key_state(&key).usable(now()))
    }

    /// Brings the account up to date with the game's history. `quiet` is for the syncs that run by
    /// themselves: they only look at the pools with live pulls waiting, and say nothing when
    /// nothing changed
    fn sync(&mut self, uid: u32, quiet: bool) {
        let key = match self.store.authkey(uid) {
            Ok(Some(key)) if !key.expired => key,
            _ => {
                if !quiet {
                    self.notice(
                        "No authkey to sync with",
                        Some("Open Warp → View Details in the game while the archiver runs".into()),
                        true,
                    );
                }
                return;
            }
        };
        let full = !quiet || self.full_pending.contains(&uid);
        let pools: Vec<Pool> = if full {
            Pool::ALL.to_vec()
        } else {
            self.store.unconfirmed_pools(uid).unwrap_or_default()
        };
        if pools.is_empty() {
            self.schedule(uid);
            return;
        }
        let known = self.store.history_ids(uid).unwrap_or_default();
        let client = Client::new(key.key.clone());

        let mut fetched: Vec<Pull> = Vec::new();
        let mut failure = None;
        for (i, pool) in pools.into_iter().enumerate() {
            if i > 0 {
                client.pause();
            }
            let label = pool.label();
            let before = fetched.len();
            let result = client.pool(
                pool,
                |id| known.contains(&id),
                |count| {
                    (self.report)(Update::Busy(Some(format!("Syncing {label} warps… {}", before + count))));
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
        self.busy(None);

        if let Some(error) = failure {
            warn!(uid, %error, "couldn't sync warps");
            // The pools fetched before it failed are whole, keep them. The one it failed in isn't:
            // keeping its newest pulls would let the next sync stop at them, short of the rest
            if !fetched.is_empty() && fetched.iter().all(|pull| pull.uid == uid) {
                match self.store.merge_history(uid, &fetched, source::API) {
                    Ok(merged) => info!(
                        uid,
                        added = merged.added,
                        confirmed = merged.confirmed,
                        "kept what synced before the failure"
                    ),
                    Err(e) => error!(%e, "couldn't keep the synced warps"),
                }
            }
            let _ = self.store.set_synced(uid, now(), Some(&error.to_string()));
            match error {
                ApiError::Expired => {
                    let _ = self.store.expire_authkey(uid);
                    self.due.remove(&uid);
                    self.full_pending.remove(&uid);
                    self.notice(
                        "The authkey has expired",
                        Some("Open Warp → View Details in the game for a new one".into()),
                        true,
                    );
                }
                ApiError::Rejected(_) => {
                    let _ = self.store.expire_authkey(uid);
                    self.due.remove(&uid);
                    self.full_pending.remove(&uid);
                    self.notice("The authkey was refused", Some(error.to_string()), true);
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
                        self.notice("Couldn't sync warps", Some(detail), true);
                    }
                }
            }
            self.show(uid);
            return;
        }

        if let Some(other) = fetched.iter().find(|pull| pull.uid != uid) {
            let detail = format!("It's UID {}'s, not {uid}'s", other.uid);
            self.notice("That authkey belongs to another account", Some(detail), true);
            return;
        }

        let merged = self.store.merge_history(uid, &fetched, source::API);
        let _ = self.store.set_synced(uid, now(), None);
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
                    self.notice("Warps synced", Some(detail), false);
                }
            }
            Err(e) => {
                error!(%e, "couldn't keep the synced warps");
                self.notice("Couldn't keep the synced warps", Some(e.to_string()), true);
            }
        }
        self.report_accounts();
        self.show(uid);
    }

    fn import_srs(&mut self, path: PathBuf, uid: u32, profile: Option<String>) {
        self.busy(Some("Reading the backup…".into()));
        let profiles = srs::read(&path);
        self.busy(None);
        let profiles = match profiles {
            Ok(profiles) if !profiles.is_empty() => profiles,
            Ok(_) => return self.notice("The backup has no warps in it", None, true),
            Err(e) => return self.notice("Couldn't import that file", Some(e), true),
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
            let detail = format!("Its pull ids don't end in {:02}, the last digits of UID {uid}", uid % 100);
            return self.notice("That profile is another account's", Some(detail), true);
        }

        let pulls: Vec<Pull> = chosen.pulls.iter().map(|p| Pull { uid, ..p.clone() }).collect();
        let merged = self.store.merge_history(uid, &pulls, source::SRS);
        let _ = self.store.add_rate_ups(&chosen.rate_ups);
        match merged {
            Ok(merged) => {
                let known = pulls.len() - merged.added - merged.confirmed;
                let detail = format!("{} new pulls, {known} already kept", merged.added + merged.confirmed);
                info!(
                    uid,
                    profile = chosen.key,
                    added = merged.added,
                    "imported a Star Rail Station backup"
                );
                self.notice("Imported from Star Rail Station", Some(detail), false);
            }
            Err(e) => self.notice("Couldn't import the backup", Some(e.to_string()), true),
        }
        self.report_accounts();
        self.show(uid);
    }

    fn show(&self, uid: u32) {
        match self.summary(uid) {
            Ok(summary) => (self.report)(Update::Summary(Box::new(summary))),
            Err(e) => error!(%e, "couldn't read the warps"),
        }
    }

    fn summary(&self, uid: u32) -> rusqlite::Result<Summary> {
        let pulls = self.store.pulls(uid)?;
        let rate_ups = self.store.rate_ups()?;
        let refs: Vec<&Pull> = pulls.iter().collect();

        let pools: Vec<PoolStats> = Pool::ALL
            .iter()
            .filter(|&&pool| matches!(pool, Pool::Character | Pool::LightCone | Pool::Standard) || pulls.iter().any(|p| p.pool == pool))
            .map(|&pool| pool_stats(pool, &refs, &rate_ups))
            .collect();

        let db = get_database();
        let mut names = HashMap::new();
        for stats in &pools {
            let items = stats
                .fives
                .iter()
                .map(|h| h.item_id)
                .chain(stats.recent.iter().map(|(p, _, _)| p.item_id));
            for item in items {
                names
                    .entry(item)
                    .or_insert_with(|| db.warp_item_name(item).unwrap_or_else(|| format!("#{item}")));
            }
        }

        // The banners running now, the standard one aside: it features its whole pool
        let mut banners: Vec<RunningBanner> = self
            .store
            .running_banners(now())?
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
                    end: (banner.end > 0 && banner.end < 4_000_000_000).then_some(banner.end),
                })
            })
            .filter(|banner| !banner.fives.is_empty())
            .collect();
        banners.sort_by_key(|b| std::cmp::Reverse(b.gacha_id));
        for item in banners.iter().flat_map(|b| b.fives.iter().chain(&b.fours)) {
            names
                .entry(*item)
                .or_insert_with(|| db.warp_item_name(*item).unwrap_or_else(|| format!("#{item}")));
        }

        // Requests: the history's, and the live ones still waiting
        let with_ids: Vec<Pull> = pulls.iter().filter(|p| p.id.is_some()).cloned().collect();
        let waiting: HashSet<(Option<u32>, i64)> = pulls
            .iter()
            .filter(|p| p.id.is_none())
            .map(|p| (p.request_no, if p.request_no.is_some() { 0 } else { p.time }))
            .collect();
        let requests = group_requests(&with_ids).len() + waiting.len();

        let account = self.store.account(uid)?;
        let key = self.store.authkey(uid)?.map(|key| key_state(&key));
        let next_look = match (self.store.oldest_unconfirmed(uid)?, &key) {
            (Some(pulled), Some(key)) if key.usable(now()) => Some(next_look(pulled, now())),
            _ => None,
        };
        Ok(Summary {
            next_look,
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

    fn report_accounts(&self) {
        if let Ok(accounts) = self.store.accounts() {
            (self.report)(Update::Accounts(accounts));
        }
    }

    fn busy(&self, what: Option<String>) {
        (self.report)(Update::Busy(what));
    }

    fn notice(&self, title: &str, detail: Option<String>, error: bool) {
        (self.report)(Update::Notice {
            title: title.into(),
            detail,
            error,
        });
    }
}

/// What the window would show for an account of the database at `path`, for snapshots of real data
pub fn summary_of(path: &std::path::Path, uid: Option<u32>) -> Option<Summary> {
    let store = Store::open(path).ok()?;
    let uid = uid.or_else(|| store.accounts().ok()?.first().copied())?;
    Worker::new(store, Box::new(|_| {})).summary(uid).ok()
}

fn key_state(key: &StoredKey) -> KeyState {
    KeyState {
        origin: key.origin.clone(),
        obtained_at: key.obtained_at,
        expired: key.expired,
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

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

    /// The whole way through with real data, when `RQA_E2E` is set: a Star Rail Station backup
    /// (`SRS_BACKUP`), the live pulls and authkey of the captures in `RQA_CAPTURES`, and a sync
    /// with the game's history using that authkey. Talks to the game's servers
    #[tokio::test]
    async fn from_backup_and_capture_to_synced_history() {
        if std::env::var("RQA_E2E").is_err() {
            return;
        }
        use futures::StreamExt;
        let dir = std::env::var("RQA_CAPTURES").unwrap();
        let mut events = Vec::new();
        for name in ["4.7standard.pcapng", "4.7recordauthkey.pcapng"] {
            let replay = crate::capture::Replay::for_path(std::path::Path::new(&dir).join(name)).unwrap();
            let manager = Arc::new(futures::lock::Mutex::new(crate::accounts::MultiAccountManager::new()));
            let mut session = crate::session::CaptureSession::new(manager, false);
            let mut packets = replay.open().unwrap();
            while let Some(packet) = packets.next().await {
                for event in session.handle_packet(packet.unwrap()).await {
                    if let crate::session::SessionEvent::Warp(event) = event {
                        events.push(event);
                    }
                }
            }
        }

        let notices = Arc::new(Mutex::new(Vec::new()));
        let heard = notices.clone();
        let mut worker = Worker::new(
            Store::open_in_memory().unwrap(),
            Box::new(move |update| {
                if let Update::Notice { title, detail, .. } = update {
                    heard.lock().unwrap().push(format!("{title}: {}", detail.unwrap_or_default()));
                }
            }),
        );
        const UID: u32 = 602857786;

        worker.handle(Command::ImportSrs {
            path: std::env::var("SRS_BACKUP").unwrap().into(),
            uid: UID,
            profile: Some("2".into()),
        });
        let after_import = worker.summary(UID).unwrap();
        println!("after import: {} pulls, {} requests", after_import.total, after_import.requests);

        // The live pulls happened about an hour before this test runs; the capture time is when
        // the session saw them in the replay, so put them back at when the capture was made
        let capture_time = std::fs::metadata(std::path::Path::new(&dir).join("4.7standard.pcapng"))
            .unwrap()
            .modified()
            .unwrap();
        let capture_ms = capture_time.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        for event in events {
            let event = match event {
                WarpEvent::Pulled(request) => WarpEvent::Pulled(LiveRequest {
                    captured_at: capture_ms - (1581 - i64::from(request.request_no.unwrap())) * 20_000,
                    ..request
                }),
                other => other,
            };
            // The authkey syncs at once
            worker.handle(Command::Event(event));
        }
        let synced = worker.summary(UID).unwrap();
        for notice in notices.lock().unwrap().iter() {
            println!("notice: {notice}");
        }
        println!(
            "after sync: {} pulls, {} requests, {} live waiting, counter {:?}, missing {:?}, error {:?}",
            synced.total,
            synced.requests,
            synced.unconfirmed,
            synced.request_counter,
            synced.missing_requests(),
            synced.last_error
        );
        for pool in &synced.pools {
            println!(
                "{:?}: pity5 {} pity4 {} guarantee5 {:?} latest 5 star {:?}",
                pool.pool,
                pool.pity5,
                pool.pity4,
                pool.guarantee5,
                pool.fives.first().map(|f| (synced.name(f.item_id), f.pity, f.outcome))
            );
        }
        assert!(synced.total > after_import.total);
        assert_eq!(synced.request_counter, Some(1581));
        let character = synced.pool(Pool::Character).unwrap();
        assert!(character.pity5 < 90, "the gap left by the missing pulls is filled");
    }
}
