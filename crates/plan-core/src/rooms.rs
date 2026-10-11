//! Rooms are derived from the wall graph (closed faces), never stored.
//! Name and materials hang on an anchor point inside the face.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::geom::*;
use crate::model::*;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DerivedRoom {
    /// Stable-ish key: sorted node ids joined.
    pub key: String,
    /// Polygon along wall centre lines (algebraic CCW).
    pub polygon: Vec<[f64; 2]>,
    /// Net polygon (inner wall faces).
    pub inner: Vec<[f64; 2]>,
    pub area_m2: f64,
    pub meta: Option<Id>,
    pub label: [f64; 2],
    pub walls: Vec<Id>,
}

struct HalfEdge {
    from: Id,
    to: Id,
    wall: Id,
}

pub fn derive_rooms(f: &Floor) -> Vec<DerivedRoom> {
    let pos: HashMap<Id, P> = f
        .nodes
        .iter()
        .map(|n| (n.id, [n.x_mm as f64, n.y_mm as f64]))
        .collect();
    let thick: HashMap<Id, f64> = f.walls.iter().map(|w| (w.id, w.thickness_mm as f64)).collect();
    let mut hes: Vec<HalfEdge> = Vec::new();
    for w in &f.walls {
        if !pos.contains_key(&w.a) || !pos.contains_key(&w.b) || w.a == w.b {
            continue;
        }
        hes.push(HalfEdge { from: w.a, to: w.b, wall: w.id });
        hes.push(HalfEdge { from: w.b, to: w.a, wall: w.id });
    }
    // outgoing half-edges per node, sorted by angle
    let mut out: HashMap<Id, Vec<usize>> = HashMap::new();
    for (i, h) in hes.iter().enumerate() {
        out.entry(h.from).or_default().push(i);
    }
    let angle = |i: usize| {
        let d = sub(pos[&hes[i].to], pos[&hes[i].from]);
        d[1].atan2(d[0])
    };
    for list in out.values_mut() {
        list.sort_by(|&a, &b| angle(a).partial_cmp(&angle(b)).unwrap());
    }
    let twin = |i: usize| i ^ 1;
    let mut visited = vec![false; hes.len()];
    let mut rooms = Vec::new();
    for start in 0..hes.len() {
        if visited[start] {
            continue;
        }
        let mut cycle = Vec::new();
        let mut e = start;
        let mut guard = 0;
        while !visited[e] && guard < hes.len() + 1 {
            guard += 1;
            visited[e] = true;
            cycle.push(e);
            let v = hes[e].to;
            let list = &out[&v];
            let k = list.len();
            let j = list.iter().position(|&x| x == twin(e)).unwrap();
            e = list[(j + k - 1) % k];
        }
        // remove spikes (dangling walls inside a face)
        let mut changed = true;
        while changed && cycle.len() >= 2 {
            changed = false;
            let n = cycle.len();
            for i in 0..n {
                let a = cycle[i];
                let b = cycle[(i + 1) % n];
                if twin(a) == b {
                    if i + 1 < n {
                        cycle.drain(i..=i + 1);
                    } else {
                        cycle.remove(i);
                        cycle.remove(0);
                    }
                    changed = true;
                    break;
                }
            }
        }
        if cycle.len() < 3 {
            continue;
        }
        let polygon: Vec<P> = cycle.iter().map(|&e| pos[&hes[e].from]).collect();
        let area = signed_area(&polygon);
        if area <= 1.0e4 {
            // outer face (negative) or tiny artefact
            continue;
        }
        // inner (net) polygon
        let n = cycle.len();
        let lines: Vec<(P, P)> = cycle
            .iter()
            .map(|&e| {
                let a = pos[&hes[e].from];
                let b = pos[&hes[e].to];
                let d = norm(sub(b, a));
                let t = thick[&hes[e].wall] / 2.0;
                (add(a, mul(perp(d), t)), d)
            })
            .collect();
        let inner: Vec<P> = (0..n)
            .map(|i| {
                let (p0, d0) = lines[(i + n - 1) % n];
                let (p1, d1) = lines[i];
                line_intersect(p0, d0, p1, d1).unwrap_or(p1)
            })
            .collect();
        let inner_area = signed_area(&inner);
        let inner = if inner_area > 0.0 { inner } else { polygon.clone() };
        let area_m2 = signed_area(&inner).max(0.0) / 1.0e6;
        let mut ids: Vec<String> = cycle.iter().map(|&e| hes[e].from.to_string()).collect();
        ids.sort();
        ids.dedup();
        let meta = f
            .rooms
            .iter()
            .find(|r| point_in_polygon([r.anchor[0] as f64, r.anchor[1] as f64], &polygon))
            .map(|r| r.id);
        let label = interior_point(&inner);
        let mut walls: Vec<Id> = cycle.iter().map(|&e| hes[e].wall).collect();
        walls.dedup();
        rooms.push(DerivedRoom {
            key: ids.join(","),
            polygon,
            inner,
            area_m2,
            meta,
            label,
            walls,
        });
    }
    rooms
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn two_rooms() {
        // 6 m x 4 m split at x=3000 by an interior wall
        let mut f = Floor::new(Uuid::new_v4(), "EG");
        let mut node = |x, y| {
            let id = Uuid::new_v4();
            f.nodes.push(Node { id, x_mm: x, y_mm: y });
            id
        };
        let a = node(0, 0);
        let b = node(3000, 0);
        let c = node(6000, 0);
        let d = node(6000, 4000);
        let e = node(3000, 4000);
        let g = node(0, 4000);
        let spur = node(1000, 2000);
        for (p, q, t) in [(a, b, 200), (b, c, 200), (c, d, 200), (d, e, 200), (e, g, 200), (g, a, 200), (b, e, 100), (spur, a, 100)] {
            f.walls.push(Wall::new(Uuid::new_v4(), p, q, t, WallKind::Exterior));
        }
        f.rooms.push(RoomMeta {
            id: Uuid::new_v4(),
            anchor: [4500, 2000],
            name: "Wohnen".into(),
            floor_material: "wood:oak".into(),
            ceiling_material: "paint:white".into(),
            open_ceiling: false,
        });
        let rooms = derive_rooms(&f);
        assert_eq!(rooms.len(), 2, "{rooms:#?}");
        let named = rooms.iter().find(|r| r.meta.is_some()).unwrap();
        // net: (3000-100-50) x (4000-200) = 2850 x 3800
        assert!((named.area_m2 - 2.85 * 3.8).abs() < 1e-6, "{}", named.area_m2);
    }
}
