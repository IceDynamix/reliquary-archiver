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

use super::{Pool, Pull};

/// The columns written, in order
const COLUMNS: [&str; 11] = [
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
];

/// The pulls as CSV, in the order given, with `name` naming each item
pub fn write(pulls: &[Pull], name: impl Fn(u32) -> Option<String>) -> String {
    let mut out = COLUMNS.join(",");
    out.push('\n');
    for pull in pulls {
        let time = jiff::Timestamp::from_second(pull.time)
            .map_or_else(|_| pull.time.to_string(), |t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string());
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
pub struct Read {
    /// The pulls with an id, `uid` 0 (the account they go into decides it)
    pub pulls: Vec<Pull>,
    /// Rows without an id: pulls the game's history didn't have yet when they were written
    pub without_id: usize,
}

/// The pulls in a CSV. `rarity` gives an item's rarity when the file has no `rarity` column, or
/// an empty one. Fails on the first row it can't read, naming it
pub fn read(text: &str, rarity: impl Fn(u32) -> Option<u8>) -> Result<Read, String> {
    let mut rows = rows(text.trim_start_matches('\u{feff}'));
    let header = rows.next().ok_or("the file is empty")?;
    let column = |name: &str| header.iter().position(|h| h.trim().eq_ignore_ascii_case(name));
    let missing = ["id", "gacha_type", "gacha_id", "item_id", "time"]
        .into_iter()
        .filter(|name| column(name).is_none())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!("it has no {} column", missing.join(", ")));
    }
    let at = |name: &str| column(name).unwrap_or(usize::MAX);
    let (id, gacha_type, gacha_id, item_id, time) = (at("id"), at("gacha_type"), at("gacha_id"), at("item_id"), at("time"));
    let (rarity_at, request_no, batch_index) = (column("rarity"), column("request_no"), column("batch_index"));

    let mut read = Read::default();
    for (n, row) in rows.enumerate() {
        // Line numbers as a spreadsheet shows them, the header being line 1
        let line = n + 2;
        if row.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let field = |i: Option<usize>| i.and_then(|i| row.get(i)).map(|f| f.trim()).filter(|f| !f.is_empty());
        let bad = |what: &str| format!("row {line} has no usable {what}");
        let Some(id) = field(Some(id)) else {
            read.without_id += 1;
            continue;
        };
        let id: i64 = id.parse().map_err(|_| bad("id"))?;
        let pool = field(Some(gacha_type))
            .and_then(|t| t.parse().ok())
            .and_then(Pool::from_gacha_type)
            .ok_or_else(|| bad("gacha_type"))?;
        let gacha_id: u32 = field(Some(gacha_id)).and_then(|g| g.parse().ok()).ok_or_else(|| bad("gacha_id"))?;
        let item_id: u32 = field(Some(item_id)).and_then(|i| i.parse().ok()).ok_or_else(|| bad("item_id"))?;
        let time = field(Some(time)).and_then(parse_time).ok_or_else(|| bad("time"))?;
        let rarity = field(rarity_at)
            .and_then(|r| r.parse().ok())
            .or_else(|| rarity(item_id))
            .ok_or_else(|| bad("rarity"))?;
        read.pulls.push(Pull {
            id: Some(id),
            uid: 0,
            pool,
            gacha_id,
            item_id,
            rarity,
            time,
            request_no: field(request_no).and_then(|r| r.parse().ok()),
            batch_index: field(batch_index).and_then(|b| b.parse().ok()).unwrap_or(0),
            sources: 0,
        });
    }
    Ok(read)
}

/// A time as RFC 3339, as `YYYY-MM-DD HH:MM:SS` in UTC, or as Unix seconds
fn parse_time(text: &str) -> Option<i64> {
    if let Ok(t) = text.parse::<jiff::Timestamp>() {
        return Some(t.as_second());
    }
    if let Ok(t) = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", text) {
        return t.to_zoned_datetime(jiff::tz::TimeZone::UTC).ok().map(|t| t.timestamp().as_second());
    }
    text.parse().ok()
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
            time,
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
        let text = write(&pulls, |item| {
            Some(if item == 1006 { "Silver Wolf".into() } else { "Arrows".into() })
        });
        assert!(text.starts_with("id,uid,gacha_type,pool,gacha_id,item_id,name,rarity,time,request_no,batch_index\n"));
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
    }

    #[test]
    fn names_with_commas_and_quotes_are_quoted() {
        let text = write(&[pull(Some(1), 21000, 4, 0)], |_| Some("Before Dawn, \"the\" cone".into()));
        assert!(text.contains(",\"Before Dawn, \"\"the\"\" cone\",4,"));
        assert_eq!(read(&text, |_| None).unwrap().pulls.len(), 1);
    }

    #[test]
    fn live_pulls_are_written_without_an_id_and_skipped_on_reading() {
        let text = write(&[pull(None, 1006, 5, 0), pull(Some(7), 1006, 5, 0)], |_| None);
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
