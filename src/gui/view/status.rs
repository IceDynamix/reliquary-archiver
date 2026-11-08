//! Where capturing stands: what to do next while waiting, what's in the way when something is,
//! and what the account holds once it's in.

use rune::{Cycle, El, Icon, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, SegmentedControl};

use super::card;
use crate::gui::icons;
use crate::gui::state::{Inventory, Stage, Ui, Update};

const NPCAP_URL: &str = "https://npcap.com/#download";
use crate::gui::theme::{self, Theme};

/// What the status card leads with
struct Headline {
    glyph: Mark,
    title: String,
    detail: String,
    actions: Vec<El>,
}

enum Mark {
    /// Waiting for something to happen, softly pulsing
    Listening,
    /// Data is coming in
    Busy,
    Problem(&'static Icon),
}

/// A slow breath for things that wait on the game
const PULSE: Cycle = Cycle {
    period: 1.8,
    phase: 0.0,
    stops: &[(0.0, 1.0), (0.5, 0.3), (1.0, 1.0)],
    still: 1.0,
    fps: 30.0,
};

/// How far each step got
#[derive(Clone, Copy, PartialEq)]
enum Step {
    Done,
    Active,
    Todo,
    Blocked,
}

pub fn view(s: &Ui, t: &Theme) -> El {
    let body = match s.stage() {
        Stage::Ready { uid, inventory } => ready(s, t, uid, inventory),
        stage => pending(s, t, stage),
    };
    card(t)
        .key("status")
        .padding(20.0)
        .gap(16.0)
        .children(account_picker(s, t))
        .child(body)
}

fn pending(s: &Ui, t: &Theme, stage: Stage) -> El {
    let headline = problem(s, t).unwrap_or_else(|| match stage {
        Stage::Reading { uid } => Headline {
            glyph: Mark::Busy,
            title: format!("Reading UID {uid}"),
            detail: "You're logged in. Your relics, light cones and characters arrive while the game finishes loading.".into(),
            actions: Vec::new(),
        },
        _ => Headline {
            glyph: Mark::Listening,
            title: "Log in to the game".into(),
            detail: "Start Honkai: Star Rail and log in. Already playing? Log out to the title screen and log back in. \
                     The archiver only sees your account while the game logs in."
                .into(),
            actions: Vec::new(),
        },
    });

    let capturing = if s.needs_admin || s.capture.error.is_some() {
        Step::Blocked
    } else {
        Step::Done
    };
    let (login, data) = match stage {
        _ if capturing != Step::Done => (Step::Todo, Step::Todo),
        Stage::Waiting if s.capture.unsupported => (Step::Blocked, Step::Todo),
        Stage::Waiting => (Step::Active, Step::Todo),
        _ if s.capture.stalled || s.capture.unsupported => (Step::Done, Step::Blocked),
        _ => (Step::Done, Step::Active),
    };

    let actions = (!headline.actions.is_empty()).then(|| row().gap(8.0).children(headline.actions));
    col()
        .key("pending")
        .w_full()
        .gap(14.0)
        .child(title_row(
            mark(&headline.glyph, t),
            text(headline.title).key("headline").font(theme::HEADLINE).color(t.text).wrap(),
        ))
        .child(text(headline.detail).key("detail").font(theme::BODY).color(t.muted).wrap())
        .children(actions)
        .child(
            row()
                .key("steps")
                .w_full()
                .gap(6.0)
                .pt(2.0)
                .child(step("Capture traffic", capturing, t))
                .child(step("Log in", login, t))
                .child(step("Load account", data, t)),
        )
}

/// Something that keeps the data from coming in, most pressing first
fn problem(s: &Ui, t: &Theme) -> Option<Headline> {
    let k = &t.kit;
    let link = |key: &'static str, label: &'static str, url: &'static str| {
        Button::new(label)
            .variant(ButtonVariant::Ghost)
            .on_press(move |_: &mut Ui| super::open_url(url))
            .build(k)
            .key(key)
    };

    if s.needs_admin {
        return Some(Headline {
            glyph: Mark::Problem(&icons::SHIELD),
            title: "Administrator rights needed".into(),
            detail: "This version needs administrator rights to see the game's traffic.".into(),
            actions: vec![
                Button::new("Restart as administrator")
                    .variant(ButtonVariant::Contrast)
                    .on_press_tx(|s: &mut Ui, tx: &mut Tx| {
                        if let Some(services) = &s.services {
                            services.restart_as_admin(tx);
                        }
                    })
                    .build(k)
                    .key("restart-as-admin"),
                link("get-pcap-version", "Get the no-admin version", super::RELEASES_URL),
            ],
        });
    }
    if let Some(error) = &s.capture.error {
        let npcap = cfg!(all(windows, feature = "pcap"));
        let detail = if npcap {
            format!(
                "The archiver uses Npcap to see the game's traffic. Install it, or reinstall it if it's already there. \
                 Trying again meanwhile. ({})",
                error.message
            )
        } else {
            format!("{}. Trying again…", capitalize(&error.message))
        };
        let mut actions = Vec::new();
        if npcap {
            actions.push(
                Button::new("Download Npcap")
                    .variant(ButtonVariant::Contrast)
                    .on_press(|_: &mut Ui| super::open_url(NPCAP_URL))
                    .build(k)
                    .key("download-npcap"),
            );
        }
        return Some(Headline {
            glyph: Mark::Problem(&icons::ALERT),
            title: "Can't capture network traffic".into(),
            detail,
            actions,
        });
    }
    if s.capture.unsupported {
        let mut actions = Vec::new();
        if s.updates_supported {
            actions.push(
                Button::new("Check for updates")
                    .variant(ButtonVariant::Contrast)
                    .on_press(|s: &mut Ui| s.check_for_update())
                    .disabled(matches!(s.update, Update::Checking | Update::Installing))
                    .build(k)
                    .key("check-for-updates"),
            );
        }
        actions.push(link("ask-on-discord", "Ask on Discord", super::DISCORD_URL));
        return Some(Headline {
            glyph: Mark::Problem(&icons::ALERT),
            title: "This game version isn't supported yet".into(),
            detail: "Each game update needs a new release of the archiver before it can read the game's traffic.".into(),
            actions,
        });
    }
    if s.capture.stalled {
        return Some(Headline {
            glyph: Mark::Problem(&icons::ALERT),
            title: "Capture interrupted".into(),
            detail: "Some game data was missed, so the rest of this login can't be read. Log out to the title screen and log back in."
                .into(),
            actions: Vec::new(),
        });
    }
    None
}

fn ready(s: &Ui, t: &Theme, uid: u32, inventory: Inventory) -> El {
    let tile = |key: &'static str, count: usize, label: &'static str| {
        col()
            .key(key)
            .grow(1.0)
            .min_w(0.0)
            .py(10.0)
            .gap(2.0)
            .items_center()
            .radius(10.0)
            .fill(t.well)
            .child(text(group_digits(count)).key("count").font(theme::NUMBER).color(t.text))
            .child(text(label).font(theme::SMALL).color(t.muted).truncate())
    };

    // An interrupted capture still has the data from before, but the user has to relog to catch up
    let (glyph, tint, title, note, note_color) = if s.capture.stalled {
        (
            &icons::ALERT,
            t.warn,
            "Missing recent changes",
            "The capture was interrupted, so changes since then aren't here. Log out to the title screen and log back in to catch up.",
            t.text,
        )
    } else {
        (
            &icons::CHECK,
            t.good,
            "Ready to export",
            "Keeps updating while you play. Live sync sends every change as it happens.",
            t.faint,
        )
    };
    let mark = el()
        .size(36.0, 36.0)
        .radius(18.0)
        .center()
        .fill(tint.alpha(0.16))
        .child(icon(glyph, 20.0).color(tint));

    col()
        .key("ready")
        .w_full()
        .gap(16.0)
        .child(title_row(
            mark,
            col()
                .gap(2.0)
                .child(text(title).key("headline").font(theme::HEADLINE).color(t.text))
                .child(text(format!("UID {uid}")).key("uid").font(theme::LABEL).color(t.muted)),
        ))
        .child(
            row()
                .key("inventory")
                .w_full()
                .gap(6.0)
                .child(tile("relics", inventory.relics, "Relics"))
                .child(tile("light-cones", inventory.light_cones, "Light cones"))
                .child(tile("characters", inventory.characters, "Characters"))
                .child(tile("materials", inventory.materials, "Materials")),
        )
        .child(text(note).key("note").font(theme::SMALL).color(note_color).wrap())
}

/// The status mark beside the headline
fn title_row(mark: El, title: El) -> El {
    row().w_full().gap(12.0).center_y().child(mark).child(title.grow(1.0).min_w(0.0))
}

/// Picks the account to export when more than one logged in
fn account_picker(s: &Ui, t: &Theme) -> Option<El> {
    if s.accounts.len() < 2 {
        return None;
    }
    let labels: Vec<String> = s.accounts.iter().map(|a| format!("UID {}", a.uid)).collect();
    let selected = s.accounts.iter().position(|a| Some(a.uid) == s.selected);
    let uids: Vec<u32> = s.accounts.iter().map(|a| a.uid).collect();
    Some(
        SegmentedControl::new(&labels, selected)
            .label("Account")
            .fit()
            .on_pick(move |s: &mut Ui, i, _| s.select(uids[i]))
            .build(&t.kit)
            .key("accounts"),
    )
}

fn mark(glyph: &Mark, t: &Theme) -> El {
    let (inner, tint) = match glyph {
        Mark::Listening => (el().size(10.0, 10.0).radius(5.0).fill(t.accent).cycle(PULSE), t.accent),
        Mark::Busy => (rune_kit::Spinner::new().size(18.0).color(t.accent).build(&t.kit), t.accent),
        Mark::Problem(glyph) => (icon(glyph, 20.0).color(t.bad), t.bad),
    };
    el().key("mark")
        .size(36.0, 36.0)
        .radius(18.0)
        .center()
        .fill(tint.alpha(0.14))
        .child(inner)
}

fn step(label: &'static str, state: Step, t: &Theme) -> El {
    // Done is a full bar with a tick, the step in progress a faint breathing bar
    let (bar, ink) = match state {
        Step::Done => (t.accent, t.muted),
        Step::Active => (t.accent.alpha(0.35), t.text),
        Step::Todo => (t.card_border, t.faint),
        Step::Blocked => (t.bad, t.bad),
    };
    let mut bar = el().w_full().h(3.0).radius(1.5).fill(bar);
    if state == Step::Active {
        bar = bar.cycle(PULSE);
    }
    let mut caption = row().gap(4.0).center_y();
    if state == Step::Done {
        caption = caption.child(icon(&icons::CHECK, 12.0).color(t.accent));
    }
    let font = if state == Step::Active { theme::LABEL } else { theme::SMALL };
    caption = caption.child(text(label).font(font).color(ink).truncate().min_w(0.0));
    col().key(label).grow(1.0).min_w(0.0).gap(6.0).child(bar).child(caption)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// 12345 as "12,345"
fn group_digits(n: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
