//! Lamp orientation and placement helpers.

#[allow(unused_imports)]
use crate::geom::*;
use crate::model::*;

/// Unit direction (plan, z up) a spot points to; None for other kinds.
pub fn light_dir(l: &Light, p: &Project) -> Option<[f64; 3]> {
    let LightKind::Spot { tilt_deg, heading_deg, .. } = l.kind else { return None };
    let base = l.attached_to.and_then(|a| p.furniture.iter().find(|f| f.id == a)).map(|f| f.rotation_deg as f64).unwrap_or(0.0);
    let (t, h) = ((tilt_deg as f64).to_radians(), (heading_deg as f64 + base).to_radians());
    Some([t.sin() * h.cos(), t.sin() * h.sin(), -t.cos()])
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
