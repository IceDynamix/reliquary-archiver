//! Warps as the capture sees them: each pull as it's made, the authkey the game asks for when its
//! warp history opens, and the banners that are up.

use jiff::Timestamp;

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

/// Unix seconds as a time, for what's kept as seconds. Out of jiff's range, the nearest it has
pub fn from_second(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap_or(if seconds < 0 { Timestamp::MIN } else { Timestamp::MAX })
}
