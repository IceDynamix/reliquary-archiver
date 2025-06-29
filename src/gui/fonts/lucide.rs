use iced::{widget::{text, Text}, Font, Pixels};

fn lucide() -> Font {
    Font::with_name("lucide")
}

macro_rules! icon {
    ($name:ident, $code:literal) => {
        pub fn $name(size: impl Into<Pixels>) -> Text<'static> {
            text($code).font(lucide()).size(size).line_height(1.0)
        }
    };
}

icon!(arrow_down_to_line, "\u{e45a}");
icon!(refresh_cw, "\u{e149}");
