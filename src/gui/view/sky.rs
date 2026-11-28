//! The chart behind the window's content: an astrolabe anchored to the bottom left, an armillary
//! sphere to the top right, orbits sweeping between them with constellations in the open sky, and
//! scales along the edges. Each piece is placed from the window's size, so the chart frames tall,
//! square and wide windows alike. Everything is drawn once into cached layers.

use rune::{Color, El, LineCap, Path, Shape, canvas, el};

use crate::gui::theme::Theme;

/// The faint accent inks the chart is drawn in
#[derive(Clone, Copy)]
struct Inks {
    strong: Color,
    mid: Color,
    faint: Color,
    wash: Color,
}

impl Inks {
    fn new(t: &Theme) -> Inks {
        let boost = if t.dark { 1.0 } else { 1.5 };
        let ink = |alpha: f32| t.accent.alpha(alpha * boost);
        Inks {
            strong: ink(0.12),
            mid: ink(0.08),
            faint: ink(0.05),
            wash: ink(0.03),
        }
    }
}

const BUTT: LineCap = LineCap::Butt;

fn at(x: f32, y: f32, radius: f32, deg: f32) -> [f32; 2] {
    let a = deg.to_radians();
    [x + radius * a.cos(), y + radius * a.sin()]
}

fn arc(x: f32, y: f32, radius: f32, from: f32, to: f32) -> Vec<[f32; 2]> {
    let steps = (((to - from).abs() / 360.0) * (radius * 0.6).clamp(48.0, 260.0)).ceil().max(4.0) as usize;
    (0..=steps)
        .map(|i| at(x, y, radius, from + (to - from) * i as f32 / steps as f32))
        .collect()
}

fn circle(x: f32, y: f32, radius: f32) -> Vec<[f32; 2]> {
    arc(x, y, radius, 0.0, 360.0)
}

/// An ellipse `rx` by `ry` around (`x`, `y`), turned by `turn` degrees
fn ellipse(x: f32, y: f32, rx: f32, ry: f32, turn: f32) -> Vec<[f32; 2]> {
    let (sin, cos) = turn.to_radians().sin_cos();
    (0..=180)
        .map(|i| {
            let a = (i as f32 * 2.0).to_radians();
            let (px, py) = (rx * a.cos(), ry * a.sin());
            [x + px * cos - py * sin, y + px * sin + py * cos]
        })
        .collect()
}

/// The parts of `points` that `keep` accepts, as separate runs
fn clipped(points: Vec<[f32; 2]>, keep: impl Fn([f32; 2]) -> bool) -> Vec<Vec<[f32; 2]>> {
    let mut runs: Vec<Vec<[f32; 2]>> = vec![Vec::new()];
    for p in points {
        if keep(p) {
            runs.last_mut().unwrap().push(p);
        } else if !runs.last().unwrap().is_empty() {
            runs.push(Vec::new());
        }
    }
    runs.into_iter().filter(|run| run.len() > 1).collect()
}

/// The chart for a window `w` by `h`
pub fn sky(w: f32, h: f32, t: &Theme) -> El {
    let inks = Inks::new(t);
    let long = w.max(h);

    // The astrolabe sits on the bottom left corner, the armillary on the top right
    let astrolabe_size = (long * 1.1).clamp(640.0, 1500.0);
    let astrolabe_at = [w * 0.1, h * 0.9];
    let armillary_size = (long * 0.5).clamp(280.0, 720.0);
    let armillary_at = [w * 0.95, h * 0.06];

    let placed = |piece: El, center: [f32; 2], size: f32| {
        el().absolute((center[0] - size / 2.0).round(), (center[1] - size / 2.0).round())
            .child(piece)
    };

    el().key("sky")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .inert()
        .a11y_hidden()
        .child(canvas(chart(w, h, astrolabe_at, armillary_at, inks)).size(w, h).cache())
        .child(placed(
            canvas(astrolabe(astrolabe_size, inks)).size(astrolabe_size, astrolabe_size).cache(),
            astrolabe_at,
            astrolabe_size,
        ))
        .child(placed(
            canvas(armillary(armillary_size, inks)).size(armillary_size, armillary_size).cache(),
            armillary_at,
            armillary_size,
        ))
}

/// Orbits from the astrolabe out across the window, constellations in the open sky between the
/// instruments, and graduated scales along the right and top edges
fn chart(w: f32, h: f32, astrolabe: [f32; 2], armillary: [f32; 2], inks: Inks) -> Shape {
    let mut shape = Shape::new();
    let in_window = |p: [f32; 2]| p[0] >= -2.0 && p[0] <= w + 2.0 && p[1] >= -2.0 && p[1] <= h + 2.0;

    // Orbits centered on the astrolabe, the outermost passing through the armillary
    let reach = (armillary[0] - astrolabe[0]).hypot(armillary[1] - astrolabe[1]);
    let toward = (armillary[1] - astrolabe[1]).atan2(armillary[0] - astrolabe[0]).to_degrees();
    for (k, (fraction, dashed, planet)) in [(0.58, false, 18.0), (0.79, true, -9.0), (1.0, false, 26.0)]
        .into_iter()
        .enumerate()
    {
        let radius = reach * fraction;
        let points = circle(astrolabe[0], astrolabe[1], radius);
        for run in clipped(points, in_window) {
            if dashed {
                for (i, pair) in run.windows(2).enumerate() {
                    if i % 3 != 2 {
                        shape = shape.line(pair[0], pair[1], 0.8, BUTT, inks.mid);
                    }
                }
            } else {
                shape = shape.stroke(run, if k == 2 { 1.0 } else { 0.8 }, inks.mid);
            }
        }
        // A planet on each orbit, with a ring round the outermost
        let p = at(astrolabe[0], astrolabe[1], radius, toward + planet);
        if in_window(p) {
            let size = 3.0 + k as f32 * 1.5;
            shape = shape
                .fill(circle(p[0], p[1], size), inks.strong)
                .stroke(circle(p[0], p[1], size + 4.0), 0.6, inks.mid);
            if k == 2 {
                shape = shape.stroke(ellipse(p[0], p[1], size * 3.2, size * 1.1, -20.0), 0.7, inks.mid);
            }
        }
    }

    // Constellations, placed by fractions of the window so they stay in its open middle
    let constellations: [&[(f32, f32)]; 3] = [
        &[(0.56, 0.3), (0.63, 0.24), (0.71, 0.27), (0.76, 0.2), (0.82, 0.29), (0.7, 0.36)],
        &[(0.82, 0.55), (0.88, 0.48), (0.93, 0.56), (0.9, 0.66), (0.84, 0.7)],
        &[(0.4, 0.62), (0.48, 0.56), (0.55, 0.64), (0.6, 0.58), (0.66, 0.68)],
    ];
    for stars in constellations {
        let points: Vec<[f32; 2]> = stars.iter().map(|(x, y)| [w * x, h * y]).collect();
        shape = shape.stroke(points.clone(), 0.6, inks.faint);
        for (i, p) in points.into_iter().enumerate() {
            let size = if i % 2 == 0 { 2.0 } else { 1.4 };
            shape = shape.fill(circle(p[0], p[1], size), inks.strong);
        }
    }

    // A scatter of faint stars over the whole window, the same every time
    let mut seed = 0x51a7_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 10_000) as f32 / 10_000.0
    };
    let count = ((w * h) / 4000.0).clamp(60.0, 400.0) as usize;
    for _ in 0..count {
        let p = [next() * w, next() * h];
        let size = 0.5 + next() * 0.9;
        shape = shape.fill(circle(p[0], p[1], size), inks.faint);
    }

    // Graduated scales down the right edge, below the armillary, and along the top, left of it
    let inset = 7.0;
    let right_from = (armillary[1] + 150.0).min(h * 0.4);
    shape = ticks(shape, right_from, h - inset, inks, |y, length| {
        ([w - inset, y], [w - inset - length, y])
    });
    shape = shape.line([w - inset, right_from], [w - inset, h - inset], 0.7, BUTT, inks.mid);
    let top_to = (armillary[0] - 150.0).max(w * 0.6);
    shape = ticks(shape, w * 0.3, top_to, inks, |x, length| ([x, inset], [x, inset + length]));
    if top_to > w * 0.3 {
        shape = shape.line([w * 0.3, inset], [top_to, inset], 0.7, BUTT, inks.mid);
    }
    shape
}

/// A graduated scale's ticks every 8 points from `from` up to `to`, longer every fifth and longest
/// and stronger every tenth. `tick` gives the ends of a tick `length` long at a point along it
fn ticks(mut shape: Shape, from: f32, to: f32, inks: Inks, tick: impl Fn(f32, f32) -> ([f32; 2], [f32; 2])) -> Shape {
    let mut at = from;
    let mut i = 0;
    while at < to {
        let length = if i % 10 == 0 {
            14.0
        } else if i % 5 == 0 {
            9.0
        } else {
            4.5
        };
        let (start, end) = tick(at, length);
        shape = shape.line(start, end, 0.6, BUTT, if i % 10 == 0 { inks.mid } else { inks.faint });
        at += 8.0;
        i += 1;
    }
    shape
}

/// An armillary sphere: graduated outer rings, the ecliptic and meridian bands seen at a tilt, the
/// axis through them and the earth at the center
fn armillary(size: f32, inks: Inks) -> Shape {
    let c = size / 2.0;
    let r = c - 2.0;
    let tilt = 23.4;
    let mut shape = Shape::new()
        .stroke(circle(c, c, r), 1.2, inks.strong)
        .stroke(circle(c, c, r - 6.0), 0.7, inks.mid)
        .stroke(circle(c, c, r - 20.0), 0.7, inks.mid);
    for i in 0..72 {
        let deg = i as f32 * 5.0;
        let length = if i % 6 == 0 { 14.0 } else { 7.0 };
        shape = shape.line(at(c, c, r - 6.0, deg), at(c, c, r - 6.0 - length, deg), 0.6, BUTT, inks.faint);
    }
    // The bands, each a pair of ellipses
    for (rx, ry, turn, color) in [
        (0.78, 0.3, tilt, inks.mid),
        (0.3, 0.78, tilt, inks.mid),
        (0.78, 0.18, -tilt * 0.6, inks.faint),
    ] {
        shape = shape.stroke(ellipse(c, c, r * rx, r * ry, turn), 0.9, color).stroke(
            ellipse(c, c, r * rx - 6.0, (r * ry - 6.0).max(2.0), turn),
            0.6,
            color,
        );
    }
    // Ticks along the ecliptic band
    let (sin, cos) = tilt.to_radians().sin_cos();
    for i in 0..48 {
        let a = (i as f32 * 7.5).to_radians();
        let (ox, oy) = (r * 0.78 * a.cos(), r * 0.3 * a.sin());
        let (ix, iy) = ((r * 0.78 - 6.0) * a.cos(), (r * 0.3 - 6.0) * a.sin());
        let rotate = |x: f32, y: f32| [c + x * cos - y * sin, c + x * sin + y * cos];
        shape = shape.line(rotate(ox, oy), rotate(ix, iy), 0.5, BUTT, inks.faint);
    }
    // The axis, poking out past the rings, with a knob at each end
    let axis = tilt + 90.0;
    let (north, south) = (at(c, c, r + 14.0, axis), at(c, c, r + 14.0, axis + 180.0));
    shape = shape
        .line(north, south, 1.0, LineCap::Round, inks.mid)
        .fill(circle(north[0], north[1], 3.5), inks.strong)
        .fill(circle(south[0], south[1], 3.5), inks.strong);
    // The earth, with its equator and a meridian
    let earth = r * 0.13;
    shape
        .fill(circle(c, c, earth), inks.wash)
        .stroke(circle(c, c, earth), 1.0, inks.strong)
        .stroke(ellipse(c, c, earth, earth * 0.35, tilt), 0.6, inks.mid)
        .stroke(ellipse(c, c, earth * 0.35, earth, tilt), 0.6, inks.mid)
}

/// An astrolabe: the limb with its degree and hour scales, the plate's tropics, almucantars and
/// azimuths, the rete with its ecliptic, arms and star pointers, and the rule across it all
fn astrolabe(size: f32, inks: Inks) -> Shape {
    let c = size / 2.0;
    let r = c - 2.0;
    let Inks { strong, mid, faint, wash } = inks;
    let mut shape = Shape::new();

    // The limb: a double rim, a degree scale and the hours
    let rim = r - 5.0;
    let scale_in = rim - 16.0;
    let hours_in = scale_in - 18.0;
    shape = shape
        .stroke(circle(c, c, r), 1.3, strong)
        .stroke(circle(c, c, rim), 0.7, mid)
        .stroke(circle(c, c, scale_in), 0.7, mid)
        .stroke(circle(c, c, hours_in), 0.7, mid);
    for i in 0..360 {
        let deg = i as f32;
        let (length, width, color) = match i {
            _ if i % 10 == 0 => (14.0, 0.9, mid),
            _ if i % 5 == 0 => (9.0, 0.7, faint),
            _ => (4.5, 0.6, faint),
        };
        shape = shape.line(at(c, c, rim, deg), at(c, c, rim - length, deg), width, BUTT, color);
    }
    for i in 0..24 {
        let from = i as f32 * 15.0;
        shape = shape.line(at(c, c, hours_in, from), at(c, c, scale_in, from), 0.6, BUTT, faint);
        if i % 2 == 0 {
            let mut wedge = arc(c, c, scale_in, from, from + 15.0);
            wedge.extend(arc(c, c, hours_in, from + 15.0, from));
            shape = shape.fill(wedge, wash);
        }
    }
    // Quarter marks on the rim
    for i in 0..4 {
        let deg = i as f32 * 90.0;
        let tip = at(c, c, r + 1.0, deg);
        let left = at(c, c, rim - 2.0, deg - 1.6);
        let right = at(c, c, rim - 2.0, deg + 1.6);
        shape = shape.fill(vec![tip, left, right], strong);
    }

    // The plate: tropics and equator, then the almucantars and azimuths, which stop at the
    // horizon and the outer tropic as they do on a real plate
    let capricorn = r * 0.66;
    for (k, width) in [(0.66, 1.0), (0.48, 0.8), (0.33, 0.8)] {
        shape = shape.stroke(circle(c, c, r * k), width, mid);
    }
    let horizon = ([c, c + r * 0.12], r * 0.58);
    let zenith = [c, c - r * 0.2];
    let inside = |p: [f32; 2], center: [f32; 2], radius: f32| (p[0] - center[0]).hypot(p[1] - center[1]) <= radius;
    let on_plate = |p: [f32; 2]| inside(p, [c, c], capricorn);
    let above_horizon = |p: [f32; 2]| inside(p, horizon.0, horizon.1 + 0.5) && on_plate(p);
    shape = shape.stroke(circle(horizon.0[0], horizon.0[1], horizon.1), 0.9, mid);
    for k in 1..10 {
        let f = k as f32 / 10.0;
        let y = horizon.0[1] + (zenith[1] - horizon.0[1]) * f.powf(0.85);
        let radius = horizon.1 * (1.0 - f) + 3.0 * f;
        for run in clipped(circle(c, y, radius), on_plate) {
            shape = shape.stroke(run, 0.6, faint);
        }
    }
    let nadir_y = c + r * 1.7;
    let mid_y = (zenith[1] + nadir_y) / 2.0;
    for k in 1..=5 {
        let offset = r * 0.16 * k as f32;
        for side in [-1.0, 1.0] {
            let center = [c + side * offset, mid_y];
            let radius = (center[0] - zenith[0]).hypot(center[1] - zenith[1]);
            for run in clipped(circle(center[0], center[1], radius), above_horizon) {
                shape = shape.stroke(run, 0.55, faint);
            }
        }
    }
    shape =
        shape
            .line([c - capricorn, c], [c + capricorn, c], 0.6, BUTT, faint)
            .line([c, c - capricorn], [c, c + capricorn], 0.6, BUTT, faint);

    // The rete: the ecliptic ring set off center, its arms, and pointers to the bright stars
    let (ex, ey, er) = (c, c - r * 0.17, r * 0.5);
    shape = shape
        .stroke(circle(ex, ey, er), 1.1, strong)
        .stroke(circle(ex, ey, er - 9.0), 0.7, mid);
    for i in 0..72 {
        let deg = i as f32 * 5.0;
        let length = if i % 6 == 0 { 9.0 } else { 4.0 };
        shape = shape.line(at(ex, ey, er, deg), at(ex, ey, er - length, deg), 0.6, BUTT, mid);
    }
    for i in 0..4 {
        let deg = 45.0 + i as f32 * 90.0;
        let start = at(c, c, 14.0, deg);
        let end = at(c, c, capricorn, deg + 18.0);
        let bend = at(c, c, capricorn * 0.55, deg - 22.0);
        let path = Path::new().move_to(start).quad_to(bend, end);
        shape = shape.stroke_path(&path, 1.0, LineCap::Round, mid);
    }
    let stars = [
        (0.62, 200.0),
        (0.55, 250.0),
        (0.6, 320.0),
        (0.5, 20.0),
        (0.58, 75.0),
        (0.4, 130.0),
        (0.3, 290.0),
        (0.45, 165.0),
        (0.36, 345.0),
        (0.24, 60.0),
    ];
    for (reach, deg) in stars {
        let tip = at(c, c, capricorn * reach, deg);
        let base_left = at(c, c, capricorn * reach - 11.0, deg - 4.0);
        let base_right = at(c, c, capricorn * reach - 11.0, deg + 4.0);
        shape = shape
            .fill(vec![base_left, tip, base_right], mid)
            .fill(circle(tip[0], tip[1], 1.6), strong);
    }

    // The rule across the face, with a scale along it, and the hub
    let turn = -28.0_f32.to_radians();
    let (dx, dy) = (turn.cos(), turn.sin());
    let (nx, ny) = (-dy, dx);
    let half = r * 0.97;
    let width = 7.0;
    let corner = |along: f32, across: f32| [c + dx * along + nx * across, c + dy * along + ny * across];
    shape = shape
        .fill(
            vec![
                corner(-half, -width),
                corner(half, -width),
                corner(half, width),
                corner(-half, width),
            ],
            wash,
        )
        .line(corner(-half, -width), corner(half, -width), 0.7, BUTT, mid)
        .line(corner(-half, width), corner(half, width), 0.7, BUTT, mid);
    let mut along = -half + 8.0;
    let mut i = 0;
    while along < half - 8.0 {
        if along.abs() > 18.0 {
            let length = if i % 5 == 0 { width * 1.4 } else { width * 0.7 };
            shape = shape.line(corner(along, -width), corner(along, -width + length), 0.5, BUTT, faint);
        }
        along += 6.0;
        i += 1;
    }
    shape
        .stroke(circle(c, c, 14.0), 1.0, strong)
        .stroke(circle(c, c, 9.0), 0.7, mid)
        .fill(circle(c, c, 3.0), strong)
}
