# Vello — distilled reference

Sources: <https://github.com/linebender/vello> (README, `vello_cpu/README.md`,
`vello_cpu/examples/*`, `vello_cpu/src/*`, `vello_cpu/examples/winit/src/main.rs`),
<https://docs.rs/vello_cpu>.

## Which crate to use (important, changed in 2025/2026)

The repo name is `vello`, but the repo now ships **three** renderers:

| Crate | Role | Notes |
|---|---|---|
| `vello_cpu` | CPU-only, SIMD + multithreaded rasterizer | **Most mature for CPU/software rendering.** No GPU, no wgpu, no Vulkan/Metal. |
| `vello_gpu` | CPU path preprocess + GPU raster/composite (wgpu or WebGL2) | Intended future primary GPU path, still maturing. |
| `vello` (v0.11.0) | Original compute-shader renderer, now under `research/` | Experimental; needs a compute-capable GPU. |

This project uses **`vello_cpu`** because it has zero GPU/driver requirements,
which keeps the macOS/Windows/Linux build story trivial (no `wgpu`, no
Vulkan/Metal/DX12 probing, no shader compilation). It renders into a `Pixmap`
that we blit into a window with `softbuffer`.

Versions used here: `vello_cpu = "0.3.0"`, `vello_common = "0.3.0"` (re-exported
by `vello_cpu`, so it usually does not need to be a direct dependency),
`glifo = 0.4.0` (text, pulled in transitively by the `text` feature).

MSRV: Vello CPU 0.3.0 requires **Rust 1.89+**. Edition 2024.

## `vello_cpu` default features

`default = ["std", "png", "text", "u8_pipeline"]`.
Optional: `multithreading` (rayon), `f32_pipeline`.
At least one of `u8_pipeline` / `f32_pipeline` must be on; `OptimizeQuality`
needs `f32_pipeline`.

## Core object model

Vello CPU is **deferred**, not immediate-mode. Three objects:

```rust
use vello_cpu::{Level, Pixmap, RasterizerSettings, RenderContext, RenderMode,
                RenderSettings, Resources};
use vello_cpu::kurbo::{Affine, Circle, Point, Rect, Shape, BezPath};
use vello_cpu::color::palette::css::{RED, BLUE};
```

1. `RenderContext::new(width: u16, height: u16)` — reusable command buffer for a
   fixed scene size. Reuse it across frames; call `reset()` /
   `reset_and_resize(w, h)` between frames so allocations are reused.
2. `Resources::new()` — per-`RenderContext` resource store (glyph atlas, image
   registry). **One `Resources` per `RenderContext`, always paired.**
3. `Pixmap` — the RGBA8 output buffer (`Pixmap::new(w, h)`,
   `pixmap.resize(w, h)`, `pixmap.data() -> &[PremulRgba8]`).

### Draw loop shape

```rust
ctx.reset();                       // or reset_and_resize on resize
ctx.set_paint(BLUE);               // paint is state
ctx.fill_path(&Circle::new(c, r).to_path(0.1));
ctx.set_paint(RED.with_alpha(0.5));
ctx.fill_rect(&Rect::from_points((x0, y0), (x1, y1)));
ctx.set_stroke(Stroke { width: 2.0, ..Stroke::DEFAULT });
ctx.stroke_path(&line_path);
ctx.flush();                       // recommended always; required for multithreading
ctx.render(&mut pixmap, &mut resources);
```

Key `RenderContext` methods (from `src/render.rs`):

* state: `set_paint(impl Into<PaintType>)`, `set_stroke(Stroke)`,
  `set_transform(Affine)`, `set_paint_transform(Affine)`, `set_fill_rule(Fill)`,
  `set_blend_mode(BlendMode)`, `set_aliasing_threshold(Option<u8>)`
* shapes: `fill_path(&BezPath)`, `stroke_path(&BezPath)`, `fill_rect(&Rect)`,
  `stroke_rect(&Rect)`, `fill_blurred_rounded_rect(..)`
* layers/clips: `push_layer`, `push_clip_layer(&BezPath)`, `push_clip_rect`,
  `push_opacity_layer(f32)`, `push_blend_layer(BlendMode)`, `pop_layer`,
  `pop_clip`
* text: `glyph_run(&mut resources, &FontData) -> GlyphRunBuilder`
* finish: `flush()`, `render(target: impl Into<PixmapMut>, &mut Resources)`,
  `render_with(target, &mut Resources, RasterizerSettings)`

`RenderSettings { level: Level::new(), num_threads: 0 }` — `Level::new()` picks
host SIMD; `Level::fallback()` is the deterministic/portable path.
`RasterizerSettings { render_mode: RenderMode::OptimizeSpeed, ..Default::default() }`.

> Note: `render()` takes `&self` — the context is not mutably borrowed during
> rasterization, so `ctx` can be reused immediately after.

## Paths / geometry (`kurbo`, re-exported as `vello_cpu::kurbo`)

* `Point { x, y }`, `Vec2`, `Size`
* `Rect::new(x0, y0, x1, y1)`, `Rect::from_points((x0,y0),(x1,y1))`
* `Circle::new((cx, cy), radius)` — implements `Shape`
* `Line::new((x0,y0),(x1,y1))`
* `Shape::to_path(&self, tolerance: 0.1) -> BezPath` — the generic way to turn any
  shape into a path for `fill_path` / `stroke_path`.
* `Affine::translate((x,y))`, `*` composition, `then_translate`,
  `then_scale_about`, `then_rotate_about`.
* `BezPath` builder: `move_to`, `line_to`, `curve_to`, `close_path`.

Y axis points **down** (screen coordinates), origin top-left.

## Colour (`vello_cpu::color`, re-exported `peniko`/`color`)

* `color::palette::css::NAME` constants (e.g. `css::STEEL_BLUE`).
* `Rgba8 { r, g, b, a }` / `Rgba { r: f32, .. }`; `.with_alpha(f32)`,
  `.premultiply()`, `.to_rgba8()`.
* `PremulRgba8 { r, g, b, a }` is the pixmap pixel type.
* `PaintType` accepts colours, gradients, images, `Fill`/`Stroke` already set via
  `set_stroke`.

## Text (`text` feature, `glifo` under the hood)

```rust
use vello_cpu::peniko::{Blob, FontData};
use vello_cpu::Glyph;

const FONT: &[u8] = include_bytes!("../assets/Roboto-Regular.ttf"); // must embed or load
let font = FontData::new(Blob::new(std::sync::Arc::new(FONT)), 0);

// Glyph ids must be resolved from the font yourself (or use a shaper like swash/rustybuzz).
ctx.glyph_run(&mut resources, &font)
   .font_size(16.0)
   .glyph_run(glyphs_iter)   // iterator of `Glyph { id, x, y }`
   .fill_glyphs()             // -> Result<(), GlyphRunError>
   .unwrap();
```

Builder methods available on `GlyphRunBuilder`: `.font_size(f32)`,
`.glyph_run(impl Iterator<Item = Glyph>)`, `.atlas_cache(bool)`,
`.fill_glyphs(iter)`, `.stroke_glyphs(..)`, `.build(..)`.
`Glyph { id: u16, x: f32, y: f32 }` positions are in **font units scaled by
font_size**, i.e. `x`/`y` are pen offsets in pixels relative to the run origin
set by the current transform.

**Practical constraint:** `glyph_run` needs glyph *IDs*, not codepoints. For an
MVP the simplest reliable route is a tiny cmap lookup (Roboto's cmap format 4)
or, as done here, a small bitmap font drawn as filled rects. Both are
implemented in `src/render/text.rs`; the glyph-run path is kept as the
"real Vello text" option.

Roboto font: `assets/roboto/Roboto-Regular.ttf` in the vello repo, Apache-2.0,
with `assets/roboto/LICENSE.txt` (Apache 2.0).

## Windowing without a GPU: `winit` + `softbuffer`

This is the pattern used by the official `vello_cpu/examples/winit` example.

```rust
use winit::application::ApplicationHandler;
use winit::event::{WindowEvent, ElementState};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

enum RenderState {
    Active { window: Rc<Window>, surface: softbuffer::Surface<Rc<Window>, Rc<Window>> },
    Suspended,
}
```

* `event_loop.set_control_flow(ControlFlow::Wait)` +
  `window.request_redraw()` on a timer for animation.
* `event_loop.create_window(Window::default_attributes().with_inner_size(PhysicalSize::new(w,h)))`
* `softbuffer::Context::new(window.clone())` → `softbuffer::Surface::new(&ctx, window.clone())`
* On `WindowEvent::Resized(size)`:
  `surface.resize(NonZeroU32::new(w).unwrap(), NonZeroU32::new(h).unwrap())`,
  `pixmap.resize(w as u16, h as u16)`, `ctx.reset_and_resize(w as u16, h as u16)`.
* On `RedrawRequested`:
  ```rust
  let mut buffer = surface.buffer_mut().unwrap();
  for (dst, src) in buffer.iter_mut().zip(pixmap.data()) {
      *dst = u32::from_le_bytes([src.b, src.g, src.r, 0]); // XRGB little-endian
  }
  buffer.present().unwrap();
  ```
* `softbuffer` = pure-software window surface → no GPU/driver dependency at all.
* `Rc<Window>` is required (softbuffer needs shared ownership); winit 0.30 API.
* `ApplicationHandler` trait (winit ≥ 0.30) with `resumed`, `window_event`,
  `suspended`, `about_to_wait`.

## Gotchas

* `Pixmap` width/height are `u16` → cap window size at 65535.
* `RenderContext::new` takes `u16` too.
* `Pixmap::data()` is **premultiplied** RGBA; do not un-premultiply before blitting.
* Complex filter graphs panic; avoid `Filter` entirely (also unsupported with
  `multithreading`).
* Glyph atlas caching is off unless `.atlas_cache(true)`; enabling it allocates a
  2048×2048 atlas.
* `vello_cpu` re-exports `kurbo`, `peniko`, `color`, `Pixmap`, `Level` via
  `vello_common::reexports::cpu::*`, so `use vello_cpu::kurbo::...` works and a
  direct `vello_common` dependency is unnecessary.

## Corrections found while building this project (0.3.0 / 0.44 / 0.3.3)

These are the signatures that actually compile, as opposed to what the docs
pages suggested:

* Colours come from the `color` crate re-export: `vello_cpu::color::{AlphaColor,
  Srgb}`. There is **no** `vello_cpu::color::space` module. `Srgb` and
  `LinearSrgb` are re-exported at the crate root alongside `AlphaColor`.
  `type Paint = AlphaColor<Srgb>; const fn rgb(r,g,b) -> Paint { Paint::new([r,g,b,1.0]) }`
  works in a `const` context, and `.with_alpha(a)` scales alpha in place.
* `Pixmap::data()` returns `&[PremulRgba8]` — **one struct per pixel**, not
  `&[u8]`. So `data().len() == width * height`, and fields are `.r .g .b .a`.
* `kurbo::Stroke` has no associated `DEFAULT` const in this version; use
  `Stroke::new(width)` or `Stroke { width, ..Stroke::default() }`.
* `kurbo::Rect` (0.13) has **no `center()`** method — compute it from `x0/x1/y0/y1`.
  It does have `width()`, `height()`, `union()`, `inset()`, `contains()`.
  `BezPath` builds with `move_to`/`line_to`, not `push_move_to`/`push_line_to`;
  for a straight segment `Line::new(a, b).to_path(0.1)` is shorter than
  hand-building the path.
* `stroke_rect` takes **one** argument (`&Rect`) — unlike `fill_rect` it has no
  radii/second parameter.
* `RenderContext::render(&mut pixmap, &mut resources)` takes the `Resources`
  directly — no wrapper or reborrow helper.
* skrifa: `face.glyph_metrics(Size::new(px), LocationRef::default())` needs an
  explicit `LocationRef` (the bare `Default::default()` is ambiguous). Advances
  returned by `glyph_metrics` are **already in pixels** — do not divide by upem
  or multiply by `size/1000.0` again, or every label renders ~10× too wide.
  Only `face.metrics(Size::new(1000.0))`-style global metrics need scaling.
* glifo (`ctx.glyph_run()` behind vello_cpu's `text` feature):
  `.font_data(&FontData).font_size(px).hint(false).fill_glyphs(glyphs)`.
  `FontData::new(Blob::new(Arc::new(bytes)), 0)` — `Blob::new` wants
  `Arc<dyn AsRef<[u8]> + Send + Sync>`.

## Dashed strokes: supported, via kurbo's `Stroke`

`vello_cpu` 0.3.0 strokes paths through `kurbo::stroke_with(path, style,
&StrokeOpts::default(), tolerance, ctx)` (`vello_common-0.3.0/src/flatten.rs`),
so kurbo's dash fields **are** honoured. Grep for `dash` in `vello_cpu`/
`vello_common` and you find nothing — the support lives entirely in `kurbo`
(`kurbo-0.13.1/src/stroke.rs`: `dash_pattern`, `dash_offset`) — so the absence
of the word in vello is *not* evidence that dashes are unsupported.

```rust
use vello_cpu::kurbo::{Cap, Stroke};

ctx.set_stroke(Stroke {
    dash_pattern: vec![5.0, 5.0].into(), // on/off lengths, repeating
    start_cap: Cap::Butt,                // round caps lengthen every dash
    end_cap: Cap::Butt,
    ..Stroke::new(1.0)
});
ctx.stroke_path(&line_path(a, b));
```

Verified empirically: a `[5.0, 5.0]` pattern over a 180px line rasterises to 18
runs / 126 lit px, not one solid run.

API notes found along the way:
* `Dashes` is a type alias for `SmallVec<[f64; 4]>`, not a struct — build it
  with `vec![on, off].into()`. There is no `Dashes::new(vec, offset)`.
* The dash offset field is `dash_offset`, not `start_offset`/`end_offset`.
* `kurbo::Shape::to_path(tolerance)` needs the `Shape` trait in scope
  (`use vello_cpu::kurbo::Shape;`), otherwise `Line::new(..).to_path(0.1)`
  fails to resolve.
* `RenderContext` has `stroke_path(&BezPath)` / `stroke_rect(&Rect)` — there is
  no `stroke(&Shape)` overload, and no `clear()`; paint a full-size rect instead.
* `Stroke::new(w)` is `const` and defaults to `Join::Round` + `Cap::Round` on
  both ends, so set `Cap::Butt` explicitly when dash length must be exact.
