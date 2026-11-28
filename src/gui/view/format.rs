//! Numbers, times and words as the window writes them

/// A whole number with its thousands set apart, 7418 as 7,418
pub fn grouped(n: impl std::fmt::Display) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `capture` as `Capture`. The rest is left as it is, so `UID` stays in capitals
pub fn sentence(words: &str) -> String {
    let mut chars = words.chars();
    chars
        .next()
        .map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// A chance as a whole percent, never rounded to a certainty it isn't
pub fn percent(p: f64) -> String {
    match p {
        p if p >= 0.9995 => "100%".into(),
        p if p > 0.995 => ">99%".into(),
        p if p > 0.0 && p < 0.005 => "<1%".into(),
        p => format!("{:.0}%", p * 100.0),
    }
}

/// A Unix time as the local time of day
pub fn clock(at: i64) -> String {
    local(crate::warps::from_second(at), "%H:%M")
}

/// A Unix time in the system's time zone, as `format` (strftime) writes it; empty when it's out of
/// range
pub fn local(at: jiff::Timestamp, format: &str) -> String {
    at.to_zoned_datetime(jiff::tz::TimeZone::system()).strftime(format).to_string()
}

/// A Unix time in UTC, as `format` (strftime) writes it; empty when it's out of range
pub fn utc(at: jiff::Timestamp, format: &str) -> String {
    at.strftime(format).to_string()
}

/// How long ago a Unix time was, roughly
pub fn ago(now: jiff::Timestamp, then: jiff::Timestamp) -> String {
    let seconds = (now.as_second() - then.as_second()).max(0);
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86400 => format!("{} h ago", seconds / 3600),
        86400..604800 => format!("{} days ago", seconds / 86400),
        _ => utc(then, "%Y-%m-%d"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1582), "1,582");
        assert_eq!(grouped(1234567), "1,234,567");
        assert_eq!(grouped(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn sentences_start_with_a_capital() {
        assert_eq!(sentence("capture"), "Capture");
        assert_eq!(sentence("UID 7"), "UID 7");
        assert_eq!(sentence(""), "");
    }

    #[test]
    fn percentages_round_honestly() {
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(0.9991), ">99%");
        assert_eq!(percent(0.4449), "44%");
        assert_eq!(percent(0.001), "<1%");
        assert_eq!(percent(0.0), "0%");
    }

    #[test]
    fn times_read_as_how_long_ago() {
        let ago = |now, then| ago(crate::warps::from_second(now), crate::warps::from_second(then));
        assert_eq!(ago(1000, 990), "just now");
        assert_eq!(ago(10_000, 10_000 - 300), "5 min ago");
        assert_eq!(ago(100_000, 100_000 - 7200), "2 h ago");
    }
}
