//! The warp screen. It takes the game's warp screen as its visual language, built from the same
//! pieces as the main window (plates that tear like tickets, tags, medallions on a gold track): the
//! banner's art is the place, not a picture on it, its color is the screen's accent, and one
//! number carries the screen. What's on it, in the order it matters:
//!
//! 1. The pity on a gold track, how near soft and hard pity it is, and whether the next 5 star is a
//!    50/50 or guaranteed.
//! 2. What the warps in hand buy: the chance of a 5 star, and of the featured one, before they run
//!    out, and the most it can take.
//! 3. How far they go towards copies of what's featured, from what the account owns.
//! 4. Every 5 star as a bar of its pity, colored by its 50/50.
//! 5. What was pulled this session, while the game's history catches up.
//!
//! The warps are tickets down a rail on the left: their art, and their pity on the stub. Under each
//! warp's banner is its history, a second page (see [`history`]): scrolling down past the banner
//! turns to it, and back up past its top turns back, while the rail stays.
//!
//! The art comes in as the game's does: the banner's layers slide in from the left at their own
//! speeds and settle, the number and the rest follow a beat behind.

use rune::{
    Color, Cx, Easing, Edge, El, LoopSpec, Spring, SpringPair, Stagger, StaggerFrom, SurfaceFit, TextStyle, Transition, Tx, col, el, icon,
    row, text,
};

use self::framing::{MainLayer, art_transform};
use super::format::{grouped, local, percent};
use super::parts::{
    Pill, check_medallion, chip, kicker, lead_ticket_on, medallion, perforation, pill, ring_button, rule, status_dot, tab, track_link,
};
use super::warps::{SyncStatus, five_keys};
use crate::gui::art::{Art, BannerArt, Layer, Ticket};
use crate::gui::icons;
use crate::gui::state::{Funds, PageScroll, Roster, Ui, WarpPage, snap_wheel, turn_page};
use crate::gui::theme::{self, Theme};
use crate::warps::odds::{self, Rates};
use crate::warps::service::{RunningBanner, Summary};
use crate::warps::stats::{Highlight, Outcome, PoolStats};
use crate::warps::{FOUR_STAR_PITY, Pool, item};

/// Narrower than this, the screen stacks
const NARROW: f32 = 760.0;
/// At least this wide, the rail shows the warps in full
const WIDE: f32 = 1180.0;
mod framing;
mod history;

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
/// A narrow window's padding over its content, the header's height there as the art and the shade
/// under it reckon it, and the gap under the header and under the rail
const NARROW_TOP: f32 = 14.0;
const NARROW_HEADER: f32 = 40.0;
const NARROW_GAP: f32 = 18.0;

/// How far down a narrow window the rail's tickets reach, as the art and the shade under the band
/// reckon it: past the header, and the rail's ticket without the room it leaves around it
fn narrow_rail_foot() -> f32 {
    NARROW_TOP + NARROW_HEADER + NARROW_GAP + COMPACT_TICKET[1]
}

/// How the screen lays out for the window's width
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fit {
    /// Rail, card and art side by side, the rail's tickets in full
    Wide,
    /// The same with a compact rail, for narrower windows
    Medium,
    /// Stacked: the tickets in a row, the art in a band, the card under it
    Narrow,
}

impl Fit {
    fn of(width: f32) -> Fit {
        if width < NARROW {
            Fit::Narrow
        } else if width >= WIDE {
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

    /// Where the room the art is sized and centered in starts: past the card
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

/// A warp's short name, on its ticket and the main window's plate
pub(super) fn label(pool: Pool) -> &'static str {
    match pool {
        Pool::Character => "Character",
        Pool::LightCone => "Light cone",
        Pool::Standard => "Standard",
        Pool::CollabCharacter => "Collab",
        Pool::CollabLightCone => "Collab cone",
        Pool::Departure => "Departure",
    }
}

/// A warp's banner's name, over its warp card
fn banner_title(pool: Pool) -> &'static str {
    match pool {
        Pool::Character => "Character event warp",
        Pool::LightCone => "Light cone event warp",
        Pool::Standard => "Stellar warp",
        Pool::CollabCharacter => "Collab warp",
        Pool::CollabLightCone => "Collab light cone warp",
        Pool::Departure => "Departure warp",
    }
}

/// Each warp's own color, for when its art has none to give
fn pool_color(pool: Pool) -> Color {
    match pool {
        Pool::Character => rune::oklch(0.68, 0.16, 22.0),
        Pool::LightCone => rune::oklch(0.7, 0.12, 250.0),
        Pool::Standard => rune::oklch(0.74, 0.11, 80.0),
        Pool::CollabCharacter | Pool::CollabLightCone => rune::oklch(0.68, 0.14, 310.0),
        Pool::Departure => rune::oklch(0.72, 0.11, 160.0),
    }
}

fn outcome_color(outcome: Outcome, t: &Theme) -> Color {
    match outcome {
        Outcome::Won => t.good,
        Outcome::Guaranteed => t.gold,
        Outcome::Lost => t.bad,
        Outcome::Unknown | Outcome::None => t.text.alpha(0.45),
    }
}

/// How a 5 star's 50/50 went, when there's something to say
fn outcome_words(outcome: Outcome) -> Option<&'static str> {
    match outcome {
        Outcome::Won => Some("won the 50/50"),
        Outcome::Guaranteed => Some("guaranteed"),
        Outcome::Lost => Some("lost the 50/50"),
        Outcome::Unknown | Outcome::None => None,
    }
}

/// [`outcome_words`] to follow a 5 star's pity: `, won the 50/50`, or nothing
fn outcome_suffix(outcome: Outcome) -> String {
    outcome_words(outcome).map_or(String::new(), |words| format!(", {words}"))
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
        let special = pool.pass() == item::SPECIAL_PASS;
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
    let [width, height] = super::window_size(cx);
    let fit = Fit::of(width);

    let Some(summary) = s.warps_summary() else {
        let message = match (&s.warps.unavailable, s.warps_uid()) {
            (Some(error), _) => format!("The warp database couldn't be opened: {error}"),
            (None, None) => "Log in to start tracking warps.".to_string(),
            (None, Some(_)) => "Reading your warps…".to_string(),
        };
        return el().key("warp-screen").w_full().h_full().fill(t.bg).padding(22.0).child(
            col()
                .w_full()
                .gap(18.0)
                .child(header(s, None, Color::WHITE, t))
                .child(text(message).font(theme::BODY).color(t.muted).wrap()),
        );
    };
    let pools: Vec<Pool> = summary.pools.iter().map(|p| p.pool).collect();
    let pool = if pools.contains(&s.warps.pool) {
        s.warps.pool
    } else {
        Pool::Character
    };
    let Some(stats) = summary.pool(pool) else {
        // Nothing pulled yet: how the history fills in, and where from
        return el().key("warp-screen").w_full().h_full().fill(t.bg).child(
            col()
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .px(PAD)
                .pt(20.0)
                .pb(24.0)
                .gap(24.0)
                .child(header(s, None, Color::WHITE, t))
                .child(
                    history::empty(s, summary, cx, t)
                        .max_w(640.0)
                        .scroll(rune::ScrollSpec { stick_bottom: false }),
                ),
        );
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

    // The history page sits under the banner's, and import and export under it: turning to them moves
    // the banner, its art and its name up and out, and the page turned to up into its place. The
    // rail stays
    let turned = s.warps.page != WarpPage::Banner;
    let pager_height = cx.layout_size("warp-pager").map_or((height - 140.0).max(1.0), |[_, h]| h);
    let shift = -pager_height * s.warps.page as usize as f32;

    let mut page = el().key("warp-screen").w_full().h_full().fill(t.bg).clip_rounded(0.0);
    page = page.child(backdrop(s, &view, t));
    page = page.child(
        stage(s, &view, width, height, fit, t)
            .offset(0.0, shift)
            .offset_motion(page_spring()),
    );
    page = page.child(scrims(width, height, fit, t));
    // Deeper shade under the history, which is all words
    page = page.child(
        el().key("history-shade")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .inert()
            .fill(t.bg.alpha(if t.dark { 0.6 } else { 0.7 }))
            .opacity(if turned { 1.0 } else { 0.0 }),
    );

    let content = if fit != Fit::Narrow {
        let pages_width = (width - fit.card_from() - PAD).max(1.0);
        let banner = col()
            .w_full()
            .min_h(pager_height - PAGE_HEADROOM)
            .gap(24.0)
            .child(
                row()
                    .w_full()
                    .grow(1.0)
                    .gap(GAP)
                    .items_start()
                    .child(banner_card(s, &view, &running, fit.card(width), cx, t).w(fit.card(width)))
                    .child(el().grow(1.0))
                    .children(hero.map(|item| col().self_stretch().justify_end().child(name_tags(&view, item, t)))),
            )
            .children(session_row(s, &view, t));
        let middle = row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(GAP)
            .items_start()
            .child(rail(s, summary, pool, fit, t))
            .child(pager(
                s,
                [
                    banner,
                    history::page(s, &view, pages_width, pager_height - PAGE_HEADROOM, cx, t),
                    history::import_export_page(s, &view, cx, t),
                ],
                pager_height,
                shift,
            ));
        col()
            .key("warp-content")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .px(PAD)
            .pt(20.0)
            .pb(24.0)
            .gap(24.0)
            .child(header(s, Some(pool), accent, t))
            .child(middle)
    } else {
        let band = narrow_band(height);
        let banner = col()
            .w_full()
            .pb(20.0)
            .gap(18.0)
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
                    .child(banner_card(s, &view, &running, width - 32.0, cx, t).w_full())
                    .children(session_row(s, &view, t)),
            );
        let history = history::page(s, &view, width - 32.0, pager_height - PAGE_HEADROOM, cx, t)
            .px(16.0)
            .pt(4.0);
        let import_export = history::import_export_page(s, &view, cx, t).px(16.0).pt(4.0);
        col()
            .key("warp-content-narrow")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .pt(NARROW_TOP)
            .gap(NARROW_GAP)
            .child(header(s, Some(pool), accent, t).px(16.0))
            .child(rail(s, summary, pool, fit, t))
            .child(pager(s, [banner, history, import_export], pager_height, shift))
    };
    page.child(content)
}

/// Room at the top of each page for what rises over its top edge, which the page would clip
const PAGE_HEADROOM: f32 = 24.0;

/// The spring the pages turn on, and the art and its name go with them
fn page_spring() -> Spring {
    Spring::smooth(0.5, 0.0)
}

/// The banner's page over the history's over import and export's, each [`WarpPage`] in `pages`, showing
/// the one turned to. Each scrolls itself when it's taller than the room, and a scroll that runs
/// on past its end turns the page (see [`snap_wheel`])
fn pager(s: &Ui, pages: [El; WarpPage::ALL.len()], height: f32, shift: f32) -> El {
    let page = |which: WarpPage, content: El| {
        let key = match which {
            WarpPage::Banner => "warp-page-banner",
            WarpPage::History => "warp-page-history",
            WarpPage::ImportExport => "warp-page-import-export",
        };
        let scroll = s.warps.page_scroll[which as usize].clone();
        scrolling(key, content, &scroll)
            .h(height)
            .pt(PAGE_HEADROOM)
            .on_wheel(move |s: &mut Ui, wheel: rune::WheelEvent, tx: &mut Tx| {
                // The page out of sight takes nothing, it shouldn't be under the pointer anyway
                if s.warps.page != which {
                    return true;
                }
                let scroll = PageScroll::of(scroll.get());
                snap_wheel(s, wheel.into(), scroll, tx.clock())
            })
    };
    // Reaches up over the gap above it, so what rises over the top of a page (the card's tab) shows
    el().key("warp-pager")
        .grow(1.0)
        .min_w(0.0)
        .self_stretch()
        .mt(-PAGE_HEADROOM)
        .clip_rounded(0.0)
        .child(
            col()
                .key("warp-pages")
                .pin(Some(0.0), Some(0.0), Some(0.0), None)
                .offset(0.0, shift)
                .offset_motion(page_spring())
                .children(WarpPage::ALL.into_iter().zip(pages).map(|(which, content)| page(which, content))),
        )
}

/// `content` in a column that scrolls itself, with `scroll` kept up with where it's scrolled to
fn scrolling(key: &str, content: El, scroll: &rune::Live<[f32; 2]>) -> El {
    let reported = scroll.clone();
    col()
        .key(key)
        .w_full()
        .child(content)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .scroll_edge_fade(16.0)
        .on_scroll_offset(move |at: rune::ScrollPos| {
            reported.set([at.offset[1], at.max[1]]);
        })
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
            // Without the banner's own, a wash of its color
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

/// The banner's art: its layers placed as the game places them (see [`art_transform`]), sliding in
/// at their own speeds. Without them, the hero's portrait on the right
fn stage(s: &Ui, view: &View, width: f32, height: f32, fit: Fit, t: &Theme) -> El {
    let Some(item) = view.hero else { return el() };
    let mut stage = el().key("warp-stage").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).inert();

    if let Some(art) = view.art.as_ref().filter(|art| art.placed())
        && let Some(main) = art.layer(Layer::Main).and_then(MainLayer::of)
    {
        let (k, ox, oy) = art_transform(&main, [width, height], fit);
        let order = [Layer::Back, Layer::Main, Layer::Front, Layer::Front2];
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
            let travel = LAYER_TRAVEL[i.min(LAYER_TRAVEL.len() - 1)] * k * 1.6;
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
    if fit != Fit::Narrow {
        let room = fit.stage_from(width);
        let side = (height - ART_TOP - PAD).min(width - room - PAD).max(240.0);
        let x = (room + width - PAD) / 2.0 - side / 2.0;
        let y = ART_TOP + (height - ART_TOP - PAD - side) / 2.0;
        stage.child(picture.pin(Some(x), Some(y), None, None).size(side, side))
    } else {
        let side = narrow_band(height) * 1.25;
        stage.child(
            picture
                .pin(Some((width - side) / 2.0), Some(narrow_rail_foot()), None, None)
                .size(side, side),
        )
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
        layer = layer.child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), None)
                .h(narrow_rail_foot() + 40.0)
                .linear_gradient(std::f32::consts::PI, [
                    (0.0, ground.alpha(0.9)),
                    (0.75, ground.alpha(0.7)),
                    (1.0, ground.alpha(0.0)),
                ]),
        );
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
        .children(selected.map(|_| {
            let (words, glyph) = match s.warps.page {
                WarpPage::Banner => ("History", &icons::LIST),
                WarpPage::History | WarpPage::ImportExport => ("Banner", &icons::WARP),
            };
            pill("warp-history", words, Pill::Secondary)
                .glyph(glyph)
                .build(t, |s: &mut Ui, _: &mut Tx| turn_page(s))
        }))
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
        row()
            .key(format!("rail-{}", pool.gacha_type()))
            .role("tab")
            .label(label(pool))
            .help(format!("{} warp, pity {} of {}", label(pool), stats.pity5, pool.hard_pity()))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .size(w, h)
            .on_press_tx(move |s: &mut Ui, tx: &mut Tx| crate::gui::state::pick_warp(s, pool, tx))
            .child(ticket_figures(stats, chosen, full, t))
            .child(rail_ticket(s, summary, stats, chosen, h, t))
    });
    if fit == Fit::Narrow {
        row()
            .key("warp-rail")
            .w_full()
            .px(16.0)
            // Room above for the selected ticket's head and below for its frame, which the
            // scrolling row would clip
            .pt(HEADROOM * COMPACT_TICKET[1] / SPRITE[1])
            .pb(SELECTED_BELOW * COMPACT_TICKET[1] / SPRITE[1])
            .gap(14.0)
            .children(tickets)
            .scroll_x()
            // Over the pages, whose headroom reaches up under it and which slide past it as they
            // turn
            .paint_order(1)
    } else {
        col().key("warp-rail").pt(8.0).gap(14.0).children(tickets)
    }
}

/// An image's size in pixels
fn image_size(image: &rune::Image) -> Option<[f32; 2]> {
    image.size().map(|[w, h]| [w as f32, h as f32])
}

/// A warp's ticket on the rail, `h` tall: the game's own where there's one, else one drawn like it
fn rail_ticket(s: &Ui, summary: &Summary, stats: &PoolStats, chosen: bool, h: f32, t: &Theme) -> El {
    let pool = stats.pool;
    // The picked 5 star's banner when several run, so the ticket follows the pick
    let ticket = Ticket::of(summary, pool, s.warps.featured.get(&pool).copied());
    // Both of a ticket's pictures (preloaded when the warps come in, see `Ui::preload_warp_art`)
    let picture = |selected: bool| ticket.and_then(|ticket| s.art.as_ref()?.get(Art::Ticket(ticket, selected)));
    let (plain, framed) = (picture(false), picture(true));
    let plain_size = plain.as_ref().and_then(image_size).unwrap_or(SPRITE);
    // The rail's height over the ticket's, and its width at that
    let k = h / plain_size[1];
    let size = [plain_size[0] * k, h];
    let key = format!("ticket-{}", pool.gacha_type());
    let sprite = if chosen { framed.clone() } else { plain };
    let Some(image) = sprite else {
        let featured = match ticket {
            Some(Ticket::Banner(item)) => Some(item),
            _ => None,
        };
        return drawn_ticket(s, stats, featured, chosen, size, t);
    };
    // Each ticket's pictures are keyed by whose ticket it is: a pick that changes the ticket swaps
    // it for another, the old fading out and the new in, rather than morphing one into the other's
    // size
    let picture_key = match ticket {
        Some(Ticket::Banner(item)) => format!("{key}-{item}"),
        other => format!("{key}-{other:?}"),
    };
    // Only for the moment after the ticket's picked: a layout that rebuilds the rail later mounts
    // the shine afresh, which would sweep again
    let fresh = s
        .warps
        .picked
        .is_some_and(|(at, when)| at == pool && when.elapsed() < crate::gui::state::SHINE_FOR);
    let shine = (chosen && fresh)
        .then_some(framed)
        .flatten()
        .and_then(|image| image_size(&image).map(|at| (image, at)))
        .map(|(white, own)| ticket_shine(&key, &white, own, plain_size, k));
    ticket_face(&picture_key, &image, chosen, plain_size, k, size)
        // The shine's holder is always there and only the shine comes and goes: mounted with its
        // holder, it entered with it and never swept
        .child(
            el().key(format!("{key}-shine-holder"))
                .pin(Some(0.0), Some(0.0), None, None)
                .size(size[0], size[1])
                .inert()
                .transition(Transition::none())
                .children(shine),
        )
}

/// The selected ticket's picture fades in on these, the plain one's a little quicker
fn selected_springs() -> SpringPair {
    SpringPair::new(Spring::smooth(0.35, 0.0), Spring::smooth(0.18, 0.0))
}
fn plain_springs() -> SpringPair {
    SpringPair::new(Spring::smooth(0.3, 0.0), Spring::smooth(0.18, 0.0))
}

/// Where the selected sprite of a ticket `plain` large is drawn over it, `k` points to its pixel,
/// when it's `own` large: its ticket lies exactly over the plain one's, its frame and the head
/// above it spilling out. `[x, y, w, h]`
fn selected_rect(own: [f32; 2], plain: [f32; 2], k: f32) -> [f32; 4] {
    let above = own[1] - SELECTED_BELOW - plain[1];
    [SELECTED_LEFT * k, -above * k, own[0] * k, own[1] * k]
}

/// The game's ticket `image`, `size` on the rail: the plain one, or the selected one drawn at the
/// same scale (`k` points to a pixel) over where the plain one, `plain` pixels large, would be
fn ticket_face(key: &str, image: &rune::Image, chosen: bool, plain: [f32; 2], k: f32, size: [f32; 2]) -> El {
    let picture = el().pixels_fit(image.pixels(), SurfaceFit::Fill);
    let picture = if chosen {
        let own = image_size(image).unwrap_or([SPRITE[0] + 14.0, SPRITE[1] + SELECTED_BELOW]);
        let [x, y, w, h] = selected_rect(own, plain, k);
        picture
            .key(format!("{key}-selected"))
            .pin(Some(x), Some(y), None, None)
            .size(w, h)
            .transition(Transition::opacity().springs(selected_springs()))
    } else {
        picture
            .key(format!("{key}-plain"))
            .pin(Some(0.0), Some(0.0), None, None)
            .size(size[0], size[1])
            .opacity(0.8)
            .transition(Transition::opacity().springs(plain_springs()))
    };
    el().size(size[0], size[1]).child(picture)
}

/// Light catching the selected ticket each time it's picked: the ticket's picture `white`, `own`
/// pixels large, drawn in white (`pixels_tint`, so only the ticket's own pixels light), seen
/// through a window that sweeps across it.
///
/// It's Rune's shimmer, slanted: a frame turned by `SLANT`, centered on the ticket, holds a window
/// that slides across it once while the white inside the window slides back as far. The window's
/// straight edges come out slanted; the white, turned back about its center (the ticket's, as the
/// slides cancel), sits upright on the ticket. It mounts afresh on each pick, so each pick sweeps;
/// played out it rests past the ticket, its window empty
fn ticket_shine(key: &str, white: &rune::Image, own: [f32; 2], plain: [f32; 2], k: f32) -> El {
    const SLANT: f32 = 0.35;
    const SWEEP: f64 = 0.5;
    let [x, y, w, h] = selected_rect(own, plain, k);
    let band = w * 0.52;
    // The frame reaches past the ticket so that turned it still covers it
    let reach = (w.max(h)) * 0.5;
    let (fw, fh) = (w + reach * 2.0, h + reach * 2.0);
    let travel = fw + band;
    // A slide's offsets are fractions of the element's own size: the window is a band wide, the
    // white inside it the whole ticket
    let slide = |to: f32| LoopSpec {
        delay: 0.06,
        ..LoopSpec::slide(SWEEP, [0.0, 0.0], [to, 0.0]).easing(Easing::LINEAR)
    };
    // Layout changes land at once inside it: the window and the white in it are placed apart, and
    // on their own springs they'd part for a few frames
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
                .loop_times(1)
                .child(
                    el().pin(Some(reach + band), Some(reach), None, None)
                        .size(w, h)
                        .rotate(-SLANT)
                        .pixels_fit(white.pixels(), SurfaceFit::Fill)
                        .pixels_tint(Color::WHITE)
                        .opacity(0.7)
                        .transition(Transition::none())
                        .looping(slide(-travel / w))
                        .loop_times(1),
                ),
        )
}

/// A ticket drawn like the game's, `size` on the rail: square cut, notched where the stub tears,
/// with the `featured` 5 star's art, or the last one pulled
fn drawn_ticket(s: &Ui, stats: &PoolStats, featured: Option<u32>, chosen: bool, size: [f32; 2], t: &Theme) -> El {
    let [w, h] = size;
    let (fill, ink) = if chosen { (t.primary, t.on_primary) } else { (t.glass, t.text) };
    let stub = w * 0.16;
    let art = featured
        .or_else(|| stats.fives.first().map(|f| f.item_id))
        .and_then(|item| s.art.as_ref()?.get(Art::Preview(item)));
    let mut face = row()
        .size(w, h)
        .fill(fill)
        .cutout_circles([([stub, 0.0], 4.5, 5.0), ([stub, h], 4.5, 5.0)]);
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
        text(label(stats.pool))
            .font(theme::LABEL)
            .color(ink)
            .pl(stub + 10.0)
            .self_center()
            .truncate(),
    )
}

/// A ticket's pity, and its name when it's shown in `full`, left of it and set against it
fn ticket_figures(stats: &PoolStats, chosen: bool, full: bool, t: &Theme) -> El {
    let pool = stats.pool;
    col()
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
        }))
}

/// Warps in hand, broken down on hover
fn budget_chip(s: &Ui, budget: Budget, t: &Theme) -> El {
    let pass = if budget.special { item::SPECIAL_PASS } else { item::PASS };
    let glyph = match s.art.as_ref().and_then(|art| art.get(Art::Item(pass))) {
        Some(image) => el().size(22.0, 22.0).pixels_fit(image.pixels(), SurfaceFit::Fit),
        None => el().size(6.0, 6.0),
    };
    chip(t)
        .key("warp-budget")
        .pl(8.0)
        .pr(14.0)
        .gap(6.0)
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
    let status = SyncStatus::of(s);
    chip(t)
        .key("warp-status")
        .role("button")
        .label("Sync")
        .px(14.0)
        .gap(8.0)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .hover(t.text.alpha(0.06))
        .help(format!("{}. Press to sync", status.long()))
        .on_press(|s: &mut Ui| s.sync_warps())
        .child(status_dot(status.tone().ink(t), s.warps.busy.is_some(), 8.0, t))
        .child(text(status.short()).font(theme::SMALL).color(t.muted).truncate().max_w(170.0))
}

/// The banner's ticket: what's featured and the pick between the running banners, the pity on its
/// track, and past the tear what the warps in hand buy and how far towards copies
fn banner_card(s: &Ui, view: &View, running: &[&RunningBanner], width: f32, cx: &Cx<Ui>, t: &Theme) -> El {
    let surface = view.featured.and_then(|(item, _)| card_surface(s, item, t));
    let card = lead_ticket_on("warp-card", &["warp-card-tear", "warp-card-tear-history"], surface, cx, t)
        .child(tab("warp-card-tab", banner_title(view.pool), view.accent, t))
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
        )
        .child(perforation("warp-card-tear-history", cx, t))
        .child(
            history_strip(view, width - 44.0, t)
                .px(22.0)
                .pt(12.0)
                .pb(16.0)
                .cursor(rune::Cursor::Pointer)
                .help("The whole history")
                .on_press(|s: &mut Ui| turn_page(s)),
        );
    rise(card, 0.05)
}

/// The featured banner's own panel under the warp card, as the game puts it behind the art: its
/// colors rising from the bottom, faint enough to read over, and its motif tiled across
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
                Color {
                    r: under.r + (over.r - under.r) * k,
                    g: under.g + (over.g - under.g) * k,
                    b: under.b + (over.b - under.b) * k,
                    a: 1.0,
                }
            }
            None => fill.end,
        };
        let strength = if t.dark { 0.24 } else { 0.2 };
        layer = layer.linear_gradient(std::f32::consts::PI, [
            (0.0, fill.start.alpha(0.0)),
            (0.45, fill.end.alpha(strength * 0.35)),
            (1.0, bottom.alpha(strength)),
        ]);
    }
    if let Some(pattern) = &surface.pattern
        && let Some(image) = art.get(Art::Pattern(item))
    {
        layer = layer.child(
            el().key("warp-card-motif")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .pixels_tile(image.pixels(), pattern.tile)
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
                .child(icon(&icons::CLOCK, 13.0).color(t.gold))
                .child(text(left).font(COUNT).color(t.gold)),
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

/// The pity, big, with the 50/50, and the gold track to hard pity: a medallion at the pity,
/// marks at soft and hard pity
fn pity_block(view: &View, cx: &Cx<Ui>, t: &Theme) -> El {
    let stats = view.stats;
    let rates = Rates::of(view.pool);
    let hard = rates.hard as f32;
    let gold = t.gold;

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
        "In soft pity".to_string()
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
                FOUR_STAR_PITY.saturating_sub(stats.pity4).max(1)
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
        None => "Average warps needed.".to_string(),
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
    const SIZE: f32 = 28.0;
    let rates = Rates::of(view.pool);
    let owned = view.roster.map(|r| r.copies(item));
    let have = owned.unwrap_or(0);
    let most = view.pool.max_copies();
    let count = (most.saturating_sub(have)).min(3) as usize;
    let costs = odds::copies(&rates, view.stats.pity5, view.stats.guarantee5 == Some(true), count);
    let level = |copies: u32| view.pool.copy_level(copies);

    let mut track = row().key("copies").w_full().items_start();
    for copy in 1..=most {
        if copy > 1 {
            track = track.child(track_link(copy <= have, SIZE, t).mx(-4.0));
        }
        let next = copy.checked_sub(have + 1).map(|i| i as usize).filter(|&i| i < costs.len());
        let mark = if copy <= have {
            check_medallion(SIZE, t)
        } else {
            let ring = if next == Some(0) {
                medallion(SIZE)
                    .fill(t.panel)
                    .border(2.0, view.accent)
                    .shadow(0.0, 12.0, view.accent.alpha(0.45))
            } else {
                medallion(SIZE)
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
        None => "Log in to see owned copies".to_string(),
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
    let kicker_words = if view.featured.is_some() {
        "Featured 5-star".to_string()
    } else {
        view.stats.fives.first().map_or("5-star".into(), |f| {
            format!("Latest 5-star  ·  pity {}{}", f.pity, outcome_suffix(f.outcome))
        })
    };
    let owned = view.roster.map(|r| r.copies(item)).map(|c| match c {
        0 => "Not owned".to_string(),
        c => format!("Owned  ·  {}", view.pool.copy_level(c)),
    });
    // Plain strips of color for the name and the tags under it, so they read over the art
    let strip = |content: El, tint: Color| row().min_h(26.0).px(12.0).center_y().fill(tint).child(content);
    let shade = Color::BLACK.alpha(0.55);
    rise(
        col()
            .key(format!("name-{item}"))
            .gap(6.0)
            .items_end()
            .max_w(440.0)
            .child(
                row()
                    .gap(-3.0)
                    .children((0..5).map(|_| icon(&icons::WARP_FILLED, 20.0).color(t.gold))),
            )
            .child(
                strip(
                    text(view.summary.name(item))
                        .font(NAME)
                        .color(Color::WHITE)
                        .wrap()
                        .align(rune::Align::End),
                    shade,
                )
                .py(4.0),
            )
            .child(strip(text(kicker_words).font(theme::SMALL).color(Color::WHITE.alpha(0.92)), shade))
            .children(owned.map(|o| strip(text(o).font(theme::SMALL).color(t.on_gold), t.gold))),
        0.4,
    )
}

/// Every 5 star of the pool as a bar of its pity, oldest on the left, colored by its 50/50: the
/// warp card's last section, in its palette, the bars soft enough not to outshout the art
fn history_strip(view: &View, width: f32, t: &Theme) -> El {
    let stats = view.stats;
    let rates = Rates::of(view.pool);
    let hard = rates.hard as f32;
    let (bar_w, gap, tall) = (6.0, 3.0, 44.0);
    let fit = ((width + gap) / (bar_w + gap)).floor().max(1.0) as usize;
    let shown: Vec<&Highlight> = stats.fives.iter().take(fit).collect();
    let n = shown.len();
    let stagger = Stagger::new(0.012).from(StaggerFrom::Last);
    let keys = five_keys(shown.iter().copied());
    let bars = shown.iter().zip(keys).rev().enumerate().map(|(i, (five, key))| {
        let when = local(five.time, "%Y-%m-%d");
        let outcome = outcome_suffix(five.outcome);
        el().key(key)
            .w(bar_w)
            .h(4.0 + (tall - 4.0) * (five.pity as f32 / hard).min(1.0))
            .fill(outcome_color(five.outcome, t).alpha(if five.confirmed { 0.78 } else { 0.4 }))
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
    let summary_words: Vec<String> = std::iter::once(format!("{} 5-stars", stats.count5)).chain(summary_words).collect();

    let legend = |color: Color, words: &'static str| {
        row()
            .gap(5.0)
            .center_y()
            .child(el().size(6.0, 6.0).radius(3.0).fill(color.alpha(0.78)))
            .child(text(words).font(theme::SMALL).color(t.faint))
    };
    let head = row().w_full().gap(8.0).child(kicker("5-star history", t.muted)).child(
        text(summary_words.join("  ·  "))
            .font(theme::SMALL)
            .color(t.faint)
            .truncate()
            .min_w(0.0),
    );
    // Soft pity, faint across the bars as on the track above
    let soft = (rates.soft_from as f32 / hard).min(1.0);
    let guide = el()
        .pin(Some(0.0), None, Some(0.0), Some(4.0 + (tall - 4.0) * soft))
        .h(1.0)
        .inert()
        .fill(t.gold.alpha(0.22));
    let body = if n == 0 {
        row()
            .h(tall)
            .center_y()
            .child(text("No 5-stars here yet").font(theme::SMALL).color(t.faint))
    } else {
        row()
            .key(format!("strip-{}", view.pool.gacha_type()))
            .w_full()
            .h(tall)
            .gap(gap)
            .items_end()
            .child(guide)
            .children(bars)
    };
    let mut col_ = col().key("history-strip").w_full().gap(8.0).child(head).child(body);
    if view.pool.has_rate_up() && n > 0 {
        col_ = col_.child(
            row()
                .gap(12.0)
                .child(legend(outcome_color(Outcome::Won, t), "won"))
                .child(legend(outcome_color(Outcome::Lost, t), "lost"))
                .child(legend(outcome_color(Outcome::Guaranteed, t), "guaranteed")),
        );
    }
    col_
}

/// What was pulled while the archiver watched, this run
fn session_row(s: &Ui, view: &View, t: &Theme) -> Option<El> {
    let mut warps = 0usize;
    let mut fives = String::new();
    for recent in view.summary.session(s.warps.session_start) {
        warps += 1;
        if recent.pull.rarity == 5 {
            fives.push_str(&format!("  ·  {} at {}", view.summary.name(recent.pull.item_id), recent.pity));
            if let Some(words) = outcome_words(recent.outcome) {
                fives.push_str(&format!(" ({words})"));
            }
        }
    }
    if warps == 0 {
        return None;
    }
    let words = format!(
        "This session  ·  {} warp{}{fives}",
        grouped(warps),
        if warps == 1 { "" } else { "s" }
    );
    let waiting = view.summary.unconfirmed;
    Some(rise(
        row()
            .key("session")
            .w_full()
            .gap(10.0)
            .center_y()
            .child(live_badge(view.accent))
            .child(text(words).font(theme::SMALL).color(t.text).truncate().grow(1.0).min_w(0.0))
            .children((waiting > 0).then(|| {
                text(format!("{waiting} waiting for the game's history"))
                    .font(theme::SMALL)
                    .color(t.faint)
            })),
        0.35,
    ))
}

/// A small outlined LIVE mark, for warps seen as they happened
pub(super) fn live_badge(tint: Color) -> El {
    el().px(5.0)
        .h(16.0)
        .center()
        .radius(3.0)
        .border(1.0, tint)
        .child(text("LIVE").font(theme::KICKER).color(tint))
}

// ---- words and numbers -------------------------------------------------------------------------

fn time_left(end: i64) -> String {
    let left = (end - jiff::Timestamp::now().as_second()).max(0);
    let (days, hours) = (left / 86400, left % 86400 / 3600);
    if days > 0 {
        format!("{days}d {hours}h left")
    } else {
        format!("{hours}h {}m left", left % 3600 / 60)
    }
}
