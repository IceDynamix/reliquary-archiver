//! The logs panel, for when something goes wrong: the latest lines, how much detail to log, and
//! the whole log to copy or save for a bug report.

use rune::{El, ScrollSpec, Tx, col, row, text};
use rune_kit::{Button, Dialog, SegmentedControl};
use tracing::level_filters::LevelFilter;

use crate::gui::state::{LOG_TAIL, Panel, Ui};
use crate::gui::theme::{self, Theme};

const LEVELS: [(LevelFilter, &str); 3] = [
    (LevelFilter::INFO, "Info"),
    (LevelFilter::DEBUG, "Debug"),
    (LevelFilter::TRACE, "Trace"),
];

pub fn dialog(s: &Ui, t: &Theme, layer: &mut Vec<El>) {
    let k = &t.kit;
    let labels = LEVELS.map(|(_, label)| label);
    let level = LEVELS.iter().position(|(level, _)| *level == s.log_level);

    let controls = row()
        .key("log-controls")
        .w_full()
        .px(16.0)
        .gap(8.0)
        .center_y()
        .child(
            SegmentedControl::new(&labels, level)
                .label("Detail")
                .fit()
                .on_pick(|s: &mut Ui, i, _| s.set_log_level(LEVELS[i].0))
                .build(k)
                .key("log-level"),
        )
        .child(col().grow(1.0))
        .child(
            Button::new("Copy")
                .on_press_tx(|_: &mut Ui, tx: &mut Tx| {
                    tx.copy(crate::logging::lines().join("\n"));
                })
                .build(k)
                .key("copy-logs"),
        )
        .child(
            Button::new("Save…")
                .on_press_tx(|s: &mut Ui, tx: &mut Tx| s.save_logs(tx))
                .build(k)
                .key("save-logs"),
        );

    let lines = s.log_tail.iter().enumerate().map(|(i, line)| {
        text(line.clone())
            .key(format!("line-{i}"))
            .font(theme::MONO)
            .color(t.muted)
            .wrap_anywhere()
            .selectable(true)
    });
    let log = col()
        .key("log-lines")
        .w_full()
        .h(360.0)
        .min_h(0.0)
        .padding(10.0)
        .gap(2.0)
        .radius(8.0)
        .fill(t.bg)
        .border(1.0, t.card_border)
        .children(lines)
        .scroll(ScrollSpec { stick_bottom: true });

    Dialog::new("logs", "Logs")
        .open(s.panel == Some(Panel::Logs))
        .width(640.0)
        .fit()
        .on_close(|s: &mut Ui| s.panel = None)
        .child(controls)
        .child(
            col().w_full().padding(16.0).gap(8.0).child(log).child(
                text(format!(
                    "Showing the latest {LOG_TAIL} lines. Copy or save the whole log to attach it to a bug report."
                ))
                .font(theme::SMALL)
                .color(t.faint)
                .wrap(),
            ),
        )
        .build(k, layer);
}
