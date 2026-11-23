//! The warp tracker's plate on the main window: each warp's pity, and the way into the warp screen
//! (see [`super::warp_screen`]), which has the rest.

use rune::{Cx, El, col, el, row, text};

use super::parts::{kicker, ring_button, stub_perforation, tab, ticket};
use crate::gui::icons;
use crate::gui::state::Ui;
use crate::gui::theme::{self, Theme};
use crate::warps::Pool;
use crate::warps::stats::PoolStats;

/// The pools the plate shows, with short names
const PLATE_POOLS: [(Pool, &str); 3] = [
    (Pool::Character, "Character"),
    (Pool::LightCone, "Light Cone"),
    (Pool::Standard, "Standard"),
];

/// The plate on the main window
pub fn plate_view(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
    let summary = s.warps_summary();
    let counters = PLATE_POOLS.iter().map(|&(pool, label)| {
        let stats = summary.and_then(|summary| summary.pool(pool));
        counter(pool, label, stats, t)
    });

    let open = ring_button("open-warps", &icons::CHEVRON, "Open the warp screen", false, t, |s| s.open_warps());

    // A warp ticket: the pity on its face, the way in on its stub
    const STUB: f32 = 72.0;
    ticket("warps", &["warps-perforation"], cx, t)
        .w_full()
        .child(tab("warps-tab", "Warps", t.secondary, cx, t))
        .child(
            row()
                .w_full()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(row().w_full().pl(22.0).pr(12.0).pt(8.0).gap(10.0).center_y().children(counters))
                        .child(status_line(s, t).pl(22.0).pr(12.0).pt(8.0).pb(14.0)),
                )
                .child(col().w(STUB).self_stretch().center().child(open)),
        )
        .child(stub_perforation("warps-perforation", STUB, cx, t))
}

/// The plate's pity numbers, smaller than the status plate's figures
const COUNTER: rune::TextStyle = rune::TextStyle::new(26.0, 500.0).lh(30.0).features("'tnum' 1");

/// One pool's pity in big numbers, the 4 star pity under it
fn counter(pool: Pool, label: &str, stats: Option<&PoolStats>, t: &Theme) -> El {
    let (pity5, pity4) = stats.map_or((None, None), |s| (Some(s.pity5), Some(s.pity4)));
    let guaranteed = stats.and_then(|s| s.guarantee5) == Some(true);
    let number = row()
        .gap(3.0)
        .items_end()
        .child(
            text(pity5.map_or("–".to_string(), |p| p.to_string()))
                .font(COUNTER)
                .color(if guaranteed { t.accent } else { t.text }),
        )
        .child(text(format!("/{}", pool.hard_pity())).font(theme::SMALL).color(t.faint).mb(4.0));
    let mut foot = row().gap(6.0).center_y().child(
        text(pity4.map_or("4★ –".to_string(), |p| format!("4★ {p}")))
            .font(theme::SMALL)
            .color(t.muted),
    );
    if guaranteed {
        foot = foot.child(text("guaranteed").font(theme::SMALL).color(t.accent).truncate().min_w(0.0));
    }
    col()
        .key(format!("counter-{}", pool.gacha_type()))
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(kicker(label, t.muted))
        .child(number)
        .child(foot)
}

/// Where syncing stands, in a line with a dot
pub fn status_line(s: &Ui, t: &Theme) -> El {
    let (message, tint) = status(s, t);
    let mut line = row().key("warps-status").w_full().gap(8.0).center_y();
    line = if s.warps.busy.is_some() {
        line.child(rune_kit::Spinner::new().size(12.0).color(t.accent).build(&t.kit))
    } else {
        line.child(el().size(6.0, 6.0).radius(3.0).fill(tint))
    };
    line.child(
        text(message)
            .font(theme::SMALL)
            .color(if tint == t.faint { t.muted } else { tint })
            .truncate()
            .grow(1.0)
            .min_w(0.0),
    )
}

/// Where syncing stands, in words, and its colour: faint when there's nothing to say, good when
/// synced, warn when pulls wait, bad when warps can't be kept
pub fn status(s: &Ui, t: &Theme) -> (String, rune::Color) {
    let now = chrono::Utc::now().timestamp();
    if let Some(error) = &s.warps.unavailable {
        (format!("Warps can't be kept: {error}"), t.bad)
    } else if let Some(busy) = &s.warps.busy {
        (busy.clone(), t.accent)
    } else if s.warps_uid().is_none() {
        ("Log in to the game to start keeping your warps".to_string(), t.faint)
    } else if let Some(summary) = s.warps_summary() {
        let usable = summary.key.as_ref().is_some_and(|key| key.usable(now));
        if summary.unconfirmed > 0 && usable {
            let when = summary.next_look.map_or(String::new(), |at| format!(" · next look {}", clock(at)));
            (
                format!("{} live pulls waiting for the game's history{when}", summary.unconfirmed),
                t.warn,
            )
        } else if summary.unconfirmed > 0 {
            (
                format!("{} live pulls kept · open Warp → View Details to sync", summary.unconfirmed),
                t.warn,
            )
        } else if let (true, Some(at)) = (usable, summary.last_sync) {
            (format!("Synced {}", ago(now, at)), t.good)
        } else if summary.total == 0 {
            ("Warps you make are kept from now on".to_string(), t.faint)
        } else {
            (
                format!("{} pulls kept · open Warp → View Details to sync", grouped(summary.total)),
                t.faint,
            )
        }
    } else {
        ("Reading your warps…".to_string(), t.faint)
    }
}

/// 7418 as 7,418
pub(super) fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A Unix time as the local time of day
pub(super) fn clock(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
}

/// How long ago a Unix time was, roughly
pub(super) fn ago(now: i64, then: i64) -> String {
    let seconds = (now - then).max(0);
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86400 => format!("{} h ago", seconds / 3600),
        86400..604800 => format!("{} days ago", seconds / 86400),
        _ => chrono::DateTime::from_timestamp(then, 0).map_or(String::new(), |d| d.format("%Y-%m-%d").to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_grouped_by_thousands() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(1582), "1,582");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    #[test]
    fn times_read_as_how_long_ago() {
        assert_eq!(ago(1000, 990), "just now");
        assert_eq!(ago(10_000, 10_000 - 300), "5 min ago");
        assert_eq!(ago(100_000, 100_000 - 7200), "2 h ago");
    }
}
