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
    pub rarity: u8,
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
    /// The latest pulls, newest first, with their pity: (pull, pity5 at it)
    pub recent: Vec<(Pull, u32, Outcome)>,
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

/// Works out the stats of one pool from its pulls, which can come in any order
pub fn pool_stats(pool: Pool, pulls: &[&Pull], rate_ups: &RateUps) -> PoolStats {
    let mut ordered: Vec<&Pull> = pulls.iter().copied().filter(|p| p.pool == pool).collect();
    ordered.sort_by_key(|p| p.sort_key());

    let mut stats = PoolStats {
        pool,
        total: ordered.len(),
        pity5: 0,
        pity4: 0,
        guarantee5: pool.has_rate_up().then_some(false),
        guarantee4: pool.has_rate_up().then_some(false),
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

        let outcome = match pull.rarity {
            5 | 4 if pool.has_rate_up() => {
                let guarantee = if pull.rarity == 5 {
                    &mut stats.guarantee5
                } else {
                    &mut stats.guarantee4
                };
                let outcome = match featured(pull, rate_ups) {
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
                    Outcome::Unknown if featured(pull, rate_ups) == Some(true) => Some(false),
                    _ => None,
                };
                outcome
            }
            5 | 4 => Outcome::None,
            _ => Outcome::None,
        };

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
                    rarity: 5,
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
            let shown_pity = match pull.rarity {
                5 => pity5,
                4 => pity4,
                _ => pity5,
            };
            stats.recent.push(((*pull).clone(), shown_pity, outcome));
        }
    }

    if stats.count5 > 0 {
        stats.avg_pity5 = Some(pity5_sum as f32 / stats.count5 as f32);
    }
    stats.fives.reverse();
    stats.recent.reverse();
    stats
}

/// Whether the pull is what its banner featured, if that's known
fn featured(pull: &Pull, rate_ups: &RateUps) -> Option<bool> {
    if let Some(known) = rate_ups.get(&pull.gacha_id) {
        if known.items.contains(&pull.item_id) {
            return Some(true);
        }
        if known.not.contains(&pull.item_id) {
            return Some(false);
        }
        // A list may only have a banner's 5 stars or only its 4 stars, it says nothing about the other
        if known.items.iter().any(|&item| rarity_hint(item) == Some(pull.rarity)) {
            return Some(false);
        }
    }
    // Without a record, the banner's 5 star is anything but the standard ones. The 50/50 has been
    // lost to more than those for a while, so this only stands in until the capture sees the banner
    match pull.rarity {
        5 => Some(!STANDARD_FIVE_STARS.contains(&pull.item_id)),
        _ => None,
    }
}

/// The rarity of an item from the game's data
fn rarity_hint(item_id: u32) -> Option<u8> {
    reliquary_archiver::export::database::get_database().warp_item_rarity(item_id)
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
        let stats = pool_stats(Pool::Standard, &refs, &RateUps::new());
        assert_eq!((stats.pity5, stats.pity4), (1, 3));
        assert_eq!(stats.fives[0].pity, 4);
        assert_eq!(stats.fives[0].outcome, Outcome::None);
    }

    #[test]
    fn losing_a_fifty_fifty_guarantees_the_next() {
        let pulls = [
            pull(1, 2116, 1003, 5), // Himeko on a character banner: lost
            pull(2, 2116, 20000, 3),
            pull(3, 2116, 1506, 5), // limited: guaranteed
            pull(4, 2142, 1506, 5), // limited again: a won 50/50
        ];
        let refs: Vec<&Pull> = pulls.iter().collect();
        let stats = pool_stats(Pool::Character, &refs, &RateUps::new());
        let outcomes: Vec<Outcome> = stats.fives.iter().rev().map(|h| h.outcome).collect();
        assert_eq!(outcomes, [Outcome::Lost, Outcome::Guaranteed, Outcome::Won]);
        assert_eq!(stats.guarantee5, Some(false));
        assert_eq!((stats.won5, stats.played5), (1, 2));
    }

    #[test]
    fn four_stars_need_to_know_the_banner() {
        let pulls = [pull(1, 2116, 1214, 4), pull(2, 2116, 21002, 4)];
        let refs: Vec<&Pull> = pulls.iter().collect();
        let unknown = pool_stats(Pool::Character, &refs, &RateUps::new());
        assert_eq!(unknown.guarantee4, None);

        let mut rate_ups = RateUps::new();
        rate_ups.insert(2116, Featured {
            items: [1506, 1214, 1109].into_iter().collect(),
            not: HashSet::new(),
        });
        let known = pool_stats(Pool::Character, &refs, &rate_ups);
        assert_eq!(known.guarantee4, Some(true));
    }
}
