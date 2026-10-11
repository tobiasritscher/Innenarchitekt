//! Lamp orientation and placement helpers.

use crate::geom::*;
use crate::model::*;

/// Unit direction (plan, z up) a spot points to; None for other kinds.
pub fn light_dir(l: &Light, p: &Project) -> Option<[f64; 3]> {
    let LightKind::Spot { tilt_deg, heading_deg, .. } = l.kind else { return None };
    let base = l.attached_to.and_then(|a| p.furniture.iter().find(|f| f.id == a)).map(|f| f.rotation_deg as f64).unwrap_or(0.0);
    let (t, h) = ((tilt_deg as f64).to_radians(), (heading_deg as f64 + base).to_radians());
    Some([t.sin() * h.cos(), t.sin() * h.sin(), -t.cos()])
}

/// Distance (mm) between a placed lamp and the surface it was put on.
pub const SURFACE_GAP_MM: f64 = 30.0;

/// `template` placed on a surface point `at` with outward `normal` (plan, z up). Spots face along the
/// normal; on furniture the lamp is attached (local offset, heading relative to the furniture).
pub fn place_light(p: &Project, mut template: Light, at: [f64; 3], normal: [f64; 3], furniture: Option<Id>) -> Light {
    let len = (normal[0].powi(2) + normal[1].powi(2) + normal[2].powi(2)).sqrt().max(1e-9);
    let n = normal.map(|v| v / len);
    let pos = [at[0] + n[0] * SURFACE_GAP_MM, at[1] + n[1] * SURFACE_GAP_MM, at[2] + n[2] * SURFACE_GAP_MM];
    let host = furniture.and_then(|id| p.furniture.iter().find(|f| f.id == id));
    let rot = host.map(|f| f.rotation_deg as f64).unwrap_or(0.0);
    if let LightKind::Spot { tilt_deg, heading_deg, .. } = &mut template.kind {
        *tilt_deg = (-n[2]).clamp(-1.0, 1.0).acos().to_degrees() as f32;
        if n[0].hypot(n[1]) > 1e-6 {
            *heading_deg = (n[1].atan2(n[0]).to_degrees() - rot).rem_euclid(360.0) as f32;
        }
    }
    match host {
        Some(f) => {
            let local = rotate(sub([pos[0], pos[1]], [f.pos_mm[0] as f64, f.pos_mm[1] as f64]), -rot);
            template.pos_mm = [local[0].round() as i32, local[1].round() as i32, (pos[2] - f.pos_mm[2] as f64).round() as i32];
            template.attached_to = Some(f.id);
        }
        None => {
            template.pos_mm = pos.map(|v| v.round() as i32);
            template.attached_to = None;
        }
    }
    template
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn spot(tilt: f32, heading: f32) -> Light {
        Light { id: Uuid::new_v4(), name: "s".into(), kind: LightKind::Spot { angle_deg: 40.0, tilt_deg: tilt, heading_deg: heading }, pos_mm: [0, 0, 2400], lumen: 450, color_temp_k: 3000, on: true, attached_to: None }
    }
    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6)
    }
    fn furniture(name: &str, pos: [i32; 3], rot: f32, dims: [u32; 3]) -> FurnitureInstance {
        FurnitureInstance { id: Uuid::new_v4(), asset: Uuid::new_v4(), name: name.into(), pos_mm: pos, rotation_deg: rot, dims_mm: dims, material: None, walkable: false, overlap_ok: false, footprint: vec![], passable: false }
    }

    #[test]
    fn old_spot_json_loads_pointing_down() {
        let l: Light = serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4(), "name": "x", "kind": {"type": "Spot", "angle_deg": 40.0},
            "pos_mm": [0, 0, 2400], "lumen": 450, "color_temp_k": 3000, "on": true, "attached_to": null
        }))
        .unwrap();
        let p = Project::new(Uuid::new_v4(), "t");
        assert!(close(light_dir(&l, &p).unwrap(), [0.0, 0.0, -1.0]));
    }

    #[test]
    fn tilt_and_heading() {
        let p = Project::new(Uuid::new_v4(), "t");
        assert!(close(light_dir(&spot(180.0, 0.0), &p).unwrap(), [0.0, 0.0, 1.0]));
        assert!(close(light_dir(&spot(90.0, 90.0), &p).unwrap(), [0.0, 1.0, 0.0]));
    }

    #[test]
    fn attached_spot_turns_with_furniture() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = furniture("Regal", [1000, 1000, 0], 90.0, [800, 300, 1800]);
        p.furniture.push(f.clone());
        let mut l = spot(90.0, 0.0);
        l.attached_to = Some(f.id);
        // local +x turned by 90° → plan +y
        assert!(close(light_dir(&l, &p).unwrap(), [0.0, 1.0, 0.0]));
    }

    #[test]
    fn place_on_ceiling_floor_wall() {
        let p = Project::new(Uuid::new_v4(), "t");
        let ceil = place_light(&p, spot(0.0, 0.0), [1000.0, 1000.0, 2500.0], [0.0, 0.0, -1.0], None);
        assert_eq!(ceil.pos_mm, [1000, 1000, 2470]);
        assert!(close(light_dir(&ceil, &p).unwrap(), [0.0, 0.0, -1.0]));
        let floor = place_light(&p, spot(0.0, 0.0), [1000.0, 1000.0, 0.0], [0.0, 0.0, 1.0], None);
        assert!(close(light_dir(&floor, &p).unwrap(), [0.0, 0.0, 1.0]));
        let wall = place_light(&p, spot(0.0, 0.0), [0.0, 1000.0, 1800.0], [1.0, 0.0, 0.0], None);
        assert_eq!(wall.pos_mm, [30, 1000, 1800]);
        assert!(close(light_dir(&wall, &p).unwrap(), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn place_on_rotated_furniture_attaches_locally() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = furniture("Sideboard", [2000, 1000, 0], 90.0, [1600, 450, 800]);
        p.furniture.push(f.clone());
        // top of the sideboard, 300 mm towards plan +y from its centre (= local +x after the 90° turn)
        let l = place_light(&p, spot(0.0, 0.0), [2000.0, 1300.0, 800.0], [0.0, 0.0, 1.0], Some(f.id));
        assert_eq!(l.attached_to, Some(f.id));
        assert_eq!(l.pos_mm, [300, 0, 830]);
        assert!(close(light_dir(&l, &p).unwrap(), [0.0, 0.0, 1.0]));
        // a wall-facing spot on the same furniture keeps its plan direction
        let side = place_light(&p, spot(0.0, 0.0), [2000.0, 1800.0, 400.0], [0.0, 1.0, 0.0], Some(f.id));
        assert!(close(light_dir(&side, &p).unwrap(), [0.0, 1.0, 0.0]));
    }

    #[test]
    fn surface_light_needs_furniture_and_survives_delete_undo() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = furniture("LED", [1000, 1000, 2000], 0.0, [1000, 10, 12]);
        let glow = Light { kind: LightKind::Surface { face: GlowFace::Down }, ..spot(0.0, 0.0) };
        // without attached_to: rejected
        assert!(crate::ops::apply_batch(&mut p, &[crate::ops::Op::PutLight { light: glow.clone() }]).is_err());
        let glow = Light { attached_to: Some(f.id), ..glow };
        crate::ops::apply_batch(&mut p, &[crate::ops::Op::PutFurniture { item: f.clone() }, crate::ops::Op::PutLight { light: glow.clone() }]).unwrap();
        let del = crate::commands::delete_furniture(&p, f.id).unwrap();
        let inverse = crate::ops::apply_batch(&mut p, &del).unwrap();
        assert!(p.lights.is_empty());
        crate::ops::apply_batch(&mut p, &inverse).unwrap();
        assert_eq!(p.lights.len(), 1);
    }
}
