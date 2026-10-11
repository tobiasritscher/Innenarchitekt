//! Parametric apartment model. All lengths are integer millimetres.
//!
//! Plan coordinates: x to the right, y downwards (like the floor plan image).
//! 3D: plan x → X, plan y → Z, height → Y (three.js, metres).
//! Angles in the plan rotate +x towards +y (clockwise on screen).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type Id = Uuid;
/// Either a library material id (e.g. `paint:white`) or a colour `#rrggbb`.
pub type MaterialRef = String;

pub const DEFAULT_CEILING_MM: u32 = 2500;
pub const DEFAULT_WALL_FINISH: &str = "paint:white";
pub const DEFAULT_FLOOR_MATERIAL: &str = "wood:oak";
pub const DEFAULT_CEILING_MATERIAL: &str = "paint:white";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Project {
    pub id: Id,
    pub name: String,
    pub location: Option<GeoLocation>,
    /// Geographic north in the plan = plan "up" (−y) rotated clockwise (on screen) by this angle.
    /// A compass azimuth A (clockwise from north) points along plan direction (sin(N+A), −cos(N+A)).
    pub north_angle_deg: f32,
    pub floors: Vec<Floor>,
    pub furniture: Vec<FurnitureInstance>,
    pub lights: Vec<Light>,
    /// Saved lamp states ("Abend", "Kochen"). Missing in older projects.
    #[serde(default)]
    pub light_scenes: Vec<LightScene>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub version: u64,
}

/// A named set of lamp states. States of lamps deleted later are kept but ignored when applied.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LightScene {
    pub id: Id,
    pub name: String,
    pub states: Vec<LightState>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LightState {
    pub light: Id,
    pub on: bool,
    pub lumen: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct GeoLocation {
    pub lat: f64,
    pub lon: f64,
    pub address: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Floor {
    pub id: Id,
    pub name: String,
    pub ceiling_height_mm: u32,
    pub nodes: Vec<Node>,
    pub walls: Vec<Wall>,
    pub openings: Vec<Opening>,
    pub rooms: Vec<RoomMeta>,
    pub reference_image: Option<RefImage>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Node {
    pub id: Id,
    pub x_mm: i32,
    pub y_mm: i32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum WallKind {
    Exterior,
    Interior,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Wall {
    pub id: Id,
    pub a: Id,
    pub b: Id,
    pub thickness_mm: u32,
    pub kind: WallKind,
    /// Left side when walking from a to b (seen from above).
    pub finish_left: MaterialRef,
    pub finish_right: MaterialRef,
    /// Full-height glass partition/facade (rendered as glazing, lets light through).
    #[serde(default)]
    pub glazed: bool,
    /// Lower than the ceiling (garden wall, railing, half-height partition). None = up to the ceiling.
    #[serde(default)]
    pub height_mm: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Hinge {
    /// Hinge at the opening edge nearer to wall node a.
    Start,
    End,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Swing {
    /// Door leaf opens towards the left side of the wall (a→b).
    Left,
    Right,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "type")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum OpeningKind {
    Door {
        hinge: Hinge,
        swing: Swing,
        /// Glass leaf (all-glass door in a slim frame).
        #[serde(default)]
        glass: bool,
    },
    Window { sill_mm: u32 },
    Passage,
}

impl OpeningKind {
    pub fn sill_mm(&self) -> u32 {
        match self {
            OpeningKind::Window { sill_mm } => *sill_mm,
            _ => 0,
        }
    }
    pub fn walkable(&self) -> bool {
        !matches!(self, OpeningKind::Window { .. })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Opening {
    pub id: Id,
    pub wall: Id,
    pub kind: OpeningKind,
    /// Distance from wall node a to the opening's near edge.
    pub offset_mm: u32,
    pub width_mm: u32,
    pub height_mm: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RoomMeta {
    pub id: Id,
    /// A point inside the room. The room is the closed face containing it.
    pub anchor: [i32; 2],
    pub name: String,
    pub floor_material: MaterialRef,
    pub ceiling_material: MaterialRef,
    /// Open to the sky (patio, terrace): no ceiling is rendered.
    #[serde(default)]
    pub open_ceiling: bool,
}

/// Original floor plan image placed in plan coordinates.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RefImage {
    /// URL path served by the backend (e.g. `/api/files/<key>`).
    pub url: String,
    pub width_px: u32,
    pub height_px: u32,
    /// Plan position (mm) of the image's top-left pixel.
    pub origin_mm: [f64; 2],
    pub mm_per_px: f64,
    pub opacity: f32,
    pub visible: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FurnitureInstance {
    pub id: Id,
    pub asset: Id,
    pub name: String,
    /// x, y = footprint centre in plan; z = elevation of the bottom above floor.
    pub pos_mm: [i32; 3],
    /// Plan rotation (deg). Local +y is the front, local −y the back.
    pub rotation_deg: f32,
    /// width (local x), depth (local y), height. The mesh is scaled to these.
    pub dims_mm: [u32; 3],
    /// Optional colour/material override (used by primitives).
    pub material: Option<MaterialRef>,
    /// Podium: people walk on it and furniture stands on it (steps along the front, see `primitives::podium_steps`).
    #[serde(default)]
    pub walkable: bool,
    /// May overlap other furniture without being flagged (e.g. chairs pushed under a table). Walls still count.
    #[serde(default)]
    pub overlap_ok: bool,
    /// Occupied parts of the box as rects [x0, y0, x1, y1], normalised to it (0..1 along local x/y; from the
    /// model, e.g. an L-shaped wardrobe). Empty = the whole box. Used for overlaps and walking.
    #[serde(default)]
    pub footprint: Vec<[f32; 4]>,
    /// Never blocks walking (you can walk through it in 3D).
    #[serde(default)]
    pub passable: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "type")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum LightKind {
    Point,
    Spot {
        angle_deg: f32,
        /// 0 = straight down, 90 = horizontal, 180 = straight up.
        #[serde(default)]
        tilt_deg: f32,
        /// Plan direction it leans towards (deg, like furniture rotation: 0 = +x, turning towards +y).
        /// Relative to the furniture if attached.
        #[serde(default)]
        heading_deg: f32,
    },
    Area { w_mm: u32, h_mm: u32 },
    /// The furniture it is attached to glows (lamp shade, LED strip). `pos_mm` is ignored. Requires `attached_to`.
    Surface { face: GlowFace },
}

/// Which part of a glowing object emits light.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum GlowFace {
    /// Whole object (lamp shade): light in all directions.
    All,
    Down,
    Up,
    /// Local +y side.
    Front,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Light {
    pub id: Id,
    pub name: String,
    pub kind: LightKind,
    /// World position in plan coordinates (z = height above floor), or, if
    /// `attached_to` is set, offset in the furniture's local frame (from its footprint centre bottom).
    pub pos_mm: [i32; 3],
    pub lumen: u32,
    pub color_temp_k: u16,
    pub on: bool,
    pub attached_to: Option<Id>,
}

impl Project {
    pub fn new(id: Id, name: impl Into<String>) -> Self {
        Project {
            id,
            name: name.into(),
            location: None,
            north_angle_deg: 0.0,
            floors: vec![Floor::new(Uuid::new_v4(), "EG")],
            furniture: vec![],
            lights: vec![],
            light_scenes: vec![],
            version: 0,
        }
    }

    pub fn floor(&self, id: Id) -> Option<&Floor> {
        self.floors.iter().find(|f| f.id == id)
    }
    pub fn floor_mut(&mut self, id: Id) -> Option<&mut Floor> {
        self.floors.iter_mut().find(|f| f.id == id)
    }
}

impl Floor {
    pub fn new(id: Id, name: impl Into<String>) -> Self {
        Floor {
            id,
            name: name.into(),
            ceiling_height_mm: DEFAULT_CEILING_MM,
            nodes: vec![],
            walls: vec![],
            openings: vec![],
            rooms: vec![],
            reference_image: None,
        }
    }
    pub fn node(&self, id: Id) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn wall(&self, id: Id) -> Option<&Wall> {
        self.walls.iter().find(|w| w.id == id)
    }
    pub fn opening(&self, id: Id) -> Option<&Opening> {
        self.openings.iter().find(|o| o.id == id)
    }
    pub fn walls_at(&self, node: Id) -> impl Iterator<Item = &Wall> {
        self.walls.iter().filter(move |w| w.a == node || w.b == node)
    }
    pub fn openings_on(&self, wall: Id) -> impl Iterator<Item = &Opening> {
        self.openings.iter().filter(move |o| o.wall == wall)
    }
    /// Wall end points as f64 (a, b).
    pub fn wall_points(&self, w: &Wall) -> Option<([f64; 2], [f64; 2])> {
        let a = self.node(w.a)?;
        let b = self.node(w.b)?;
        Some(([a.x_mm as f64, a.y_mm as f64], [b.x_mm as f64, b.y_mm as f64]))
    }
    pub fn wall_length(&self, w: &Wall) -> f64 {
        self.wall_points(w)
            .map(|(a, b)| crate::geom::dist(a, b))
            .unwrap_or(0.0)
    }
}

impl Wall {
    pub fn new(id: Id, a: Id, b: Id, thickness_mm: u32, kind: WallKind) -> Self {
        Wall {
            id,
            a,
            b,
            thickness_mm,
            kind,
            finish_left: DEFAULT_WALL_FINISH.into(),
            finish_right: DEFAULT_WALL_FINISH.into(),
            glazed: false,
            height_mm: None,
        }
    }
    /// Effective height (mm): own height, capped at the ceiling.
    pub fn height(&self, ceiling_mm: u32) -> u32 {
        self.height_mm.map_or(ceiling_mm, |h| h.min(ceiling_mm))
    }
    pub fn other(&self, node: Id) -> Id {
        if self.a == node {
            self.b
        } else {
            self.a
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Projects stored before `glazed` / `open_ceiling` existed must still load.
    #[test]
    fn old_json_loads_with_defaults() {
        let id = Uuid::nil();
        let w: Wall = serde_json::from_value(serde_json::json!({
            "id": id, "a": id, "b": id, "thickness_mm": 100, "kind": "Interior",
            "finish_left": "paint:white", "finish_right": "paint:white"
        }))
        .unwrap();
        assert!(!w.glazed);
        assert_eq!(w.height(2500), 2500);
        let r: RoomMeta = serde_json::from_value(serde_json::json!({
            "id": id, "anchor": [0, 0], "name": "x", "floor_material": "wood:oak", "ceiling_material": "paint:white"
        }))
        .unwrap();
        assert!(!r.open_ceiling);
        let p: Project = serde_json::from_value(serde_json::json!({
            "id": id, "name": "x", "location": null, "north_angle_deg": 0.0,
            "floors": [], "furniture": [], "lights": [], "version": 3
        }))
        .unwrap();
        assert!(p.light_scenes.is_empty());
        let f: FurnitureInstance = serde_json::from_value(serde_json::json!({
            "id": id, "asset": id, "name": "x", "pos_mm": [0, 0, 0], "rotation_deg": 0.0, "dims_mm": [1, 1, 1], "material": null
        }))
        .unwrap();
        assert!(!f.walkable);
    }
}
