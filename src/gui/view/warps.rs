//! The warp tracker's plate on the main window: each warp's pity, and the way into the warp screen
//! (see [`super::warp_screen`]), which has the rest.

use rune::{Cx, El, col, row, text};

use super::format::{ago, clock, grouped};
use super::parts::{Tone, kicker, ring_button, status_dot, stub_perforation, tab, ticket};
use super::warp_screen::label;
use crate::gui::icons;
use crate::gui::state::Ui;
use crate::gui::theme::{self, Theme};
use crate::warps::Pool;
use crate::warps::stats::{Highlight, PoolStats};

/// The pools the plate shows
const PLATE_POOLS: [Pool; 3] = [Pool::Character, Pool::LightCone, Pool::Standard];

/// The plate on the main window
pub fn plate_view(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
    let summary = s.warps_summary();
    let counters = PLATE_POOLS.map(|pool| counter(pool, summary.and_then(|summary| summary.pool(pool)), t));

    let open = ring_button("open-warps", &icons::CHEVRON, "Open the warp screen", false, t, |s| s.open_warps());

    // A warp ticket: the pity on its face, the way in on its stub
    const STUB: f32 = 72.0;
    ticket("warps", &["warps-perforation"], cx, t)
        .w_full()
        .child(tab("warps-tab", "Warps", t.secondary, t))
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
fn counter(pool: Pool, stats: Option<&PoolStats>, t: &Theme) -> El {
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
        .child(kicker(label(pool), t.muted))
        .child(number)
        .child(foot)
}

/// Where syncing stands, in a line with a dot
pub fn status_line(s: &Ui, t: &Theme) -> El {
    let status = SyncStatus::of(s);
    let tone = status.tone();
    row()
        .key("warps-status")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(status_dot(tone.dot(t), s.warps.busy.is_some(), 6.0, t))
        .child(
            text(status.long())
                .font(theme::SMALL)
                .color(tone.ink(t))
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// Where syncing warps stands
#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    /// Warps can't be kept, for this reason
    Unavailable(String),
    /// Something's under way, in words
    Busy(String),
    LoggedOut,
    /// Pulls seen live wait for the game's history, which the authkey can fetch, `next` time
    Pending {
        count: usize,
        next: Option<i64>,
    },
    /// Pulls seen live wait for the game's history, and it takes a new authkey to fetch it
    PendingKey {
        count: usize,
    },
    /// The history was last fetched this long ago
    Synced {
        ago: String,
    },
    /// Nothing kept yet; new warps will be
    Tracking,
    /// Pulls kept, and it takes a new authkey to fetch the history
    Kept {
        total: usize,
    },
    /// The warps are still being read
    Reading,
}

impl SyncStatus {
    pub fn of(s: &Ui) -> SyncStatus {
        let now = jiff::Timestamp::now().as_second();
        if let Some(error) = &s.warps.unavailable {
            return SyncStatus::Unavailable(error.clone());
        }
        if let Some(busy) = &s.warps.busy {
            return SyncStatus::Busy(busy.clone());
        }
        if s.warps_uid().is_none() {
            return SyncStatus::LoggedOut;
        }
        let Some(summary) = s.warps_summary() else {
            return SyncStatus::Reading;
        };
        let usable = summary.key.as_ref().is_some_and(|key| key.usable(now));
        match (summary.unconfirmed, usable, summary.last_sync) {
            (count @ 1.., true, _) => SyncStatus::Pending {
                count,
                next: summary.next_look,
            },
            (count @ 1.., false, _) => SyncStatus::PendingKey { count },
            (_, true, Some(at)) => SyncStatus::Synced { ago: ago(now, at) },
            _ if summary.total == 0 => SyncStatus::Tracking,
            _ => SyncStatus::Kept { total: summary.total },
        }
    }

    /// In words
    pub fn long(&self) -> String {
        match self {
            SyncStatus::Unavailable(error) => format!("Can't save warps: {error}"),
            SyncStatus::Busy(words) => words.clone(),
            SyncStatus::LoggedOut => "Log in to start tracking warps".to_string(),
            SyncStatus::Pending { count, next } => {
                let when = next.map_or(String::new(), |at| format!(" · next check {}", clock(at)));
                format!("{count} pulls pending{when}")
            }
            SyncStatus::PendingKey { count } => format!("{count} pulls pending · open Warp → View Details to sync"),
            SyncStatus::Synced { ago } => format!("Synced {ago}"),
            SyncStatus::Tracking => "Tracking new warps".to_string(),
            SyncStatus::Kept { total } => format!("{} pulls · open Warp → View Details to sync", grouped(total)),
            SyncStatus::Reading => "Reading your warps…".to_string(),
        }
    }

    /// In a couple of words, for the warp screen's header
    pub fn short(&self) -> String {
        match self {
            SyncStatus::Pending { count, .. } | SyncStatus::PendingKey { count } => format!("{count} waiting"),
            SyncStatus::LoggedOut => "Not logged in".to_string(),
            other => {
                let long = other.long();
                long.split(" · ").next().unwrap_or(&long).to_string()
            }
        }
    }

    /// Quiet when there's nothing to say, good when synced, warn when pulls wait, bad when warps
    /// can't be kept
    pub fn tone(&self) -> Tone {
        match self {
            SyncStatus::Unavailable(_) => Tone::Bad,
            SyncStatus::Busy(_) => Tone::Busy,
            SyncStatus::Pending { .. } | SyncStatus::PendingKey { .. } => Tone::Warn,
            SyncStatus::Synced { .. } => Tone::Good,
            SyncStatus::LoggedOut | SyncStatus::Tracking | SyncStatus::Kept { .. } | SyncStatus::Reading => Tone::Quiet,
        }
    }
}

/// A key for each of `fives` (in the pool's order, newest first): when it came and what it was,
/// and for another of the same in the same second (a ten pull can give two) how many came before
/// it. A count of the repeats only, so a new 5 star doesn't change the others' keys
pub(super) fn five_keys<'a>(fives: impl IntoIterator<Item = &'a Highlight>) -> Vec<String> {
    let mut seen: std::collections::HashMap<(i64, u32), usize> = std::collections::HashMap::new();
    fives
        .into_iter()
        .map(|five| {
            let n = seen.entry((five.time, five.item_id)).or_default();
            let key = match *n {
                0 => format!("five-{}-{}", five.time, five.item_id),
                n => format!("five-{}-{}-{n}", five.time, five.item_id),
            };
            *n += 1;
            key
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_shorten() {
        let synced = SyncStatus::Synced { ago: "2 min ago".into() };
        assert_eq!(synced.long(), "Synced 2 min ago");
        assert_eq!(synced.short(), "Synced 2 min ago");
        let pending = SyncStatus::Pending { count: 4, next: None };
        assert_eq!(pending.long(), "4 pulls pending");
        assert_eq!(pending.short(), "4 waiting");
        let pending = SyncStatus::PendingKey { count: 4 };
        assert_eq!(pending.long(), "4 pulls pending · open Warp → View Details to sync");
        assert_eq!(pending.short(), "4 waiting");
        assert_eq!(SyncStatus::LoggedOut.short(), "Not logged in");
        let kept = SyncStatus::Kept { total: 1582 };
        assert_eq!(kept.long(), "1,582 pulls · open Warp → View Details to sync");
        assert_eq!(kept.short(), "1,582 pulls");
    }

    #[test]
    fn five_keys_tell_two_of_the_same_in_one_second_apart() {
        use crate::warps::stats::Outcome;
        let five = |time, item_id| Highlight {
            item_id,
            pity: 80,
            outcome: Outcome::Unknown,
            time,
            confirmed: true,
        };
        let fives = [five(300, 1), five(200, 7), five(200, 7), five(200, 8), five(100, 7)];
        assert_eq!(five_keys(&fives), [
            "five-300-1",
            "five-200-7",
            "five-200-7-1",
            "five-200-8",
            "five-100-7"
        ]);
        // A newer 5 star leaves the others' keys as they were
        let newer = [five(400, 2), five(300, 1), five(200, 7), five(200, 7)];
        assert_eq!(five_keys(&newer)[1..], ["five-300-1", "five-200-7", "five-200-7-1"]);
    }
}
