//! The logs sheet, for when something goes wrong: the latest lines, how much detail to log, and
//! the whole log to copy or save for a bug report.

use rune::{Cx, El, ScrollSpec, Span, Tx, col, rich, row, text};
use rune_kit::SegmentedControl;
use tracing::level_filters::LevelFilter;

use super::parts::{Pill, Sheet, kicker, pill, sheet};
use crate::gui::state::{LOG_TAIL, Panel, Ui};
use crate::gui::theme::{self, Theme};

const LEVELS: [(LevelFilter, &str); 3] = [
    (LevelFilter::INFO, "Info"),
    (LevelFilter::DEBUG, "Debug"),
    (LevelFilter::TRACE, "Trace"),
];

pub fn dialog(s: &Ui, cx: &Cx<Ui>, t: &Theme, layer: &mut Vec<El>) {
    if s.panel != Some(Panel::Logs) {
        return;
    }
    let labels = LEVELS.map(|(_, label)| label);
    let level = LEVELS.iter().position(|(level, _)| *level == s.log_level);

    let controls = row()
        .key("log-controls")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(kicker("Detail", t.muted))
        .child(
            SegmentedControl::new(&labels, level)
                .label("Detail")
                .fit()
                .on_pick(|s: &mut Ui, i, _| s.set_log_level(LEVELS[i].0))
                .build(&t.kit)
                .key("log-level"),
        )
        .child(col().grow(1.0))
        .child(pill("copy-logs", "Copy all", None, Pill::Secondary, true, t, |_, tx: &mut Tx| {
            tx.copy(crate::logging::lines().join("\n"))
        }))
        .child(pill(
            "save-logs",
            "Save…",
            None,
            Pill::Secondary,
            true,
            t,
            |s: &mut Ui, tx: &mut Tx| s.save_logs(tx),
        ));

    let lines = s.log_tail.iter().enumerate().map(|(i, line)| log_line(i, line, t));
    let log = col()
        .key("log-lines")
        .w_full()
        .h(340.0)
        .min_h(0.0)
        .padding(12.0)
        .gap(3.0)
        .fill(t.well)
        .border(1.0, t.line)
        .children(lines)
        .scroll(ScrollSpec { stick_bottom: true });

    let body = col()
        .w_full()
        .px(22.0)
        .pt(16.0)
        .pb(22.0)
        .gap(12.0)
        .child(controls)
        .child(log)
        .child(
            text(format!(
                "The latest {LOG_TAIL} lines. Copy or save the whole log to attach it to a bug report."
            ))
            .font(theme::SMALL)
            .color(t.faint)
            .wrap(),
        );
    let spec = Sheet {
        key: "logs",
        kicker: "Diagnostics",
        title: "Logs",
        width: 680.0,
        close: |s| s.panel = None,
    };
    sheet(spec, body, cx, t, layer);
}

/// A line with its time faded and its level coloured, wrapping as one paragraph
fn log_line(i: usize, line: &str, t: &Theme) -> El {
    let mut words = line.split_whitespace();
    let (Some(time), Some(level)) = (words.next(), words.next()) else {
        return text(line.to_string())
            .key(format!("line-{i}"))
            .font(theme::MONO)
            .color(t.muted)
            .wrap_anywhere();
    };
    let message = line[line.find(level).map_or(0, |at| at + level.len())..].trim_start();
    if !matches!(level, "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE") {
        return text(line.to_string())
            .key(format!("line-{i}"))
            .font(theme::MONO)
            .color(t.muted)
            .wrap_anywhere();
    }
    // 2026-11-07T21:12:40.120Z shows as 21:12:40
    let time = time.split('T').nth(1).map_or(time, |clock| clock.get(..8).unwrap_or(clock));
    let level_color = match level {
        "ERROR" => t.bad,
        "WARN" => t.warn,
        "INFO" => t.accent,
        _ => t.faint,
    };

    let content = format!("{time}  {level:<5}  {message}");
    let level_start = time.len() + 2;
    let message_start = level_start + level.len().max(5) + 2;
    let spans = vec![
        Span::new(0, time.len()).color(t.faint),
        Span::new(level_start, level_start + level.len()).color(level_color),
        Span::new(message_start, content.len()).color(t.text),
    ];
    rich(content, spans)
        .key(format!("line-{i}"))
        .font(theme::MONO)
        .color(t.text)
        .wrap_anywhere()
        .selectable(true)
}
