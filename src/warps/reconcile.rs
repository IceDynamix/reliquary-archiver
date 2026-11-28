//! Gives live pulls their ids, once the game's history has them.
//!
//! The capture sees a request's banner, items and when it happened; the history has the same,
//! with ids. A live request is the history request on the same banner, with the same items, at
//! about the same time. Identical requests (single pulls of the same 3 star, minutes apart) are
//! told apart by order: both sides are in the order they were made, so they are matched in order,
//! and by how close their times are, so a request the capture missed doesn't shift the rest.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use super::{ID_STEP, Pull};

/// How far apart the capture's clock and the server's may be
pub const TOLERANCE_SECS: i64 = 15 * 60;

/// The most pulls one request makes
const MAX_REQUEST: usize = 10;

/// Splits pulls that have ids into the requests that made them, as indices into `pulls`, oldest
/// first.
///
/// One request's pulls have consecutive ids, but so can two requests' when nobody else on the
/// server pulled in between. A request's pulls also share a second (give or take one, a ten-pull
/// can straddle one) and a banner, and there are at most ten of them; two requests can't meet all
/// of that, since a pull takes longer than a second.
pub fn group_requests(pulls: &[Pull]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..pulls.len()).filter(|&i| pulls[i].id.is_some()).collect();
    order.sort_by_key(|&i| pulls[i].id);

    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let pull = &pulls[i];
        let joins = groups.last().is_some_and(|group| {
            let last = &pulls[*group.last().unwrap()];
            group.len() < MAX_REQUEST
                && last.gacha_id == pull.gacha_id
                && pull.id.unwrap() - last.id.unwrap() == ID_STEP
                && (pull.time.as_second() - last.time.as_second()).abs() <= 1
        });
        if joins {
            groups.last_mut().unwrap().push(i);
        } else {
            groups.push(vec![i]);
        }
    }
    groups
}

/// A live pull still waiting for its id, as the store keeps it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveRow {
    pub rowid: i64,
    pub gacha_id: u32,
    pub item_id: u32,
    /// Unix seconds
    pub time: i64,
    pub request_no: Option<u32>,
}

impl LiveRow {
    pub fn key(&self) -> LiveKey {
        LiveKey::new(self.gacha_id, self.time, self.request_no)
    }
}

/// What tells live requests apart: the request counter, or without one, the banner and second.
/// Two requests can't share a second, the warp animation alone takes longer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiveKey {
    Counter(u32),
    Seen { gacha_id: u32, time: i64 },
}

impl LiveKey {
    pub fn new(gacha_id: u32, time: i64, request_no: Option<u32>) -> Self {
        match request_no {
            Some(n) => LiveKey::Counter(n),
            None => LiveKey::Seen { gacha_id, time },
        }
    }
}

/// Gathers live pulls into the requests that made them, as indices into the pulls' order, each
/// request where its first pull comes
pub fn group_live(keys: impl IntoIterator<Item = LiveKey>) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut at: HashMap<LiveKey, usize> = HashMap::new();
    for (i, key) in keys.into_iter().enumerate() {
        match at.get(&key) {
            Some(&group) => groups[group].push(i),
            None => {
                at.insert(key, groups.len());
                groups.push(vec![i]);
            }
        }
    }
    groups
}

/// Which `history` pulls the `live` ones are, as (live, history) indices. `live` is in the order
/// the requests were made, by request counter
pub fn confirm(live: &[LiveRow], history: &[Pull]) -> Vec<(usize, usize)> {
    let Some(reach) = reach(live) else {
        return Vec::new();
    };
    let live_groups = group_live(live.iter().map(LiveRow::key));
    let live_requests: Vec<Request> = live_groups
        .iter()
        .map(|group| {
            let first = &live[group[0]];
            Request::new(first.gacha_id, group.iter().map(|&i| live[i].item_id).collect(), first.time)
        })
        .collect();

    // Matching weighs every live request against every history one, so the history is cut to
    // what can be any of them
    let history_groups: Vec<Vec<usize>> = group_requests(history)
        .into_iter()
        .filter(|group| reach.contains(&history[group[0]].time.as_second()))
        .collect();
    let history_requests: Vec<Request> = history_groups
        .iter()
        .map(|group| {
            let first = &history[group[0]];
            Request::new(
                first.gacha_id,
                group.iter().map(|&i| history[i].item_id).collect(),
                first.time.as_second(),
            )
        })
        .collect();

    let mut pairs = Vec::new();
    for (l, h) in match_requests(&live_requests, &history_requests) {
        let mut rows = live_groups[l].clone();
        // The history's order within the request is the real one, the items are the same
        for &i in &history_groups[h] {
            if let Some(at) = rows.iter().position(|&row| live[row].item_id == history[i].item_id) {
                pairs.push((rows.remove(at), i));
            }
        }
    }
    pairs
}

/// When history requests that can match the live pulls were made: a match is within
/// [`TOLERANCE_SECS`] of the clock skew, which is itself at most that. `None` without live pulls
fn reach(live: &[LiveRow]) -> Option<RangeInclusive<i64>> {
    let oldest = live.iter().map(|row| row.time).min()?;
    let newest = live.iter().map(|row| row.time).max()?;
    Some(oldest - 2 * TOLERANCE_SECS..=newest + 2 * TOLERANCE_SECS)
}

/// A request to match: what it pulled, and when
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub gacha_id: u32,
    /// What it pulled, sorted, since the order within a request is the history's to give
    pub items: Vec<u32>,
    /// Unix seconds
    pub time: i64,
}

impl Request {
    pub fn new(gacha_id: u32, mut items: Vec<u32>, time: i64) -> Self {
        items.sort_unstable();
        Self { gacha_id, items, time }
    }
}

/// Pairs live requests with history requests, as (live, history) indices.
///
/// Both lists are in the order the requests were made: live ones by their request counter, history
/// ones by id. Every pair matches on banner and items, pairs keep both lists' order, and of the
/// ways to pair as many as possible, the one whose times agree best wins.
pub fn match_requests(live: &[Request], history: &[Request]) -> Vec<(usize, usize)> {
    let skew = clock_skew(live, history);
    let cost = |l: &Request, h: &Request| -> Option<i64> {
        if l.gacha_id != h.gacha_id || l.items != h.items {
            return None;
        }
        let off = (h.time - l.time - skew).abs();
        (off <= TOLERANCE_SECS).then_some(off)
    };

    // best[i][j]: the most pairs, and their least total time difference, among live[i..] and history[j..]
    let (n, m) = (live.len(), history.len());
    let mut best = vec![vec![(0usize, 0i64); m + 1]; n + 1];
    let better = |a: (usize, i64), b: (usize, i64)| a.0 > b.0 || (a.0 == b.0 && a.1 < b.1);
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let mut choice = best[i + 1][j];
            if better(best[i][j + 1], choice) {
                choice = best[i][j + 1];
            }
            if let Some(c) = cost(&live[i], &history[j]) {
                let (pairs, total) = best[i + 1][j + 1];
                let paired = (pairs + 1, total + c);
                if better(paired, choice) {
                    choice = paired;
                }
            }
            best[i][j] = choice;
        }
    }

    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if let Some(c) = cost(&live[i], &history[j]) {
            let (rest, total) = best[i + 1][j + 1];
            if best[i][j] == (rest + 1, total + c) {
                pairs.push((i, j));
                i += 1;
                j += 1;
                continue;
            }
        }
        if best[i][j] == best[i + 1][j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// How far the server's clock is ahead of the capture's, from the requests that can only be each other
fn clock_skew(live: &[Request], history: &[Request]) -> i64 {
    let fits = |l: &Request, h: &Request| l.gacha_id == h.gacha_id && l.items == h.items && (h.time - l.time).abs() <= TOLERANCE_SECS;
    let mut offsets: Vec<i64> = live
        .iter()
        .filter_map(|l| {
            let mut found = history.iter().filter(|h| fits(l, h));
            let h = found.next()?;
            if found.next().is_some() || live.iter().filter(|other| fits(other, h)).count() != 1 {
                return None;
            }
            Some(h.time - l.time)
        })
        .collect();
    if offsets.is_empty() {
        return 0;
    }
    offsets.sort_unstable();
    offsets[offsets.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warps::{Pool, source};

    fn pull(id: i64, gacha_id: u32, time: i64) -> Pull {
        Pull {
            id: Some(id),
            uid: 602857786,
            pool: Pool::from_gacha_id(gacha_id).unwrap(),
            gacha_id,
            item_id: 20000,
            rarity: 3,
            time: crate::warps::from_second(time),
            request_no: None,
            batch_index: 0,
            sources: source::API,
        }
    }

    #[test]
    fn a_ten_pull_is_one_request_even_across_a_second() {
        let mut pulls: Vec<Pull> = (0..10).map(|k| pull(1777245000000100086 + k * 100, 3074, 1000 + (k / 8))).collect();
        // A single pull straight after, with the next id: nobody else pulled in between
        pulls.push(pull(1777245000000101086, 3074, 1004));
        let groups = group_requests(&pulls);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 10);
        assert_eq!(groups[1], vec![10]);
    }

    #[test]
    fn consecutive_singles_seconds_apart_are_separate_requests() {
        let pulls = [pull(100086, 1001, 50), pull(100186, 1001, 53)];
        assert_eq!(group_requests(&pulls).len(), 2);
    }

    #[test]
    fn identical_requests_pair_up_by_time_when_one_was_missed() {
        // The capture missed the first of three identical single pulls
        let history = [
            Request::new(1001, vec![20000], 1000),
            Request::new(1001, vec![20000], 1030),
            Request::new(1001, vec![20000], 1060),
        ];
        let live = [Request::new(1001, vec![20000], 1031), Request::new(1001, vec![20000], 1062)];
        assert_eq!(match_requests(&live, &history), [(0, 1), (1, 2)]);
    }

    #[test]
    fn a_skewed_clock_is_allowed_for() {
        let history = [
            Request::new(2142, vec![20011], 1000),
            Request::new(1001, vec![20000], 1100),
            Request::new(1001, vec![20000], 1130),
        ];
        // The capture's clock is 5 minutes behind
        let live = [Request::new(2142, vec![20011], 700), Request::new(1001, vec![20000], 830)];
        assert_eq!(match_requests(&live, &history), [(0, 0), (1, 2)]);
    }

    #[test]
    fn live_requests_without_a_counter_are_told_apart_by_banner_and_second() {
        let keys = [
            LiveKey::new(2142, 1000, Some(7)),
            LiveKey::new(1001, 1000, None),
            LiveKey::new(2142, 1000, None),
            LiveKey::new(2142, 1001, Some(7)),
            LiveKey::new(1001, 1000, None),
        ];
        assert_eq!(group_live(keys), [vec![0, 3], vec![1, 4], vec![2]]);
    }

    #[test]
    fn history_out_of_reach_of_the_live_pulls_is_left_out() {
        let row = |rowid, item_id, time| LiveRow {
            rowid,
            gacha_id: 1001,
            item_id,
            time,
            request_no: None,
        };
        let live = [row(1, 20000, 100_000), row(2, 20001, 100_030)];
        assert_eq!(reach(&live), Some(100_000 - 2 * TOLERANCE_SECS..=100_030 + 2 * TOLERANCE_SECS));
        assert_eq!(reach(&[]), None);

        // A long history before them, then the two
        let mut history: Vec<Pull> = (0..5000).map(|k| pull(100086 + k * ID_STEP * 3, 1001, k)).collect();
        history.push(Pull {
            item_id: 20000,
            ..pull(900_000_086, 1001, 100_002)
        });
        history.push(Pull {
            item_id: 20001,
            ..pull(900_000_386, 1001, 100_031)
        });
        assert_eq!(confirm(&live, &history), [(0, 5000), (1, 5001)]);
    }

    #[test]
    fn different_items_or_banners_never_match() {
        let history = [Request::new(1001, vec![20000], 1000)];
        assert!(match_requests(&[Request::new(1001, vec![20001], 1000)], &history).is_empty());
        assert!(match_requests(&[Request::new(2142, vec![20000], 1000)], &history).is_empty());
        assert!(match_requests(&[Request::new(1001, vec![20000], 1000 + TOLERANCE_SECS + 1)], &history).is_empty());
    }
}
