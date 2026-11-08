//! Colours and type. A quiet neutral base with one warm accent, the gold of a relic's rarity, which
//! only marks what the user should look at or press next.

use rune::{Color, TextStyle, oklch};
use rune_kit::{Kit, PaletteSeed};

pub struct Theme {
    pub kit: Kit,
    /// The window's background
    pub bg: Color,
    /// Cards sitting on the background
    pub card: Color,
    /// Insets inside cards, like the inventory counts
    pub well: Color,
    /// A background image shows through the cards, which blur it so text stays readable
    pub frosted: bool,
    pub card_border: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    /// A tint of the accent for backgrounds
    pub accent_soft: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
}

impl Theme {
    pub fn new(dark: bool, frosted: bool) -> Theme {
        let (bg, fg, accent) = if dark {
            (oklch(0.165, 0.008, 270.0), oklch(0.95, 0.005, 270.0), oklch(0.82, 0.12, 78.0))
        } else {
            (oklch(0.975, 0.004, 85.0), oklch(0.22, 0.012, 270.0), oklch(0.6, 0.13, 68.0))
        };
        let mut kit = Kit::from_palette(PaletteSeed { bg, fg, accent });
        kit.radius.sm = 6.0;
        kit.radius.md = 8.0;
        let card = if dark { oklch(0.205, 0.01, 270.0) } else { oklch(1.0, 0.0, 0.0) };
        // Dialogs sit on the opaque card surface, over a scrim that sets them apart from the window
        kit.c.layer_01 = card;
        kit.c.scrim = Color::BLACK.alpha(if dark { 0.55 } else { 0.25 });

        Theme {
            card: if frosted {
                card.alpha(if dark { 0.72 } else { 0.78 })
            } else {
                card
            },
            well: if frosted { bg.alpha(0.6) } else { bg },
            frosted,
            card_border: if dark { oklch(0.27, 0.012, 270.0) } else { oklch(0.9, 0.006, 85.0) },
            text: fg,
            muted: kit.c.text_muted,
            faint: kit.c.text_faint,
            accent_soft: if dark { oklch(0.26, 0.04, 78.0) } else { oklch(0.94, 0.04, 80.0) },
            good: if dark { oklch(0.76, 0.14, 152.0) } else { oklch(0.55, 0.14, 152.0) },
            warn: if dark { oklch(0.8, 0.13, 70.0) } else { oklch(0.58, 0.14, 60.0) },
            bad: if dark { oklch(0.7, 0.17, 25.0) } else { oklch(0.55, 0.19, 27.0) },
            accent,
            bg,
            kit,
        }
    }
}

pub const HEADLINE: TextStyle = TextStyle::new(20.0, 600.0).lh(26.0).tracking(-0.2);
pub const TITLE: TextStyle = TextStyle::new(13.5, 600.0).lh(18.0);
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(19.0);
pub const LABEL: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
pub const SMALL: TextStyle = TextStyle::new(11.5, 400.0).lh(16.0);
pub const NUMBER: TextStyle = TextStyle::new(22.0, 600.0).lh(28.0).tracking(-0.4).features("'tnum' 1");
pub const MONO: TextStyle = TextStyle::new(11.5, 400.0).lh(16.0).mono();
