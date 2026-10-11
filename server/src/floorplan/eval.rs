//! Quality metrics: align a pipeline result to a hand-made truth floor and count corrections.
//!
//! Metric (Spec «Qualität messen»): share of truth walls whose two endpoints are matched within
//! ±5 cm, and the number of corrections = missing + extra + misplaced walls and openings.

use plan_core::geom::*;
use plan_core::rooms::derive_rooms;
use plan_core::{Floor, Node, OpeningKind};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    pub walls_truth: usize,
    pub walls_ok: usize,
    pub walls_misplaced: usize,
    pub walls_missing: usize,
    pub walls_extra: usize,
    pub open_truth: usize,
    pub open_ok: usize,
    pub open_misplaced: usize,
    pub open_missing: usize,
    pub open_extra: usize,
    pub rooms_truth: usize,
    pub rooms_result: usize,
}

impl Metrics {
    pub fn corrections(&self) -> usize {
        self.walls_misplaced + self.walls_missing + self.walls_extra + self.open_misplaced + self.open_missing + self.open_extra
    }
    pub fn wall_share(&self) -> f64 {
        if self.walls_truth == 0 {
            1.0
        } else {
            self.walls_ok as f64 / self.walls_truth as f64
        }
    }
    pub fn add(&mut self, o: &Metrics) {
        self.walls_truth += o.walls_truth;
        self.walls_ok += o.walls_ok;
        self.walls_misplaced += o.walls_misplaced;
        self.walls_missing += o.walls_missing;
        self.walls_extra += o.walls_extra;
        self.open_truth += o.open_truth;
        self.open_ok += o.open_ok;
        self.open_misplaced += o.open_misplaced;
        self.open_missing += o.open_missing;
        self.open_extra += o.open_extra;
        self.rooms_truth += o.rooms_truth;
        self.rooms_result += o.rooms_result;
    }
}

/// p' = p·s + t for nodes, rooms; lengths along walls scale too.
pub fn transform(f: &Floor, s: f64, t: P) -> Floor {
    let mut g = f.clone();
    for n in g.nodes.iter_mut() {
        let q = add(mul([n.x_mm as f64, n.y_mm as f64], s), t);
        *n = Node { id: n.id, x_mm: q[0].round() as i32, y_mm: q[1].round() as i32 };
    }
    for w in g.walls.iter_mut() {
        w.thickness_mm = ((w.thickness_mm as f64 * s).round() as u32).max(1);
    }
    for o in g.openings.iter_mut() {
        o.offset_mm = (o.offset_mm as f64 * s).round() as u32;
        o.width_mm = ((o.width_mm as f64 * s).round() as u32).max(1);
    }
    for r in g.rooms.iter_mut() {
        let q = add(mul([r.anchor[0] as f64, r.anchor[1] as f64], s), t);
        r.anchor = [q[0].round() as i32, q[1].round() as i32];
    }
    g
}

fn bbox(f: &Floor) -> Option<(P, P)> {
    if f.nodes.is_empty() {
        return None;
    }
    let mut lo = [f64::MAX, f64::MAX];
    let mut hi = [f64::MIN, f64::MIN];
    for n in &f.nodes {
        lo = [lo[0].min(n.x_mm as f64), lo[1].min(n.y_mm as f64)];
        hi = [hi[0].max(n.x_mm as f64), hi[1].max(n.y_mm as f64)];
    }
    Some((lo, hi))
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(v[v.len() / 2])
}

/// Align `result` (mm = rectified px × `result_mpp`) to `truth`.
/// Scale: from the truth's reference image (true mm/px of the source image) and the
/// source→rectified transform (`src_offset`, `src_scale`, from preprocessing); otherwise from
/// bounding boxes. Translation refined by nearest-node matching.
/// Returns the aligned result and the relative scale error of the pipeline (if truth scale known).
pub fn align(truth: &Floor, result: &Floor, result_mpp: f64, src_offset: P, src_scale: f64) -> (Floor, Option<f64>) {
    let (s, mut t, err) = match &truth.reference_image {
        Some(ri) if ri.mm_per_px > 0.0 && src_scale.is_finite() && src_scale > 0.0 => {
            let true_mpp = ri.mm_per_px / src_scale; // per rectified px
            let s = true_mpp / result_mpp;
            let t = add(mul(src_offset, ri.mm_per_px), ri.origin_mm);
            (s, t, Some(result_mpp / true_mpp - 1.0))
        }
        _ => match (bbox(truth), bbox(result)) {
            (Some((tl, th)), Some((rl, rh))) => {
                let sx = (th[0] - tl[0]) / (rh[0] - rl[0]).max(1.0);
                let sy = (th[1] - tl[1]) / (rh[1] - rl[1]).max(1.0);
                let s = (sx + sy) / 2.0;
                (s, sub(tl, mul(rl, s)), None)
            }
            _ => (1.0, [0.0, 0.0], None),
        },
    };
    for radius in [600.0, 300.0, 150.0, 80.0] {
        let g = transform(result, s, t);
        let mut dx = Vec::new();
        let mut dy = Vec::new();
        for n in &g.nodes {
            let p = [n.x_mm as f64, n.y_mm as f64];
            if let Some(q) = truth
                .nodes
                .iter()
                .map(|m| [m.x_mm as f64, m.y_mm as f64])
                .min_by(|a, b| dist(*a, p).partial_cmp(&dist(*b, p)).unwrap())
                .filter(|q| dist(*q, p) <= radius)
            {
                dx.push(q[0] - p[0]);
                dy.push(q[1] - p[1]);
            }
        }
        if let (Some(mx), Some(my)) = (median(dx), median(dy)) {
            t = add(t, [mx, my]);
        }
    }
    (transform(result, s, t), err)
}

fn walls_of(f: &Floor) -> Vec<(P, P)> {
    f.walls.iter().filter_map(|w| f.wall_points(w)).collect()
}

fn overlaps(t: (P, P), r: (P, P), max_off: f64, min_frac: f64) -> bool {
    let dt = norm(sub(t.1, t.0));
    let dr = norm(sub(r.1, r.0));
    if cross(dt, dr).abs() > (5f64).to_radians().sin() {
        return false;
    }
    let lt = dist(t.0, t.1);
    let lr = dist(r.0, r.1);
    if cross(dt, sub(lerp(r.0, r.1, 0.5), t.0)).abs() > max_off {
        return false;
    }
    let a = dot(sub(r.0, t.0), dt);
    let b = dot(sub(r.1, t.0), dt);
    let ov = a.max(b).min(lt) - a.min(b).max(0.0);
    ov >= min_frac * lt.min(lr)
}

fn opening_centres(f: &Floor) -> Vec<(P, bool, f64)> {
    f.openings
        .iter()
        .filter_map(|o| {
            let w = f.wall(o.wall)?;
            let (a, b) = f.wall_points(w)?;
            let d = norm(sub(b, a));
            let c = add(a, mul(d, o.offset_mm as f64 + o.width_mm as f64 / 2.0));
            Some((c, matches!(o.kind, OpeningKind::Window { .. }), o.width_mm as f64))
        })
        .collect()
}

/// Compare an aligned result with the truth.
pub fn compare(truth: &Floor, result: &Floor, tol: f64) -> Metrics {
    let tw = walls_of(truth);
    let rw = walls_of(result);
    let mut m = Metrics { walls_truth: tw.len(), ..Default::default() };
    let mut r_used = vec![false; rw.len()];
    for t in &tw {
        let ok = rw.iter().position(|r| {
            (dist(t.0, r.0) <= tol && dist(t.1, r.1) <= tol) || (dist(t.0, r.1) <= tol && dist(t.1, r.0) <= tol)
        });
        if let Some(i) = ok {
            m.walls_ok += 1;
            r_used[i] = true;
            continue;
        }
        let near: Vec<usize> = (0..rw.len()).filter(|&i| overlaps(*t, rw[i], 300.0, 0.5)).collect();
        if near.is_empty() {
            m.walls_missing += 1;
        } else {
            m.walls_misplaced += 1;
            for i in near {
                r_used[i] = true;
            }
        }
    }
    for (i, r) in rw.iter().enumerate() {
        if !r_used[i] && !tw.iter().any(|t| overlaps(*t, *r, 300.0, 0.3)) {
            m.walls_extra += 1;
        }
    }
    let to = opening_centres(truth);
    let ro = opening_centres(result);
    m.open_truth = to.len();
    let mut used = vec![false; ro.len()];
    for t in &to {
        let ok = (0..ro.len()).find(|&i| !used[i] && ro[i].1 == t.1 && dist(ro[i].0, t.0) <= 2.0 * tol && (ro[i].2 - t.2).abs() <= 2.0 * tol);
        if let Some(i) = ok {
            used[i] = true;
            m.open_ok += 1;
            continue;
        }
        let near = (0..ro.len()).find(|&i| !used[i] && dist(ro[i].0, t.0) <= 600.0);
        if let Some(i) = near {
            used[i] = true;
            m.open_misplaced += 1;
        } else {
            m.open_missing += 1;
        }
    }
    m.open_extra = used.iter().filter(|u| !**u).count();
    m.rooms_truth = derive_rooms(truth).len();
    m.rooms_result = derive_rooms(result).len();
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn identical_floor_is_perfect_and_shift_is_found() {
        let c = &super::super::synth::cases()[0];
        let shifted = transform(&c.truth, 1.0, [123.0, -77.0]);
        let mut truth = c.truth.clone();
        truth.reference_image = None;
        let (al, err) = align(&truth, &shifted, 1.0, [0.0, 0.0], 1.0);
        assert!(err.is_none());
        let m = compare(&truth, &al, 50.0);
        assert_eq!(m.walls_ok, m.walls_truth);
        assert_eq!(m.open_ok, m.open_truth);
        assert_eq!(m.corrections(), 0);
        let mut broken = al.clone();
        broken.walls.pop();
        broken.nodes.push(Node { id: Uuid::new_v4(), x_mm: 0, y_mm: 0 });
        let m2 = compare(&truth, &broken, 50.0);
        assert_eq!(m2.walls_ok + 1, m2.walls_truth);
        assert!(m2.corrections() >= 1);
    }
}
