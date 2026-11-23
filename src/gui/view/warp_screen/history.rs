//! The warp screen's second page, under the banner: one warp's whole history. Its numbers across
//! the top, every 5 star as a medallion with the pity it came at, ringed by its 50/50, the latest
//! pulls, and where the history comes from (the game's history page, Star Rail Station backups),
//! with what may be missing from it.
//!
//! It's a page rather than a sheet over the screen: the rail stays beside it, so picking another
//! warp shows that warp's history, and scrolling back up past its top returns to the banner.

use rune::{Color, Cx, El, Icon, SurfaceFit, Tx, col, el, icon, row, text};
use rune_kit::TextInput;

use super::{COUNT, GAP, View, figure, label, outcome_color, outcome_words, rise};
use crate::gui::art::Art;
use crate::gui::icons;
use crate::gui::state::Ui;
use crate::gui::theme::{self, Theme};
use crate::gui::view::parts::{Pill, kicker, pill, rule, tab, ticket_on};
use crate::gui::view::warps::{ago, clock, grouped};
use crate::warps::service::Summary;
use crate::warps::stats::{Highlight, Outcome};

/// From this wide the page has two columns: the 5 stars, and the latest pulls with the sources
const TWO_COLUMNS: f32 = 820.0;
/// A 5 star's medallion, and the room each takes in the grid
const MEDALLION: f32 = 54.0;
const CELL: f32 = 74.0;
/// How many of the latest pulls are listed
const LATEST: usize = 40;

/// The page, `width` wide. The warp screen scrolls it
pub(super) fn page(s: &Ui, view: &View, width: f32, cx: &Cx<Ui>, t: &Theme) -> El {
    let two_columns = width >= TWO_COLUMNS;
    let side = (width * 0.36).clamp(320.0, 400.0);

    let sources_card = sources_card(s, view.summary, cx, t);
    let fives = fives_card(s, view, cx, t);
    let latest = latest_card(view, cx, t);

    // A backup's profile to pick waits in the sources, which then come first
    let waiting = s.warps.choose.is_some();
    let body = if two_columns {
        let side_column = if waiting {
            col().w(side).gap(GAP).child(sources_card).child(latest)
        } else {
            col().w(side).gap(GAP).child(latest).child(sources_card)
        };
        row()
            .w_full()
            .gap(GAP)
            .items_start()
            .child(col().grow(1.0).min_w(0.0).child(fives))
            .child(side_column)
    } else if waiting {
        col().w_full().gap(GAP).child(sources_card).child(fives).child(latest)
    } else {
        col().w_full().gap(GAP).child(fives).child(latest).child(sources_card)
    };
    col()
        .key(format!("history-{}", view.pool.gacha_type()))
        .w_full()
        .pb(28.0)
        .gap(22.0)
        .child(rise(heading(view, t), 0.0))
        .child(rise(body, 0.08))
}

/// For an account with no warps kept yet: how the history fills in, and where from
pub(super) fn empty(s: &Ui, summary: &Summary, cx: &Cx<Ui>, t: &Theme) -> El {
    col()
        .key("history-empty")
        .w_full()
        .gap(GAP)
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(text("No warps kept yet").font(theme::HEADLINE).color(t.text))
                .child(
                    text(
                        "Every warp you make while the archiver runs is kept from now on. For the ones before, open                          Warp → View Details in the game to fetch the last six months, and import a Star Rail Station                          backup for anything older.",
                    )
                    .font(theme::BODY)
                    .color(t.muted)
                    .wrap(),
                ),
        )
        .child(sources_card(s, summary, cx, t))
}

/// Where the history comes from, with the pick of a backup's profile when one waits
fn sources_card(s: &Ui, summary: &Summary, cx: &Cx<Ui>, t: &Theme) -> El {
    let mut card = card("history-sources-card", "Where it comes from", t.secondary, cx, t);
    if let Some(pick) = &s.warps.choose {
        card = card.child(profile_picker(pick, t).px(22.0).pt(12.0));
    }
    card.child(sources(s, summary, t).px(22.0).pt(12.0).pb(20.0))
}

/// A card on the page: a ticket with its name on a tab
fn card(key: &str, title: &str, tint: Color, cx: &Cx<Ui>, t: &Theme) -> El {
    ticket_on(key, &[], None, cx, t)
        .w_full()
        .child(tab(&format!("{key}-tab"), title, tint, cx, t))
}

/// The warp's name, and its numbers in a row
fn heading(view: &View, t: &Theme) -> El {
    let stats = view.stats;
    let mut figures = vec![
        figure(grouped(stats.total), "warps", t),
        figure(grouped(stats.count5), "5-stars", t),
        figure(grouped(stats.count4), "4-stars", t),
    ];
    if let Some(avg) = stats.avg_pity5 {
        figures.push(figure(format!("{avg:.1}"), "average 5-star pity", t));
    }
    if stats.played5 > 0 {
        figures.push(figure(
            format!("{}%", (stats.won5 * 100 + stats.played5 / 2) / stats.played5),
            &format!("won {} of {} 50/50s", stats.won5, stats.played5),
            t,
        ));
    }
    col()
        .key("history-heading")
        .w_full()
        .gap(14.0)
        .child(
            row()
                .gap(10.0)
                .center_y()
                .child(kicker(&format!("{} warp", label(view.pool)), view.accent))
                .child(el().size(3.0, 3.0).radius(1.5).fill(t.faint))
                .child(kicker("History", t.muted)),
        )
        .child(row().w_full().gap(30.0).flex_wrap().children(figures))
}

/// Every 5 star, newest first, as a medallion: the character or light cone, ringed by how its
/// 50/50 went, with the pity it came at under it
fn fives_card(s: &Ui, view: &View, cx: &Cx<Ui>, t: &Theme) -> El {
    let stats = view.stats;
    let body = if stats.fives.is_empty() {
        col().w_full().child(text("No 5-stars here yet.").font(theme::BODY).color(t.muted))
    } else {
        let keys = super::super::warps::five_keys(&stats.fives);
        let medallions = stats.fives.iter().zip(keys).map(|(five, key)| medallion(s, view, five, key, t));
        col()
            .w_full()
            .gap(16.0)
            .child(row().key("history-fives").w_full().flex_wrap().children(medallions))
            .children(view.pool.has_rate_up().then(|| legend(t)))
    };
    card("history-fives-card", "5-stars", view.accent, cx, t).child(body.px(22.0).pt(16.0).pb(20.0))
}

fn medallion(s: &Ui, view: &View, five: &Highlight, key: String, t: &Theme) -> El {
    let name = view.summary.name(five.item_id);
    let ring = outcome_color(five.outcome, t);
    let when = chrono::DateTime::from_timestamp(five.time, 0)
        .map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string());
    let outcome = match outcome_words(five.outcome) {
        "" => String::new(),
        words => format!(", {words}"),
    };
    let live = if five.confirmed {
        ""
    } else {
        "  ·  seen live, not in the game's history yet"
    };

    let inner = MEDALLION - 6.0;
    let mut face = el()
        .size(inner, inner)
        .radius(inner / 2.0)
        .clip_rounded(inner / 2.0)
        .fill(t.text.alpha(0.08));
    if let Some(image) = s.art.as_ref().and_then(|art| art.get(Art::Icon(five.item_id))) {
        face = face.pixels_fit(image.pixels(), SurfaceFit::Fill);
    }
    let pity = row()
        .gap(4.0)
        .center_y()
        .child(text(five.pity.to_string()).font(COUNT).color(ring))
        .children((!five.confirmed).then(|| text("LIVE").font(theme::KICKER).color(t.warn)));
    col()
        .key(key)
        .w(CELL)
        .pb(14.0)
        .gap(5.0)
        .items_center()
        .help(format!("{name}  ·  pity {}{outcome}  ·  {when}{live}", five.pity))
        .child(
            el().size(MEDALLION, MEDALLION)
                .radius(MEDALLION / 2.0)
                .center()
                .border(2.0, ring.alpha(if five.confirmed { 0.9 } else { 0.45 }))
                .child(face),
        )
        .child(pity)
        .child(text(name).font(theme::SMALL).color(t.muted).truncate().max_w(CELL - 4.0))
}

fn legend(t: &Theme) -> El {
    let entry = |label: &'static str, outcome: Outcome| {
        row()
            .gap(6.0)
            .center_y()
            .child(el().size(8.0, 8.0).radius(4.0).border(1.5, outcome_color(outcome, t)))
            .child(text(label).font(theme::SMALL).color(t.faint))
    };
    row()
        .gap(16.0)
        .flex_wrap()
        .child(entry("Won the 50/50", Outcome::Won))
        .child(entry("Guaranteed", Outcome::Guaranteed))
        .child(entry("Lost the 50/50", Outcome::Lost))
}

/// The latest pulls, newest first, with the pity the 5 stars came at
fn latest_card(view: &View, cx: &Cx<Ui>, t: &Theme) -> El {
    let now = chrono::Utc::now().timestamp();
    let stats = view.stats;
    let rows = stats.recent.iter().take(LATEST).enumerate().map(|(i, (pull, pity, outcome))| {
        let tint = match pull.rarity {
            5 => outcome_color(*outcome, t),
            4 => rune::oklch(0.68, 0.13, 300.0),
            _ => t.line,
        };
        let name_color = match pull.rarity {
            5 => t.text,
            4 => t.text.alpha(0.88),
            _ => t.muted,
        };
        let mut line = row()
            .key(format!("latest-{i}"))
            .w_full()
            .h(28.0)
            .gap(10.0)
            .center_y()
            .child(el().w(3.0).h(16.0).radius(1.5).fill(tint))
            .child(
                text(view.summary.name(pull.item_id))
                    .font(if pull.rarity == 5 { theme::LABEL } else { theme::SMALL })
                    .color(name_color)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if pull.id.is_none() {
            line = line.child(
                el().px(5.0)
                    .h(16.0)
                    .center()
                    .radius(3.0)
                    .border(1.0, t.warn)
                    .help("Seen live, the game's history doesn't have it yet")
                    .child(text("LIVE").font(theme::KICKER).color(t.warn)),
            );
        }
        let pity = if pull.rarity == 5 { pity.to_string() } else { String::new() };
        line.child(text(pity).font(COUNT).color(tint).w(30.0).align(rune::Align::End))
            .child(
                text(ago(now, pull.time))
                    .font(theme::SMALL)
                    .color(t.faint)
                    .w(84.0)
                    .align(rune::Align::End),
            )
    });
    let body = if stats.recent.is_empty() {
        col().child(text("No pulls yet.").font(theme::BODY).color(t.muted))
    } else {
        col().key("latest-rows").w_full().children(rows)
    };
    card("history-latest-card", "Latest pulls", t.secondary, cx, t).child(body.px(22.0).pt(12.0).pb(16.0))
}

/// Where the history comes from: the game's history page, and Star Rail Station backups
fn sources(s: &Ui, summary: &Summary, t: &Theme) -> El {
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

    let mut section = col()
        .key("history-sources")
        .w_full()
        .gap(10.0)
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
    col().key("profile-picker").w_full().child(
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
