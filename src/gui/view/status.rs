//! Where capturing stands: the steps as a node track, what to do next while waiting, what's in
//! the way when something is, and what the account holds once it's in.

use rune::{Color, Cx, El, Icon, col, el, icon, row, text};
use rune_kit::SegmentedControl;

use super::parts::{Node, Pill, kicker, node_track, pill, plate, rule, tab};
use crate::gui::icons;
use crate::gui::state::{Inventory, Stage, Ui, Update};
use crate::gui::theme::{self, Theme};

const NPCAP_URL: &str = "https://npcap.com/#download";

/// What the status plate leads with
struct Headline {
    /// The small capitals over the title
    kicker: &'static str,
    title: String,
    detail: String,
    /// Marks a problem, in red
    problem: Option<&'static Icon>,
    actions: Vec<El>,
}

pub fn view(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
    let body = match s.stage() {
        Stage::Ready { uid, inventory } => ready(s, t, uid, inventory),
        stage => pending(s, t, stage),
    };
    let label = match s.stage() {
        Stage::Ready { .. } => "Archive",
        _ => "Capture",
    };
    plate("status", cx, t)
        .w_full()
        .child(tab("status-tab", label, t.accent, cx, t))
        .child(
            col()
                .w_full()
                .px(22.0)
                .pt(12.0)
                .pb(20.0)
                .gap(16.0)
                .children(account_picker(s, t))
                .child(body),
        )
}

fn pending(s: &Ui, t: &Theme, stage: Stage) -> El {
    let headline = problem(s, t).unwrap_or_else(|| match stage {
        Stage::Reading { uid } => Headline {
            kicker: "Logged in",
            title: format!("Reading UID {uid}"),
            detail: "Your relics, light cones and characters arrive while the game finishes loading.".into(),
            problem: None,
            actions: Vec::new(),
        },
        _ => Headline {
            kicker: "Waiting for the game",
            title: "Log in to the game".into(),
            detail: "Start Honkai: Star Rail and log in. Already playing? Log out to the title screen and log back in. \
                     The archiver only sees your account while the game logs in."
                .into(),
            problem: None,
            actions: Vec::new(),
        },
    });

    let capturing = if s.needs_admin || s.capture.error.is_some() {
        Node::Blocked
    } else {
        Node::Done
    };
    let (login, data) = match stage {
        _ if capturing != Node::Done => (Node::Todo, Node::Todo),
        Stage::Waiting if s.capture.unsupported => (Node::Blocked, Node::Todo),
        Stage::Waiting => (Node::Active, Node::Todo),
        _ if s.capture.stalled || s.capture.unsupported => (Node::Done, Node::Blocked),
        _ => (Node::Done, Node::Active),
    };

    let accent = if headline.problem.is_some() { t.bad } else { t.accent };
    let mut title = row().w_full().gap(10.0).center_y();
    if let Some(glyph) = headline.problem {
        title = title.child(icon(glyph, 22.0).color(t.bad));
    }
    title = title.child(
        text(headline.title)
            .key("headline")
            .font(theme::HEADLINE)
            .color(t.text)
            .wrap()
            .grow(1.0)
            .min_w(0.0),
    );

    let actions = (!headline.actions.is_empty()).then(|| row().gap(8.0).children(headline.actions));
    col()
        .key("pending")
        .w_full()
        .gap(14.0)
        .child(node_track(&[("Capture", capturing), ("Log in", login), ("Load account", data)], t))
        .child(rule(t))
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(kicker(headline.kicker, accent))
                .child(title)
                .child(text(headline.detail).key("detail").font(theme::BODY).color(t.muted).wrap()),
        )
        .children(actions)
}

/// Something that keeps the data from coming in, most pressing first
fn problem(s: &Ui, t: &Theme) -> Option<Headline> {
    if s.needs_admin {
        return Some(Headline {
            kicker: "Can't capture",
            title: "Administrator rights needed".into(),
            detail: "This version needs administrator rights to see the game's traffic.".into(),
            problem: Some(&icons::SHIELD),
            actions: vec![
                pill(
                    "restart-as-admin",
                    "Restart as administrator",
                    None,
                    Pill::Primary,
                    true,
                    t,
                    |s, tx| {
                        if let Some(services) = &s.services {
                            services.restart_as_admin(tx);
                        }
                    },
                ),
                pill(
                    "get-pcap-version",
                    "Get the no-admin version",
                    None,
                    Pill::Quiet,
                    true,
                    t,
                    |_, _| super::open_url(super::RELEASES_URL),
                ),
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
            actions.push(pill("download-npcap", "Download Npcap", None, Pill::Primary, true, t, |_, _| {
                super::open_url(NPCAP_URL)
            }));
        }
        return Some(Headline {
            kicker: "Can't capture",
            title: "No network traffic".into(),
            detail,
            problem: Some(&icons::ALERT),
            actions,
        });
    }
    if s.capture.unsupported {
        let mut actions = Vec::new();
        if s.updates_supported {
            let checking = matches!(s.update, Update::Checking | Update::Installing);
            actions.push(pill(
                "check-for-updates",
                "Check for updates",
                None,
                Pill::Primary,
                !checking,
                t,
                |s, _| s.check_for_update(),
            ));
        }
        actions.push(pill("ask-on-discord", "Ask on Discord", None, Pill::Quiet, true, t, |_, _| {
            super::open_url(super::DISCORD_URL)
        }));
        return Some(Headline {
            kicker: "Can't read the game",
            title: "This game version isn't supported yet".into(),
            detail: "Each game update needs a new release of the archiver before it can read the game's traffic.".into(),
            problem: Some(&icons::ALERT),
            actions,
        });
    }
    if s.capture.stalled {
        return Some(Headline {
            kicker: "Data missed",
            title: "Capture interrupted".into(),
            detail: "Some game data was missed, so the rest of this login can't be read. Log out to the title screen and log back in."
                .into(),
            problem: Some(&icons::ALERT),
            actions: Vec::new(),
        });
    }
    None
}

fn ready(s: &Ui, t: &Theme, uid: u32, inventory: Inventory) -> El {
    // An interrupted capture still has the data from before, but the user has to relog to catch up.
    // So does one picked up from the last run, which missed what changed while it was closed
    let picked_up = s.accounts.iter().any(|a| a.uid == uid && a.picked_up);
    let (kicker_text, kicker_tint, title, note, note_color): (_, Color, _, _, Color) = if s.capture.stalled {
        (
            "Capture interrupted",
            t.warn,
            "Missing recent changes",
            "Changes since the interruption aren't here. Log out to the title screen and log back in to catch up.",
            t.text,
        )
    } else if picked_up {
        (
            "Picked up from last run",
            t.warn,
            "May be missing changes",
            "The archiver restarted while you were logged in, so anything that changed while it was closed isn't here \
             and the export may be out of date. Log out to the title screen and log back in to catch up.",
            t.text,
        )
    } else {
        (
            "Recorded",
            t.good,
            "Archive ready",
            "Keeps updating while you play. Live sync sends every change as it happens.",
            t.faint,
        )
    };

    let count = |key: &'static str, count: usize, label: &'static str| {
        col()
            .key(key)
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .items_center()
            .child(text(group_digits(count)).key("count").font(theme::NUMBER).color(t.accent))
            .child(kicker(label, t.muted))
    };
    let divider = || el().w(1.0).h(40.0).fill(t.line).self_center();

    col()
        .key("ready")
        .w_full()
        .gap(16.0)
        .child(
            col()
                .gap(4.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(kicker(kicker_text, kicker_tint))
                        .child(el().size(3.0, 3.0).radius(1.5).fill(t.faint))
                        .child(kicker(&format!("UID {uid}"), t.muted).key("uid")),
                )
                .child(text(title).key("headline").font(theme::HEADLINE).color(t.text)),
        )
        .child(rule(t))
        .child(
            row()
                .key("inventory")
                .w_full()
                .py(4.0)
                .child(count("relics", inventory.relics, "Relics"))
                .child(divider())
                .child(count("light-cones", inventory.light_cones, "Light cones"))
                .child(divider())
                .child(count("characters", inventory.characters, "Characters"))
                .child(divider())
                .child(count("materials", inventory.materials, "Materials")),
        )
        .child(text(note).key("note").font(theme::SMALL).color(note_color).wrap())
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
        row().w_full().gap(10.0).center_y().child(kicker("Account", t.muted)).child(
            SegmentedControl::new(&labels, selected)
                .label("Account")
                .fit()
                .on_pick(move |s: &mut Ui, i, _| s.select(uids[i]))
                .build(&t.kit)
                .key("accounts"),
        ),
    )
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
