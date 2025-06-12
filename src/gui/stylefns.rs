use iced::{border, widget::{button, container, text}, Theme};

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
