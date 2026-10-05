//! CPU frame rasteriser.
//!
//! The Raspberry Pi's V3D GPU hangs when GPUI's glyph-atlas text pipeline is
//! used, so the device never asks GPUI to render text. Instead we compose the
//! entire 800×480 frame in software into a BGRA8 buffer and hand that one image
//! to GPUI to blit.
//!
//! Text is shaped with [`cosmic_text`] (rustybuzz under the hood) against a
//! bundled-only font database and rasterised with swash. Rasterised glyphs are
//! cached in a [`HashMap`] keyed by `(font id, glyph id, quantised px size)`.
//!
//! This module is compiled on every target and has no GPU/GPUI dependency, so
//! the compositor and its tests run on plain hosts and CI.

use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, CacheKey, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, SwashCache,
    SwashContent, Weight, Wrap,
};
use fontdb::ID as FontId;

/// Frame width in pixels.
pub const WIDTH: usize = 800;
/// Frame height in pixels.
pub const HEIGHT: usize = 480;

/// Bundled fonts, in the order they are loaded into the database.
const FONT_DATA: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Silkscreen-Regular.ttf"),
    include_bytes!("../assets/fonts/VT323-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSansSymbols2-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoEmoji-Regular.ttf"),
];

/// Fallback chain used for codepoints the requested family does not cover.
const FALLBACK_CHAIN: &[&str] = &[
    "Silkscreen",
    "VT323",
    "JetBrains Mono",
    "Noto Sans",
    "Noto Sans Symbols2",
    "Noto Emoji",
];

/// A fallback list that keeps the Screensight faces first, then the Noto
/// coverage fonts. Because we load no system fonts, this list is the complete
/// resolution chain.
struct DesignFallback;

impl cosmic_text::Fallback for DesignFallback {
    fn common_fallback(&self) -> &[&'static str] {
        FALLBACK_CHAIN
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, _script: unicode_script::Script, _locale: &str) -> &[&'static str] {
        // The common fallback list already covers everything we ship.
        &[]
    }
}

/// An sRGB colour with straight alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    /// Opaque colour from `0xRRGGBB`.
    #[must_use]
    pub const fn hex(rgb: u32) -> Self {
        Self(
            ((rgb >> 16) & 0xff) as u8,
            ((rgb >> 8) & 0xff) as u8,
            (rgb & 0xff) as u8,
            0xff,
        )
    }

    /// Opaque colour from components.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 0xff)
    }

    /// Colour from components including alpha.
    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(r, g, b, a)
    }

    /// Bytes in the frame buffer's channel order (BGRA).
    #[must_use]
    pub const fn to_bgra(self) -> [u8; 4] {
        [self.2, self.1, self.0, self.3]
    }

    /// Linear interpolation towards `other` by `t` in `[0, 1]`.
    #[must_use]
    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Self(
            f(self.0, other.0),
            f(self.1, other.1),
            f(self.2, other.2),
            f(self.3, other.3),
        )
    }
}

/// Horizontal alignment within a text box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    /// Flush left.
    Left,
    /// Centred within the box.
    Center,
    /// Flush right.
    Right,
}

/// How to lay out and paint a block of text.
#[derive(Clone, Copy, Debug)]
pub struct TextStyle<'a> {
    /// Family name requested from the bundled font database.
    pub family: &'a str,
    /// Font size in pixels.
    pub px: f32,
    /// Line height in pixels.
    pub line_height: f32,
    /// Font weight.
    pub weight: Weight,
    /// Text colour.
    pub color: Color,
    /// Wrap width. `<= 0` disables wrapping.
    pub max_width: f32,
    /// Per-line horizontal alignment.
    pub align: TextAlign,
}

impl Default for TextStyle<'_> {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono",
            px: 14.0,
            line_height: 18.0,
            weight: Weight::NORMAL,
            color: Color::rgb(0xff, 0xff, 0xff),
            max_width: 0.0,
            align: TextAlign::Left,
        }
    }
}

/// Bounding box of one rendered line of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineBox {
    /// Left edge in frame pixels.
    pub x: f32,
    /// Top edge in frame pixels.
    pub y: f32,
    /// Line width in pixels.
    pub w: f32,
    /// Line height in pixels.
    pub h: f32,
    /// Baseline offset from the frame top.
    pub baseline: f32,
}

/// A rasterised glyph mask ready to composite.
#[derive(Clone, Debug, Default)]
struct RasterGlyph {
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    content: u8,
    data: Vec<u8>,
}

impl RasterGlyph {
    const MASK: u8 = 0;
    const COLOR: u8 = 1;
}

/// A placed glyph returned by shaping, before rasterisation.
struct PlacedGlyph {
    x: f32,
    y: f32,
    font_id: FontId,
    glyph_id: u16,
    size: f32,
    flags: CacheKeyFlags,
}

/// One shaped line.
struct ShapedLine {
    x: f32,
    baseline: f32,
    top: f32,
    width: f32,
    height: f32,
    glyphs: Vec<PlacedGlyph>,
}

/// A complete 800×480 BGRA8 frame plus the text engine used to draw into it.
pub struct FrameCanvas {
    width: usize,
    height: usize,
    buf: Vec<u8>,
    font_system: FontSystem,
    swash: SwashCache,
    glyphs: HashMap<(FontId, u16, u32), RasterGlyph>,
}

impl Default for FrameCanvas {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCanvas {
    /// Create a new, cleared frame.
    #[must_use]
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        for bytes in FONT_DATA {
            db.load_font_data(bytes.to_vec());
        }
        db.set_sans_serif_family("Noto Sans");
        db.set_monospace_family("JetBrains Mono");
        db.set_serif_family("Noto Sans");
        let font_system =
            FontSystem::new_with_locale_and_db_and_fallback("en-US".to_owned(), db, DesignFallback);
        Self {
            width: WIDTH,
            height: HEIGHT,
            buf: vec![0; WIDTH * HEIGHT * 4],
            font_system,
            swash: SwashCache::new(),
            glyphs: HashMap::new(),
        }
    }

    /// Frame width in pixels.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// The raw BGRA8 frame buffer.
    #[must_use]
    pub fn frame(&self) -> &[u8] {
        &self.buf
    }

    /// Fill the whole frame with one colour.
    pub fn fill(&mut self, color: Color) {
        let bgra = color.to_bgra();
        for chunk in self.buf.as_chunks_mut::<4>().0 {
            chunk.copy_from_slice(&bgra);
        }
    }

    /// Composite one pixel with a coverage value (`0..=255`).
    fn blend(&mut self, x: i32, y: i32, color: Color, coverage: u8) {
        if coverage == 0 || color.3 == 0 {
            return;
        }
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let sa = (u32::from(color.3) * u32::from(coverage) + 127) / 255;
        if sa == 0 {
            return;
        }
        let inv = 255 - sa;
        let idx = (y as usize * self.width + x as usize) * 4;
        let (sr, sg, sb) = (u32::from(color.0), u32::from(color.1), u32::from(color.2));
        let buf = &mut self.buf;
        buf[idx] = ((sb * sa + u32::from(buf[idx]) * inv + 127) / 255) as u8;
        buf[idx + 1] = ((sg * sa + u32::from(buf[idx + 1]) * inv + 127) / 255) as u8;
        buf[idx + 2] = ((sr * sa + u32::from(buf[idx + 2]) * inv + 127) / 255) as u8;
        let da = u32::from(buf[idx + 3]);
        buf[idx + 3] = (sa + (da * inv + 127) / 255).min(255) as u8;
    }

    /// Composite one straight-alpha RGBA source pixel.
    fn blend_rgba(&mut self, x: i32, y: i32, r: u8, g: u8, b: u8, a: u8) {
        let color = Color::rgba(r, g, b, a);
        self.blend(x, y, color, 255);
    }

    /// Filled rectangle. Coordinates are clipped to the frame.
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width as i32);
        let y1 = (y + h).min(self.height as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, color, 255);
            }
        }
    }

    /// Single-pixel-tall horizontal line.
    pub fn hline(&mut self, x: i32, y: i32, w: i32, color: Color) {
        self.rect(x, y, w, 1, color);
    }

    /// Filled rectangle with anti-aliased rounded corners.
    pub fn rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: f32, color: Color) {
        if w <= 0 || h <= 0 {
            return;
        }
        let r = radius.max(0.0).min(w as f32 / 2.0).min(h as f32 / 2.0);
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width as i32);
        let y1 = (y + h).min(self.height as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                // Signed distance to the rounded rectangle: the pixel centre is
                // projected onto the inner rectangle, and the distance to that
                // inner rectangle is compared against the corner radius.
                let fx = px as f32 + 0.5;
                let fy = py as f32 + 0.5;
                let cx = fx.clamp(x as f32 + r, x as f32 + w as f32 - r);
                let cy = fy.clamp(y as f32 + r, y as f32 + h as f32 - r);
                let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                let coverage = if d <= r - 0.5 {
                    1.0
                } else if d >= r + 0.5 {
                    0.0
                } else {
                    r + 0.5 - d
                };
                if coverage > 0.0 {
                    let a = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                    self.blend(px, py, color, a);
                }
            }
        }
    }

    /// Shape and draw a block of text, returning the box of each rendered line.
    pub fn text(&mut self, x: f32, y: f32, text: &str, style: &TextStyle) -> Vec<LineBox> {
        let lines = self.shape_lines(text, x, style);
        let embolden = style.weight.0 >= 600;
        let mut boxes = Vec::with_capacity(lines.len());
        for line in &lines {
            for glyph in &line.glyphs {
                // Uncovered codepoints shape to glyph 0 (`.notdef`). Prefer the
                // font's own U+FFFD replacement glyph; fall back to a visible
                // tofu box so nothing is silently dropped.
                //
                // `glyph.x` is the shaped pen position (already absolute, in
                // frame pixels); drawing at the line origin would stack every
                // glyph on top of the first.
                let pen_x = glyph.x;
                if glyph.glyph_id == 0 {
                    match self.replacement_glyph(glyph.font_id) {
                        Some(gid) => {
                            let raster =
                                self.rasterize(glyph.font_id, gid, glyph.size, glyph.flags);
                            self.draw_glyph(
                                pen_x,
                                y,
                                line.baseline,
                                glyph.y,
                                &raster,
                                style.color,
                                embolden,
                            );
                        }
                        None => {
                            let raster = self.rasterize(glyph.font_id, 0, glyph.size, glyph.flags);
                            if raster.width == 0 || raster.height == 0 {
                                self.tofu(
                                    pen_x as i32,
                                    (y + line.baseline) as i32,
                                    glyph.size,
                                    style.color,
                                );
                            } else {
                                self.draw_glyph(
                                    pen_x,
                                    y,
                                    line.baseline,
                                    glyph.y,
                                    &raster,
                                    style.color,
                                    embolden,
                                );
                            }
                        }
                    }
                } else {
                    let raster =
                        self.rasterize(glyph.font_id, glyph.glyph_id, glyph.size, glyph.flags);
                    self.draw_glyph(
                        pen_x,
                        y,
                        line.baseline,
                        glyph.y,
                        &raster,
                        style.color,
                        embolden,
                    );
                }
            }
            boxes.push(LineBox {
                x: line.x,
                y: y + line.top,
                w: line.width,
                h: line.height,
                baseline: y + line.baseline,
            });
        }
        boxes
    }

    /// Shape text without drawing, returning `(width, height)` in pixels.
    pub fn measure(&mut self, text: &str, style: &TextStyle) -> (f32, f32) {
        let lines = self.shape_lines(text, 0.0, style);
        let mut w = 0.0f32;
        let mut h = 0.0f32;
        for line in &lines {
            w = w.max(line.x + line.width);
            h = (line.top + line.height).max(h);
        }
        (w, h)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_glyph(
        &mut self,
        x: f32,
        y: f32,
        baseline: f32,
        dy: f32,
        raster: &RasterGlyph,
        color: Color,
        embolden: bool,
    ) {
        if raster.width == 0 || raster.height == 0 {
            return;
        }
        let gx = x as i32 + raster.left;
        let gy = y as i32 + baseline as i32 + dy as i32 - raster.top;
        self.blit_glyph(gx, gy, raster, color);
        if embolden {
            // Faux bold: a one-pixel right-shifted copy thickens the stroke.
            self.blit_glyph(gx + 1, gy, raster, color);
        }
    }

    fn blit_glyph(&mut self, x: i32, y: i32, raster: &RasterGlyph, color: Color) {
        let w = raster.width as i32;
        let h = raster.height as i32;
        match raster.content {
            RasterGlyph::MASK => {
                let mut i = 0usize;
                for row in 0..h {
                    for col in 0..w {
                        let coverage = raster.data[i];
                        i += 1;
                        self.blend(x + col, y + row, color, coverage);
                    }
                }
            }
            RasterGlyph::COLOR => {
                let mut i = 0usize;
                for row in 0..h {
                    for col in 0..w {
                        let r = raster.data[i];
                        let g = raster.data[i + 1];
                        let b = raster.data[i + 2];
                        let a = raster.data[i + 3];
                        i += 4;
                        self.blend_rgba(x + col, y + row, r, g, b, a);
                    }
                }
            }
            _ => {}
        }
    }

    /// Shape text into owned line/glyph geometry so the font system borrow ends
    /// before rasterisation begins.
    fn shape_lines(&mut self, text: &str, x0: f32, style: &TextStyle) -> Vec<ShapedLine> {
        let px = style.px.max(1.0);
        let line_height = style.line_height.max(1.0);
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(px, line_height));
        let wrap_width = if style.max_width > 0.0 {
            Some(style.max_width)
        } else {
            None
        };
        buffer.set_size(&mut self.font_system, wrap_width, None);
        buffer.set_wrap(
            &mut self.font_system,
            if wrap_width.is_some() {
                Wrap::Word
            } else {
                Wrap::None
            },
        );
        // Every bundled face is regular (JetBrains Mono ships as a variable
        // font whose default instance is 400, and cosmic-text's fallback only
        // matches faces whose weight is an exact hit). Shaping therefore always
        // requests the regular instance; heavier roles are faux-emboldened at
        // raster time so they still resolve to the requested family.
        let attrs = Attrs::new()
            .family(Family::Name(style.family))
            .weight(Weight::NORMAL);
        buffer.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, true);

        let mut out = Vec::new();
        for run in buffer.layout_runs() {
            let dx = match style.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => ((style.max_width - run.line_w) / 2.0).max(0.0),
                TextAlign::Right => (style.max_width - run.line_w).max(0.0),
            }
            .round();
            let mut glyphs = Vec::with_capacity(run.glyphs.len());
            for glyph in run.glyphs {
                let physical = glyph.physical((x0 + dx, 0.0), 1.0);
                glyphs.push(PlacedGlyph {
                    x: physical.x as f32,
                    y: physical.y as f32,
                    font_id: glyph.font_id,
                    glyph_id: glyph.glyph_id,
                    size: glyph.font_size,
                    flags: glyph.cache_key_flags,
                });
            }
            out.push(ShapedLine {
                x: x0 + dx,
                baseline: run.line_y,
                top: run.line_top,
                width: run.line_w,
                height: run.line_height,
                glyphs,
            });
        }
        out
    }

    /// The U+FFFD replacement glyph in `font_id`, if it has one.
    fn replacement_glyph(&mut self, font_id: FontId) -> Option<u16> {
        let font = self.font_system.get_font(font_id)?;
        let gid = font.as_swash().charmap().map('\u{FFFD}');
        (gid != 0).then_some(gid)
    }

    /// Rasterise (and cache) one glyph.
    fn rasterize(
        &mut self,
        font_id: FontId,
        glyph_id: u16,
        size: f32,
        flags: CacheKeyFlags,
    ) -> RasterGlyph {
        // Quantise the size to quarter-pixels so nearby sizes share a cache
        // entry without visible stepping.
        let quantised = (size * 4.0).round().max(1.0) as u32;
        let key = (font_id, glyph_id, quantised);
        if let Some(cached) = self.glyphs.get(&key) {
            return cached.clone();
        }
        // Zero subpixel bins: glyphs are pixel-snapped, which keeps the cache
        // key independent of fractional positioning.
        let (cache_key, _, _) = CacheKey::new(font_id, glyph_id, size, (0.0, 0.0), flags);
        let raster = match self
            .swash
            .get_image_uncached(&mut self.font_system, cache_key)
        {
            Some(image) => RasterGlyph {
                width: image.placement.width,
                height: image.placement.height,
                left: image.placement.left,
                top: image.placement.top,
                content: match image.content {
                    SwashContent::Mask => RasterGlyph::MASK,
                    SwashContent::Color => RasterGlyph::COLOR,
                    _ => RasterGlyph::MASK,
                },
                data: image.data,
            },
            None => RasterGlyph::default(),
        };
        self.glyphs.insert(key, raster.clone());
        raster
    }

    /// A visible replacement box for a codepoint no bundled font can render.
    fn tofu(&mut self, x: i32, baseline_y: i32, size: f32, color: Color) {
        let w = (size * 0.55).round().max(4.0) as i32;
        let h = (size * 0.8).round().max(6.0) as i32;
        let top = baseline_y - h + 2;
        self.hline(x, top, w, color);
        self.hline(x, top + h - 1, w, color);
        self.rect(x, top, 1, h, color);
        self.rect(x + w - 1, top, 1, h, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles() -> TextStyle<'static> {
        TextStyle {
            family: "JetBrains Mono",
            px: 24.0,
            line_height: 28.0,
            weight: Weight::NORMAL,
            color: Color::rgb(0xff, 0xff, 0xff),
            max_width: 0.0,
            align: TextAlign::Left,
        }
    }

    #[test]
    fn canvas_has_expected_dimensions() {
        let canvas = FrameCanvas::new();
        assert_eq!(canvas.width(), WIDTH);
        assert_eq!(canvas.height(), HEIGHT);
        assert_eq!(canvas.frame().len(), WIDTH * HEIGHT * 4);
    }

    #[test]
    fn rect_writes_expected_bgra_pixel() {
        let mut canvas = FrameCanvas::new();
        canvas.fill(Color::hex(0x000000));
        let color = Color::hex(0x0bb2b3); // plumage-500
        canvas.rect(10, 20, 5, 5, color);
        let idx = (20 * WIDTH + 10) * 4;
        assert_eq!(&canvas.frame()[idx..idx + 4], &color.to_bgra());
    }

    #[test]
    fn text_changes_pixels_and_reports_lines() {
        let mut canvas = FrameCanvas::new();
        canvas.fill(Color::hex(0x000000));
        let boxes = canvas.text(20.0, 20.0, "Hello Screensight", &styles());
        assert!(!boxes.is_empty());
        assert!(boxes[0].w > 0.0);
        assert!(
            canvas.frame().iter().any(|&b| b != 0),
            "drawing text should change at least one pixel"
        );
    }

    /// Highest lit pixel column, used to check horizontal advance.
    fn lit_extent(canvas: &FrameCanvas) -> i32 {
        let mut max = 0i32;
        for y in 0..canvas.height() {
            for x in 0..canvas.width() {
                let idx = (y * canvas.width() + x) * 4;
                if canvas.frame()[idx] != 0 || canvas.frame()[idx + 1] != 0 {
                    max = max.max(x as i32);
                }
            }
        }
        max
    }

    #[test]
    fn glyphs_advance_horizontally() {
        // Regression: passing the line origin instead of each glyph's pen
        // position stacks every glyph on the first one.
        let mut one = FrameCanvas::new();
        one.fill(Color::hex(0x000000));
        one.text(10.0, 10.0, "M", &styles());

        let mut many = FrameCanvas::new();
        many.fill(Color::hex(0x000000));
        many.text(10.0, 10.0, "MMMM", &styles());

        let single = lit_extent(&one);
        let multiple = lit_extent(&many);
        assert!(
            multiple > single + 20,
            "text must advance horizontally (one={single}, many={multiple})"
        );
    }

    #[test]
    fn wrapping_splits_long_text_into_lines() {
        let mut canvas = FrameCanvas::new();
        let style = TextStyle {
            max_width: 120.0,
            ..styles()
        };
        let boxes = canvas.text(0.0, 0.0, "one two three four five six", &style);
        assert!(boxes.len() > 1, "expected wrapped output, got {boxes:?}");
    }

    #[test]
    fn bold_roles_still_resolve_to_the_requested_family() {
        // The variable JetBrains Mono face reports weight 400, so a naive
        // `Weight::BOLD` request would fall through to Silkscreen. Shaping must
        // keep the requested family and faux-bold at raster time instead.
        for weight in [Weight::NORMAL, Weight::BOLD] {
            let mut canvas = FrameCanvas::new();
            let style = TextStyle {
                family: "JetBrains Mono",
                weight,
                ..styles()
            };
            let lines = canvas.shape_lines("ABC", 0.0, &style);
            let font_id = lines[0].glyphs[0].font_id;
            let family = canvas
                .font_system
                .db()
                .face(font_id)
                .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
                .unwrap_or_default();
            assert_eq!(family, "JetBrains Mono", "weight {weight:?}");
        }
    }

    #[test]
    fn emoji_resolves_through_the_fallback_chain() {
        let mut canvas = FrameCanvas::new();
        let style = TextStyle {
            family: "Silkscreen",
            ..styles()
        };
        let lines = canvas.shape_lines("\u{1F980}", 0.0, &style);
        let font_id = lines[0].glyphs[0].font_id;
        let family = canvas
            .font_system
            .db()
            .face(font_id)
            .and_then(|f| f.families.first().map(|(name, _)| name.clone()))
            .unwrap_or_default();
        assert_eq!(family, "Noto Emoji");
    }

    #[test]
    fn emoji_and_unknown_codepoints_fall_back_without_panicking() {
        let mut canvas = FrameCanvas::new();
        canvas.fill(Color::hex(0x000000));
        // A crab (Noto Emoji), a snowman (symbols), an unassigned codepoint and
        // a non-BMP plane-16 codepoint.
        canvas.text(
            10.0,
            10.0,
            "crab \u{1F980} snow \u{2603} \u{10FFFF}",
            &styles(),
        );
        assert!(canvas.frame().iter().any(|&b| b != 0));
    }
}
