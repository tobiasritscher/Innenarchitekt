//! Parametric placeholder furniture, generated as meshes (metres).
//! Origin: bottom centre. Width along X, depth along Z (front = +Z), height along Y.

use serde::{Deserialize, Serialize};

use crate::mesh::{Mesh, MeshBuilder};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum PrimitiveShape {
    Box,
    Cylinder,
    Plate,
    Sofa,
    Bed,
    Table,
    Chair,
    Shelf,
    Wardrobe,
    /// Raised floor area (walkable) with steps along its front side.
    Podest,
}

impl PrimitiveShape {
    pub const ALL: [PrimitiveShape; 10] = [
        PrimitiveShape::Box,
        PrimitiveShape::Cylinder,
        PrimitiveShape::Plate,
        PrimitiveShape::Sofa,
        PrimitiveShape::Bed,
        PrimitiveShape::Table,
        PrimitiveShape::Chair,
        PrimitiveShape::Shelf,
        PrimitiveShape::Wardrobe,
        PrimitiveShape::Podest,
    ];
    pub fn label_de(&self) -> &'static str {
        match self {
            PrimitiveShape::Box => "Box",
            PrimitiveShape::Cylinder => "Zylinder",
            PrimitiveShape::Plate => "Platte",
            PrimitiveShape::Sofa => "Sofa",
            PrimitiveShape::Bed => "Bett",
            PrimitiveShape::Table => "Tisch",
            PrimitiveShape::Chair => "Stuhl",
            PrimitiveShape::Shelf => "Regal",
            PrimitiveShape::Wardrobe => "Schrank",
            PrimitiveShape::Podest => "Podest mit Stufen",
        }
    }
    /// Default dimensions (w, d, h) in mm.
    pub fn default_dims(&self) -> [u32; 3] {
        match self {
            PrimitiveShape::Box => [500, 500, 500],
            PrimitiveShape::Cylinder => [400, 400, 600],
            PrimitiveShape::Plate => [1200, 600, 30],
            PrimitiveShape::Sofa => [2200, 950, 850],
            PrimitiveShape::Bed => [1600, 2050, 900],
            PrimitiveShape::Table => [1600, 900, 750],
            PrimitiveShape::Chair => [450, 520, 850],
            PrimitiveShape::Shelf => [800, 350, 1800],
            PrimitiveShape::Wardrobe => [1500, 600, 2200],
            PrimitiveShape::Podest => [3000, 2500, 340],
        }
    }
    /// Walkable (people and furniture stand on it) instead of an obstacle.
    pub fn walkable(&self) -> bool {
        matches!(self, PrimitiveShape::Podest)
    }
}

fn add_box(b: &mut MeshBuilder, slot: &str, c: [f32; 3], s: [f32; 3]) {
    let (hx, hy, hz) = (s[0] / 2.0, s[1] / 2.0, s[2] / 2.0);
    let (x0, x1, y0, y1, z0, z1) = (c[0] - hx, c[0] + hx, c[1] - hy, c[1] + hy, c[2] - hz, c[2] + hz);
    let faces: [([[f32; 3]; 4], [f32; 3], [f32; 2]); 6] = [
        ([[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]], [1.0, 0.0, 0.0], [s[2], s[1]]),
        ([[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]], [-1.0, 0.0, 0.0], [s[2], s[1]]),
        ([[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]], [0.0, 1.0, 0.0], [s[0], s[2]]),
        ([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, -1.0, 0.0], [s[0], s[2]]),
        ([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], [0.0, 0.0, 1.0], [s[0], s[1]]),
        ([[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]], [0.0, 0.0, -1.0], [s[0], s[1]]),
    ];
    for (pts, n, uvs) in faces {
        b.quad_fan(slot, &pts, n, &[[0.0, 0.0], [uvs[0], 0.0], [uvs[0], uvs[1]], [0.0, uvs[1]]]);
    }
}

fn add_cylinder(b: &mut MeshBuilder, slot: &str, c: [f32; 3], rx: f32, rz: f32, h: f32, seg: usize) {
    let y0 = c[1] - h / 2.0;
    let y1 = c[1] + h / 2.0;
    let pt = |i: usize| {
        let a = i as f32 / seg as f32 * std::f32::consts::TAU;
        (a.cos(), a.sin())
    };
    for i in 0..seg {
        let (c0, s0) = pt(i);
        let (c1, s1) = pt(i + 1);
        let p0 = [c[0] + rx * c0, c[2] + rz * s0];
        let p1 = [c[0] + rx * c1, c[2] + rz * s1];
        let nm = [(c0 + c1) / 2.0, 0.0, (s0 + s1) / 2.0];
        let u0 = i as f32 / seg as f32;
        let u1 = (i + 1) as f32 / seg as f32;
        b.quad_fan(
            slot,
            &[[p1[0], y0, p1[1]], [p0[0], y0, p0[1]], [p0[0], y1, p0[1]], [p1[0], y1, p1[1]]],
            nm,
            &[[u1, 0.0], [u0, 0.0], [u0, h], [u1, h]],
        );
        b.quad_fan(slot, &[[c[0], y1, c[2]], [p1[0], y1, p1[1]], [p0[0], y1, p0[1]]], [0.0, 1.0, 0.0], &[[0.5, 0.5], [0.5 + c1 / 2.0, 0.5 + s1 / 2.0], [0.5 + c0 / 2.0, 0.5 + s0 / 2.0]]);
        b.quad_fan(slot, &[[c[0], y0, c[2]], [p0[0], y0, p0[1]], [p1[0], y0, p1[1]]], [0.0, -1.0, 0.0], &[[0.5, 0.5], [0.5 + c0 / 2.0, 0.5 + s0 / 2.0], [0.5 + c1 / 2.0, 0.5 + s1 / 2.0]]);
    }
}

/// Steps of a podium: (number of rises, tread depth mm). Rise ≈ 170 mm; the front `rises − 1` treads
/// lie inside the footprint, the rest is the platform at full height.
pub fn podium_steps(dims_mm: [u32; 3]) -> (u32, f64) {
    let h = dims_mm[2] as f64;
    let d = dims_mm[1] as f64;
    let n = ((h / 170.0).round() as u32).max(1);
    let tread = if n > 1 { 280.0f64.min(d / 2.0 / (n - 1) as f64) } else { 0.0 };
    (n, tread)
}

/// Surface height (mm) of a podium at local depth `y` (front = +d/2).
pub fn podium_height_at(dims_mm: [u32; 3], y_local: f64) -> f64 {
    let (n, tread) = podium_steps(dims_mm);
    let h = dims_mm[2] as f64;
    let s = dims_mm[1] as f64 / 2.0 - y_local; // distance from the front edge
    if n > 1 && s < (n - 1) as f64 * tread {
        let j = (s.max(0.0) / tread).floor() as u32 + 1;
        h * j as f64 / n as f64
    } else {
        h
    }
}

pub fn primitive_mesh(shape: PrimitiveShape, dims_mm: [u32; 3]) -> Mesh {
    let w = dims_mm[0] as f32 / 1000.0;
    let d = dims_mm[1] as f32 / 1000.0;
    let h = dims_mm[2] as f32 / 1000.0;
    let mut b = MeshBuilder::default();
    match shape {
        PrimitiveShape::Box | PrimitiveShape::Plate => add_box(&mut b, "main", [0.0, h / 2.0, 0.0], [w, h, d]),
        PrimitiveShape::Cylinder => add_cylinder(&mut b, "main", [0.0, h / 2.0, 0.0], w / 2.0, d / 2.0, h, 32),
        PrimitiveShape::Sofa => {
            let seat_h = (h * 0.5).min(0.45);
            let arm = (w * 0.1).min(0.2);
            let back = (d * 0.22).min(0.25);
            add_box(&mut b, "accent", [0.0, 0.06, 0.0], [w - 0.04, 0.12, d - 0.04]);
            add_box(&mut b, "main", [0.0, 0.12 + (seat_h - 0.12) / 2.0, back / 2.0], [w - 2.0 * arm, seat_h - 0.12, d - back]);
            add_box(&mut b, "main", [0.0, 0.12 + (h - 0.12) / 2.0, -d / 2.0 + back / 2.0], [w, h - 0.12, back]);
            let arm_h = (seat_h + 0.2).min(h);
            for s in [-1.0, 1.0] {
                add_box(&mut b, "main", [s * (w / 2.0 - arm / 2.0), 0.12 + (arm_h - 0.12) / 2.0, back / 2.0], [arm, arm_h - 0.12, d - back]);
            }
        }
        PrimitiveShape::Bed => {
            let frame_h = (h * 0.3).min(0.3);
            let mattress = 0.2f32.min(h - frame_h).max(0.0);
            let head = 0.08;
            add_box(&mut b, "accent", [0.0, frame_h / 2.0, head / 2.0], [w, frame_h, d - head]);
            add_box(&mut b, "main", [0.0, frame_h + mattress / 2.0, head / 2.0 + 0.01], [w - 0.04, mattress, d - head - 0.04]);
            add_box(&mut b, "accent", [0.0, h / 2.0, -d / 2.0 + head / 2.0], [w, h, head]);
            // pillows
            let pw = if w > 1.2 { w / 2.0 - 0.1 } else { w - 0.15 };
            let n = if w > 1.2 { 2 } else { 1 };
            for i in 0..n {
                let x = if n == 2 { (i as f32 - 0.5) * (pw + 0.05) } else { 0.0 };
                add_box(&mut b, "main", [x, frame_h + mattress + 0.06, -d / 2.0 + head + 0.25], [pw, 0.12, 0.4]);
            }
        }
        PrimitiveShape::Table => {
            let top = 0.03;
            let leg = 0.05;
            add_box(&mut b, "main", [0.0, h - top / 2.0, 0.0], [w, top, d]);
            for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                add_box(&mut b, "accent", [sx * (w / 2.0 - leg), (h - top) / 2.0, sz * (d / 2.0 - leg)], [leg, h - top, leg]);
            }
        }
        PrimitiveShape::Chair => {
            let seat = (h * 0.53).min(0.47);
            let leg = 0.035;
            add_box(&mut b, "main", [0.0, seat - 0.02, 0.0], [w, 0.04, d]);
            for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                add_box(&mut b, "accent", [sx * (w / 2.0 - leg), (seat - 0.04) / 2.0, sz * (d / 2.0 - leg)], [leg, seat - 0.04, leg]);
            }
            add_box(&mut b, "main", [0.0, seat + (h - seat) / 2.0, -d / 2.0 + 0.02], [w, h - seat, 0.04]);
        }
        PrimitiveShape::Shelf => {
            let t = 0.018;
            for s in [-1.0, 1.0] {
                add_box(&mut b, "main", [s * (w / 2.0 - t / 2.0), h / 2.0, 0.0], [t, h, d]);
            }
            add_box(&mut b, "accent", [0.0, h / 2.0, -d / 2.0 + 0.004], [w - 2.0 * t, h, 0.008]);
            let n = ((h / 0.35).round() as i32).max(2);
            for i in 0..=n {
                let y = (t / 2.0 + (h - t) * i as f32 / n as f32).min(h - t / 2.0);
                add_box(&mut b, "main", [0.0, y, 0.0], [w - 2.0 * t, t, d]);
            }
        }
        PrimitiveShape::Podest => {
            let (n, tread) = podium_steps(dims_mm);
            let t = tread as f32 / 1000.0;
            let front = d / 2.0 - (n - 1) as f32 * t;
            add_box(&mut b, "main", [0.0, h / 2.0, (-d / 2.0 + front) / 2.0], [w, h, front + d / 2.0]);
            for j in 1..n {
                // step j from the front: z range [d/2 − j·t, d/2 − (j−1)·t]
                let sh = h * j as f32 / n as f32;
                let z1 = d / 2.0 - (j - 1) as f32 * t;
                add_box(&mut b, "main", [0.0, sh / 2.0, z1 - t / 2.0], [w, sh, t]);
                // dark nosing line on each step edge
                add_box(&mut b, "accent", [0.0, sh - 0.004, z1 - 0.004], [w, 0.008, 0.008]);
            }
        }
        PrimitiveShape::Wardrobe => {
            add_box(&mut b, "main", [0.0, h / 2.0, -0.01], [w, h, d - 0.02]);
            let doors = ((w / 0.5).round() as i32).max(1);
            let dw = w / doors as f32;
            for i in 0..doors {
                let x = -w / 2.0 + dw * (i as f32 + 0.5);
                add_box(&mut b, "main", [x, h / 2.0, d / 2.0 - 0.01], [dw - 0.004, h - 0.01, 0.018]);
                let hx = if i % 2 == 0 { x + dw / 2.0 - 0.05 } else { x - dw / 2.0 + 0.05 };
                add_box(&mut b, "accent", [hx, h * 0.5, d / 2.0 + 0.01], [0.015, 0.2, 0.02]);
            }
        }
    }
    b.build()
}
