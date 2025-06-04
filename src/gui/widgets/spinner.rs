//! Show a circular progress indicator.
use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Clipboard, Layout, Shell, Widget};
use iced::mouse;
use iced::time::Instant;
use iced::widget::canvas;
use iced::window;
use iced::Point;
use iced::{
    Background, Color, Element, Event, Length, Radians, Rectangle, Renderer,
    Size, Vector,
};

use super::easing::{self, Easing};

use std::f32::consts::PI;
use std::time::Duration;

const MIN_ANGLE: Radians = Radians(PI / 8.0);
const WRAP_ANGLE: Radians = Radians(2.0 * PI - PI / 4.0);
const FULL_ANGLE: Radians = Radians(2.0 * PI);
const BASE_ROTATION_SPEED: u32 = u32::MAX / 80;

#[allow(missing_debug_implementations)]
pub struct Spinner<'a, Theme>
where
    Theme: StyleSheet,
{
    size: f32,
    bar_height: f32,
    style: <Theme as StyleSheet>::Style,
    easing: &'a Easing,
    cycle_duration: Duration,
    rotation_duration: Duration,
    completed: bool,
}

impl<'a, Theme> Spinner<'a, Theme>
where
    Theme: StyleSheet,
{
    /// Creates a new [`Spinner`] with the given content.
    pub fn new() -> Self {
        Spinner {
            size: 40.0,
            bar_height: 4.0,
            style: <Theme as StyleSheet>::Style::default(),
            easing: &easing::STANDARD,
            cycle_duration: Duration::from_millis(600),
            rotation_duration: Duration::from_secs(2),
            completed: false,
        }
    }

    /// Sets the size of the [`Spinner`].
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Sets the bar height of the [`Spinner`].
    pub fn bar_height(mut self, bar_height: f32) -> Self {
        self.bar_height = bar_height;
        self
    }

    /// Sets the style variant of this [`Spinner`].
    pub fn style(mut self, style: <Theme as StyleSheet>::Style) -> Self {
        self.style = style;
        self
    }

    /// Sets the easing of this [`Spinner`].
    pub fn easing(mut self, easing: &'a Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Sets the cycle duration of this [`Spinner`].
    pub fn cycle_duration(mut self, duration: Duration) -> Self {
        self.cycle_duration = duration / 2;
        self
    }

    /// Sets the base rotation duration of this [`Spinner`]. This is the duration that a full
    /// rotation would take if the cycle rotation were set to 0.0 (no expanding or contracting)
    pub fn rotation_duration(mut self, duration: Duration) -> Self {
        self.rotation_duration = duration;
        self
    }

    /// Sets whether the circular progress indicator should complete to a full circle.
    pub fn completed(mut self, completed: bool) -> Self {
        self.completed = completed;
        self
    }
}

impl<Theme> Default for Spinner<'_, Theme>
where
    Theme: StyleSheet,
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
enum Animation {
    Expanding {
        start: Instant,
        progress: f32,
        rotation: u32,
        last: Instant,
    },
    Contracting {
        start: Instant,
        progress: f32,
        rotation: u32,
        last: Instant,
    },
    Completed {
        start: Instant,
        progress: f32,
        rotation: u32,
        last: Instant,
    },
}

impl Default for Animation {
    fn default() -> Self {
        Self::Expanding {
            start: Instant::now(),
            progress: 0.0,
            rotation: 0,
            last: Instant::now(),
        }
    }
}

impl Animation {
    fn next(&self, additional_rotation: u32, now: Instant, completed: bool) -> Self {
        match self {
            Self::Expanding { rotation, .. } => Self::Contracting {
                start: now,
                progress: 0.0,
                rotation: rotation.wrapping_add(additional_rotation),
                last: now,
            },
            Self::Contracting { rotation, .. } => {
                let next_rotation = rotation.wrapping_add(
                    BASE_ROTATION_SPEED.wrapping_add(
                        (f64::from(WRAP_ANGLE / (2.0 * Radians::PI))
                            * u32::MAX as f64) as u32,
                    ),
                );
                
                if completed {
                    Self::Completed {
                        start: now,
                        progress: 0.0,
                        rotation: next_rotation,
                        last: now,
                    }
                } else {
                    Self::Expanding {
                        start: now,
                        progress: 0.0,
                        rotation: next_rotation,
                        last: now,
                    }
                }
            },
            Self::Completed { start, rotation, last, .. } => {
                if completed {
                    // Stay completed
                    Self::Completed {
                        start: *start,
                        progress: 1.0,
                        rotation: *rotation,
                        last: *last,
                    }
                } else {
                    // Reset to initial state
                    Self::Expanding {
                        start: now,
                        progress: 0.0,
                        rotation: 0,
                        last: now,
                    }
                }
            },
        }
    }

    fn start(&self) -> Instant {
        match self {
            Self::Expanding { start, .. } 
            | Self::Contracting { start, .. } 
            | Self::Completed { start, .. } => *start,
        }
    }

    fn last(&self) -> Instant {
        match self {
            Self::Expanding { last, .. } 
            | Self::Contracting { last, .. } 
            | Self::Completed { last, .. } => *last,
        }
    }

    fn timed_transition(
        &self,
        cycle_duration: Duration,
        rotation_duration: Duration,
        now: Instant,
        completed: bool,
    ) -> Self {
        let elapsed = now.duration_since(self.start());
        let additional_rotation = ((now - self.last()).as_secs_f32()
            / rotation_duration.as_secs_f32()
            * (u32::MAX) as f32) as u32;

        let animation = match elapsed {
            elapsed if elapsed > cycle_duration => {
                self.next(additional_rotation, now, completed)
            }
            _ => self.with_elapsed(
                cycle_duration,
                additional_rotation,
                elapsed,
                now,
            ),
        };

        animation
    }

    fn with_elapsed(
        &self,
        cycle_duration: Duration,
        additional_rotation: u32,
        elapsed: Duration,
        now: Instant,
    ) -> Self {
        let progress = elapsed.as_secs_f32() / cycle_duration.as_secs_f32();
        match self {
            Self::Expanding {
                start, rotation, ..
            } => Self::Expanding {
                start: *start,
                progress,
                rotation: rotation.wrapping_add(additional_rotation),
                last: now,
            },
            Self::Contracting {
                start, rotation, ..
            } => Self::Contracting {
                start: *start,
                progress,
                rotation: rotation.wrapping_add(additional_rotation),
                last: now,
            },
            Self::Completed {
                start, rotation, ..
            } => Self::Completed {
                start: *start,
                progress,
                rotation: rotation.wrapping_add(additional_rotation),
                last: now,
            },
        }
    }

    fn rotation(&self) -> f32 {
        match self {
            Self::Expanding { rotation, .. }
            | Self::Contracting { rotation, .. }
            | Self::Completed { rotation, .. } => {
                *rotation as f32 / u32::MAX as f32
            }
        }
    }
}

#[derive(Default)]
struct State {
    animation: Animation,
    cache: canvas::Cache,
}

impl<'a, Message, Theme> Widget<Message, Theme, Renderer>
    for Spinner<'a, Theme>
where
    Message: 'a + Clone,
    Theme: StyleSheet,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fixed(self.size),
            height: Length::Fixed(self.size),
        }
    }

    fn layout(
        &self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.size, self.size)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();

        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            state.animation = state.animation.timed_transition(
                self.cycle_duration,
                self.rotation_duration,
                *now,
                self.completed,
            );

            state.cache.clear();

            if let Animation::Completed { progress, .. } = state.animation {
                if progress >= 1.0 {
                    // Don't redraw if the animation is completed
                    return;
                }
            }

            shell.request_redraw();
        }
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
        use advanced::Renderer as _;

        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let custom_style =
            <Theme as StyleSheet>::appearance(theme, &self.style);

        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let track_radius = frame.width() / 2.0 - self.bar_height;
            let track_path = canvas::Path::new(|path| {
                path.circle(frame.center(), track_radius);
                path.close();
            });

            frame.stroke(
                &track_path,
                canvas::Stroke::default()
                    .with_color(custom_style.track_color)
                    .with_width(self.bar_height),
            );

            let mut builder = canvas::path::Builder::new();

            let start = Radians(state.animation.rotation() * 2.0 * PI);

            match state.animation {
                Animation::Expanding { progress, .. } => {
                    builder.arc(canvas::path::Arc {
                        center: frame.center(),
                        radius: track_radius,
                        start_angle: start,
                        end_angle: start
                            + MIN_ANGLE
                            + WRAP_ANGLE * (self.easing.y_at_x(progress)),
                    });
                }
                Animation::Contracting { progress, .. } => {
                    builder.arc(canvas::path::Arc {
                        center: frame.center(),
                        radius: track_radius,
                        start_angle: start
                            + WRAP_ANGLE * (self.easing.y_at_x(progress)),
                        end_angle: start + MIN_ANGLE + WRAP_ANGLE,
                    });
                }
                Animation::Completed { progress, .. } => {
                    if progress >= 1.0 {
                        // Draw full circle
                        builder.circle(frame.center(), track_radius);
                        builder.close();
                    } else {
                        builder.arc(canvas::path::Arc {
                            center: frame.center(),
                            radius: track_radius,
                            start_angle: start,
                            end_angle: start
                                + MIN_ANGLE
                                + FULL_ANGLE * (self.easing.y_at_x(progress)),
                        });
                    }
                }
            }

            let bar_path = builder.build();

            let bar_color = if let Animation::Completed { .. } = state.animation {
                custom_style.completed_bar_color
            } else {
                custom_style.bar_color
            };

            frame.stroke(
                &bar_path,
                canvas::Stroke::default()
                    .with_color(bar_color)
                    .with_width(self.bar_height),
            );

            // Draw checkmark during completion phase 2
            if let Animation::Completed { progress, .. } = state.animation {
                let start_at = 0.2;
                if progress > start_at {
                    let checkmark_progress = (progress - start_at) / (1.0 - start_at); // Normalize phase 2 to 0.0-1.0
                    let eased_progress = self.easing.y_at_x(checkmark_progress);
                    
                    // Create checkmark path
                    let center = frame.center();
                    let checkmark_size = track_radius * 1.2;

                    // Define checkmark points relative to center
                    let left_point = Point::new(
                        center.x - checkmark_size * 0.375,
                        center.y + checkmark_size * 0.075,
                    );
                    let middle_point = Point::new(
                        center.x - checkmark_size * 0.125,
                        center.y + checkmark_size * 0.325,
                    );
                    let right_point = Point::new(
                        center.x + checkmark_size * 0.375,
                        center.y - checkmark_size * 0.325,
                    );
                    
                    // Create checkmark with animated progress
                    let mut checkmark_builder = canvas::path::Builder::new();
                    checkmark_builder.move_to(left_point);
                    
                    if eased_progress <= 0.5 {
                        // First half: draw line from left to middle
                        let segment_progress = eased_progress * 2.0;
                        let current_point = Point::new(
                            left_point.x + (middle_point.x - left_point.x) * segment_progress,
                            left_point.y + (middle_point.y - left_point.y) * segment_progress,
                        );
                        checkmark_builder.line_to(current_point);
                    } else {
                        // Second half: complete first segment and draw second segment
                        checkmark_builder.line_to(middle_point);
                        let segment_progress = (eased_progress - 0.5) * 2.0;
                        let current_point = Point::new(
                            middle_point.x + (right_point.x - middle_point.x) * segment_progress,
                            middle_point.y + (right_point.y - middle_point.y) * segment_progress,
                        );
                        checkmark_builder.line_to(current_point);
                    }
                    
                    let checkmark_path = checkmark_builder.build();
                    
                    frame.stroke(
                        &checkmark_path,
                        canvas::Stroke::default()
                            .with_color(bar_color)
                            .with_width(self.bar_height),
                    );
                }
            }
        });

        renderer.with_translation(
            Vector::new(bounds.x, bounds.y),
            |renderer| {
                use iced::advanced::graphics::geometry::Renderer as _;

                renderer.draw_geometry(geometry);
            },
        );
    }
}

impl<'a, Message, Theme> From<Spinner<'a, Theme>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: StyleSheet + 'a,
{
    fn from(spinner: Spinner<'a, Theme>) -> Self {
        Self::new(spinner)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Appearance {
    /// The [`Background`] of the progress indicator.
    pub background: Option<Background>,
    /// The track [`Color`] of the progress indicator.
    pub track_color: Color,
    /// The bar [`Color`] of the progress indicator.
    pub bar_color: Color,
    /// The bar [`Color`] of the progress indicator after it has completed
    pub completed_bar_color: Color,
}

impl std::default::Default for Appearance {
    fn default() -> Self {
        Self {
            background: None,
            track_color: Color::TRANSPARENT,
            bar_color: Color::BLACK,
            completed_bar_color: Color::WHITE,
        }
    }
}

/// A set of rules that dictate the style of an indicator.
pub trait StyleSheet {
    /// The supported style of the [`StyleSheet`].
    type Style: Default;

    /// Produces the active [`Appearance`] of a indicator.
    fn appearance(&self, style: &Self::Style) -> Appearance;
}

impl StyleSheet for iced::Theme {
    type Style = ();

    fn appearance(&self, _style: &Self::Style) -> Appearance {
        let palette = self.extended_palette();

        Appearance {
            background: None,
            track_color: palette.background.weak.color,
            bar_color: palette.primary.base.color,
            completed_bar_color: palette.success.strong.color,
        }
    }
}

pub fn spinner<'a, Theme>() -> Spinner<'a, Theme>
where
    Theme: StyleSheet + 'a,
{
    Spinner::new()
}
