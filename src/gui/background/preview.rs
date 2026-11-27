//! The settings' preview of where the background sits: the window drawn small, with the image
//! placed in it, and what pressing and dragging on it does to the placement.

use super::{Grab, place};
use crate::gui::state::Ui;

/// How far an arrow key moves the image, as a share of the window
const POINT_STEP: f32 = 0.05;
/// How much of the image the preview shows past the window's edges, as a share of the window's
/// longer side
const MARGIN: f32 = 0.15;
/// The shortest and tallest the preview stands
const STAGE_HEIGHT: [f32; 2] = [200.0, 300.0];
/// How near a press has to be to the pin or the frame's corner to take it, in points
const REACH: f32 = 14.0;

/// The preview of where the image sits: the window drawn small, with the image placed in it and
/// what fills around it, and the rest of the image dim past its edges
#[derive(Clone, Copy)]
pub struct Stage {
    pub size: [f32; 2],
    /// Preview points to a window point
    pub k: f32,
    /// The window's own size
    real: [f32; 2],
    /// The image's own size, in points
    own: [f32; 2],
}

impl Stage {
    /// A preview `room` wide of a window `real` large, for an image `own` large
    pub fn new(room: f32, real: [f32; 2], own: [f32; 2]) -> Self {
        let margin = real[0].max(real[1]) * MARGIN;
        let span = real.map(|v| v + 2.0 * margin);
        let height = (room * span[1] / span[0]).clamp(STAGE_HEIGHT[0], STAGE_HEIGHT[1]);
        let k = (room / span[0]).min(height / span[1]);
        Stage {
            size: [room, height],
            k,
            real,
            own,
        }
    }

    /// The window the preview shows: the one tried, else the window's own
    pub fn window(&self, s: &Ui) -> [f32; 2] {
        s.background.trial.unwrap_or(self.real)
    }

    /// Where the image goes in that window, `[x, y, w, h]` in its points
    pub fn rect(&self, s: &Ui) -> [f32; 4] {
        let settings = &s.settings;
        place(
            settings.image_fit,
            settings.background_zoom,
            self.own,
            self.window(s),
            settings.background_anchor,
            settings.background_target,
        )
    }

    /// Where the pin is, 0 to 1 across and down the window
    pub fn pin(&self, s: &Ui) -> [f32; 2] {
        s.settings.background_target.unwrap_or(s.settings.background_anchor)
    }

    /// Where a window `window` large has its top-left drawn: in the middle of the preview, so a
    /// size tried grows and shrinks about it
    pub fn origin(&self, window: [f32; 2]) -> [f32; 2] {
        [0, 1].map(|n| (self.size[n] - window[n] * self.k) / 2.0)
    }

    /// A point on the preview, in the points of a window `window` large
    fn in_window(&self, window: [f32; 2], at: [f32; 2]) -> [f32; 2] {
        let origin = self.origin(window);
        [0, 1].map(|n| (at[n] - origin[n]) / self.k)
    }

    /// A point in a window `window` large, on the preview
    fn on_stage(&self, window: [f32; 2], at: [f32; 2]) -> [f32; 2] {
        let origin = self.origin(window);
        [0, 1].map(|n| origin[n] + at[n] * self.k)
    }

    /// What a press at `at` on the preview takes: the pin, the frame's corner, else the image
    pub fn grab_at(&self, s: &Ui, at: [f32; 2]) -> Grab {
        let window = self.window(s);
        let near = |p: [f32; 2]| {
            let p = self.on_stage(window, p);
            (p[0] - at[0]).hypot(p[1] - at[1]) <= REACH
        };
        let pin = self.pin(s);
        let [x, y, ..] = self.rect(s);
        if near([pin[0] * window[0], pin[1] * window[1]]) {
            Grab::Pin { from: [x, y], out: None }
        } else if near(window) {
            Grab::Resize
        } else {
            Grab::Pan([x, y])
        }
    }

    /// A drag on the preview: pressed, it takes what's under the press, then it moves that, and
    /// let go, it lets go of it
    pub fn drag(&self, s: &mut Ui, drag: &rune::Drag) {
        if drag.phase == rune::Phase::Down {
            s.background.grab = Some(self.grab_at(s, [drag.start_x, drag.start_y]));
        }
        match s.background.grab {
            Some(Grab::Pan(from)) => self.pan(s, [from[0] + drag.dx / self.k, from[1] + drag.dy / self.k]),
            Some(Grab::Pin { from, .. }) => {
                let released = drag.phase == rune::Phase::Up;
                let out = self.repin(s, from, [drag.x, drag.y], released);
                s.background.grab = Some(Grab::Pin { from, out });
            }
            Some(Grab::Resize) => self.resize(s, [drag.x, drag.y]),
            None => {}
        }
        if drag.phase == rune::Phase::Up {
            s.background.grab = None;
        }
    }

    /// Moves the image a step `by` (-1, 0 or 1 across and down), as an arrow key does
    pub fn nudge(&self, s: &mut Ui, by: [f32; 2]) {
        let window = self.window(s);
        let [x, y, ..] = self.rect(s);
        self.pan(s, [x + by[0] * POINT_STEP * window[0], y + by[1] * POINT_STEP * window[1]]);
    }

    /// Puts the image `to` from the window's top-left, as near as it goes. With the pin where its
    /// point is in the image, the point moves; with a pin of its own, the pin moves with the image
    fn pan(&self, s: &mut Ui, to: [f32; 2]) {
        let window = self.window(s);
        let [_, _, w, h] = self.rect(s);
        let size = [w, h];
        let anchor = s.settings.background_anchor;
        if s.settings.background_target.is_some() {
            let target = [0, 1].map(|n| ((to[n] + anchor[n] * size[n]) / window[n]).clamp(0.0, 1.0));
            s.set_background_target(Some(target));
        } else {
            s.set_background_anchor(follow(anchor, to, window, size));
        }
    }

    /// Pins the image, `from` the window's top-left, at its point under the preview's point `at`,
    /// as near as the image has one.
    ///
    /// Over the window the image stays exactly where it is: the pin goes to a whole percent of the
    /// window, as it's kept, and its point on the image is worked out from that, unrounded, rather
    /// than from where the image is now, so nothing drifts as it moves. Past the window's edges the
    /// pin waits there, as handed back, until `released`; then it goes to the nearest place in the
    /// window, and the image moves to keep the point under it
    fn repin(&self, s: &mut Ui, from: [f32; 2], at: [f32; 2], released: bool) -> Option<[f32; 2]> {
        let window = self.window(s);
        let [_, _, w, h] = self.rect(s);
        let size = [w.max(1.0), h.max(1.0)];
        // Over the image, and the part of it the preview shows
        let [seen_from, seen_to] = [[0.0, 0.0], self.size].map(|p| self.in_window(window, p));
        let at = self.in_window(window, at);
        let at = [0, 1].map(|n| at[n].clamp(from[n].max(seen_from[n]), (from[n] + size[n]).min(seen_to[n])));
        let inside = (0..2).all(|n| (0.0..=window[n]).contains(&at[n]));
        if !inside && !released {
            return Some(at);
        }
        let mut target = [0.0; 2];
        let mut anchor = [0.0; 2];
        for n in 0..2 {
            let to = (at[n].clamp(0.0, window[n]) / window[n] * 100.0).round() / 100.0;
            if inside {
                // Rounded to where the image still has a point
                let (least, most) = (from[n].max(0.0), (from[n] + size[n]).min(window[n]));
                let on = (to * window[n]).clamp(least, most);
                target[n] = on / window[n];
                anchor[n] = ((on - from[n]) / size[n]).clamp(0.0, 1.0);
            } else {
                target[n] = to;
                anchor[n] = ((at[n] - from[n]) / size[n]).clamp(0.0, 1.0);
            }
        }
        s.set_background_pin(anchor, Some(target));
        None
    }

    /// Puts the pin back where its point is in the image, the image staying where it is
    pub fn unpin(&self, s: &mut Ui) {
        let window = self.window(s);
        let [x, y, w, h] = self.rect(s);
        let anchor = follow(s.settings.background_anchor, [x, y], window, [w, h]);
        s.set_background_pin(anchor, None);
    }

    /// Tries the window at the size, about the preview's middle, with a corner at its point `at`
    fn resize(&self, s: &mut Ui, at: [f32; 2]) {
        s.background.trial = Some([0, 1].map(|n| {
            let size = 2.0 * (at[n] - self.size[n] / 2.0).abs() / self.k;
            let most = ((self.size[n] - 8.0) / self.k).max(self.real[n]);
            size.clamp(self.real[n] * 0.35, most)
        }));
    }
}

/// The point of an image `size` large that puts it `to` from the window's top-left when it goes
/// to the same place in the window, as CSS's `object-position` has it. Along a side as long as
/// the window's, where it goes doesn't matter, so that stays as `anchor` has it
fn follow(anchor: [f32; 2], to: [f32; 2], window: [f32; 2], size: [f32; 2]) -> [f32; 2] {
    [0, 1].map(|n| {
        let slack = window[n] - size[n];
        if slack.abs() < 0.5 {
            anchor[n]
        } else {
            (to[n] / slack).clamp(0.0, 1.0)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::settings::{ImageFit, Settings};

    /// A 1200 × 800 window with a 1920 × 1080 image covering it, zoomed in a little
    fn staged() -> (Ui, Stage) {
        let mut ui = Ui::new(Settings::default());
        ui.settings.image_fit = ImageFit::Cover;
        ui.settings.background_zoom = 1.3;
        ui.settings.background_anchor = [0.37, 0.61];
        (ui, Stage::new(600.0, [1200.0, 800.0], [1920.0, 1080.0]))
    }

    /// Pins the image at the window's point `at`, dragging there from where it is and letting go
    /// there if `released`
    fn drag_pin(ui: &mut Ui, stage: &Stage, from: [f32; 2], at: [f32; 2], released: bool) -> Option<[f32; 2]> {
        let at = stage.on_stage(stage.window(ui), at);
        stage.repin(ui, from, at, released)
    }

    fn assert_still(before: [f32; 4], after: [f32; 4]) {
        for n in 0..4 {
            assert!((after[n] - before[n]).abs() < 0.01, "moved to {after:?} from {before:?}");
        }
    }

    #[test]
    fn moving_the_pin_leaves_the_image_where_it_is() {
        let (mut ui, stage) = staged();
        let before = stage.rect(&ui);
        let from = [before[0], before[1]];
        // Wandering over the window and past its edges, a fraction of a point at a time
        for i in 0..400 {
            let i = i as f32;
            drag_pin(&mut ui, &stage, from, [i * 3.7 - 50.0, 900.0 - i * 2.3], false);
            assert_still(before, stage.rect(&ui));
        }
        assert!(ui.settings.background_target.is_some());
    }

    #[test]
    fn a_pin_let_go_past_the_window_goes_to_its_edge_with_its_point() {
        let (mut ui, stage) = staged();
        // Contained and zoomed in, the image reaches past the window's left edge, with more of it
        // the preview shows there
        ui.settings.image_fit = ImageFit::Contain;
        ui.settings.background_zoom = 1.2;
        ui.settings.background_anchor = [0.5, 0.5];
        ui.settings.background_target = Some([0.5, 0.5]);
        let before = stage.rect(&ui);
        let from = [before[0], before[1]];
        let past = [-40.0, 300.0];
        let out = drag_pin(&mut ui, &stage, from, past, false).expect("held past the edge");
        assert!(
            (out[0] - past[0]).abs() < 0.01 && (out[1] - past[1]).abs() < 0.01,
            "held at {out:?}"
        );
        assert_still(before, stage.rect(&ui));

        assert_eq!(drag_pin(&mut ui, &stage, from, past, true), None);
        let target = ui.settings.background_target.unwrap();
        assert_eq!(target[0], 0.0);
        let anchor = ui.settings.background_anchor;
        assert!((anchor[0] - (past[0] - from[0]) / before[2]).abs() < 1e-3);
        // The point the pin was on is at the window's left edge, as near as the image goes
        let after = stage.rect(&ui);
        assert!(after[0] > from[0]);
    }

    #[test]
    fn unpinning_leaves_the_image_where_it_is() {
        let (mut ui, stage) = staged();
        let before = stage.rect(&ui);
        drag_pin(&mut ui, &stage, [before[0], before[1]], [300.0, 200.0], true);
        stage.unpin(&mut ui);
        assert_eq!(ui.settings.background_target, None);
        assert_still(before, stage.rect(&ui));
    }

    #[test]
    fn a_size_tried_stays_in_the_middle() {
        let (mut ui, stage) = staged();
        let middle = stage.size.map(|v| v / 2.0);
        stage.resize(&mut ui, [middle[0] + 100.0, middle[1] + 50.0]);
        let window = stage.window(&ui);
        let origin = stage.origin(window);
        let corner = stage.on_stage(window, window);
        assert!((origin[0] + corner[0] - stage.size[0]).abs() < 0.01);
        assert!((origin[1] + corner[1] - stage.size[1]).abs() < 0.01);
        assert!((corner[0] - middle[0] - 100.0).abs() < 0.01);
    }
}
