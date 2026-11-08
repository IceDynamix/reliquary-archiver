//! The two ways into the optimizer: live sync, which keeps it current while the app runs, and an
//! export file for its import.

use rune::{El, Icon, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};

use super::card;
use crate::gui::icons;
use crate::gui::state::{LiveSync, Panel, Stage, Ui};
use crate::gui::theme::{self, Theme};

pub fn view(s: &Ui, t: &Theme) -> El {
    card(t)
        .key("delivery")
        .child(text("Send to the optimizer").font(theme::LABEL).color(t.muted).px(14.0).pt(12.0))
        .child(live_sync(s, t))
        .child(el().w_full().h(1.0).fill(t.card_border))
        .child(export_file(s, t))
}

fn live_sync(s: &Ui, t: &Theme) -> El {
    let k = &t.kit;
    let (status, tint, address) = match &s.sync {
        LiveSync::Starting => ("Starting…".to_string(), t.faint, None),
        LiveSync::Running { port, clients: 0 } => ("Waiting for the optimizer to connect".to_string(), t.faint, Some(*port)),
        LiveSync::Running { port, clients: 1 } => ("Optimizer connected".to_string(), t.good, Some(*port)),
        LiveSync::Running { port, clients } => (format!("{clients} optimizer tabs connected"), t.good, Some(*port)),
        LiveSync::Failed(error) => (format!("Couldn't start: {error}"), t.bad, None),
        LiveSync::Closed => ("Paused while the update installs".to_string(), t.faint, None),
    };

    let status = row().gap(6.0).center_y().child(el().size(6.0, 6.0).radius(3.0).fill(tint)).child(
        text(status)
            .key("sync-status")
            .font(theme::SMALL)
            .color(if tint == t.faint { t.muted } else { tint })
            .truncate(),
    );

    let (detail, trailing) = match (&s.sync, address) {
        (_, Some(port)) => {
            let url = format!("ws://localhost:{port}/ws");
            let address = text(url.clone()).key("address").font(theme::MONO).color(t.faint).selectable(true);
            let copy = IconButton::new(&icons::COPY)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Copy address")
                .help("Copy address")
                .on_press_tx(move |s: &mut Ui, tx: &mut Tx| {
                    tx.copy(url.clone());
                    s.notify("Address copied", None, false, None, tx);
                })
                .build(k)
                .key("copy-address");
            (
                col()
                    .gap(1.0)
                    .child(status)
                    .child(row().gap(2.0).center_y().child(address).child(copy)),
                el(),
            )
        }
        (LiveSync::Failed(_), None) => (
            status,
            Button::new("Change port")
                .on_press(|s: &mut Ui| s.toggle_panel(Panel::Settings))
                .build(k)
                .key("change-port"),
        ),
        _ => (status, el()),
    };

    option_row("live-sync", &icons::BROADCAST, "Live sync", detail, trailing, t)
}

fn export_file(s: &Ui, t: &Theme) -> El {
    let ready = matches!(s.stage(), Stage::Ready { .. });
    let detail = match s.stage() {
        Stage::Ready { .. } => "A .json file for the optimizer's import".to_string(),
        _ => "Available once your account's data is in".to_string(),
    };
    let save = Button::new("Save…")
        .variant(if ready { ButtonVariant::Contrast } else { ButtonVariant::Neutral })
        .icon(&icons::DOWNLOAD)
        .disabled(!ready)
        .on_press_tx(|s: &mut Ui, tx: &mut Tx| s.save_export(tx))
        .build(&t.kit)
        .key("save-export");

    option_row(
        "export-file",
        &icons::DOWNLOAD,
        "Export file",
        text(detail).font(theme::SMALL).color(t.muted).truncate(),
        save,
        t,
    )
}

fn option_row(key: &'static str, glyph: &'static Icon, title: &'static str, detail: El, trailing: El, t: &Theme) -> El {
    row()
        .key(key)
        .w_full()
        .padding(14.0)
        .gap(12.0)
        .center_y()
        .child(
            el().size(32.0, 32.0)
                .radius(8.0)
                .center()
                .fill(t.well)
                .child(icon(glyph, 16.0).color(t.muted)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(text(title).font(theme::TITLE).color(t.text))
                .child(detail),
        )
        .child(trailing)
}
