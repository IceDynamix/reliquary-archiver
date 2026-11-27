//! The two ways into the optimizer: live sync, which keeps it current while the app runs, and an
//! export file for its import. Laid out like the game's detail band: a gold title, what it means,
//! and the action past a divider.

use rune::{Cx, El, Tx, col, row, text};

use super::parts::{Pill, Tone, option_row, perforation, pill, status_dot, tab, ticket};
use crate::gui::icons;
use crate::gui::state::{LiveSync, Panel, Stage, Ui};
use crate::gui::theme::{self, Theme};

pub fn view(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
    ticket("delivery", &["delivery-perforation"], cx, t)
        .w_full()
        .child(tab("delivery-tab", "Optimizer", t.secondary, t))
        .child(live_sync(s, t).px(22.0).py(14.0))
        .child(perforation("delivery-perforation", cx, t))
        .child(export_file(s, t).px(22.0).py(14.0))
}

fn live_sync(s: &Ui, t: &Theme) -> El {
    let (status, tone, address) = match &s.sync {
        LiveSync::Starting => ("Starting…".to_string(), Tone::Quiet, None),
        LiveSync::Running { port, clients: 0 } => ("Waiting for the optimizer".to_string(), Tone::Quiet, Some(*port)),
        LiveSync::Running { port, clients: 1 } => ("Optimizer connected".to_string(), Tone::Good, Some(*port)),
        LiveSync::Running { port, clients } => (format!("{clients} optimizer tabs connected"), Tone::Good, Some(*port)),
        LiveSync::Failed(error) => (format!("Couldn't start: {error}"), Tone::Bad, None),
        LiveSync::Closed => ("Paused for update".to_string(), Tone::Quiet, None),
    };

    let mut detail = col().gap(3.0).child(
        row()
            .gap(7.0)
            .center_y()
            .child(status_dot(tone.dot(t), false, 6.0, t))
            .child(text(status).key("sync-status").font(theme::SMALL).color(tone.ink(t)).truncate()),
    );

    let action = match (&s.sync, address) {
        (_, Some(port)) => {
            let url = format!("ws://localhost:{port}/ws");
            detail = detail.child(text(url.clone()).key("address").font(theme::MONO).color(t.faint).selectable(true));
            Some(
                pill("copy-address", "Copy", Pill::Secondary)
                    .glyph(&icons::COPY)
                    .build(t, move |s: &mut Ui, tx: &mut Tx| {
                        tx.copy(url.clone());
                        s.notify_info("Address copied", "Paste it into the optimizer", tx);
                    }),
            )
        }
        (LiveSync::Failed(_), None) => {
            Some(pill("change-port", "Change port", Pill::Secondary).build(t, |s, _| s.toggle_panel(Panel::Settings)))
        }
        _ => None,
    };

    option_row("live-sync", &icons::BROADCAST, "Live sync", detail, action, t)
}

fn export_file(s: &Ui, t: &Theme) -> El {
    let ready = matches!(s.stage(), Stage::Ready { .. });
    let detail = if ready {
        "JSON for the optimizer's import"
    } else {
        "Available after login"
    };
    let save = pill("save-export", "Save", if ready { Pill::Primary } else { Pill::Secondary })
        .glyph(&icons::DOWNLOAD)
        .enabled(ready)
        .build(t, |s: &mut Ui, tx: &mut Tx| s.save_export(tx));
    option_row(
        "export-file",
        &icons::DOWNLOAD,
        "Export file",
        text(detail).font(theme::SMALL).color(t.muted).truncate(),
        save,
        t,
    )
}
