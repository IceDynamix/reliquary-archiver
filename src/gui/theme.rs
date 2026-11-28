//! Colors and type, after the game's own interface: hairlines and numbers in the accent, a
//! second color for secondary labels, and cyan for things going well, in a dark or a light
//! variant.
//!
//! Accents are hand picked rather than free: each gem's lightness is tuned for contrast in both
//! variants, it comes with a second color that sets it off, and the error color moves away
//! from it where they'd be confused.

use rune::{Color, TextStyle, oklch};
use rune_kit::{Kit, PaletteSeed};

use super::settings::Accent;

/// The interface face, and the weights the app uses of it
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/BarlowSemiCondensed-Regular.ttf"),
    include_bytes!("../../assets/fonts/BarlowSemiCondensed-Medium.ttf"),
    include_bytes!("../../assets/fonts/BarlowSemiCondensed-SemiBold.ttf"),
];
/// Barlow for the interface, the system's monospace font for logs and addresses
pub const FAMILIES: (&str, &str) = ("Barlow Semi Condensed", "ui-monospace");

pub struct Theme {
    pub kit: Kit,
    pub dark: bool,
    /// The window's background
    pub bg: Color,
    /// Panels on the background
    pub panel: Color,
    /// The fill of plates: nearly opaque, a background image only just shows through
    pub glass: Color,
    /// Insets inside panels
    pub well: Color,
    /// Panel edges and dividers
    pub line: Color,
    /// Marks what matters: titles, numbers, the current step
    pub accent: Color,
    /// The accent faded down, for hairlines that frame rather than mark
    pub accent_line: Color,
    /// Text on an accent fill
    pub on_accent: Color,
    /// Secondary labels, set off against the accent
    pub secondary: Color,
    /// What to press: the fill of primary buttons
    pub primary: Color,
    pub on_primary: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    /// A tile's bone face, and its lower edge where the light falls off
    pub bone: Color,
    pub bone_shade: Color,
    /// The back of this tile set, the deep version of the accent
    pub tile_back: Color,
    /// Gold, for progress and what's done, in every theme as the game keeps it
    pub gold: Color,
    /// Dark ink on gold
    pub on_gold: Color,
}

/// How an accent is made, in OKLCH
struct Gem {
    hue: f32,
    chroma: f32,
    /// Lightness on dark and on light backgrounds
    on_dark: f32,
    on_light: f32,
    /// The second color's hue and chroma
    second: (f32, f32),
    /// The error color's hue, moved off the accent where they'd be confused
    bad_hue: f32,
}

fn gem(accent: Accent) -> Gem {
    const GOLD: (f32, f32) = (82.0, 0.11);
    match accent {
        Accent::Gold => Gem {
            hue: 82.0,
            chroma: 0.105,
            on_dark: 0.83,
            on_light: 0.56,
            second: (302.0, 0.15),
            bad_hue: 25.0,
        },
        Accent::Silver => Gem {
            hue: 250.0,
            chroma: 0.018,
            on_dark: 0.86,
            on_light: 0.47,
            second: (235.0, 0.1),
            bad_hue: 25.0,
        },
        Accent::Ruby => Gem {
            hue: 8.0,
            chroma: 0.16,
            on_dark: 0.71,
            on_light: 0.52,
            second: GOLD,
            bad_hue: 48.0,
        },
        Accent::Amethyst => Gem {
            hue: 305.0,
            chroma: 0.14,
            on_dark: 0.75,
            on_light: 0.5,
            second: GOLD,
            bad_hue: 25.0,
        },
        Accent::Emerald => Gem {
            hue: 158.0,
            chroma: 0.13,
            on_dark: 0.79,
            on_light: 0.5,
            second: GOLD,
            bad_hue: 25.0,
        },
        Accent::Sapphire => Gem {
            hue: 258.0,
            chroma: 0.14,
            on_dark: 0.73,
            on_light: 0.5,
            second: GOLD,
            bad_hue: 25.0,
        },
    }
}

/// How an accent looks on a dark or a light background, for swatches
pub fn accent_color(accent: Accent, dark: bool) -> Color {
    let gem = gem(accent);
    if dark {
        oklch(gem.on_dark, gem.chroma, gem.hue)
    } else {
        oklch(gem.on_light, gem.chroma * 1.1, gem.hue)
    }
}

/// The back of an accent's tile set, and its lower edge. Real mahjong sets come with backs in
/// colors like these: jade, sapphire, amber, slate
pub fn tile_back(accent: Accent, dark: bool) -> (Color, Color) {
    let gem = gem(accent);
    let (back, edge) = if dark { (0.5, 0.41) } else { (0.6, 0.52) };
    let chroma = gem.chroma * if dark { 0.9 } else { 1.0 };
    (oklch(back, chroma, gem.hue), oklch(edge, chroma, gem.hue))
}

impl Theme {
    /// `frosted` lets a background image show through the panels
    pub fn new(dark: bool, accent: Accent, frosted: bool) -> Theme {
        let gem = gem(accent);
        // Neutrals lean a little towards the accent, warm under gold and cool under sapphire
        let tint = |l: f32, c: f32| oklch(l, c * if accent == Accent::Silver { 0.6 } else { 1.0 }, gem.hue);
        let (second_hue, second_chroma) = gem.second;

        let on_gold = oklch(0.25, 0.04, 70.0);

        let theme = if dark {
            let panel = tint(0.185, 0.013);
            let panel = if frosted { panel.alpha(0.86) } else { panel };
            let accent_ink = accent_color(accent, true);
            Theme {
                kit: Kit::neutral(true),
                dark,
                bg: tint(0.155, 0.012),
                panel,
                glass: panel.alpha(0.93),
                well: tint(0.13, 0.01).alpha(if frosted { 0.7 } else { 1.0 }),
                line: tint(0.32, 0.03),
                accent: accent_ink,
                accent_line: accent_ink.alpha(0.42),
                on_accent: tint(0.18, 0.02),
                secondary: oklch(0.68, second_chroma, second_hue),
                primary: tint(0.93, 0.03),
                on_primary: tint(0.2, 0.015),
                text: tint(0.94, 0.018),
                muted: tint(0.76, 0.025),
                faint: tint(0.58, 0.025),
                good: oklch(0.82, 0.11, 200.0),
                warn: oklch(0.8, 0.13, 65.0),
                bad: oklch(0.68, 0.17, gem.bad_hue),
                bone: oklch(0.93, 0.012, 88.0),
                bone_shade: oklch(0.87, 0.016, 85.0),
                tile_back: tile_back(accent, true).0,
                gold: oklch(0.8, 0.12, 82.0),
                on_gold,
            }
        } else {
            let panel = tint(0.99, 0.006);
            let panel = if frosted { panel.alpha(0.88) } else { panel };
            Theme {
                kit: Kit::neutral(false),
                dark,
                bg: tint(0.955, 0.012),
                panel,
                glass: panel.alpha(0.95),
                well: tint(0.935, 0.01).alpha(if frosted { 0.75 } else { 1.0 }),
                line: tint(0.84, 0.025),
                accent: accent_color(accent, false),
                accent_line: oklch(gem.on_light, gem.chroma, gem.hue).alpha(0.5),
                on_accent: tint(0.99, 0.005),
                secondary: oklch(0.54, second_chroma * 1.1, second_hue),
                primary: tint(0.25, 0.03),
                on_primary: tint(0.97, 0.008),
                text: tint(0.22, 0.015),
                muted: tint(0.42, 0.02),
                faint: tint(0.55, 0.02),
                good: oklch(0.55, 0.11, 210.0),
                warn: oklch(0.58, 0.14, 60.0),
                bad: oklch(0.55, 0.19, gem.bad_hue),
                bone: oklch(0.985, 0.008, 88.0),
                bone_shade: oklch(0.93, 0.014, 85.0),
                tile_back: tile_back(accent, false).0,
                gold: oklch(0.68, 0.13, 75.0),
                on_gold,
            }
        };
        theme.with_kit()
    }

    /// The component kit, in this theme's colors
    fn with_kit(mut self) -> Theme {
        let mut kit = Kit::from_palette(PaletteSeed {
            bg: self.bg,
            fg: self.text,
            accent: self.accent,
        });
        kit.radius.sm = 3.0;
        kit.radius.md = 4.0;
        kit.c.layer_01 = self.panel;
        kit.c.scrim = Color::BLACK.alpha(if self.dark { 0.62 } else { 0.32 });
        kit.c.bg_accent = self.accent;
        kit.c.border_focus = self.accent;
        self.kit = kit;
        self
    }
}

pub const HEADLINE: TextStyle = TextStyle::new(25.0, 600.0).lh(30.0).tracking(0.2);
pub const TITLE: TextStyle = TextStyle::new(16.0, 600.0).lh(20.0).tracking(0.3);
pub const BODY: TextStyle = TextStyle::new(14.5, 400.0).lh(20.0).tracking(0.15);
pub const LABEL: TextStyle = TextStyle::new(13.5, 500.0).lh(18.0).tracking(0.3);
pub const SMALL: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0).tracking(0.2);
pub const BUTTON: TextStyle = TextStyle::new(14.0, 600.0).lh(18.0).tracking(0.6);
pub const NUMBER: TextStyle = TextStyle::new(32.0, 500.0).lh(36.0).features("'tnum' 1");
/// The words on a plate's tag
pub const TAG: TextStyle = TextStyle::new(13.0, 650.0).lh(16.0).tracking(0.3);
/// A label over a group, in sentence case: quieter than a title, bolder than [`LABEL`]
pub const GROUP: TextStyle = TextStyle::new(13.0, 600.0).lh(17.0).tracking(0.2);
pub const MONO: TextStyle = TextStyle::new(11.5, 400.0).lh(16.0).mono();
