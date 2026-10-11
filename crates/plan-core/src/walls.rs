//! Wall footprints with mitred joints, and wall pieces split at openings.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::geom::*;
use crate::model::*;

/// Wall footprint in plan coordinates.
/// `corners` is the (convex) polygon: left_a, left_b, [node b], right_b, right_a, [node a].
/// At junctions of 3+ walls the node centre is included so the pieces tile the junction without holes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WallFootprint {
    pub wall: Id,
    pub corners: Vec<[f64; 2]>,
    /// Left face (a → b) and right face (a → b).
    pub left: [[f64; 2]; 2],
    pub right: [[f64; 2]; 2],
    pub length_mm: f64,
}

/// For each (node, wall) pair: corner on the +perp side and on the −perp side
/// (perp = algebraic +90° of the outward direction from the node).
fn node_corners(f: &Floor) -> HashMap<(Id, Id), (P, P)> {
    let mut out = HashMap::new();
    let pos: HashMap<Id, P> = f
        .nodes
        .iter()
        .map(|n| (n.id, [n.x_mm as f64, n.y_mm as f64]))
        .collect();
    let mut incident: HashMap<Id, Vec<(f64, &Wall, P)>> = HashMap::new();
    for w in &f.walls {
        let (Some(&pa), Some(&pb)) = (pos.get(&w.a), pos.get(&w.b)) else { continue };
        if dist(pa, pb) < 1e-6 {
            continue;
        }
        let d = norm(sub(pb, pa));
        incident.entry(w.a).or_default().push((d[1].atan2(d[0]), w, d));
        let e = mul(d, -1.0);
        incident.entry(w.b).or_default().push((e[1].atan2(e[0]), w, e));
    }
    for (node, mut list) in incident {
        let n = pos[&node];
        list.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let k = list.len();
        if k == 1 {
            let (_, w, d) = list[0];
            let h = w.thickness_mm as f64 / 2.0;
            out.insert((node, w.id), (add(n, mul(perp(d), h)), add(n, mul(perp(d), -h))));
            continue;
        }
        // corner between wall i (+perp side) and wall i+1 (−perp side)
        let mut plus = vec![[0.0; 2]; k];
        let mut minus = vec![[0.0; 2]; k];
        for i in 0..k {
            let j = (i + 1) % k;
            let (_, wi, di) = list[i];
            let (_, wj, dj) = list[j];
            let hi = wi.thickness_mm as f64 / 2.0;
            let hj = wj.thickness_mm as f64 / 2.0;
            let pi = add(n, mul(perp(di), hi));
            let pj = add(n, mul(perp(dj), -hj));
            match line_intersect(pi, di, pj, dj) {
                // Mitre limit: very sharp angles fall back to square ends.
                Some(c) if dist(c, n) <= 3.0 * hi.max(hj) + 1.0 => {
                    plus[i] = c;
                    minus[j] = c;
                }
                _ => {
                    plus[i] = pi;
                    minus[j] = pj;
                }
            }
        }
        for i in 0..k {
            out.insert((node, list[i].1.id), (plus[i], minus[i]));
        }
    }
    out
}

pub fn footprints(f: &Floor) -> Vec<WallFootprint> {
    let corners = node_corners(f);
    let mut degree: HashMap<Id, usize> = HashMap::new();
    for w in &f.walls {
        *degree.entry(w.a).or_default() += 1;
        *degree.entry(w.b).or_default() += 1;
    }
    let mut out = Vec::new();
    for w in &f.walls {
        let Some((pa, pb)) = f.wall_points(w) else { continue };
        let (Some(&(a_plus, a_minus)), Some(&(b_plus, b_minus))) =
            (corners.get(&(w.a, w.id)), corners.get(&(w.b, w.id)))
        else {
            continue;
        };
        // At a, outward dir d: perp(d) = right side. At b, outward −d: perp(−d) = left side.
        let left_a = a_minus;
        let right_a = a_plus;
        let left_b = b_plus;
        let right_b = b_minus;
        let mut poly = vec![left_a, left_b];
        if degree.get(&w.b).copied().unwrap_or(0) >= 3 {
            poly.push(pb);
        }
        poly.push(right_b);
        poly.push(right_a);
        if degree.get(&w.a).copied().unwrap_or(0) >= 3 {
            poly.push(pa);
        }
        out.push(WallFootprint {
            wall: w.id,
            corners: poly,
            left: [left_a, left_b],
            right: [right_a, right_b],
            length_mm: dist(pa, pb),
        });
    }
    out
}

/// A solid prism piece of a wall: footprint polygon (plan) and z range (mm).
#[derive(Clone, Debug)]
pub struct WallPiece {
    pub wall: Id,
    pub poly: Vec<P>,
    pub z0: f64,
    pub z1: f64,
    /// Wall direction and left normal, for side classification.
    pub dir: P,
    pub left: P,
}

/// Split wall footprints at openings. If `walk` is true, only pieces that block
/// walking are returned (windows block, doors/passages don't).
pub fn wall_pieces(f: &Floor, fps: &[WallFootprint], walk: bool) -> Vec<WallPiece> {
    wall_pieces_closed(f, fps, walk, &Default::default())
}

/// Like [`wall_pieces`]; for walking, the openings in `closed` (shut doors) block like wall.
pub fn wall_pieces_closed(f: &Floor, fps: &[WallFootprint], walk: bool, closed: &std::collections::HashSet<Id>) -> Vec<WallPiece> {
    let mut out = Vec::new();
    for fp in fps {
        let Some(w) = f.wall(fp.wall) else { continue };
        let h = w.height(f.ceiling_height_mm) as f64;
        let Some((pa, pb)) = f.wall_points(w) else { continue };
        let l = dist(pa, pb);
        if l < 1e-6 {
            continue;
        }
        let d = norm(sub(pb, pa));
        let left = left_normal(d);
        let base = dot(pa, d);
        let mut ops: Vec<&Opening> = f.openings_on(w.id).collect();
        ops.sort_by_key(|o| o.offset_mm);
        let poly = fp.corners.clone();
        let slab = |u0: f64, u1: f64| -> Vec<P> {
            let mut p = poly.clone();
            if u0.is_finite() {
                p = clip_half_plane(&p, d, base + u0);
            }
            if u1.is_finite() {
                p = clip_half_plane(&p, mul(d, -1.0), -(base + u1));
            }
            p
        };
        let mut push = |poly: Vec<P>, z0: f64, z1: f64| {
            if poly.len() >= 3 && z1 - z0 > 0.5 && signed_area(&poly).abs() > 1.0 {
                out.push(WallPiece { wall: w.id, poly, z0, z1, dir: d, left });
            }
        };
        let mut u = f64::NEG_INFINITY;
        for o in ops {
            let s = (o.offset_mm as f64).clamp(0.0, l);
            let e = (o.offset_mm as f64 + o.width_mm as f64).clamp(0.0, l);
            if e - s < 1.0 {
                continue;
            }
            let s = s.max(if u.is_finite() { u } else { s });
            if s > u || !u.is_finite() {
                push(slab(u, s), 0.0, h);
            }
            let sill = o.kind.sill_mm() as f64;
            let top = (sill + o.height_mm as f64).min(h);
            if walk {
                if !o.kind.walkable() || closed.contains(&o.id) {
                    push(slab(s, e), 0.0, h);
                }
            } else {
                if sill > 0.0 {
                    push(slab(s, e), 0.0, sill.min(h));
                }
                if top < h {
                    push(slab(s, e), top, h);
                }
            }
            u = e;
        }
        push(slab(u, f64::INFINITY), 0.0, h);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    pub fn square_floor(size: i32, t: u32) -> Floor {
        let mut f = Floor::new(Uuid::new_v4(), "EG");
        let pts = [(0, 0), (size, 0), (size, size), (0, size)];
        let ids: Vec<Id> = pts
            .iter()
            .map(|&(x, y)| {
                let id = Uuid::new_v4();
                f.nodes.push(Node { id, x_mm: x, y_mm: y });
                id
            })
            .collect();
        for i in 0..4 {
            f.walls.push(Wall::new(Uuid::new_v4(), ids[i], ids[(i + 1) % 4], t, WallKind::Exterior));
        }
        f
    }

    #[test]
    fn mitred_square() {
        let f = square_floor(4000, 200);
        let fps = footprints(&f);
        assert_eq!(fps.len(), 4);
        // Wall 0 goes (0,0)→(4000,0); left side is −y (north).
        let c = &fps[0].corners;
        assert_eq!(c.len(), 4);
        assert!((c[0][0] + 100.0).abs() < 1e-6 && (c[0][1] + 100.0).abs() < 1e-6, "{c:?}");
        assert!((c[1][0] - 4100.0).abs() < 1e-6 && (c[1][1] + 100.0).abs() < 1e-6, "{c:?}");
        assert!((c[2][0] - 3900.0).abs() < 1e-6 && (c[2][1] - 100.0).abs() < 1e-6, "{c:?}");
        assert!((c[3][0] - 100.0).abs() < 1e-6 && (c[3][1] - 100.0).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn pieces_with_window() {
        let mut f = square_floor(4000, 200);
        let w = f.walls[0].id;
        f.openings.push(Opening {
            id: Uuid::new_v4(),
            wall: w,
            kind: OpeningKind::Window { sill_mm: 900 },
            offset_mm: 1000,
            width_mm: 1200,
            height_mm: 1200,
        });
        let fps = footprints(&f);
        let p = wall_pieces(&f, &fps, false);
        // wall0: left, sill, lintel, right = 4; others 1 each
        assert_eq!(p.len(), 7);
        let walk = wall_pieces(&f, &fps, true);
        assert_eq!(walk.len(), 4 + 2);
    }

    #[test]
    fn low_wall_pieces() {
        let mut f = square_floor(4000, 200);
        f.walls[0].height_mm = Some(1000);
        let w = f.walls[0].id;
        f.openings.push(Opening { id: Uuid::new_v4(), wall: w, kind: OpeningKind::Window { sill_mm: 900 }, offset_mm: 1000, width_mm: 1200, height_mm: 1200 });
        let fps = footprints(&f);
        let p: Vec<_> = wall_pieces(&f, &fps, false).into_iter().filter(|p| p.wall == w).collect();
        // left, sill (0..900), right; nothing above 1000
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|p| p.z1 <= 1000.0));
    }

    #[test]
    fn t_junction_has_no_hole() {
        // through wall split at x=2000, branch going down
        let mut f = Floor::new(Uuid::new_v4(), "EG");
        let mut n = |x, y| {
            let id = Uuid::new_v4();
            f.nodes.push(Node { id, x_mm: x, y_mm: y });
            id
        };
        let (a, m, b, c) = (n(0, 0), n(2000, 0), n(4000, 0), n(2000, 3000));
        f.walls.push(Wall::new(Uuid::new_v4(), a, m, 300, WallKind::Exterior));
        f.walls.push(Wall::new(Uuid::new_v4(), m, b, 300, WallKind::Exterior));
        f.walls.push(Wall::new(Uuid::new_v4(), m, c, 100, WallKind::Interior));
        let fps = footprints(&f);
        let total: f64 = fps.iter().map(|fp| signed_area(&fp.corners).abs()).sum();
        // union area: 4000x300 + 100 x (3000-150) (square end at c)
        let expected = 4000.0 * 300.0 + 100.0 * 2850.0;
        assert!((total - expected).abs() < 1.0, "{total} vs {expected}");
        // points just around the node are covered
        for p in [[2000.0, -140.0], [2010.0, 0.0], [1990.0, 0.0], [2000.0, 140.0]] {
            assert!(fps.iter().any(|fp| point_in_polygon(p, &fp.corners)), "{p:?} uncovered");
        }
    }
}
