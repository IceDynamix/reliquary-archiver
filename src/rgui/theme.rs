//! UI theming constants and helper functions.
//!
//! This module defines the visual design system for the application including:
//! - Spacing and padding values
//! - Color palette
//! - Drop shadow and text shadow presets
//! - Helper functions for conditional styling

use raxis::gfx::color::Oklch;
use raxis::layout::model::{Color, DropShadow, TextShadow};
use raxis::widgets::text::Text;

// ============================================================================
// Spacing Constants
// ============================================================================
/// Small padding (4px)
pub const PAD_SM: f32 = 4.0;
/// Medium padding (8px)
pub const PAD_MD: f32 = 8.0;
/// Large padding (16px)
pub const PAD_LG: f32 = 16.0;

/// Small spacing/gap (4px)
pub const SPACE_SM: f32 = 4.0;
/// Medium spacing/gap (8px)
pub const SPACE_MD: f32 = 8.0;
/// Large spacing/gap (16px)
pub const SPACE_LG: f32 = 16.0;

/// Standard border radius (8px)
pub const BORDER_RADIUS: f32 = 8.0;
/// Small border radius (4px)
pub const BORDER_RADIUS_SM: f32 = 4.0;

// ============================================================================
// Color Palette
// ============================================================================
/// Semi-transparent background for cards and panels
pub const CARD_BACKGROUND: Color = Color::from_oklch(Oklch::deg(0.17, 0.006, 285.885, 0.6));
/// Scrollbar thumb color
pub const SCROLLBAR_THUMB_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.6));
/// Scrollbar track color
pub const SCROLLBAR_TRACK_COLOR: Color = Color::from_oklch(Oklch::deg(0.47, 0.006, 285.885, 0.2));
/// Fully opaque card background (for modals)
pub const OPAQUE_CARD_BACKGROUND: Color = Color::from_oklch(Oklch::deg(0.17, 0.006, 285.885, 1.0));

/// Muted/secondary text color (60% white)
pub const TEXT_MUTED: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.6,
};
/// Primary text color (90% white)
pub const TEXT_COLOR: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.9,
};
/// Text color for light backgrounds (90% black)
pub const TEXT_ON_LIGHT_COLOR: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.9,
};
/// Subtle border color (10% white)
pub const BORDER_COLOR: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.1,
};
/// Color for error states and warnings (red)
pub const DANGER_COLOR: Color = Color {
    r: 0.9,
    g: 0.2,
    b: 0.2,
    a: 0.6,
};
/// Color for success states (green)
pub const SUCCESS_COLOR: Color = Color {
    r: 0.2,
    g: 0.8,
    b: 0.2,
    a: 0.6,
};
/// Primary brand color (purple)
pub const PRIMARY_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.6));
/// Selection highlight background
pub const SELECTION_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.3));
/// Selection highlight on hover
pub const SELECTION_HOVER_COLOR: Color = Color::from_oklch(Oklch::deg(0.541, 0.281, 293.009, 0.4));

// ============================================================================
// Shadow Presets
// ============================================================================

/// Extra small drop shadow
pub const SHADOW_XS: DropShadow = DropShadow {
    offset_y: 1.0,
    blur_radius: 2.0,
    color: Color::from_hex(0x0000000D),
    ..DropShadow::default()
};

/// Small drop shadow
pub const SHADOW_SM: DropShadow = DropShadow {
    offset_y: 1.0,
    blur_radius: 3.0,
    color: Color::from_hex(0x0000001A),
    ..DropShadow::default()
};

/// Extra large drop shadow (for modals)
pub const SHADOW_XL: DropShadow = DropShadow {
    offset_y: 20.0,
    blur_radius: 25.0,
    spread_radius: -5.0,
    color: Color::from_hex(0x0000008A),
    ..DropShadow::default()
};

/// 4px blur text shadow for readability over images
pub const TEXT_SHADOW_4PX: TextShadow = TextShadow {
    offset_x: 0.0,
    offset_y: 0.0,
    blur_radius: 4.0,
    color: Color::BLACK,
};

/// 2px blur text shadow for subtle depth
pub const TEXT_SHADOW_2PX: TextShadow = TextShadow {
    offset_x: 0.0,
    offset_y: 0.0,
    blur_radius: 2.0,
    color: Color::BLACK,
};

// ============================================================================
// Helper Functions
// ============================================================================

/// Conditionally applies text shadow to a Text widget.
///
/// When enabled, applies a 4-directional shadow for improved readability
/// over background images.
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
