//! Shared light-card styling and correctly laid out text for raster charts.
use color_eyre::{eyre::eyre, Result};
use plotters::{
    coord::Shift,
    prelude::*,
    style::text_anchor::{HPos, Pos, VPos},
};

use super::chart_text::layout_text;

pub(crate) const WIDTH: u32 = 1500;
pub(crate) const BACKGROUND: RGBColor = RGBColor(243, 246, 249);
pub(crate) const INK: RGBColor = RGBColor(25, 43, 54);
pub(crate) const MUTED: RGBColor = RGBColor(101, 118, 129);
pub(crate) const BORDER: RGBColor = RGBColor(229, 234, 240);
pub(crate) const ACCENT: RGBColor = RGBColor(54, 137, 153);

pub(crate) fn frame<B: DrawingBackend>(root: &DrawingArea<B, Shift>) -> Result<()> {
    let (width, height) = root.dim_in_pixel();
    let right = width as i32 - 30;
    let bottom = height as i32 - 32;
    root.fill(&BACKGROUND).map_err(|e| eyre!("{e:?}"))?;
    for corners in [
        [(54, 30), (right - 24, bottom)],
        [(30, 54), (right, bottom - 24)],
    ] {
        root.draw(&Rectangle::new(corners, WHITE.filled()))
            .map_err(|e| eyre!("{e:?}"))?;
    }
    for center in [
        (54, 54),
        (right - 24, 54),
        (54, bottom - 24),
        (right - 24, bottom - 24),
    ] {
        root.draw(&Circle::new(center, 24, WHITE.filled()))
            .map_err(|e| eyre!("{e:?}"))?;
    }
    Ok(())
}

pub(crate) fn text<B: DrawingBackend>(
    root: &DrawingArea<B, Shift>,
    value: &str,
    position: (i32, i32),
    size: u32,
    color: RGBColor,
    bold: bool,
) -> Result<()> {
    let style = TextStyle::from(("sans-serif", size).into_font().style(if bold {
        FontStyle::Bold
    } else {
        FontStyle::Normal
    }))
    .color(&color)
    .pos(Pos::new(HPos::Left, VPos::Top));
    root.draw(&Text::new(value, position, style))
        .map_err(|e| eyre!("{e:?}"))?;
    Ok(())
}

pub(crate) fn fit_text(value: &str, size: u32, max_width: u32) -> String {
    let fits = |value: &str| {
        layout_text(value, size, false)
            .bounds
            .is_none_or(|b| b.max.x - b.min.x <= max_width as f32)
    };
    if fits(value) {
        return value.to_owned();
    }
    let mut shortened = value.to_owned();
    while !shortened.is_empty() {
        shortened.pop();
        let candidate = format!("{shortened}…");
        if fits(&candidate) {
            return candidate;
        }
    }
    "…".into()
}

/// Wrap explanatory text so increasing the font never removes its meaning.
pub(crate) fn wrap_text(value: &str, size: u32, max_width: u32) -> Vec<String> {
    let fits = |value: &str| {
        layout_text(value, size, false)
            .bounds
            .is_none_or(|b| b.max.x - b.min.x <= max_width as f32)
    };
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if fits(&candidate) {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for character in word.chars() {
            let candidate = format!("{line}{character}");
            if !line.is_empty() && !fits(&candidate) {
                lines.push(std::mem::take(&mut line));
            }
            line.push(character);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[derive(Debug, Clone)]
pub(crate) struct ChartPanel {
    pub rows: Vec<(String, String)>,
    pub notes: Vec<String>,
}

impl ChartPanel {
    pub fn height(&self) -> u32 {
        let lines: usize = self
            .notes
            .iter()
            .map(|note| wrap_text(note, 36, 1300).len())
            .sum();
        150 + self.rows.len() as u32 * 56 + lines as u32 * 44
    }
}
