//! Log viewer component.
//!
//! Provides a virtualized, scrollable view of application logs with:
//! - Line selection (click, Ctrl+click, Shift+click, drag)
//! - Copy to clipboard (Ctrl+C)
//! - Truncation of very long lines with "Show more" button
//! - Sticky-bottom auto-scrolling for new log entries

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use raxis::layout::model::{
    BackdropFilter, Border, BorderRadius, BoxAmount, Color, Direction, Element, ScrollBarSize, ScrollConfig, ScrollbarStyle, Sizing,
};
use raxis::runtime::font_manager::FontIdentifier;
use raxis::runtime::scroll::ScrollPosition;
use raxis::runtime::task;
use raxis::runtime::task::{ClipboardAction, Task};
use raxis::runtime::vkey::VKey;
use raxis::util::unique::combine_id;
use raxis::widgets::button::Button;
use raxis::widgets::mouse_area::{MouseArea, MouseAreaEvent};
use raxis::widgets::text::{ParagraphAlignment, Text};
use raxis::widgets::{widget, Widget};
use raxis::{w_id, HookManager};

use crate::rgui::messages::RootMessage;
use crate::rgui::theme::{
    BORDER_COLOR, BORDER_RADIUS_SM, CARD_BACKGROUND, SCROLLBAR_THUMB_COLOR, SCROLLBAR_TRACK_COLOR, SELECTION_COLOR, TEXT_ON_LIGHT_COLOR,
};
use crate::{LOG_BUFFER, LOG_NOTIFY};

/// Formats a byte size to a human-readable string (B, KB, MB).
pub fn short_size(size: usize) -> String {
    let size_f = size as f64;
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.2} KB", size_f / 1024.0)
    } else {
        format!("{:.2} MB", size_f / 1024.0 / 1024.0)
    }
}

struct InvalidateOnBoundsChanged<Message, E: Fn(&raxis::widgets::Event, &mut raxis::Shell<Message>) -> Option<Task<Message>>> {
    _marker: std::marker::PhantomData<Message>,
    event_listener: E,
}

impl<Message, E: Fn(&raxis::widgets::Event, &mut raxis::Shell<Message>) -> Option<Task<Message>>> std::fmt::Debug
    for InvalidateOnBoundsChanged<Message, E>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvalidateOnBoundsChanged").finish()
    }
}

struct InvalidateOnBoundsChangedState {
    prev_bounds: raxis::widgets::Bounds,
}

impl<Message, E: Fn(&raxis::widgets::Event, &mut raxis::Shell<Message>) -> Option<Task<Message>>> Widget<Message>
    for InvalidateOnBoundsChanged<Message, E>
{
    fn state(&self, arenas: &raxis::layout::UIArenas, device_resources: &raxis::runtime::DeviceResources) -> raxis::widgets::State {
        Some(Box::new(InvalidateOnBoundsChangedState {
            prev_bounds: raxis::widgets::Bounds::default(),
        }))
    }

    fn paint(
        &mut self,
        arenas: &raxis::layout::UIArenas,
        instance: &mut raxis::widgets::Instance,
        shell: &mut raxis::Shell<Message>,
        recorder: &mut raxis::gfx::command_recorder::CommandRecorder,
        style: raxis::layout::model::ElementStyle,
        bounds: raxis::widgets::Bounds,
        now: Instant,
    ) {
        // Nothing to do
    }

    fn update(
        &mut self,
        arenas: &mut raxis::layout::UIArenas,
        instance: &mut raxis::widgets::Instance,
        hwnd: windows::Win32::Foundation::HWND,
        shell: &mut raxis::Shell<Message>,
        event: &raxis::widgets::Event,
        bounds: raxis::widgets::Bounds,
    ) {
        if matches!(event, raxis::widgets::Event::Redraw { .. }) {
            let state = raxis::with_state!(mut instance as InvalidateOnBoundsChangedState);
            if state.prev_bounds != bounds {
                shell.request_redraw(hwnd, raxis::RedrawRequest::Immediate);
                state.prev_bounds = bounds;
            }
        }

        if let Some(task) = (self.event_listener)(event, shell) {
            shell.dispatch_task(task);
        }
    }
}

/// Renders the virtualized log viewer component.
///
/// Uses viewport-based rendering to efficiently display thousands of log lines
/// without performance degradation. Only visible lines are rendered, with
/// spacer elements for scrolling.
pub fn log_view(hook: &mut HookManager<RootMessage>) -> Element<RootMessage> {
    let container_id = w_id!();

    let mut state = hook.instance(container_id);
    let show_more = state.use_hook(|| Rc::new(RefCell::new(HashSet::<usize>::new()))).clone();
    let max_content_width = state.use_hook(|| Rc::new(Cell::new(0.0f32))).clone();
    let max_line_length = state.use_hook(|| Rc::new(Cell::new(0usize))).clone();
    let prev_item_count = state.use_hook(|| Rc::new(Cell::new(0usize))).clone();

    // Selection state
    let selection_state = state.use_hook(|| Rc::new(RefCell::new(Option::<Range<usize>>::None))).clone();
    let drag_start = state.use_hook(|| Rc::new(Cell::new(Option::<usize>::None))).clone();
    let is_dragging = state.use_hook(|| Rc::new(Cell::new(false))).clone();

    let lines = LOG_BUFFER.lock().unwrap();

    let total_items = lines.len();
    if total_items != prev_item_count.replace(total_items) {
        hook.invalidate_layout();
    }

    let line_height_no_gap = 12.0;
    let gap = 0.0;
    let padding = BoxAmount::new(8.0, 16.0, 16.0, 8.0);
    let buffer_items_per_side = 2usize;

    let truncate_threshold = 3000;

    let line_height = line_height_no_gap + gap;

    let container_dims = hook.scroll_state_manager.get_container_dimensions(container_id);

    let content_dims = hook.scroll_state_manager.get_previous_content_dimensions(container_id);

    max_content_width.replace(max_content_width.get().max(content_dims.0));

    let visible_items = (container_dims.1 / line_height).ceil() as usize + buffer_items_per_side * 2;

    let ScrollPosition { x: _scroll_x, y: scroll_y } = hook.scroll_state_manager.get_scroll_position(container_id);

    let pre_scroll_items = (((scroll_y + gap - padding.top) / line_height).floor() as usize).saturating_sub(buffer_items_per_side);
    let post_scroll_items = total_items.saturating_sub(pre_scroll_items).saturating_sub(visible_items).max(0);

    Element {
        id: Some(container_id),
        direction: Direction::TopToBottom,
        width: Sizing::grow(),
        height: Sizing::fixed(150.0),
        scroll: Some(ScrollConfig {
            horizontal: Some(true),
            vertical: Some(true),
            sticky_bottom: Some(true),
            scrollbar_style: Some(ScrollbarStyle {
                thumb_color: SCROLLBAR_THUMB_COLOR,
                track_color: SCROLLBAR_TRACK_COLOR,
                size: ScrollBarSize::ThinThick(8.0, 12.0),
                ..Default::default()
            }),
            ..Default::default()
        }),
        background_color: Some(CARD_BACKGROUND),
        backdrop_filter: Some(BackdropFilter::blur(10.0)),
        border: Some(Border {
            width: 1.0,
            color: BORDER_COLOR,
            ..Default::default()
        }),
        border_radius: Some(BorderRadius::all(BORDER_RADIUS_SM)),
        child_gap: gap,
        padding,
        content: widget(InvalidateOnBoundsChanged {
            _marker: std::marker::PhantomData,
            event_listener: {
                let selection_state = selection_state.clone();
                let is_dragging = is_dragging.clone();
                let drag_start = drag_start.clone();
                move |e, shell| match e {
                    raxis::widgets::Event::KeyDown { key, modifiers } => {
                        if matches!(key, VKey::C | VKey::X) && modifiers.ctrl {
                            if let Some(selection_range) = selection_state.borrow_mut().take() {
                                let lines = LOG_BUFFER.lock().unwrap();
                                let selected_lines = lines[selection_range.start..selection_range.end]
                                    .iter()
                                    .map(|line| line.clone())
                                    .collect::<Vec<_>>();

                                return Some(task::effect(task::Action::Clipboard(ClipboardAction::Set(
                                    selected_lines.join("\n"),
                                ))));
                            }
                        }
                        None
                    }
                    raxis::widgets::Event::MouseButtonDown {
                        x,
                        y,
                        click_count,
                        modifiers,
                    } => {
                        shell.capture_event(container_id);
                        None
                    }

                    raxis::widgets::Event::MouseButtonUp { .. } => {
                        is_dragging.set(false);
                        drag_start.set(None);
                        None
                    }
                    _ => None,
                }
            },
        }),
        children: {
            // DWrite runs into precision issues with really long text (it only uses f32)
            // So we have to calculate the width manually with a f64
            // Obviously won't work with special glyphs but what are you gonna do? /shrug
            const MONO_CHAR_WIDTH: f64 = 6.02411;

            let mut text_children = (pre_scroll_items..(pre_scroll_items + visible_items).min(total_items))
                .map(|i| {
                    // Determine if this line is selected
                    let is_selected = if let Some(selection_range) = selection_state.borrow().as_ref() {
                        selection_range.contains(&i)
                    } else {
                        false
                    };

                    let line_element = if lines[i].len() > truncate_threshold && !show_more.borrow().contains(&i) {
                        max_line_length.replace(max_line_length.get().max(truncate_threshold));

                        Element {
                            id: Some(combine_id(w_id!(), i % visible_items)),
                            height: Sizing::fixed(line_height_no_gap),
                            background_color: if is_selected { Some(SELECTION_COLOR) } else { None },
                            children: vec![
                                Text::new(lines[i][0..truncate_threshold].to_string())
                                    .with_word_wrap(false)
                                    .with_font_family(FontIdentifier::System("Lucida Console".to_string()))
                                    .with_assisted_width((MONO_CHAR_WIDTH * truncate_threshold as f64) as f32)
                                    .with_font_size(10.0)
                                    .with_paragraph_alignment(ParagraphAlignment::Center)
                                    .as_element()
                                    .with_id(combine_id(w_id!(), i % visible_items))
                                    .with_height(Sizing::fixed(line_height_no_gap))
                                    .with_background_color(if is_selected { SELECTION_COLOR } else { Color::TRANSPARENT }),
                                Button::new()
                                    .with_click_handler({
                                        let show_more = show_more.clone();
                                        move |_, _| {
                                            show_more.borrow_mut().insert(i);
                                        }
                                    })
                                    .as_element(
                                        combine_id(w_id!(), i % visible_items),
                                        Text::new(format!("Show more ({})", short_size(lines[i].len())))
                                            .with_font_size(8.0)
                                            .with_color(TEXT_ON_LIGHT_COLOR)
                                            .with_assisted_id(combine_id(w_id!(), i % visible_items)),
                                    ),
                            ],

                            ..Default::default()
                        }
                    } else {
                        max_line_length.replace(max_line_length.get().max(lines[i].len()));

                        Text::new(lines[i].to_string())
                            .with_word_wrap(false)
                            .with_font_family(FontIdentifier::System("Lucida Console".to_string()))
                            .with_font_size(10.0)
                            .with_assisted_width((MONO_CHAR_WIDTH * lines[i].len() as f64) as f32)
                            .with_paragraph_alignment(ParagraphAlignment::Center)
                            .as_element()
                            .with_id(combine_id(w_id!(), i % visible_items))
                            .with_height(Sizing::fixed(line_height_no_gap))
                            .with_background_color(if is_selected { SELECTION_COLOR } else { Color::TRANSPARENT })
                    };

                    // Wrap the line with MouseArea for selection
                    MouseArea::new({
                        let selection_state = selection_state.clone();
                        let drag_start = drag_start.clone();
                        let is_dragging = is_dragging.clone();
                        let line_index = i;

                        move |event, shell| {
                            match event {
                                MouseAreaEvent::MouseButtonDown { modifiers, .. } => {
                                    if modifiers.ctrl {
                                        // Toggle selection for this line
                                        let mut selection = selection_state.borrow_mut();
                                        match selection.as_mut() {
                                            Some(range) => {
                                                if range.contains(&line_index) {
                                                    // Remove from selection - this is complex with ranges
                                                    // For now, just clear selection if clicking on selected line
                                                    *selection = None;
                                                } else {
                                                    // Expand selection to include this line
                                                    let new_start = range.start.min(line_index);
                                                    let new_end = range.end.max(line_index + 1);
                                                    *selection = Some(new_start..new_end);
                                                }
                                            }
                                            None => {
                                                *selection = Some(line_index..line_index + 1);
                                            }
                                        }
                                    } else if modifiers.shift {
                                        // Extend selection from existing start to this line
                                        let mut selection = selection_state.borrow_mut();
                                        if let Some(existing_range) = selection.as_ref() {
                                            let start = existing_range.start.min(line_index);
                                            let end = existing_range.end.max(line_index + 1);
                                            *selection = Some(start..end);
                                        } else {
                                            *selection = Some(line_index..line_index + 1);
                                        }
                                    } else {
                                        // Start new selection
                                        *selection_state.borrow_mut() = Some(line_index..line_index + 1);
                                        drag_start.set(Some(line_index));
                                        is_dragging.set(true);
                                    }
                                }
                                MouseAreaEvent::MouseMove { inside, .. } => {
                                    if inside && is_dragging.get() {
                                        if let Some(start_line) = drag_start.get() {
                                            let start = start_line.min(line_index);
                                            let end = start_line.max(line_index) + 1;
                                            *selection_state.borrow_mut() = Some(start..end);
                                        }
                                    }
                                }
                                MouseAreaEvent::MouseButtonUp { .. } => {
                                    is_dragging.set(false);
                                    drag_start.set(None);
                                }
                                _ => {}
                            };

                            Some(RootMessage::TriggerRender)
                        }
                    })
                    .as_element(combine_id(container_id, i % visible_items), line_element)
                })
                .collect();

            let keep_width =
                ((max_line_length.get() as f64 * MONO_CHAR_WIDTH) as f32).max(max_content_width.get() - padding.left - padding.right);

            let mut children = vec![];
            if pre_scroll_items > 0 {
                children.push(Element {
                    id: Some(w_id!()),
                    width: Sizing::fixed(keep_width),
                    height: Sizing::fixed(line_height * pre_scroll_items as f32 - gap),
                    ..Default::default()
                });
            }

            children.append(&mut text_children);

            if post_scroll_items > 0 {
                children.push(Element {
                    id: Some(w_id!()),
                    width: Sizing::fixed(keep_width),
                    height: Sizing::fixed(line_height * post_scroll_items as f32 - gap),
                    ..Default::default()
                });
            }
            children
        },
        ..Default::default()
    }
}
