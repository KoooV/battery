//! Tray icon rendering.
//!
//! Icons are drawn with `tiny-skia` and converted into the ARGB32 pixmaps the
//! StatusNotifierItem protocol expects: a coloured background, the percentage
//! and a colour that turns red before the battery is empty.
//!
//! The digits come from a small built-in bitmap font, so the binary needs no
//! font files and the result stays crisp at tray sizes.

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

/// Width and height of a generated icon, in pixels.
pub const ICON_SIZE: u32 = 32;

/// At or below this level the icon is red and the item asks for attention.
pub const CRITICAL_LEVEL: u8 = 10;

/// At or below this level the icon is amber.
pub const WARNING_LEVEL: u8 = 30;

/// Corner radius of the icon background, in pixels.
const CORNER_RADIUS: f32 = 7.0;

/// Glyphs are 3 pixels wide and 5 tall.
const GLYPH_WIDTH: u32 = 3;
const GLYPH_HEIGHT: u32 = 5;
/// Space between two glyphs, in font pixels.
const GLYPH_GAP: u32 = 1;

/// Digits, one byte per row; bit 2 is the leftmost pixel of the row.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Drawn instead of digits when the level is unknown.
const UNKNOWN_GLYPH: [u8; 5] = [0b111, 0b001, 0b010, 0b000, 0b010];

fn green() -> Color {
    Color::from_rgba8(56, 142, 60, 255)
}

fn amber() -> Color {
    Color::from_rgba8(245, 158, 11, 255)
}

fn red() -> Color {
    Color::from_rgba8(198, 40, 40, 255)
}

fn grey() -> Color {
    Color::from_rgba8(120, 120, 120, 255)
}

/// Background colour for a charge level; an unknown level is neutral grey.
fn background(level: Option<u8>) -> Color {
    match level {
        None => grey(),
        Some(level) if level <= CRITICAL_LEVEL => red(),
        Some(level) if level <= WARNING_LEVEL => amber(),
        Some(_) => green(),
    }
}

/// Text drawn inside the icon: the percentage, or `?` when it is unknown.
fn label(level: Option<u8>) -> String {
    match level {
        Some(level) => level.min(100).to_string(),
        None => "?".to_owned(),
    }
}

/// Font scale that keeps the label inside the icon.
fn font_scale(label: &str) -> u32 {
    match label.chars().count() {
        0 | 1 => 4,
        2 => 3,
        _ => 2,
    }
}

/// Glyph for one character of the label.
fn glyph(ch: char) -> Option<[u8; 5]> {
    if let Some(digit) = ch.to_digit(10) {
        return DIGITS.get(digit as usize).copied();
    }

    (ch == '?').then_some(UNKNOWN_GLYPH)
}

/// Renders the icon for a charge level.
///
/// Returns `None` only when `tiny-skia` refuses to allocate the pixmap; the size
/// is a constant, so callers can treat that as "no icon" without panicking.
pub fn render(level: Option<u8>) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(ICON_SIZE, ICON_SIZE)?;
    pixmap.fill(Color::TRANSPARENT);

    let path = rounded_square(ICON_SIZE as f32, CORNER_RADIUS)?;
    let mut paint = Paint::default();
    paint.set_color(background(level));
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    draw_label(&mut pixmap, &label(level));
    Some(pixmap)
}

/// Offset from `x` to the right by one font pixel.
fn draw_label(pixmap: &mut Pixmap, text: &str) {
    let scale = font_scale(text);
    let count = text.chars().count() as u32;
    let gaps = count.saturating_sub(1);

    let text_width = (count * GLYPH_WIDTH + gaps * GLYPH_GAP) * scale;
    let text_height = GLYPH_HEIGHT * scale;
    let origin_x = ICON_SIZE.saturating_sub(text_width) / 2;
    let origin_y = ICON_SIZE.saturating_sub(text_height) / 2;

    let mut paint = Paint::default();
    paint.set_color(Color::WHITE);

    for (index, ch) in text.chars().enumerate() {
        let Some(rows) = glyph(ch) else { continue };
        let glyph_x = origin_x + index as u32 * (GLYPH_WIDTH + GLYPH_GAP) * scale;

        for (row, bits) in rows.iter().enumerate() {
            for column in 0..GLYPH_WIDTH {
                let lit = bits & (1 << (GLYPH_WIDTH - 1 - column)) != 0;
                if !lit {
                    continue;
                }

                let x = glyph_x + column * scale;
                let y = origin_y + row as u32 * scale;
                let Some(rect) = Rect::from_xywh(x as f32, y as f32, scale as f32, scale as f32)
                else {
                    continue;
                };
                pixmap.fill_rect(rect, &paint, Transform::identity(), None);
            }
        }
    }
}

/// Square with rounded corners, built from four edges and four quadratic bends.
fn rounded_square(size: f32, radius: f32) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let (left, top, right, bottom) = (0.0, 0.0, size, size);

    builder.move_to(left + radius, top);
    builder.line_to(right - radius, top);
    builder.quad_to(right, top, right, top + radius);
    builder.line_to(right, bottom - radius);
    builder.quad_to(right, bottom, right - radius, bottom);
    builder.line_to(left + radius, bottom);
    builder.quad_to(left, bottom, left, bottom - radius);
    builder.line_to(left, top + radius);
    builder.quad_to(left, top, left + radius, top);
    builder.finish()
}

/// Converts a pixmap into ARGB32 data in network byte order, as the tray
/// protocol wants it.
pub fn to_argb32(pixmap: &Pixmap) -> Vec<u8> {
    let mut argb = Vec::with_capacity(pixmap.data().len());

    for &[r, g, b, a] in pixmap.data().as_chunks::<4>().0 {
        let (r, g, b) = straighten(r, g, b, a);
        argb.extend_from_slice(&[a, r, g, b]);
    }

    argb
}

/// `tiny-skia` keeps premultiplied colours, the tray protocol wants straight
/// ones.
fn straighten(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    if a == 0 {
        return (0, 0, 0);
    }
    if a == 255 {
        return (r, g, b);
    }

    let alpha = u16::from(a);
    let up = |channel: u8| -> u8 { (u16::from(channel) * 255 / alpha).min(255) as u8 };
    (up(r), up(g), up(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Colour of one pixel, straight (non-premultiplied) alpha.
    fn pixel(pixmap: &Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let data = to_argb32(pixmap);
        let offset = ((y * ICON_SIZE + x) * 4) as usize;
        (
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset],
        )
    }

    #[test]
    fn icon_has_the_documented_size() {
        let pixmap = render(Some(50)).expect("icon must render");

        assert_eq!(pixmap.width(), ICON_SIZE);
        assert_eq!(pixmap.height(), ICON_SIZE);
        assert_eq!(
            to_argb32(&pixmap).len(),
            (ICON_SIZE * ICON_SIZE * 4) as usize
        );
    }

    #[test]
    fn corners_are_transparent() {
        let pixmap = render(Some(50)).expect("icon must render");

        assert_eq!(pixel(&pixmap, 0, 0).3, 0);
        assert_eq!(pixel(&pixmap, ICON_SIZE - 1, 0).3, 0);
    }

    #[test]
    fn background_colour_follows_the_level() {
        assert_eq!(background(Some(80)), green());
        assert_eq!(background(Some(31)), green());
        assert_eq!(background(Some(30)), amber());
        assert_eq!(background(Some(11)), amber());
        assert_eq!(background(Some(10)), red());
        assert_eq!(background(Some(0)), red());
        assert_eq!(background(None), grey());
    }

    #[test]
    fn background_is_opaque_and_matches_the_level_colour() {
        let pixmap = render(Some(5)).expect("icon must render");

        // Top centre is inside the background but above the digits.
        assert_eq!(pixel(&pixmap, ICON_SIZE / 2, 2), (198, 40, 40, 255));
    }

    #[test]
    fn digits_are_drawn_in_white() {
        let pixmap = render(Some(55)).expect("icon must render");
        let data = to_argb32(&pixmap);

        let whites = data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| **pixel == [255, 255, 255, 255])
            .count();

        assert!(whites > 20, "only {whites} white pixels drawn");
    }

    #[test]
    fn different_levels_render_different_icons() {
        let two_digits = render(Some(55)).expect("icon must render");
        let three_digits = render(Some(100)).expect("icon must render");
        let unknown = render(None).expect("icon must render");

        assert_ne!(two_digits.data(), three_digits.data());
        assert_ne!(two_digits.data(), unknown.data());
    }

    #[test]
    fn label_is_the_percentage_or_a_question_mark() {
        assert_eq!(label(Some(7)), "7");
        assert_eq!(label(Some(100)), "100");
        assert_eq!(label(Some(120)), "100");
        assert_eq!(label(None), "?");
    }

    #[test]
    fn scale_shrinks_for_longer_labels() {
        assert_eq!(font_scale("7"), 4);
        assert_eq!(font_scale("55"), 3);
        assert_eq!(font_scale("100"), 2);
    }

    #[test]
    fn label_always_fits_inside_the_icon() {
        for text in ["7", "55", "100", "?"] {
            let scale = font_scale(text);
            let count = text.chars().count() as u32;
            let gaps = count.saturating_sub(1);
            let width = (count * GLYPH_WIDTH + gaps * GLYPH_GAP) * scale;
            let height = GLYPH_HEIGHT * scale;

            assert!(width <= ICON_SIZE, "{text} is {width}px wide");
            assert!(height <= ICON_SIZE, "{text} is {height}px tall");
        }
    }

    #[test]
    fn argb_bytes_are_in_network_order() {
        let pixmap = render(Some(5)).expect("icon must render");
        let data = to_argb32(&pixmap);
        let offset = ((2 * ICON_SIZE + ICON_SIZE / 2) * 4) as usize;

        assert_eq!(&data[offset..offset + 4], &[255, 198, 40, 40]);
    }

    #[test]
    fn semi_transparent_pixels_are_unpremultiplied() {
        // Premultiplied half-transparent white becomes straight white.
        assert_eq!(straighten(128, 128, 128, 128), (255, 255, 255));
        assert_eq!(straighten(0, 0, 0, 0), (0, 0, 0));
        assert_eq!(straighten(10, 20, 30, 255), (10, 20, 30));
    }
}
