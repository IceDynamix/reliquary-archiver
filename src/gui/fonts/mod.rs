use iced::{font, Font};

pub mod lucide;

pub trait FontSettings {
    fn weight(self, weight: font::Weight) -> Self;
    fn styled(self, style: font::Style) -> Self;
}

impl FontSettings for Font {
    fn weight(mut self, weight: font::Weight) -> Self {
        self.weight = weight;
        self
    }

    fn styled(mut self, style: font::Style) -> Self {
        self.style = style;
        self
    }
}

pub fn inter() -> Font {
    Font::with_name("Inter 18pt")
}
