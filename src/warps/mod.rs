//! Warp history: every pull an account made, kept in a local database and brought up to date from
//! three sources.
//!
//! - **Live**: the capture sees each pull as it happens ([`WarpEvent::Pulled`]). The packets don't
//!   carry the pull's id, so a live pull is kept by the account's warp request counter until the
//!   game's history has it.
//! - **The game's history** ([`api`]): what the in-game warp history shows, fetched with the
//!   authkey the game requests when that page opens ([`authkey`]). It lags behind by up to an hour
//!   and only reaches back about six months, but it has the ids, which [`reconcile`] hands to the
//!   live pulls they belong to.
//! - **Star Rail Station backups** ([`srs`]): for history older than the game keeps.
//!
//! [`store`] holds it all, [`stats`] works out pity and 50/50s, and [`service`] runs it off the
//! threads that capture and draw.
//!
//! # Pull ids
//!
//! A pull's id is 19 digits: the UTC hour it was made in (Unix seconds plus 600, so it reads as
//! HH:10:00), a 7 digit sequence shared with the other players on the server that starts over every
//! hour, and the last two digits of the account's uid. The sequence makes ids sort in the order the
//! pulls were made, and the pulls of one request are consecutive ([`ID_STEP`] apart).

// Keeping warps is the window's: the headless capture only reports them
#[cfg(feature = "gui")]
pub mod api;
#[cfg(feature = "gui")]
pub mod authkey;
#[cfg(feature = "gui")]
pub mod csv;
#[cfg(feature = "gui")]
pub mod odds;
#[cfg(feature = "gui")]
pub mod reconcile;
#[cfg(feature = "gui")]
pub mod service;
#[cfg(feature = "gui")]
pub mod srs;
#[cfg(feature = "gui")]
pub mod stats;
#[cfg(feature = "gui")]
pub mod store;

use jiff::Timestamp;

/// The warp types the game keeps separate pity for, numbered as the game's history numbers them
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pool {
    Standard = 1,
    Departure = 2,
    Character = 11,
    LightCone = 12,
    CollabCharacter = 21,
    CollabLightCone = 22,
}

impl Pool {
    /// In the order the window lists them
    pub const ALL: [Pool; 6] = [
        Pool::Character,
        Pool::LightCone,
        Pool::Standard,
        Pool::CollabCharacter,
        Pool::CollabLightCone,
        Pool::Departure,
    ];

    pub fn from_gacha_type(gacha_type: u32) -> Option<Pool> {
        Some(match gacha_type {
            1 => Pool::Standard,
            2 => Pool::Departure,
            11 => Pool::Character,
            12 => Pool::LightCone,
            21 => Pool::CollabCharacter,
            22 => Pool::CollabLightCone,
            _ => return None,
        })
    }

    /// Which pool a banner belongs to. Banner ids count up from a different thousand for each
    pub fn from_gacha_id(gacha_id: u32) -> Option<Pool> {
        Some(match gacha_id {
            1001 => Pool::Standard,
            4001 => Pool::Departure,
            2000..=2999 => Pool::Character,
            3000..=3999 => Pool::LightCone,
            5000..=5999 => Pool::CollabCharacter,
            6000..=6999 => Pool::CollabLightCone,
            _ => return None,
        })
    }

    pub fn gacha_type(self) -> u32 {
        self as u32
    }

    pub fn label(self) -> &'static str {
        match self {
            Pool::Standard => "Stellar Warp",
            Pool::Departure => "Departure Warp",
            Pool::Character => "Character",
            Pool::LightCone => "Light Cone",
            Pool::CollabCharacter => "Collab Character",
            Pool::CollabLightCone => "Collab Light Cone",
        }
    }

    /// The pull a 5 star is guaranteed by
    pub fn hard_pity(self) -> u32 {
        match self {
            Pool::LightCone | Pool::CollabLightCone => 80,
            Pool::Departure => 50,
            _ => 90,
        }
    }

    /// Whether a 5 star can be the featured one or not
    pub fn has_rate_up(self) -> bool {
        !matches!(self, Pool::Standard | Pool::Departure)
    }

    /// Collab warps are listed by a history of their own
    pub fn is_collab(self) -> bool {
        matches!(self, Pool::CollabCharacter | Pool::CollabLightCone)
    }

    /// Whether the pool's 5 stars are light cones rather than characters
    pub fn is_light_cone(self) -> bool {
        matches!(self, Pool::LightCone | Pool::CollabLightCone)
    }

    /// How many copies of a 5 star count: up to S5 for a light cone, E6 for a character
    pub fn max_copies(self) -> u32 {
        if self.is_light_cone() { 5 } else { 7 }
    }

    /// What having `copies` of a 5 star (at least 1) is called: S1 for a light cone, E0 for a
    /// character
    pub fn copy_level(self, copies: u32) -> String {
        if self.is_light_cone() {
            format!("S{copies}")
        } else {
            format!("E{}", copies.saturating_sub(1))
        }
    }

    /// The pass a warp on it costs
    pub fn pass(self) -> u32 {
        if self.has_rate_up() { item::SPECIAL_PASS } else { item::PASS }
    }
}

/// The pull a 4 star is guaranteed by, in every pool
pub const FOUR_STAR_PITY: u32 = 10;

/// The game's ids for the items warps are bought with
pub mod item {
    /// Star Rail Pass, for the standard and departure warps
    pub const PASS: u32 = 101;
    /// Star Rail Special Pass, for the event warps
    pub const SPECIAL_PASS: u32 = 102;
    pub const UNDYING_STARLIGHT: u32 = 252;
}

/// Where a pull is known from, as bits
pub mod source {
    pub const API: u8 = 1;
    pub const SRS: u8 = 2;
    pub const LIVE: u8 = 4;
    /// A CSV the user imported
    pub const CSV: u8 = 8;
}

/// One pull
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pull {
    /// The game's id for the pull, once known. A live pull gets it once the history has caught up
    pub id: Option<i64>,
    pub uid: u32,
    pub pool: Pool,
    pub gacha_id: u32,
    pub item_id: u32,
    pub rarity: u8,
    /// When it was pulled. Kept to the second
    pub time: Timestamp,
    /// The account's warp request counter, for pulls seen live
    pub request_no: Option<u32>,
    /// Where it came in its request, 0 for a single pull
    pub batch_index: u8,
    /// [`source`] bits
    pub sources: u8,
}

impl Pull {
    /// The order pulls were made in: known ids sort among themselves, pulls without one by when
    /// they were seen, after any pull with an id from the same second
    pub fn sort_key(&self) -> (Timestamp, i64, u32, u8) {
        (
            self.time,
            self.id.unwrap_or(i64::MAX),
            self.request_no.unwrap_or(u32::MAX),
            self.batch_index,
        )
    }
}

/// What the capture saw of warps
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WarpEvent {
    /// The account warped
    Pulled(LiveRequest),
    /// The game asked for the authkey its warp history page opens with
    Authkey { uid: u32, authkey: String },
    /// The banners that are up, sent when the warp screen opens
    Banners { banners: Vec<BannerInfo> },
}

/// One warp request as the capture saw it: a single pull or a ten-pull
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveRequest {
    pub uid: u32,
    pub gacha_id: u32,
    /// The account's warp request counter, sent with the request
    pub request_no: Option<u32>,
    /// What was pulled, in the order the game listed it
    pub items: Vec<u32>,
    /// When the capture saw it, on this computer's clock
    pub captured_at: Timestamp,
}

/// A banner and what it features
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerInfo {
    pub gacha_id: u32,
    pub begin: Timestamp,
    /// `None` when the banner doesn't end
    pub end: Option<Timestamp>,
    /// The items with a raised rate, 5 and 4 stars
    pub rate_up: Vec<u32>,
}

/// How far apart consecutive pull ids are: the sequence sits above the uid's two digits
pub const ID_STEP: i64 = 100;

/// The part of a pull id that tells whose it is: the last two digits, the account uid's last two
pub fn id_suffix(id: i64) -> i64 {
    id.rem_euclid(ID_STEP)
}

/// An account's uid, and what it tells: the servers it plays on, and how its pull ids end
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uid(pub u32);

impl Uid {
    /// Mainland China accounts start with 1, 2 or 5 and use their own servers
    pub fn is_mainland(self) -> bool {
        let uid = self.0;
        let first_digit = uid.checked_ilog10().map_or(0, |digits| uid / 10u32.pow(digits));
        matches!(first_digit, 1 | 2 | 5)
    }

    /// How every pull id of the account ends ([`id_suffix`])
    pub fn id_suffix(self) -> i64 {
        i64::from(self.0) % ID_STEP
    }

    /// Whether the pull id can be the account's
    pub fn owns(self, id: i64) -> bool {
        id_suffix(id) == self.id_suffix()
    }
}

/// A `2024-05-01 12:00:00` time, without the zone it's in
fn parse_datetime(text: &str) -> Option<jiff::civil::DateTime> {
    jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", text).ok()
}

/// Unix seconds as a time, for what's kept as seconds. Out of jiff's range, the nearest it has
pub fn from_second(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap_or(if seconds < 0 { Timestamp::MIN } else { Timestamp::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banners_belong_to_the_pool_of_their_thousand() {
        assert_eq!(Pool::from_gacha_id(1001), Some(Pool::Standard));
        assert_eq!(Pool::from_gacha_id(2142), Some(Pool::Character));
        assert_eq!(Pool::from_gacha_id(3116), Some(Pool::LightCone));
        assert_eq!(Pool::from_gacha_id(5001), Some(Pool::CollabCharacter));
        assert_eq!(Pool::from_gacha_id(6001), Some(Pool::CollabLightCone));
        assert_eq!(Pool::from_gacha_id(4001), Some(Pool::Departure));
        assert_eq!(Pool::from_gacha_id(7001), None);
    }

    #[test]
    fn ids_end_in_the_uids_last_two_digits() {
        assert!(Uid(602857786).owns(1777245000000166886));
        assert!(!Uid(602857785).owns(1777245000000166886));
        assert!(Uid(100000001).is_mainland() && !Uid(602857786).is_mainland());
    }
}
