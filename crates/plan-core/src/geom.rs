//! 2D geometry helpers (f64, millimetres).

pub type P = [f64; 2];

pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
pub fn mul(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}
pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
pub fn len(a: P) -> f64 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}
pub fn dist(a: P, b: P) -> f64 {
    len(sub(a, b))
}
pub fn norm(a: P) -> P {
    let l = len(a);
    if l < 1e-12 {
        [0.0, 0.0]
    } else {
        [a[0] / l, a[1] / l]
    }
}
/// Rotate +90° in the algebraic sense: (x, y) → (−y, x).
pub fn perp(a: P) -> P {
    [-a[1], a[0]]
}
/// Left normal of a direction when looking at the plan from above (y down).
pub fn left_normal(d: P) -> P {
    [d[1], -d[0]]
}
pub fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
pub fn rotate(p: P, deg: f64) -> P {
    let (s, c) = deg.to_radians().sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}

/// Intersection of lines p + s·d and q + t·e. None if (nearly) parallel.
pub fn line_intersect(p: P, d: P, q: P, e: P) -> Option<P> {
    let den = cross(d, e);
    if den.abs() < 1e-9 {
        return None;
    }
    let s = cross(sub(q, p), e) / den;
    Some(add(p, mul(d, s)))
}

/// Proper segment intersection: returns (t on ab, u on cd, point).
pub fn segment_intersect(a: P, b: P, c: P, d: P) -> Option<(f64, f64, P)> {
    let r = sub(b, a);
    let s = sub(d, c);
    let den = cross(r, s);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = cross(sub(c, a), s) / den;
    let u = cross(sub(c, a), r) / den;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some((t, u, add(a, mul(r, t))))
    } else {
        None
    }
}

/// Closest point on segment ab to p: (t, point, distance).
pub fn closest_on_segment(p: P, a: P, b: P) -> (f64, P, f64) {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    let t = if l2 < 1e-12 {
        0.0
    } else {
        (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0)
    };
    let q = add(a, mul(ab, t));
    (t, q, dist(p, q))
}

/// Signed area (algebraic orientation, shoelace).
pub fn signed_area(poly: &[P]) -> f64 {
    let n = poly.len();
    let mut s = 0.0;
    for i in 0..n {
        s += cross(poly[i], poly[(i + 1) % n]);
    }
    s / 2.0
}

pub fn centroid(poly: &[P]) -> P {
    let a = signed_area(poly);
    if a.abs() < 1e-9 {
        let n = poly.len().max(1) as f64;
        let s = poly.iter().fold([0.0, 0.0], |acc, p| add(acc, *p));
        return mul(s, 1.0 / n);
    }
    let n = poly.len();
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..n {
        let p = poly[i];
        let q = poly[(i + 1) % n];
        let c = cross(p, q);
        cx += (p[0] + q[0]) * c;
        cy += (p[1] + q[1]) * c;
    }
    [cx / (6.0 * a), cy / (6.0 * a)]
}

pub fn point_in_polygon(p: P, poly: &[P]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0];
            if p[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Distance from p to polygon boundary.
pub fn dist_to_polygon(p: P, poly: &[P]) -> f64 {
    let n = poly.len();
    (0..n)
        .map(|i| closest_on_segment(p, poly[i], poly[(i + 1) % n]).2)
        .fold(f64::INFINITY, f64::min)
}

/// A point that is guaranteed (best effort) to lie inside the polygon, preferably far from edges.
pub fn interior_point(poly: &[P]) -> P {
    let c = centroid(poly);
    if point_in_polygon(c, poly) && dist_to_polygon(c, poly) > 200.0 {
        return c;
    }
    // Coarse grid search for the point with the largest edge distance.
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in poly {
        minx = minx.min(p[0]);
        miny = miny.min(p[1]);
        maxx = maxx.max(p[0]);
        maxy = maxy.max(p[1]);
    }
    let mut best = c;
    let mut best_d = if point_in_polygon(c, poly) { dist_to_polygon(c, poly) } else { -1.0 };
    let steps = 24;
    for i in 0..=steps {
        for j in 0..=steps {
            let p = [
                minx + (maxx - minx) * i as f64 / steps as f64,
                miny + (maxy - miny) * j as f64 / steps as f64,
            ];
            if point_in_polygon(p, poly) {
                let d = dist_to_polygon(p, poly);
                if d > best_d {
                    best_d = d;
                    best = p;
                }
            }
        }
    }
    best
}

/// Clip polygon to the half-plane dot(p, n) >= c (Sutherland–Hodgman).
pub fn clip_half_plane(poly: &[P], n: P, c: f64) -> Vec<P> {
    let mut out = Vec::new();
    let k = poly.len();
    for i in 0..k {
        let cur = poly[i];
        let prev = poly[(i + k - 1) % k];
        let dc = dot(cur, n) - c;
        let dp = dot(prev, n) - c;
        if dc >= 0.0 {
            if dp < 0.0 {
                out.push(lerp(prev, cur, dp / (dp - dc)));
            }
            out.push(cur);
        } else if dp >= 0.0 {
            out.push(lerp(prev, cur, dp / (dp - dc)));
        }
    }
    out
}

/// Ear-clipping triangulation. Returns index triples into `poly`.
/// Output triangles have the same orientation as the input polygon.
pub fn triangulate(poly: &[P]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return vec![];
    }
    let ccw = signed_area(poly) > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut tris = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (poly[ia], poly[ib], poly[ic]);
            let cr = cross(sub(b, a), sub(c, b));
            let convex = if ccw { cr > 1e-9 } else { cr < -1e-9 };
            if !convex {
                continue;
            }
            let tri = [a, b, c];
            let contains = idx.iter().any(|&j| {
                j != ia && j != ib && j != ic && point_in_triangle(poly[j], tri)
            });
            if contains {
                continue;
            }
            tris.push([ia, ib, ic]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // Degenerate polygon: drop a collinear vertex.
            let m = idx.len();
            let mut removed = false;
            for i in 0..m {
                let (a, b, c) = (poly[idx[(i + m - 1) % m]], poly[idx[i]], poly[idx[(i + 1) % m]]);
                if cross(sub(b, a), sub(c, b)).abs() <= 1e-6 {
                    idx.remove(i);
                    removed = true;
                    break;
                }
            }
            if !removed {
                break;
            }
        }
    }
    if idx.len() == 3 {
        tris.push([idx[0], idx[1], idx[2]]);
    }
    tris
}

fn point_in_triangle(p: P, t: [P; 3]) -> bool {
    let d1 = cross(sub(t[1], t[0]), sub(p, t[0]));
    let d2 = cross(sub(t[2], t[1]), sub(p, t[1]));
    let d3 = cross(sub(t[0], t[2]), sub(p, t[2]));
    let neg = d1 < -1e-9 || d2 < -1e-9 || d3 < -1e-9;
    let pos = d1 > 1e-9 || d2 > 1e-9 || d3 > 1e-9;
    !(neg && pos)
}

/// Oriented rectangle corners (plan) for centre, size and rotation.
pub fn obb(center: P, w: f64, d: f64, rot_deg: f64) -> [P; 4] {
    let hw = w / 2.0;
    let hd = d / 2.0;
    [[-hw, -hd], [hw, -hd], [hw, hd], [-hw, hd]].map(|p| add(center, rotate(p, rot_deg)))
}

/// SAT overlap test for two convex polygons (strict, with small tolerance).
pub fn convex_overlap(a: &[P], b: &[P], tol: f64) -> bool {
    for poly in [a, b] {
        let n = poly.len();
        for i in 0..n {
            let e = sub(poly[(i + 1) % n], poly[i]);
            let ax = norm(perp(e));
            let (mut amin, mut amax) = (f64::MAX, f64::MIN);
            for p in a {
                let v = dot(*p, ax);
                amin = amin.min(v);
                amax = amax.max(v);
            }
            let (mut bmin, mut bmax) = (f64::MAX, f64::MIN);
            for p in b {
                let v = dot(*p, ax);
                bmin = bmin.min(v);
                bmax = bmax.max(v);
            }
            if amax <= bmin + tol || bmax <= amin + tol {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulate_l_shape() {
        let l = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let t = triangulate(&l);
        assert_eq!(t.len(), 4);
        let area: f64 = t
            .iter()
            .map(|t| signed_area(&[l[t[0]], l[t[1]], l[t[2]]]).abs())
            .sum();
        assert!((area - 3.0).abs() < 1e-9);
    }
}
