# Motion

`rake::motion` moves and fades painted content with springs. Animation frames skip view rendering,
layout and paint, so a busy view does not slow the motion down.

This API is experimental. It needs the `motion` feature and Rake's patched `gpui-pre`.

## Setup

Enable the feature:

```toml
[dependencies]
rake = { package = "rake-app", version = "...", features = ["motion"] }
```

Cargo does not apply a dependency's patches, so an application outside the Rake workspace must
patch `gpui-pre` in its own workspace root. Use a checkout of Rake's `vendor/gpui-pre`:

```toml
[patch.crates-io]
gpui-pre = { path = "../rake/vendor/gpui-pre" }
```

`motion` is unavailable with the `gpui-fast` feature, because GPUI Fast has no presentation layers.

## Animate a layer

Wrap content in a layer, then spring it toward a target from an event handler or task:

```rust
use rake::gpui::{self, IntoElement, Styled};
use rake::motion::{self, Spring, Target};

fn panel() -> impl IntoElement {
    motion::layer("panel", gpui::div().size(gpui::px(120.)))
}

fn slide_away(window: &mut gpui::Window, cx: &gpui::App) {
    let target = Target::offset(gpui::px(240.), gpui::px(0.)).with_opacity(0.0);

    motion::animate("panel", target, Spring::SMOOTH, window, cx);
}
```

Offsets are relative to the laid-out position, and `Target::REST` returns a layer to it. Calling
`animate` during a running spring retargets it and keeps its velocity. `jump` moves a layer
immediately. When the application has reduced motion enabled, `animate` behaves like `jump`.

| Spring | Use |
| --- | --- |
| `Spring::SNAPPY` | Direct responses to input |
| `Spring::SMOOTH` | Transitions that should not overshoot |
| `Spring::BOUNCY` | Playful motion with visible overshoot |
| `Spring::new(stiffness, damping, mass)` | Custom physics |

## Limits

- Clicks and hover use the laid-out position while a layer is moving.
- Layers cannot scale or rotate.
- Opacity applies to each painted item, so overlapping children show through each other.
- Native GPU surfaces move but do not fade.
- A layer ID must be unique within its window.
