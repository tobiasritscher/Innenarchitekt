//! Classic CV wall detection on the binarised plan.
//!
//! Walls are thick dark strokes; text, dimension lines and furniture are thin. We estimate the
//! stroke width distribution, then extract axis-aligned walls by grouping run lengths
//! (column runs → horizontal walls, row runs → vertical walls) and diagonal walls from the
//! remaining thick-stroke components (PCA).

use imageproc::distance_transform::Norm;
use imageproc::region_labelling::{connected_components, Connectivity};

use super::preprocess::Mask;
use super::P;

/// Wall candidate: centre line endpoints (px), thickness (px), score 0..1.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub a: P,
    pub b: P,
    pub thickness: f64,
    pub score: f64,
}

impl Segment {
    pub fn len(&self) -> f64 {
        super::dist(self.a, self.b)
    }
}

#[derive(Clone, Debug, Default)]
pub struct StrokeStats {
    /// Minimum stroke width treated as wall (px).
    pub tmin: usize,
    /// Maximum wall thickness (px).
    pub tmax: usize,
    /// Dominant wall thickness (px).
    pub wall_mode: usize,
}

pub struct CvResult {
    pub segments: Vec<Segment>,
    pub stats: StrokeStats,
}

pub fn detect(ink: &Mask) -> CvResult {
    let stats = stroke_stats(ink);
    let maxdim = ink.w.max(ink.h) as f64;
    let min_len = (0.012 * maxdim).max(10.0);
    let mut segs = Vec::new();
    for s in bands(ink, &stats) {
        segs.push(s);
    }
    let t = ink.transpose();
    for s in bands(&t, &stats) {
        segs.push(Segment { a: [s.a[1], s.a[0]], b: [s.b[1], s.b[0]], ..s });
    }
    segs.retain(|s| s.len() >= (1.5 * s.thickness).max(min_len));
    let diag = diagonal_segments(ink, &stats, &segs, min_len);
    segs.extend(diag);
    CvResult { segments: segs, stats }
}

/// Dominant skew (degrees, image rotation of the axis-aligned walls; + = clockwise on screen),
/// from the length-weighted median deviation of long near-axis segments.
pub fn dominant_skew(segs: &[Segment]) -> Option<f64> {
    let mut v: Vec<(f64, f64)> = segs
        .iter()
        .filter(|s| s.len() >= 8.0 * s.thickness)
        .filter_map(|s| {
            let d = [s.b[0] - s.a[0], s.b[1] - s.a[1]];
            let ang = d[1].atan2(d[0]).to_degrees();
            // deviation from the nearest axis, in (-45, 45]
            let dev = ang - (ang / 90.0).round() * 90.0;
            (dev.abs() <= 5.0).then_some((dev, s.len()))
        })
        .collect();
    let total: f64 = v.iter().map(|x| x.1).sum();
    if v.len() < 4 || total <= 0.0 {
        return None;
    }
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut acc = 0.0;
    for (dev, l) in &v {
        acc += l;
        if acc >= total / 2.0 {
            return Some(*dev);
        }
    }
    None
}

/// Stroke width histogram: per ink pixel min(horizontal run, vertical run).
pub fn stroke_stats(ink: &Mask) -> StrokeStats {
    const CAP: usize = 160;
    let (w, h) = (ink.w, ink.h);
    let mut hrun = vec![0u16; w * h];
    for y in 0..h {
        let mut x = 0;
        while x < w {
            if !ink.at(x, y) {
                x += 1;
                continue;
            }
            let s = x;
            while x < w && ink.at(x, y) {
                x += 1;
            }
            let l = (x - s).min(u16::MAX as usize) as u16;
            for i in s..x {
                hrun[y * w + i] = l;
            }
        }
    }
    let mut hist = vec![0f64; CAP + 1];
    for x in 0..w {
        let mut y = 0;
        while y < h {
            if !ink.at(x, y) {
                y += 1;
                continue;
            }
            let s = y;
            while y < h && ink.at(x, y) {
                y += 1;
            }
            let l = y - s;
            for i in s..y {
                let v = (hrun[i * w + x] as usize).min(l).min(CAP);
                hist[v] += 1.0;
            }
        }
    }
    stats_from_hist(&hist)
}

fn stats_from_hist(hist: &[f64]) -> StrokeStats {
    let cap = hist.len() - 1;
    // Thin peak (text, lines) among 1..=4.
    let thin_peak = (1..=4.min(cap)).max_by(|&a, &b| hist[a].partial_cmp(&hist[b]).unwrap()).unwrap_or(1);
    // Wall mode: pixel-weighted histogram favours thick strokes; smooth a little.
    let sm = |v: usize| {
        let lo = v.saturating_sub(1).max(1);
        let hi = (v + 1).min(cap - 1);
        (lo..=hi).map(|i| hist[i]).sum::<f64>() / (hi - lo + 1) as f64
    };
    let wall_mode = (5..cap).max_by(|&a, &b| sm(a).partial_cmp(&sm(b)).unwrap()).unwrap_or(8);
    // Valley between thin peak and wall mode.
    let lo = thin_peak + 1;
    let valley_min = (lo..wall_mode).map(sm).fold(f64::INFINITY, f64::min);
    let valley = if valley_min.is_finite() {
        let floor = (valley_min * 1.5 + 1.0).max(0.12 * sm(thin_peak));
        (lo..wall_mode).find(|&v| sm(v) <= floor).unwrap_or(lo)
    } else {
        lo
    };
    let cap_t = ((wall_mode as f64 * 0.35).round() as usize).max(3);
    let tmin = valley.clamp(3, cap_t.max(3));
    let tmax = ((wall_mode as f64 * 3.5) as usize).max(12);
    StrokeStats { tmin, tmax, wall_mode }
}

struct Band {
    y0: usize,
    y1: usize,
    x0: usize,
    last: usize,
    cover: usize,
    miss: usize,
    xs: Vec<f64>,
    mids: Vec<f64>,
    thick: Vec<usize>,
    /// Slowly adapting centre / thickness (EMA) so the band cannot drift along a diagonal.
    ref_mid: f64,
    ref_t: f64,
}

/// Horizontal walls from column runs. Returns segments in (x, y) of the given mask.
fn bands(m: &Mask, st: &StrokeStats) -> Vec<Segment> {
    let (tmin, tmax) = (st.tmin, st.tmax);
    let mut active: Vec<Band> = Vec::new();
    let mut done: Vec<Band> = Vec::new();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for x in 0..m.w {
        runs.clear();
        let mut y = 0;
        while y < m.h {
            if !m.at(x, y) {
                y += 1;
                continue;
            }
            let s = y;
            while y < m.h && m.at(x, y) {
                y += 1;
            }
            runs.push((s, y));
        }
        let mut used = vec![false; runs.len()];
        let mut still = Vec::with_capacity(active.len());
        for mut b in active.drain(..) {
            let tol_mid = (0.2 * b.ref_t).max(1.5);
            let tol_t = (0.3 * b.ref_t).max(2.0);
            let proper = runs.iter().enumerate().find(|(i, &(r0, r1))| {
                !used[*i]
                    && (tmin..=tmax).contains(&(r1 - r0))
                    && (((r0 + r1) as f64 / 2.0) - b.ref_mid).abs() <= tol_mid
                    && ((r1 - r0) as f64 - b.ref_t).abs() <= tol_t
            });
            if let Some((i, &(r0, r1))) = proper {
                used[i] = true;
                b.ref_mid += 0.03 * ((r0 + r1) as f64 / 2.0 - b.ref_mid);
                b.ref_t += 0.05 * ((r1 - r0) as f64 - b.ref_t);
                b.y0 = r0;
                b.y1 = r1;
                b.last = x;
                b.cover = 0;
                b.miss = 0;
                b.xs.push(x as f64 + 0.5);
                b.mids.push((r0 + r1) as f64 / 2.0);
                b.thick.push(r1 - r0);
                still.push(b);
                continue;
            }
            let covered = runs.iter().any(|&(r0, r1)| r0 <= b.y0 + 1 && r1 + 1 >= b.y1);
            if covered && b.cover < tmax {
                b.cover += 1;
                b.miss = 0;
                b.last = x;
                still.push(b);
                continue;
            }
            b.miss += 1;
            if b.miss <= 1 && !covered {
                still.push(b);
            } else {
                done.push(b);
            }
        }
        active = still;
        for (i, &(r0, r1)) in runs.iter().enumerate() {
            if used[i] || !(tmin..=tmax).contains(&(r1 - r0)) {
                continue;
            }
            // extend backwards through a crossing wall (fully covered columns)
            let mut x0 = x;
            let mut k = 0;
            while x0 > 0 && k < tmax && (r0..r1).all(|yy| m.at(x0 - 1, yy)) {
                x0 -= 1;
                k += 1;
            }
            active.push(Band {
                y0: r0,
                y1: r1,
                x0,
                last: x,
                cover: 0,
                miss: 0,
                xs: vec![x as f64 + 0.5],
                mids: vec![(r0 + r1) as f64 / 2.0],
                thick: vec![r1 - r0],
                ref_mid: (r0 + r1) as f64 / 2.0,
                ref_t: (r1 - r0) as f64,
            });
        }
    }
    done.extend(active);
    let mut out = Vec::new();
    for b in done {
        let n = b.xs.len();
        if n < 3 {
            continue;
        }
        let mut th = b.thick.clone();
        th.sort_unstable();
        let t = th[n / 2] as f64;
        let x0 = b.x0 as f64;
        let x1 = b.last as f64 + 1.0;
        let len = x1 - x0;
        if (n as f64) < (0.5 * t).max(3.0) || (n as f64) < 0.3 * len {
            continue;
        }
        // robust line fit mid = c + k·x: inliers around the median centre (junction ends deviate)
        let mut sorted = b.mids.clone();
        sorted.sort_by(|p, q| p.partial_cmp(q).unwrap());
        let med = sorted[n / 2];
        let tol = (0.15 * t).max(1.5);
        let pts: Vec<(f64, f64)> = b.xs.iter().zip(&b.mids).filter(|(_, m)| (*m - med).abs() <= tol).map(|(x, m)| (*x, *m)).collect();
        let ni = pts.len();
        if ni < 3 {
            continue;
        }
        let mx = pts.iter().map(|p| p.0).sum::<f64>() / ni as f64;
        let my = pts.iter().map(|p| p.1).sum::<f64>() / ni as f64;
        let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
        let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let k = if sxx > 1e-9 && ni >= 10 { sxy / sxx } else { 0.0 };
        if (ni as f64) < 0.6 * n as f64 || k.abs() > 0.07 {
            continue; // follows a diagonal stroke; handled by the diagonal pass
        }
        let yat = |x: f64| my + k * (x - mx);
        out.push(Segment { a: [x0, yat(x0)], b: [x1, yat(x1)], thickness: t, score: (n as f64 / len).min(1.0) });
    }
    out
}

/// Diagonal walls: thick-stroke components not explained by axis-aligned segments.
fn diagonal_segments(ink: &Mask, st: &StrokeStats, axis: &[Segment], min_len: f64) -> Vec<Segment> {
    let r = (st.tmin / 2).clamp(1, 255) as u8;
    let thick = imageproc::morphology::open(&ink.to_gray(), Norm::LInf, r);
    let mut res = Mask::from_gray(&thick, 128);
    // invert: from_gray marks < 128 as true, i.e. background; flip.
    for v in res.d.iter_mut() {
        *v = !*v;
    }
    // erase axis-aligned wall rectangles (+margin)
    for s in axis {
        let m = s.thickness / 2.0 + 3.0;
        let (x0, x1) = (s.a[0].min(s.b[0]) - m, s.a[0].max(s.b[0]) + m);
        let (y0, y1) = (s.a[1].min(s.b[1]) - m, s.a[1].max(s.b[1]) + m);
        let (x0, y0) = (x0.max(0.0) as usize, y0.max(0.0) as usize);
        let (x1, y1) = ((x1.max(0.0) as usize).min(res.w), (y1.max(0.0) as usize).min(res.h));
        for y in y0..y1 {
            for x in x0..x1 {
                res.set(x, y, false);
            }
        }
    }
    let labels = connected_components(&res.to_gray(), Connectivity::Eight, image::Luma([0u8]));
    let mut acc: std::collections::HashMap<u32, (f64, f64, f64, f64, f64, f64)> = Default::default();
    for (x, y, p) in labels.enumerate_pixels() {
        let l = p[0];
        if l == 0 {
            continue;
        }
        let e = acc.entry(l).or_insert((0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
        e.0 += 1.0;
        e.1 += fx;
        e.2 += fy;
        e.3 += fx * fx;
        e.4 += fy * fy;
        e.5 += fx * fy;
    }
    let mut out = Vec::new();
    for (_, (n, sx, sy, sxx, syy, sxy)) in acc {
        if n < 40.0 {
            continue;
        }
        let (mx, my) = (sx / n, sy / n);
        let cxx = sxx / n - mx * mx;
        let cyy = syy / n - my * my;
        let cxy = sxy / n - mx * my;
        let tr = cxx + cyy;
        let det = cxx * cyy - cxy * cxy;
        let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
        let l1 = tr / 2.0 + disc;
        let l2 = (tr / 2.0 - disc).max(0.0);
        // uniform bar: variance along = L²/12, across = T²/12
        let len = (12.0 * l1).sqrt();
        let th = (12.0 * l2).sqrt();
        if th < st.tmin as f64 * 0.8 || th > st.tmax as f64 || len < (1.6 * th).max(min_len) {
            continue;
        }
        let d = if (l1 - cxx).abs() < 1e-9 && cxy.abs() < 1e-9 { [1.0, 0.0] } else { super::norm([l1 - cyy, cxy]) };
        let deg = d[1].atan2(d[0]).to_degrees().rem_euclid(90.0);
        if !(8.0..=82.0).contains(&deg) {
            continue; // axis-aligned leftovers
        }
        // fill ratio check: area ≈ len·th
        if n < 0.6 * len * th {
            continue;
        }
        // bar check: constant cross-section along the axis (rejects L-shaped corner leftovers)
        let nrm = [-d[1], d[0]];
        let consistent = [-0.3, -0.15, 0.0, 0.15, 0.3].iter().all(|f| {
            let c = [mx + d[0] * f * len, my + d[1] * f * len];
            let steps = (1.5 * th).ceil() as i64;
            let w = (-steps..=steps).filter(|&k| res.get((c[0] + nrm[0] * k as f64).floor() as i64, (c[1] + nrm[1] * k as f64).floor() as i64)).count() as f64;
            (w - th).abs() <= 0.35 * th + 2.0
        });
        if !consistent {
            continue;
        }
        let h = len / 2.0;
        out.push(Segment { a: [mx - d[0] * h, my - d[1] * h], b: [mx + d[0] * h, my + d[1] * h], thickness: th, score: 0.6 });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(m: &mut Mask, x0: usize, y0: usize, x1: usize, y1: usize) {
        for y in y0..y1 {
            for x in x0..x1 {
                m.set(x, y, true);
            }
        }
    }

    #[test]
    fn box_with_thin_lines() {
        let mut m = Mask::new(600, 400);
        // outer walls 12 px
        rect(&mut m, 50, 50, 550, 62);
        rect(&mut m, 50, 338, 550, 350);
        rect(&mut m, 50, 50, 62, 350);
        rect(&mut m, 538, 50, 550, 350);
        // interior wall 6 px at x=300
        rect(&mut m, 297, 62, 303, 338);
        // thin lines (text-like / dims)
        rect(&mut m, 80, 200, 250, 201);
        rect(&mut m, 100, 100, 102, 110);
        rect(&mut m, 30, 20, 570, 21);
        let r = detect(&m);
        assert!(r.stats.tmin >= 3 && r.stats.tmin <= 6, "{:?}", r.stats);
        let h: Vec<_> = r.segments.iter().filter(|s| (s.a[1] - s.b[1]).abs() < 1.0 && s.len() > 200.0).collect();
        let v: Vec<_> = r.segments.iter().filter(|s| (s.a[0] - s.b[0]).abs() < 1.0 && s.len() > 200.0).collect();
        assert_eq!(h.len(), 2, "{:#?}", r.segments);
        assert_eq!(v.len(), 3, "{:#?}", r.segments);
        let inner = v.iter().find(|s| (s.a[0] - 300.0).abs() < 1.0).unwrap();
        assert!((inner.thickness - 6.0).abs() < 0.5);
        let top = h.iter().find(|s| s.a[1] < 100.0).unwrap();
        assert!((top.a[1] - 56.0).abs() < 0.5, "{top:?}");
        assert!(top.a[0].min(top.b[0]) <= 51.0 && top.a[0].max(top.b[0]) >= 549.0, "{top:?}");
        assert!(r.segments.len() <= 6, "{:#?}", r.segments);
    }

    #[test]
    fn diagonal_wall() {
        let mut m = Mask::new(400, 400);
        for i in 0..250 {
            for j in 0..10 {
                let x = 80 + i;
                let y = 80 + i + j;
                m.set(x, y.min(399), true);
            }
        }
        // keep a horizontal wall for stats
        rect(&mut m, 20, 360, 380, 370);
        let r = detect(&m);
        let d: Vec<_> = r.segments.iter().filter(|s| (s.a[0] - s.b[0]).abs() > 50.0 && (s.a[1] - s.b[1]).abs() > 50.0).collect();
        assert_eq!(d.len(), 1, "{:#?}", r.segments);
        assert!(d[0].len() > 300.0);
    }
}
