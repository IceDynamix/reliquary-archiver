use iced::{border, widget::{button, container, text}, Background, Theme};

pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        background: None,
        text_color: palette.secondary.base.text,
        border: border::rounded(8),
        ..button::Style::default()
    };

    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(palette.secondary.base.color)),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(palette.secondary.strong.color)),
            ..base
        },
        button::Status::Active | button::Status::Disabled => base,
    }
}

pub fn rounded_box_md(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: border::rounded(8),
        ..container::Style::default()
    }
}

pub fn rounded_button_primary(theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        border: border::rounded(8),
        ..button::primary(theme, status)
    }
}

pub fn rounded_button_secondary(theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        border: border::rounded(8),
        ..button::secondary(theme, status)
    }
}

pub fn text_muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().background.base.text.scale_alpha(0.5)),
    }
}

pub const PAD_SM: u16 = 4;
pub const PAD_MD: u16 = 8;
pub const PAD_LG: u16 = 16;

pub const SPACE_SM: u32 = 4;
pub const SPACE_MD: u32 = 8;
pub const SPACE_LG: u32 = 16;
