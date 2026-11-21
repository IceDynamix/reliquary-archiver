//! The warp screen. It takes the game's warp screen as its visual language rather than its layout:
//! the banner's art is the place, not a picture on it, its colour is the screen's accent, and one
//! number carries the screen. What's on it, in the order it matters:
//!
//! 1. The pity, how near soft pity it is, and whether the next 5 star is a 50/50 or guaranteed.
//! 2. What the warps in hand buy: the chance of a 5 star, and of the featured one, before they run
//!    out, and the most it can take.
//! 3. How far they go towards copies of what's featured, from what the account owns.
//! 4. Every 5 star as a bar of its pity, coloured by its 50/50.
//! 5. What was pulled this session, while the game's history catches up.
//!
//! The rest (sync, authkeys, imports, the full list) is in the history sheet behind "History".
//!
//! The art comes in as the game's does: the banner's layers slide in from the left at their own
//! speeds and settle, the number and the rest follow a beat behind.

use rune::{Color, Cx, Edge, El, Spring, SpringPair, Stagger, StaggerFrom, SurfaceFit, TextStyle, Transition, col, el, icon, row, text};

use super::parts::ring_button;
use crate::gui::art::{Art, BannerArt, Layer};
use crate::gui::icons;
use crate::gui::state::{Funds, Panel, Roster, Ui};
use crate::gui::theme::{self, Theme};
use crate::warps::odds::{self, Rates};
use crate::warps::service::{RunningBanner, Summary};
use crate::warps::stats::{Highlight, Outcome, PoolStats};
use crate::warps::{Pool, source};

/// Narrower than this, the screen stacks
const WIDE: f32 = 760.0;
/// The game lays its warp screen out on this
const REFERENCE: [f32; 2] = [1920.0, 1080.0];
/// How far left the game starts each layer, in reference units, back to front
const LAYER_TRAVEL: [f32; 3] = [77.0, 93.0, 104.0];

const PITY: TextStyle = TextStyle::new(96.0, 650.0).lh(92.0).tracking(-1.5).features("'tnum' 1");
const FIGURE: TextStyle = TextStyle::new(26.0, 600.0).lh(30.0).features("'tnum' 1");
const NAME: TextStyle = TextStyle::new(30.0, 650.0).lh(34.0).tracking(0.2);
const TAB: TextStyle = TextStyle::new(14.5, 600.0).lh(18.0).tracking(0.3);
const COUNT: TextStyle = TextStyle::new(13.0, 600.0).lh(16.0).features("'tnum' 1");

/// The art settles slowly, like the game's 3.5 s slide; things leave quickly
fn art_springs() -> SpringPair {
    SpringPair::new(Spring::smooth(1.7, 0.0), Spring::smooth(0.25, 0.0))
}
fn rise_springs() -> SpringPair {
    SpringPair::new(Spring::smooth(0.55, 0.0), Spring::smooth(0.2, 0.0))
}
/// Rises a little and fades in, `delay` seconds after it mounts
fn rise(el: El, delay: f32) -> El {
    el.transition(
        Transition::offset(0.0, 14.0)
            .combined(Transition::opacity())
            .springs(rise_springs()),
    )
    .enter_delay(delay)
}

/// Each warp's own colour, for when its art has none to give
fn pool_color(pool: Pool) -> Color {
    match pool {
        Pool::Character => rune::oklch(0.68, 0.16, 22.0),
        Pool::LightCone => rune::oklch(0.7, 0.12, 250.0),
        Pool::Standard => rune::oklch(0.74, 0.11, 80.0),
        Pool::CollabCharacter | Pool::CollabLightCone => rune::oklch(0.68, 0.14, 310.0),
        Pool::Departure => rune::oklch(0.72, 0.11, 160.0),
    }
}

fn label(pool: Pool) -> &'static str {
    match pool {
        Pool::Character => "Character",
        Pool::LightCone => "Light cone",
        Pool::Standard => "Standard",
        Pool::CollabCharacter => "Collab",
        Pool::CollabLightCone => "Collab cone",
        Pool::Departure => "Departure",
    }
}

fn outcome_color(outcome: Outcome) -> Color {
    match outcome {
        Outcome::Won => rune::oklch(0.74, 0.12, 185.0),
        Outcome::Guaranteed => rune::oklch(0.8, 0.13, 85.0),
        Outcome::Lost => rune::oklch(0.64, 0.19, 25.0),
        Outcome::Unknown | Outcome::None => rune::oklch(0.72, 0.03, 260.0),
    }
}

fn outcome_words(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Won => "won the 50/50",
        Outcome::Guaranteed => "guaranteed",
        Outcome::Lost => "lost the 50/50",
        Outcome::Unknown | Outcome::None => "",
    }
}

/// Everything the screen shows of one pool
struct View<'a> {
    summary: &'a Summary,
    stats: &'a PoolStats,
    pool: Pool,
    /// The featured 5 star picked, and the banner it's on, when a banner with one is running
    featured: Option<(u32, &'a RunningBanner)>,
    /// What the art shows: the featured 5 star, or the last one pulled
    hero: Option<u32>,
    accent: Color,
    /// Warps the account can make on this pool, when it logged in this run
    budget: Option<Budget>,
    roster: Option<&'a Roster>,
    art: Option<BannerArt>,
}

#[derive(Clone, Copy)]
struct Budget {
    warps: u32,
    passes: u32,
    jade: u32,
    special: bool,
}

impl Budget {
    fn of(funds: Funds, pool: Pool) -> Budget {
        let special = pool != Pool::Standard;
        let passes = if special { funds.special_passes } else { funds.passes };
        Budget {
            warps: odds::warps_in_hand(funds.stellar_jade, passes),
            passes,
            jade: funds.stellar_jade,
            special,
        }
    }
}

/// The whole window, as the warp screen
pub fn screen(s: &Ui, cx: &Cx<Ui>, t: &Theme) -> El {
    let [width, height] = cx.layout_size("page").unwrap_or([460.0, 700.0]);
    let wide = width >= WIDE;

    let Some(summary) = s.warps_summary() else {
        return el().key("warp-screen").w_full().h_full().fill(t.bg).padding(22.0).child(
            col()
                .w_full()
                .gap(18.0)
                .child(header(s, None, None, Color::WHITE, t))
                .child(text("Reading your warps…").font(theme::BODY).color(t.muted)),
        );
    };
    let pools: Vec<Pool> = summary.pools.iter().map(|p| p.pool).collect();
    let pool = if pools.contains(&s.warps.pool) {
        s.warps.pool
    } else {
        Pool::Character
    };
    let Some(stats) = summary.pool(pool) else {
        return el().key("warp-screen").w_full().h_full().fill(t.bg);
    };

    let running: Vec<&RunningBanner> = summary.running(pool).collect();
    let picked = s.warps.featured.get(&pool).copied();
    let featured = picked
        .and_then(|item| running.iter().find(|b| b.fives.contains(&item)).map(|b| (item, *b)))
        .or_else(|| running.first().and_then(|b| b.fives.first().map(|&item| (item, *b))));
    let hero = featured.map(|(item, _)| item).or_else(|| stats.fives.first().map(|f| f.item_id));
    let art = hero.and_then(|item| s.art.as_ref()?.banner(item));
    let accent = hero.and_then(|item| s.art.as_ref()?.tint(item)).unwrap_or_else(|| pool_color(pool));
    let view = View {
        summary,
        stats,
        pool,
        featured,
        hero,
        accent,
        budget: s.warp_funds().map(|funds| Budget::of(funds, pool)),
        roster: s.warp_roster(),
        art,
    };

    let mut page = el().key("warp-screen").w_full().h_full().fill(t.bg).clip_rounded(0.0);
    page = page.child(backdrop(s, &view, t));
    page = page.child(stage(s, &view, width, height, wide, t));
    page = page.child(scrims(width, height, wide, t));

    let content = if wide {
        let left = col()
            .key("warp-left")
            .w((width * 0.42).clamp(400.0, 520.0))
            .gap(22.0)
            .child(banner_line(s, &view, &running, t))
            .child(pity_block(&view, t))
            .child(odds_row(&view, t))
            .children(targets(&view, t));
        let middle = row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .items_end()
            .child(left)
            .child(el().grow(1.0))
            .children(hero.map(|item| name_tags(&view, item, t)));
        col()
            .key("warp-content")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .px(34.0)
            .pt(22.0)
            .pb(24.0)
            .gap(22.0)
            .child(header(s, Some(summary), Some(pool), accent, t))
            .child(middle)
            .child(history_strip(&view, width - 68.0, t))
            .children(session_row(s, &view, t))
    } else {
        let band = (height * 0.42).clamp(220.0, 420.0);
        let column = col()
            .key("warp-content-narrow")
            .w_full()
            .px(18.0)
            .pt(16.0)
            .pb(20.0)
            .gap(20.0)
            .child(header(s, Some(summary), Some(pool), accent, t))
            // Room for the art, its name over its foot
            .child(
                col()
                    .w_full()
                    .h(band - 40.0)
                    .justify_end()
                    .items_end()
                    .children(hero.map(|item| name_tags(&view, item, t))),
            )
            .child(banner_line(s, &view, &running, t))
            .child(pity_block(&view, t))
            .child(odds_row(&view, t))
            .children(targets(&view, t))
            .child(history_strip(&view, width - 36.0, t))
            .children(session_row(s, &view, t));
        col()
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(column.scroll(rune::ScrollSpec { stick_bottom: false }))
    };
    page.child(content)
}

// ---- the place ---------------------------------------------------------------------------------

/// The banner's blurred backdrop over the whole window, dimmed so text reads on it.
///
/// The slot and the dimming over it are always there, and come and go with nothing: only the
/// picture in the slot fades. Every element fades in when it mounts, so dimming that mounted with
/// its picture would start clear and show the picture undimmed for a moment
fn backdrop(s: &Ui, view: &View, t: &Theme) -> El {
    let full = || el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert();
    let mut slot = full().key("backdrop-slot").transition(Transition::none());
    if let Some(item) = view.hero {
        let picture = view
            .art
            .as_ref()
            .and_then(|art| art.layer(Layer::Backdrop))
            .and_then(|_| s.art.as_ref()?.get(Art::Layer(item, Layer::Backdrop)));
        slot = slot.child(match picture {
            Some(image) => full()
                .key(format!("backdrop-{item}"))
                .pixels_fit(image.pixels(), SurfaceFit::Fill)
                .transition(Transition::opacity().springs(art_springs())),
            // Without the banner's own, a wash of its colour
            None => full()
                .key(format!("backdrop-wash-{item}"))
                .radial_gradient([0.78, 0.35], 0.75, [(0.0, view.accent.alpha(0.4)), (1.0, view.accent.alpha(0.0))])
                .transition(Transition::opacity().springs(art_springs())),
        });
    }
    full().key("backdrop").transition(Transition::none()).child(slot).child(
        full()
            .key("backdrop-dim")
            .transition(Transition::none())
            .fill(t.bg.alpha(if t.dark { 0.45 } else { 0.3 })),
    )
}

/// The banner's art: its layers placed as the game places them, sliding in at their own speeds.
/// Without them, the hero's portrait on the right
fn stage(s: &Ui, view: &View, width: f32, height: f32, wide: bool, t: &Theme) -> El {
    let Some(item) = view.hero else { return el() };
    let mut stage = el().key("warp-stage").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert();

    if let Some(art) = &view.art
        && let Some(main) = art.layer(Layer::Main)
        && let Some(main_on_screen) = main.on_screen
    {
        // Wide: the game's screen scaled to cover the window, as it is in game. Narrow: the
        // character fitted into the band at the top
        let (k, ox, oy) = if wide {
            let k = (width / REFERENCE[0]).max(height / REFERENCE[1]);
            (
                k,
                (width - REFERENCE[0] * k) / 2.0 + width * 0.03,
                (height - REFERENCE[1] * k) / 2.0,
            )
        } else {
            let band = (height * 0.42).clamp(220.0, 420.0);
            let scale = main_on_screen[2] / main.canvas[0];
            let shown = [
                main_on_screen[0] + main.rect[0] * scale,
                main_on_screen[1] + main.rect[1] * scale,
                main.rect[2] * scale,
                main.rect[3] * scale,
            ];
            let k = (band * 1.1 / shown[3]).min(width * 1.05 / shown[2]);
            let ox = width / 2.0 - (shown[0] + shown[2] / 2.0) * k;
            let oy = band * 0.04 - shown[1] * k;
            (k, ox, oy)
        };
        let order = [Layer::Back, Layer::Main, Layer::Front];
        for (i, layer) in order.into_iter().enumerate() {
            let Some(placed) = art.layer(layer) else { continue };
            let Some(on_screen) = placed.on_screen else { continue };
            let Some(image) = s.art.as_ref().and_then(|a| a.get(Art::Layer(item, layer))) else {
                continue;
            };
            let scale = on_screen[2] / placed.canvas[0] * k;
            let (x, y) = (
                ox + (on_screen[0] + placed.rect[0] * on_screen[2] / placed.canvas[0]) * k,
                oy + (on_screen[1] + placed.rect[1] * on_screen[3] / placed.canvas[1]) * k,
            );
            let travel = LAYER_TRAVEL[i] * k * 1.6;
            stage = stage.child(
                el().key(format!("layer-{item}-{layer:?}"))
                    .pin(Some(x), Some(y), None, None)
                    .size(placed.rect[2] * scale, placed.rect[3] * scale)
                    .pixels_fit(image.pixels(), SurfaceFit::Fill)
                    .transition(
                        Transition::offset(-travel, 0.0)
                            .combined(Transition::opacity())
                            .springs(art_springs()),
                    )
                    .enter_delay(i as f32 * 0.06),
            );
        }
        return stage;
    }

    // The portrait, over the right of the window or behind the top of a narrow one
    let Some(image) = s.art.as_ref().and_then(|art| art.get(Art::Portrait(item))) else {
        return stage;
    };
    // Its edges fade into the window, so it reads as part of the place rather than a picture on it
    let ground = t.bg;
    let fade = |angle: f32| {
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .linear_gradient(angle, [(0.0, ground), (0.28, ground.alpha(0.0))])
    };
    let picture = el()
        .key(format!("portrait-{item}"))
        .pixels_fit(image.pixels(), SurfaceFit::Fill)
        .child(fade(std::f32::consts::FRAC_PI_2))
        .child(fade(-std::f32::consts::FRAC_PI_2))
        .child(fade(0.0))
        .child(fade(std::f32::consts::PI))
        .transition(
            Transition::offset(-90.0, 0.0)
                .combined(Transition::opacity())
                .springs(art_springs()),
        );
    if wide {
        let side = (height * 1.05).min(width * 0.66);
        stage.child(picture.pin(None, Some((height - side) / 2.0), Some(0.0), None).size(side, side))
    } else {
        let side = (height * 0.42).clamp(220.0, 420.0) * 1.15;
        stage.child(
            picture
                .pin(Some((width - side) / 2.0), Some(-side * 0.06), None, None)
                .size(side, side),
        )
    }
}

/// Shade where the words go: down the left of a wide window, under the band of a narrow one, and
/// along the foot for the history
fn scrims(width: f32, height: f32, wide: bool, t: &Theme) -> El {
    let ground = t.bg;
    let mut layer = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert();
    if wide {
        layer = layer
            .child(
                el().pin(Some(0.0), Some(0.0), None, Some(0.0))
                    .w(width * 0.62)
                    .linear_gradient(std::f32::consts::FRAC_PI_2, [
                        (0.0, ground.alpha(0.94)),
                        (0.55, ground.alpha(0.72)),
                        (1.0, ground.alpha(0.0)),
                    ]),
            )
            .child(
                el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                    .h((height * 0.3).max(150.0))
                    .gradient(ground.alpha(0.0), ground.alpha(0.92)),
            )
            .child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), None)
                    .h(90.0)
                    .gradient(ground.alpha(0.7), ground.alpha(0.0)),
            );
    } else {
        let band = (height * 0.42).clamp(220.0, 420.0);
        layer = layer
            .child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), None)
                    .h(80.0)
                    .gradient(ground.alpha(0.75), ground.alpha(0.0)),
            )
            .child(
                el().pin(Some(0.0), Some(band * 0.55), Some(0.0), None)
                    .h(band * 0.5)
                    .gradient(ground.alpha(0.0), ground),
            )
            .child(el().pin(Some(0.0), Some(band * 1.05), Some(0.0), Some(0.0)).fill(ground));
    }
    layer
}

// ---- the words ---------------------------------------------------------------------------------

/// The warps as tabs, what's in hand, where syncing stands, the history, and the way back
fn header(s: &Ui, summary: Option<&Summary>, selected: Option<Pool>, accent: Color, t: &Theme) -> El {
    let mut bar = row().key("warp-header").w_full().gap(6.0).center_y().flex_wrap();
    bar = bar.child(icon(&icons::WARP, 22.0).color(accent)).child(el().w(6.0));
    if let Some(summary) = summary {
        for stats in &summary.pools {
            let pool = stats.pool;
            let chosen = Some(pool) == selected;
            let minor = matches!(pool, Pool::Departure | Pool::CollabCharacter | Pool::CollabLightCone);
            bar = bar.child(
                col()
                    .key(format!("tab-{}", pool.gacha_type()))
                    .role("tab")
                    .label(label(pool))
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .px(10.0)
                    .pt(6.0)
                    .gap(5.0)
                    .radius(6.0)
                    .hover(t.text.alpha(0.06))
                    .on_press(move |s: &mut Ui| s.warps.pool = pool)
                    .child(
                        row()
                            .gap(6.0)
                            .items_end()
                            .child(text(label(pool)).font(if minor { theme::LABEL } else { TAB }).color(if chosen {
                                t.text
                            } else {
                                t.muted
                            }))
                            .child(
                                text(stats.pity5.to_string())
                                    .font(COUNT)
                                    .color(if chosen { accent } else { t.faint }),
                            ),
                    )
                    .child(
                        el().h(2.0)
                            .w_full()
                            .radius(1.0)
                            .fill(if chosen { accent } else { Color::TRANSPARENT }),
                    ),
            );
        }
    }
    bar = bar.child(el().grow(1.0));
    if let (Some(pool), Some(funds)) = (selected, s.warp_funds()) {
        bar = bar.child(budget_chip(s, Budget::of(funds, pool), t));
    }
    bar.child(status_chip(s, t))
        .child(ghost_button("warp-history", "History", &icons::LIST, t, |s: &mut Ui| {
            s.panel = Some(Panel::Warps)
        }))
        .child(ring_button("close-warps", &icons::CLOSE, "Back", false, t, |s| s.close_warps()))
}

/// Warps in hand, broken down on hover
fn budget_chip(s: &Ui, budget: Budget, t: &Theme) -> El {
    let pass = if budget.special { 102 } else { 101 };
    let glyph = match s.art.as_ref().and_then(|art| art.get(Art::Item(pass))) {
        Some(image) => el().size(22.0, 22.0).pixels_fit(image.pixels(), SurfaceFit::Fit),
        None => el().size(6.0, 6.0),
    };
    row()
        .key("warp-budget")
        .h(32.0)
        .pl(6.0)
        .pr(12.0)
        .gap(6.0)
        .center_y()
        .radius(16.0)
        .fill(t.text.alpha(0.07))
        .backdrop_blur(10.0)
        .help(format!(
            "{} {} and {} Stellar Jade",
            grouped(budget.passes),
            if budget.special { "Special Passes" } else { "Passes" },
            grouped(budget.jade)
        ))
        .child(glyph)
        .child(text(grouped(budget.warps)).font(COUNT).color(t.text))
        .child(text("warps").font(theme::SMALL).color(t.muted))
}

/// A dot for where syncing stands; its words on hover, and a sync on press
fn status_chip(s: &Ui, t: &Theme) -> El {
    let (message, tint) = super::warps::status(s, t);
    let dot = if s.warps.busy.is_some() {
        rune_kit::Spinner::new().size(12.0).color(t.accent).build(&t.kit)
    } else {
        el().size(8.0, 8.0).radius(4.0).fill(if tint == t.faint { t.muted } else { tint })
    };
    row()
        .key("warp-status")
        .role("button")
        .label("Sync")
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(16.0)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .hover(t.text.alpha(0.06))
        .help(format!("{message}. Press to sync"))
        .on_press(|s: &mut Ui| s.sync_warps())
        .child(dot)
        .child(
            text(short_status(&message))
                .font(theme::SMALL)
                .color(t.muted)
                .truncate()
                .max_w(170.0),
        )
}

/// The status in a couple of words, for the header
fn short_status(message: &str) -> String {
    if let Some(rest) = message.strip_prefix("Synced ") {
        return format!("Synced {rest}");
    }
    if message.contains("live pulls waiting") {
        return message.split(" live").next().map_or("Waiting".into(), |n| format!("{n} waiting"));
    }
    if message.starts_with("Log in") {
        return "Not logged in".into();
    }
    message.split(" · ").next().unwrap_or(message).to_string()
}

fn ghost_button(key: &'static str, label: &'static str, glyph: &'static rune::Icon, t: &Theme, on_press: impl Fn(&mut Ui) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(32.0)
        .px(12.0)
        .gap(7.0)
        .center_y()
        .radius(16.0)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .hover(t.text.alpha(0.08))
        .on_press(on_press)
        .child(icon(glyph, 15.0).color(t.muted))
        .child(text(label).font(theme::LABEL).color(t.text))
}

/// What's featured now and how long it runs, with a pick between a pool's running banners
fn banner_line(s: &Ui, view: &View, running: &[&RunningBanner], t: &Theme) -> El {
    let mut line = col().key("banner-line").gap(10.0);
    let words = match view.featured {
        Some((item, banner)) => {
            let left = banner.end.map_or(String::new(), |end| format!("  ·  {}", time_left(end)));
            format!("Rate-up  {}{left}", view.summary.name(item))
        }
        None if view.pool.has_rate_up() => "No rate-up banner seen yet. Open the warp screen in game".into(),
        None => match view.pool {
            Pool::Standard => "Stellar Warp, every standard 5-star".into(),
            _ => format!("{} Warp", label(view.pool)),
        },
    };
    line = line.child(text(words).font(theme::LABEL).color(t.muted).truncate());

    // More than one featured 5 star running: a pick between them
    let choices: Vec<u32> = running.iter().flat_map(|b| b.fives.iter().copied()).collect();
    if choices.len() > 1 {
        let chosen = view.featured.map(|(item, _)| item);
        let picks = choices.into_iter().map(|item| {
            let on = Some(item) == chosen;
            let pool = view.pool;
            let mut pick = el()
                .key(format!("pick-{item}"))
                .size(44.0, 44.0)
                .radius(22.0)
                .clip_rounded(22.0)
                .fill(t.text.alpha(0.08))
                .border(if on { 2.0 } else { 1.0 }, if on { view.accent } else { t.text.alpha(0.15) })
                .opacity(if on { 1.0 } else { 0.6 })
                .role("tab")
                .label(view.summary.name(item))
                .help(view.summary.name(item))
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut Ui| {
                    s.warps.featured.insert(pool, item);
                });
            if let Some(image) = s.art.as_ref().and_then(|art| art.get(Art::Preview(item))) {
                pick = pick.pixels_fit(image.pixels(), SurfaceFit::Fill);
            }
            pick
        });
        line = line.child(row().gap(8.0).children(picks));
    }
    rise(line, 0.05)
}

/// The pity, big, with the 50/50 and a bar to hard pity with the soft pity stretch marked
fn pity_block(view: &View, t: &Theme) -> El {
    let stats = view.stats;
    let rates = Rates::of(view.pool);
    let hard = rates.hard as f32;

    let state = match (view.pool.has_rate_up(), stats.guarantee5) {
        (false, _) => None,
        (true, Some(true)) => Some(("Guaranteed", view.accent)),
        (true, Some(false)) => Some(("50/50", t.text.alpha(0.7))),
        (true, None) => Some(("50/50?", t.text.alpha(0.5))),
    };
    let number = row()
        .key(format!("pity-{}", view.pool.gacha_type()))
        .gap(12.0)
        .items_end()
        .child(text(stats.pity5.to_string()).font(PITY).color(t.text))
        .child(
            col()
                .pb(12.0)
                .gap(8.0)
                .children(state.map(|(words, tint)| {
                    row()
                        .h(24.0)
                        .px(10.0)
                        .center_y()
                        .radius(12.0)
                        .border(1.0, tint)
                        .child(text(words).font(theme::KICKER).color(tint))
                }))
                .child(text(format!("of {} pity", rates.hard)).font(theme::LABEL).color(t.muted)),
        );

    let filled = (stats.pity5 as f32 / hard).clamp(0.0, 1.0);
    let soft = (rates.soft_from - 1) as f32 / hard;
    let in_soft = stats.pity5 + 1 >= rates.soft_from;
    let bar = el()
        .key(format!("pity-bar-{}", view.pool.gacha_type()))
        .w_full()
        .h(8.0)
        .radius(4.0)
        .fill(t.text.alpha(0.1))
        .clip_rounded(4.0)
        .child(span(soft, 1.0, view.accent.alpha(0.22), |fill| fill))
        .child(span(0.0, filled, view.accent, |fill| {
            fill.transition(Transition::reveal(Edge::Leading).springs(art_springs()))
                .enter_delay(0.25)
        }));
    let marks = row().w_full().gap(10.0).child(
        text(if in_soft {
            "In soft pity: the odds climb every warp".to_string()
        } else {
            format!(
                "{} to soft pity, {} to hard pity",
                rates.soft_from - 1 - stats.pity5,
                rates.hard - stats.pity5
            )
        })
        .font(theme::SMALL)
        .color(if in_soft { view.accent } else { t.muted })
        .grow(1.0)
        .min_w(0.0)
        .truncate(),
    );

    let four = format!(
        "4-star within {}  ·  {}",
        10u32.saturating_sub(stats.pity4).max(1),
        match stats.guarantee4 {
            Some(true) => "next 4-star is a featured one",
            Some(false) => "next 4-star is a 50/50",
            None => "",
        }
    );
    rise(
        col()
            .key("pity-block")
            .w_full()
            .gap(10.0)
            .child(number)
            .child(bar)
            .child(marks)
            .child(text(four.trim_end_matches("  ·  ").to_string()).font(theme::SMALL).color(t.faint)),
        0.12,
    )
}

/// A stretch of a bar in `color`, from and to fractions of its width. `dress` adds to the stretch
fn span(from: f32, to: f32, color: Color, dress: impl FnOnce(El) -> El) -> El {
    let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
    row()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(el().grow(from).w(0.0))
        .child(dress(el().grow((to - from).max(0.0)).w(0.0).h_full().fill(color)))
        .child(el().grow(1.0 - to).w(0.0))
}

/// What the warps in hand buy, or without them what a 5 star takes
fn odds_row(view: &View, t: &Theme) -> El {
    let rates = Rates::of(view.pool);
    let stats = view.stats;
    let guaranteed = stats.guarantee5 == Some(true);
    let five = odds::next_five(&rates, stats.pity5);
    let featured = view
        .pool
        .has_rate_up()
        .then(|| odds::next_featured(&rates, stats.pity5, guaranteed));
    let worst = odds::worst(featured.as_ref().unwrap_or(&five));

    let mut figures = Vec::new();
    match view.budget {
        Some(budget) => {
            figures.push(figure(percent(odds::within(&five, budget.warps)), "a 5-star", t));
            if let Some(featured) = &featured {
                let name = view
                    .featured
                    .map_or("the featured one".to_string(), |(item, _)| view.summary.name(item));
                figures.push(figure(percent(odds::within(featured, budget.warps)), &name, t));
            }
            let note = if budget.warps >= worst {
                "covered".to_string()
            } else {
                format!("{} short", grouped(worst - budget.warps))
            };
            figures.push(figure(grouped(worst), &format!("at most, {note}"), t));
        }
        None => {
            figures.push(figure(format!("~{:.0}", odds::expected(&five)), "to a 5-star", t));
            if let Some(featured) = &featured {
                figures.push(figure(format!("~{:.0}", odds::expected(featured)), "to the featured one", t));
            }
            figures.push(figure(grouped(worst), "at most", t));
        }
    }
    let caption = match view.budget {
        Some(budget) => format!("With your {} warps", grouped(budget.warps)),
        None => "Warps it takes, on average. Log in to see what yours buy".to_string(),
    };
    rise(
        col()
            .key("odds")
            .w_full()
            .gap(10.0)
            .child(text(caption).font(theme::KICKER).color(t.faint))
            .child(row().w_full().gap(26.0).children(figures)),
        0.18,
    )
}

fn figure(value: String, words: &str, t: &Theme) -> El {
    col()
        .gap(2.0)
        .min_w(0.0)
        .child(text(value).font(FIGURE).color(t.text))
        .child(text(words.to_string()).font(theme::SMALL).color(t.muted).truncate().max_w(150.0))
}

/// How far the warps in hand go towards the next copies of the featured 5 star
fn targets(view: &View, t: &Theme) -> Option<El> {
    let (item, _) = view.featured?;
    let rates = Rates::of(view.pool);
    let light_cone = matches!(view.pool, Pool::LightCone | Pool::CollabLightCone);
    let owned = view.roster.map(|r| r.copies(item));
    let have = owned.unwrap_or(0);
    let most = if light_cone { 5 } else { 7 };
    if have >= most {
        return Some(rise(
            text(format!("{} is maxed", view.summary.name(item)))
                .font(theme::LABEL)
                .color(t.muted),
            0.24,
        ));
    }
    let count = (most - have).min(3) as usize;
    let costs = odds::copies(&rates, view.stats.pity5, view.stats.guarantee5 == Some(true), count);
    let level = |copies: u32| {
        if light_cone {
            format!("S{copies}")
        } else {
            format!("E{}", copies - 1)
        }
    };
    let rows = costs.iter().enumerate().map(|(i, cost)| {
        let target = have + i as u32 + 1;
        let chance = view.budget.map(|b| odds::within(cost, b.warps));
        let mut row = row()
            .key(format!("target-{target}"))
            .w_full()
            .gap(12.0)
            .center_y()
            .child(text(level(target)).font(TAB).color(t.text).w(30.0));
        row = match chance {
            Some(chance) => row
                .child(
                    el().grow(1.0)
                        .h(6.0)
                        .radius(3.0)
                        .fill(t.text.alpha(0.1))
                        .clip_rounded(3.0)
                        .child(span(0.0, chance as f32, view.accent.alpha(0.85 - i as f32 * 0.2), |fill| fill)),
                )
                .child(text(percent(chance)).font(COUNT).color(t.text).w(44.0).align(rune::Align::End)),
            None => row.child(el().grow(1.0)),
        };
        row.child(
            text(format!("~{:.0} warps", odds::expected(cost)))
                .font(theme::SMALL)
                .color(t.muted)
                .w(82.0)
                .align(rune::Align::End),
        )
    });
    let owned_words = match owned {
        Some(0) => "Not owned yet".to_string(),
        Some(c) => format!("You have {}", level(c)),
        None => "Log in to count what you own".to_string(),
    };
    Some(rise(
        col()
            .key("targets")
            .w_full()
            .gap(9.0)
            .child(
                row()
                    .gap(8.0)
                    .child(text("Copies").font(theme::KICKER).color(t.faint))
                    .child(text(owned_words).font(theme::SMALL).color(t.muted)),
            )
            .children(rows),
        0.24,
    ))
}

/// The hero's name over its art, with what the account has of it
fn name_tags(view: &View, item: u32, t: &Theme) -> El {
    let light_cone = matches!(view.pool, Pool::LightCone | Pool::CollabLightCone);
    let kicker = if view.featured.is_some() {
        "Featured 5-star".to_string()
    } else {
        view.stats.fives.first().map_or("5-star".into(), |f| {
            format!("Latest 5-star  ·  pity {}{}", f.pity, match outcome_words(f.outcome) {
                "" => String::new(),
                w => format!(", {w}"),
            })
        })
    };
    let owned = view.roster.map(|r| r.copies(item)).map(|c| match (c, light_cone) {
        (0, _) => "Not owned".to_string(),
        (c, true) => format!("Owned  ·  S{c}"),
        (c, false) => format!("Owned  ·  E{}", c - 1),
    });
    let glass = |content: El| {
        row()
            .h(26.0)
            .px(10.0)
            .center_y()
            .radius(13.0)
            .fill(Color::BLACK.alpha(0.32))
            .backdrop_blur(12.0)
            .child(content)
    };
    let _ = t;
    rise(
        col()
            .key(format!("name-{item}"))
            .gap(8.0)
            .items_end()
            .max_w(420.0)
            .child(text("★★★★★").font(theme::LABEL).color(rune::oklch(0.86, 0.14, 85.0)))
            .child(
                text(view.summary.name(item))
                    .font(NAME)
                    .color(Color::WHITE)
                    .wrap()
                    .align(rune::Align::End),
            )
            .child(
                row()
                    .gap(6.0)
                    .child(glass(text(kicker).font(theme::SMALL).color(Color::WHITE.alpha(0.9))))
                    .children(owned.map(|o| glass(text(o).font(theme::SMALL).color(Color::WHITE.alpha(0.9))))),
            ),
        0.4,
    )
}

/// Every 5 star of the pool as a bar of its pity, oldest on the left, coloured by its 50/50
fn history_strip(view: &View, width: f32, t: &Theme) -> El {
    let stats = view.stats;
    let hard = Rates::of(view.pool).hard as f32;
    let (bar_w, gap) = (9.0, 4.0);
    let fit = ((width + gap) / (bar_w + gap)).floor().max(1.0) as usize;
    let shown: Vec<&Highlight> = stats.fives.iter().take(fit).collect();
    let n = shown.len();
    let stagger = Stagger::new(0.012).from(StaggerFrom::Last);
    let bars = shown.iter().rev().enumerate().map(|(i, five)| {
        let when = chrono::DateTime::from_timestamp(five.time, 0)
            .map_or(String::new(), |d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string());
        let outcome = match outcome_words(five.outcome) {
            "" => String::new(),
            w => format!(", {w}"),
        };
        el().key(format!("five-{}-{}", five.time, five.item_id))
            .w(bar_w)
            .h(6.0 + 58.0 * (five.pity as f32 / hard).min(1.0))
            .radius(2.5)
            .fill(outcome_color(five.outcome))
            .opacity(if five.confirmed { 1.0 } else { 0.6 })
            .help(format!(
                "{}  ·  pity {}{outcome}  ·  {when}",
                view.summary.name(five.item_id),
                five.pity
            ))
            .transition(
                Transition::offset(0.0, 10.0)
                    .combined(Transition::opacity())
                    .springs(rise_springs()),
            )
            .enter_delay(0.3 + stagger.delay(i, n))
    });

    let mut summary_words = Vec::new();
    if let Some(avg) = stats.avg_pity5 {
        summary_words.push(format!("avg {avg:.0}"));
    }
    if stats.played5 > 0 {
        summary_words.push(format!("won {} of {}", stats.won5, stats.played5));
    }
    summary_words.push(format!("{} warps", grouped(stats.total)));

    let legend = |color: Color, words: &'static str| {
        row()
            .gap(5.0)
            .center_y()
            .child(el().size(8.0, 8.0).radius(2.0).fill(color))
            .child(text(words).font(theme::SMALL).color(t.faint))
    };
    let mut head = row()
        .w_full()
        .gap(14.0)
        .center_y()
        .child(text(format!("{} 5-stars", stats.count5)).font(theme::KICKER).color(t.faint))
        .child(text(summary_words.join("  ·  ")).font(theme::SMALL).color(t.muted));
    if view.pool.has_rate_up() {
        head = head
            .child(el().grow(1.0))
            .child(legend(outcome_color(Outcome::Won), "won"))
            .child(legend(outcome_color(Outcome::Lost), "lost"))
            .child(legend(outcome_color(Outcome::Guaranteed), "guaranteed"));
    }
    let body = if n == 0 {
        row()
            .h(64.0)
            .center_y()
            .child(text("No 5-stars here yet").font(theme::SMALL).color(t.faint))
    } else {
        row()
            .key(format!("strip-{}", view.pool.gacha_type()))
            .h(64.0)
            .gap(gap)
            .items_end()
            .children(bars)
    };
    col().key("history-strip").w_full().gap(10.0).child(head).child(body)
}

/// What was pulled while the archiver watched, this run
fn session_row(s: &Ui, view: &View, t: &Theme) -> Option<El> {
    let since = s.warps.session_start;
    let mut warps = 0usize;
    let mut fives = Vec::new();
    for stats in &view.summary.pools {
        for (pull, pity, outcome) in &stats.recent {
            if pull.sources & source::LIVE != 0 && pull.time >= since {
                warps += 1;
                if pull.rarity == 5 {
                    fives.push((pull.item_id, *pity, *outcome));
                }
            }
        }
    }
    if warps == 0 {
        return None;
    }
    let mut words = format!("This session  ·  {} warp{}", grouped(warps), if warps == 1 { "" } else { "s" });
    for (item, pity, outcome) in &fives {
        words.push_str(&format!("  ·  {} at {pity}", view.summary.name(*item)));
        if !outcome_words(*outcome).is_empty() {
            words.push_str(&format!(" ({})", outcome_words(*outcome)));
        }
    }
    let waiting = view.summary.unconfirmed;
    Some(rise(
        row()
            .key("session")
            .w_full()
            .gap(10.0)
            .center_y()
            .child(
                row()
                    .h(22.0)
                    .px(8.0)
                    .center_y()
                    .radius(11.0)
                    .fill(view.accent.alpha(0.2))
                    .child(text("LIVE").font(theme::KICKER).color(view.accent)),
            )
            .child(text(words).font(theme::SMALL).color(t.text).truncate().grow(1.0).min_w(0.0))
            .children((waiting > 0).then(|| {
                text(format!("{waiting} waiting for the game's history"))
                    .font(theme::SMALL)
                    .color(t.faint)
            })),
        0.35,
    ))
}

// ---- words and numbers -------------------------------------------------------------------------

fn percent(p: f64) -> String {
    match p {
        p if p >= 0.9995 => "100%".into(),
        p if p > 0.995 => ">99%".into(),
        p if p > 0.0 && p < 0.005 => "<1%".into(),
        p => format!("{:.0}%", p * 100.0),
    }
}

fn time_left(end: i64) -> String {
    let left = (end - chrono::Utc::now().timestamp()).max(0);
    let (days, hours) = (left / 86400, left % 86400 / 3600);
    if days > 0 {
        format!("{days}d {hours}h left")
    } else {
        format!("{hours}h {}m left", left % 3600 / 60)
    }
}

fn grouped(n: impl TryInto<u64>) -> String {
    let digits = n.try_into().unwrap_or(0).to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_round_honestly() {
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(0.9991), ">99%");
        assert_eq!(percent(0.4449), "44%");
        assert_eq!(percent(0.001), "<1%");
        assert_eq!(percent(0.0), "0%");
    }

    #[test]
    fn statuses_shorten() {
        assert_eq!(short_status("Synced 2 min ago"), "Synced 2 min ago");
        assert_eq!(
            short_status("4 live pulls waiting for the game's history · next look 01:45"),
            "4 waiting"
        );
        assert_eq!(short_status("Log in to the game to start keeping your warps"), "Not logged in");
    }
}
