//! The two ways into the optimizer: live sync, which keeps it current while the app runs, and an
//! export file for its import. Laid out like the game's detail band: a gold title, what it means,
//! and the action past a divider.

use rune::{Cx, El, Icon, Tx, col, el, icon, row, text};

use super::parts::{Pill, pill, plate, rule, tab};
use crate::gui::icons;
use crate::gui::state::{LiveSync, Panel, Stage, Ui};
use crate::gui::theme::{self, Theme};

pub fn view(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
    plate("delivery", cx, t)
        .w_full()
        .child(tab("delivery-tab", "Send to the optimizer", t.secondary, cx, t))
        .child(live_sync(s, t))
        .child(el().w_full().px(22.0).child(rule(t)))
        .child(export_file(s, t))
}

fn live_sync(s: &Ui, t: &Theme) -> El {
    let (status, tint, address) = match &s.sync {
        LiveSync::Starting => ("Starting…".to_string(), t.faint, None),
        LiveSync::Running { port, clients: 0 } => ("Waiting for the optimizer".to_string(), t.faint, Some(*port)),
        LiveSync::Running { port, clients: 1 } => ("Optimizer connected".to_string(), t.good, Some(*port)),
        LiveSync::Running { port, clients } => (format!("{clients} optimizer tabs connected"), t.good, Some(*port)),
        LiveSync::Failed(error) => (format!("Couldn't start: {error}"), t.bad, None),
        LiveSync::Closed => ("Paused while the update installs".to_string(), t.faint, None),
    };

    let mut detail = col().gap(3.0).child(
        row().gap(7.0).center_y().child(el().size(6.0, 6.0).radius(3.0).fill(tint)).child(
            text(status)
                .key("sync-status")
                .font(theme::SMALL)
                .color(if tint == t.faint { t.muted } else { tint })
                .truncate(),
        ),
    );

    let action = match (&s.sync, address) {
        (_, Some(port)) => {
            let url = format!("ws://localhost:{port}/ws");
            detail = detail.child(text(url.clone()).key("address").font(theme::MONO).color(t.faint).selectable(true));
            Some(pill(
                "copy-address",
                "Copy",
                Some(&icons::COPY),
                Pill::Secondary,
                true,
                t,
                move |s: &mut Ui, tx: &mut Tx| {
                    tx.copy(url.clone());
                    s.notify(
                        "Address copied",
                        Some("Paste it into the optimizer's live sync".into()),
                        false,
                        None,
                        tx,
                    );
                },
            ))
        }
        (LiveSync::Failed(_), None) => Some(pill("change-port", "Change port", None, Pill::Secondary, true, t, |s, _| {
            s.toggle_panel(Panel::Settings)
        })),
        _ => None,
    };

    option_row("live-sync", &icons::BROADCAST, "Live sync", detail, action, t)
}

fn export_file(s: &Ui, t: &Theme) -> El {
    let ready = matches!(s.stage(), Stage::Ready { .. });
    let detail = if ready {
        "A .json file for the optimizer's import"
    } else {
        "Ready once your account's data is in"
    };
    let save = pill(
        "save-export",
        "Save",
        Some(&icons::DOWNLOAD),
        if ready { Pill::Primary } else { Pill::Secondary },
        ready,
        t,
        |s: &mut Ui, tx: &mut Tx| s.save_export(tx),
    );
    option_row(
        "export-file",
        &icons::DOWNLOAD,
        "Export file",
        text(detail).font(theme::SMALL).color(t.muted).truncate(),
        Some(save),
        t,
    )
}

fn option_row(key: &'static str, glyph: &'static Icon, title: &'static str, detail: El, action: Option<El>, t: &Theme) -> El {
    // An icon in a double ring, like the nodes of the game's circuits
    let medallion = el().size(38.0, 38.0).radius(19.0).center().border(1.0, t.accent_line).child(
        el().size(30.0, 30.0)
            .radius(15.0)
            .center()
            .fill(t.well)
            .border(1.0, t.line)
            .child(icon(glyph, 15.0).color(t.accent)),
    );
    let mut row = row()
        .key(key)
        .w_full()
        .px(22.0)
        .py(14.0)
        .gap(14.0)
        .center_y()
        .child(medallion)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(text(title).font(theme::TITLE).color(t.accent))
                .child(detail),
        );
    if let Some(action) = action {
        row = row.child(el().w(1.0).h(34.0).fill(t.line)).child(action);
    }
    row
}
