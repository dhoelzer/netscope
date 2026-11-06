//! Text drawing through Vello CPU's glyph run API.
//!
//! `RenderContext::glyph_run` takes glyph *ids*, not text, so this module owns
//! the codepoint→glyph mapping and the advance-width layout, using `skrifa`
//! (already in the tree as a `glifo` dependency).

use std::sync::Arc;

use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::FontRef;
use vello_cpu::kurbo::{Affine, Point};
use vello_cpu::peniko::{Blob, FontData};
use vello_cpu::{Glyph, RenderContext, Resources};

/// The bundled UI font (Apache-2.0, see `assets/LICENSE-Roboto.txt`).
pub const ROBOTO: &[u8] = include_bytes!("../../assets/Roboto-Regular.ttf");

/// A loaded font plus the metrics needed to lay out a string.
pub struct Font {
    data: FontData,
    face: FontRef<'static>,
    ascent: f32,
    descent: f32,
}

// The `FontRef` borrows `ROBOTO`, which is `&'static [u8]` from `include_bytes!`.
// `data` holds the same bytes behind an owned `Arc`, so the borrow cannot
// outlive the font.
unsafe impl Send for Font {}

impl Font {
    #[must_use]
    pub fn bundled() -> Self {
        let data = FontData::new(Blob::new(Arc::new(ROBOTO)), 0);
        let face = FontRef::from_index(ROBOTO, 0).expect("bundled Roboto must parse");
        let metrics = face.metrics(Size::new(1000.0), LocationRef::default());
        Self {
            data,
            face,
            ascent: metrics.ascent,
            descent: metrics.descent,
        }
    }

    /// Height above the baseline, scaled to `size` pixels per em.
    #[must_use]
    pub fn ascent(&self, size: f32) -> f32 {
        self.ascent * size / 1000.0
    }

    /// Depth below the baseline, scaled to `size` pixels per em.
    #[must_use]
    pub fn descent(&self, size: f32) -> f32 {
        self.descent * size / 1000.0
    }

    #[must_use]
    pub fn line_height(&self, size: f32) -> f32 {
        self.ascent(size) - self.descent(size)
    }

    /// Lays `text` out at `size`, returning glyph runs and the total advance.
    fn shape(&self, text: &str, size: f32) -> (Vec<Glyph>, f32) {
        let charmap = self.face.charmap();
        // `Size::new(size)` already puts advances in pixels, so they are used
        // as-is; only the global metrics (loaded at 1000 upem) need scaling.
        let metrics = self
            .face
            .glyph_metrics(Size::new(size), LocationRef::default());
        let mut glyphs = Vec::with_capacity(text.len());
        let mut pen_x = 0.0f32;
        for ch in text.chars() {
            let Some(id) = charmap.map(ch) else { continue };
            glyphs.push(Glyph {
                id: id.to_u32(),
                x: pen_x,
                y: 0.0,
            });
            pen_x += metrics.advance_width(id).unwrap_or(size * 0.5);
        }
        (glyphs, pen_x)
    }

    /// Width `text` would occupy at `size`.
    #[must_use]
    pub fn measure(&self, text: &str, size: f32) -> f32 {
        self.shape(text, size).1
    }

    /// Draws `text` with its baseline starting at `origin`.
    pub fn draw(
        &self,
        ctx: &mut RenderContext,
        resources: &mut Resources,
        origin: Point,
        text: &str,
        size: f32,
    ) -> f32 {
        let (glyphs, advance) = self.shape(text, size);
        if glyphs.is_empty() {
            return 0.0;
        }
        ctx.set_transform(Affine::translate((origin.x, origin.y)));
        let run = ctx
            .glyph_run(resources, &self.data)
            .font_size(size)
            .hint(true)
            .fill_glyphs(glyphs.into_iter());
        if run.is_err() {
            // A glyph the font cannot render is not worth killing the frame for.
            debug_assert!(false, "glyph run failed for {text:?}");
        }
        ctx.reset_transform();
        advance
    }

    /// Draws `text` centred horizontally on `center.x`, baseline at `baseline_y`.
    pub fn draw_centered(
        &self,
        ctx: &mut RenderContext,
        resources: &mut Resources,
        center: Point,
        baseline_y: f64,
        text: &str,
        size: f32,
    ) {
        let width = f64::from(self.measure(text, size));
        let x = center.x - width / 2.0;
        self.draw(ctx, resources, Point { x, y: baseline_y }, text, size);
    }

    /// Trims `text` to fit `max_width` at `size`, appending an ellipsis.
    #[must_use]
    pub fn fit(&self, text: &str, size: f32, max_width: f32) -> String {
        if self.measure(text, size) <= max_width {
            return text.to_string();
        }
        let mut out = String::new();
        for ch in text.chars() {
            let candidate = format!("{out}{ch}…");
            if self.measure(&candidate, size) > max_width {
                break;
            }
            out.push(ch);
        }
        out.push('…');
        out
    }
}

impl Default for Font {
    fn default() -> Self {
        Self::bundled()
    }
}
