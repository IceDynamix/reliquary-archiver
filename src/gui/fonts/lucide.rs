use iced::{widget::{text, Text}, Font};

fn lucide() -> Font {
    Font::with_name("lucide")
}

pub fn arrow_down_to_line(size: u32) -> Text<'static> {
    text("\u{e45a}").font(lucide()).size(size).line_height(1.0)
}
