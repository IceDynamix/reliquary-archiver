//! Pity and 50/50s, worked out from the pulls in the order they were made.
//!
//! The counters work as the game's do: every pull adds one to both, a 4 star resets the 4 star
//! counter and a 5 star the 5 star one (a 5 star leaves the 4 star counter running). A pull's pity
//! is the count it was made at.
//!
//! A 5 star from a banner with a raised rate either is the featured one (won the 50/50, or came
//! guaranteed after losing the last one) or isn't (lost). Which items a banner features comes from
//! what the capture saw of it, from Star Rail Station's records, or, for 5 stars, from not being one
//! of the standard ones.

use std::collections::{HashMap, HashSet};

use super::{Pool, Pull};

/// The 5 stars the standard warp gives, which a featured banner gives when its 50/50 is lost
pub const STANDARD_FIVE_STARS: [u32; 14] = [
    // Himeko, Welt, Bronya, Gepard, Clara, Yanqing, Bailu
    1003, 1004, 1101, 1104, 1107, 1209, 1211, // Their light cones
    23000, 23002, 23003, 23004, 23005, 23012, 23013,
];

/// How a 4 or 5 star turned out on a banner with a raised rate
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The featured item, on a 50/50
    Won,
    /// Not the featured item, so the next one is guaranteed
    Lost,
    /// The featured item, guaranteed by losing the last 50/50
    Guaranteed,
    /// What the banner featured isn't known
    Unknown,
    /// The banner has no raised rate (standard, departure)
    None,
}

/// A 4 or 5 star and the pity it came at
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub item_id: u32,
    pub pity: u32,
    pub outcome: Outcome,
    pub time: i64,
    /// The game's history has it, it isn't only known from the capture
    pub confirmed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoolStats {
    pub pool: Pool,
    pub total: usize,
    /// Pulls since the last 5 star and 4 star
    pub pity5: u32,
    pub pity4: u32,
    /// The next one is guaranteed to be featured, when that's known
    pub guarantee5: Option<bool>,
    pub guarantee4: Option<bool>,
    pub count5: usize,
    pub count4: usize,
    pub avg_pity5: Option<f32>,
    /// 50/50s won and played, over the 5 stars where it's known
    pub won5: usize,
    pub played5: usize,
    /// The 5 stars, newest first
    pub fives: Vec<Highlight>,
    /// The latest pulls, newest first
    pub recent: Vec<RecentPull>,
}

/// One of a pool's latest pulls
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentPull {
    pub pull: Pull,
    /// The count it was made at: the 4 star counter's for a 4 star, the 5 star counter's otherwise
    pub pity: u32,
    pub outcome: Outcome,
}

/// What's known of what a banner features
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Featured {
    /// Items it features
    pub items: HashSet<u32>,
    /// Items it gives when a 50/50 is lost
    pub not: HashSet<u32>,
}

/// What banners feature, by banner id
pub type RateUps = HashMap<u32, Featured>;

/// How many of the latest pulls [`PoolStats::recent`] keeps
pub const RECENT: usize = 200;

/// Works out the stats of one pool from its pulls, which can come in any order. `complete` says
/// they reach back to the account's first warp, so nothing was guaranteed before them. `rarity`
/// gives an item's star rating, for banners whose list only has some of what they feature
pub fn pool_stats(pool: Pool, pulls: &[&Pull], rate_ups: &RateUps, complete: bool, rarity: impl Fn(u32) -> Option<u8>) -> PoolStats {
    let mut ordered: Vec<&Pull> = pulls.iter().copied().filter(|p| p.pool == pool).collect();
    ordered.sort_by_key(|p| p.sort_key());

    let mut walk = Walk::new(pool, complete);
    let mut stats = PoolStats {
        pool,
        total: ordered.len(),
        pity5: 0,
        pity4: 0,
        guarantee5: walk.guarantee5,
        guarantee4: walk.guarantee4,
        count5: 0,
        count4: 0,
        avg_pity5: None,
        won5: 0,
        played5: 0,
        fives: Vec::new(),
        recent: Vec::new(),
    };
    let mut pity5_sum = 0u32;
    let recent_from = ordered.len().saturating_sub(RECENT);

    for (i, pull) in ordered.iter().enumerate() {
        stats.pity4 += 1;
        stats.pity5 += 1;
        let (pity4, pity5) = (stats.pity4, stats.pity5);
        let (outcome, _) = walk.step(pull, rate_ups, &rarity);

        match pull.rarity {
            5 => {
                stats.count5 += 1;
                pity5_sum += pity5;
                if matches!(outcome, Outcome::Won | Outcome::Lost) {
                    stats.played5 += 1;
                    stats.won5 += usize::from(outcome == Outcome::Won);
                }
                stats.fives.push(Highlight {
                    item_id: pull.item_id,
                    pity: pity5,
                    outcome,
                    time: pull.time,
                    confirmed: pull.id.is_some(),
                });
                stats.pity5 = 0;
            }
            4 => {
                stats.count4 += 1;
                stats.pity4 = 0;
            }
            _ => {}
        }

        if i >= recent_from {
            stats.recent.push(RecentPull {
                pull: (*pull).clone(),
                pity: if pull.rarity == 4 { pity4 } else { pity5 },
                outcome,
            });
        }
    }

    (stats.guarantee5, stats.guarantee4) = (walk.guarantee5, walk.guarantee4);
    if stats.count5 > 0 {
        stats.avg_pity5 = Some(pity5_sum as f32 / stats.count5 as f32);
    }
    stats.fives.reverse();
    stats.recent.reverse();
    stats
}

/// Each pull's outcome, in the order of `pulls`, which can hold any pools in any order. An outcome
/// that rests on the standard 5 stars rather than on what's known of the banner is `Unknown`, for
/// keeping outcomes somewhere they'd be taken as known. `complete` as for [`pool_stats`]
pub fn recorded_outcomes(pulls: &[Pull], rate_ups: &RateUps, complete: bool, rarity: impl Fn(u32) -> Option<u8>) -> Vec<Outcome> {
    let mut outcomes = vec![Outcome::None; pulls.len()];
    for pool in Pool::ALL {
        let mut ordered: Vec<usize> = (0..pulls.len()).filter(|&i| pulls[i].pool == pool).collect();
        ordered.sort_by_key(|&i| pulls[i].sort_key());
        let mut walk = Walk::new(pool, complete);
        for i in ordered {
            outcomes[i] = match walk.step(&pulls[i], rate_ups, &rarity) {
                (_, Some(Basis::Guess)) => Outcome::Unknown,
                (outcome, _) => outcome,
            };
        }
    }
    outcomes
}

/// The guarantees through a pool's pulls, taken in the order they were made
struct Walk {
    pool: Pool,
    guarantee5: Option<bool>,
    guarantee4: Option<bool>,
}

impl Walk {
    /// From the account's first warp when `complete`, when nothing is guaranteed. Otherwise what
    /// came before is unknown, until a 50/50 tells
    fn new(pool: Pool, complete: bool) -> Self {
        let start = (pool.has_rate_up() && complete).then_some(false);
        Walk {
            pool,
            guarantee5: start,
            guarantee4: start,
        }
    }

    /// The next pull's outcome, and what told whether it was featured
    fn step(&mut self, pull: &Pull, rate_ups: &RateUps, rarity: impl Fn(u32) -> Option<u8>) -> (Outcome, Option<Basis>) {
        if !matches!(pull.rarity, 5 | 4) || !self.pool.has_rate_up() {
            return (Outcome::None, None);
        }
        let guarantee = if pull.rarity == 5 {
            &mut self.guarantee5
        } else {
            &mut self.guarantee4
        };
        // Not being a standard 5 star doesn't make one featured: a lost 50/50 can give others too.
        // Being one still means it was lost
        let found = featured(pull, rate_ups, rarity).filter(|&(featured, basis)| !(featured && basis == Basis::Guess));
        let featured = found.map(|(featured, _)| featured);
        let outcome = match featured {
            None => Outcome::Unknown,
            Some(false) => Outcome::Lost,
            Some(true) if *guarantee == Some(true) => Outcome::Guaranteed,
            // Without knowing the last one, a featured item may have been guaranteed
            Some(true) if guarantee.is_none() => Outcome::Unknown,
            Some(true) => Outcome::Won,
        };
        *guarantee = match outcome {
            Outcome::Lost => Some(true),
            Outcome::Won | Outcome::Guaranteed => Some(false),
            // A featured item ends any guarantee, whether it was one or not
            Outcome::Unknown if featured == Some(true) => Some(false),
            _ => None,
        };
        (outcome, found.map(|(_, basis)| basis))
    }
}

/// What told whether a pull was featured
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Basis {
    /// What's known of its banner
    Record,
    /// Not being one of the standard 5 stars
    Guess,
}

/// Whether the pull is what its banner featured, if that's known, and what told. `rarity` gives an
/// item's star rating
fn featured(pull: &Pull, rate_ups: &RateUps, rarity: impl Fn(u32) -> Option<u8>) -> Option<(bool, Basis)> {
    if let Some(known) = rate_ups.get(&pull.gacha_id) {
        if known.items.contains(&pull.item_id) {
            return Some((true, Basis::Record));
        }
        if known.not.contains(&pull.item_id) {
            return Some((false, Basis::Record));
        }
        // A list may only have a banner's 5 stars or only its 4 stars, it says nothing about the other
        if known.items.iter().any(|&item| rarity(item) == Some(pull.rarity)) {
            return Some((false, Basis::Record));
        }
    }
    // Without a record, the banner's 5 star is anything but the standard ones. The 50/50 has been
    // lost to more than those for a while, so this only stands in until the capture sees the banner
    match pull.rarity {
        5 => Some((!STANDARD_FIVE_STARS.contains(&pull.item_id), Basis::Guess)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warps::source;

    fn pull(id: i64, gacha_id: u32, item_id: u32, rarity: u8) -> Pull {
        Pull {
            id: Some(id),
            uid: 1,
            pool: Pool::from_gacha_id(gacha_id).unwrap(),
            gacha_id,
            item_id,
            rarity,
            time: id,
            request_no: None,
            batch_index: 0,
            sources: source::API,
        }
    }

    #[test]
    fn a_five_star_resets_only_its_own_counter() {
        let pulls = [
            pull(1, 1001, 20000, 3),
            pull(2, 1001, 1013, 4),
            pull(3, 1001, 20001, 3),
            pull(4, 1001, 1003, 5),
            pull(5, 1001, 20002, 3),
        ];
        let refs: Vec<&Pull> = pulls.iter().collect();
        let stats = pool_stats(Pool::Standard, &refs, &RateUps::new(), true, |_| None);
        assert_eq!((stats.pity5, stats.pity4), (1, 3));
        assert_eq!(stats.fives[0].pity, 4);
        assert_eq!(stats.fives[0].outcome, Outcome::None);
    }

    #[test]
    fn losing_a_fifty_fifty_guarantees_the_next() {
        let pulls = [
            pull(1, 2116, 1003, 5), // Himeko on a character banner: lost
            pull(2, 2116, 20000, 3),
            pull(3, 2116, 1506, 5), // featured: guaranteed
            pull(4, 2142, 1506, 5), // featured again: a won 50/50
        ];
        let refs: Vec<&Pull> = pulls.iter().collect();
        let mut rate_ups = RateUps::new();
        for gacha_id in [2116, 2142] {
            rate_ups.insert(gacha_id, Featured {
                items: HashSet::from([1506]),
                not: HashSet::new(),
            });
        }
        let stats = pool_stats(Pool::Character, &refs, &rate_ups, true, |_| Some(5));
        let outcomes: Vec<Outcome> = stats.fives.iter().rev().map(|h| h.outcome).collect();
        assert_eq!(outcomes, [Outcome::Lost, Outcome::Guaranteed, Outcome::Won]);
        assert_eq!(stats.guarantee5, Some(false));
        assert_eq!((stats.won5, stats.played5), (1, 2));
    }

    #[test]
    fn four_stars_need_to_know_the_banner() {
        let pulls = [pull(1, 2116, 1214, 4), pull(2, 2116, 21002, 4)];
        let refs: Vec<&Pull> = pulls.iter().collect();
        let unknown = pool_stats(Pool::Character, &refs, &RateUps::new(), true, |_| None);
        assert_eq!(unknown.guarantee4, None);

        let mut rate_ups = RateUps::new();
        rate_ups.insert(2116, Featured {
            items: [1506, 1214, 1109].into_iter().collect(),
            not: HashSet::new(),
        });
        // 1506 is a 5 star, 1214 and 1109 are 4 stars
        let rarity = |item| Some(if item == 1506 { 5 } else { 4 });
        let known = pool_stats(Pool::Character, &refs, &rate_ups, true, rarity);
        assert_eq!(known.guarantee4, Some(true));
    }

    #[test]
    fn what_was_guaranteed_before_the_history_is_unknown() {
        let pulls = [pull(1, 2116, 1506, 5), pull(2, 2116, 1003, 5), pull(3, 2116, 1506, 5)];
        let refs: Vec<&Pull> = pulls.iter().collect();
        let mut rate_ups = RateUps::new();
        rate_ups.insert(2116, Featured {
            items: HashSet::from([1506]),
            not: HashSet::new(),
        });
        let stats = pool_stats(Pool::Character, &refs, &rate_ups, false, |_| Some(5));
        let outcomes: Vec<Outcome> = stats.fives.iter().rev().map(|h| h.outcome).collect();
        // The first may have come guaranteed; the loss after it tells the rest
        assert_eq!(outcomes, [Outcome::Unknown, Outcome::Lost, Outcome::Guaranteed]);
        assert_eq!(stats.guarantee5, Some(false));
    }

    #[test]
    fn a_five_star_only_guessed_featured_leaves_the_guarantee_unknown() {
        // No record of the banner: 1506 isn't a standard 5 star, which doesn't make it featured
        let pulls = [pull(1, 2116, 1506, 5)];
        let refs: Vec<&Pull> = pulls.iter().collect();
        let stats = pool_stats(Pool::Character, &refs, &RateUps::new(), true, |_| None);
        assert_eq!(stats.fives[0].outcome, Outcome::Unknown);
        assert_eq!(stats.guarantee5, None);
    }
}
