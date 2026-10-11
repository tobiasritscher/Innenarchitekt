//! Commands: higher-level edits that compile to op batches (not applied).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::geom::*;
use crate::model::*;
use crate::ops::{apply, Op, OpError};

/// Accumulates ops while keeping a working copy so later steps see earlier ones.
pub struct Builder {
    pub p: Project,
    pub ops: Vec<Op>,
    pub floor: Id,
}

impl Builder {
    pub fn new(p: &Project, floor: Id) -> Result<Self, OpError> {
        if p.floor(floor).is_none() {
            return Err(OpError::FloorNotFound(floor));
        }
        Ok(Builder { p: p.clone(), ops: vec![], floor })
    }
    pub fn push(&mut self, op: Op) -> Result<(), OpError> {
        apply(&mut self.p, &op)?;
        self.ops.push(op);
        Ok(())
    }
    pub fn f(&self) -> &Floor {
        self.p.floor(self.floor).unwrap()
    }
    fn pos(&self, n: Id) -> P {
        let n = self.f().node(n).unwrap();
        [n.x_mm as f64, n.y_mm as f64]
    }
    fn new_node(&mut self, x: i32, y: i32) -> Result<Id, OpError> {
        let id = Uuid::new_v4();
        self.push(Op::PutNode { floor: self.floor, node: Node { id, x_mm: x, y_mm: y } })?;
        Ok(id)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ChainPoint {
    /// Snap target: an existing node …
    pub node: Option<Id>,
    /// … or a wall to split at (x, y).
    pub wall: Option<Id>,
    pub x: i32,
    pub y: i32,
}

/// Split `wall` at the projection of (x, y). Returns the new node id.
pub fn split_wall_b(b: &mut Builder, wall: Id, x: f64, y: f64) -> Result<Id, OpError> {
    let w = b.f().wall(wall).cloned().ok_or(OpError::NotFound(wall))?;
    let (pa, pb) = b.f().wall_points(&w).ok_or(OpError::NotFound(wall))?;
    let (t, q, _) = closest_on_segment([x, y], pa, pb);
    let l = dist(pa, pb);
    if t * l < 1.0 {
        return Ok(w.a);
    }
    if (1.0 - t) * l < 1.0 {
        return Ok(w.b);
    }
    let n = b.new_node(q[0].round() as i32, q[1].round() as i32)?;
    let u = t * l;
    let w2 = Wall { id: Uuid::new_v4(), a: n, b: w.b, ..w.clone() };
    let w1 = Wall { b: n, ..w.clone() };
    b.push(Op::PutWall { floor: b.floor, wall: w2.clone() })?;
    let openings: Vec<Opening> = b.f().openings_on(w.id).cloned().collect();
    for o in openings {
        if o.offset_mm as f64 >= u {
            let o2 = Opening { wall: w2.id, offset_mm: (o.offset_mm as f64 - u).round() as u32, ..o };
            b.push(Op::PutOpening { floor: b.floor, opening: o2 })?;
        }
    }
    b.push(Op::PutWall { floor: b.floor, wall: w1 })?;
    Ok(n)
}

pub fn split_wall(p: &Project, floor: Id, wall: Id, x: i32, y: i32) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    split_wall_b(&mut b, wall, x as f64, y as f64)?;
    Ok(b.ops)
}

fn resolve_point(b: &mut Builder, cp: &ChainPoint) -> Result<Id, OpError> {
    if let Some(n) = cp.node {
        if b.f().node(n).is_some() {
            return Ok(n);
        }
    }
    let p = [cp.x as f64, cp.y as f64];
    if let Some(w) = cp.wall {
        if b.f().wall(w).is_some() {
            return split_wall_b(b, w, p[0], p[1]);
        }
    }
    if let Some(n) = b.f().nodes.iter().find(|n| dist([n.x_mm as f64, n.y_mm as f64], p) < 1.5) {
        return Ok(n.id);
    }
    let on_wall = b.f().walls.iter().find_map(|w| {
        let (a, c) = b.f().wall_points(w)?;
        let (t, _, d) = closest_on_segment(p, a, c);
        (d < 2.0 && t > 0.0 && t < 1.0).then_some(w.id)
    });
    if let Some(w) = on_wall {
        return split_wall_b(b, w, p[0], p[1]);
    }
    b.new_node(cp.x, cp.y)
}

/// Add a wall between two nodes, splitting at crossings with existing walls.
fn add_segment(b: &mut Builder, n1: Id, n2: Id, thickness: u32, kind: WallKind) -> Result<(), OpError> {
    if n1 == n2 {
        return Ok(());
    }
    let p = b.pos(n1);
    let q = b.pos(n2);
    // crossings and nodes on the segment
    let mut stops: Vec<(f64, Option<Id>, Option<Id>, P)> = Vec::new(); // (t, node, wall, point)
    for n in &b.f().nodes {
        if n.id == n1 || n.id == n2 {
            continue;
        }
        let np = [n.x_mm as f64, n.y_mm as f64];
        let (t, _, d) = closest_on_segment(np, p, q);
        if d < 1.5 && t > 0.0 && t < 1.0 {
            stops.push((t, Some(n.id), None, np));
        }
    }
    for w in &b.f().walls {
        if w.a == n1 || w.b == n1 || w.a == n2 || w.b == n2 {
            continue;
        }
        let Some((a, c)) = b.f().wall_points(w) else { continue };
        if let Some((t, u, x)) = segment_intersect(p, q, a, c) {
            let lw = dist(a, c);
            if t > 1e-6 && t < 1.0 - 1e-6 && u * lw > 1.5 && (1.0 - u) * lw > 1.5 {
                stops.push((t, None, Some(w.id), x));
            }
        }
    }
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut chain = vec![n1];
    for (_, node, wall, x) in stops {
        let id = match (node, wall) {
            (Some(n), _) => n,
            (None, Some(w)) => split_wall_b(b, w, x[0], x[1])?,
            _ => continue,
        };
        if *chain.last().unwrap() != id {
            chain.push(id);
        }
    }
    chain.push(n2);
    for pair in chain.windows(2) {
        let (a, c) = (pair[0], pair[1]);
        if a == c {
            continue;
        }
        let exists = b.f().walls.iter().any(|w| (w.a == a && w.b == c) || (w.a == c && w.b == a));
        if !exists {
            b.push(Op::PutWall { floor: b.floor, wall: Wall::new(Uuid::new_v4(), a, c, thickness, kind) })?;
        }
    }
    Ok(())
}

pub fn add_wall_chain(
    p: &Project,
    floor: Id,
    points: &[ChainPoint],
    closed: bool,
    thickness_mm: u32,
    kind: WallKind,
) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    let mut ids = Vec::new();
    for cp in points {
        let id = resolve_point(&mut b, cp)?;
        if ids.last() != Some(&id) {
            ids.push(id);
        }
    }
    if closed && ids.len() > 2 && ids.first() != ids.last() {
        ids.push(ids[0]);
    }
    for pair in ids.windows(2) {
        add_segment(&mut b, pair[0], pair[1], thickness_mm, kind)?;
    }
    Ok(b.ops)
}

fn delete_wall_b(b: &mut Builder, wall: Id, prune_nodes: bool) -> Result<(), OpError> {
    let w = b.f().wall(wall).cloned().ok_or(OpError::NotFound(wall))?;
    let ops: Vec<Id> = b.f().openings_on(wall).map(|o| o.id).collect();
    for o in ops {
        b.push(Op::DeleteOpening { floor: b.floor, id: o })?;
    }
    b.push(Op::DeleteWall { floor: b.floor, id: wall })?;
    if prune_nodes {
        for n in [w.a, w.b] {
            if b.f().walls_at(n).next().is_none() {
                b.push(Op::DeleteNode { floor: b.floor, id: n })?;
            }
        }
    }
    Ok(())
}

pub fn delete_wall(p: &Project, floor: Id, wall: Id) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    delete_wall_b(&mut b, wall, true)?;
    Ok(b.ops)
}

/// Delete a node. With exactly two walls, they are merged into one (corner removed).
pub fn delete_node(p: &Project, floor: Id, node: Id) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    let walls: Vec<Wall> = b.f().walls_at(node).cloned().collect();
    if walls.len() == 2 {
        let (w1, w2) = (&walls[0], &walls[1]);
        let o1 = w1.other(node);
        let o2 = w2.other(node);
        let dup = b.f().walls.iter().any(|w| (w.a == o1 && w.b == o2) || (w.a == o2 && w.b == o1));
        if o1 != o2 && !dup {
            // new wall o1 → o2 keeps w1's properties; finishes swap if w1 ran the other way.
            let p1 = b.pos(o1);
            let pn = b.pos(node);
            let p2 = b.pos(o2);
            let (l1, l2) = (dist(p1, pn), dist(pn, p2));
            let mut merged = Wall { id: Uuid::new_v4(), a: o1, b: o2, ..w1.clone() };
            if w1.a != o1 {
                std::mem::swap(&mut merged.finish_left, &mut merged.finish_right);
            }
            b.push(Op::PutWall { floor: b.floor, wall: merged.clone() })?;
            let ml = dist(p1, p2);
            let mut moved = Vec::new();
            for o in b.f().openings_on(w1.id).chain(b.f().openings_on(w2.id)).cloned().collect::<Vec<_>>() {
                let (off, wd) = (o.offset_mm as f64, o.width_mm as f64);
                let start = if o.wall == w1.id {
                    if w1.a == o1 { off } else { l1 - off - wd }
                } else if w2.a == node {
                    l1 + off
                } else {
                    l1 + l2 - off - wd
                };
                let start = start.clamp(0.0, (ml - wd).max(0.0));
                moved.push(Opening { wall: merged.id, offset_mm: start.round() as u32, ..o });
            }
            for w in [w1, w2] {
                for o in b.f().openings_on(w.id).map(|o| o.id).collect::<Vec<_>>() {
                    b.push(Op::DeleteOpening { floor: b.floor, id: o })?;
                }
                b.push(Op::DeleteWall { floor: b.floor, id: w.id })?;
            }
            for o in moved {
                b.push(Op::PutOpening { floor: b.floor, opening: o })?;
            }
            b.push(Op::DeleteNode { floor: b.floor, id: node })?;
            return Ok(b.ops);
        }
    }
    for w in walls {
        delete_wall_b(&mut b, w.id, false)?;
    }
    b.push(Op::DeleteNode { floor: b.floor, id: node })?;
    Ok(b.ops)
}

/// Merge node `remove` into `keep` (e.g. after dragging a corner onto another).
pub fn merge_nodes(p: &Project, floor: Id, keep: Id, remove: Id) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    if keep == remove {
        return Ok(vec![]);
    }
    let walls: Vec<Wall> = b.f().walls_at(remove).cloned().collect();
    for w in walls {
        let other = w.other(remove);
        let dup = b.f().walls.iter().any(|x| x.id != w.id && ((x.a == keep && x.b == other) || (x.a == other && x.b == keep)));
        if other == keep || dup {
            delete_wall_b(&mut b, w.id, false)?;
        } else {
            let nw = if w.a == remove { Wall { a: keep, ..w } } else { Wall { b: keep, ..w } };
            b.push(Op::PutWall { floor: b.floor, wall: nw })?;
        }
    }
    b.push(Op::DeleteNode { floor: b.floor, id: remove })?;
    Ok(b.ops)
}

/// Move a wall parallel by `offset_mm` along its left normal. Single neighbours are extended/shortened.
pub fn move_wall(p: &Project, floor: Id, wall: Id, offset_mm: f64) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    let w = b.f().wall(wall).cloned().ok_or(OpError::NotFound(wall))?;
    let (pa, pb) = b.f().wall_points(&w).ok_or(OpError::NotFound(wall))?;
    let d = norm(sub(pb, pa));
    let nl = left_normal(d);
    let line_p = add(pa, mul(nl, offset_mm));
    for (node, pos) in [(w.a, pa), (w.b, pb)] {
        let others: Vec<Wall> = b.f().walls_at(node).filter(|x| x.id != w.id).cloned().collect();
        let mut target = add(pos, mul(nl, offset_mm));
        if others.len() == 1 {
            let o = &others[0];
            let op = b.pos(o.other(node));
            let e = norm(sub(op, pos));
            if cross(e, d).abs() > 0.2 {
                if let Some(x) = line_intersect(pos, e, line_p, d) {
                    target = x;
                }
            }
        }
        b.push(Op::PutNode {
            floor: b.floor,
            node: Node { id: node, x_mm: target[0].round() as i32, y_mm: target[1].round() as i32 },
        })?;
    }
    Ok(b.ops)
}

/// Set the length of a wall; the node at `fixed_a ? a : b` stays.
pub fn set_wall_length(p: &Project, floor: Id, wall: Id, length_mm: u32, fixed_a: bool) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    let w = b.f().wall(wall).cloned().ok_or(OpError::NotFound(wall))?;
    let (pa, pb) = b.f().wall_points(&w).ok_or(OpError::NotFound(wall))?;
    let old = dist(pa, pb);
    if old < 1.0 || length_mm == 0 {
        return Err(OpError::Invalid("Wandlänge".into()));
    }
    let d = norm(sub(pb, pa));
    let l = length_mm as f64;
    if fixed_a {
        let t = add(pa, mul(d, l));
        b.push(Op::PutNode { floor: b.floor, node: Node { id: w.b, x_mm: t[0].round() as i32, y_mm: t[1].round() as i32 } })?;
    } else {
        let t = sub(pb, mul(d, l));
        b.push(Op::PutNode { floor: b.floor, node: Node { id: w.a, x_mm: t[0].round() as i32, y_mm: t[1].round() as i32 } })?;
        // openings are measured from a: keep them where they are in space
        let delta = l - old;
        for o in b.f().openings_on(w.id).cloned().collect::<Vec<_>>() {
            let off = (o.offset_mm as f64 + delta).max(0.0).round() as u32;
            b.push(Op::PutOpening { floor: b.floor, opening: Opening { offset_mm: off, ..o } })?;
        }
    }
    Ok(b.ops)
}

pub fn delete_furniture(p: &Project, id: Id) -> Result<Vec<Op>, OpError> {
    let mut ops: Vec<Op> = p.lights.iter().filter(|l| l.attached_to == Some(id)).map(|l| Op::DeleteLight { id: l.id }).collect();
    if !p.furniture.iter().any(|f| f.id == id) {
        return Err(OpError::NotFound(id));
    }
    ops.push(Op::DeleteFurniture { id });
    Ok(ops)
}

/// Scale and translate a floor: p' = (p − origin)·scale + origin + translate.
/// Lengths along walls (openings) scale too; thickness is scaled and rounded to 5 mm.
pub fn transform_floor(p: &Project, floor: Id, scale: f64, origin: [f64; 2], translate: [f64; 2]) -> Result<Vec<Op>, OpError> {
    let mut b = Builder::new(p, floor)?;
    let tf = |q: P| add(add(mul(sub(q, origin), scale), origin), translate);
    let f = b.f().clone();
    for n in &f.nodes {
        let q = tf([n.x_mm as f64, n.y_mm as f64]);
        b.push(Op::PutNode { floor, node: Node { id: n.id, x_mm: q[0].round() as i32, y_mm: q[1].round() as i32 } })?;
    }
    for w in &f.walls {
        let t = ((w.thickness_mm as f64 * scale / 5.0).round() * 5.0).max(50.0) as u32;
        b.push(Op::PutWall { floor, wall: Wall { thickness_mm: t, ..w.clone() } })?;
    }
    for o in &f.openings {
        let o2 = Opening {
            offset_mm: (o.offset_mm as f64 * scale).round() as u32,
            width_mm: ((o.width_mm as f64 * scale).round() as u32).max(1),
            ..o.clone()
        };
        b.push(Op::PutOpening { floor, opening: o2 })?;
    }
    for r in &f.rooms {
        let q = tf([r.anchor[0] as f64, r.anchor[1] as f64]);
        b.push(Op::PutRoom { floor, room: RoomMeta { anchor: [q[0].round() as i32, q[1].round() as i32], ..r.clone() } })?;
    }
    if let Some(img) = &f.reference_image {
        let o = tf(img.origin_mm);
        let img2 = RefImage { origin_mm: o, mm_per_px: img.mm_per_px * scale, ..img.clone() };
        b.push(Op::SetFloor { floor, name: f.name.clone(), ceiling_height_mm: f.ceiling_height_mm, reference_image: Some(img2) })?;
    }
    Ok(b.ops)
}

/// Ops that turn `old` into `new` (by element id). Deletes first (openings → walls → nodes), then puts.
pub fn diff(old: &Project, new: &Project) -> Vec<Op> {
    let mut ops = Vec::new();
    if old.name != new.name || old.location != new.location || old.north_angle_deg != new.north_angle_deg {
        ops.push(Op::SetProject { name: new.name.clone(), location: new.location.clone(), north_angle_deg: new.north_angle_deg });
    }
    let mut puts = Vec::new();
    for nf in &new.floors {
        let Some(of) = old.floor(nf.id) else { continue };
        let floor = nf.id;
        for o in &of.openings {
            if nf.opening(o.id).is_none() {
                ops.push(Op::DeleteOpening { floor, id: o.id });
            }
        }
        for r in &of.rooms {
            if !nf.rooms.iter().any(|x| x.id == r.id) {
                ops.push(Op::DeleteRoom { floor, id: r.id });
            }
        }
        for w in &of.walls {
            if nf.wall(w.id).is_none() {
                ops.push(Op::DeleteWall { floor, id: w.id });
            }
        }
        for n in &of.nodes {
            if nf.node(n.id).is_none() {
                ops.push(Op::DeleteNode { floor, id: n.id });
            }
        }
        for n in &nf.nodes {
            if of.node(n.id) != Some(n) {
                puts.push(Op::PutNode { floor, node: *n });
            }
        }
        for w in &nf.walls {
            if of.wall(w.id) != Some(w) {
                puts.push(Op::PutWall { floor, wall: w.clone() });
            }
        }
        for o in &nf.openings {
            if of.opening(o.id) != Some(o) {
                puts.push(Op::PutOpening { floor, opening: o.clone() });
            }
        }
        for r in &nf.rooms {
            if of.rooms.iter().find(|x| x.id == r.id) != Some(r) {
                puts.push(Op::PutRoom { floor, room: r.clone() });
            }
        }
        if of.name != nf.name || of.ceiling_height_mm != nf.ceiling_height_mm || of.reference_image != nf.reference_image {
            puts.push(Op::SetFloor {
                floor,
                name: nf.name.clone(),
                ceiling_height_mm: nf.ceiling_height_mm,
                reference_image: nf.reference_image.clone(),
            });
        }
    }
    for l in &old.lights {
        if !new.lights.iter().any(|x| x.id == l.id) {
            ops.push(Op::DeleteLight { id: l.id });
        }
    }
    for f in &old.furniture {
        if !new.furniture.iter().any(|x| x.id == f.id) {
            ops.push(Op::DeleteFurniture { id: f.id });
        }
    }
    for f in &new.furniture {
        if old.furniture.iter().find(|x| x.id == f.id) != Some(f) {
            puts.push(Op::PutFurniture { item: f.clone() });
        }
    }
    for l in &new.lights {
        if old.lights.iter().find(|x| x.id == l.id) != Some(l) {
            puts.push(Op::PutLight { light: l.clone() });
        }
    }
    ops.extend(puts);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::apply_batch;
    use crate::rooms::derive_rooms;

    fn cp(x: i32, y: i32) -> ChainPoint {
        ChainPoint { node: None, wall: None, x, y }
    }

    #[test]
    fn closed_chain_and_crossing_wall() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = p.floors[0].id;
        let ops = add_wall_chain(&p, f, &[cp(0, 0), cp(6000, 0), cp(6000, 4000), cp(0, 4000)], true, 200, WallKind::Exterior).unwrap();
        apply_batch(&mut p, &ops).unwrap();
        assert_eq!(p.floors[0].walls.len(), 4);
        // interior wall across the whole room from outside to outside: splits both exterior walls
        let ops = add_wall_chain(&p, f, &[cp(3000, -1000), cp(3000, 5000)], false, 100, WallKind::Interior).unwrap();
        apply_batch(&mut p, &ops).unwrap();
        let rooms = derive_rooms(&p.floors[0]);
        assert_eq!(rooms.len(), 2);
    }

    #[test]
    fn delete_middle_node_merges() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = p.floors[0].id;
        let ops = add_wall_chain(&p, f, &[cp(0, 0), cp(2000, 0), cp(5000, 0)], false, 200, WallKind::Exterior).unwrap();
        apply_batch(&mut p, &ops).unwrap();
        let fl = &p.floors[0];
        let mid = fl.nodes.iter().find(|n| n.x_mm == 2000).unwrap().id;
        let second = fl.walls.iter().find(|w| fl.node(w.a).unwrap().x_mm == 2000).unwrap().id;
        let op = Opening { id: Uuid::new_v4(), wall: second, kind: OpeningKind::Passage, offset_mm: 500, width_mm: 900, height_mm: 2000 };
        apply_batch(&mut p, &[Op::PutOpening { floor: f, opening: op }]).unwrap();
        let ops = delete_node(&p, f, mid).unwrap();
        apply_batch(&mut p, &ops).unwrap();
        let fl = &p.floors[0];
        assert_eq!(fl.walls.len(), 1);
        assert_eq!(fl.openings[0].offset_mm, 2500);
    }

    #[test]
    fn set_length_and_move_wall() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = p.floors[0].id;
        let ops = add_wall_chain(&p, f, &[cp(0, 0), cp(4000, 0), cp(4000, 3000), cp(0, 3000)], true, 200, WallKind::Exterior).unwrap();
        apply_batch(&mut p, &ops).unwrap();
        let w = p.floors[0].walls.iter().find(|w| {
            let fl = &p.floors[0];
            fl.node(w.a).unwrap().x_mm == 4000 && fl.node(w.b).unwrap().x_mm == 4000
        }).unwrap().clone();
        // move east wall (direction south, left normal = east) by +500
        let ops = move_wall(&p, f, w.id, 500.0).unwrap();
        apply_batch(&mut p, &ops).unwrap();
        let rooms = derive_rooms(&p.floors[0]);
        assert!((rooms[0].area_m2 - 4.3 * 2.8).abs() < 1e-6, "{}", rooms[0].area_m2);
        let ops = set_wall_length(&p, f, w.id, 2000, true).unwrap();
        apply_batch(&mut p, &ops).unwrap();
        assert_eq!(p.floors[0].node(w.b).unwrap().y_mm, 2000);
    }

    #[test]
    fn diff_roundtrip() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = p.floors[0].id;
        let ops = add_wall_chain(&p, f, &[cp(0, 0), cp(4000, 0), cp(4000, 3000)], false, 200, WallKind::Exterior).unwrap();
        let mut q = p.clone();
        apply_batch(&mut q, &ops).unwrap();
        let d = diff(&p, &q);
        apply_batch(&mut p, &d).unwrap();
        assert_eq!(p.floors[0].walls.len(), 2);
        let d2 = diff(&q, &Project { floors: vec![Floor { id: f, ..Floor::new(f, "EG") }], ..q.clone() });
        apply_batch(&mut q, &d2).unwrap();
        assert!(q.floors[0].walls.is_empty() && q.floors[0].nodes.is_empty());
    }
}
