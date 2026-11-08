//! Drives the real view headlessly: presses go through Rune's hit testing and handlers, as a
//! click in the window would.

use rune::{Env, FileDialogKind, Runtime};

use super::scenes::SCENES;
use super::settings::{Anchor, Settings};
use super::state::{Panel, Ui};
use super::view::view;

fn runtime(scene: &str) -> Runtime<Ui> {
    let mut ui = Ui::new(Settings::default());
    let (_, apply) = SCENES.iter().find(|(name, _)| *name == scene).expect("no such scene");
    apply(&mut ui);
    let mut rt = Runtime::new(
        ui,
        view,
        (440.0, 600.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    rt.frame(0.0);
    rt
}

fn click(rt: &mut Runtime<Ui>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("nothing keyed {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn saving_an_export_asks_where_to_save_a_json_file() {
    let mut rt = runtime("ready");
    click(&mut rt, "save-export");
    let (kind, options) = rt.file_dialog().expect("no file dialog");
    assert_eq!(kind, FileDialogKind::Save);
    assert!(
        options
            .file_name
            .as_deref()
            .is_some_and(|name| name.starts_with("archive_output-") && name.ends_with(".json"))
    );
}

#[test]
fn nothing_can_be_saved_before_the_account_is_ready() {
    let mut rt = runtime("reading");
    click(&mut rt, "save-export");
    assert!(rt.file_dialog().is_none());
}

#[test]
fn a_capture_file_is_picked_with_an_open_dialog() {
    let mut rt = runtime("waiting");
    click(&mut rt, "open-capture-file");
    let (kind, options) = rt.file_dialog().expect("no file dialog");
    assert_eq!(kind, FileDialogKind::Open);
    assert!(
        options
            .filters
            .iter()
            .any(|(_, extensions)| extensions.iter().any(|e| e == "pcap" || e == "etl"))
    );
}

#[test]
fn settings_open_from_the_header_and_switches_change_them() {
    let mut rt = runtime("waiting");
    rt.update(|s, _| s.has_tray = true);
    click(&mut rt, "open-settings");
    assert_eq!(rt.state.panel, Some(Panel::Settings));

    click(&mut rt, "close-to-tray/switch");
    assert!(rt.state.settings.close_to_tray);
    click(&mut rt, "start-minimized/switch");
    assert!(rt.state.settings.start_minimized);
}

#[test]
fn a_new_port_waits_for_the_server_before_it_is_saved() {
    let mut rt = runtime("settings");
    rt.update(|s, _| s.port_edit.replace_text("4000"));
    rt.frame(0.0);
    click(&mut rt, "apply-port");
    assert_eq!(rt.state.pending_port, Some(4000));
    assert_eq!(rt.state.settings.ws_port, 23313);
}

#[test]
fn the_position_picker_moves_the_cards() {
    let mut rt = runtime("settings");
    click(&mut rt, "anchor-2-2");
    assert_eq!(rt.state.settings.card_anchor, Anchor::BottomRight);
    click(&mut rt, "anchor-0-1");
    assert_eq!(rt.state.settings.card_anchor, Anchor::Left);
}

#[test]
fn every_scene_renders() {
    for (name, _) in SCENES {
        let rt = runtime(name);
        assert!(rt.find("page").is_some(), "{name} has no page");
    }
}
