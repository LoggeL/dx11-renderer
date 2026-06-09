//! Self-contained TrueType loader + rasterizer, no dependencies.
//!
//! Parsing covers what glyph rendering needs: cmap (format 4), loca, glyf
//! (simple + composite), head, hhea, hmtx. Rasterization flattens the
//! quadratic outlines into line segments and accumulates signed coverage
//! per cell, resolved by a single linear prefix-sum pass — O(segments x
//! touched scanlines + pixels), exact anti-aliasing, no supersampling.

const MAX_COMPOSITE_DEPTH: u8 = 4;

const FONT_CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\consola.ttf",
    "C:\\Windows\\Fonts\\cour.ttf",
    "C:\\Windows\\Fonts\\segoeui.ttf",
];

/// Loads the first available monospace-ish system font.
pub fn load_system() -> Result<Font, String> {
    let data = FONT_CANDIDATES
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .ok_or("no system font found")?;
    Font::parse(data)
}

fn u16be(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}
fn i16be(d: &[u8], o: usize) -> i16 {
    u16be(d, o) as i16
}
fn u32be(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn f2dot14(d: &[u8], o: usize) -> f32 {
    i16be(d, o) as f32 / 16384.0
}

#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
    on: bool,
}

/// Affine transform: x' = a*x + b*y + e, y' = c*x + d*y + f
#[derive(Clone, Copy)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

const IDENTITY: Affine = Affine {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

impl Affine {
    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.b * y + self.e,
            self.c * x + self.d * y + self.f,
        )
    }
    /// self ∘ child (child applied first)
    fn compose(&self, m: Affine) -> Affine {
        Affine {
            a: self.a * m.a + self.b * m.c,
            b: self.a * m.b + self.b * m.d,
            c: self.c * m.a + self.d * m.c,
            d: self.c * m.b + self.d * m.d,
            e: self.a * m.e + self.b * m.f + self.e,
            f: self.c * m.e + self.d * m.f + self.f,
        }
    }
}

pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub new_line_size: f32,
}

pub struct RasterGlyph {
    pub width: usize,
    pub height: usize,
    /// left bearing of the bitmap relative to the pen position
    pub xmin: i32,
    /// bottom of the bitmap relative to the baseline (y up, usually negative)
    pub ymin: i32,
    pub advance: f32,
    /// width * height coverage values, row 0 = top
    pub coverage: Vec<u8>,
}

pub struct Font {
    data: Vec<u8>,
    units_per_em: f32,
    ascender: i16,
    descender: i16,
    line_gap: i16,
    long_loca: bool,
    num_hmetrics: u16,
    loca: usize,
    glyf: usize,
    hmtx: usize,
    cmap4: usize,
}

impl Font {
    pub fn parse(data: Vec<u8>) -> Result<Self, String> {
        if data.len() < 12 {
            return Err("font file too small".into());
        }
        let num_tables = u16be(&data, 4) as usize;
        if data.len() < 12 + num_tables * 16 {
            return Err("truncated table directory".into());
        }
        let table = |tag: &[u8; 4]| -> Option<usize> {
            (0..num_tables)
                .map(|i| 12 + i * 16)
                .find(|&o| &data[o..o + 4] == tag)
                .map(|o| u32be(&data, o + 8) as usize)
        };
        let head = table(b"head").ok_or("missing head table")?;
        let hhea = table(b"hhea").ok_or("missing hhea table")?;
        let hmtx = table(b"hmtx").ok_or("missing hmtx table")?;
        let loca = table(b"loca").ok_or("missing loca table (not a glyf font?)")?;
        let glyf = table(b"glyf").ok_or("missing glyf table")?;
        let cmap = table(b"cmap").ok_or("missing cmap table")?;

        // pick a unicode cmap subtable, format 4
        let n_enc = u16be(&data, cmap + 2) as usize;
        let mut cmap4 = None;
        let mut best = -1i32;
        for i in 0..n_enc {
            let rec = cmap + 4 + i * 8;
            let platform = u16be(&data, rec);
            let encoding = u16be(&data, rec + 2);
            let off = cmap + u32be(&data, rec + 4) as usize;
            let score = match (platform, encoding) {
                (3, 1) => 3,
                (0, _) => 2,
                (3, 10) => 1,
                _ => -1,
            };
            if score > best && u16be(&data, off) == 4 {
                best = score;
                cmap4 = Some(off);
            }
        }
        let cmap4 = cmap4.ok_or("no format-4 unicode cmap subtable")?;

        Ok(Self {
            units_per_em: u16be(&data, head + 18) as f32,
            long_loca: i16be(&data, head + 50) != 0,
            ascender: i16be(&data, hhea + 4),
            descender: i16be(&data, hhea + 6),
            line_gap: i16be(&data, hhea + 8),
            num_hmetrics: u16be(&data, hhea + 34).max(1),
            loca,
            glyf,
            hmtx,
            cmap4,
            data,
        })
    }

    pub fn line_metrics(&self, px: f32) -> LineMetrics {
        let s = px / self.units_per_em;
        let (asc, desc, gap) = (
            self.ascender as f32 * s,
            self.descender as f32 * s,
            self.line_gap as f32 * s,
        );
        LineMetrics {
            ascent: asc,
            descent: desc,
            line_gap: gap,
            new_line_size: asc - desc + gap,
        }
    }

    pub fn glyph_id(&self, c: u32) -> u16 {
        let d = &self.data;
        let t = self.cmap4;
        if c > 0xffff {
            return 0;
        }
        let segx2 = u16be(d, t + 6) as usize;
        for i in (0..segx2).step_by(2) {
            let end = u16be(d, t + 14 + i) as u32;
            if c > end {
                continue;
            }
            let start = u16be(d, t + 16 + segx2 + i) as u32;
            if c < start {
                return 0;
            }
            let delta = u16be(d, t + 16 + 2 * segx2 + i);
            let ro_at = t + 16 + 3 * segx2 + i;
            let ro = u16be(d, ro_at) as usize;
            if ro == 0 {
                return (c as u16).wrapping_add(delta);
            }
            let gi_at = ro_at + ro + 2 * (c - start) as usize;
            if gi_at + 1 >= d.len() {
                return 0;
            }
            let g = u16be(d, gi_at);
            return if g == 0 { 0 } else { g.wrapping_add(delta) };
        }
        0
    }

    fn advance_units(&self, gid: u16) -> f32 {
        let i = gid.min(self.num_hmetrics - 1) as usize;
        u16be(&self.data, self.hmtx + i * 4) as f32
    }

    /// Byte range of a glyph in the glyf table; None for empty glyphs.
    fn glyph_data(&self, gid: u16) -> Option<usize> {
        let d = &self.data;
        let i = gid as usize;
        let (start, end) = if self.long_loca {
            let at = self.loca + i * 4;
            if at + 8 > d.len() {
                return None;
            }
            (u32be(d, at) as usize, u32be(d, at + 4) as usize)
        } else {
            let at = self.loca + i * 2;
            if at + 4 > d.len() {
                return None;
            }
            (u16be(d, at) as usize * 2, u16be(d, at + 2) as usize * 2)
        };
        if end <= start {
            return None;
        }
        let off = self.glyf + start;
        (off + 10 <= d.len()).then_some(off)
    }

    /// Collects the outline (font units) of a glyph, recursing into composites.
    fn outline(&self, gid: u16, depth: u8, m: Affine, contours: &mut Vec<Vec<Point>>) {
        if depth > MAX_COMPOSITE_DEPTH {
            return;
        }
        let Some(g) = self.glyph_data(gid) else {
            return;
        };
        let d = &self.data;
        let n_contours = i16be(d, g);
        if n_contours >= 0 {
            self.simple_outline(g, n_contours as usize, m, contours);
            return;
        }
        // composite glyph
        const ARGS_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY: u16 = 0x0002;
        const HAVE_SCALE: u16 = 0x0008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const HAVE_XY_SCALE: u16 = 0x0040;
        const HAVE_2X2: u16 = 0x0080;
        let mut p = g + 10;
        loop {
            if p + 4 > d.len() {
                return;
            }
            let flags = u16be(d, p);
            let child = u16be(d, p + 2);
            p += 4;
            let (dx, dy) = if flags & ARGS_ARE_WORDS != 0 {
                let v = (i16be(d, p) as f32, i16be(d, p + 2) as f32);
                p += 4;
                v
            } else {
                let v = (d[p] as i8 as f32, d[p + 1] as i8 as f32);
                p += 2;
                v
            };
            let mut cm = Affine {
                e: dx,
                f: dy,
                ..IDENTITY
            };
            if flags & ARGS_ARE_XY == 0 {
                // point-matching composites are rare; skip the offset
                cm.e = 0.0;
                cm.f = 0.0;
            }
            if flags & HAVE_SCALE != 0 {
                cm.a = f2dot14(d, p);
                cm.d = cm.a;
                p += 2;
            } else if flags & HAVE_XY_SCALE != 0 {
                cm.a = f2dot14(d, p);
                cm.d = f2dot14(d, p + 2);
                p += 4;
            } else if flags & HAVE_2X2 != 0 {
                cm.a = f2dot14(d, p);
                cm.b = f2dot14(d, p + 2);
                cm.c = f2dot14(d, p + 4);
                cm.d = f2dot14(d, p + 6);
                p += 8;
            }
            self.outline(child, depth + 1, m.compose(cm), contours);
            if flags & MORE_COMPONENTS == 0 {
                return;
            }
        }
    }

    fn simple_outline(&self, g: usize, n_contours: usize, m: Affine, contours: &mut Vec<Vec<Point>>) {
        let d = &self.data;
        let mut end_pts = Vec::with_capacity(n_contours);
        for i in 0..n_contours {
            end_pts.push(u16be(d, g + 10 + 2 * i) as usize);
        }
        let Some(&last) = end_pts.last() else {
            return;
        };
        let n_points = last + 1;
        let ins_len = u16be(d, g + 10 + 2 * n_contours) as usize;
        let mut p = g + 12 + 2 * n_contours + ins_len;

        // flags with run-length repeats
        let mut flags = Vec::with_capacity(n_points);
        while flags.len() < n_points {
            if p >= d.len() {
                return;
            }
            let f = d[p];
            p += 1;
            flags.push(f);
            if f & 0x08 != 0 {
                if p >= d.len() {
                    return;
                }
                let r = d[p];
                p += 1;
                for _ in 0..r {
                    flags.push(f);
                }
            }
        }
        flags.truncate(n_points);

        // delta-encoded coordinates
        let read_coords = |short_bit: u8, same_bit: u8, p: &mut usize| -> Option<Vec<i32>> {
            let mut out = Vec::with_capacity(n_points);
            let mut v = 0i32;
            for &f in &flags {
                if f & short_bit != 0 {
                    let b = *d.get(*p)? as i32;
                    *p += 1;
                    v += if f & same_bit != 0 { b } else { -b };
                } else if f & same_bit == 0 {
                    if *p + 1 >= d.len() {
                        return None;
                    }
                    v += i16be(d, *p) as i32;
                    *p += 2;
                }
                out.push(v);
            }
            Some(out)
        };
        let Some(xs) = read_coords(0x02, 0x10, &mut p) else {
            return;
        };
        let Some(ys) = read_coords(0x04, 0x20, &mut p) else {
            return;
        };

        let mut start = 0usize;
        for &e in &end_pts {
            if e >= n_points || e < start {
                return;
            }
            if e - start + 1 >= 2 {
                let contour = (start..=e)
                    .map(|i| {
                        let (x, y) = m.apply(xs[i] as f32, ys[i] as f32);
                        Point {
                            x,
                            y,
                            on: flags[i] & 0x01 != 0,
                        }
                    })
                    .collect();
                contours.push(contour);
            }
            start = e + 1;
        }
    }

    pub fn rasterize(&self, ch: char, px: f32) -> RasterGlyph {
        let scale = px / self.units_per_em;
        let gid = self.glyph_id(ch as u32);
        let advance = self.advance_units(gid) * scale;
        let empty = |advance| RasterGlyph {
            width: 0,
            height: 0,
            xmin: 0,
            ymin: 0,
            advance,
            coverage: Vec::new(),
        };

        let mut contours = Vec::new();
        self.outline(gid, 0, IDENTITY, &mut contours);
        if contours.is_empty() {
            return empty(advance);
        }

        // flatten to line segments in pixel space (y still up)
        let mut segs: Vec<[f32; 4]> = Vec::new();
        for c in &mut contours {
            for pt in c.iter_mut() {
                pt.x *= scale;
                pt.y *= scale;
            }
            contour_segments(c, &mut segs);
        }
        if segs.is_empty() {
            return empty(advance);
        }

        // pixel bounding box over the flattened outline
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for s in &segs {
            min_x = min_x.min(s[0]).min(s[2]);
            max_x = max_x.max(s[0]).max(s[2]);
            min_y = min_y.min(s[1]).min(s[3]);
            max_y = max_y.max(s[1]).max(s[3]);
        }
        let xmin = min_x.floor() as i32;
        let ymin = min_y.floor() as i32;
        let xmax = max_x.ceil() as i32;
        let ymax = max_y.ceil() as i32;
        let w = (xmax - xmin).max(0) as usize;
        let h = (ymax - ymin).max(0) as usize;
        if w == 0 || h == 0 {
            return empty(advance);
        }

        // accumulate signed coverage deltas; +2 columns so the fractional
        // right-edge spill never goes out of bounds
        let aw = w + 2;
        let mut acc = vec![0.0f32; aw * h];
        for s in &segs {
            // shift into bitmap space and flip y down
            let p0 = (s[0] - xmin as f32, ymax as f32 - s[1]);
            let p1 = (s[2] - xmin as f32, ymax as f32 - s[3]);
            draw_line(&mut acc, aw, h, p0, p1);
        }

        // single prefix-sum pass resolves winding into coverage
        let mut coverage = vec![0u8; w * h];
        let mut sum = 0.0f32;
        for y in 0..h {
            let row = y * aw;
            for x in 0..aw {
                sum += acc[row + x];
                if x < w {
                    coverage[y * w + x] = (sum.abs().min(1.0) * 255.0 + 0.5) as u8;
                }
            }
        }

        RasterGlyph {
            width: w,
            height: h,
            xmin,
            ymin,
            advance,
            coverage,
        }
    }
}

fn mid(a: Point, b: Point) -> (f32, f32) {
    ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

/// Walks one contour (quadratic b-splines with implied on-curve midpoints)
/// and emits flattened line segments.
fn contour_segments(pts: &[Point], out: &mut Vec<[f32; 4]>) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let first_on = pts.iter().position(|p| p.on);
    let start = match first_on {
        Some(i) => (pts[i].x, pts[i].y),
        None => mid(pts[n - 1], pts[0]),
    };
    let begin = first_on.map_or(0, |i| i + 1);
    let endk = begin + n;
    let mut cur = start;
    let mut k = begin;
    while k < endk {
        let p = pts[k % n];
        if p.on {
            out.push([cur.0, cur.1, p.x, p.y]);
            cur = (p.x, p.y);
            k += 1;
        } else {
            let nxt = pts[(k + 1) % n];
            let end = if nxt.on {
                k += 2;
                (nxt.x, nxt.y)
            } else {
                k += 1;
                mid(p, nxt)
            };
            flatten_quad(cur, (p.x, p.y), end, out);
            cur = end;
        }
    }
    if cur != start {
        out.push([cur.0, cur.1, start.0, start.1]);
    }
}

fn flatten_quad(p0: (f32, f32), c: (f32, f32), p1: (f32, f32), out: &mut Vec<[f32; 4]>) {
    // max deviation from the chord is |p0 - 2c + p1| / 4; step count for a
    // ~0.12 px tolerance, so curves stay smooth at HUD sizes
    let dx = p0.0 - 2.0 * c.0 + p1.0;
    let dy = p0.1 - 2.0 * c.1 + p1.1;
    let dev = (dx * dx + dy * dy).sqrt();
    let n = ((dev * 2.0).sqrt().ceil() as usize).clamp(1, 32);
    let mut prev = p0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let pt = (
            u * u * p0.0 + 2.0 * u * t * c.0 + t * t * p1.0,
            u * u * p0.1 + 2.0 * u * t * c.1 + t * t * p1.1,
        );
        out.push([prev.0, prev.1, pt.0, pt.1]);
        prev = pt;
    }
}

/// Accumulates the signed area a line segment sweeps per cell (y down).
/// Each scanline the segment crosses distributes `dy * dir` across the
/// columns it spans, weighted by horizontal overlap — exact analytic AA.
fn draw_line(acc: &mut [f32], w: usize, h: usize, p0: (f32, f32), p1: (f32, f32)) {
    if p0.1 == p1.1 {
        return;
    }
    let (dir, top, bot) = if p0.1 < p1.1 {
        (1.0f32, p0, p1)
    } else {
        (-1.0f32, p1, p0)
    };
    let dxdy = (bot.0 - top.0) / (bot.1 - top.1);
    let y_start = (top.1.floor().max(0.0)) as usize;
    let y_end = (bot.1.ceil().min(h as f32)) as usize;
    let clamp_x = (w - 2) as f32;
    let mut x = top.0 + (y_start as f32 - top.1).max(0.0) * dxdy;
    for y in y_start..y_end {
        let row = y * w;
        let dy = ((y + 1) as f32).min(bot.1) - (y as f32).max(top.1);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let x0 = x0.clamp(0.0, clamp_x);
        let x1 = x1.clamp(0.0, clamp_x);
        let x0floor = x0.floor();
        let x0i = x0floor as usize;
        let x1ceil = x1.ceil();
        let x1i = x1ceil as usize;
        if x1i <= x0i + 1 {
            // the span stays within one column: split the area between it
            // and its right neighbor by the midpoint
            let xm = 0.5 * (x0 + x1) - x0floor;
            acc[row + x0i] += d * (1.0 - xm);
            acc[row + x0i + 1] += d * xm;
        } else {
            let s = (x1 - x0).recip();
            let x0f = x0 - x0floor;
            let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
            let x1f = x1 - x1ceil + 1.0;
            let am = 0.5 * s * x1f * x1f;
            acc[row + x0i] += d * a0;
            if x1i == x0i + 2 {
                acc[row + x0i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - x0f);
                acc[row + x0i + 1] += d * (a1 - a0);
                for xi in x0i + 2..x1i - 1 {
                    acc[row + xi] += d * s;
                }
                let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                acc[row + x1i - 1] += d * (1.0 - a2 - am);
            }
            acc[row + x1i] += d * am;
        }
        x = xnext;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load() -> Font {
        let data = std::fs::read("C:\\Windows\\Fonts\\consola.ttf").expect("consolas present");
        Font::parse(data).expect("parses")
    }

    #[test]
    fn maps_and_measures_ascii() {
        let font = load();
        assert_ne!(font.glyph_id('A' as u32), 0);
        assert_ne!(font.glyph_id('0' as u32), 0);
        let lm = font.line_metrics(17.0);
        assert!(lm.ascent > 5.0 && lm.ascent < 30.0);
        assert!(lm.new_line_size > lm.ascent);
    }

    #[test]
    fn rasterizes_all_ascii() {
        let font = load();
        for c in 32u8..=126 {
            let g = font.rasterize(c as char, 17.0);
            assert!(g.advance > 0.0, "advance of {:?}", c as char);
            assert_eq!(g.coverage.len(), g.width * g.height, "{:?}", c as char);
            if !(c as char).is_whitespace() {
                assert!(g.width > 0 && g.height > 0, "empty bitmap for {:?}", c as char);
            }
        }
    }

    #[test]
    fn glyph_has_solid_interior() {
        let font = load();
        let g = font.rasterize('M', 32.0);
        let solid = g.coverage.iter().filter(|&&v| v > 200).count();
        // an M at 32px should have a substantial fully-covered area
        assert!(solid > 50, "only {solid} solid pixels");
        // and coverage must not exceed the bitmap bounds claimed
        assert!(g.width < 40 && g.height < 40);
    }
}
