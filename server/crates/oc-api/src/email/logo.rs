//! The logo every email carries, as a 96 px PNG drawn once from the geometry of
//! `web/public/logo.svg` (mail clients don't show SVG): the teal square, the white ring and the
//! amber line. Keep the two in step.

use std::sync::OnceLock;

use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub(crate) const SIZE: u32 = 96;

fn paint(r: u8, g: u8, b: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(r, g, b, 255));
    p.anti_alias = true;
    p
}

fn draw() -> Option<Vec<u8>> {
    let mut pm = Pixmap::new(SIZE, SIZE)?;
    // The SVG's 64-unit view box.
    let t = Transform::from_scale(SIZE as f32 / 64.0, SIZE as f32 / 64.0);
    // <rect width="64" height="64" rx="14" fill="#0b5f5c"/>
    let (r, k) = (14.0f32, 14.0f32 * 0.552_284_8);
    let mut pb = PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(64.0 - r, 0.0);
    pb.cubic_to(64.0 - r + k, 0.0, 64.0, r - k, 64.0, r);
    pb.line_to(64.0, 64.0 - r);
    pb.cubic_to(64.0, 64.0 - r + k, 64.0 - r + k, 64.0, 64.0 - r, 64.0);
    pb.line_to(r, 64.0);
    pb.cubic_to(r - k, 64.0, 0.0, 64.0 - r + k, 0.0, 64.0 - r);
    pb.line_to(0.0, r);
    pb.cubic_to(0.0, r - k, r - k, 0.0, r, 0.0);
    pb.close();
    pm.fill_path(&pb.finish()?, &paint(0x0b, 0x5f, 0x5c), FillRule::Winding, t, None);
    // <circle cx="32" cy="32" r="17" stroke="#fff" stroke-width="6"/>
    let ring = PathBuilder::from_circle(32.0, 32.0, 17.0)?;
    pm.stroke_path(&ring, &paint(0xff, 0xff, 0xff), &Stroke { width: 6.0, ..Stroke::default() }, t, None);
    // <path d="M15 40 L25 30 L32 36 L44 22 L50 27" stroke="#ffb547" stroke-width="5" round caps and joins/>
    let mut line = PathBuilder::new();
    line.move_to(15.0, 40.0);
    for (x, y) in [(25.0, 30.0), (32.0, 36.0), (44.0, 22.0), (50.0, 27.0)] {
        line.line_to(x, y);
    }
    let stroke = Stroke { width: 5.0, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    pm.stroke_path(&line.finish()?, &paint(0xff, 0xb5, 0x47), &stroke, t, None);
    pm.encode_png().ok()
}

/// The logo PNG (empty only if drawing failed, which a test rules out).
pub(crate) fn png() -> &'static [u8] {
    static LOGO: OnceLock<Vec<u8>> = OnceLock::new();
    LOGO.get_or_init(|| draw().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    #[test]
    fn logo_is_a_96px_png_in_the_brand_colour() {
        let png = super::png();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 96);
        let pm = tiny_skia::Pixmap::decode_png(png).unwrap();
        // A corner of the square (inside the rounding) is the brand teal.
        let p = pm.pixel(48, 4).unwrap();
        assert_eq!((p.red(), p.green(), p.blue()), (0x0b, 0x5f, 0x5c));
    }
}
