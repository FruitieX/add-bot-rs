//! Correct glyph positioning for all Plotters-generated raster chart text.
use plotters_backend::{
    text_anchor::{HPos, VPos},
    BackendColor, BackendCoord, BackendStyle, BackendTextStyle, DrawingBackend, DrawingErrorKind,
    FontStyle,
};

// The shared font layout is below the backend implementation.

pub(crate) struct ChartBackend<B>(pub B);

impl<B: DrawingBackend> DrawingBackend for ChartBackend<B> {
    type ErrorType = B::ErrorType;

    fn get_size(&self) -> (u32, u32) {
        self.0.get_size()
    }
    fn ensure_prepared(&mut self) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        self.0.ensure_prepared()
    }
    fn present(&mut self) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        self.0.present()
    }
    fn draw_pixel(
        &mut self,
        point: BackendCoord,
        color: BackendColor,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        self.0.draw_pixel(point, color)
    }

    // Retain the bitmap backend's optimized line and rectangle rendering.
    fn draw_line<S: BackendStyle>(
        &mut self,
        from: BackendCoord,
        to: BackendCoord,
        style: &S,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        self.0.draw_line(from, to, style)
    }
    fn draw_rect<S: BackendStyle>(
        &mut self,
        from: BackendCoord,
        to: BackendCoord,
        style: &S,
        fill: bool,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        self.0.draw_rect(from, to, style, fill)
    }

    fn estimate_text_size<S: BackendTextStyle>(
        &self,
        text: &str,
        style: &S,
    ) -> Result<(u32, u32), DrawingErrorKind<Self::ErrorType>> {
        let layout = layout_text(
            text,
            style.size().round() as u32,
            matches!(style.style(), FontStyle::Bold),
        );
        Ok(layout
            .bounds
            .map(|bounds| {
                (
                    (bounds.max.x - bounds.min.x).ceil() as u32,
                    (bounds.max.y - bounds.min.y).ceil() as u32,
                )
            })
            .unwrap_or((0, 0)))
    }

    fn draw_text<S: BackendTextStyle>(
        &mut self,
        text: &str,
        style: &S,
        position: BackendCoord,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        let color = style.color();
        if color.alpha == 0.0 {
            return Ok(());
        }
        let layout = layout_text(
            text,
            style.size().round() as u32,
            matches!(style.style(), FontStyle::Bold),
        );
        let Some(bounds) = layout.bounds else {
            return Ok(());
        };
        let anchor = style.anchor();
        let x = match anchor.h_pos {
            HPos::Left => bounds.min.x,
            HPos::Center => (bounds.min.x + bounds.max.x) / 2.0,
            HPos::Right => bounds.max.x,
        }
        .round() as i32;
        let y = match anchor.v_pos {
            VPos::Top => bounds.min.y,
            VPos::Center => (bounds.min.y + bounds.max.y) / 2.0,
            VPos::Bottom => bounds.max.y,
        }
        .round() as i32;
        let transform = style.transform();
        let (width, height) = self.get_size();
        for glyph in layout.glyphs {
            let bounds = glyph.px_bounds();
            let mut failure = None;
            glyph.draw(|px, py, coverage| {
                if coverage <= 0.0 || failure.is_some() {
                    return;
                }
                let (dx, dy) = transform.transform(
                    bounds.min.x as i32 + px as i32 - x,
                    bounds.min.y as i32 + py as i32 - y,
                );
                let point = (position.0 + dx, position.1 + dy);
                if point.0 >= 0 && point.1 >= 0 && point.0 < width as i32 && point.1 < height as i32
                {
                    failure = self
                        .0
                        .draw_pixel(
                            point,
                            BackendColor {
                                alpha: color.alpha * coverage as f64,
                                rgb: color.rgb,
                            },
                        )
                        .err();
                }
            });
            if let Some(error) = failure {
                return Err(error);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plotters::{prelude::*, style::text_anchor::Pos};
    use plotters_backend::FontTransform;

    #[test]
    fn plotters_text_matches_results_renderer_and_preserves_rotation() {
        let render = |rotation| {
            let mut buffer = vec![255; 320 * 160 * 3];
            {
                let root = ChartBackend(BitMapBackend::with_buffer(&mut buffer, (320, 160)))
                    .into_drawing_area();
                root.draw(&Text::new(
                    "jAV fj",
                    (160, 80),
                    TextStyle::from(("sans-serif", 31))
                        .pos(Pos::new(HPos::Center, VPos::Center))
                        .transform(rotation),
                ))
                .unwrap();
            }
            image::RgbImage::from_raw(320, 160, buffer).unwrap()
        };
        let horizontal = render(FontTransform::None);
        let vertical = render(FontTransform::Rotate90);
        let pixels = |image: &image::RgbImage| {
            image
                .enumerate_pixels()
                .filter(|(_, _, p)| p.0 != [255; 3])
                .map(|(x, y, p)| ((x as i32, y as i32), p.0))
                .collect::<Vec<_>>()
        };
        let horizontal_pixels = pixels(&horizontal);
        assert!(!horizontal_pixels.is_empty());
        for ((x, y), color) in horizontal_pixels {
            assert_eq!(
                vertical
                    .get_pixel((160 - (y - 80)) as u32, (80 + (x - 160)) as u32)
                    .0,
                color
            );
        }
        let mut reference = vec![255; 320 * 160 * 3];
        let mut backend = BitMapBackend::with_buffer(&mut reference, (320, 160));
        let layout = layout_text("jAV fj", 31, false);
        let bounds = layout.bounds.unwrap();
        let x = ((bounds.min.x + bounds.max.x) / 2.0).round() as i32;
        let y = ((bounds.min.y + bounds.max.y) / 2.0).round() as i32;
        for glyph in layout.glyphs {
            let bounds = glyph.px_bounds();
            glyph.draw(|px, py, coverage| {
                if coverage > 0.0 {
                    backend
                        .draw_pixel(
                            (
                                160 + bounds.min.x as i32 + px as i32 - x,
                                80 + bounds.min.y as i32 + py as i32 - y,
                            ),
                            BackendColor {
                                rgb: (0, 0, 0),
                                alpha: coverage as f64,
                            },
                        )
                        .unwrap();
                }
            });
        }
        drop(backend);
        assert_eq!(horizontal.as_raw(), &reference);
    }
}

use ab_glyph::{point, Font, FontRef, OutlinedGlyph, Rect, ScaleFont};
use std::sync::LazyLock;

pub(crate) static REGULAR_FONT: LazyLock<FontRef<'static>> = LazyLock::new(|| {
    FontRef::try_from_slice(include_bytes!("../../assets/Roboto-Regular.ttf"))
        .expect("bundled regular font must be valid")
});
static BOLD_FONT: LazyLock<FontRef<'static>> = LazyLock::new(|| {
    FontRef::try_from_slice(include_bytes!("../../assets/Roboto-Bold.ttf"))
        .expect("bundled bold font must be valid")
});

pub(crate) struct TextLayout {
    pub(crate) glyphs: Vec<OutlinedGlyph>,
    pub(crate) bounds: Option<Rect>,
}

pub(crate) fn layout_text(value: &str, size: u32, bold: bool) -> TextLayout {
    let font = if bold { &*BOLD_FONT } else { &*REGULAR_FONT };
    let scaled = font.as_scaled(size as f32);
    let mut cursor = 0.0;
    let mut previous = None;
    let mut layout = TextLayout {
        glyphs: Vec::new(),
        bounds: None,
    };
    for character in value.chars() {
        let mut glyph = scaled.scaled_glyph(character);
        if let Some(previous) = previous {
            cursor += scaled.kern(previous, glyph.id);
        }
        // Outline at its fractional position, including the glyph's side bearings.
        glyph.position = point(cursor, 0.0);
        cursor += scaled.h_advance(glyph.id);
        previous = Some(glyph.id);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            layout.bounds = Some(match layout.bounds {
                Some(old) => Rect {
                    min: point(old.min.x.min(bounds.min.x), old.min.y.min(bounds.min.y)),
                    max: point(old.max.x.max(bounds.max.x), old.max.y.max(bounds.max.y)),
                },
                None => bounds,
            });
            layout.glyphs.push(outlined);
        }
    }
    layout
}
