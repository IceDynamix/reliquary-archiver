use raxis::gfx::color::Oklch;
use raxis::layout::model::{Color, DropShadow, TextShadow};
use raxis::widgets::text::Text;

// Spacing constants
pub const PAD_SM: f32 = 4.0;
pub const PAD_MD: f32 = 8.0;
pub const PAD_LG: f32 = 16.0;

pub const SPACE_SM: f32 = 4.0;
pub const SPACE_MD: f32 = 8.0;
pub const SPACE_LG: f32 = 16.0;

pub const BORDER_RADIUS: f32 = 8.0;
pub const BORDER_RADIUS_SM: f32 = 4.0;

// Color constants
pub const CARD_BACKGROUND: Color = Color::from_oklch(Oklch::deg(0.17, 0.006, 285.885, 0.6));
pub const SCROLLBAR_THUMB_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.6));
pub const SCROLLBAR_TRACK_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.2));

pub const OPAQUE_CARD_BACKGROUND: Color = Color::from_oklch(Oklch::deg(0.17, 0.006, 285.885, 1.0));

pub const TEXT_MUTED: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.6,
};
pub const TEXT_COLOR: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.9,
};
pub const TEXT_ON_LIGHT_COLOR: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.9,
};
pub const BORDER_COLOR: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.1,
};
pub const DANGER_COLOR: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.2,
    a: 0.6,
};
pub const SUCCESS_COLOR: Color = Color {
    r: 0.2,
    g: 0.8,
    b: 0.2,
    a: 0.6,
};
pub const PRIMARY_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.6));
pub const SELECTION_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.3));
pub const SELECTION_HOVER_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.4));

pub const SHADOW_XS: DropShadow = DropShadow {
    offset_y: 1.0,
    blur_radius: 2.0,
    color: Color::from_hex(0x0000000D),
    ..DropShadow::default()
};

pub const SHADOW_SM: DropShadow = DropShadow {
    offset_y: 1.0,
    blur_radius: 3.0,
    color: Color::from_hex(0x0000001A),
    ..DropShadow::default()
};

pub const SHADOW_XL: DropShadow = DropShadow {
    offset_y: 20.0,
    blur_radius: 25.0,
    spread_radius: -5.0,
    color: Color::from_hex(0x0000008A),
    ..DropShadow::default()
};

pub const TEXT_SHADOW_4PX: TextShadow = TextShadow {
    offset_x: 0.0,
    offset_y: 0.0,
    blur_radius: 4.0,
    color: Color::BLACK,
};

pub const TEXT_SHADOW_2PX: TextShadow = TextShadow {
    offset_x: 0.0,
    offset_y: 0.0,
    blur_radius: 2.0,
    color: Color::BLACK,
};

// Helper function to conditionally apply text shadow to Text widgets
pub fn maybe_text_shadow(text: Text, enabled: bool) -> Text {
    if enabled {
        text.with_text_shadows(vec![
            TextShadow {
                offset_x: -1.0,
                offset_y: -1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
            TextShadow {
                offset_x: 1.0,
                offset_y: 1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
            TextShadow {
                offset_x: -1.0,
                offset_y: 1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
            TextShadow {
                offset_x: 1.0,
                offset_y: -1.0,
                blur_radius: 1.0,
                color: Color::BLACK,
            },
        ])
    } else {
        text
    }
}
