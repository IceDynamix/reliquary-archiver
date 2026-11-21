//! Reads Star Rail Station backups (`starrailstation-backup_….dat`), for history older than the
//! game keeps.
//!
//! A backup is `srs` followed by lz-string's UTF-16 compression of a JSON document. Its profiles
//! are the site's accounts, each with its own pulls. The site doesn't record which game account a
//! profile is, but pull ids end in the last two digits of their account's uid, which tells them
//! apart in most cases.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::stats::RateUps;
use super::{Pool, Pull, source};

/// A Star Rail Station profile and its pulls
#[derive(Clone, Debug)]
pub struct Profile {
    pub key: String,
    pub name: String,
    /// Without a uid, which the profile doesn't know
    pub pulls: Vec<Pull>,
    /// What each banner featured, from the outcomes the site recorded
    pub rate_ups: RateUps,
    /// The last two digits all its pull ids end in, if they agree
    pub uid_suffix: Option<u32>,
}

impl Profile {
    /// Whether the pulls can be the account's: their ids end in the uid's last two digits
    pub fn fits(&self, uid: u32) -> bool {
        self.uid_suffix == Some(uid % 100)
    }

    /// The newest pull's time
    pub fn latest(&self) -> Option<i64> {
        self.pulls.iter().map(|p| p.time).max()
    }
}

pub fn read(path: &Path) -> Result<Vec<Profile>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("couldn't read it: {e}"))?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<Vec<Profile>, String> {
    let not_a_backup = || "it isn't a Star Rail Station backup".to_string();
    let compressed = text.strip_prefix("srs").ok_or_else(not_a_backup)?;
    let wide = lz_str::decompress_from_utf16(compressed).ok_or_else(not_a_backup)?;
    let json = String::from_utf16(&wide).map_err(|_| not_a_backup())?;
    let root: Value = serde_json::from_str(&json).map_err(|_| not_a_backup())?;

    let stores = root.pointer("/data/stores").and_then(Value::as_object).ok_or_else(not_a_backup)?;
    let names: HashMap<String, String> = root
        .get("profiles")
        .and_then(Value::as_object)
        .map(|profiles| {
            profiles
                .iter()
                .map(|(key, profile)| (key.clone(), profile.get("name").and_then(Value::as_str).unwrap_or(key).to_string()))
                .collect()
        })
        .unwrap_or_default();

    let mut profiles = Vec::new();
    for (store_name, store) in stores {
        let Some(key) = store_name.strip_suffix("_warp-v2") else {
            continue;
        };
        let mut pulls = Vec::new();
        let mut rate_ups = RateUps::new();
        for pool in Pool::ALL {
            let Some(items) = store.get(format!("items_{}", pool.gacha_type())).and_then(Value::as_array) else {
                continue;
            };
            for item in items {
                let Some((pull, result)) = to_pull(item, pool) else {
                    continue;
                };
                // The site records 2 for a won 50/50 and 3 for a guaranteed one, both featured, and
                // 4 for a lost one
                match result {
                    2 | 3 => {
                        rate_ups.entry(pull.gacha_id).or_default().items.insert(pull.item_id);
                    }
                    4 => {
                        rate_ups.entry(pull.gacha_id).or_default().not.insert(pull.item_id);
                    }
                    _ => {}
                }
                pulls.push(pull);
            }
        }
        if pulls.is_empty() {
            continue;
        }
        let suffixes: HashSet<i64> = pulls.iter().filter_map(|p| p.id).map(|id| id.rem_euclid(100)).collect();
        let uid_suffix = (suffixes.len() == 1).then(|| *suffixes.iter().next().unwrap() as u32);
        profiles.push(Profile {
            key: key.to_string(),
            name: names.get(key).cloned().unwrap_or_else(|| key.to_string()),
            pulls,
            rate_ups,
            uid_suffix,
        });
    }
    profiles.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(profiles)
}

fn to_pull(item: &Value, pool: Pool) -> Option<(Pull, i64)> {
    let number = |name: &str| item.get(name).and_then(Value::as_i64);
    // Pulls the user typed in themselves have no id from the game
    if item.get("manual").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let id = item.get("uid")?.as_str()?.parse::<i64>().ok()?;
    let pull = Pull {
        id: Some(id),
        uid: 0,
        pool,
        gacha_id: number("gachaId")? as u32,
        item_id: number("itemId")? as u32,
        rarity: number("rarity")? as u8,
        time: number("timestamp")? / 1000,
        request_no: None,
        batch_index: 0,
        sources: source::SRS,
    };
    Some((pull, number("result").unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a real backup, when `SRS_BACKUP` names one
    #[test]
    fn reads_a_real_backup() {
        let Ok(path) = std::env::var("SRS_BACKUP") else {
            return;
        };
        let profiles = read(Path::new(&path)).unwrap();
        for profile in &profiles {
            println!(
                "{} {:?}: {} pulls, suffix {:?}, {} banners with rate-ups",
                profile.key,
                profile.name,
                profile.pulls.len(),
                profile.uid_suffix,
                profile.rate_ups.len()
            );
        }
        assert!(profiles.iter().any(|p| !p.pulls.is_empty()));
    }

    #[test]
    fn other_files_are_refused() {
        assert!(parse("{}").is_err());
        assert!(parse("srs not compressed").is_err());
    }
}

/// Checks the stats against what Star Rail Station worked out for the same pulls
#[cfg(test)]
mod agreement {
    use super::*;
    use crate::warps::stats::{Outcome, pool_stats};

    #[test]
    fn pity_and_outcomes_agree_with_star_rail_station() {
        let Ok(path) = std::env::var("SRS_BACKUP") else {
            return;
        };
        let text = std::fs::read_to_string(path).unwrap();
        let wide = lz_str::decompress_from_utf16(text.strip_prefix("srs").unwrap()).unwrap();
        let root: Value = serde_json::from_str(&String::from_utf16(&wide).unwrap()).unwrap();
        let profile = parse(&text).unwrap().into_iter().max_by_key(|p| p.pulls.len()).unwrap();
        let store = &root["data"]["stores"][format!("{}_warp-v2", profile.key)];

        let refs: Vec<&Pull> = profile.pulls.iter().collect();
        for pool in Pool::ALL {
            let t = pool.gacha_type();
            let Some(theirs) = store["types"].get(t.to_string()) else {
                continue;
            };
            let ours = pool_stats(pool, &refs, &profile.rate_ups);
            println!(
                "{pool:?}: pity5 {} vs {}, pity4 {} vs {}, guarantee5 {:?} vs {}",
                ours.pity5, theirs["pity5"], ours.pity4, theirs["pity4"], ours.guarantee5, theirs["guarantee5"]
            );
            assert_eq!(i64::from(ours.pity5), theirs["pity5"].as_i64().unwrap(), "{pool:?} 5 star pity");
            assert_eq!(i64::from(ours.pity4), theirs["pity4"].as_i64().unwrap(), "{pool:?} 4 star pity");
            assert_eq!(ours.count5 as i64, theirs["pullCount5"].as_i64().unwrap(), "{pool:?} 5 stars");
            if pool.has_rate_up() {
                assert_eq!(ours.guarantee5, theirs["guarantee5"].as_bool(), "{pool:?} guarantee");
                assert_eq!(ours.played5 as i64, theirs["rateupChallenges"].as_i64().unwrap(), "{pool:?} 50/50s");
                assert_eq!(ours.won5 as i64, theirs["rateupWins"].as_i64().unwrap(), "{pool:?} won");
            }

            // Each 5 star's pity and outcome, oldest first on both sides
            let mut recorded: Vec<&Value> = store[format!("items_{t}")]
                .as_array()
                .unwrap()
                .iter()
                .filter(|i| i["rarity"] == 5)
                .collect();
            recorded.sort_by_key(|i| i["uid"].as_str().unwrap().parse::<i64>().unwrap());
            assert_eq!(recorded.len(), ours.fives.len());
            for (five, theirs) in ours.fives.iter().rev().zip(recorded) {
                assert_eq!(i64::from(five.item_id), theirs["itemId"].as_i64().unwrap());
                assert_eq!(
                    i64::from(five.pity),
                    theirs["pity5"].as_i64().unwrap(),
                    "{pool:?} pity of {} at {}",
                    five.item_id,
                    five.time
                );
                let expected = match theirs["result"].as_i64().unwrap() {
                    2 => Outcome::Won,
                    3 => Outcome::Guaranteed,
                    4 => Outcome::Lost,
                    _ => Outcome::None,
                };
                assert_eq!(five.outcome, expected, "{pool:?} outcome of {} at {}", five.item_id, five.time);
            }
        }
    }
}
