//! The warp history as CSV, one pull a row, to keep a copy of it or move it to another tool, and to
//! read one back.
//!
//! The first row names the columns, and reading goes by those names, so the columns can come in any
//! order and extra ones are ignored. Reading needs `id`, `gacha_type`, `gacha_id`, `item_id` and
//! `time`; `rarity` is looked up when it's missing. Times are written in UTC, as RFC 3339, and read
//! as that, as `YYYY-MM-DD HH:MM:SS` in UTC, or as Unix seconds.
//!
//! Pulls seen live that the game's history doesn't have yet are written without an id. Reading skips
//! them, as an import from anywhere else would: the game's history fills them in.
//!
//! `result` is how a 4 or 5 star on a banner with a raised rate turned out: `won`, `lost` or
//! `guaranteed`, or empty when that isn't known from a record of the banner. Reading it tells what
//! banners featured, as a Star Rail Station backup does.

use super::stats::{Outcome, RateUps};
use super::{Pool, Pull};

/// The columns written, in order
const COLUMNS: [&str; 12] = [
    "id",
    "uid",
    "gacha_type",
    "pool",
    "gacha_id",
    "item_id",
    "name",
    "rarity",
    "time",
    "request_no",
    "batch_index",
    "result",
];

/// The pulls as CSV, in the order given, with `outcomes` their outcomes in the same order and
/// `name` naming each item
pub fn write(pulls: &[Pull], outcomes: &[Outcome], name: impl Fn(u32) -> Option<String>) -> String {
    let mut out = COLUMNS.join(",");
    out.push('\n');
    for (i, pull) in pulls.iter().enumerate() {
        let time = pull.time.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
        let fields = [
            pull.id.map_or_else(String::new, |id| id.to_string()),
            pull.uid.to_string(),
            pull.pool.gacha_type().to_string(),
            pull.pool.label().to_string(),
            pull.gacha_id.to_string(),
            pull.item_id.to_string(),
            name(pull.item_id).unwrap_or_default(),
            pull.rarity.to_string(),
            time,
            pull.request_no.map_or_else(String::new, |n| n.to_string()),
            pull.batch_index.to_string(),
            match outcomes.get(i) {
                Some(Outcome::Won) => "won",
                Some(Outcome::Lost) => "lost",
                Some(Outcome::Guaranteed) => "guaranteed",
                _ => "",
            }
            .to_string(),
        ];
        let row: Vec<String> = fields.iter().map(|field| quoted(field)).collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// A field as CSV writes it: in quotes, its own quotes doubled, when it holds a comma, a quote or
/// a line break
fn quoted(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// What reading a CSV found
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CsvImport {
    /// The pulls with an id, `uid` 0 (the account they go into decides it)
    pub pulls: Vec<Pull>,
    /// Rows without an id: pulls the game's history didn't have yet when they were written
    pub without_id: usize,
    /// What each banner featured, from the `result` column
    pub rate_ups: RateUps,
}

/// The pulls in a CSV. `rarity` gives an item's rarity when the file has no `rarity` column, or
/// an empty one. Fails on the first row it can't read, naming it
pub fn read(text: &str, rarity: impl Fn(u32) -> Option<u8>) -> Result<CsvImport, String> {
    let mut rows = rows(text.trim_start_matches('\u{feff}'));
    let header = rows.next().ok_or("the file is empty")?;
    let column = |name: &str| header.iter().position(|h| h.trim().eq_ignore_ascii_case(name));
    let required = ["id", "gacha_type", "gacha_id", "item_id", "time"];
    let [Some(id), Some(gacha_type), Some(gacha_id), Some(item_id), Some(time)] = required.map(column) else {
        let missing: Vec<&str> = required.into_iter().filter(|name| column(name).is_none()).collect();
        return Err(format!("it has no {} column", missing.join(", ")));
    };
    let (rarity_at, request_no, batch_index) = (column("rarity"), column("request_no"), column("batch_index"));
    let result = column("result");

    let mut read = CsvImport::default();
    for (n, row) in rows.enumerate() {
        // Line numbers as a spreadsheet shows them, the header being line 1
        let line = n + 2;
        if row.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let opt_field = |i: Option<usize>| i.and_then(|i| row.get(i)).map(|f| f.trim()).filter(|f| !f.is_empty());
        let field = |i: usize| opt_field(Some(i));
        let bad = |what: &str| format!("row {line} has no usable {what}");
        let Some(id) = field(id) else {
            read.without_id += 1;
            continue;
        };
        let id: i64 = id.parse().map_err(|_| bad("id"))?;
        let pool = field(gacha_type)
            .and_then(|t| t.parse().ok())
            .and_then(Pool::from_gacha_type)
            .ok_or_else(|| bad("gacha_type"))?;
        let gacha_id: u32 = field(gacha_id).and_then(|g| g.parse().ok()).ok_or_else(|| bad("gacha_id"))?;
        let item_id: u32 = field(item_id).and_then(|i| i.parse().ok()).ok_or_else(|| bad("item_id"))?;
        let time = field(time).and_then(parse_time).ok_or_else(|| bad("time"))?;
        let rarity = opt_field(rarity_at)
            .and_then(|r| r.parse().ok())
            .or_else(|| rarity(item_id))
            .ok_or_else(|| bad("rarity"))?;
        let featured = match opt_field(result).map(str::to_ascii_lowercase).as_deref() {
            Some("won" | "guaranteed") => Some(true),
            Some("lost") => Some(false),
            _ => None,
        };
        if let Some(featured) = featured {
            let banner = read.rate_ups.entry(gacha_id).or_default();
            if featured { &mut banner.items } else { &mut banner.not }.insert(item_id);
        }
        read.pulls.push(Pull {
            id: Some(id),
            uid: 0,
            pool,
            gacha_id,
            item_id,
            rarity,
            time,
            request_no: opt_field(request_no).and_then(|r| r.parse().ok()),
            batch_index: opt_field(batch_index).and_then(|b| b.parse().ok()).unwrap_or(0),
            sources: 0,
        });
    }
    Ok(read)
}

/// A time as RFC 3339, as `YYYY-MM-DD HH:MM:SS` in UTC, or as Unix seconds. Kept to the second
fn parse_time(text: &str) -> Option<jiff::Timestamp> {
    let time = if let Ok(time) = text.parse::<jiff::Timestamp>() {
        time
    } else if let Some(utc) = super::parse_datetime(text) {
        utc.to_zoned_datetime(jiff::tz::TimeZone::UTC).ok()?.timestamp()
    } else {
        jiff::Timestamp::from_second(text.parse().ok()?).ok()?
    };
    Some(super::from_second(time.as_second()))
}

/// The rows of a CSV, each its fields: quoted fields may hold commas, doubled quotes and line
/// breaks
fn rows(text: &str) -> impl Iterator<Item = Vec<String>> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        chars.peek()?;
        let mut row = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' if quoted => quoted = false,
                '"' if field.is_empty() => quoted = true,
                ',' if !quoted => row.push(std::mem::take(&mut field)),
                '\r' if !quoted && chars.peek() == Some(&'\n') => {}
                '\n' if !quoted => break,
                c => field.push(c),
            }
        }
        row.push(field);
        Some(row)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warps::source;

    fn pull(id: Option<i64>, item_id: u32, rarity: u8, time: i64) -> Pull {
        Pull {
            id,
            uid: 801234567,
            pool: Pool::Character,
            gacha_id: 2142,
            item_id,
            rarity,
            time: crate::warps::from_second(time),
            request_no: None,
            batch_index: 0,
            sources: source::API,
        }
    }

    #[test]
    fn what_is_written_reads_back() {
        let pulls = [
            pull(Some(1_700_000_000_000_000_067), 1006, 5, 1_700_000_000),
            pull(Some(1_700_000_000_000_001_067), 20000, 3, 1_700_000_060),
        ];
        let text = write(&pulls, &[], |item| {
            Some(if item == 1006 { "Silver Wolf".into() } else { "Arrows".into() })
        });
        assert!(text.starts_with("id,uid,gacha_type,pool,gacha_id,item_id,name,rarity,time,request_no,batch_index,result\n"));
        assert!(text.contains(",Silver Wolf,5,2023-11-14T22:13:20Z,"));
        let read = read(&text, |_| None).unwrap();
        let expected: Vec<Pull> = pulls
            .iter()
            .map(|p| Pull {
                uid: 0,
                sources: 0,
                ..p.clone()
            })
            .collect();
        assert_eq!(read.pulls, expected);
        assert_eq!(read.without_id, 0);
        assert!(read.rate_ups.is_empty());
    }

    #[test]
    fn results_tell_what_banners_featured() {
        let pulls = [
            pull(Some(1), 1003, 5, 0),
            pull(Some(2), 1006, 5, 1),
            pull(Some(3), 1102, 5, 2),
            pull(Some(4), 20000, 3, 3),
        ];
        let outcomes = [Outcome::Lost, Outcome::Guaranteed, Outcome::Won, Outcome::None];
        let text = write(&pulls, &outcomes, |_| None);
        assert!(text.contains(",lost\n") && text.contains(",guaranteed\n") && text.contains(",won\n"));
        let read = read(&text, |_| None).unwrap();
        let banner = &read.rate_ups[&2142];
        assert_eq!(banner.items, [1006, 1102].into_iter().collect());
        assert_eq!(banner.not, [1003].into_iter().collect());
    }

    #[test]
    fn names_with_commas_and_quotes_are_quoted() {
        let text = write(&[pull(Some(1), 21000, 4, 0)], &[], |_| Some("Before Dawn, \"the\" cone".into()));
        assert!(text.contains(",\"Before Dawn, \"\"the\"\" cone\",4,"));
        assert_eq!(read(&text, |_| None).unwrap().pulls.len(), 1);
    }

    #[test]
    fn live_pulls_are_written_without_an_id_and_skipped_on_reading() {
        let text = write(&[pull(None, 1006, 5, 0), pull(Some(7), 1006, 5, 0)], &[], |_| None);
        let read = read(&text, |_| None).unwrap();
        assert_eq!(read.pulls.len(), 1);
        assert_eq!(read.without_id, 1);
    }

    #[test]
    fn columns_go_by_name_and_rarity_is_looked_up() {
        let text = "Item_ID,time,gacha_id,extra,id,gacha_type\r\n1006,2023-11-14 22:13:20,2142,x,67,11\r\n\r\n";
        let read = read(text, |item| (item == 1006).then_some(5)).unwrap();
        assert_eq!(read.pulls, vec![Pull {
            uid: 0,
            sources: 0,
            ..pull(Some(67), 1006, 5, 1_700_000_000)
        }]);
    }

    #[test]
    fn a_file_without_the_columns_or_with_a_bad_row_says_which() {
        assert_eq!(
            read("id,name\n1,x\n", |_| None),
            Err("it has no gacha_type, gacha_id, item_id, time column".into())
        );
        let bad = "id,gacha_type,gacha_id,item_id,time,rarity\n1,11,2142,1006,0,5\n2,99,2142,1006,0,5\n";
        assert_eq!(read(bad, |_| None), Err("row 3 has no usable gacha_type".into()));
    }
}
