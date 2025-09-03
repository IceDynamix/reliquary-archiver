---
trigger: model_decision
description: when writing raxis ui components
---

# Raxis UI Component Development Guide

This guide defines the standards and patterns for writing UI components in the Raxis framework, based on the established patterns in the codebase.

---

## **Element Structure & Field Ordering**

### **Rule: Standard Element Field Order**

When creating `Element<Message>` structs, maintain this field order for consistency:

```rust
Element {
    id: Some(w_id!()),
    direction: Direction::TopToBottom,
    width: Sizing::grow(),
    height: Sizing::fit(),
    background_color: Some(Color::WHITE),
    padding: BoxAmount::all(12.0),
    border: Some(border),
    border_radius: Some(BorderRadius::all(8.0)),
    vertical_alignment: VerticalAlignment::Center,
    child_gap: 10.0,
    scroll: Some(ScrollConfig::default()),
    content: widget(/* widget */),
    children: vec![/* elements */],
    ..Default::default()
}
```

The `Element<Message>` struct supports fluent configuration through `with_*` methods for clean, chainable element construction:

```rust
Element::default()
    .with_id(w_id!())
    .with_width(Sizing::grow())
    .with_height(Sizing::fixed(40.0))
    .with_background_color(Color::from(0xF5F5F5FF))
    .with_border_radius(BorderRadius::all(8.0))
    .with_padding(BoxAmount::all(12.0))
    .with_child_gap(10.0)
```

**Available fluent methods:**

- **Layout**: `with_width()`, `with_height()`, `with_child_gap()`
- **Alignment**: `with_horizontal_alignment()`, `with_vertical_alignment()`
- **Styling**: `with_background_color()`, `with_color()`, `with_border()`, `with_border_radius()`, `with_drop_shadow()`
- **Spacing**: `with_padding()`
- **Behavior**: `with_scroll()`, `with_floating()`, `with_word_break()`
- **Identity**: `with_id()`

### **Rule: ID Generation Patterns**

Follow these patterns for consistent element identification:

- **Unique elements**: `id: Some(w_id!())`
- **List/dynamic elements**: `id: Some(combine_id(w_id!(), item.id))`
- **Named elements**: `id: Some(combine_id(w_id!(), "label_name"))`

---

## **Color Definitions**

### **Rule: Color Format Standards**

Use consistent color formats throughout the codebase:

- **Hex colors**: Use `Color::from(0xRRGGBBAA)` format for simple colors
- **RGBA colors**: Use explicit `Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }` for precise control
- **Common colors**: Use `Color::WHITE`, `Color::BLACK` constants when available

```rust
// Preferred hex format for backgrounds
background_color: Some(Color::from(0xF5F5F5FF)),

// Precise RGBA for borders and themed colors
color: Color { r: 0.85, g: 0.85, b: 0.85, a: 1.0 },
```

---

## **Border & Styling Patterns**

### **Rule: Border Definition Structure**

Always use this format for consistent border styling:

```rust
border: Some(Border {
    width: 1.0,
    color: Color { r: 0.85, g: 0.85, b: 0.85, a: 1.0 },
    placement: BorderPlacement::Center,
    dash_style: Some(StrokeDashStyle::Dash),
    dash_cap: StrokeCap::Round,
    ..Default::default()
}),
```

### **Rule: BorderRadius Patterns**

Use semantic border radius methods:

- **Uniform**: `BorderRadius::all(8.0)`
- **Directional**: `BorderRadius::top(8.0)`, `BorderRadius::bottom(8.0)`
- **Corner-specific**: `BorderRadius::tl_br(8.0)`, `BorderRadius::tr_bl(8.0)`

---

## **Layout Patterns**

### **Rule: Common Layout Configurations**

Standard layout patterns for different use cases:

- **Containers**: `direction: Direction::TopToBottom`, `child_gap: 8.0`
- **Rows**: `direction: Direction::LeftToRight`, `child_gap: 8.0`
- **Full-width**: `width: Sizing::grow()`
- **Fit content**: `height: Sizing::fit()`
- **Fixed sizes**: `width: Sizing::fixed(160.0)`
- **Flexible with constraints**: `height: Sizing::fit().min(40.0).max(120.0)`

### **Rule: Padding Conventions**

Consistent spacing patterns:

- **Container padding**: `BoxAmount::all(8.0)`
- **Input padding**: `BoxAmount::new(4.0, 8.0, 4.0, 8.0)`
- **Item padding**: `BoxAmount::all(8.0)`

### **Rule: Layout Helper Functions**

Use helper functions from `layout::helpers` for common layout patterns:

```rust
use raxis::layout::helpers::{row, column, container, center};

// Horizontal layout
row(vec![element1, element2, element3])

// Vertical layout
column(vec![element1, element2, element3])

// Single element container
container(my_widget)

// Centered content
center(my_content)
```

**Helper Macros** for cleaner syntax:

```rust
// Macro versions
row![element1, element2, element3]
column![element1, element2, element3]
```

### **Rule: Dividers and Separators**

Use the `Rule` struct for visual separators:

```rust
use raxis::layout::helpers::Rule;

// Horizontal divider (full width, 1px height)
Rule::horizontal().with_color(Color::from(0xE0E0E0FF)).into()

// Vertical divider (full height, 1px width)
Rule::vertical().with_color(Color::from(0xE0E0E0FF)).into()
```

### **Rule: Element Alignment Extensions**

Use the `ElementAlignmentExt` trait for fluent alignment operations:

```rust
use raxis::layout::helpers::ElementAlignmentExt;

// Align single element
element.align_x(HorizontalAlignment::Center).align_y(VerticalAlignment::Center)

// Align multiple elements
children.align_x(HorizontalAlignment::Right).align_y(VerticalAlignment::Top)
```

---

## **Widget Content Patterns**

### **Rule: Widget Embedding**

Always wrap widgets with the `widget()` function:

```rust
content: widget(
    Text::new("Label")
        .with_font_size(16.0)
        .with_paragraph_alignment(ParagraphAlignment::Center)
),
```

### **Rule: Button Styling Chain**

Use fluent API pattern for button styling with logical order:

```rust
content: widget(
    Button::new()
        .with_bg_color(Color { r: 0.2, g: 0.6, b: 1.0, a: 1.0 })
        .with_border_radius(8.0)
        .with_border(1.0, Color { r: 0.1, g: 0.4, b: 0.8, a: 1.0 })
        .with_click_handler(|_| { /* handler */ })
),
```

### **Rule: Text Widget Patterns**

Chain text properties in this logical order:

```rust
Text::new("Content")
    .with_font_size(16.0)
    .with_paragraph_alignment(ParagraphAlignment::Center)
    .with_text_alignment(TextAlignment::Center)
    .with_color(Color::WHITE)
```

---

## **Function Organization**

### **Rule: Component Function Patterns**

Follow these conventions for component functions:

- Functions returning `Element<Message>` should be named descriptively
- Use `hook: &mut HookManager<Message>` parameter for stateful components
- Clone shared state references when needed from hooks: `let state = hook.use_hook(|| ...).clone()`

---

## **Scrolling & Interactive Elements**

### **Rule: Scroll Configuration**

Standard scroll configuration pattern:

```rust
scroll: Some(ScrollConfig {
    vertical: Some(true),
    sticky_bottom: Some(true),
    ..Default::default()
}),
```

### **Rule: Event Handler Patterns**

Move shared state to closure scope for clean event handling:

```rust
.with_click_handler({
    let todo_state = todo_state.clone();
    let item_id = item.id;
    move |_| {
        let mut state = todo_state.borrow_mut();
        // handler logic here
    }
})
```

---

## **SVG & Graphics Elements**

### **Rule: SvgPath Integration**

Consistent pattern for SVG path elements:

```rust
SvgPath::new(svg_path!("M20 6 9 17l-5-5"), ViewBox::new(24.0, 24.0))
    .with_size(16.0, 16.0)
    .with_stroke(Color::WHITE)
    .with_stroke_width(2.0)
    .with_stroke_cap(StrokeCap::Round)
    .with_stroke_join(StrokeLineJoin::Round)
    .as_element(w_id!())
```

---

## **Code Style Guidelines**

### **Formatting**

- Break long method chains across multiple lines with proper indentation
- Group related fields together in Element structs
- Always include `..Default::default()` at the end of Element definitions

### **Naming Conventions**

- Use descriptive function names that indicate the UI component purpose
- Prefix helper functions with the component type (e.g., `demo_box`, `todo_item`)
- Use semantic variable names for colors and styling (e.g., `inset`, `center`, `outset`)

### **Comments**

- Add inline comments for non-obvious styling choices
- Document complex layout patterns

---

## **Best Practices**

- **Reusability**: Create helper functions for common UI patterns
- **State Management**: Use proper closure patterns for event handlers
- **Maintainability**: Group related styling properties together
- **Accessibility**: Use semantic colors and proper contrast ratios

---

This guide should be followed when implementing new UI components or modifying existing ones in the Raxis framework to ensure consistency and maintainability across the codebase.
