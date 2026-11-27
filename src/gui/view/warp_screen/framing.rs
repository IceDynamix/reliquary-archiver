//! Where the banner's art goes in the window: the scale and offset that take the game's reference
//! screen, where the art is placed, into the room the warp screen leaves it.

use super::{ART_TOP, Fit, NARROW_GAP, PAD, REFERENCE, narrow_band, narrow_rail_foot};
use crate::gui::art::{LayerArt, Weight};

/// The share of the art's room a character takes, the same for every character so each carries
/// the same weight however much art reaches out from them: their person's box, as detected, or
/// without one the core of their opacity
const FIGURE_AREA: f32 = 0.3;
const CORE_AREA: f32 = 0.34;
/// The share of the room their head takes, as the person's box takes `FIGURE_AREA` for the
/// typical character. The head only ever draws them larger than their box does, when a sprawled
/// pose widens it: a big head is a close crop, which the box already sizes well
const HEAD_AREA: f32 = 0.0104;
/// How far past the game's own scale a character with a small core may be drawn
const MAX_ZOOM: f32 = 1.5;

/// A banner's main layer, the character, as the game places it
#[derive(Clone, Copy, Debug)]
pub(super) struct MainLayer {
    /// The canvas the layers are cut from
    pub canvas: [f32; 2],
    /// The layer's piece of it: x, y, width, height from the top left
    pub rect: [f32; 4],
    /// Where the game draws the canvas on its reference screen
    pub on_screen: [f32; 4],
    pub weight: Option<Weight>,
}

impl MainLayer {
    /// The layer, when the game places it on its screen
    pub fn of(layer: &LayerArt) -> Option<MainLayer> {
        Some(MainLayer {
            canvas: layer.canvas,
            rect: layer.rect,
            on_screen: layer.on_screen?,
            weight: layer.weight,
        })
    }
}

/// How the banner's art is drawn in a window `window` large, laid out as `fit`: `(k, ox, oy)`,
/// which take a point `[x, y]` on the game's reference screen to `[ox + x * k, oy + y * k]`.
///
/// Side by side, the character stands in the middle of the room right of the card. Where they've
/// been measured, they're sized by themselves: their person's box, or without one the core of their
/// opacity (the body without the props and effects reaching out from it), takes the same share of
/// the room for everyone, or more when their head says a sprawled pose widened the box. The box
/// always fits in the room; whatever reaches out past the room falls off it. Unmeasured, the game's
/// screen is drawn at the scale that covers the window, as in game, or smaller if the layer would
/// then overflow the room.
///
/// Narrow, the character is fitted into the band under the tickets.
pub(super) fn art_transform(main: &MainLayer, window: [f32; 2], fit: Fit) -> (f32, f32, f32) {
    let [width, height] = window;
    let MainLayer {
        canvas,
        rect,
        on_screen,
        weight,
    } = *main;
    let scale = on_screen[2] / canvas[0];
    let shown = [
        on_screen[0] + rect[0] * scale,
        on_screen[1] + rect[1] * scale,
        rect[2] * scale,
        rect[3] * scale,
    ];
    if fit == Fit::Narrow {
        let band = narrow_band(height);
        let top = narrow_rail_foot() + NARROW_GAP;
        let k = (band * 1.15 / shown[3]).min(width * 1.05 / shown[2]);
        return (k, width / 2.0 - (shown[0] + shown[2] / 2.0) * k, top - shown[1] * k);
    }

    let room = fit.stage_from(width);
    let (room_w, room_h) = ((width - room - PAD).max(1.0), (height - ART_TOP - PAD).max(1.0));
    let cover = (width / REFERENCE[0]).max(height / REFERENCE[1]);
    let center = [(room + width - PAD) / 2.0, ART_TOP + room_h / 2.0];
    let Some(weight) = weight else {
        let k = cover.min(room_w / shown[2]).min(room_h / shown[3]);
        return (
            k,
            center[0] - (shown[0] + shown[2] / 2.0) * k,
            center[1] - (shown[1] + shown[3] / 2.0) * k,
        );
    };

    // A point on the canvas, on the reference screen
    let at = |x: f32, y: f32| [on_screen[0] + x * scale, on_screen[1] + y * on_screen[3] / canvas[1]];
    let (figure, area) = match weight.person {
        Some(person) => (person, FIGURE_AREA),
        None => (weight.core, CORE_AREA),
    };
    let [x0, y0] = at(figure[0], figure[1]);
    let [x1, y1] = at(figure[0] + figure[2], figure[1] + figure[3]);
    // Centered on their person's box, or else across on the center of their opacity, and up and
    // down on the core's middle
    let middle = match weight.person {
        Some(_) => (x0 + x1) / 2.0,
        None => at(weight.center[0], weight.center[1])[0],
    };
    let (figure_w, figure_h) = (x1 - x0, y1 - y0);
    let by_figure = (area * room_w * room_h / (figure_w * figure_h)).sqrt();
    let by_head = weight.person.and(weight.head).map(|head| {
        let (head_w, head_h) = (head[2] * scale, head[3] * on_screen[3] / canvas[1]);
        (HEAD_AREA * room_w * room_h / (head_w * head_h)).sqrt()
    });
    let k = by_head
        .unwrap_or(by_figure)
        .max(by_figure)
        .min(room_w / figure_w)
        .min(room_h / figure_h)
        .min(cover * MAX_ZOOM);
    (k, center[0] - middle * k, center[1] - (y0 + y1) / 2.0 * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1000 × 2000 canvas drawn 600 wide at the reference screen's right, its layer the whole of
    /// it
    fn main(weight: Option<Weight>) -> MainLayer {
        MainLayer {
            canvas: [1000.0, 2000.0],
            rect: [0.0, 0.0, 1000.0, 2000.0],
            on_screen: [1200.0, -100.0, 600.0, 1200.0],
            weight,
        }
    }

    fn apply((k, ox, oy): (f32, f32, f32), [x, y]: [f32; 2]) -> [f32; 2] {
        [ox + x * k, oy + y * k]
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn narrow_the_character_is_centered_under_the_tickets() {
        let window = [540.0, 800.0];
        let fit = art_transform(&main(None), window, Fit::Narrow);
        let [left, top] = apply(fit, [1200.0, -100.0]);
        let [right, _] = apply(fit, [1800.0, -100.0]);
        assert!(near((left + right) / 2.0, 230.0));
        assert!(near(top, narrow_rail_foot() + NARROW_GAP));
    }

    #[test]
    fn unmeasured_the_layer_fits_the_room_right_of_the_card() {
        let window = [1600.0, 900.0];
        let wide = Fit::of(window[0]);
        let transform = art_transform(&main(None), window, wide);
        let [left, top] = apply(transform, [1200.0, -100.0]);
        let [right, bottom] = apply(transform, [1800.0, 1100.0]);
        let room = wide.stage_from(window[0]);
        assert!(left >= room - 0.01 && right <= window[0] - PAD + 0.01, "{left}..{right}");
        assert!(top >= ART_TOP - 0.01 && bottom <= window[1] - PAD + 0.01, "{top}..{bottom}");
        assert!(near((left + right) / 2.0, (room + window[0] - PAD) / 2.0));
    }

    #[test]
    fn a_person_takes_the_same_share_of_the_room_and_stands_in_its_middle() {
        let window = [1600.0, 900.0];
        let wide = Fit::of(window[0]);
        let person = [300.0, 400.0, 400.0, 1200.0];
        let weight = Weight {
            center: [500.0, 1000.0],
            core: person,
            person: Some(person),
            head: None,
        };
        let transform = art_transform(&main(Some(weight)), window, wide);
        // The person's box on the reference screen: 0.6 to the canvas point
        let [x0, y0] = apply(transform, [1200.0 + 300.0 * 0.6, -100.0 + 400.0 * 0.6]);
        let [x1, y1] = apply(transform, [1200.0 + 700.0 * 0.6, -100.0 + 1600.0 * 0.6]);
        let room = wide.stage_from(window[0]);
        let (room_w, room_h) = (window[0] - room - PAD, window[1] - ART_TOP - PAD);
        assert!(near((x0 + x1) / 2.0, (room + window[0] - PAD) / 2.0));
        assert!(near((y0 + y1) / 2.0, ART_TOP + room_h / 2.0));
        let share = (x1 - x0) * (y1 - y0) / (room_w * room_h);
        assert!(share <= FIGURE_AREA + 1e-3, "takes {share}");
        assert!(y1 - y0 <= room_h + 0.01);
    }
}
