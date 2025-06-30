#[macro_export]
macro_rules! elements {
    ($($x:expr),+ $(,)?) => {
        [
            $(
                iced::Element::from($x)
            ),+
        ]
    };
}
