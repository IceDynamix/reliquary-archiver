//! The warp database: an SQLite file in the app's data folder.
//!
//! SQLite because the history is a few thousand rows per account that change a few at a time (a
//! live pull, a sync that gives live pulls their ids), and every change has to land whole: a
//! crash halfway through a sync must not leave pulls counted twice. A transaction gives that,
//! where a JSON file would be rewritten in full for every pull.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::api::HistoryKey;
use super::reconcile::{self, LiveRow};
use super::stats::{Featured, RateUps};
use super::{BannerInfo, LiveRequest, Pool, Pull, source};

/// Where the database lives
pub fn default_path() -> Option<PathBuf> {
    crate::dirs::data_dir().map(|dir| dir.join("warps.sqlite3"))
}

const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE pulls (
    uid         INTEGER NOT NULL,
    -- The game's id, null until the history has the pull. Ids end in the uid's last two digits,
    -- so two accounts can share one
    id          INTEGER,
    pool        INTEGER NOT NULL,
    gacha_id    INTEGER NOT NULL,
    item_id     INTEGER NOT NULL,
    rarity      INTEGER NOT NULL,
    -- Unix seconds
    time        INTEGER NOT NULL,
    -- The account's warp request counter, for pulls seen live
    request_no  INTEGER,
    batch_index INTEGER NOT NULL DEFAULT 0,
    sources     INTEGER NOT NULL,
    UNIQUE (uid, id),
    UNIQUE (uid, request_no, batch_index)
);
CREATE INDEX pulls_by_account ON pulls (uid, pool, time);

CREATE TABLE accounts (
    uid             INTEGER PRIMARY KEY,
    -- The latest warp request counter seen
    request_counter INTEGER,
    -- Unix seconds
    last_sync       INTEGER,
    last_error      TEXT
);

CREATE TABLE authkeys (
    uid         INTEGER PRIMARY KEY,
    authkey     TEXT NOT NULL,
    host        TEXT NOT NULL,
    game_biz    TEXT NOT NULL,
    -- capture, cache or link
    origin      TEXT NOT NULL,
    obtained_at INTEGER NOT NULL,
    expired     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE banners (
    gacha_id INTEGER PRIMARY KEY,
    -- JSON arrays of item ids: what it features, and what it gives on a lost 50/50
    rate_up  TEXT NOT NULL,
    off      TEXT NOT NULL DEFAULT '[]',
    -- From the game, null when pieced together from outcomes
    begin    INTEGER,
    end      INTEGER
);
";

/// Schema 1 to 2: a pull's id is unique per account, not across all of them
const MIGRATE_1_TO_2: &str = "
CREATE TABLE pulls_2 (
    uid         INTEGER NOT NULL,
    id          INTEGER,
    pool        INTEGER NOT NULL,
    gacha_id    INTEGER NOT NULL,
    item_id     INTEGER NOT NULL,
    rarity      INTEGER NOT NULL,
    time        INTEGER NOT NULL,
    request_no  INTEGER,
    batch_index INTEGER NOT NULL DEFAULT 0,
    sources     INTEGER NOT NULL,
    UNIQUE (uid, id),
    UNIQUE (uid, request_no, batch_index)
);
INSERT INTO pulls_2 SELECT uid, id, pool, gacha_id, item_id, rarity, time, request_no, batch_index, sources FROM pulls;
DROP TABLE pulls;
ALTER TABLE pulls_2 RENAME TO pulls;
CREATE INDEX pulls_by_account ON pulls (uid, pool, time);
";

/// How far apart in time the same live request can be seen twice. A counter that comes again
/// further apart than this is the counter starting over, not the same request
const SAME_REQUEST_WITHIN: i64 = 24 * 3600;

/// Where an authkey came from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOrigin {
    /// The capture saw the game ask for it
    Capture,
    /// The game's web cache
    Cache,
    /// A link the user pasted
    Link,
}

impl KeyOrigin {
    /// As the database keeps it
    pub fn as_str(self) -> &'static str {
        match self {
            KeyOrigin::Capture => "capture",
            KeyOrigin::Cache => "cache",
            KeyOrigin::Link => "link",
        }
    }
}

impl FromStr for KeyOrigin {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Ok(match text {
            "capture" => KeyOrigin::Capture,
            "cache" => KeyOrigin::Cache,
            "link" => KeyOrigin::Link,
            _ => return Err(format!("no authkey origin is called {text:?}")),
        })
    }
}

/// An authkey and where it came from
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredKey {
    pub key: HistoryKey,
    pub origin: KeyOrigin,
    /// Unix seconds
    pub obtained_at: i64,
    pub expired: bool,
}

/// What syncing knows about an account
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountState {
    pub request_counter: Option<u32>,
    pub last_sync: Option<i64>,
    pub last_error: Option<String>,
}

/// What a merge did
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// Pulls that weren't known at all
    pub added: usize,
    /// Live pulls that got their ids
    pub confirmed: usize,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        // Another copy of the app may hold the file a moment
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::setup(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Store> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(mut conn: Connection) -> rusqlite::Result<Store> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                conn.execute_batch(SCHEMA)?;
                conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            1 => {
                let tx = conn.transaction()?;
                tx.execute_batch(MIGRATE_1_TO_2)?;
                tx.pragma_update(None, "user_version", 2)?;
                tx.commit()?;
            }
            SCHEMA_VERSION => {}
            _ => {
                // Writing to it could undo what the newer version keeps
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                    Some("The warp database is from a newer version of the archiver".into()),
                ));
            }
        }
        Ok(Store { conn })
    }

    /// Accounts with anything kept, the one with the newest pull first
    pub fn accounts(&self) -> rusqlite::Result<Vec<u32>> {
        let mut statement = self.conn.prepare(
            "SELECT uid FROM (SELECT uid, MAX(time) AS latest FROM pulls GROUP BY uid
             UNION ALL SELECT uid, 0 FROM accounts UNION ALL SELECT uid, 0 FROM authkeys)
             GROUP BY uid ORDER BY MAX(latest) DESC",
        )?;
        statement.query_map([], |row| row.get(0))?.collect()
    }

    /// The account's pulls. A pull from a warp type this version doesn't know is left out
    pub fn pulls(&self, uid: u32) -> rusqlite::Result<Vec<Pull>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, pool, gacha_id, item_id, rarity, time, request_no, batch_index, sources FROM pulls WHERE uid = ?1")?;
        let rows = statement.query_map([uid], |row| {
            let Some(pool) = known_pool(row, 1)? else {
                return Ok(None);
            };
            Ok(Some(Pull {
                id: row.get(0)?,
                uid,
                pool,
                gacha_id: row.get(2)?,
                item_id: row.get(3)?,
                rarity: row.get(4)?,
                time: row.get(5)?,
                request_no: row.get(6)?,
                batch_index: row.get(7)?,
                sources: row.get(8)?,
            }))
        })?;
        rows.filter_map(Result::transpose).collect()
    }

    /// Ids the game's history already gave, so a sync can stop where it reaches them
    pub fn history_ids(&self, uid: u32) -> rusqlite::Result<HashSet<i64>> {
        let mut statement = self
            .conn
            .prepare("SELECT id FROM pulls WHERE uid = ?1 AND id IS NOT NULL AND sources & ?2 != 0")?;
        statement.query_map(params![uid, source::API], |row| row.get(0))?.collect()
    }

    /// Live pulls still waiting for their ids
    pub fn unconfirmed(&self, uid: u32) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM pulls WHERE uid = ?1 AND id IS NULL", [uid], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count as usize)
    }

    /// When the oldest live pull still waiting for its id was made
    pub fn oldest_unconfirmed(&self, uid: u32) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row("SELECT MIN(time) FROM pulls WHERE uid = ?1 AND id IS NULL", [uid], |row| row.get(0))
    }

    /// The pools that have live pulls waiting for their ids, of those this version knows
    pub fn unconfirmed_pools(&self, uid: u32) -> rusqlite::Result<Vec<Pool>> {
        let mut statement = self.conn.prepare("SELECT DISTINCT pool FROM pulls WHERE uid = ?1 AND id IS NULL")?;
        let rows = statement.query_map([uid], |row| known_pool(row, 0))?;
        rows.filter_map(Result::transpose).collect()
    }

    /// Keeps a request the capture saw. Seeing the same request again changes nothing.
    /// `rarity` gives an item's star rating
    pub fn record_live(&mut self, request: &LiveRequest, rarity: impl Fn(u32) -> u8) -> rusqlite::Result<bool> {
        let Some(pool) = Pool::from_gacha_id(request.gacha_id) else {
            return Ok(false);
        };
        let tx = self.conn.transaction()?;
        let time = request.captured_at.div_euclid(1000);
        let first_item = request.items.first().copied().unwrap_or_default();

        // The counter only ever goes up. A request at or below it that isn't one already kept means
        // the counter can't be trusted: it's kept without it, rather than taken for a kept one
        let mut request_no = request.request_no;
        if let Some(n) = request_no {
            let again: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM pulls WHERE uid = ?1 AND request_no = ?2 AND batch_index = 0
                 AND gacha_id = ?3 AND item_id = ?4 AND ABS(time - ?5) < ?6)",
                params![request.uid, n, request.gacha_id, first_item, time, SAME_REQUEST_WITHIN],
                |row| row.get(0),
            )?;
            if again {
                return Ok(false);
            }
            let counter: Option<u32> = tx
                .query_row("SELECT request_counter FROM accounts WHERE uid = ?1", [request.uid], |row| {
                    row.get(0)
                })
                .optional()?
                .flatten();
            if counter.is_some_and(|counter| n <= counter) {
                tracing::warn!(
                    uid = request.uid,
                    n,
                    ?counter,
                    "the warp request counter went back, keeping the request without it"
                );
                request_no = None;
            }
        }
        // Without a counter, a request is known by its banner and second, as `LiveKey` tells them
        // apart
        if request_no.is_none() {
            let seen: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM pulls
                 WHERE uid = ?1 AND gacha_id = ?2 AND time = ?3 AND request_no IS NULL AND sources & ?4 != 0)",
                params![request.uid, request.gacha_id, time, source::LIVE],
                |row| row.get(0),
            )?;
            if seen {
                return Ok(false);
            }
        }
        if let Some(n) = request_no {
            tx.execute(
                "INSERT INTO accounts (uid, request_counter) VALUES (?1, ?2)
                 ON CONFLICT (uid) DO UPDATE SET request_counter = ?2",
                params![request.uid, n],
            )?;
        }
        for (index, &item) in request.items.iter().enumerate() {
            tx.execute(
                "INSERT INTO pulls (uid, pool, gacha_id, item_id, rarity, time, request_no, batch_index, sources)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    request.uid,
                    pool.gacha_type(),
                    request.gacha_id,
                    item,
                    rarity(item),
                    time,
                    request_no,
                    index as u8,
                    source::LIVE
                ],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Adds pulls from the game's history or a backup. Live pulls they turn out to be get their
    /// ids, pulls already kept only note the new source
    pub fn merge_history(&mut self, uid: u32, pulls: &[Pull], from: u8) -> rusqlite::Result<Merged> {
        let tx = self.conn.transaction()?;
        let mut merged = Merged::default();

        let mut fresh = Vec::new();
        for pull in pulls {
            let Some(id) = pull.id else { continue };
            // Another account's pull, from a backup profile or history that isn't this account's
            if !super::id_belongs_to(id, uid) {
                continue;
            }
            let updated = tx.execute("UPDATE pulls SET sources = sources | ?3 WHERE uid = ?1 AND id = ?2", params![
                uid, id, from
            ])?;
            if updated == 0 {
                fresh.push(pull.clone());
            }
        }

        merged.confirmed = confirm_live(&tx, uid, &mut fresh, from)?;

        for pull in fresh.iter().filter(|p| p.id.is_some()) {
            merged.added += tx.execute(
                "INSERT OR IGNORE INTO pulls (uid, id, pool, gacha_id, item_id, rarity, time, sources)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    uid,
                    pull.id,
                    pull.pool.gacha_type(),
                    pull.gacha_id,
                    pull.item_id,
                    pull.rarity,
                    pull.time,
                    from
                ],
            )?;
        }
        tx.commit()?;
        Ok(merged)
    }

    pub fn set_authkey(&self, uid: u32, key: &HistoryKey, origin: KeyOrigin, obtained_at: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO authkeys (uid, authkey, host, game_biz, origin, obtained_at, expired)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![uid, key.authkey, key.host(), key.game_biz(), origin.as_str(), obtained_at],
        )?;
        Ok(())
    }

    pub fn authkey(&self, uid: u32) -> rusqlite::Result<Option<StoredKey>> {
        self.conn
            .query_row(
                "SELECT authkey, game_biz, origin, obtained_at, expired FROM authkeys WHERE uid = ?1",
                [uid],
                |row| {
                    Ok(StoredKey {
                        key: HistoryKey::for_region(row.get::<_, String>(1)? == "hkrpg_cn", row.get(0)?),
                        origin: row
                            .get::<_, String>(2)?
                            .parse()
                            .map_err(|e: String| rusqlite::Error::FromSqlConversionFailure(2, Type::Text, e.into()))?,
                        obtained_at: row.get(3)?,
                        expired: row.get(4)?,
                    })
                },
            )
            .optional()
    }

    pub fn expire_authkey(&self, uid: u32) -> rusqlite::Result<()> {
        // A key that's no use is no reason to keep a secret on disk
        self.conn
            .execute("UPDATE authkeys SET expired = 1, authkey = '' WHERE uid = ?1", [uid])?;
        Ok(())
    }

    /// Drops the account's live pulls in `pools` that were made before `before` (Unix seconds, on
    /// the clock that saw them) and still have no id. A sync after that has the history of then,
    /// so the pull is in it and wasn't recognized (with the clock far off, say): it's been kept
    /// from the history already, and would otherwise count twice. Returns how many were dropped
    pub fn drop_stale_live(&self, uid: u32, pools: &[Pool], before: i64) -> rusqlite::Result<usize> {
        let mut dropped = 0;
        for pool in pools {
            dropped += self.conn.execute(
                "DELETE FROM pulls WHERE uid = ?1 AND pool = ?2 AND id IS NULL AND time < ?3",
                params![uid, pool.gacha_type(), before],
            )?;
        }
        Ok(dropped)
    }

    #[cfg(test)]
    pub fn set_request_counter(&self, uid: u32, counter: u32) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (uid, request_counter) VALUES (?1, ?2) ON CONFLICT (uid) DO UPDATE SET request_counter = ?2",
            params![uid, counter],
        )?;
        Ok(())
    }

    pub fn account(&self, uid: u32) -> rusqlite::Result<AccountState> {
        Ok(self
            .conn
            .query_row(
                "SELECT request_counter, last_sync, last_error FROM accounts WHERE uid = ?1",
                [uid],
                |row| {
                    Ok(AccountState {
                        request_counter: row.get(0)?,
                        last_sync: row.get(1)?,
                        last_error: row.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn set_synced(&self, uid: u32, at: i64, error: Option<&str>) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (uid, last_sync, last_error) VALUES (?1, ?2, ?3)
             ON CONFLICT (uid) DO UPDATE SET last_sync = COALESCE(?2, last_sync), last_error = ?3",
            params![uid, (error.is_none()).then_some(at), error],
        )?;
        Ok(())
    }

    /// Keeps what banners feature. A banner's list from the game replaces one pieced together
    pub fn save_banners(&mut self, banners: &[BannerInfo]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for banner in banners {
            tx.execute(
                "INSERT INTO banners (gacha_id, rate_up, begin, end) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (gacha_id) DO UPDATE SET rate_up = ?2, begin = ?3, end = ?4",
                params![
                    banner.gacha_id,
                    serde_json::to_string(&banner.rate_up).unwrap_or_default(),
                    banner.begin,
                    banner.end
                ],
            )?;
        }
        tx.commit()
    }

    /// Adds what outcomes in a backup tell about banners. A banner the game described keeps its
    /// own list of what it features, but learns what it gives on a lost 50/50
    pub fn add_rate_ups(&mut self, rate_ups: &RateUps) -> rusqlite::Result<()> {
        let known = self.rate_ups()?;
        let tx = self.conn.transaction()?;
        let json = |items: &mut dyn Iterator<Item = &u32>| {
            let mut all: Vec<u32> = items.copied().collect();
            all.sort_unstable();
            all.dedup();
            serde_json::to_string(&all).unwrap_or_default()
        };
        for (gacha_id, learned) in rate_ups {
            let before = known.get(gacha_id).cloned().unwrap_or_default();
            let items = json(&mut before.items.iter().chain(&learned.items));
            let off = json(&mut before.not.iter().chain(&learned.not));
            tx.execute(
                "INSERT INTO banners (gacha_id, rate_up, off) VALUES (?1, ?2, ?3)
                 ON CONFLICT (gacha_id) DO UPDATE SET off = ?3, rate_up = CASE WHEN begin IS NULL THEN ?2 ELSE rate_up END",
                params![gacha_id, items, off],
            )?;
        }
        tx.commit()
    }

    /// Banners the game described that are running at `now`, in Unix seconds
    pub fn running_banners(&self, now: i64) -> rusqlite::Result<Vec<BannerInfo>> {
        let mut statement = self.conn.prepare(
            "SELECT gacha_id, rate_up, begin, end FROM banners
             WHERE begin IS NOT NULL AND begin <= ?1 AND (end = 0 OR end > ?1) ORDER BY gacha_id",
        )?;
        statement
            .query_map([now], |row| {
                Ok(BannerInfo {
                    gacha_id: row.get(0)?,
                    rate_up: serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or_default(),
                    begin: row.get(2)?,
                    end: row.get(3)?,
                })
            })?
            .collect()
    }

    pub fn rate_ups(&self) -> rusqlite::Result<RateUps> {
        let mut statement = self.conn.prepare("SELECT gacha_id, rate_up, off FROM banners")?;
        let list = |text: String| serde_json::from_str::<Vec<u32>>(&text).unwrap_or_default().into_iter().collect();
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, Featured {
                    items: list(row.get(1)?),
                    not: list(row.get(2)?),
                }))
            })?
            .collect()
    }
}

/// Above every request counter, which is a `u32`: live pulls without one sort after the rest
const AFTER_ANY_COUNTER: i64 = 1 << 32;

/// A column's pool, `None` (with a warning) for a warp type this version doesn't know
fn known_pool(row: &Row, column: usize) -> rusqlite::Result<Option<Pool>> {
    let gacha_type: u32 = row.get(column)?;
    let pool = Pool::from_gacha_type(gacha_type);
    if pool.is_none() {
        tracing::warn!(gacha_type, "left out a pull from a warp type this version doesn't know");
    }
    Ok(pool)
}

/// Gives live pulls that `fresh` history pulls turn out to be their ids, and takes those pulls
/// out of `fresh`. Returns how many live pulls were confirmed
fn confirm_live(tx: &Transaction, uid: u32, fresh: &mut Vec<Pull>, from: u8) -> rusqlite::Result<usize> {
    if fresh.is_empty() {
        return Ok(0);
    }
    // Live pulls without an id, in the order they were made
    let mut statement = tx.prepare(
        "SELECT rowid, gacha_id, item_id, time, request_no FROM pulls WHERE uid = ?1 AND id IS NULL
         ORDER BY COALESCE(request_no, ?2), time, batch_index",
    )?;
    let live: Vec<LiveRow> = statement
        .query_map(params![uid, AFTER_ANY_COUNTER], |row| {
            Ok(LiveRow {
                rowid: row.get(0)?,
                gacha_id: row.get(1)?,
                item_id: row.get(2)?,
                time: row.get(3)?,
                request_no: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);

    let pairs = reconcile::confirm(&live, fresh);
    let mut used = HashSet::new();
    for &(l, h) in &pairs {
        let pull = &fresh[h];
        tx.execute(
            "UPDATE pulls SET id = ?2, time = ?3, rarity = ?4, sources = sources | ?5 WHERE rowid = ?1",
            params![live[l].rowid, pull.id, pull.time, pull.rarity, from],
        )?;
        used.insert(h);
    }
    let mut index = 0;
    fresh.retain(|_| {
        let keep = !used.contains(&index);
        index += 1;
        keep
    });
    Ok(pairs.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UID: u32 = 602857786;

    fn history(id: i64, gacha_id: u32, item_id: u32, time: i64) -> Pull {
        Pull {
            id: Some(id),
            uid: UID,
            pool: Pool::from_gacha_id(gacha_id).unwrap(),
            gacha_id,
            item_id,
            rarity: 3,
            time,
            request_no: None,
            batch_index: 0,
            sources: source::API,
        }
    }

    fn live(request_no: u32, gacha_id: u32, items: Vec<u32>, time: i64) -> LiveRequest {
        LiveRequest {
            uid: UID,
            gacha_id,
            request_no: Some(request_no),
            items,
            captured_at: time * 1000,
        }
    }

    #[test]
    fn live_pulls_get_their_ids_when_the_history_catches_up() {
        let mut store = Store::open_in_memory().unwrap();
        assert!(store.record_live(&live(1576, 2142, vec![20011], 1000), |_| 3).unwrap());
        assert!(store.record_live(&live(1577, 1001, vec![20000], 1010), |_| 3).unwrap());
        // Seeing a request twice keeps it once
        assert!(!store.record_live(&live(1577, 1001, vec![20000], 1010), |_| 3).unwrap());
        assert_eq!(store.unconfirmed(UID).unwrap(), 2);

        let merged = store
            .merge_history(
                UID,
                &[
                    history(1777245000000100086, 1001, 20005, 900),
                    history(1777245000000200086, 2142, 20011, 1001),
                    history(1777245000000300086, 1001, 20000, 1011),
                ],
                source::API,
            )
            .unwrap();
        assert_eq!(merged, Merged { added: 1, confirmed: 2 });
        assert_eq!(store.unconfirmed(UID).unwrap(), 0);

        let pulls = store.pulls(UID).unwrap();
        assert_eq!(pulls.len(), 3);
        let confirmed = pulls.iter().find(|p| p.request_no == Some(1577)).unwrap();
        assert_eq!(confirmed.id, Some(1777245000000300086));
        assert_eq!(confirmed.sources, source::LIVE | source::API);
        assert_eq!(store.account(UID).unwrap().request_counter, Some(1577));

        // The same history again changes nothing
        let again = store
            .merge_history(UID, &[history(1777245000000300086, 1001, 20000, 1011)], source::API)
            .unwrap();
        assert_eq!(again, Merged::default());
    }

    #[test]
    fn a_counter_that_goes_back_is_not_taken_for_a_kept_request() {
        let mut store = Store::open_in_memory().unwrap();
        assert!(store.record_live(&live(1576, 2142, vec![20011], 1000), |_| 3).unwrap());
        // The same number on another banner, much later: the counter started over
        assert!(
            store
                .record_live(&live(1576, 1001, vec![20000], 1000 + 2 * SAME_REQUEST_WITHIN), |_| 3)
                .unwrap()
        );
        // And lower than the last one
        assert!(
            store
                .record_live(&live(12, 1001, vec![20001], 1000 + 3 * SAME_REQUEST_WITHIN), |_| 3)
                .unwrap()
        );
        assert_eq!(store.unconfirmed(UID).unwrap(), 3);
        // The counter stays where it was
        assert_eq!(store.account(UID).unwrap().request_counter, Some(1576));
        // The first request again is still kept once
        assert!(!store.record_live(&live(1576, 2142, vec![20011], 1005), |_| 3).unwrap());
    }

    #[test]
    fn two_accounts_can_have_the_same_pull_id() {
        let mut store = Store::open_in_memory().unwrap();
        let other = UID + 100;
        store.merge_history(UID, &[history(100086, 1001, 20000, 5)], source::API).unwrap();
        let theirs = Pull {
            uid: other,
            ..history(100086, 1001, 20001, 5)
        };
        assert_eq!(store.merge_history(other, &[theirs], source::API).unwrap().added, 1);
        assert_eq!(store.pulls(UID).unwrap()[0].item_id, 20000);
        assert_eq!(store.pulls(other).unwrap()[0].item_id, 20001);
    }

    #[test]
    fn live_pulls_the_history_never_recognized_are_dropped() {
        let mut store = Store::open_in_memory().unwrap();
        store.record_live(&live(1, 1001, vec![20000], 1000), |_| 3).unwrap();
        store.record_live(&live(2, 2142, vec![20011], 1000), |_| 3).unwrap();
        store.record_live(&live(3, 1001, vec![20001], 9000), |_| 3).unwrap();
        // Only the pools synced, and only what's older than the history
        assert_eq!(store.drop_stale_live(UID, &[Pool::Standard], 5000).unwrap(), 1);
        assert_eq!(store.unconfirmed(UID).unwrap(), 2);
    }

    #[test]
    fn a_first_schema_database_moves_to_the_second() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            &SCHEMA
                .replace("    UNIQUE (uid, id),\n", "")
                .replace("    id          INTEGER,", "    id          INTEGER UNIQUE,"),
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO pulls (uid, id, pool, gacha_id, item_id, rarity, time, sources) VALUES (?1, 100086, 1, 1001, 20000, 3, 5, 2)",
            [UID],
        )
        .unwrap();
        let mut store = Store::setup(conn).unwrap();
        assert_eq!(store.pulls(UID).unwrap().len(), 1);
        let theirs = Pull {
            uid: UID + 100,
            ..history(100086, 1001, 20001, 5)
        };
        assert_eq!(store.merge_history(UID + 100, &[theirs], source::API).unwrap().added, 1);

        let newer = Connection::open_in_memory().unwrap();
        newer.pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
        assert!(Store::setup(newer).is_err());
    }

    #[test]
    fn a_backup_marks_pulls_it_also_has() {
        let mut store = Store::open_in_memory().unwrap();
        store.merge_history(UID, &[history(100086, 1001, 20000, 5)], source::API).unwrap();
        let merged = store
            .merge_history(UID, &[history(100086, 1001, 20000, 5), history(99986, 1001, 20001, 4)], source::SRS)
            .unwrap();
        assert_eq!(merged.added, 1);
        let pulls = store.pulls(UID).unwrap();
        assert!(pulls.iter().any(|p| p.sources == source::API | source::SRS));
        assert_eq!(store.history_ids(UID).unwrap().len(), 1);
    }

    #[test]
    fn keys_and_banners_are_kept() {
        let mut store = Store::open_in_memory().unwrap();
        let key = HistoryKey::for_account(UID, "secret".into());
        store.set_authkey(UID, &key, KeyOrigin::Link, 100).unwrap();
        let stored = store.authkey(UID).unwrap().unwrap();
        assert_eq!((stored.key, stored.origin), (key, KeyOrigin::Link));
        store.expire_authkey(UID).unwrap();
        assert!(store.authkey(UID).unwrap().unwrap().expired);

        let mut from_backup = RateUps::new();
        from_backup.insert(2116, Featured {
            items: HashSet::from([1214]),
            not: HashSet::from([21002]),
        });
        store.add_rate_ups(&from_backup).unwrap();
        store
            .save_banners(&[BannerInfo {
                gacha_id: 2142,
                begin: 1,
                end: 2,
                rate_up: vec![1506],
            }])
            .unwrap();
        let rate_ups = store.rate_ups().unwrap();
        assert!(rate_ups[&2116].items.contains(&1214));
        assert!(rate_ups[&2116].not.contains(&21002));
        assert!(rate_ups[&2142].items.contains(&1506));
        assert_eq!(store.accounts().unwrap(), vec![UID]);
    }

    #[test]
    fn a_request_without_a_counter_is_kept_once() {
        let mut store = Store::open_in_memory().unwrap();
        let request = LiveRequest {
            request_no: None,
            ..live(0, 1001, vec![20000, 20001], 1010)
        };
        assert!(store.record_live(&request, |_| 3).unwrap());
        assert!(!store.record_live(&request, |_| 3).unwrap());
        assert_eq!(store.unconfirmed(UID).unwrap(), 2);
    }

    #[test]
    fn pulls_from_an_unknown_warp_type_are_left_out() {
        let mut store = Store::open_in_memory().unwrap();
        store.merge_history(UID, &[history(100086, 1001, 20000, 5)], source::API).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO pulls (uid, pool, gacha_id, item_id, rarity, time, sources) VALUES (?1, 99, 9001, 20000, 3, 6, ?2)",
                params![UID, source::LIVE],
            )
            .unwrap();
        let pools: Vec<Pool> = store.pulls(UID).unwrap().iter().map(|p| p.pool).collect();
        assert_eq!(pools, [Pool::Standard]);
        assert!(store.unconfirmed_pools(UID).unwrap().is_empty());
    }

    #[test]
    fn banners_are_kept_all_or_none() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TRIGGER refuse BEFORE INSERT ON banners WHEN NEW.gacha_id = 9999
                 BEGIN SELECT RAISE(ABORT, 'refused'); END",
            )
            .unwrap();
        let banner = |gacha_id| BannerInfo {
            gacha_id,
            begin: 1,
            end: 2,
            rate_up: vec![1506],
        };
        assert!(store.save_banners(&[banner(2142), banner(9999)]).is_err());
        let mut learned = RateUps::new();
        for gacha_id in [2116, 9999] {
            learned.insert(gacha_id, Featured {
                items: HashSet::from([1214]),
                not: HashSet::new(),
            });
        }
        assert!(store.add_rate_ups(&learned).is_err());
        assert!(store.rate_ups().unwrap().is_empty());
    }

    #[test]
    fn live_requests_without_a_counter_on_different_banners_in_one_second_stay_apart() {
        let mut store = Store::open_in_memory().unwrap();
        let unnumbered = |gacha_id, item| LiveRequest {
            request_no: None,
            ..live(0, gacha_id, vec![item], 1000)
        };
        assert!(store.record_live(&unnumbered(1001, 20000), |_| 3).unwrap());
        assert!(store.record_live(&unnumbered(2142, 20000), |_| 3).unwrap());
        let merged = store
            .merge_history(
                UID,
                &[
                    history(1777245000000100086, 1001, 20000, 1000),
                    history(1777245000000300086, 2142, 20000, 1000),
                ],
                source::API,
            )
            .unwrap();
        assert_eq!(merged, Merged { added: 0, confirmed: 2 });
    }
}
