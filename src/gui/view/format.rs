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
}
