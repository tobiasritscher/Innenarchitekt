//! Triangle meshes derived from the model (three.js convention, metres, Y up).
//! Plan (x, y) mm → (X, Z) m, height mm → Y m.

use serde::{Deserialize, Serialize};

use crate::geom::*;
use crate::model::*;
use crate::rooms::{derive_rooms, DerivedRoom};
use crate::walls::{footprints, wall_pieces, WallPiece};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MeshGroup {
    pub start: u32,
    pub count: u32,
    /// Semantic slot, e.g. "left", "right", "edge", "top" for walls; "main", "accent" for primitives.
    pub slot: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Mesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
    pub groups: Vec<MeshGroup>,
}

/// Collects triangles per slot, then emits a mesh with one group per slot.
#[derive(Default)]
pub struct MeshBuilder {
    slots: Vec<(String, Mesh)>,
}

impl MeshBuilder {
    fn slot(&mut self, name: &str) -> &mut Mesh {
        if let Some(i) = self.slots.iter().position(|(n, _)| n == name) {
            return &mut self.slots[i].1;
        }
        self.slots.push((name.to_string(), Mesh::default()));
        &mut self.slots.last_mut().unwrap().1
    }

    /// Add a planar polygon (3D, metres) with a normal; vertices must be convex and ordered CCW around the normal.
    pub fn quad_fan(&mut self, slot: &str, pts: &[[f32; 3]], n: [f32; 3], uvs: &[[f32; 2]]) {
        let m = self.slot(slot);
        let base = (m.positions.len() / 3) as u32;
        for (p, uv) in pts.iter().zip(uvs) {
            m.positions.extend_from_slice(p);
            m.normals.extend_from_slice(&n);
            m.uvs.extend_from_slice(uv);
        }
        for i in 1..pts.len() as u32 - 1 {
            m.indices.extend_from_slice(&[base, base + i, base + i + 1]);
        }
    }

    pub fn triangles(&mut self, slot: &str, pts: &[[f32; 3]], n: [f32; 3], uvs: &[[f32; 2]], tris: &[[u32; 3]]) {
        let m = self.slot(slot);
        let base = (m.positions.len() / 3) as u32;
        for (p, uv) in pts.iter().zip(uvs) {
            m.positions.extend_from_slice(p);
            m.normals.extend_from_slice(&n);
            m.uvs.extend_from_slice(uv);
        }
        for t in tris {
            m.indices.extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
        }
    }

    pub fn build(self) -> Mesh {
        let mut out = Mesh::default();
        for (name, m) in self.slots {
            let base = (out.positions.len() / 3) as u32;
            let start = out.indices.len() as u32;
            out.positions.extend(m.positions);
            out.normals.extend(m.normals);
            out.uvs.extend(m.uvs);
            out.indices.extend(m.indices.iter().map(|i| i + base));
            out.groups.push(MeshGroup { start, count: m.indices.len() as u32, slot: name });
        }
        out
    }
}

fn v3(p: P, z: f64) -> [f32; 3] {
    [(p[0] / 1000.0) as f32, (z / 1000.0) as f32, (p[1] / 1000.0) as f32]
}

/// Extrude a plan polygon (mm) between z0 and z1 (mm). `side_slot` decides the slot per side edge.
pub fn extrude(
    b: &mut MeshBuilder,
    poly: &[P],
    z0: f64,
    z1: f64,
    top_slot: Option<&str>,
    bottom_slot: Option<&str>,
    side_slot: &dyn Fn(P) -> String,
) {
    let mut poly = poly.to_vec();
    // In 3D (X=x, Z=y, Y up), algebraic CCW in (x, y) appears clockwise when seen from +Y.
    // Normalise to algebraic CW so that the top face (seen from +Y) is CCW.
    if signed_area(&poly) > 0.0 {
        poly.reverse();
    }
    let n = poly.len();
    let tris = triangulate(&poly);
    if let Some(slot) = top_slot {
        let pts: Vec<[f32; 3]> = poly.iter().map(|p| v3(*p, z1)).collect();
        let uvs: Vec<[f32; 2]> = poly.iter().map(|p| [(p[0] / 1000.0) as f32, (p[1] / 1000.0) as f32]).collect();
        let t: Vec<[u32; 3]> = tris.iter().map(|t| [t[0] as u32, t[1] as u32, t[2] as u32]).collect();
        b.triangles(slot, &pts, [0.0, 1.0, 0.0], &uvs, &t);
    }
    if let Some(slot) = bottom_slot {
        let pts: Vec<[f32; 3]> = poly.iter().map(|p| v3(*p, z0)).collect();
        let uvs: Vec<[f32; 2]> = poly.iter().map(|p| [(p[0] / 1000.0) as f32, (p[1] / 1000.0) as f32]).collect();
        let t: Vec<[u32; 3]> = tris.iter().map(|t| [t[0] as u32, t[2] as u32, t[1] as u32]).collect();
        b.triangles(slot, &pts, [0.0, -1.0, 0.0], &uvs, &t);
    }
    // Sides: polygon is algebraic CW, so the outward normal of edge p→q is perp(q−p) (algebraic +90°)... verify:
    // for CW order, interior is to the right of each edge; right of d (algebraic) = (d.y, −d.x).
    for i in 0..n {
        let p = poly[i];
        let q = poly[(i + 1) % n];
        let d = sub(q, p);
        let l = len(d);
        if l < 1e-6 {
            continue;
        }
        let out = norm([-d[1], d[0]]);
        let slot = side_slot(out);
        let u0 = (dot(p, norm(d)) / 1000.0) as f32;
        let u1 = u0 + (l / 1000.0) as f32;
        let (h0, h1) = ((z0 / 1000.0) as f32, (z1 / 1000.0) as f32);
        // Seen from outside, CCW: p bottom, p top, q top, q bottom? Determine with outward normal.
        let pts = [v3(p, z0), v3(q, z0), v3(q, z1), v3(p, z1)];
        let nrm = [out[0] as f32, 0.0, out[1] as f32];
        let uvs = [[u0, h0], [u1, h0], [u1, h1], [u0, h1]];
        // Ensure winding matches normal.
        let e1 = sub3(pts[1], pts[0]);
        let e2 = sub3(pts[2], pts[0]);
        let c = cross3(e1, e2);
        if c[0] * nrm[0] + c[1] * nrm[1] + c[2] * nrm[2] >= 0.0 {
            b.quad_fan(&slot, &pts, nrm, &uvs);
        } else {
            let pts = [pts[0], pts[3], pts[2], pts[1]];
            let uvs = [uvs[0], uvs[3], uvs[2], uvs[1]];
            b.quad_fan(&slot, &pts, nrm, &uvs);
        }
    }
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct WallMesh {
    pub wall: Id,
    pub mesh: Mesh,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RoomMesh {
    pub room: DerivedRoom,
    pub floor: Mesh,
    pub ceiling: Mesh,
}

/// Placement of an opening for doors/glass (metres, three.js frame).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct OpeningPlacement {
    pub opening: Id,
    pub wall: Id,
    pub kind: OpeningKind,
    /// Centre of the opening at its bottom (sill), on the wall centre line.
    pub position: [f32; 3],
    /// Rotation around Y (rad) so that local +X runs along the wall a→b and local +Z points to the wall's right side.
    pub rotation_y: f32,
    pub width: f32,
    pub height: f32,
    pub thickness: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SceneMeshes {
    pub walls: Vec<WallMesh>,
    pub rooms: Vec<RoomMesh>,
    pub openings: Vec<OpeningPlacement>,
    pub ceiling_height: f32,
}

fn piece_side_slot(p: &WallPiece) -> impl Fn(P) -> String + '_ {
    move |n: P| {
        let s = dot(n, p.left);
        if s > 0.7 {
            "left".into()
        } else if s < -0.7 {
            "right".into()
        } else {
            "edge".into()
        }
    }
}

pub fn scene_meshes(f: &Floor) -> SceneMeshes {
    let fps = footprints(f);
    let pieces = wall_pieces(f, &fps, false);
    let h = f.ceiling_height_mm as f64;
    let mut walls = Vec::new();
    for w in &f.walls {
        let mut b = MeshBuilder::default();
        let wh = w.height(f.ceiling_height_mm) as f64;
        for p in pieces.iter().filter(|p| p.wall == w.id) {
            let top = if (p.z1 - wh).abs() < 0.5 { "top" } else { "edge" };
            let bottom = if p.z0 > 0.5 { Some("edge") } else { None };
            extrude(&mut b, &p.poly, p.z0, p.z1, Some(top), bottom, &piece_side_slot(p));
        }
        walls.push(WallMesh { wall: w.id, mesh: b.build() });
    }
    let rooms = derive_rooms(f)
        .into_iter()
        .map(|r| {
            let mut fb = MeshBuilder::default();
            flat_polygon(&mut fb, "floor", &r.polygon, 0.0, true);
            let mut cb = MeshBuilder::default();
            flat_polygon(&mut cb, "ceiling", &r.polygon, h, false);
            RoomMesh { room: r, floor: fb.build(), ceiling: cb.build() }
        })
        .collect();
    let openings = f
        .openings
        .iter()
        .filter_map(|o| {
            let w = f.wall(o.wall)?;
            let (a, bpt) = f.wall_points(w)?;
            let l = dist(a, bpt);
            if l < 1.0 {
                return None;
            }
            let d = norm(sub(bpt, a));
            let c = add(a, mul(d, o.offset_mm as f64 + o.width_mm as f64 / 2.0));
            let sill = o.kind.sill_mm() as f64;
            Some(OpeningPlacement {
                opening: o.id,
                wall: w.id,
                kind: o.kind,
                position: v3(c, sill),
                // three.js: rotation.y = φ maps local +X to (cos φ, 0, −sin φ). We want (d.x, 0, d.y).
                rotation_y: (-d[1]).atan2(d[0]) as f32,
                width: o.width_mm as f32 / 1000.0,
                height: o.height_mm.min(w.height(f.ceiling_height_mm).saturating_sub(o.kind.sill_mm())) as f32 / 1000.0,
                thickness: w.thickness_mm as f32 / 1000.0,
            })
        })
        .collect();
    SceneMeshes { walls, rooms, openings, ceiling_height: (h / 1000.0) as f32 }
}

/// Horizontal polygon at height z. `up` = normal points up (floor) or down (ceiling).
pub fn flat_polygon(b: &mut MeshBuilder, slot: &str, poly: &[P], z: f64, up: bool) {
    let mut poly = poly.to_vec();
    if signed_area(&poly) > 0.0 {
        poly.reverse();
    }
    let tris = triangulate(&poly);
    let pts: Vec<[f32; 3]> = poly.iter().map(|p| v3(*p, z)).collect();
    let uvs: Vec<[f32; 2]> = poly.iter().map(|p| [(p[0] / 1000.0) as f32, (p[1] / 1000.0) as f32]).collect();
    let t: Vec<[u32; 3]> = tris
        .iter()
        .map(|t| if up { [t[0] as u32, t[1] as u32, t[2] as u32] } else { [t[0] as u32, t[2] as u32, t[1] as u32] })
        .collect();
    b.triangles(slot, &pts, if up { [0.0, 1.0, 0.0] } else { [0.0, -1.0, 0.0] }, &uvs, &t);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_face_points_up() {
        let mut b = MeshBuilder::default();
        let sq = vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]];
        extrude(&mut b, &sq, 0.0, 1000.0, Some("top"), Some("bottom"), &|_| "side".into());
        let m = b.build();
        // every triangle's geometric normal must agree with the stored vertex normal
        for t in m.indices.chunks(3) {
            let p = |i: u32| [m.positions[i as usize * 3], m.positions[i as usize * 3 + 1], m.positions[i as usize * 3 + 2]];
            let c = cross3(sub3(p(t[1]), p(t[0])), sub3(p(t[2]), p(t[0])));
            let n = [m.normals[t[0] as usize * 3], m.normals[t[0] as usize * 3 + 1], m.normals[t[0] as usize * 3 + 2]];
            assert!(c[0] * n[0] + c[1] * n[1] + c[2] * n[2] > 0.0);
            // and side normals point away from the centre
            let ctr = [0.5f32, 0.5, 0.5];
            let pc = sub3(p(t[0]), ctr);
            assert!(pc[0] * n[0] + pc[1] * n[1] + pc[2] * n[2] > 0.0);
        }
    }

    pub fn assert_windings(m: &Mesh) {
        for t in m.indices.chunks(3) {
            let p = |i: u32| [m.positions[i as usize * 3], m.positions[i as usize * 3 + 1], m.positions[i as usize * 3 + 2]];
            let c = cross3(sub3(p(t[1]), p(t[0])), sub3(p(t[2]), p(t[0])));
            let n = [m.normals[t[0] as usize * 3], m.normals[t[0] as usize * 3 + 1], m.normals[t[0] as usize * 3 + 2]];
            let area = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
            if area < 1e-9 {
                continue;
            }
            assert!(c[0] * n[0] + c[1] * n[1] + c[2] * n[2] > 0.0, "bad winding {t:?}");
        }
    }

    #[test]
    fn primitives_and_walls_wind_correctly() {
        use crate::primitives::*;
        for s in PrimitiveShape::ALL {
            assert_windings(&primitive_mesh(s, s.default_dims()));
        }
        let mut f = crate::model::Floor::new(uuid::Uuid::new_v4(), "EG");
        let ids: Vec<_> = [(0, 0), (4000, 0), (4000, 3000), (0, 3000)]
            .iter()
            .map(|&(x, y)| {
                let id = uuid::Uuid::new_v4();
                f.nodes.push(crate::model::Node { id, x_mm: x, y_mm: y });
                id
            })
            .collect();
        for i in 0..4 {
            f.walls.push(crate::model::Wall::new(uuid::Uuid::new_v4(), ids[i], ids[(i + 1) % 4], 200, crate::model::WallKind::Exterior));
        }
        f.openings.push(crate::model::Opening {
            id: uuid::Uuid::new_v4(),
            wall: f.walls[0].id,
            kind: crate::model::OpeningKind::Window { sill_mm: 900 },
            offset_mm: 1000,
            width_mm: 1200,
            height_mm: 1200,
        });
        let sm = scene_meshes(&f);
        for w in &sm.walls {
            assert_windings(&w.mesh);
        }
        for r in &sm.rooms {
            assert_windings(&r.floor);
            assert_windings(&r.ceiling);
        }
        assert_eq!(sm.rooms.len(), 1);
        // wall 0 runs west→east along y=0: left = north (−y), so the "right" group faces into the room (+Z)
        let w0 = &sm.walls[0].mesh;
        let g = w0.groups.iter().find(|g| g.slot == "right").unwrap();
        let i = w0.indices[g.start as usize] as usize;
        assert!(w0.normals[i * 3 + 2] > 0.9);
    }
}
