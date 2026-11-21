//! The warp database: an SQLite file in the app's data folder.
//!
//! SQLite because the history is a few thousand rows per account that change a few at a time (a
//! live pull, a sync that gives live pulls their ids), and every change has to land whole: a
//! crash halfway through a sync must not leave pulls counted twice. A transaction gives that,
//! where a JSON file would be rewritten in full for every pull.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::api::HistoryKey;
use super::reconcile::{Request, group_requests, match_requests};
use super::stats::{Featured, RateUps};
use super::{BannerInfo, LiveRequest, Pool, Pull, source};

/// Where the database lives: the local app data folder on Windows (beside the settings),
/// Application Support on macOS, the XDG data folder elsewhere
pub fn default_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))
    };
    base.filter(|path| path.is_absolute())
        .map(|path| path.join("reliquary-archiver").join("warps.sqlite3"))
}

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE pulls (
    uid         INTEGER NOT NULL,
    -- The game's id, null until the history has the pull
    id          INTEGER UNIQUE,
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

/// An authkey and where it came from
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredKey {
    pub key: HistoryKey,
    pub origin: String,
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

    fn setup(conn: Connection) -> rusqlite::Result<Store> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 0 {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version > SCHEMA_VERSION {
            tracing::warn!(version, "the warp database is from a newer version of the archiver");
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

    pub fn pulls(&self, uid: u32) -> rusqlite::Result<Vec<Pull>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, pool, gacha_id, item_id, rarity, time, request_no, batch_index, sources FROM pulls WHERE uid = ?1")?;
        statement
            .query_map([uid], |row| {
                Ok(Pull {
                    id: row.get(0)?,
                    uid,
                    pool: Pool::from_gacha_type(row.get(1)?).unwrap_or(Pool::Standard),
                    gacha_id: row.get(2)?,
                    item_id: row.get(3)?,
                    rarity: row.get(4)?,
                    time: row.get(5)?,
                    request_no: row.get(6)?,
                    batch_index: row.get(7)?,
                    sources: row.get(8)?,
                })
            })?
            .collect()
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

    /// The pools that have live pulls waiting for their ids
    pub fn unconfirmed_pools(&self, uid: u32) -> rusqlite::Result<Vec<Pool>> {
        let mut statement = self.conn.prepare("SELECT DISTINCT pool FROM pulls WHERE uid = ?1 AND id IS NULL")?;
        statement
            .query_map([uid], |row| row.get::<_, u32>(0))?
            .map(|pool| pool.map(|pool| Pool::from_gacha_type(pool).unwrap_or(Pool::Standard)))
            .collect()
    }

    /// Keeps a request the capture saw. Seeing the same request again changes nothing.
    /// `rarity` gives an item's star rating
    pub fn record_live(&mut self, request: &LiveRequest, rarity: impl Fn(u32) -> u8) -> rusqlite::Result<bool> {
        let Some(pool) = Pool::from_gacha_id(request.gacha_id) else {
            return Ok(false);
        };
        let tx = self.conn.transaction()?;
        if let Some(n) = request.request_no {
            let seen: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM pulls WHERE uid = ?1 AND request_no = ?2)",
                params![request.uid, n],
                |row| row.get(0),
            )?;
            if seen {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO accounts (uid, request_counter) VALUES (?1, ?2)
                 ON CONFLICT (uid) DO UPDATE SET request_counter = MAX(COALESCE(request_counter, 0), ?2)",
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
                    request.captured_at.div_euclid(1000),
                    request.request_no,
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
            let updated = tx.execute("UPDATE pulls SET sources = sources | ?2 WHERE id = ?1", params![id, from])?;
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

    pub fn set_authkey(&self, uid: u32, key: &HistoryKey, origin: &str, obtained_at: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO authkeys (uid, authkey, host, game_biz, origin, obtained_at, expired)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![uid, key.authkey, key.host, key.game_biz, origin, obtained_at],
        )?;
        Ok(())
    }

    pub fn authkey(&self, uid: u32) -> rusqlite::Result<Option<StoredKey>> {
        self.conn
            .query_row(
                "SELECT authkey, host, game_biz, origin, obtained_at, expired FROM authkeys WHERE uid = ?1",
                [uid],
                |row| {
                    Ok(StoredKey {
                        key: HistoryKey {
                            authkey: row.get(0)?,
                            host: row.get(1)?,
                            game_biz: row.get(2)?,
                        },
                        origin: row.get(3)?,
                        obtained_at: row.get(4)?,
                        expired: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    pub fn expire_authkey(&self, uid: u32) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE authkeys SET expired = 1 WHERE uid = ?1", [uid])?;
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
    pub fn save_banners(&self, banners: &[BannerInfo]) -> rusqlite::Result<()> {
        for banner in banners {
            self.conn.execute(
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
        Ok(())
    }

    /// Adds what outcomes in a backup tell about banners. A banner the game described keeps its
    /// own list of what it features, but learns what it gives on a lost 50/50
    pub fn add_rate_ups(&self, rate_ups: &RateUps) -> rusqlite::Result<()> {
        let known = self.rate_ups()?;
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
            self.conn.execute(
                "INSERT INTO banners (gacha_id, rate_up, off) VALUES (?1, ?2, ?3)
                 ON CONFLICT (gacha_id) DO UPDATE SET off = ?3, rate_up = CASE WHEN begin IS NULL THEN ?2 ELSE rate_up END",
                params![gacha_id, items, off],
            )?;
        }
        Ok(())
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

/// Gives live pulls that `fresh` history pulls turn out to be their ids, and takes those pulls
/// out of `fresh`. Returns how many live pulls were confirmed
fn confirm_live(tx: &Transaction, uid: u32, fresh: &mut Vec<Pull>, from: u8) -> rusqlite::Result<usize> {
    // Live pulls without an id, a request at a time
    let mut statement = tx.prepare(
        "SELECT rowid, gacha_id, item_id, time, request_no FROM pulls WHERE uid = ?1 AND id IS NULL
         ORDER BY COALESCE(request_no, 4294967296), time, batch_index",
    )?;
    let rows: Vec<(i64, u32, u32, i64, Option<u32>)> = statement
        .query_map([uid], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    if rows.is_empty() || fresh.is_empty() {
        return Ok(0);
    }

    let mut live: Vec<Vec<&(i64, u32, u32, i64, Option<u32>)>> = Vec::new();
    for row in &rows {
        let same = live.last().is_some_and(|request| {
            let first = request[0];
            match (first.4, row.4) {
                (Some(a), Some(b)) => a == b,
                (None, None) => first.1 == row.1 && first.3 == row.3,
                _ => false,
            }
        });
        if same {
            live.last_mut().unwrap().push(row);
        } else {
            live.push(vec![row]);
        }
    }
    let live_requests: Vec<Request> = live
        .iter()
        .map(|request| Request::new(request[0].1, request.iter().map(|row| row.2).collect(), request[0].3))
        .collect();

    let groups = group_requests(fresh);
    let history_requests: Vec<Request> = groups
        .iter()
        .map(|group| {
            let first = &fresh[group[0]];
            Request::new(first.gacha_id, group.iter().map(|&i| fresh[i].item_id).collect(), first.time)
        })
        .collect();

    let mut confirmed = 0;
    let mut used = HashSet::new();
    for (l, h) in match_requests(&live_requests, &history_requests) {
        let mut rows: Vec<&(i64, u32, u32, i64, Option<u32>)> = live[l].clone();
        // The history's order within the request is the real one, the items are the same
        for &i in &groups[h] {
            let pull = &fresh[i];
            let Some(at) = rows.iter().position(|row| row.2 == pull.item_id) else {
                continue;
            };
            let row = rows.remove(at);
            tx.execute(
                "UPDATE pulls SET id = ?2, time = ?3, rarity = ?4, sources = sources | ?5 WHERE rowid = ?1",
                params![row.0, pull.id, pull.time, pull.rarity, from],
            )?;
            used.insert(i);
            confirmed += 1;
        }
    }
    let mut index = 0;
    fresh.retain(|_| {
        let keep = !used.contains(&index);
        index += 1;
        keep
    });
    Ok(confirmed)
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
        let store = Store::open_in_memory().unwrap();
        let key = HistoryKey::for_account(UID, "secret".into());
        store.set_authkey(UID, &key, "capture", 100).unwrap();
        assert_eq!(store.authkey(UID).unwrap().unwrap().key, key);
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
}
