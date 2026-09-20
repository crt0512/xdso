//! drawing the graticule and the traces.
//!
//! the python rendered into a fixed 800x400 pixel buffer with numpy and
//! blitted it. we paint egui shapes instead, so the plot resizes with the
//! window and stays sharp instead of getting stretched. the decimation is
//! still the same idea : one vertical span per screen column, from the min and
//! max of the samples that land in it, which is exactly what the scope itself
//! draws.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2};

use xdso_proto::{CENTRE_COUNT, COUNTS_PER_DIV, H_DIVS, V_DIVS};

use crate::theme;

/// the graticule, sized and placed inside whatever room weve got.
///
/// divisions are kept square like a real scope, so the graticule is always
/// 2:1 and gets letterboxed rather than stretched
pub struct Graticule {
    pub rect: Rect,
}

impl Graticule {
    pub fn fit(area: Rect) -> Graticule {
        let aspect = H_DIVS as f32 / V_DIVS as f32; // 2:1
        let mut size = Vec2::new(area.width(), area.width() / aspect);
        if size.y > area.height() {
            size = Vec2::new(area.height() * aspect, area.height());
        }
        Graticule { rect: Rect::from_center_size(area.center(), size) }
    }

    /// pixels per adc count
    fn px_per_count(&self) -> f32 {
        (self.rect.height() / V_DIVS as f32) / COUNTS_PER_DIV
    }

    /// a sample value -> the row it belongs on. samples already have the
    /// channels vertical position baked in by the scope, so theres nothing to
    /// add here
    pub fn y_of(&self, count: f32) -> f32 {
        self.rect.center().y - (count - CENTRE_COUNT) * self.px_per_count()
    }

    /// same going across, for xy mode
    pub fn x_of(&self, count: f32) -> f32 {
        self.rect.center().x + (count - CENTRE_COUNT) * self.px_per_count()
    }

    pub fn draw_grid(&self, p: &Painter, kind: u8) {
        let r = self.rect;
        p.rect_stroke(r, 0.0, Stroke::new(1.0, theme::GRID_EDGE), egui::StrokeKind::Inside);

        // kind 2 is "no grid" on the display menu
        if kind != 2 {
            // dotted, because a scope graticule is dotted. one mesh for the
            // lot so its a single draw rather than 1500 little shapes
            let step = if kind == 0 { 8.0 } else { 32.0 };
            let mut dots = Mesh::default();
            let dx = r.width() / H_DIVS as f32;
            let dy = r.height() / V_DIVS as f32;
            for i in 1..H_DIVS {
                let x = r.left() + i as f32 * dx;
                let mut y = r.top() + 4.0;
                while y < r.bottom() {
                    dots.add_colored_rect(
                        Rect::from_min_size(pos2(x, y), Vec2::splat(1.0)),
                        theme::GRID_DOT,
                    );
                    y += step;
                }
            }
            for j in 1..V_DIVS {
                let y = r.top() + j as f32 * dy;
                let mut x = r.left() + 4.0;
                while x < r.right() {
                    dots.add_colored_rect(
                        Rect::from_min_size(pos2(x, y), Vec2::splat(1.0)),
                        theme::GRID_DOT,
                    );
                    x += step;
                }
            }
            p.add(Shape::mesh(dots));
        }

        // centre cross is always drawn, its the only way to see where zero is
        let c = r.center();
        let axis = Stroke::new(1.0, theme::GRID_AXIS);
        p.line_segment([pos2(c.x, r.top()), pos2(c.x, r.bottom())], axis);
        p.line_segment([pos2(r.left(), c.y), pos2(r.right(), c.y)], axis);
    }

    /// one vertical span per column, clipped to the graticule.
    ///
    /// neighbouring columns get joined up so a fast edge stays a continuous
    /// line instead of a dotted one. without that a square wave looks like its
    /// missing its edges
    pub fn draw_wave(&self, p: &Painter, counts: &[u8], color: Color32, dots: bool) {
        if counts.is_empty() {
            return;
        }
        let r = self.rect;
        let cols = r.width().round().max(1.0) as usize;
        let mut lo = Vec::with_capacity(cols);
        let mut hi = Vec::with_capacity(cols);

        for c in 0..cols {
            let a = c * counts.len() / cols;
            let b = ((c + 1) * counts.len() / cols).max(a + 1).min(counts.len());
            let slice = &counts[a..b];
            let min = *slice.iter().min().unwrap_or(&128) as f32;
            let max = *slice.iter().max().unwrap_or(&128) as f32;
            // y is upside down relative to counts, so the max count is the top
            lo.push(self.y_of(max).clamp(r.top(), r.bottom()));
            hi.push(self.y_of(min).clamp(r.top(), r.bottom()));
        }

        if !dots {
            for c in 1..cols {
                lo[c] = lo[c].min(hi[c - 1]);
                hi[c] = hi[c].max(lo[c - 1]);
            }
        }

        let mut mesh = Mesh::default();
        for c in 0..cols {
            let x = r.left() + c as f32;
            if dots {
                mesh.add_colored_rect(Rect::from_min_size(pos2(x, lo[c]), Vec2::splat(1.0)), color);
                mesh.add_colored_rect(Rect::from_min_size(pos2(x, hi[c]), Vec2::splat(1.0)), color);
            } else {
                let h = (hi[c] - lo[c]).max(1.0);
                mesh.add_colored_rect(Rect::from_min_size(pos2(x, lo[c]), Vec2::new(1.0, h)), color);
            }
        }
        p.add(Shape::mesh(mesh));
    }

    /// the trigger level, as a marker against the right hand edge.
    ///
    /// `TRIG-VPOS` turns out to be in exactly the same counts as the channel
    /// position, centred on 128, so it maps through [`Graticule::y_of`] like
    /// any sample does. that was worth checking rather than assuming : pressing
    /// "trig 50%" and comparing against the traces own mid level gave 55 vs
    /// 54.5, 56 vs 56.0, 57 vs 57.0 as the channel was moved up the screen. it
    /// follows the trace, so it is not an absolute voltage
    pub fn draw_trigger(&self, p: &Painter, level: i32, colour: Color32) {
        let y = self.y_of(CENTRE_COUNT + level as f32);
        let r = self.rect;
        if !(r.top()..=r.bottom()).contains(&y) {
            return; // wound off the top or bottom of the graticule
        }

        // dashed, so it never gets mistaken for part of a trace
        let faint = colour.gamma_multiply(0.45);
        let mut x = r.left();
        while x < r.right() - 10.0 {
            p.line_segment([pos2(x, y), pos2((x + 5.0).min(r.right() - 10.0), y)],
                Stroke::new(1.0, faint));
            x += 11.0;
        }

        // and a little arrow on the edge, pointing in at the level it marks
        let base = r.right();
        p.add(Shape::convex_polygon(
            vec![pos2(base - 9.0, y), pos2(base, y - 5.0), pos2(base, y + 5.0)],
            colour,
            Stroke::NONE,
        ));
    }

    /// DISPLAY-FORMAT = XY : ch1 drives horizontal, ch2 vertical. lissajous
    /// figures and not a lot else.
    ///
    /// consecutive samples are consecutive in time, so joining them up is the
    /// honest thing to do and it fills in the gaps youd otherwise get on
    /// anything with fast edges. a square wave in XY is two dots and nothing
    /// in between until you draw the line
    pub fn draw_xy(&self, p: &Painter, ch1: &[u8], ch2: &[u8], dots: bool) {
        let mut pts: Vec<Pos2> = ch1
            .iter()
            .zip(ch2)
            .map(|(&a, &b)| Pos2::new(self.x_of(a as f32), self.y_of(b as f32)))
            .collect();
        // a trace that sits still gives you thousands of identical points,
        // which is just work for the tessellator
        pts.dedup();

        if dots {
            let r = self.rect;
            let mut mesh = Mesh::default();
            for pt in pts.iter().filter(|p| r.contains(**p)) {
                mesh.add_colored_rect(Rect::from_min_size(*pt, Vec2::splat(1.0)), theme::XY);
            }
            p.add(Shape::mesh(mesh));
            return;
        }

        // clip rather than filter : a segment with one end off screen still
        // wants drawing up to the edge
        let inside = p.with_clip_rect(self.rect);
        inside.add(Shape::line(pts, Stroke::new(1.0, theme::XY)));
    }
}
