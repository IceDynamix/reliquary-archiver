//! Rules divide space horizontally or vertically.
use iced::{self, Point, Renderer, Vector};
use iced::border;
use iced::advanced::{layout, Layout, Widget};
use iced::mouse;
use iced::advanced::renderer;
use iced::advanced::Renderer as _;
use iced::advanced::widget::{tree, Tree};
use iced::widget::canvas::{self, LineDash};
use iced::{
    Color, Element, Length, Pixels, Rectangle, Size, Theme,
};

/// Display a horizontal or vertical rule for dividing content.
#[allow(missing_debug_implementations)]
pub struct DashedRule<'a, Theme = iced::Theme>
where
    Theme: Catalog,
{
    width: Length,
    height: Length,
    is_horizontal: bool,
    class: Theme::Class<'a>,
}

impl<'a, Theme> DashedRule<'a, Theme>
where
    Theme: Catalog,
{
    /// Creates a horizontal [`DashedRule`] with the given height.
    pub fn horizontal(height: impl Into<Pixels>) -> Self {
        DashedRule {
            width: Length::Fill,
            height: Length::Fixed(height.into().0),
            is_horizontal: true,
            class: Theme::default(),
        }
    }

    /// Creates a vertical [`DashedRule`] with the given width.
    pub fn vertical(width: impl Into<Pixels>) -> Self {
        DashedRule {
            width: Length::Fixed(width.into().0),
            height: Length::Fill,
            is_horizontal: false,
            class: Theme::default(),
        }
    }

    /// Sets the style of the [`DashedRule`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }
}

#[derive(Default)]
struct State {
    cache: canvas::Cache,
}

impl<Message, Theme> Widget<Message, Theme, Renderer>
    for DashedRule<'_, Theme>
where
    Theme: Catalog,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let style = theme.style(&self.class);

        let bounds = if self.is_horizontal {
            let line_y = (bounds.y + (bounds.height / 2.0)
                - (style.width as f32 / 2.0))
                .round();

            let (offset, line_width) = style.fill_mode.fill(bounds.width);
            let line_x = bounds.x + offset;

            Rectangle {
                x: line_x,
                y: line_y,
                width: line_width,
                height: style.width as f32,
            }
        } else {
            let line_x = (bounds.x + (bounds.width / 2.0)
                - (style.width as f32 / 2.0))
                .round();

            let (offset, line_height) = style.fill_mode.fill(bounds.height);
            let line_y = bounds.y + offset;

            Rectangle {
                x: line_x,
                y: line_y,
                width: style.width as f32,
                height: line_height,
            }
        };

        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let line = if self.is_horizontal {
                canvas::Path::line(Point::new(0.0, bounds.height / 2.0), Point::new(bounds.width, bounds.height / 2.0))
            } else {
                canvas::Path::line(Point::new(bounds.width / 2.0, 0.0), Point::new(bounds.width / 2.0, bounds.height))
            };

            frame.stroke(&line, canvas::Stroke {
                style: canvas::Style::Solid(style.color),
                width: style.width as f32,
                line_dash: style.dash,
                ..Default::default()
            });
        });

        renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
            use iced::advanced::graphics::geometry::Renderer as _;

            renderer.draw_geometry(geometry);
        });
    }
}

impl<'a, Message, Theme> From<DashedRule<'a, Theme>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a + Catalog,
{
    fn from(rule: DashedRule<'a, Theme>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(rule)
    }
}

/// The appearance of a rule.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// The color of the rule.
    pub color: Color,
    /// The width (thickness) of the rule line.
    pub width: u16,
    /// The radius of the line corners.
    pub dash: LineDash<'static>,
    /// The [`FillMode`] of the rule.
    pub fill_mode: FillMode,
    /// Whether the rule should be snapped to the pixel grid.
    pub snap: bool,
}

/// The fill mode of a rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FillMode {
    /// Fill the whole length of the container.
    Full,
    /// Fill a percent of the length of the container. The rule
    /// will be centered in that container.
    ///
    /// The range is `[0.0, 100.0]`.
    Percent(f32),
    /// Uniform offset from each end, length units.
    Padded(u16),
    /// Different offset on each end of the rule, length units.
    /// First = top or left.
    AsymmetricPadding(u16, u16),
}

impl FillMode {
    /// Return the starting offset and length of the rule.
    ///
    /// * `space` - The space to fill.
    ///
    /// # Returns
    ///
    /// * (`starting_offset`, `length`)
    pub fn fill(&self, space: f32) -> (f32, f32) {
        match *self {
            FillMode::Full => (0.0, space),
            FillMode::Percent(percent) => {
                if percent >= 100.0 {
                    (0.0, space)
                } else {
                    let percent_width = (space * percent / 100.0).round();

                    (((space - percent_width) / 2.0).round(), percent_width)
                }
            }
            FillMode::Padded(padding) => {
                if padding == 0 {
                    (0.0, space)
                } else {
                    let padding = padding as f32;
                    let mut line_width = space - (padding * 2.0);
                    if line_width < 0.0 {
                        line_width = 0.0;
                    }

                    (padding, line_width)
                }
            }
            FillMode::AsymmetricPadding(first_pad, second_pad) => {
                let first_pad = first_pad as f32;
                let second_pad = second_pad as f32;
                let mut line_width = space - first_pad - second_pad;
                if line_width < 0.0 {
                    line_width = 0.0;
                }

                (first_pad, line_width)
            }
        }
    }
}

/// The theme catalog of a [`DashedRule`].
pub trait Catalog: Sized {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>) -> Style;
}

/// A styling function for a [`DashedRule`].
///
/// This is just a boxed closure: `Fn(&Theme, Status) -> Style`.
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

pub const DASH_MD: LineDash<'static> = LineDash {
    segments: &[5.0, 5.0],
    offset: 0,
};

/// The default styling of a [`DashedRule`].
pub fn default(theme: &Theme) -> Style {
    let palette = theme.extended_palette();

    Style {
        color: palette.background.strong.color,
        width: 1,
        dash: DASH_MD,
        fill_mode: FillMode::Full,
        snap: true,
    }
}
