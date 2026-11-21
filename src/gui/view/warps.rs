//! The warp tracker: a plate with each warp's pity under the other two, and a sheet with the whole
//! picture: pity with how close it is to the guarantee, the 5 stars drawn as tiles with the pity
//! they came at, the latest pulls, and where the history comes from (the game's history page,
//! Star Rail Station backups).

use rune::{Color, Cx, El, Icon, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{SegmentedControl, TextInput};

use super::parts::{Pill, Sheet, TileLook, kicker, pill, plate, ring_button, rule, sheet, tab, tile};
use crate::gui::icons;
use crate::gui::state::{Panel, Ui};
use crate::gui::theme::{self, Theme};
use crate::warps::Pool;
use crate::warps::service::Summary;
use crate::warps::stats::{Outcome, PoolStats};

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

    plate("warps", cx, t)
        .w_full()
        .child(tab("warps-tab", "Warps", t.secondary, cx, t))
        .child(
            row()
                .w_full()
                .pl(22.0)
                .pr(14.0)
                .pt(8.0)
                .gap(10.0)
                .center_y()
                .children(counters)
                .child(el().w(1.0).h(40.0).fill(t.line))
                .child(open),
        )
        .child(status_line(s, t).px(22.0).pt(8.0).pb(14.0))
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
        .child(kicker(label, t.secondary))
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

/// The sheet with the whole tracker
pub fn dialog(s: &Ui, cx: &Cx<Ui>, t: &Theme, layer: &mut Vec<El>) {
    if s.panel != Some(Panel::Warps) {
        return;
    }
    let mut body = col()
        .key("warps-body")
        .w_full()
        // As tall as the window leaves room for under the sheet's title
        .max_h(cx.layout_size("page").map_or(560.0, |[_, h]| (h - 170.0).max(320.0)))
        .min_h(0.0)
        .pt(16.0)
        .pb(6.0);

    if let Some(pick) = &s.warps.choose {
        body = body.child(profile_picker(pick, t));
    }

    match s.warps_summary() {
        Some(summary) => {
            let pools: Vec<Pool> = summary.pools.iter().map(|p| p.pool).collect();
            let selected = pools.iter().position(|&p| p == s.warps.pool).unwrap_or(0);
            let pool = summary.pools.get(selected);
            let labels: Vec<&str> = pools.iter().map(|p| short_label(*p)).collect();
            body = body.children((summary.total > 0).then(|| {
                section("warp-pools", t).child(
                    SegmentedControl::new(&labels, Some(selected))
                        .label("Warp")
                        .fit()
                        .on_pick(move |s: &mut Ui, i, _| {
                            if let Some(&pool) = pools.get(i) {
                                s.warps.pool = pool;
                            }
                        })
                        .build(&t.kit)
                        .key("warp-pool-picker"),
                )
            }));
            if summary.total == 0 {
                body = body.child(
                    section("warps-start", t)
                        .child(text("No warps kept yet").font(theme::TITLE).color(t.text))
                        .child(
                            text(
                                "Every warp you make while the archiver runs is kept from now on. For the ones before,                                  open Warp → View Details in the game to fetch the last six months, and import a Star                                  Rail Station backup for anything older.",
                            )
                            .font(theme::BODY)
                            .color(t.muted)
                            .wrap(),
                        ),
                );
            } else if let Some(stats) = pool {
                body = body.child(five_stars(stats, summary, t)).child(recent(stats, summary, t));
            }
            body = body.child(history(s, summary, t));
        }
        None => {
            let message = match (&s.warps.unavailable, s.warps_uid()) {
                (Some(error), _) => format!("The warp database couldn't be opened: {error}"),
                (None, None) => "Log in to the game once, and the warps of the account are kept from then on.".to_string(),
                (None, Some(_)) => "Reading your warps…".to_string(),
            };
            body = body.child(section("warps-empty", t).child(text(message).font(theme::BODY).color(t.muted).wrap()));
        }
    }

    let spec = Sheet {
        key: "warps",
        kicker: "Warp tracker",
        title: "Warp history",
        width: 560.0,
        close: |s| s.panel = None,
    };
    let body = body.scroll(ScrollSpec { stick_bottom: false }).scroll_edge_fade(12.0);
    sheet(spec, body, cx, t, layer);
}

fn section(key: &'static str, t: &Theme) -> El {
    let _ = t;
    col().key(key).w_full().px(22.0).pb(20.0).gap(10.0)
}

fn short_label(pool: Pool) -> &'static str {
    match pool {
        Pool::Character => "Character",
        Pool::LightCone => "Light Cone",
        Pool::Standard => "Standard",
        Pool::CollabCharacter => "Collab",
        Pool::CollabLightCone => "Collab LC",
        Pool::Departure => "Departure",
    }
}

/// The 5 stars as tiles with the pity they came at, newest first
fn five_stars(stats: &PoolStats, summary: &Summary, t: &Theme) -> El {
    let mut figures = vec![format!("{} pulls", grouped(stats.total)), format!("{} × 5★", stats.count5)];
    if let Some(avg) = stats.avg_pity5 {
        figures.push(format!("{avg:.1} average pity"));
    }
    if stats.played5 > 0 {
        figures.push(format!(
            "won {} of {} 50/50s ({}%)",
            stats.won5,
            stats.played5,
            (stats.won5 * 100 + stats.played5 / 2) / stats.played5
        ));
    }

    let mut section = section("five-stars", t)
        .child(heading("5★ pulls", t))
        .child(text(figures.join("  ·  ")).font(theme::SMALL).color(t.muted).wrap());
    if stats.fives.is_empty() {
        return section.child(text("None yet.").font(theme::SMALL).color(t.faint));
    }

    let tiles = stats.fives.iter().take(24).enumerate().map(|(i, five)| {
        let ink = outcome_ink(five.outcome, t);
        let look = TileLook::face_up(t);
        let face = col()
            .items_center()
            .child(text(five.pity.to_string()).font(theme::TITLE).color(ink))
            .children((!five.confirmed).then(|| text("LIVE").font(theme::KICKER).color(t.face_ink)));
        col()
            .key(format!("five-{i}"))
            .w(66.0)
            .gap(5.0)
            .items_center()
            .help(format!("{}, {}", summary.name(five.item_id), outcome_label(five.outcome)))
            .child(tile(36.0, 46.0, look, Some(face)))
            .child(
                text(summary.name(five.item_id))
                    .font(theme::SMALL)
                    .color(t.text)
                    .truncate()
                    .max_w(66.0),
            )
    });
    section = section.child(row().key("five-tiles").w_full().flex_wrap().gap(4.0).children(tiles));
    section.child(legend(t))
}

fn legend(t: &Theme) -> El {
    let entry = |label: &'static str, color: Color| {
        row()
            .gap(5.0)
            .center_y()
            .child(el().size(7.0, 7.0).radius(2.0).fill(color))
            .child(text(label).font(theme::SMALL).color(t.faint))
    };
    row()
        .gap(14.0)
        .flex_wrap()
        .child(entry("Won the 50/50", outcome_ink(Outcome::Won, t)))
        .child(entry("Guaranteed", outcome_ink(Outcome::Guaranteed, t)))
        .child(entry("Lost the 50/50", outcome_ink(Outcome::Lost, t)))
}

/// How a tile's number is coloured: the tile is pale in every theme, so these are the face inks
fn outcome_ink(outcome: Outcome, t: &Theme) -> Color {
    match outcome {
        Outcome::Won => rune::oklch(0.52, 0.12, 200.0),
        Outcome::Guaranteed => t.face_ink,
        Outcome::Lost => t.face_bad,
        Outcome::Unknown | Outcome::None => rune::oklch(0.35, 0.02, 85.0),
    }
}

fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Won => "won the 50/50",
        Outcome::Guaranteed => "guaranteed",
        Outcome::Lost => "lost the 50/50",
        Outcome::Unknown => "50/50 unknown",
        Outcome::None => "no 50/50",
    }
}

/// The latest pulls, newest first
fn recent(stats: &PoolStats, summary: &Summary, t: &Theme) -> El {
    let now = chrono::Utc::now().timestamp();
    let rows = stats.recent.iter().take(20).enumerate().map(|(i, (pull, pity, outcome))| {
        let tint = match pull.rarity {
            5 => t.accent,
            4 => t.secondary,
            _ => t.line,
        };
        let name_color = match pull.rarity {
            5 => t.accent,
            4 => t.text,
            _ => t.muted,
        };
        let mut line = row()
            .key(format!("recent-{i}"))
            .w_full()
            .h(30.0)
            .gap(10.0)
            .center_y()
            .child(el().w(3.0).h(18.0).radius(1.5).fill(tint))
            .child(
                text(summary.name(pull.item_id))
                    .font(if pull.rarity >= 4 { theme::LABEL } else { theme::SMALL })
                    .color(name_color)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if pull.id.is_none() {
            line = line.child(
                el().px(6.0)
                    .h(17.0)
                    .center()
                    .radius(3.0)
                    .border(1.0, t.warn)
                    .help("Seen live, the game's history doesn't have it yet")
                    .child(text("LIVE").font(theme::KICKER).color(t.warn)),
            );
        }
        if pull.rarity >= 4 {
            let mark = if matches!(outcome, Outcome::Lost) { " ✕" } else { "" };
            line = line.child(
                text(format!("{pity}{mark}"))
                    .font(theme::LABEL)
                    .color(tint)
                    .w(40.0)
                    .align(rune::Align::End),
            );
        } else {
            line = line.child(el().w(40.0));
        }
        line.child(
            text(ago(now, pull.time))
                .font(theme::SMALL)
                .color(t.faint)
                .w(92.0)
                .align(rune::Align::End),
        )
    });
    section("recent", t)
        .child(heading("Latest pulls", t))
        .child(col().key("recent-rows").w_full().children(rows))
}

/// Where the history comes from: the game's history page, and Star Rail Station backups
fn history(s: &Ui, summary: &Summary, t: &Theme) -> El {
    let now = chrono::Utc::now().timestamp();
    let busy = s.warps.busy.is_some();
    let usable = summary.key.as_ref().is_some_and(|key| key.usable(now));

    let key_detail = match &summary.key {
        Some(key) if key.usable(now) => format!(
            "Authkey from {}, {} · works for about a day",
            origin_label(&key.origin),
            ago(now, key.obtained_at)
        ),
        Some(_) => "The authkey has expired. Open Warp → View Details in the game for a new one.".to_string(),
        None => "Open Warp → View Details in the game while the archiver runs, and it syncs by itself.".to_string(),
    };
    let mut detail = col().gap(3.0).child(text(key_detail).font(theme::SMALL).color(t.muted).wrap());
    if let Some(at) = summary.last_sync {
        detail = detail.child(text(format!("Last synced {}", ago(now, at))).font(theme::SMALL).color(t.faint));
    }
    if let Some(error) = &summary.last_error {
        detail = detail.child(text(error.clone()).font(theme::SMALL).color(t.bad).wrap());
    }
    let sync = pill(
        "sync-warps",
        "Sync",
        Some(&icons::SYNC),
        if usable { Pill::Primary } else { Pill::Secondary },
        usable && !busy,
        t,
        |s: &mut Ui, _| s.sync_warps(),
    );

    let link_field = TextInput::new(&s.warps.link_edit)
        .label("Warp history link")
        .placeholder("Paste a warp history link")
        .on_edit(|s: &mut Ui| &mut s.warps.link_edit)
        .on_key(|s: &mut Ui, ev: &rune::KeyEvent, _| {
            if ev.key == rune::Key::Enter {
                s.use_history_link();
                return true;
            }
            false
        })
        .build(&t.kit)
        .key("history-link")
        .grow(1.0)
        .min_w(0.0);
    let key_actions = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(link_field)
        .child(pill(
            "use-link",
            "Use",
            None,
            Pill::Secondary,
            !busy && !s.warps.link_edit.text.trim().is_empty(),
            t,
            |s: &mut Ui, _| s.use_history_link(),
        ))
        .child(pill(
            "find-cached-key",
            "From game cache",
            Some(&icons::KEY),
            Pill::Secondary,
            !busy,
            t,
            |s: &mut Ui, _| s.find_cached_key(),
        ));

    let mut completeness = vec![format!(
        "{} pulls in {} requests",
        grouped(summary.total),
        grouped(summary.requests)
    )];
    if summary.unconfirmed > 0 {
        let when = summary
            .next_look
            .map_or(String::new(), |at| format!(" (it adds an hour at a time, next look {})", clock(at)));
        completeness.push(format!("{} seen live, waiting for the history{when}", summary.unconfirmed));
    }
    let gap_note = match summary.missing_requests() {
        Some(0) => Some(("Nothing is missing: the game's request counter agrees.".to_string(), t.good)),
        Some(missing) => Some((
            format!(
                "The game counts {} requests, {missing} aren't kept. The history only reaches back about six months, \
                 a Star Rail Station backup can fill in older ones.",
                grouped(summary.request_counter.unwrap_or_default() as usize + 1)
            ),
            t.warn,
        )),
        None => None,
    };

    let mut section = section("history", t)
        .child(heading("History", t))
        .child(option_row("history-sync", &icons::SYNC, "Game history", detail, Some(sync), t))
        .child(key_actions)
        .child(el().w_full().pt(6.0).child(rule(t)))
        .child(option_row(
            "import-srs",
            &icons::IMPORT,
            "Star Rail Station",
            text("Import a backup (.dat) for pulls older than the game's history")
                .font(theme::SMALL)
                .color(t.muted)
                .wrap(),
            Some(pill(
                "import-srs-file",
                "Import…",
                None,
                Pill::Secondary,
                !busy,
                t,
                |s: &mut Ui, tx: &mut Tx| s.import_srs(tx),
            )),
            t,
        ))
        .child(el().w_full().pt(6.0).child(rule(t)))
        .child(text(completeness.join("  ·  ")).font(theme::SMALL).color(t.muted).wrap());
    if let Some((note, color)) = gap_note {
        section = section.child(text(note).key("gap-note").font(theme::SMALL).color(color).wrap());
    }
    section
}

/// Asks which profile of a backup is the account
fn profile_picker(pick: &crate::gui::state::ProfilePick, t: &Theme) -> El {
    let rows = pick.profiles.iter().map(|profile| {
        let latest = profile.latest.map_or(String::new(), |at| {
            chrono::DateTime::from_timestamp(at, 0).map_or(String::new(), |d| format!(" · latest {}", d.format("%Y-%m-%d")))
        });
        let fits = if profile.fits { "" } else { " · another account's" };
        let key = profile.key.clone();
        row()
            .key(format!("profile-{}", profile.key))
            .w_full()
            .gap(10.0)
            .center_y()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(profile.name.clone()).font(theme::LABEL).color(t.text).truncate())
                    .child(
                        text(format!("{} pulls{latest}{fits}", grouped(profile.pulls)))
                            .font(theme::SMALL)
                            .color(if profile.fits { t.muted } else { t.bad }),
                    ),
            )
            .child(pill(
                &format!("pick-{}", profile.key),
                "Import",
                None,
                Pill::Secondary,
                profile.fits,
                t,
                move |s: &mut Ui, _| s.choose_profile(Some(key.clone())),
            ))
    });
    section("profile-picker", t).child(
        col()
            .w_full()
            .padding(14.0)
            .gap(10.0)
            .fill(t.well)
            .border(1.0, t.accent_line)
            .child(kicker("Which profile is this account?", t.accent))
            .child(
                text(format!(
                    "The backup has more than one profile that could be UID {}. Pulls already kept are skipped, so importing \
                     several is safe.",
                    pick.uid
                ))
                .font(theme::SMALL)
                .color(t.muted)
                .wrap(),
            )
            .children(rows)
            .child(row().w_full().child(el().grow(1.0)).child(pill(
                "cancel-pick",
                "Cancel",
                None,
                Pill::Quiet,
                true,
                t,
                |s: &mut Ui, _| s.choose_profile(None),
            ))),
    )
}

fn heading(label: &'static str, t: &Theme) -> El {
    kicker(label, t.accent)
}

/// An icon in a double ring, a title and what it means, and its action past a divider, like the
/// delivery plate's rows
fn option_row(key: &'static str, glyph: &'static Icon, title: &'static str, detail: El, action: Option<El>, t: &Theme) -> El {
    let medallion = el().size(38.0, 38.0).radius(19.0).center().border(1.0, t.accent_line).child(
        el().size(30.0, 30.0)
            .radius(15.0)
            .center()
            .fill(t.well)
            .border(1.0, t.line)
            .child(icon(glyph, 15.0).color(t.accent)),
    );
    let mut line = row().key(key).w_full().py(4.0).gap(14.0).center_y().child(medallion).child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(3.0)
            .child(text(title).font(theme::TITLE).color(t.text))
            .child(detail),
    );
    if let Some(action) = action {
        line = line.child(el().w(1.0).h(34.0).fill(t.line)).child(action);
    }
    line
}

fn origin_label(origin: &str) -> &'static str {
    match origin {
        "capture" => "the game",
        "cache" => "the game's cache",
        "link" => "a pasted link",
        _ => "somewhere",
    }
}

/// 7418 as 7,418
fn grouped(n: usize) -> String {
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
fn clock(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
}

/// How long ago a Unix time was, roughly
fn ago(now: i64, then: i64) -> String {
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
