//! The warp screen. It takes the game's warp screen as its visual language, built from the same
//! pieces as the main window (plates that tear like tickets, tags, medallions on a gold track): the
//! banner's art is the place, not a picture on it, its colour is the screen's accent, and one
//! number carries the screen. What's on it, in the order it matters:
//!
//! 1. The pity on a gold track, how near soft and hard pity it is, and whether the next 5 star is a
//!    50/50 or guaranteed.
//! 2. What the warps in hand buy: the chance of a 5 star, and of the featured one, before they run
//!    out, and the most it can take.
//! 3. How far they go towards copies of what's featured, from what the account owns.
//! 4. Every 5 star as a bar of its pity, coloured by its 50/50.
//! 5. What was pulled this session, while the game's history catches up.
//!
//! The warps are tickets down a rail on the left: their art, and their pity on the stub. The rest
//! (sync, authkeys, imports, the full list) is in the history sheet behind "History".
//!
//! The art comes in as the game's does: the banner's layers slide in from the left at their own
//! speeds and settle, the number and the rest follow a beat behind.

use rune::{
    Color, Cx, Easing, Edge, El, LoopSpec, Spring, SpringPair, Stagger, StaggerFrom, SurfaceFit, TextStyle, Transition, Tx, col, el, icon,
    row, text,
};

use super::parts::{self, Pill, kicker, perforation, pill, plate, ring_button, rule, tab, ticket_on};
use crate::gui::art::{Art, BannerArt, Layer, Ticket};
use crate::gui::icons;
use crate::gui::state::{Funds, Panel, Roster, Ui};
use crate::gui::theme::{self, Theme};
use crate::warps::odds::{self, Rates};
use crate::warps::service::{RunningBanner, Summary};
use crate::warps::stats::{Highlight, Outcome, PoolStats};
use crate::warps::{Pool, source};

/// Narrower than this, the screen stacks
const NARROW: f32 = 760.0;
/// At least this wide, and at least this much wider than tall, the rail shows the warps in full
const WIDE: f32 = 1180.0;
const WIDE_ASPECT: f32 = 1.45;
const PAD: f32 = 30.0;
const GAP: f32 = 26.0;
/// The game lays its warp screen out on this
const REFERENCE: [f32; 2] = [1920.0, 1080.0];
/// How far left the game starts each layer, in reference units, back to front
const LAYER_TRAVEL: [f32; 3] = [77.0, 93.0, 104.0];

const PITY: TextStyle = TextStyle::new(88.0, 650.0).lh(84.0).tracking(-1.5).features("'tnum' 1");
const FIGURE: TextStyle = TextStyle::new(26.0, 600.0).lh(30.0).features("'tnum' 1");
const NAME: TextStyle = TextStyle::new(32.0, 650.0).lh(36.0).tracking(0.2);
const TAB: TextStyle = TextStyle::new(14.5, 600.0).lh(18.0).tracking(0.3);
const COUNT: TextStyle = TextStyle::new(13.0, 600.0).lh(16.0).features("'tnum' 1");
const STUB: TextStyle = TextStyle::new(20.0, 600.0).lh(22.0).features("'tnum' 1");
/// A rail entry's width and its ticket's height, full and compact
const TICKET: [f32; 2] = [262.0, 62.0];
const COMPACT_TICKET: [f32; 2] = [172.0, 44.0];
/// The game's tickets are about 262 by 96. Selected, the same ticket is framed, the character's
/// head rises above it, and the frame's offset line runs below: in the selected sprite the ticket
/// sits a point left of its left edge and `SELECTED_BELOW` points above its bottom, however tall
/// the head makes it
const SPRITE: [f32; 2] = [262.0, 96.0];
const SELECTED_LEFT: f32 = 1.0;
const SELECTED_BELOW: f32 = 17.0;
/// Room above the rail for the tallest head
const HEADROOM: f32 = 46.0;
/// Where the art's room starts from the top, under the header
const ART_TOP: f32 = 64.0;
/// The banner's motif behind the warp card, at this share of its size on the game's reference
/// screen: smaller than there, as texture rather than picture
const MOTIF_SCALE: f32 = 1.0;

/// How the screen lays out for the window's shape
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fit {
    /// Rail, card and art side by side, the rail's tickets in full
    Wide,
    /// The same with a compact rail, for windows nearer square
    Medium,
    /// Stacked: the tickets in a row, the art in a band, the card under it
    Narrow,
}

impl Fit {
    fn of(width: f32, height: f32) -> Fit {
        if width < NARROW {
            Fit::Narrow
        } else if width >= WIDE && width / height.max(1.0) >= WIDE_ASPECT {
            Fit::Wide
        } else {
            Fit::Medium
        }
    }

    fn ticket(self) -> [f32; 2] {
        if self == Fit::Wide { TICKET } else { COMPACT_TICKET }
    }

    /// The card's width in a window `width` wide, side by side
    fn card(self, width: f32) -> f32 {
        match self {
            Fit::Wide => (width * 0.3).clamp(360.0, 420.0),
            _ => (width * 0.4).clamp(340.0, 400.0),
        }
    }

    /// Where the art's room starts, from the window's left: past the rail and the card
    fn art_from(self, width: f32) -> f32 {
        match self {
            Fit::Narrow => 0.0,
            _ => self.card_from() + self.card(width) + GAP,
        }
    }

    /// Where the card starts, from the window's left: past the rail
    fn card_from(self) -> f32 {
        PAD + self.ticket()[0] + GAP
    }

    /// Where the room the art is sized and centred in starts: past the card
    fn stage_from(self, width: f32) -> f32 {
        self.card_from() + self.card(width)
    }
}

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

fn outcome_color(outcome: Outcome, t: &Theme) -> Color {
    match outcome {
        Outcome::Won => t.good,
        Outcome::Guaranteed => parts::progress(t),
        Outcome::Lost => t.bad,
        Outcome::Unknown | Outcome::None => t.text.alpha(0.45),
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
    let fit = Fit::of(width, height);

    let Some(summary) = s.warps_summary() else {
        return el().key("warp-screen").w_full().h_full().fill(t.bg).padding(22.0).child(
            col()
                .w_full()
                .gap(18.0)
                .child(header(s, None, Color::WHITE, t))
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
    page = page.child(stage(s, &view, width, height, fit, t));
    page = page.child(scrims(width, height, fit, t));

    let content = if fit != Fit::Narrow {
        let middle = row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(GAP)
            .items_start()
            .child(rail(s, summary, pool, fit, t))
            .child(banner_card(s, &view, &running, cx, t).w(fit.card(width)))
            .child(el().grow(1.0))
            .children(hero.map(|item| col().h_full().justify_end().child(name_tags(&view, item, t))));
        col()
            .key("warp-content")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .px(PAD)
            .pt(20.0)
            .pb(24.0)
            .gap(24.0)
            .child(header(s, Some(pool), accent, t))
            .child(middle)
            .child(history_card(&view, width - PAD * 2.0, cx, t))
            .children(session_row(s, &view, t))
    } else {
        let band = narrow_band(height);
        let column = col()
            .key("warp-content-narrow")
            .w_full()
            .pt(14.0)
            .pb(20.0)
            .gap(18.0)
            .child(header(s, Some(pool), accent, t).px(16.0))
            .child(rail(s, summary, pool, fit, t))
            // Room for the art, its name over its foot
            .child(
                col()
                    .w_full()
                    .px(16.0)
                    .h(band)
                    .justify_end()
                    .items_end()
                    .children(hero.map(|item| name_tags(&view, item, t))),
            )
            .child(
                col()
                    .w_full()
                    .px(16.0)
                    .gap(22.0)
                    .child(banner_card(s, &view, &running, cx, t).w_full())
                    .child(history_card(&view, width - 32.0, cx, t))
                    .children(session_row(s, &view, t)),
            );
        col()
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(column.scroll(rune::ScrollSpec { stick_bottom: false }))
    };
    page.child(content)
}

/// How tall the art's band is in a narrow window
fn narrow_band(height: f32) -> f32 {
    (height * 0.36).clamp(200.0, 380.0)
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
            .fill(t.bg.alpha(if t.dark { 0.15 } else { 0.1 })),
    )
}

/// The banner's art: its layers placed as the game places them, sliding in at their own speeds.
/// Without them, the hero's portrait on the right
fn stage(s: &Ui, view: &View, width: f32, height: f32, fit: Fit, t: &Theme) -> El {
    let wide = fit != Fit::Narrow;
    let Some(item) = view.hero else { return el() };
    let mut stage = el().key("warp-stage").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert();

    if let Some(art) = view.art.as_ref().filter(|art| art.placed())
        && let Some(main) = art.layer(Layer::Main)
        && let Some(main_on_screen) = main.on_screen
    {
        // Side by side: the game's screen at the scale that covers the window, as in game, unless
        // the character would then overflow the room right of the card; then smaller, so the whole
        // of them fits. They stand in the middle of that room. Narrow: the character fitted into
        // the band under the tickets
        let scale = main_on_screen[2] / main.canvas[0];
        let shown = [
            main_on_screen[0] + main.rect[0] * scale,
            main_on_screen[1] + main.rect[1] * scale,
            main.rect[2] * scale,
            main.rect[3] * scale,
        ];
        let (k, ox, oy) = if wide {
            let room = fit.stage_from(width);
            let (room_w, room_h) = ((width - room - PAD).max(1.0), (height - ART_TOP - PAD).max(1.0));
            let cover = (width / REFERENCE[0]).max(height / REFERENCE[1]);
            let k = cover.min(room_w / shown[2]).min(room_h / shown[3]);
            let centre = [(room + width - PAD) / 2.0, ART_TOP + room_h / 2.0];
            (k, centre[0] - (shown[0] + shown[2] / 2.0) * k, centre[1] - (shown[1] + shown[3] / 2.0) * k)
        } else {
            let band = narrow_band(height);
            let top = 14.0 + 40.0 + 18.0 + COMPACT_TICKET[1] + 18.0;
            let k = (band * 1.15 / shown[3]).min(width * 1.05 / shown[2]);
            let ox = width / 2.0 - (shown[0] + shown[2] / 2.0) * k;
            let oy = top - shown[1] * k;
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
        let room = fit.stage_from(width);
        let side = (height - ART_TOP - PAD).min(width - room - PAD).max(240.0);
        let x = (room + width - PAD) / 2.0 - side / 2.0;
        let y = ART_TOP + (height - ART_TOP - PAD - side) / 2.0;
        stage.child(picture.pin(Some(x), Some(y), None, None).size(side, side))
    } else {
        let side = narrow_band(height) * 1.25;
        let top = 14.0 + 40.0 + 18.0 + COMPACT_TICKET[1];
        stage.child(picture.pin(Some((width - side) / 2.0), Some(top), None, None).size(side, side))
    }
}

/// Shade where the words go: down the left of a wide window, under the band of a narrow one, and
/// along the foot for the history
fn scrims(width: f32, height: f32, fit: Fit, t: &Theme) -> El {
    let ground = t.bg;
    let mut layer = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert();
    if fit != Fit::Narrow {
        layer = layer
            .child(
                el().pin(Some(0.0), Some(0.0), None, Some(0.0))
                    .w((fit.art_from(width) + 120.0).min(width))
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
        let top = 14.0 + 40.0 + 18.0 + COMPACT_TICKET[1];
        let band = narrow_band(height);
        layer = layer
            .child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), None)
                    .h(top + 40.0)
                    .linear_gradient(std::f32::consts::PI, [
                        (0.0, ground.alpha(0.9)),
                        (0.75, ground.alpha(0.7)),
                        (1.0, ground.alpha(0.0)),
                    ]),
            )
            .child(
                el().pin(Some(0.0), Some(top + band * 0.55), Some(0.0), None)
                    .h(band * 0.5)
                    .gradient(ground.alpha(0.0), ground),
            )
            .child(el().pin(Some(0.0), Some(top + band * 1.05), Some(0.0), Some(0.0)).fill(ground));
    }
    layer
}

// ---- the words ---------------------------------------------------------------------------------

/// The warp glyph and the screen's name; what's in hand, where syncing stands, the history, and
/// the way back
fn header(s: &Ui, selected: Option<Pool>, accent: Color, t: &Theme) -> El {
    let mut bar = row()
        .key("warp-header")
        .w_full()
        .gap(10.0)
        .center_y()
        .flex_wrap()
        .child(icon(&icons::WARP, 24.0).color(accent))
        .child(text("Warp").font(theme::TITLE).color(t.text))
        .child(el().grow(1.0));
    if let (Some(pool), Some(funds)) = (selected, s.warp_funds()) {
        bar = bar.child(budget_chip(s, Budget::of(funds, pool), t));
    }
    bar.child(status_chip(s, t))
        .child(pill(
            "warp-history",
            "History",
            Some(&icons::LIST),
            Pill::Secondary,
            true,
            t,
            |s: &mut Ui, _: &mut Tx| s.panel = Some(Panel::Warps),
        ))
        .child(ring_button("close-warps", &icons::CLOSE, "Back", false, t, |s| s.close_warps()))
}

/// The warps as tickets, the game's own from its warp screen where there's one: the selected one
/// framed, the character rising over it. Left of each its pity, and its name when there's room. A
/// column down the left, compact in a window nearer square; a row across the top of a narrow
/// one, scrolling
fn rail(s: &Ui, summary: &Summary, selected: Pool, fit: Fit, t: &Theme) -> El {
    let [w, h] = fit.ticket();
    let full = fit == Fit::Wide;
    let tickets = summary.pools.iter().map(|stats| {
        let pool = stats.pool;
        let chosen = pool == selected;
        let featured = summary.running(pool).next().and_then(|b| b.fives.first().copied());
        let ticket = Ticket::of(summary, pool);
        // Both of a ticket's pictures (preloaded when the warps come in, see `Ui::preload_warp_art`)
        let picture = |selected: bool| ticket.and_then(|ticket| s.art.as_ref()?.get(Art::Ticket(ticket, selected)));
        let (plain, framed) = (picture(false), picture(true));
        let size = |image: &rune::Image| image.size().map(|[w, h]| [w as f32, h as f32]);
        let plain_size = plain.as_ref().and_then(size).unwrap_or(SPRITE);
        // The rail's height over the ticket's, and its width at that
        let k = h / plain_size[1];
        let ticket_w = plain_size[0] * k;
        let sprite = if chosen { framed.clone() } else { plain.clone() };
        let key = format!("ticket-{}", pool.gacha_type());

        let face = match sprite {
            // The game's ticket. The selected one is drawn at the same scale, placed so its ticket
            // lies exactly over the plain one's, its frame and the head above it spilling out
            Some(image) => {
                let picture = el().pixels_fit(image.pixels(), SurfaceFit::Fill);
                let picture = if chosen {
                    let [sw, sh] = size(&image).unwrap_or([SPRITE[0] + 14.0, SPRITE[1] + SELECTED_BELOW]);
                    let above = sh - SELECTED_BELOW - plain_size[1];
                    picture
                        .key(format!("{key}-selected"))
                        .pin(Some(SELECTED_LEFT * k), Some(-above * k), None, None)
                        .size(sw * k, sh * k)
                        .transition(Transition::opacity().springs(SpringPair::new(Spring::smooth(0.35, 0.0), Spring::smooth(0.18, 0.0))))
                } else {
                    picture
                        .key(format!("{key}-plain"))
                        .pin(Some(0.0), Some(0.0), None, None)
                        .size(ticket_w, h)
                        .opacity(0.8)
                        .transition(Transition::opacity().springs(SpringPair::new(Spring::smooth(0.3, 0.0), Spring::smooth(0.18, 0.0))))
                };
                // Light catching it each time it's picked: the ticket drawn in white (`pixels_tint`, so
                // only the ticket's own pixels light), seen through a window that sweeps across it
                let face = el().size(ticket_w, h).child(picture);
                // Only for the moment after the ticket's picked: a layout that rebuilds the rail
                // later mounts the shine afresh, which would sweep again
                let fresh = s
                    .warps
                    .picked
                    .is_some_and(|(at, when)| at == pool && when.elapsed() < crate::gui::state::SHINE_FOR);
                let glint = (chosen && fresh)
                    .then(|| framed.clone())
                    .flatten()
                    .and_then(|image| size(&image).map(|at| (image, at)));
                let shine = glint.map(|(white, [sw, sh])| {
                    let above = sh - SELECTED_BELOW - plain_size[1];
                    let (x, y, w, h) = (SELECTED_LEFT * k, -above * k, sw * k, sh * k);
                    // Rune's shimmer, slanted: a frame turned by `SLANT`, centred on the ticket, holds
                    // a window that slides across it once while the white inside the window slides
                    // back as far. The window's straight edges come out slanted; the white, turned
                    // back about its centre (the ticket's, as the slides cancel), sits upright on the
                    // ticket. It mounts afresh on each pick, so each pick sweeps; played out it rests
                    // past the ticket, its window empty
                    const SLANT: f32 = 0.35;
                    const SWEEP: f64 = 0.5;
                    let band = w * 0.52;
                    // The frame reaches past the ticket so that turned it still covers it
                    let reach = (w.max(h)) * 0.5;
                    let (fw, fh) = (w + reach * 2.0, h + reach * 2.0);
                    let travel = fw + band;
                    // A slide's offsets are fractions of the element's own size: the window is a
                    // band wide, the white inside it the whole ticket
                    let slide = |to: f32| LoopSpec {
                        delay: 0.06,
                        ..LoopSpec::slide(SWEEP, [0.0, 0.0], [to, 0.0]).easing(Easing::LINEAR).times(1)
                    };
                    // Layout changes land at once inside it: the window and the white in it are placed
                    // apart, and on their own springs they'd part for a few frames
                    el().key(format!("{key}-shine"))
                        .instant_changes()
                        .pin(Some(x - reach), Some(y - reach), None, None)
                        .size(fw, fh)
                        .rotate(SLANT)
                        .inert()
                        .transition(Transition::none())
                        .child(
                            el().pin(Some(-band), Some(0.0), None, None)
                                .size(band, fh)
                                .clip_fade(band * 0.18, 0.0, band * 0.18, 0.0)
                                .transition(Transition::none())
                                .looping(slide(travel / band))
                                .child(
                                    el().pin(Some(reach + band), Some(reach), None, None)
                                        .size(w, h)
                                        .rotate(-SLANT)
                                        .pixels_fit(white.pixels(), SurfaceFit::Fill)
                                        .pixels_tint(Color::WHITE)
                                        .opacity(0.7)
                                        .transition(Transition::none())
                                        .looping(slide(-travel / w)),
                                ),
                        )
                });
                // The shine's holder is always there and only the shine comes and goes: mounted
                // with its holder, it entered with it and never swept
                face.child(
                    el().key(format!("{key}-shine-holder"))
                        .pin(Some(0.0), Some(0.0), None, None)
                        .size(ticket_w, h)
                        .inert()
                        .transition(Transition::none())
                        .children(shine),
                )
            }
            // One drawn like it: square cut, notched where the stub tears
            None => {
                let (fill, ink) = if chosen {
                    (t.primary, t.on_primary)
                } else {
                    (parts::glass(t), t.text)
                };
                let stub = ticket_w * 0.16;
                let art = featured
                    .or_else(|| stats.fives.first().map(|f| f.item_id))
                    .and_then(|item| s.art.as_ref()?.get(Art::Preview(item)));
                let mut face = row()
                    .size(ticket_w, h)
                    .fill(fill)
                    .cutout_circle([stub, 0.0], 4.5, 5.0)
                    .cutout_circle([stub, h], 4.5, 5.0);
                if let Some(image) = art {
                    face = face.child(
                        el().pin(Some(stub), Some(0.0), Some(0.0), Some(0.0))
                            .pixels_fit(image.pixels(), SurfaceFit::Fill)
                            .opacity(if chosen { 1.0 } else { 0.6 })
                            .child(
                                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                                    .linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, fill), (0.7, fill.alpha(0.0))]),
                            ),
                    );
                }
                face.child(
                    text(label(pool))
                        .font(theme::LABEL)
                        .color(ink)
                        .pl(stub + 10.0)
                        .self_center()
                        .truncate(),
                )
            }
        };

        // Left of the ticket, set against it
        let figures = col()
            .grow(1.0)
            .min_w(0.0)
            .pr(12.0)
            .justify_center()
            .items_end()
            .child(
                row()
                    .gap(4.0)
                    .items_end()
                    .child(
                        text(stats.pity5.to_string())
                            .font(if full { STUB } else { TAB })
                            .color(if chosen { t.text } else { t.muted })
                            .shadow(0.0, 6.0, t.bg.alpha(0.9)),
                    )
                    .children(full.then(|| text(format!("/{}", pool.hard_pity())).font(theme::SMALL).color(t.faint).mb(2.0))),
            )
            .children(full.then(|| {
                text(label(pool))
                    .font(theme::SMALL)
                    .color(if chosen { t.text } else { t.faint })
                    .align(rune::Align::End)
                    .truncate()
            }));

        row()
            .key(format!("rail-{}", pool.gacha_type()))
            .role("tab")
            .label(label(pool))
            .help(format!("{} warp, pity {} of {}", label(pool), stats.pity5, pool.hard_pity()))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .size(w, h)
            .on_press_tx(move |s: &mut Ui, tx: &mut Tx| crate::gui::state::pick_warp(s, pool, tx))
            .child(figures)
            .child(face)
    });
    if fit == Fit::Narrow {
        row()
            .key("warp-rail")
            .w_full()
            .px(16.0)
            .pt(HEADROOM * COMPACT_TICKET[1] / SPRITE[1])
            .gap(14.0)
            .children(tickets)
            .scroll_x()
    } else {
        col().key("warp-rail").pt(8.0).gap(14.0).children(tickets)
    }
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
        .h(34.0)
        .pl(8.0)
        .pr(14.0)
        .gap(6.0)
        .center_y()
        .radius(16.0)
        .fill(t.text.alpha(0.08))
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
        .h(34.0)
        .px(14.0)
        .gap(8.0)
        .center_y()
        .radius(17.0)
        .fill(t.text.alpha(0.08))
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

/// The banner's ticket: what's featured and the pick between the running banners, the pity on its
/// track, and past the tear what the warps in hand buy and how far towards copies
fn banner_card(s: &Ui, view: &View, running: &[&RunningBanner], cx: &Cx<Ui>, t: &Theme) -> El {
    let title = match view.pool {
        Pool::Character => "Character event warp",
        Pool::LightCone => "Light cone event warp",
        Pool::Standard => "Stellar warp",
        Pool::CollabCharacter => "Collab warp",
        Pool::CollabLightCone => "Collab light cone warp",
        Pool::Departure => "Departure warp",
    };
    let surface = view.featured.and_then(|(item, _)| card_surface(s, item, t));
    let card = ticket_on("warp-card", &["warp-card-tear"], surface, cx, t)
        .child(tab("warp-card-tab", title, view.accent, cx, t))
        .child(
            col()
                .w_full()
                .px(22.0)
                .pt(12.0)
                .pb(18.0)
                .gap(16.0)
                .child(banner_line(s, view, running, t))
                .child(rule(t))
                .child(pity_block(view, cx, t)),
        )
        .child(perforation("warp-card-tear", cx, t))
        .child(
            col()
                .w_full()
                .px(22.0)
                .pt(14.0)
                .pb(20.0)
                .gap(18.0)
                .child(odds_row(view, t))
                .children(targets(view, t)),
        );
    rise(card, 0.05)
}

/// The featured banner's own panel under the warp card, as the game puts it behind the art: its
/// colours rising from the bottom, faint enough to read over, and its motif tiled across
fn card_surface(s: &Ui, item: u32, t: &Theme) -> Option<El> {
    let art = s.art.as_ref()?;
    let surface = art.banner(item)?.surface?;
    let mut layer = el();
    if let Some(fill) = surface.fill {
        // Down the panel: the fill's start, its end, and at the bottom where the fade is whole,
        // the fade's end over the fill's (a fade can end clear)
        let bottom = match surface.fade {
            Some(fade) => {
                let (over, under, k) = (fade.end, fill.end, fade.end.a);
                Color { r: under.r + (over.r - under.r) * k, g: under.g + (over.g - under.g) * k, b: under.b + (over.b - under.b) * k, a: 1.0 }
            }
            None => fill.end,
        };
        let strength = if t.dark { 0.24 } else { 0.2 };
        layer = layer.linear_gradient(
            std::f32::consts::PI,
            [(0.0, fill.start.alpha(0.0)), (0.45, fill.end.alpha(strength * 0.35)), (1.0, bottom.alpha(strength))],
        );
    }
    if let Some(pattern) = &surface.pattern
        && let Some(image) = art.get(Art::Pattern(item))
    {
        let tile = pattern.tile.map(|v| v * MOTIF_SCALE);
        layer = layer.child(
            el().key("warp-card-motif")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .pixels_tile(image.pixels(), tile)
                .pixels_tint(t.text.alpha(if t.dark { 0.025 } else { 0.045 })),
        );
    }
    Some(layer)
}

/// What's featured now and how long it runs, with a pick between a pool's running banners
fn banner_line(s: &Ui, view: &View, running: &[&RunningBanner], t: &Theme) -> El {
    let mut line = col().key("banner-line").gap(10.0);
    let (words, left) = match view.featured {
        Some((item, banner)) => (view.summary.name(item), banner.end.map(time_left)),
        None if view.pool.has_rate_up() => ("No rate-up banner seen yet".into(), None),
        None => match view.pool {
            Pool::Standard => ("Every standard 5-star".into(), None),
            _ => (format!("{} warp", label(view.pool)), None),
        },
    };
    let mut top = row()
        .w_full()
        .gap(10.0)
        .center_y()
        .child(text(words).font(theme::TITLE).color(t.text).truncate().grow(1.0).min_w(0.0));
    if let Some(left) = left {
        top = top.child(
            row()
                .h(24.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(12.0)
                .fill(Color::BLACK.alpha(if t.dark { 0.35 } else { 0.08 }))
                .child(icon(&icons::CLOCK, 13.0).color(parts::progress(t)))
                .child(text(left).font(COUNT).color(parts::progress(t))),
        );
    }
    line = line.child(top);

    // More than one featured 5 star running: a pick between them
    let choices: Vec<u32> = running.iter().flat_map(|b| b.fives.iter().copied()).collect();
    if choices.len() > 1 {
        let chosen = view.featured.map(|(item, _)| item);
        let picks = choices.into_iter().map(|item| {
            let on = Some(item) == chosen;
            let pool = view.pool;
            let mut pick = el()
                .key(format!("pick-{item}"))
                .size(40.0, 40.0)
                .radius(20.0)
                .clip_rounded(20.0)
                .fill(t.text.alpha(0.08))
                .border(if on { 2.0 } else { 1.0 }, if on { view.accent } else { t.text.alpha(0.12) })
                .opacity(if on { 1.0 } else { 0.55 })
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
    line
}

/// A round marker on a track
fn medallion(size: f32) -> El {
    el().size(size, size).radius(size / 2.0).center()
}

/// The pity, big, with the 50/50, and the gold track to hard pity: a medallion at the pity,
/// marks at soft and hard pity
fn pity_block(view: &View, cx: &Cx<Ui>, t: &Theme) -> El {
    let stats = view.stats;
    let rates = Rates::of(view.pool);
    let hard = rates.hard as f32;
    let gold = parts::progress(t);

    let state = match (view.pool.has_rate_up(), stats.guarantee5) {
        (false, _) => None,
        (true, Some(true)) => Some(("Guaranteed", gold)),
        (true, Some(false)) => Some(("50/50", t.text.alpha(0.75))),
        (true, None) => Some(("50/50?", t.text.alpha(0.5))),
    };
    let number = row()
        .key(format!("pity-{}", view.pool.gacha_type()))
        .gap(14.0)
        .items_end()
        .child(text(stats.pity5.to_string()).font(PITY).color(t.text))
        .child(
            col()
                .pb(10.0)
                .gap(8.0)
                .children(state.map(|(words, tint)| {
                    row()
                        .h(24.0)
                        .px(11.0)
                        .center_y()
                        .radius(12.0)
                        .border(1.0, tint)
                        .child(text(words).font(COUNT).color(tint))
                }))
                .child(text(format!("of {} pity", rates.hard)).font(theme::LABEL).color(t.muted)),
        );

    // The track: gold to the pity, a glowing medallion there, soft and hard pity marked
    let key = format!("pity-track-{}", view.pool.gacha_type());
    let filled = (stats.pity5 as f32 / hard).clamp(0.0, 1.0);
    let soft = (rates.soft_from - 1) as f32 / hard;
    let in_soft = stats.pity5 + 1 >= rates.soft_from;
    let mut track = el().key(key.clone()).w_full().h(30.0);
    if let Some([w, _]) = cx.layout_size(&key) {
        let at = |f: f32| f * (w - 16.0) + 8.0;
        let line = |from: f32, to: f32, color: Color| {
            el().pin(Some(at(from)), Some(13.5), None, None)
                .size((at(to) - at(from)).max(0.0), 3.0)
                .radius(1.5)
                .fill(color)
        };
        let mark = |f: f32, color: Color| {
            medallion(10.0)
                .pin(Some(at(f) - 5.0), Some(10.0), None, None)
                .fill(t.panel)
                .border(2.0, color)
        };
        track = track
            .child(line(0.0, 1.0, t.text.alpha(0.14)))
            .child(line(soft, 1.0, gold.alpha(0.22)))
            .child(
                line(0.0, filled, gold)
                    .transition(Transition::reveal(Edge::Leading).springs(art_springs()))
                    .enter_delay(0.25),
            )
            .child(mark(soft, if in_soft { gold } else { t.text.alpha(0.3) }))
            .child(mark(1.0, t.text.alpha(0.3)))
            .child(
                medallion(18.0)
                    .pin(Some(at(filled) - 9.0), Some(6.0), None, None)
                    .fill(t.panel)
                    .border(2.0, view.accent)
                    .shadow(0.0, 14.0, view.accent.alpha(0.55))
                    .child(medallion(8.0).fill(view.accent)),
            );
    }
    let label_at = |words: String, ink: Color| text(words).font(theme::SMALL).color(ink);
    let marks = row()
        .w_full()
        .child(label_at("0".into(), t.faint))
        .child(el().grow(soft))
        .child(label_at(format!("Soft {}", rates.soft_from), if in_soft { gold } else { t.muted }))
        .child(el().grow(1.0 - soft))
        .child(label_at(format!("Hard {}", rates.hard), t.muted));

    let left = if in_soft {
        "In soft pity: the odds climb every warp".to_string()
    } else {
        format!(
            "{} to soft pity, {} to hard pity",
            rates.soft_from - 1 - stats.pity5,
            rates.hard - stats.pity5
        )
    };
    let four = match stats.guarantee4 {
        Some(true) => "  ·  next 4-star featured",
        Some(false) => "  ·  next 4-star a 50/50",
        None => "",
    };
    col()
        .key("pity-block")
        .w_full()
        .gap(8.0)
        .child(number)
        .child(track)
        .child(marks)
        .child(
            text(format!(
                "{left}  ·  4-star within {}{four}",
                10u32.saturating_sub(stats.pity4).max(1)
            ))
            .font(theme::SMALL)
            .color(t.faint)
            .wrap(),
        )
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
    col()
        .key("odds")
        .w_full()
        .gap(10.0)
        .child(kicker(&caption, t.muted))
        .child(row().w_full().gap(26.0).children(figures))
}

fn figure(value: String, words: &str, t: &Theme) -> El {
    col()
        .gap(2.0)
        .min_w(0.0)
        .child(text(value).font(FIGURE).color(t.text))
        .child(text(words.to_string()).font(theme::SMALL).color(t.muted).truncate().max_w(150.0))
}

/// Every copy of the featured 5 star as a medallion on the gold track: the ones owned gold with a
/// check, the next three with the chance the warps in hand reach them
fn targets(view: &View, t: &Theme) -> Option<El> {
    let (item, _) = view.featured?;
    let rates = Rates::of(view.pool);
    let light_cone = matches!(view.pool, Pool::LightCone | Pool::CollabLightCone);
    let owned = view.roster.map(|r| r.copies(item));
    let have = owned.unwrap_or(0);
    let most: u32 = if light_cone { 5 } else { 7 };
    let gold = parts::progress(t);
    let count = (most.saturating_sub(have)).min(3) as usize;
    let costs = odds::copies(&rates, view.stats.pity5, view.stats.guarantee5 == Some(true), count);
    let level = |copies: u32| {
        if light_cone {
            format!("S{copies}")
        } else {
            format!("E{}", copies - 1)
        }
    };

    let mut track = row().key("copies").w_full().items_start();
    for copy in 1..=most {
        if copy > 1 {
            let done = copy <= have;
            track = track.child(
                el().grow(1.0)
                    .h(3.0)
                    .radius(1.5)
                    .mt(12.5)
                    .mx(-4.0)
                    .fill(if done { gold } else { t.text.alpha(0.14) }),
            );
        }
        let next = copy.checked_sub(have + 1).map(|i| i as usize).filter(|&i| i < costs.len());
        let mark = if copy <= have {
            medallion(28.0)
                .fill(gold)
                .child(icon(&icons::CHECK, 14.0).color(rune::oklch(0.25, 0.04, 70.0)))
        } else {
            let ring = if next == Some(0) {
                medallion(28.0)
                    .fill(t.panel)
                    .border(2.0, view.accent)
                    .shadow(0.0, 12.0, view.accent.alpha(0.45))
            } else {
                medallion(28.0)
                    .fill(t.panel)
                    .border(2.0, t.text.alpha(if next.is_some() { 0.3 } else { 0.12 }))
            };
            ring.child(
                text(level(copy))
                    .font(theme::SMALL)
                    .color(if next.is_some() { t.text } else { t.faint }),
            )
        };
        let under = match (next, view.budget) {
            (Some(i), Some(budget)) => {
                let chance = odds::within(&costs[i], budget.warps);
                text(percent(chance)).font(COUNT).color(if i == 0 { t.text } else { t.muted })
            }
            (Some(i), None) => text(format!("~{:.0}", odds::expected(&costs[i]))).font(COUNT).color(t.muted),
            (None, _) => text(" ").font(COUNT),
        };
        track = track.child(
            col()
                .key(format!("copy-{copy}"))
                .gap(6.0)
                .items_center()
                .help(match next {
                    Some(i) => format!("{}: about {:.0} warps on average", level(copy), odds::expected(&costs[i])),
                    None if copy <= have => format!("{}: owned", level(copy)),
                    None => level(copy),
                })
                .child(mark)
                .child(under),
        );
    }
    let owned_words = match owned {
        Some(0) => "Not owned yet".to_string(),
        Some(c) if c >= most => format!("{} maxed", level(c)),
        Some(c) => format!("You have {}", level(c)),
        None => "Log in to count what you own".to_string(),
    };
    Some(
        col()
            .key("targets")
            .w_full()
            .gap(10.0)
            .child(
                row()
                    .gap(8.0)
                    .child(kicker("Copies", t.muted))
                    .child(text(owned_words).font(theme::SMALL).color(t.faint)),
            )
            .child(track),
    )
}

/// The hero's name over its art, with what the account has of it
fn name_tags(view: &View, item: u32, t: &Theme) -> El {
    let light_cone = matches!(view.pool, Pool::LightCone | Pool::CollabLightCone);
    let kicker_words = if view.featured.is_some() {
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
    // Tags that fade out to the left, towards the art they sit on
    let strip = |content: El, tint: Color| {
        row()
            .h(26.0)
            .pl(28.0)
            .pr(12.0)
            .center_y()
            .linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, tint.alpha(0.0)), (0.3, tint), (1.0, tint)])
            .child(content)
    };
    let shade = Color::BLACK.alpha(0.55);
    rise(
        col()
            .key(format!("name-{item}"))
            .gap(6.0)
            .items_end()
            .max_w(440.0)
            .child(text("★★★★★").font(theme::LABEL).color(parts::progress(t)))
            .child(
                text(view.summary.name(item))
                    .font(NAME)
                    .color(Color::WHITE)
                    .wrap()
                    .align(rune::Align::End)
                    .shadow(0.0, 10.0, Color::BLACK.alpha(0.6)),
            )
            .child(strip(text(kicker_words).font(theme::SMALL).color(Color::WHITE.alpha(0.92)), shade))
            .children(owned.map(|o| strip(text(o).font(theme::SMALL).color(rune::oklch(0.25, 0.04, 70.0)), parts::progress(t)))),
        0.4,
    )
}

/// Every 5 star of the pool as a bar of its pity, oldest on the left, coloured by its 50/50, on a
/// plate of its own
fn history_card(view: &View, width: f32, cx: &Cx<Ui>, t: &Theme) -> El {
    plate("history-card", cx, t)
        .w_full()
        .child(tab("history-card-tab", "5-star history", parts::progress(t), cx, t))
        .child(history_strip(view, width - 44.0, t).px(22.0).pt(12.0).pb(18.0))
}

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
            .fill(outcome_color(five.outcome, t))
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
            .child(el().size(8.0, 8.0).fill(color))
            .child(text(words).font(theme::SMALL).color(t.faint))
    };
    let mut head = row()
        .w_full()
        .gap(14.0)
        .center_y()
        .child(text(format!("{} 5-stars", stats.count5)).font(theme::LABEL).color(t.text))
        .child(text(summary_words.join("  ·  ")).font(theme::SMALL).color(t.muted));
    if view.pool.has_rate_up() {
        head = head
            .child(el().grow(1.0))
            .child(legend(outcome_color(Outcome::Won, t), "won"))
            .child(legend(outcome_color(Outcome::Lost, t), "lost"))
            .child(legend(outcome_color(Outcome::Guaranteed, t), "guaranteed"));
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
                    .pl(10.0)
                    .pr(22.0)
                    .center_y()
                    .linear_gradient(std::f32::consts::FRAC_PI_2, [
                        (0.0, view.accent),
                        (0.65, view.accent),
                        (1.0, view.accent.alpha(0.0)),
                    ])
                    .child(text("Live").font(COUNT).color(t.on_accent)),
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
