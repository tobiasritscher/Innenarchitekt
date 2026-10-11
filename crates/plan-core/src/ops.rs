//! Operations: every change to a project is an `Op` with an inverse.
//! Undo/redo, autosave and AI proposals all use this one mechanism.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::*;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Op {
    PutNode { floor: Id, node: Node },
    DeleteNode { floor: Id, id: Id },
    PutWall { floor: Id, wall: Wall },
    DeleteWall { floor: Id, id: Id },
    PutOpening { floor: Id, opening: Opening },
    DeleteOpening { floor: Id, id: Id },
    PutRoom { floor: Id, room: RoomMeta },
    DeleteRoom { floor: Id, id: Id },
    PutFurniture { item: FurnitureInstance },
    DeleteFurniture { id: Id },
    PutLight { light: Light },
    DeleteLight { id: Id },
    PutLightScene { scene: LightScene },
    DeleteLightScene { id: Id },
    SetProject {
        name: String,
        location: Option<GeoLocation>,
        north_angle_deg: f32,
    },
    SetFloor {
        floor: Id,
        name: String,
        ceiling_height_mm: u32,
        reference_image: Option<RefImage>,
    },
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum OpError {
    #[error("Stockwerk {0} nicht gefunden")]
    FloorNotFound(Id),
    #[error("Element {0} nicht gefunden")]
    NotFound(Id),
    #[error("Knoten {0} wird noch von Wänden verwendet")]
    NodeInUse(Id),
    #[error("Wand {0} hat noch Öffnungen")]
    WallHasOpenings(Id),
    #[error("Ungültig: {0}")]
    Invalid(String),
}

fn put<T: Clone, F: Fn(&T) -> Id>(list: &mut Vec<T>, item: T, id_of: F) -> Option<T> {
    let id = id_of(&item);
    if let Some(slot) = list.iter_mut().find(|x| id_of(x) == id) {
        Some(std::mem::replace(slot, item))
    } else {
        list.push(item);
        None
    }
}

fn delete<T, F: Fn(&T) -> Id>(list: &mut Vec<T>, id: Id, id_of: F) -> Option<T> {
    let idx = list.iter().position(|x| id_of(x) == id)?;
    Some(list.remove(idx))
}

fn floor_mut(p: &mut Project, id: Id) -> Result<&mut Floor, OpError> {
    p.floor_mut(id).ok_or(OpError::FloorNotFound(id))
}

/// Apply one op, returning its inverse.
pub fn apply(p: &mut Project, op: &Op) -> Result<Op, OpError> {
    Ok(match op.clone() {
        Op::PutNode { floor, node } => {
            let f = floor_mut(p, floor)?;
            match put(&mut f.nodes, node, |n| n.id) {
                Some(old) => Op::PutNode { floor, node: old },
                None => Op::DeleteNode { floor, id: node.id },
            }
        }
        Op::DeleteNode { floor, id } => {
            let f = floor_mut(p, floor)?;
            if f.walls_at(id).next().is_some() {
                return Err(OpError::NodeInUse(id));
            }
            let old = delete(&mut f.nodes, id, |n| n.id).ok_or(OpError::NotFound(id))?;
            Op::PutNode { floor, node: old }
        }
        Op::PutWall { floor, wall } => {
            let f = floor_mut(p, floor)?;
            if wall.a == wall.b {
                return Err(OpError::Invalid("Wand mit identischen Endknoten".into()));
            }
            if f.node(wall.a).is_none() {
                return Err(OpError::NotFound(wall.a));
            }
            if f.node(wall.b).is_none() {
                return Err(OpError::NotFound(wall.b));
            }
            if wall.thickness_mm == 0 {
                return Err(OpError::Invalid("Wandstärke 0".into()));
            }
            let id = wall.id;
            match put(&mut f.walls, wall, |w| w.id) {
                Some(old) => Op::PutWall { floor, wall: old },
                None => Op::DeleteWall { floor, id },
            }
        }
        Op::DeleteWall { floor, id } => {
            let f = floor_mut(p, floor)?;
            if f.openings_on(id).next().is_some() {
                return Err(OpError::WallHasOpenings(id));
            }
            let old = delete(&mut f.walls, id, |w| w.id).ok_or(OpError::NotFound(id))?;
            Op::PutWall { floor, wall: old }
        }
        Op::PutOpening { floor, opening } => {
            let f = floor_mut(p, floor)?;
            if f.wall(opening.wall).is_none() {
                return Err(OpError::NotFound(opening.wall));
            }
            if opening.width_mm == 0 || opening.height_mm == 0 {
                return Err(OpError::Invalid("Öffnung ohne Breite/Höhe".into()));
            }
            let id = opening.id;
            match put(&mut f.openings, opening, |o| o.id) {
                Some(old) => Op::PutOpening { floor, opening: old },
                None => Op::DeleteOpening { floor, id },
            }
        }
        Op::DeleteOpening { floor, id } => {
            let f = floor_mut(p, floor)?;
            let old = delete(&mut f.openings, id, |o| o.id).ok_or(OpError::NotFound(id))?;
            Op::PutOpening { floor, opening: old }
        }
        Op::PutRoom { floor, room } => {
            let f = floor_mut(p, floor)?;
            let id = room.id;
            match put(&mut f.rooms, room, |r| r.id) {
                Some(old) => Op::PutRoom { floor, room: old },
                None => Op::DeleteRoom { floor, id },
            }
        }
        Op::DeleteRoom { floor, id } => {
            let f = floor_mut(p, floor)?;
            let old = delete(&mut f.rooms, id, |r| r.id).ok_or(OpError::NotFound(id))?;
            Op::PutRoom { floor, room: old }
        }
        Op::PutFurniture { item } => {
            if item.dims_mm.iter().any(|d| *d == 0) {
                return Err(OpError::Invalid("Möbel ohne Masse".into()));
            }
            let id = item.id;
            match put(&mut p.furniture, item, |x| x.id) {
                Some(old) => Op::PutFurniture { item: old },
                None => Op::DeleteFurniture { id },
            }
        }
        Op::DeleteFurniture { id } => {
            let old = delete(&mut p.furniture, id, |x| x.id).ok_or(OpError::NotFound(id))?;
            Op::PutFurniture { item: old }
        }
        Op::PutLight { light } => {
            if matches!(light.kind, LightKind::Surface { .. }) && light.attached_to.is_none() {
                return Err(OpError::Invalid("Leuchtfläche ohne Objekt".into()));
            }
            if let Some(att) = light.attached_to {
                if !p.furniture.iter().any(|f| f.id == att) {
                    return Err(OpError::NotFound(att));
                }
            }
            let id = light.id;
            match put(&mut p.lights, light, |x| x.id) {
                Some(old) => Op::PutLight { light: old },
                None => Op::DeleteLight { id },
            }
        }
        Op::DeleteLight { id } => {
            let old = delete(&mut p.lights, id, |x| x.id).ok_or(OpError::NotFound(id))?;
            Op::PutLight { light: old }
        }
        Op::PutLightScene { scene } => {
            if scene.name.trim().is_empty() {
                return Err(OpError::Invalid("Lichtszene ohne Namen".into()));
            }
            let id = scene.id;
            match put(&mut p.light_scenes, scene, |x| x.id) {
                Some(old) => Op::PutLightScene { scene: old },
                None => Op::DeleteLightScene { id },
            }
        }
        Op::DeleteLightScene { id } => {
            let old = delete(&mut p.light_scenes, id, |x| x.id).ok_or(OpError::NotFound(id))?;
            Op::PutLightScene { scene: old }
        }
        Op::SetProject {
            name,
            location,
            north_angle_deg,
        } => {
            let inv = Op::SetProject {
                name: std::mem::replace(&mut p.name, name),
                location: std::mem::replace(&mut p.location, location),
                north_angle_deg: std::mem::replace(&mut p.north_angle_deg, north_angle_deg),
            };
            inv
        }
        Op::SetFloor {
            floor,
            name,
            ceiling_height_mm,
            reference_image,
        } => {
            let f = floor_mut(p, floor)?;
            if ceiling_height_mm < 1000 {
                return Err(OpError::Invalid("Raumhöhe unter 1 m".into()));
            }
            Op::SetFloor {
                floor,
                name: std::mem::replace(&mut f.name, name),
                ceiling_height_mm: std::mem::replace(&mut f.ceiling_height_mm, ceiling_height_mm),
                reference_image: std::mem::replace(&mut f.reference_image, reference_image),
            }
        }
    })
}

/// Apply a batch atomically. Returns the inverse batch (already in undo order).
pub fn apply_batch(p: &mut Project, ops: &[Op]) -> Result<Vec<Op>, (usize, OpError)> {
    let backup = p.clone();
    let mut inverse = Vec::with_capacity(ops.len());
    for (i, op) in ops.iter().enumerate() {
        match apply(p, op) {
            Ok(inv) => inverse.push(inv),
            Err(e) => {
                *p = backup;
                return Err((i, e));
            }
        }
    }
    inverse.reverse();
    Ok(inverse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn put_and_undo() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = p.floors[0].id;
        let n1 = Node { id: Uuid::new_v4(), x_mm: 0, y_mm: 0 };
        let n2 = Node { id: Uuid::new_v4(), x_mm: 1000, y_mm: 0 };
        let w = Wall::new(Uuid::new_v4(), n1.id, n2.id, 100, WallKind::Interior);
        let before = p.clone();
        let ops = vec![
            Op::PutNode { floor: f, node: n1 },
            Op::PutNode { floor: f, node: n2 },
            Op::PutWall { floor: f, wall: w },
        ];
        let inv = apply_batch(&mut p, &ops).unwrap();
        assert_eq!(p.floors[0].walls.len(), 1);
        apply_batch(&mut p, &inv).unwrap();
        assert_eq!(p, before);
    }

    #[test]
    fn light_scene_roundtrip() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let before = p.clone();
        let sc = LightScene { id: Uuid::new_v4(), name: "Abend".into(), states: vec![LightState { light: Uuid::new_v4(), on: true, lumen: 800 }] };
        let inv = apply_batch(&mut p, &[Op::PutLightScene { scene: sc.clone() }]).unwrap();
        assert_eq!(p.light_scenes, vec![sc.clone()]);
        let renamed = LightScene { name: "Nacht".into(), ..sc.clone() };
        let inv2 = apply_batch(&mut p, &[Op::PutLightScene { scene: renamed }]).unwrap();
        apply_batch(&mut p, &inv2).unwrap();
        assert_eq!(p.light_scenes[0].name, "Abend");
        let inv3 = apply_batch(&mut p, &[Op::DeleteLightScene { id: sc.id }]).unwrap();
        assert!(p.light_scenes.is_empty());
        apply_batch(&mut p, &inv3).unwrap();
        apply_batch(&mut p, &inv).unwrap();
        assert_eq!(p, before);
        let empty = LightScene { name: " ".into(), ..sc };
        assert!(apply_batch(&mut p, &[Op::PutLightScene { scene: empty }]).is_err());
    }

    #[test]
    fn batch_is_atomic() {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = p.floors[0].id;
        let n1 = Node { id: Uuid::new_v4(), x_mm: 0, y_mm: 0 };
        let before = p.clone();
        let bad = Wall::new(Uuid::new_v4(), n1.id, Uuid::new_v4(), 100, WallKind::Interior);
        let r = apply_batch(
            &mut p,
            &[Op::PutNode { floor: f, node: n1 }, Op::PutWall { floor: f, wall: bad }],
        );
        assert!(r.is_err());
        assert_eq!(p, before);
    }
}
