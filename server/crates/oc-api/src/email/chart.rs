//! The monitor graph of notification emails: the group's series over about an hour around the
//! event, the thresholds as dashed lines and the line in the state's colour, as a PNG (2× for
//! sharp screens) drawn with tiny-skia.
//!
//! Axis labels use a small stroke font defined here (digits, `.`, `:`, `-`, `%`, `k`, `M`, `G`):
//! no font file and no system fonts, so the server image needs nothing more.

use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, StrokeDash, Transform};

/// Logical size (the `<img>`'s width and height); the PNG has twice as many pixels.
pub(crate) const WIDTH: u32 = 516;
pub(crate) const HEIGHT: u32 = 120;
const SCALE: f32 = 2.0;
/// At most this many points are drawn (the query asks for fewer).
pub(crate) const MAX_POINTS: usize = 600;

pub(crate) struct Chart {
    /// `(timestamp ms, value)`, sorted.
    pub points: Vec<(i64, f64)>,
    pub from: i64,
    pub to: i64,
    /// Threshold lines and their colours.
    pub thresholds: Vec<(f64, &'static str)>,
    /// The line's colour: the state the email is about.
    pub color: &'static str,
    /// The transition: shaded from here to the end.
    pub event_at: Option<i64>,
}

fn color(hex: &str, alpha: f32) -> Color {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    let mut c = Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, 255);
    c.set_alpha(alpha);
    c
}

fn paint(hex: &str, alpha: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(hex, alpha));
    p.anti_alias = true;
    p
}

// ---------------------------------------------------------------- stroke font

/// A glyph: its polylines and its advance.
type Glyph = (&'static [&'static [(f32, f32)]], f32);

/// Glyphs on a 4 × 6 grid (y down, 6 = baseline).
fn glyph(c: char) -> Option<Glyph> {
    Some(match c {
        '0' => (&[&[(1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0)]], 4.0),
        '1' => (&[&[(0.8, 1.2), (2.2, 0.0), (2.2, 6.0)], &[(0.8, 6.0), (3.6, 6.0)]], 4.0),
        '2' => (&[&[(0.0, 1.0), (1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 2.2), (0.0, 6.0), (4.0, 6.0)]], 4.0),
        '3' => (&[&[(0.0, 0.0), (4.0, 0.0), (1.8, 2.6), (3.0, 2.6), (4.0, 3.6), (4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0)]], 4.0),
        '4' => (&[&[(3.0, 6.0), (3.0, 0.0), (0.0, 4.2), (4.0, 4.2)]], 4.0),
        '5' => (&[&[(4.0, 0.0), (0.4, 0.0), (0.0, 2.8), (3.0, 2.6), (4.0, 3.6), (4.0, 5.0), (3.0, 6.0), (0.0, 6.0)]], 4.0),
        '6' => (&[&[(3.4, 0.0), (1.0, 0.0), (0.0, 1.0), (0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 3.6), (3.0, 2.6), (0.0, 2.6)]], 4.0),
        '7' => (&[&[(0.0, 0.0), (4.0, 0.0), (1.4, 6.0)]], 4.0),
        '8' => (
            &[
                &[(1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 1.9), (3.0, 2.8), (1.0, 2.8), (0.0, 3.8), (0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 3.8), (3.0, 2.8)],
                &[(1.0, 2.8), (0.0, 1.9), (0.0, 1.0), (1.0, 0.0)],
            ],
            4.0,
        ),
        '9' => (&[&[(4.0, 3.4), (1.0, 3.4), (0.0, 2.4), (0.0, 1.0), (1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 5.0), (3.0, 6.0), (0.6, 6.0)]], 4.0),
        '.' => (&[&[(0.6, 5.7), (0.6, 6.0)]], 1.2),
        ':' => (&[&[(0.6, 1.7), (0.6, 2.0)], &[(0.6, 5.7), (0.6, 6.0)]], 1.2),
        '-' => (&[&[(0.3, 3.4), (3.0, 3.4)]], 3.3),
        '%' => (&[&[(0.0, 6.0), (4.0, 0.0)], &[(0.4, 0.2), (1.4, 0.2), (1.4, 1.6), (0.4, 1.6), (0.4, 0.2)], &[(2.6, 4.4), (3.6, 4.4), (3.6, 5.8), (2.6, 5.8), (2.6, 4.4)]], 4.0),
        'k' => (&[&[(0.0, 0.0), (0.0, 6.0)], &[(3.2, 2.4), (0.0, 4.6)], &[(1.2, 3.8), (3.4, 6.0)]], 3.4),
        'M' => (&[&[(0.0, 6.0), (0.0, 0.0), (2.0, 3.6), (4.0, 0.0), (4.0, 6.0)]], 4.0),
        'G' => (&[&[(4.0, 1.0), (3.0, 0.0), (1.0, 0.0), (0.0, 1.0), (0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 3.4), (2.4, 3.4)]], 4.0),
        ' ' => (&[], 2.5),
        _ => return None,
    })
}

/// Cap height of the labels in logical pixels.
const CAP: f32 = 6.8;
const GAP: f32 = 1.5;

fn text_width(s: &str) -> f32 {
    let k = CAP / 6.0;
    let w: f32 = s.chars().filter_map(glyph).map(|(_, adv)| adv * k + GAP).sum();
    (w - GAP).max(0.0)
}

/// Draws `s` with its left edge at `x` and its baseline at `y` (logical pixels).
fn draw_text(pm: &mut Pixmap, s: &str, x: f32, y: f32, hex: &str) {
    let k = CAP / 6.0;
    let mut pb = PathBuilder::new();
    let mut at = x;
    for (lines, adv) in s.chars().filter_map(glyph) {
        for line in lines {
            for (i, (gx, gy)) in line.iter().enumerate() {
                let (px, py) = (at + gx * k, y - CAP + gy * k);
                if i == 0 {
                    pb.move_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
        }
        at += adv * k + GAP;
    }
    if let Some(path) = pb.finish() {
        let stroke = Stroke { width: 1.05, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        pm.stroke_path(&path, &paint(hex, 1.0), &stroke, Transform::from_scale(SCALE, SCALE), None);
    }
}

// ---------------------------------------------------------------- scales

/// About `n` round ticks covering `[lo, hi]`, and their step.
pub(crate) fn nice_ticks(lo: f64, hi: f64, n: usize) -> (Vec<f64>, f64) {
    let raw = ((hi - lo) / n.max(1) as f64).abs().max(f64::MIN_POSITIVE);
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    let mut ticks = Vec::new();
    let mut v = (lo / step).ceil() * step;
    while v <= hi + step * 1e-9 && ticks.len() < 12 {
        ticks.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        v += step;
    }
    (ticks, step)
}

/// A tick's label: `2.5`, `750`, `1.2k`, `3M`.
pub(crate) fn number_label(v: f64, step: f64) -> String {
    let (div, suffix) = match v.abs().max(step) {
        a if a >= 1e9 => (1e9, "G"),
        a if a >= 1e6 => (1e6, "M"),
        a if a >= 1e3 => (1e3, "k"),
        _ => (1.0, ""),
    };
    // Enough decimals to tell ticks apart (2.5 needs one, 0.004 three).
    let unit = step / div;
    let decimals = (0..=4).find(|d| ((unit * 10f64.powi(*d)) - (unit * 10f64.powi(*d)).round()).abs() < 1e-6).unwrap_or(4) as usize;
    let mut s = format!("{:.*}", decimals, v / div);
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" {
        s = "0".into();
    }
    format!("{s}{suffix}")
}

fn time_label(ms: i64) -> String {
    let minutes = ms.div_euclid(60_000).rem_euclid(1440);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

// ---------------------------------------------------------------- drawing

/// The PNG, or None when there is nothing to draw (no finite points) or encoding fails.
pub(crate) fn render(c: &Chart) -> Option<Vec<u8>> {
    let points: Vec<(i64, f64)> = c.points.iter().copied().filter(|p| p.1.is_finite() && p.0 >= c.from && p.0 <= c.to).take(MAX_POINTS).collect();
    if points.is_empty() || c.to <= c.from {
        return None;
    }
    let (w, h) = (WIDTH as f32, HEIGHT as f32);
    let mut pm = Pixmap::new((w * SCALE) as u32, (h * SCALE) as u32)?;
    pm.fill(Color::WHITE);
    let t = Transform::from_scale(SCALE, SCALE);

    // Value range: the points and the thresholds, padded.
    let values = points.iter().map(|p| p.1).chain(c.thresholds.iter().map(|t| t.0));
    let (mut lo, mut hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    if (hi - lo).abs() < f64::EPSILON {
        let pad = if hi.abs() > 0.0 { hi.abs() * 0.1 } else { 1.0 };
        lo -= pad;
        hi += pad;
    } else {
        let pad = (hi - lo) * 0.08;
        hi += pad;
        lo = if lo >= 0.0 && lo - pad < 0.0 { 0.0 } else { lo - pad };
    }
    let (ticks, step) = nice_ticks(lo, hi, 3);
    lo = lo.min(ticks.first().copied().unwrap_or(lo));
    hi = hi.max(ticks.last().copied().unwrap_or(hi));
    let labels: Vec<String> = ticks.iter().map(|v| number_label(*v, step)).collect();
    let gutter = labels.iter().map(|l| text_width(l)).fold(0.0, f32::max) + 12.0;
    let (x0, x1, y0, y1) = (gutter.max(28.0), w - 8.0, 10.0, h - 24.0);
    let x_of = |ms: i64| x0 + (x1 - x0) * ((ms - c.from) as f32 / (c.to - c.from) as f32);
    let y_of = |v: f64| y1 - (y1 - y0) * ((v - lo) / (hi - lo)) as f32;

    // Grid and value labels.
    let grid = Stroke { width: 1.0, ..Stroke::default() };
    for (v, label) in ticks.iter().zip(&labels) {
        let y = y_of(*v);
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y);
        pb.line_to(x1, y);
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &paint("#eef2f1", 1.0), &grid, t, None);
        }
        draw_text(&mut pm, label, x0 - 8.0 - text_width(label), y + CAP / 2.0, "#7a8a87");
    }
    // Time labels on round minutes.
    let span_min = (c.to - c.from) / 60_000;
    let every = [5, 10, 15, 20, 30, 60, 120, 180, 360, 720, 1440].into_iter().find(|m| span_min / m <= 3).unwrap_or(1440) * 60_000;
    let mut at = (c.from + every - 1).div_euclid(every) * every;
    while at <= c.to {
        let label = time_label(at);
        let tw = text_width(&label);
        let x = (x_of(at) - tw / 2.0).clamp(x0, x1 - tw);
        draw_text(&mut pm, &label, x, h - 7.0, "#7a8a87");
        at += every;
    }

    // Since the transition: a light band in the state's colour.
    if let Some(e) = c.event_at.filter(|e| *e > c.from && *e < c.to) {
        if let Some(r) = Rect::from_ltrb(x_of(e), y0, x1, y1) {
            pm.fill_rect(r, &paint(c.color, 0.07), t, None);
        }
    }

    // The series: area, then line, broken where points are missing.
    let gap = gap_limit(&points);
    let mut segments: Vec<Vec<(f32, f32)>> = vec![Vec::new()];
    for (i, (ms, v)) in points.iter().enumerate() {
        if i > 0 && ms - points[i - 1].0 > gap {
            segments.push(Vec::new());
        }
        if let Some(s) = segments.last_mut() {
            s.push((x_of(*ms), y_of(*v)));
        }
    }
    let line = Stroke { width: 2.0, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    for s in &segments {
        let mut area = PathBuilder::new();
        let mut stroke = PathBuilder::new();
        for (i, (x, y)) in s.iter().enumerate() {
            if i == 0 {
                area.move_to(*x, y1);
                stroke.move_to(*x, *y);
            } else {
                stroke.line_to(*x, *y);
            }
            area.line_to(*x, *y);
        }
        if let (Some(last), Some(first)) = (s.last(), s.first()) {
            area.line_to(last.0, y1);
            area.line_to(first.0, y1);
            area.close();
        }
        if let Some(p) = area.finish() {
            pm.fill_path(&p, &paint(c.color, 0.10), FillRule::Winding, t, None);
        }
        if s.len() > 1 {
            if let Some(p) = stroke.finish() {
                pm.stroke_path(&p, &paint(c.color, 1.0), &line, t, None);
            }
        }
    }
    // Thresholds over the series, dashed.
    for (v, hex) in &c.thresholds {
        let y = y_of(*v);
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y);
        pb.line_to(x1, y);
        let dashed = Stroke { width: 1.5, dash: StrokeDash::new(vec![5.0, 4.0], 0.0), ..Stroke::default() };
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &paint(hex, 1.0), &dashed, t, None);
        }
    }
    // The latest point.
    if let Some((ms, v)) = points.last() {
        if let Some(p) = PathBuilder::from_circle(x_of(*ms), y_of(*v), 3.5) {
            pm.fill_path(&p, &paint(c.color, 1.0), FillRule::Winding, t, None);
        }
    }
    pm.encode_png().ok()
}

/// Points further apart than this are drawn as a gap (no data in between).
fn gap_limit(points: &[(i64, f64)]) -> i64 {
    let mut steps: Vec<i64> = points.windows(2).map(|w| w[1].0 - w[0].0).filter(|d| *d > 0).collect();
    steps.sort_unstable();
    steps.get(steps.len() / 2).map(|m| m * 5).unwrap_or(i64::MAX).max(60_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart(points: Vec<(i64, f64)>) -> Chart {
        Chart { points, from: 0, to: 3_600_000, thresholds: vec![(5.0, "#b42318")], color: "#b42318", event_at: Some(3_000_000) }
    }

    #[test]
    fn renders_a_png_at_twice_the_size() {
        let pts = (0..60).map(|i| (i * 60_000, 2.0 + (i as f64 / 10.0))).collect();
        let png = render(&chart(pts)).expect("png");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR: width and height (big endian) at bytes 16..24.
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), WIDTH * 2);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), HEIGHT * 2);
    }

    #[test]
    fn nothing_to_draw_is_none() {
        assert!(render(&chart(vec![])).is_none());
        assert!(render(&chart(vec![(60_000, f64::NAN)])).is_none());
        // A single point or a flat line still draws.
        assert!(render(&chart(vec![(60_000, 3.0)])).is_some());
        assert!(render(&Chart { thresholds: vec![], ..chart(vec![(0, 0.0), (60_000, 0.0)]) }).is_some());
    }

    #[test]
    fn ticks_and_labels_are_round() {
        assert_eq!(nice_ticks(0.0, 7.5, 3), (vec![0.0, 2.5, 5.0, 7.5], 2.5));
        assert_eq!(number_label(2.5, 2.5), "2.5");
        assert_eq!(number_label(5.0, 2.5), "5");
        assert_eq!(number_label(1500.0, 500.0), "1.5k");
        assert_eq!(number_label(2_000_000.0, 1_000_000.0), "2M");
        assert_eq!(number_label(0.004, 0.002), "0.004");
        assert_eq!(time_label(14 * 3_600_000 + 5 * 60_000), "14:05");
        // Every label character has a glyph.
        for s in ["0123456789", ".:-%", "kMG"] {
            assert!(s.chars().all(|c| glyph(c).is_some()), "{s}");
        }
    }
}
