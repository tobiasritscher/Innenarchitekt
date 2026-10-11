//! Furniture placement helpers: wall snapping, clearances, overlaps, walk collision.

use serde::{Deserialize, Serialize};

use crate::geom::*;
use crate::model::*;
use crate::primitives::podium_height_at;
use crate::rooms::derive_rooms;
use crate::walls::{footprints, wall_pieces, WallFootprint};

/// A wall face (inner/outer side line) with outward normal (pointing away from the wall body).
#[derive(Clone, Debug)]
struct Face {
    wall: Id,
    a: P,
    b: P,
    n: P,
}

fn faces(fps: &[WallFootprint]) -> Vec<Face> {
    let mut out = Vec::new();
    for fp in fps {
        let [la, lb] = fp.left;
        let [ra, rb] = fp.right;
        let d = norm(sub(lb, la));
        let left = left_normal(d);
        out.push(Face { wall: fp.wall, a: la, b: lb, n: left });
        out.push(Face { wall: fp.wall, a: ra, b: rb, n: mul(left, -1.0) });
    }
    out
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SnapResult {
    pub x_mm: i32,
    pub y_mm: i32,
    pub rotation_deg: f32,
    pub wall: Id,
}

/// Snap a furniture footprint with its back against the nearest wall face within `threshold_mm`.
/// Once the plan has closed rooms, only faces looking into a room are used (never the outside of the flat).
pub fn snap_to_wall(f: &Floor, x: f64, y: f64, dims: [u32; 3], rot_deg: f64, threshold_mm: f64) -> Option<SnapResult> {
    let fps = footprints(f);
    let rooms: Vec<Vec<P>> = derive_rooms(f).into_iter().map(|r| r.polygon).collect();
    let c = [x, y];
    let depth = dims[1] as f64;
    let mut best: Option<(f64, SnapResult)> = None;
    for face in faces(&fps) {
        let (t, q, _) = closest_on_segment(c, face.a, face.b);
        if t <= 0.0 || t >= 1.0 {
            continue;
        }
        if !rooms.is_empty() && !rooms.iter().any(|r| point_in_polygon(add(q, mul(face.n, 10.0)), r)) {
            continue; // outer face
        }
        // signed distance of the centre in front of the face
        let s = dot(sub(c, q), face.n);
        if s < -depth / 2.0 {
            continue; // behind the wall
        }
        let gap = (s - depth / 2.0).abs();
        if gap > threshold_mm {
            continue;
        }
        // front (+y local) must point along face normal: (−sinθ, cosθ) = n
        let rot = (-face.n[0]).atan2(face.n[1]).to_degrees();
        // only snap if roughly aligned already or rotation is free and close
        let mut diff = (rot - rot_deg).rem_euclid(360.0);
        if diff > 180.0 {
            diff -= 360.0;
        }
        if diff.abs() > 60.0 && gap > threshold_mm / 2.0 {
            continue;
        }
        let p = add(q, mul(face.n, depth / 2.0));
        let res = SnapResult {
            x_mm: p[0].round() as i32,
            y_mm: p[1].round() as i32,
            rotation_deg: (rot.rem_euclid(360.0)) as f32,
            wall: face.wall,
        };
        if best.as_ref().map_or(true, |(g, _)| gap < *g) {
            best = Some((gap, res));
        }
    }
    best.map(|(_, r)| r)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Clearances {
    /// Distance from each side of the footprint to the nearest wall face along that side's normal (mm).
    pub left: Option<f64>,
    pub right: Option<f64>,
    pub front: Option<f64>,
    pub back: Option<f64>,
}

fn ray_to_faces(o: P, d: P, faces: &[Face]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for f in faces {
        if let Some((t, _u, _)) = segment_intersect(o, add(o, mul(d, 1.0e5)), f.a, f.b) {
            let dist = t * 1.0e5;
            if dist > 0.5 && best.map_or(true, |b| dist < b) {
                best = Some(dist);
            }
        }
    }
    best
}

pub fn clearances(f: &Floor, item: &FurnitureInstance) -> Clearances {
    let fps = footprints(f);
    let fs = faces(&fps);
    let c = [item.pos_mm[0] as f64, item.pos_mm[1] as f64];
    let rot = item.rotation_deg as f64;
    let hw = item.dims_mm[0] as f64 / 2.0;
    let hd = item.dims_mm[1] as f64 / 2.0;
    let dir = |v: P| rotate(v, rot);
    let ray = |v: P, half: f64| ray_to_faces(c, dir(v), &fs).map(|d| (d - half).max(0.0).round());
    Clearances {
        right: ray([1.0, 0.0], hw),
        left: ray([-1.0, 0.0], hw),
        front: ray([0.0, 1.0], hd),
        back: ray([0.0, -1.0], hd),
    }
}

pub fn furniture_footprint(item: &FurnitureInstance) -> [P; 4] {
    obb(
        [item.pos_mm[0] as f64, item.pos_mm[1] as f64],
        item.dims_mm[0] as f64,
        item.dims_mm[1] as f64,
        item.rotation_deg as f64,
    )
}

/// Plan rectangles the item actually occupies: its footprint parts, or the whole box.
pub fn footprint_parts(item: &FurnitureInstance) -> Vec<[P; 4]> {
    if item.footprint.is_empty() {
        return vec![furniture_footprint(item)];
    }
    let (w, d) = (item.dims_mm[0] as f64, item.dims_mm[1] as f64);
    let c = [item.pos_mm[0] as f64, item.pos_mm[1] as f64];
    let rot = item.rotation_deg as f64;
    item.footprint
        .iter()
        .map(|r| {
            let [x0, y0, x1, y1] = r.map(|v| v.clamp(0.0, 1.0) as f64);
            let local = [((x0 + x1) / 2.0 - 0.5) * w, ((y0 + y1) / 2.0 - 0.5) * d];
            obb(add(c, rotate(local, rot)), (x1 - x0) * w, (y1 - y0) * d, rot)
        })
        .collect()
}

/// Surface height (mm) of a walkable item at plan point `p`, None if outside its footprint.
pub fn surface_height_at(item: &FurnitureInstance, p: P) -> Option<f64> {
    if !item.walkable {
        return None;
    }
    let c = [item.pos_mm[0] as f64, item.pos_mm[1] as f64];
    let l = rotate(sub(p, c), -(item.rotation_deg as f64));
    let (hw, hd) = (item.dims_mm[0] as f64 / 2.0, item.dims_mm[1] as f64 / 2.0);
    if l[0].abs() > hw || l[1].abs() > hd {
        return None;
    }
    Some(item.pos_mm[2] as f64 + podium_height_at(item.dims_mm, l[1]))
}

/// Floor height (mm) at plan point `at`: top of the highest walkable item there (ignoring `except`), else 0.
pub fn floor_height_at(p: &Project, at: P, except: Option<Id>) -> f64 {
    p.furniture.iter().filter(|i| Some(i.id) != except).filter_map(|i| surface_height_at(i, at)).fold(0.0, f64::max)
}

/// Ids of furniture that overlap other furniture or walls.
pub fn overlaps(p: &Project) -> Vec<Id> {
    let Some(f) = p.floors.first() else { return vec![] };
    let fps = footprints(f);
    let pieces = wall_pieces(f, &fps, false);
    let items: Vec<(&FurnitureInstance, Vec<[P; 4]>)> = p.furniture.iter().map(|i| (i, footprint_parts(i))).collect();
    let any_overlap = |a: &[[P; 4]], b: &[[P; 4]]| a.iter().any(|x| b.iter().any(|y| convex_overlap(x, y, 1.0)));
    let mut out = Vec::new();
    for (i, (a, pa)) in items.iter().enumerate() {
        let za = (a.pos_mm[2] as f64, a.pos_mm[2] as f64 + a.dims_mm[2] as f64);
        let hit_furniture = items.iter().enumerate().any(|(j, (b, pb))| {
            if i == j || a.overlap_ok || b.overlap_ok {
                return false;
            }
            let zb = (b.pos_mm[2] as f64, b.pos_mm[2] as f64 + b.dims_mm[2] as f64);
            // standing on a podium is fine
            let on = |top: &FurnitureInstance, item: &FurnitureInstance| {
                surface_height_at(top, [item.pos_mm[0] as f64, item.pos_mm[1] as f64]).is_some_and(|h| item.pos_mm[2] as f64 >= h - 5.0)
            };
            if on(b, a) || on(a, b) {
                return false;
            }
            za.0 < zb.1 - 1.0 && zb.0 < za.1 - 1.0 && any_overlap(pa, pb)
        });
        let hit_wall = pieces.iter().any(|w| za.0 < w.z1 - 1.0 && w.z0 < za.1 - 1.0 && pa.iter().any(|x| convex_overlap(x, &w.poly, 1.0)));
        if hit_furniture || hit_wall {
            out.push(a.id);
        }
    }
    out
}

/// Convex colliders for walking (plan, mm).
pub fn walk_colliders(p: &Project) -> Vec<Vec<P>> {
    walk_colliders_closed(p, &Default::default())
}

/// Walk colliders with the doors in `closed` shut.
pub fn walk_colliders_closed(p: &Project, closed: &std::collections::HashSet<Id>) -> Vec<Vec<P>> {
    let Some(f) = p.floors.first() else { return vec![] };
    let fps = footprints(f);
    let mut out: Vec<Vec<P>> = crate::walls::wall_pieces_closed(f, &fps, true, closed).into_iter().map(|w| w.poly).collect();
    for item in &p.furniture {
        // low items (rugs), things hanging above head height and items marked passable don't block
        if item.walkable || item.passable || item.dims_mm[2] < 150 || item.pos_mm[2] > 1800 {
            continue;
        }
        out.extend(footprint_parts(item).into_iter().map(|r| r.to_vec()));
    }
    out
}

/// Move a circle (radius r) from `pos` by `delta`, sliding along colliders.
pub fn walk_move(colliders: &[Vec<P>], pos: P, delta: P, r: f64) -> P {
    let steps = ((len(delta) / (r * 0.5)).ceil() as usize).clamp(1, 16);
    let step = mul(delta, 1.0 / steps as f64);
    let mut p = pos;
    for _ in 0..steps {
        p = add(p, step);
        for _ in 0..3 {
            for poly in colliders {
                let inside = point_in_polygon(p, poly);
                let n = poly.len();
                let mut best = (f64::INFINITY, p);
                for i in 0..n {
                    let (_, q, d) = closest_on_segment(p, poly[i], poly[(i + 1) % n]);
                    if d < best.0 {
                        best = (d, q);
                    }
                }
                let (d, q) = best;
                if inside {
                    let dir = norm(sub(q, p));
                    p = add(q, mul(dir, r));
                } else if d < r {
                    let dir = norm(sub(p, q));
                    p = add(q, mul(dir, r));
                }
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn room() -> Floor {
        let mut f = Floor::new(Uuid::new_v4(), "EG");
        let pts = [(0, 0), (4000, 0), (4000, 4000), (0, 4000)];
        let ids: Vec<Id> = pts
            .iter()
            .map(|&(x, y)| {
                let id = Uuid::new_v4();
                f.nodes.push(Node { id, x_mm: x, y_mm: y });
                id
            })
            .collect();
        for i in 0..4 {
            f.walls.push(Wall::new(Uuid::new_v4(), ids[i], ids[(i + 1) % 4], 200, WallKind::Exterior));
        }
        f
    }

    #[test]
    fn snaps_back_to_north_wall() {
        let f = room();
        // inner face of north wall at y=100. Item depth 600, centre at y=500 → gap 100.
        let s = snap_to_wall(&f, 2000.0, 500.0, [1000, 600, 800], 0.0, 250.0).unwrap();
        assert_eq!(s.y_mm, 400);
        assert!((s.rotation_deg - 0.0).abs() < 1e-3 || (s.rotation_deg - 360.0).abs() < 1e-3, "{s:?}");
    }

    #[test]
    fn never_snaps_to_outer_face() {
        let f = room();
        // just outside the north wall (outer face at y=-100), item would fit outside
        assert!(snap_to_wall(&f, 2000.0, -500.0, [1000, 600, 800], 180.0, 250.0).is_none());
    }

    #[test]
    fn clearances_in_room() {
        let f = room();
        let item = FurnitureInstance {
            id: Uuid::new_v4(),
            asset: Uuid::new_v4(),
            name: "x".into(),
            pos_mm: [2000, 2000, 0],
            rotation_deg: 0.0,
            dims_mm: [1000, 600, 800],
            material: None,
            walkable: false,
            overlap_ok: false,
            footprint: vec![],
            passable: false,
        };
        let c = clearances(&f, &item);
        assert_eq!(c.left, Some(1400.0));
        assert_eq!(c.back, Some(1600.0));
    }

    #[test]
    fn podium_is_walkable_and_carries_furniture() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        p.floors[0] = room();
        // 2 rises (340 mm): front tread 280 mm at 170 mm, rest at 340 mm. Front faces +y.
        let pod = FurnitureInstance {
            id: Uuid::new_v4(),
            asset: Uuid::new_v4(),
            name: "Podest".into(),
            pos_mm: [2000, 1000, 0],
            rotation_deg: 0.0,
            dims_mm: [3000, 1600, 340],
            material: None,
            walkable: true,
            overlap_ok: false,
            footprint: vec![],
            passable: false,
        };
        assert_eq!(floor_height_at(&{ let mut q = p.clone(); q.furniture.push(pod.clone()); q }, [2000.0, 1700.0], None), 170.0);
        let table = FurnitureInstance { id: Uuid::new_v4(), name: "Tisch".into(), pos_mm: [2000, 800, 340], dims_mm: [800, 800, 750], walkable: false, ..pod.clone() };
        p.furniture = vec![pod.clone(), table];
        assert_eq!(floor_height_at(&p, [2000.0, 500.0], None), 340.0);
        assert_eq!(floor_height_at(&p, [2000.0, 500.0], Some(pod.id)), 0.0);
        assert_eq!(floor_height_at(&p, [2000.0, 3000.0], None), 0.0);
        assert!(overlaps(&p).is_empty(), "{:?}", overlaps(&p));
        // the podium itself is no obstacle
        assert_eq!(walk_colliders(&p).len(), 4 + 1); // walls + table, not the podium
    }

    #[test]
    fn overlap_ok_items_are_not_flagged_against_furniture() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        p.floors[0] = room();
        let table = FurnitureInstance {
            id: Uuid::new_v4(),
            asset: Uuid::new_v4(),
            name: "Tisch".into(),
            pos_mm: [2000, 2000, 0],
            rotation_deg: 0.0,
            dims_mm: [1600, 900, 750],
            material: None,
            walkable: false,
            overlap_ok: false,
            footprint: vec![],
            passable: false,
        };
        let chair = FurnitureInstance { id: Uuid::new_v4(), name: "Stuhl".into(), pos_mm: [2000, 2400, 0], dims_mm: [450, 520, 850], ..table.clone() };
        p.furniture = vec![table, chair.clone()];
        assert_eq!(overlaps(&p).len(), 2);
        p.furniture[1].overlap_ok = true;
        assert!(overlaps(&p).is_empty(), "{:?}", overlaps(&p));
    }

    #[test]
    fn walk_stops_at_wall() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        p.floors[0] = room();
        let cols = walk_colliders(&p);
        let end = walk_move(&cols, [2000.0, 2000.0], [0.0, -5000.0], 250.0);
        assert!((end[1] - 350.0).abs() < 1.0, "{end:?}");
    }

    /// L-shaped wardrobe (2 × 2 m box, arms 600 mm deep along the back and left): the open corner is free.
    fn l_wardrobe() -> FurnitureInstance {
        FurnitureInstance {
            id: Uuid::new_v4(),
            asset: Uuid::new_v4(),
            name: "PAX".into(),
            pos_mm: [1200, 1200, 0],
            rotation_deg: 0.0,
            dims_mm: [2000, 2000, 2200],
            material: None,
            walkable: false,
            overlap_ok: false,
            footprint: vec![[0.0, 0.0, 1.0, 0.3], [0.0, 0.3, 0.3, 1.0]],
            passable: false,
        }
    }

    #[test]
    fn footprint_parts_follow_rotation() {
        let mut it = l_wardrobe();
        it.rotation_deg = 90.0;
        let parts = footprint_parts(&it);
        // back arm (local y −1000..−400) ends up right of the centre after turning +x towards +y
        let c = parts[0].iter().fold([0.0, 0.0], |a, p| add(a, mul(*p, 0.25)));
        assert!((c[0] - 1900.0).abs() < 0.1 && (c[1] - 1200.0).abs() < 0.1, "{c:?}");
    }

    #[test]
    fn walk_into_open_corner_of_l_shape() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        p.floors[0] = room();
        let mut it = l_wardrobe();
        it.footprint.clear();
        p.furniture = vec![it.clone()];
        let from = [3000.0, 3000.0];
        let blocked = walk_move(&walk_colliders(&p), from, [-1300.0, -1300.0], 250.0);
        assert!(blocked[0] > 2300.0, "whole box blocks: {blocked:?}");
        p.furniture[0].footprint = l_wardrobe().footprint;
        let end = walk_move(&walk_colliders(&p), from, [-1300.0, -1300.0], 250.0);
        assert!((end[0] - 1700.0).abs() < 1.0 && (end[1] - 1700.0).abs() < 1.0, "{end:?}");
        // a chair in the free corner doesn't overlap, one inside an arm does
        let chair = |x, y| FurnitureInstance { id: Uuid::new_v4(), name: "Stuhl".into(), pos_mm: [x, y, 0], dims_mm: [450, 450, 850], footprint: vec![], ..it.clone() };
        p.furniture.push(chair(1500, 1500));
        assert!(overlaps(&p).is_empty(), "{:?}", overlaps(&p));
        p.furniture.push(chair(1200, 400));
        assert_eq!(overlaps(&p).len(), 2);
        // passable: no collider at all
        p.furniture = vec![FurnitureInstance { passable: true, ..l_wardrobe() }];
        assert_eq!(walk_colliders(&p).len(), 4);
    }
}
