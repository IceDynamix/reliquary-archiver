//! The game's warp history, as its in-game page fetches it.
//!
//! Each pool is listed newest first, twenty pulls a page, each page continuing from the last id
//! of the one before. Times are in the server's time zone, which the response names. The history
//! reaches back about six months and lags up to an hour behind.

use std::time::Duration;

use serde::Deserialize;
use tracing::{debug, info};

use super::{Pool, Pull, source};

/// What a request to the history needs besides the pool and page
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryKey {
    /// From the game, valid for about a day
    pub authkey: String,
    /// The server the history is on, e.g. `public-operation-hkrpg-sg.hoyoverse.com`
    pub host: String,
    pub game_biz: String,
}

impl HistoryKey {
    /// The server an account's history is on
    pub fn for_account(uid: u32, authkey: String) -> Self {
        let (host, game_biz) = if super::is_mainland(uid) {
            ("public-operation-hkrpg.mihoyo.com", "hkrpg_cn")
        } else {
            ("public-operation-hkrpg-sg.hoyoverse.com", "hkrpg_global")
        };
        Self {
            authkey,
            host: host.into(),
            game_biz: game_biz.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The authkey is too old, the game hands out a new one when the history page is opened
    Expired,
    /// The authkey isn't one the history takes
    Rejected(String),
    /// Asked too often, try again later
    Throttled,
    Network(String),
    Other(i64, String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Expired => write!(f, "the authkey has expired"),
            ApiError::Rejected(message) => write!(f, "the authkey was refused ({message})"),
            ApiError::Throttled => write!(f, "the server asks to slow down"),
            ApiError::Network(message) => write!(f, "couldn't reach the server: {message}"),
            ApiError::Other(code, message) => write!(f, "the server said {message} ({code})"),
        }
    }
}

#[derive(Deserialize)]
struct Response {
    retcode: i64,
    message: String,
    data: Option<Page>,
}

#[derive(Deserialize)]
struct Page {
    list: Vec<Record>,
    #[serde(default)]
    region_time_zone: i32,
}

#[derive(Deserialize, Debug)]
struct Record {
    uid: String,
    gacha_id: String,
    gacha_type: String,
    item_id: String,
    time: String,
    rank_type: String,
    id: String,
}

/// The longest wait after being told to slow down, the waits before it double up from 2 seconds
const THROTTLE_GIVE_UP: Duration = Duration::from_secs(32);

/// Fetches pages
pub struct Client {
    agent: ureq::Agent,
    key: HistoryKey,
    /// The pause between pages, the server refuses clients that ask too fast
    pace: Duration,
}

impl Client {
    pub fn new(key: HistoryKey) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("reliquary-archiver/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent,
            key,
            pace: Duration::from_millis(600),
        }
    }

    /// One page of a pool, the pulls before `end_id` (0 for the newest). When the server asks to
    /// slow down, waits longer each time before asking again, for about a minute in all
    fn page(&self, pool: Pool, end_id: i64, size: u32) -> Result<Vec<Pull>, ApiError> {
        let mut wait = Duration::from_secs(2);
        loop {
            match self.page_once(pool, end_id, size) {
                Err(ApiError::Throttled) if wait <= THROTTLE_GIVE_UP => {
                    debug!(?pool, ?wait, "the warp history asks to slow down, waiting");
                    std::thread::sleep(wait);
                    wait *= 2;
                }
                result => return result,
            }
        }
    }

    fn page_once(&self, pool: Pool, end_id: i64, size: u32) -> Result<Vec<Pull>, ApiError> {
        let endpoint = if pool.is_collab() { "getLdGachaLog" } else { "getGachaLog" };
        let url = format!("https://{}/common/hkrpg_gacha_record/api/{endpoint}", self.key.host);
        let request = self
            .agent
            .get(&url)
            // These two are what the game's page asks with, whatever the authkey response said
            .query("authkey_ver", "1")
            .query("sign_type", "2")
            .query("lang", "en")
            .query("game_biz", &self.key.game_biz)
            .query("authkey", &self.key.authkey)
            .query("gacha_type", &pool.gacha_type().to_string())
            .query("page", "1")
            .query("size", &size.to_string())
            .query("end_id", &end_id.to_string());

        let response: Response = request
            .call()
            .map_err(|e| ApiError::Network(e.to_string()))?
            .into_string()
            .map_err(|e| ApiError::Network(e.to_string()))
            .and_then(|body| serde_json::from_str(&body).map_err(|e| ApiError::Other(0, format!("unexpected response: {e}"))))?;

        match response.retcode {
            0 => {}
            -101 => return Err(ApiError::Expired),
            -100 | -109 => return Err(ApiError::Rejected(response.message)),
            -110 => return Err(ApiError::Throttled),
            code => return Err(ApiError::Other(code, response.message)),
        }
        let page = response.data.ok_or_else(|| ApiError::Other(0, "no data".into()))?;
        page.list.iter().map(|record| to_pull(record, page.region_time_zone)).collect()
    }

    /// The pause the server wants between requests
    pub fn pause(&self) {
        std::thread::sleep(self.pace);
    }

    /// Which account the key belongs to, from its newest pull in any pool. `None` for an account
    /// that never warped
    pub fn account(&self) -> Result<Option<u32>, ApiError> {
        for pool in [Pool::Character, Pool::Standard, Pool::LightCone, Pool::Departure] {
            if let Some(pull) = self.page(pool, 0, 1)?.first() {
                return Ok(Some(pull.uid));
            }
            std::thread::sleep(self.pace);
        }
        Ok(None)
    }

    /// A pool's history, newest first, until a pull `known` says is already kept (it and what's
    /// older are left out) or the history ends. `progress` hears how many were fetched so far
    pub fn pool(&self, pool: Pool, known: impl Fn(i64) -> bool, mut progress: impl FnMut(usize)) -> Result<Vec<Pull>, ApiError> {
        let mut pulls = Vec::new();
        let mut end_id = 0;
        loop {
            let page = self.page(pool, end_id, 20)?;
            let Some(last) = page.last() else {
                break;
            };
            end_id = last.id.unwrap_or_default();
            let full = page.len() == 20;
            let mut reached_known = false;
            for pull in page {
                if known(pull.id.unwrap_or_default()) {
                    reached_known = true;
                    break;
                }
                pulls.push(pull);
            }
            progress(pulls.len());
            if reached_known || !full {
                break;
            }
            std::thread::sleep(self.pace);
        }
        debug!(?pool, fetched = pulls.len(), "fetched warp history");
        Ok(pulls)
    }
}

fn to_pull(record: &Record, time_zone: i32) -> Result<Pull, ApiError> {
    let bad = |what: &str| ApiError::Other(0, format!("unexpected {what} in the history: {record:?}"));
    let number = |text: &str, what: &str| text.parse::<i64>().map_err(|_| bad(what));
    let gacha_type = number(&record.gacha_type, "warp type")? as u32;
    let pool = Pool::from_gacha_type(gacha_type).ok_or_else(|| bad("warp type"))?;
    // The server's own time of day, `time_zone` hours ahead of UTC
    let local = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", &record.time)
        .ok()
        .and_then(|t| t.to_zoned_datetime(jiff::tz::TimeZone::UTC).ok())
        .ok_or_else(|| bad("time"))?;
    Ok(Pull {
        id: Some(number(&record.id, "id")?),
        uid: number(&record.uid, "uid")? as u32,
        pool,
        gacha_id: number(&record.gacha_id, "banner")? as u32,
        item_id: number(&record.item_id, "item")? as u32,
        rarity: number(&record.rank_type, "rarity")? as u8,
        time: local.timestamp().as_second() - i64::from(time_zone) * 3600,
        request_no: None,
        batch_index: 0,
        sources: source::API,
    })
}

/// Tells whether a key works, and whose it is
pub fn probe(key: &HistoryKey) -> Result<Option<u32>, ApiError> {
    let account = Client::new(key.clone()).account()?;
    info!(?account, "authkey works");
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_read_in_the_servers_zone() {
        let record = Record {
            uid: "602857786".into(),
            gacha_id: "3116".into(),
            gacha_type: "12".into(),
            item_id: "21008".into(),
            time: "2026-04-26 18:27:09".into(),
            rank_type: "4".into(),
            id: "1777245000000166586".into(),
        };
        let pull = to_pull(&record, -5).unwrap();
        // 23:27:09 UTC, the same as Star Rail Station recorded it
        assert_eq!(pull.time, 1777246029);
        assert_eq!(pull.pool, Pool::LightCone);
    }

    #[test]
    fn accounts_find_their_server() {
        assert_eq!(HistoryKey::for_account(602857786, String::new()).game_biz, "hkrpg_global");
        assert_eq!(HistoryKey::for_account(100000001, String::new()).game_biz, "hkrpg_cn");
    }
}
