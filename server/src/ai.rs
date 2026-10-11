//! KI-Assistent (Phase 2): Befehle in natürlicher Sprache, Rückfragen zum Modell, Materialien aus Fotos.
//!
//! The model sees a compact JSON of the plan (short ids like `w3`, `r2`) and answers with exactly one
//! forced tool call: an answer text plus a list of high-level actions. The backend translates the
//! actions into plan-core ops on a copy of the project and returns them as a proposal — nothing is
//! saved here; the client previews the ops and the user accepts or discards them.

use std::collections::HashMap;

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use plan_core::assets::Asset;
use plan_core::primitives::PrimitiveShape;
use plan_core::commands::{self, ChainPoint};
use plan_core::furniture::{clearances, snap_to_wall};
use plan_core::geom::*;
use plan_core::rooms::{derive_rooms, DerivedRoom};
use plan_core::*;
use serde_json::{json, Value};
use sqlx::types::Json as SqlJson;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_PHOTOS: usize = 6;

// ------------------------------------------------------------------ short ids

#[derive(Default)]
pub(crate) struct Ids {
    pub short: HashMap<Uuid, String>,
    pub long: HashMap<String, Uuid>,
}

impl Ids {
    pub fn add(&mut self, prefix: &str, id: Uuid) -> String {
        if let Some(s) = self.short.get(&id) {
            return s.clone();
        }
        let s = format!("{prefix}{}", self.long.keys().filter(|k| k.starts_with(prefix) && k[prefix.len()..].chars().all(|c| c.is_ascii_digit())).count() + 1);
        self.short.insert(id, s.clone());
        self.long.insert(s.clone(), id);
        s
    }
    pub fn s(&self, id: Uuid) -> String {
        self.short.get(&id).cloned().unwrap_or_else(|| id.to_string())
    }
    pub fn get(&self, s: &str) -> Option<Uuid> {
        self.long.get(s.trim()).copied().or_else(|| Uuid::parse_str(s.trim()).ok())
    }
}

/// Everything the translator needs besides the project.
pub(crate) struct Ctx {
    pub ids: Ids,
    pub rooms: Vec<DerivedRoom>,
    pub room_ids: Vec<String>,
    pub assets: Vec<Asset>,
    pub materials: Vec<(String, String)>,
    /// Material id → display colour "#rrggbb" (for elevation images).
    pub colors: HashMap<String, String>,
    room_seq: usize,
}

impl Ctx {
    pub fn room(&self, s: &str) -> Option<&DerivedRoom> {
        let s = s.trim();
        if let Some(i) = self.room_ids.iter().position(|r| r == s) {
            return self.rooms.get(i);
        }
        // by name as a fallback
        self.rooms.iter().find(|r| r.meta.is_some() && r.key == s)
    }
}

fn p2(n: &Node) -> P {
    [n.x_mm as f64, n.y_mm as f64]
}

/// Room (index) on the left (+1) or right (−1) side of a wall.
pub(crate) fn side_room(f: &Floor, rooms: &[DerivedRoom], w: &Wall, side: f64) -> Option<usize> {
    let (a, b) = f.wall_points(w)?;
    if dist(a, b) < 1.0 {
        return None;
    }
    let n = left_normal(norm(sub(b, a)));
    let probe = add(mul(add(a, b), 0.5), mul(n, side * (w.thickness_mm as f64 / 2.0 + 80.0)));
    rooms.iter().position(|r| point_in_polygon(probe, &r.polygon))
}

pub(crate) fn room_name(f: &Floor, r: &DerivedRoom) -> Option<String> {
    r.meta.and_then(|m| f.rooms.iter().find(|x| x.id == m)).map(|m| m.name.clone())
}

pub(crate) fn bbox(poly: &[P]) -> [i64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in poly {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b.map(|v| v.round() as i64)
}

pub(crate) fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

// ------------------------------------------------------------------ state for the model

pub(crate) fn build_ctx(p: &Project, assets: Vec<Asset>, materials: Vec<(String, String)>) -> Ctx {
    let mut ids = Ids::default();
    let f = &p.floors[0];
    for n in &f.nodes {
        ids.add("n", n.id);
    }
    for w in &f.walls {
        ids.add("w", w.id);
    }
    for o in &f.openings {
        ids.add("o", o.id);
    }
    for it in &p.furniture {
        ids.add("f", it.id);
    }
    for l in &p.lights {
        ids.add("l", l.id);
    }
    for a in &assets {
        ids.add("a", a.id);
    }
    let rooms = derive_rooms(f);
    let room_ids = (1..=rooms.len()).map(|i| format!("r{i}")).collect();
    let room_seq = rooms.len();
    Ctx { ids, rooms, room_ids, assets, materials, colors: HashMap::new(), room_seq }
}

impl Ctx {
    /// After changes: short ids for new elements; rooms re-derived, keeping the short id of rooms whose key survived.
    pub fn refresh(&mut self, p: &Project) {
        let f = &p.floors[0];
        for n in &f.nodes {
            self.ids.add("n", n.id);
        }
        for w in &f.walls {
            self.ids.add("w", w.id);
        }
        for o in &f.openings {
            self.ids.add("o", o.id);
        }
        for it in &p.furniture {
            self.ids.add("f", it.id);
        }
        for l in &p.lights {
            self.ids.add("l", l.id);
        }
        let rooms = derive_rooms(f);
        let mut ids = Vec::with_capacity(rooms.len());
        for r in &rooms {
            match self.rooms.iter().position(|o| o.key == r.key || (o.meta.is_some() && o.meta == r.meta)) {
                Some(i) => ids.push(self.room_ids[i].clone()),
                None => {
                    self.room_seq += 1;
                    ids.push(format!("r{}", self.room_seq));
                }
            }
        }
        self.rooms = rooms;
        self.room_ids = ids;
    }
}

pub(crate) fn state_json(p: &Project, cx: &Ctx) -> Value {
    let f = &p.floors[0];
    let ids = &cx.ids;
    let n = p.north_angle_deg as f64;
    let north = [n.to_radians().sin(), -n.to_radians().cos()];
    let walls: Vec<Value> = f
        .walls
        .iter()
        .filter_map(|w| {
            let (a, b) = f.wall_points(w)?;
            let left = side_room(f, &cx.rooms, w, 1.0).map(|i| cx.room_ids[i].clone());
            let right = side_room(f, &cx.rooms, w, -1.0).map(|i| cx.room_ids[i].clone());
            let ln = left_normal(norm(sub(b, a)));
            Some(json!({
                "id": ids.s(w.id), "a": ids.s(w.a), "b": ids.s(w.b),
                "length": dist(a, b).round(), "thickness": w.thickness_mm,
                "kind": if w.kind == WallKind::Exterior { "exterior" } else { "interior" },
                "glazed": w.glazed, "height": w.height(f.ceiling_height_mm),
                "left_normal": [r1(ln[0]), r1(ln[1])],
                "left_room": left, "right_room": right,
                "finish_left": w.finish_left, "finish_right": w.finish_right,
            }))
        })
        .collect();
    let openings: Vec<Value> = f
        .openings
        .iter()
        .map(|o| {
            let mut v = json!({
                "id": ids.s(o.id), "wall": ids.s(o.wall), "offset_from_a": o.offset_mm,
                "width": o.width_mm, "height": o.height_mm,
            });
            match o.kind {
                OpeningKind::Door { hinge, swing, .. } => {
                    v["type"] = json!("door");
                    v["hinge"] = json!(if hinge == Hinge::Start { "a" } else { "b" });
                    v["swing"] = json!(if swing == Swing::Left { "left" } else { "right" });
                }
                OpeningKind::Window { sill_mm } => {
                    v["type"] = json!("window");
                    v["sill"] = json!(sill_mm);
                }
                OpeningKind::Passage => v["type"] = json!("passage"),
            }
            v
        })
        .collect();
    let rooms: Vec<Value> = cx
        .rooms
        .iter()
        .zip(&cx.room_ids)
        .map(|(r, rid)| {
            let meta = r.meta.and_then(|m| f.rooms.iter().find(|x| x.id == m));
            json!({
                "id": rid, "name": meta.map(|m| m.name.clone()),
                "area_m2": r1(r.area_m2), "bbox": bbox(&r.inner),
                "walls": r.walls.iter().map(|w| ids.s(*w)).collect::<Vec<_>>(),
                "floor_material": meta.map(|m| m.floor_material.clone()),
                "ceiling_material": meta.map(|m| m.ceiling_material.clone()),
                "open_ceiling": meta.map(|m| m.open_ceiling).unwrap_or(false),
            })
        })
        .collect();
    let furniture: Vec<Value> = p
        .furniture
        .iter()
        .map(|it| {
            let c = clearances(f, it);
            json!({
                "id": ids.s(it.id), "name": it.name, "asset": ids.s(it.asset),
                "x": it.pos_mm[0], "y": it.pos_mm[1], "z": it.pos_mm[2], "rotation_deg": r1(it.rotation_deg as f64),
                "width": it.dims_mm[0], "depth": it.dims_mm[1], "height": it.dims_mm[2],
                "material": it.material,
                "clearance_to_walls": {"left": c.left, "right": c.right, "front": c.front, "back": c.back},
            })
        })
        .collect();
    let lights: Vec<Value> = p
        .lights
        .iter()
        .map(|l| {
            json!({
                "id": ids.s(l.id), "name": l.name, "type": match l.kind { LightKind::Point => "point", LightKind::Spot { tilt_deg, .. } if tilt_deg > 135.0 => "uplight", LightKind::Spot { .. } => "spot", LightKind::Area { .. } => "area", LightKind::Surface { .. } => "glow" },
                "pos": l.pos_mm, "attached_to": l.attached_to.map(|a| ids.s(a)),
                "lumen": l.lumen, "kelvin": l.color_temp_k, "on": l.on,
            })
        })
        .collect();
    let assets: Vec<Value> = cx
        .assets
        .iter()
        .map(|a| json!({"id": ids.s(a.id), "name": a.name, "width": a.dims_mm[0], "depth": a.dims_mm[1], "height": a.dims_mm[2], "has_light": a.light.is_some()}))
        .collect();
    let issues: Vec<String> = plan_core::validate::validate(f).into_iter().map(|i| i.message).collect();
    json!({
        "project": p.name,
        "north_direction_in_plan": [r1(north[0]), r1(north[1])],
        "ceiling_height": f.ceiling_height_mm,
        "nodes": f.nodes.iter().map(|n| json!([ids.s(n.id), n.x_mm, n.y_mm])).collect::<Vec<_>>(),
        "walls": walls,
        "openings": openings,
        "rooms": rooms,
        "furniture": furniture,
        "lights": lights,
        "asset_library": assets,
        "materials": cx.materials.iter().map(|(id, name)| json!([id, name])).collect::<Vec<_>>(),
        "issues": issues,
    })
}

// ------------------------------------------------------------------ prompt + schema

pub(crate) const CONVENTIONS: &str = "You are the assistant inside an interior design web app for one private user (Swiss German speaker). \
You get the current apartment model as compact JSON and the user's request (sometimes with photos of the real apartment). \
\n\nModel conventions:\n\
- All lengths in millimetres. Plan coordinates: x to the right, y DOWN (screen). `north_direction_in_plan` is the unit vector pointing north.\n\
- Nodes are [id, x, y]. Walls connect nodes a→b; `left_normal` is the unit vector to the wall's left side (seen from a to b); \
`left_room`/`right_room` are the rooms on each side (null = outside). finish_left/right are the wall surface materials of each side.\n\
- Openings sit on a wall at `offset_from_a` (distance from node a to the opening's near edge).\n\
- Rooms are derived from closed wall loops (ids r1, r2 …). bbox = [min_x, min_y, max_x, max_y] of the net floor.\n\
- Furniture: x/y = footprint centre, rotation_deg = plan rotation (0 = front faces +y/down), width along local x, depth along local y. \
`clearance_to_walls` = free distance from each side to the next wall face.\n\
- Materials: use ONLY ids from `materials` or a colour \"#rrggbb\". Library furniture is placed from `asset_library`; anything else is built from parametric shapes (`add_primitive`).\n\
\n";


/// One action (shared by the single-shot tool and the agent's `apply_actions`).
pub(crate) fn action_schema() -> Value {
    json!({
                    "type": "object",
                    "required": ["type"],
                    "properties": {
                        "type": {"type": "string", "enum": [
                            "move_wall", "set_wall_length", "set_wall", "add_wall", "delete_wall", "move_node",
                            "add_opening", "set_opening", "delete_opening",
                            "set_room", "paint_room_walls",
                            "place_furniture", "add_primitive", "move_furniture", "set_furniture", "delete_furniture",
                            "add_light", "set_light", "delete_light", "set_ceiling_height"
                        ]},
                        "wall": {"type": "string"}, "node": {"type": "string"}, "opening": {"type": "string"},
                        "room": {"type": "string"}, "furniture": {"type": "string"}, "light": {"type": "string"}, "asset": {"type": "string"},
                        "distance_mm": {"type": "number", "description": "move_wall: how far to move the wall"},
                        "away_from_room": {"type": "string", "description": "move_wall: room that grows (wall moves away from it); omit to use dx/dy"},
                        "dx_mm": {"type": "number"}, "dy_mm": {"type": "number"},
                        "x_mm": {"type": "number"}, "y_mm": {"type": "number"}, "z_mm": {"type": "number"},
                        "length_mm": {"type": "number"}, "keep_end": {"type": "string", "enum": ["a", "b"]},
                        "points": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "description": "add_wall: [[x,y],…] chain"},
                        "thickness_mm": {"type": "number"}, "kind": {"type": "string", "enum": ["exterior", "interior"]},
                        "glazed": {"type": "boolean"}, "height_mm": {"type": "number"},
                        "finish_left": {"type": "string"}, "finish_right": {"type": "string"},
                        "opening_type": {"type": "string", "enum": ["door", "window", "passage"]},
                        "offset_mm": {"type": "number"}, "width_mm": {"type": "number"}, "depth_mm": {"type": "number"},
                        "sill_mm": {"type": "number"}, "hinge": {"type": "string", "enum": ["a", "b"]}, "swing": {"type": "string", "enum": ["left", "right"]},
                        "name": {"type": "string"}, "floor_material": {"type": "string"}, "ceiling_material": {"type": "string"},
                        "open_ceiling": {"type": "boolean"}, "material": {"type": "string"},
                        "rotation_deg": {"type": "number"}, "snap_to_wall": {"type": "boolean"},
                        "on": {"type": "boolean"}, "lumen": {"type": "number"}, "kelvin": {"type": "number"},
                        "light_type": {"type": "string", "enum": ["point", "spot"]}, "ceiling_height_mm": {"type": "number"},
                        "shape": {"type": "string", "enum": ["box", "cylinder", "plate", "sofa", "bed", "table", "chair", "shelf", "wardrobe", "podest"], "description": "add_primitive: parametric placeholder shape"}
                    }
                })
}


// ------------------------------------------------------------------ action → ops

type R<T> = Result<T, String>;

pub(crate) fn num(a: &Value, k: &str) -> Option<f64> {
    a.get(k).and_then(|v| v.as_f64()).filter(|v| v.is_finite())
}
pub(crate) fn txt<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}
fn boolean(a: &Value, k: &str) -> Option<bool> {
    a.get(k).and_then(|v| v.as_bool())
}

fn shape(s: &str) -> Option<PrimitiveShape> {
    Some(match s.trim().to_lowercase().as_str() {
        "box" => PrimitiveShape::Box,
        "cylinder" => PrimitiveShape::Cylinder,
        "plate" => PrimitiveShape::Plate,
        "sofa" => PrimitiveShape::Sofa,
        "bed" => PrimitiveShape::Bed,
        "table" => PrimitiveShape::Table,
        "chair" => PrimitiveShape::Chair,
        "shelf" => PrimitiveShape::Shelf,
        "wardrobe" => PrimitiveShape::Wardrobe,
        "podest" | "podium" => PrimitiveShape::Podest,
        _ => return None,
    })
}

fn fmt_m(mm: f64) -> String {
    format!("{:.2} m", mm / 1000.0)
}

pub(crate) struct Tr<'a> {
    pub p: Project,
    pub cx: &'a Ctx,
}

impl Tr<'_> {
    fn f(&self) -> &Floor {
        &self.p.floors[0]
    }
    fn fid(&self) -> Uuid {
        self.p.floors[0].id
    }
    fn id(&self, a: &Value, k: &str) -> R<Uuid> {
        let s = txt(a, k).ok_or_else(|| format!("«{k}» fehlt"))?;
        self.cx.ids.get(s).ok_or_else(|| format!("unbekannte Id «{s}»"))
    }
    fn wall(&self, a: &Value) -> R<Wall> {
        let id = self.id(a, "wall")?;
        self.f().wall(id).cloned().ok_or_else(|| "Wand nicht gefunden".into())
    }
    /// Human-readable name: walls by their adjacent rooms, others by name.
    fn label(&self, id: Uuid) -> String {
        human_label(&self.p, self.cx, id)
    }
    fn material(&self, m: &str) -> R<String> {
        let ok = self.cx.materials.iter().any(|(id, _)| id == m) || (m.len() == 7 && m.starts_with('#') && m[1..].chars().all(|c| c.is_ascii_hexdigit()));
        if ok {
            Ok(m.to_string())
        } else {
            Err(format!("unbekanntes Material «{m}»"))
        }
    }
    fn mat_name(&self, m: &str) -> String {
        self.cx.materials.iter().find(|(id, _)| id == m).map(|(_, n)| n.clone()).unwrap_or_else(|| m.to_string())
    }
    fn push(&mut self, ops: Vec<Op>) -> R<()> {
        apply_batch(&mut self.p, &ops).map(|_| ()).map_err(|(_, e)| format!("{e}"))
    }
    fn room_label(&self, r: &DerivedRoom) -> String {
        room_name(self.f(), r).unwrap_or_else(|| "Raum".into())
    }

    /// Translate one action. Returns a German description of the change.
    pub(crate) fn action(&mut self, a: &Value) -> R<String> {
        let fid = self.fid();
        let ty = txt(a, "type").ok_or("«type» fehlt")?;
        match ty {
            "move_wall" => {
                let w = self.wall(a)?;
                let (pa, pb) = self.f().wall_points(&w).ok_or("Wand ohne Knoten")?;
                let ln = left_normal(norm(sub(pb, pa)));
                let offset = if let (Some(d), Some(r)) = (num(a, "distance_mm"), txt(a, "away_from_room")) {
                    let room = self.cx.room(r).ok_or_else(|| format!("unbekannter Raum «{r}»"))?;
                    let left = side_room(self.f(), &self.cx.rooms, &w, 1.0).map(|i| &self.cx.rooms[i].key);
                    // the room grows → the wall moves away from it, i.e. towards the other side
                    if left == Some(&room.key) {
                        -d
                    } else {
                        d
                    }
                } else {
                    let (dx, dy) = (num(a, "dx_mm").unwrap_or(0.0), num(a, "dy_mm").unwrap_or(0.0));
                    dx * ln[0] + dy * ln[1]
                };
                if offset.abs() < 1.0 {
                    return Err("Verschiebung 0".into());
                }
                let ops = commands::move_wall(&self.p, fid, w.id, offset).map_err(|e| e.to_string())?;
                self.push(ops)?;
                Ok(format!("Wand {} um {} verschieben", self.label(w.id), fmt_m(offset.abs())))
            }
            "set_wall_length" => {
                let w = self.wall(a)?;
                let l = num(a, "length_mm").ok_or("«length_mm» fehlt")?;
                let fixed_a = txt(a, "keep_end") != Some("b");
                let ops = commands::set_wall_length(&self.p, fid, w.id, l.round().max(1.0) as u32, fixed_a).map_err(|e| e.to_string())?;
                self.push(ops)?;
                Ok(format!("Wand {} auf {} Länge", self.label(w.id), fmt_m(l)))
            }
            "set_wall" => {
                let mut w = self.wall(a)?;
                let mut what = Vec::new();
                if let Some(t) = num(a, "thickness_mm") {
                    w.thickness_mm = t.round().clamp(20.0, 1000.0) as u32;
                    what.push(format!("Stärke {} cm", w.thickness_mm / 10));
                }
                if let Some(k) = txt(a, "kind") {
                    w.kind = if k == "exterior" { WallKind::Exterior } else { WallKind::Interior };
                    what.push(if k == "exterior" { "Aussenwand".into() } else { "Innenwand".into() });
                }
                if let Some(g) = boolean(a, "glazed") {
                    w.glazed = g;
                    what.push(if g { "Glas".into() } else { "massiv".into() });
                }
                if let Some(h) = num(a, "height_mm") {
                    let ceil = self.f().ceiling_height_mm as f64;
                    w.height_mm = if h <= 0.0 || h >= ceil { None } else { Some(h.round().max(50.0) as u32) };
                    what.push(format!("Höhe {}", fmt_m(h.min(ceil))));
                }
                if let Some(m) = txt(a, "finish_left") {
                    w.finish_left = self.material(m)?;
                    what.push(format!("links {}", self.mat_name(m)));
                }
                if let Some(m) = txt(a, "finish_right") {
                    w.finish_right = self.material(m)?;
                    what.push(format!("rechts {}", self.mat_name(m)));
                }
                if what.is_empty() {
                    return Err("keine Änderung angegeben".into());
                }
                let id = w.id;
                self.push(vec![Op::PutWall { floor: fid, wall: w }])?;
                Ok(format!("Wand {}: {}", self.label(id), what.join(", ")))
            }
            "add_wall" => {
                let pts: Vec<P> = a
                    .get("points")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect())
                    .unwrap_or_default();
                if pts.len() < 2 {
                    return Err("mindestens zwei Punkte nötig".into());
                }
                let t = num(a, "thickness_mm").unwrap_or(120.0).round().clamp(20.0, 1000.0) as u32;
                let kind = if txt(a, "kind") == Some("exterior") { WallKind::Exterior } else { WallKind::Interior };
                let f = self.f();
                let chain: Vec<ChainPoint> = pts
                    .iter()
                    .map(|&q| {
                        if let Some(n) = f.nodes.iter().find(|n| dist(p2(n), q) <= 30.0) {
                            return ChainPoint { node: Some(n.id), wall: None, x: n.x_mm, y: n.y_mm };
                        }
                        let near = f.walls.iter().filter_map(|w| {
                            let (wa, wb) = f.wall_points(w)?;
                            let (tt, c, d) = closest_on_segment(q, wa, wb);
                            (tt > 0.0 && tt < 1.0 && d <= w.thickness_mm as f64 / 2.0 + 30.0).then_some((d, w.id, c))
                        });
                        match near.min_by(|x, y| x.0.partial_cmp(&y.0).unwrap()) {
                            Some((_, wid, c)) => ChainPoint { node: None, wall: Some(wid), x: c[0].round() as i32, y: c[1].round() as i32 },
                            None => ChainPoint { node: None, wall: None, x: q[0].round() as i32, y: q[1].round() as i32 },
                        }
                    })
                    .collect();
                let ops = commands::add_wall_chain(&self.p, fid, &chain, false, t, kind).map_err(|e| e.to_string())?;
                self.push(ops)?;
                let len: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
                Ok(format!("Neue Wand, {} lang, {} cm stark", fmt_m(len), t / 10))
            }
            "delete_wall" => {
                let w = self.wall(a)?;
                let name = self.label(w.id);
                let ops = commands::delete_wall(&self.p, fid, w.id).map_err(|e| e.to_string())?;
                self.push(ops)?;
                Ok(format!("Wand {name} löschen"))
            }
            "move_node" => {
                let id = self.id(a, "node")?;
                self.f().node(id).ok_or("Knoten nicht gefunden")?;
                let (x, y) = (num(a, "x_mm").ok_or("«x_mm» fehlt")?, num(a, "y_mm").ok_or("«y_mm» fehlt")?);
                self.push(vec![Op::PutNode { floor: fid, node: Node { id, x_mm: x.round() as i32, y_mm: y.round() as i32 } }])?;
                Ok("Ecke verschieben".to_string())
            }
            "add_opening" | "set_opening" => {
                let existing = if ty == "set_opening" {
                    let id = self.id(a, "opening")?;
                    Some(self.f().openings.iter().find(|o| o.id == id).cloned().ok_or("Öffnung nicht gefunden")?)
                } else {
                    None
                };
                let wall = match &existing {
                    Some(o) => self.f().wall(o.wall).cloned().ok_or("Wand nicht gefunden")?,
                    None => self.wall(a)?,
                };
                let (pa, pb) = self.f().wall_points(&wall).ok_or("Wand ohne Knoten")?;
                let wl = dist(pa, pb);
                let tname = txt(a, "opening_type");
                let prev_kind = existing.as_ref().map(|o| o.kind);
                let kind = match (tname, prev_kind) {
                    (Some("window"), Some(k @ OpeningKind::Window { .. })) | (None, Some(k @ OpeningKind::Window { .. })) => match k {
                        OpeningKind::Window { sill_mm } => OpeningKind::Window { sill_mm: num(a, "sill_mm").map_or(sill_mm, |s| s.round().max(0.0) as u32) },
                        _ => unreachable!(),
                    },
                    (Some("window"), _) => OpeningKind::Window { sill_mm: num(a, "sill_mm").unwrap_or(850.0).round().max(0.0) as u32 },
                    (Some("passage"), _) => OpeningKind::Passage,
                    (None, Some(k)) => k,
                    _ => {
                        let (h0, s0, glass) = match prev_kind {
                            Some(OpeningKind::Door { hinge, swing, glass }) => (hinge, swing, glass),
                            _ => (Hinge::Start, Swing::Left, false),
                        };
                        let hinge = match txt(a, "hinge") {
                            Some("a") => Hinge::Start,
                            Some("b") => Hinge::End,
                            _ => h0,
                        };
                        let swing = match txt(a, "swing") {
                            Some("left") => Swing::Left,
                            Some("right") => Swing::Right,
                            _ => s0,
                        };
                        OpeningKind::Door { hinge, swing, glass }
                    }
                };
                let def_w = match kind {
                    OpeningKind::Window { .. } => 1200.0,
                    OpeningKind::Door { .. } => 900.0,
                    OpeningKind::Passage => 1000.0,
                };
                let def_h = match kind {
                    OpeningKind::Window { .. } => 1400.0,
                    _ => 2050.0,
                };
                let width = num(a, "width_mm").or(existing.as_ref().map(|o| o.width_mm as f64)).unwrap_or(def_w).clamp(100.0, wl);
                let height = num(a, "height_mm").or(existing.as_ref().map(|o| o.height_mm as f64)).unwrap_or(def_h).max(100.0);
                let offset = num(a, "offset_mm").or(existing.as_ref().map(|o| o.offset_mm as f64)).unwrap_or((wl - width) / 2.0).clamp(0.0, (wl - width).max(0.0));
                let o = Opening {
                    id: existing.as_ref().map_or_else(Uuid::new_v4, |o| o.id),
                    wall: wall.id,
                    kind,
                    offset_mm: offset.round() as u32,
                    width_mm: width.round() as u32,
                    height_mm: height.round() as u32,
                };
                let name = match kind {
                    OpeningKind::Window { .. } => "Fenster",
                    OpeningKind::Door { .. } => "Tür",
                    OpeningKind::Passage => "Durchgang",
                };
                self.push(vec![Op::PutOpening { floor: fid, opening: o }])?;
                Ok(if existing.is_some() {
                    format!("{name} anpassen ({} breit)", fmt_m(width))
                } else {
                    format!("{name} {} breit in Wand {}", fmt_m(width), self.label(wall.id))
                })
            }
            "delete_opening" => {
                let id = self.id(a, "opening")?;
                let name = self.label(id);
                self.push(vec![Op::DeleteOpening { floor: fid, id }])?;
                Ok(format!("{name} löschen"))
            }
            "set_room" => {
                let r = self.cx.room(txt(a, "room").ok_or("«room» fehlt")?).ok_or("Raum nicht gefunden")?.clone();
                let mut meta = r.meta.and_then(|m| self.f().rooms.iter().find(|x| x.id == m).cloned()).unwrap_or_else(|| RoomMeta {
                    id: Uuid::new_v4(),
                    anchor: [r.label[0].round() as i32, r.label[1].round() as i32],
                    name: "Raum".into(),
                    floor_material: "wood:oak".into(),
                    ceiling_material: "paint:white".into(),
                    open_ceiling: false,
                });
                let mut what = Vec::new();
                if let Some(n) = txt(a, "name") {
                    meta.name = n.to_string();
                    what.push(format!("Name «{n}»"));
                }
                if let Some(m) = txt(a, "floor_material") {
                    meta.floor_material = self.material(m)?;
                    what.push(format!("Boden {}", self.mat_name(m)));
                }
                if let Some(m) = txt(a, "ceiling_material") {
                    meta.ceiling_material = self.material(m)?;
                    what.push(format!("Decke {}", self.mat_name(m)));
                }
                if let Some(o) = boolean(a, "open_ceiling") {
                    meta.open_ceiling = o;
                    what.push(if o { "ohne Decke".into() } else { "mit Decke".into() });
                }
                if what.is_empty() {
                    return Err("keine Änderung angegeben".into());
                }
                let label = room_name(self.f(), &r).unwrap_or_else(|| meta.name.clone());
                self.push(vec![Op::PutRoom { floor: fid, room: meta }])?;
                Ok(format!("{label}: {}", what.join(", ")))
            }
            "paint_room_walls" => {
                let r = self.cx.room(txt(a, "room").ok_or("«room» fehlt")?).ok_or("Raum nicht gefunden")?.clone();
                let m = self.material(txt(a, "material").ok_or("«material» fehlt")?)?;
                let mut ops = Vec::new();
                for wid in &r.walls {
                    let Some(mut w) = self.f().wall(*wid).cloned() else { continue };
                    if w.glazed {
                        continue;
                    }
                    let left = side_room(self.f(), &self.cx.rooms, &w, 1.0).map(|i| &self.cx.rooms[i].key) == Some(&r.key);
                    let right = side_room(self.f(), &self.cx.rooms, &w, -1.0).map(|i| &self.cx.rooms[i].key) == Some(&r.key);
                    if left {
                        w.finish_left = m.clone();
                    }
                    if right {
                        w.finish_right = m.clone();
                    }
                    if left || right {
                        ops.push(Op::PutWall { floor: fid, wall: w });
                    }
                }
                if ops.is_empty() {
                    return Err("keine Wände zum Raum gefunden".into());
                }
                let n = ops.len();
                self.push(ops)?;
                Ok(format!("{}: Wände ({n}) {}", self.room_label(&r), self.mat_name(&m)))
            }
            "place_furniture" => {
                let aid = self.id(a, "asset")?;
                let asset = self.cx.assets.iter().find(|x| x.id == aid).cloned().ok_or("Möbel nicht in der Bibliothek")?;
                let (mut x, mut y) = (num(a, "x_mm").ok_or("«x_mm» fehlt")?, num(a, "y_mm").ok_or("«y_mm» fehlt")?);
                let mut rot = num(a, "rotation_deg").unwrap_or(0.0);
                if boolean(a, "snap_to_wall").unwrap_or(true) {
                    if let Some(s) = snap_to_wall(self.f(), x, y, asset.dims_mm, rot, 400.0) {
                        (x, y, rot) = (s.x_mm as f64, s.y_mm as f64, s.rotation_deg as f64);
                    }
                }
                let id = Uuid::new_v4();
                let mut ops = vec![Op::PutFurniture {
                    item: FurnitureInstance {
                        id,
                        asset: asset.id,
                        name: asset.name.clone(),
                        pos_mm: [x.round() as i32, y.round() as i32, num(a, "z_mm").unwrap_or(0.0).round() as i32],
                        rotation_deg: rot.rem_euclid(360.0) as f32,
                        dims_mm: asset.dims_mm,
                        material: None,
                        walkable: asset.primitive.is_some_and(|s| s.walkable()),
                        overlap_ok: asset.primitive == Some(PrimitiveShape::Chair),
                        footprint: asset.footprint.clone().unwrap_or_default(),
                        passable: false,
                    },
                }];
                if let Some(l) = &asset.light {
                    ops.push(Op::PutLight {
                        light: Light {
                            id: Uuid::new_v4(),
                            name: format!("Licht {}", asset.name),
                            kind: l.kind,
                            pos_mm: l.offset_mm,
                            lumen: l.lumen,
                            color_temp_k: l.color_temp_k,
                            on: true,
                            attached_to: Some(id),
                        },
                    });
                }
                self.push(ops)?;
                Ok(format!("{} platzieren", asset.name))
            }
            "add_primitive" => {
                let sh = shape(txt(a, "shape").ok_or("«shape» fehlt")?).ok_or("unbekannte Form")?;
                let asset = self.cx.assets.iter().find(|x| x.primitive == Some(sh)).cloned().ok_or("Basisform nicht verfügbar")?;
                let mut dims = sh.default_dims();
                for (i, k) in ["width_mm", "depth_mm", "height_mm"].iter().enumerate() {
                    if let Some(v) = num(a, k) {
                        dims[i] = v.round().clamp(1.0, 50_000.0) as u32;
                    }
                }
                let (mut x, mut y) = (num(a, "x_mm").ok_or("«x_mm» fehlt")?, num(a, "y_mm").ok_or("«y_mm» fehlt")?);
                let mut rot = num(a, "rotation_deg").unwrap_or(0.0);
                if boolean(a, "snap_to_wall").unwrap_or(false) {
                    if let Some(s) = snap_to_wall(self.f(), x, y, dims, rot, 400.0) {
                        (x, y, rot) = (s.x_mm as f64, s.y_mm as f64, s.rotation_deg as f64);
                    }
                }
                let material = match txt(a, "material") {
                    Some(m) => Some(self.material(m)?),
                    None => None,
                };
                let name = txt(a, "name").map(str::to_string).unwrap_or_else(|| sh.label_de().to_string());
                self.push(vec![Op::PutFurniture {
                    item: FurnitureInstance {
                        id: Uuid::new_v4(),
                        asset: asset.id,
                        name: name.clone(),
                        pos_mm: [x.round() as i32, y.round() as i32, num(a, "z_mm").unwrap_or(0.0).round().max(0.0) as i32],
                        rotation_deg: rot.rem_euclid(360.0) as f32,
                        dims_mm: dims,
                        material,
                        walkable: sh.walkable(),
                        overlap_ok: sh == PrimitiveShape::Chair,
                        footprint: vec![],
                        passable: false,
                    },
                }])?;
                Ok(format!("{name} ({}×{}×{} cm)", dims[0] / 10, dims[1] / 10, dims[2] / 10))
            }
            "move_furniture" | "set_furniture" => {
                let id = self.id(a, "furniture")?;
                let mut it = self.p.furniture.iter().find(|x| x.id == id).cloned().ok_or("Möbel nicht gefunden")?;
                let old_dims = it.dims_mm;
                if let Some(x) = num(a, "x_mm") {
                    it.pos_mm[0] = x.round() as i32;
                }
                if let Some(y) = num(a, "y_mm") {
                    it.pos_mm[1] = y.round() as i32;
                }
                if let Some(z) = num(a, "z_mm") {
                    it.pos_mm[2] = z.round() as i32;
                }
                if let Some(r) = num(a, "rotation_deg") {
                    it.rotation_deg = r.rem_euclid(360.0) as f32;
                }
                for (i, k) in ["width_mm", "depth_mm", "height_mm"].iter().enumerate() {
                    if let Some(v) = num(a, k) {
                        it.dims_mm[i] = v.round().max(1.0) as u32;
                    }
                }
                if let Some(n) = txt(a, "name") {
                    it.name = n.to_string();
                }
                if let Some(m) = txt(a, "material") {
                    it.material = Some(self.material(m)?);
                }
                if ty == "move_furniture" && boolean(a, "snap_to_wall").unwrap_or(false) {
                    if let Some(s) = snap_to_wall(self.f(), it.pos_mm[0] as f64, it.pos_mm[1] as f64, it.dims_mm, it.rotation_deg as f64, 400.0) {
                        it.pos_mm[0] = s.x_mm;
                        it.pos_mm[1] = s.y_mm;
                        it.rotation_deg = s.rotation_deg;
                    }
                }
                let mut ops = vec![Op::PutFurniture { item: it.clone() }];
                // attached lights scale with the furniture (same as the properties panel)
                for l in self.p.lights.iter().filter(|l| l.attached_to == Some(id)) {
                    let mut l = l.clone();
                    for i in 0..3 {
                        l.pos_mm[i] = (l.pos_mm[i] as f64 * it.dims_mm[i] as f64 / old_dims[i].max(1) as f64).round() as i32;
                    }
                    ops.push(Op::PutLight { light: l });
                }
                self.push(ops)?;
                Ok(format!("{} {}", it.name, if ty == "move_furniture" { "verschieben" } else { "anpassen" }))
            }
            "delete_furniture" => {
                let id = self.id(a, "furniture")?;
                let name = self.p.furniture.iter().find(|x| x.id == id).map(|x| x.name.clone()).unwrap_or_default();
                let ops = commands::delete_furniture(&self.p, id).map_err(|e| e.to_string())?;
                self.push(ops)?;
                Ok(format!("{name} entfernen"))
            }
            "add_light" | "set_light" => {
                let mut l = if ty == "set_light" {
                    let id = self.id(a, "light")?;
                    self.p.lights.iter().find(|x| x.id == id).cloned().ok_or("Lampe nicht gefunden")?
                } else {
                    let ceil = self.f().ceiling_height_mm as f64;
                    Light {
                        id: Uuid::new_v4(),
                        name: "Leuchte".into(),
                        kind: LightKind::Point,
                        pos_mm: [
                            num(a, "x_mm").ok_or("«x_mm» fehlt")?.round() as i32,
                            num(a, "y_mm").ok_or("«y_mm» fehlt")?.round() as i32,
                            num(a, "z_mm").unwrap_or(ceil - 150.0).round() as i32,
                        ],
                        lumen: 800,
                        color_temp_k: 2700,
                        on: true,
                        attached_to: None,
                    }
                };
                if ty == "set_light" {
                    for (i, k) in ["x_mm", "y_mm", "z_mm"].iter().enumerate() {
                        if let Some(v) = num(a, k) {
                            l.pos_mm[i] = v.round() as i32;
                        }
                    }
                }
                if let Some(n) = txt(a, "name") {
                    l.name = n.to_string();
                }
                if let Some(t) = txt(a, "light_type") {
                    l.kind = if t == "spot" { LightKind::Spot { angle_deg: 40.0, tilt_deg: 0.0, heading_deg: 0.0 } } else { LightKind::Point };
                }
                if let Some(v) = boolean(a, "on") {
                    l.on = v;
                }
                if let Some(v) = num(a, "lumen") {
                    l.lumen = v.round().clamp(0.0, 100_000.0) as u32;
                }
                if let Some(v) = num(a, "kelvin") {
                    l.color_temp_k = v.round().clamp(1000.0, 10_000.0) as u16;
                }
                let desc = format!("{} {} ({} lm, {} K{})", if ty == "add_light" { "Neue Leuchte" } else { "Leuchte" }, l.name, l.lumen, l.color_temp_k, if l.on { "" } else { ", aus" });
                self.push(vec![Op::PutLight { light: l }])?;
                Ok(desc)
            }
            "delete_light" => {
                let id = self.id(a, "light")?;
                let name = self.label(id);
                self.push(vec![Op::DeleteLight { id }])?;
                Ok(format!("{name} löschen"))
            }
            "set_ceiling_height" => {
                let h = num(a, "ceiling_height_mm").or(num(a, "height_mm")).ok_or("«ceiling_height_mm» fehlt")?;
                let f = self.f().clone();
                self.push(vec![Op::SetFloor { floor: fid, name: f.name, ceiling_height_mm: h.round().clamp(1800.0, 6000.0) as u32, reference_image: f.reference_image }])?;
                Ok(format!("Raumhöhe {}", fmt_m(h)))
            }
            other => Err(format!("unbekannte Aktion «{other}»")),
        }
    }
}

pub(crate) fn human_label(p: &Project, cx: &Ctx, id: Uuid) -> String {
    let f = &p.floors[0];
    let rname = |i: usize| room_name(f, &cx.rooms[i]).unwrap_or_else(|| "Raum".into());
    if let Some(w) = f.wall(id) {
        let l = side_room(f, &cx.rooms, w, 1.0).map(rname);
        let r = side_room(f, &cx.rooms, w, -1.0).map(rname);
        return match (l, r) {
            (Some(a), Some(b)) if a != b => format!("{a} / {b}"),
            (Some(a), _) | (_, Some(a)) => format!("{a} (aussen)"),
            _ => "freistehend".into(),
        };
    }
    if let Some(o) = f.openings.iter().find(|o| o.id == id) {
        let k = match o.kind {
            OpeningKind::Door { .. } => "Tür",
            OpeningKind::Window { .. } => "Fenster",
            OpeningKind::Passage => "Durchgang",
        };
        return format!("{k} {}", human_label(p, cx, o.wall));
    }
    if let Some(it) = p.furniture.iter().find(|x| x.id == id) {
        return it.name.clone();
    }
    if let Some(l) = p.lights.iter().find(|x| x.id == id) {
        return l.name.clone();
    }
    cx.ids.s(id)
}

/// Short ids from the model → `{kind, id, label}` for highlighting in the client.
pub(crate) fn map_refs(p: &Project, cx: &Ctx, refs: &[Value]) -> Vec<Value> {
    let f = &p.floors[0];
    let mut out = Vec::new();
    for r in refs {
        let Some(s) = r.as_str() else { continue };
        if let Some(room) = cx.room(s) {
            let label = room_name(f, room).unwrap_or_else(|| "Raum".into());
            out.push(json!({"kind": "room", "id": room.meta.map(|m| m.to_string()).unwrap_or_else(|| room.key.clone()), "label": label}));
            continue;
        }
        let Some(id) = cx.ids.get(s) else { continue };
        let kind = if f.walls.iter().any(|w| w.id == id) {
            "wall"
        } else if f.openings.iter().any(|o| o.id == id) {
            "opening"
        } else if f.nodes.iter().any(|n| n.id == id) {
            "node"
        } else if p.furniture.iter().any(|x| x.id == id) {
            "furniture"
        } else if p.lights.iter().any(|x| x.id == id) {
            "light"
        } else {
            continue;
        };
        out.push(json!({"kind": kind, "id": id, "label": human_label(p, cx, id)}));
    }
    out
}

/// Result of a model response, translated.
#[derive(Debug, Default)]
struct Proposal {
    answer: String,
    ops: Vec<Op>,
    changes: Vec<String>,
    warnings: Vec<String>,
    refs: Vec<Value>,
}

fn translate(p: &Project, cx: &Ctx, v: &Value) -> Proposal {
    let mut out = Proposal { answer: txt(v, "answer").unwrap_or("").to_string(), ..Default::default() };
    let mut tr = Tr { p: p.clone(), cx };
    for a in v.get("actions").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
        match tr.action(&a) {
            Ok(d) => out.changes.push(d),
            Err(e) => out.warnings.push(format!("{} übersprungen: {e}", txt(&a, "type").unwrap_or("Aktion"))),
        }
    }
    out.ops = commands::diff(p, &tr.p);
    out.refs = map_refs(&tr.p, cx, v.get("refs").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]));
    out
}

// ------------------------------------------------------------------ HTTP

/// POST /api/projects/{id}/ai (multipart: `text`, `project` (snapshot JSON as the client sees it),
/// `materials` (JSON [[id, name], …]), `history` (optional JSON [{role, text}]), `photo` 0..6 JPEG/PNG)
pub async fn command(State(st): State<AppState>, Path(project_id): Path<Uuid>, mut mp: Multipart) -> ApiResult<axum::response::Response> {
    let llm = st.llm.clone().ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "KI ist nicht konfiguriert (LLM_API_KEY fehlt)"))?;
    let mut text = String::new();
    let mut project: Option<Project> = None;
    let mut materials: Vec<Vec<String>> = Vec::new();
    let mut history: Vec<Value> = Vec::new();
    let mut photos: Vec<(String, Vec<u8>)> = Vec::new();
    let bad = |e: axum::extract::multipart::MultipartError| ApiError::bad_request(format!("Upload fehlerhaft: {e}"));
    while let Some(field) = mp.next_field().await.map_err(bad)? {
        match field.name() {
            Some("text") => text = field.text().await.map_err(bad)?,
            Some("project") => {
                let t = field.text().await.map_err(bad)?;
                project = Some(serde_json::from_str(&t).map_err(|e| ApiError::unprocessable(format!("«project» ungültig: {e}")))?);
            }
            Some("materials") => {
                let t = field.text().await.map_err(bad)?;
                materials = serde_json::from_str(&t).map_err(|_| ApiError::unprocessable("«materials» muss [[id, name, farbe?], …] sein"))?;
            }
            Some("history") => {
                let t = field.text().await.map_err(bad)?;
                history = serde_json::from_str(&t).unwrap_or_default();
            }
            Some("photo") => {
                let bytes = field.bytes().await.map_err(bad)?;
                let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                    "image/png"
                } else if bytes.starts_with(&[0xFF, 0xD8]) {
                    "image/jpeg"
                } else {
                    return Err(ApiError::unprocessable("Fotos bitte als JPG oder PNG"));
                };
                if photos.len() >= MAX_PHOTOS {
                    return Err(ApiError::unprocessable(format!("Höchstens {MAX_PHOTOS} Fotos")));
                }
                photos.push((mime.to_string(), bytes.to_vec()));
            }
            _ => {}
        }
    }
    let text = text.trim().to_string();
    if text.is_empty() && photos.is_empty() {
        return Err(ApiError::unprocessable("Bitte eine Frage oder einen Befehl eingeben"));
    }
    let project = match project {
        Some(p) => p,
        None => crate::projects::load(&st.db, project_id).await?,
    };
    if project.id != project_id || project.floors.is_empty() {
        return Err(ApiError::unprocessable("Projekt passt nicht zur URL"));
    }
    let mut assets: Vec<Asset> = sqlx::query("SELECT data FROM assets ORDER BY created_at DESC")
        .fetch_all(&st.db)
        .await?
        .iter()
        .filter_map(|r| r.try_get::<SqlJson<Asset>, _>("data").ok().map(|j| j.0))
        .collect();
    // Every primitive shape is placeable. Shapes without a library asset get an in-memory one that is only
    // stored if the proposal actually uses it.
    let mut virtual_ids = Vec::new();
    for sh in PrimitiveShape::ALL {
        if !assets.iter().any(|a| a.primitive == Some(sh)) {
            let a = crate::assets::primitive_template(sh);
            virtual_ids.push(a.id);
            assets.push(a);
        }
    }
    let mats: Vec<(String, String)> = materials.iter().filter(|m| m.len() >= 2).map(|m| (m[0].clone(), m[1].clone())).collect();
    let mut cx = build_ctx(&project, assets, mats);
    cx.colors = materials.iter().filter(|m| m.len() >= 3).map(|m| (m[0].clone(), m[2].clone())).collect();
    let state = state_json(&project, &cx);
    let mut user = String::new();
    if !history.is_empty() {
        user.push_str("Bisheriger Verlauf (nur Kontext, bereits erledigt):\n");
        for h in history.iter().rev().take(6).rev() {
            user.push_str(&format!("- {}: {}\n", txt(h, "role").unwrap_or("?"), txt(h, "text").unwrap_or("")));
        }
        user.push('\n');
    }
    user.push_str(&format!("Aktuelles Modell:\n{}\n\nAnfrage: {}", state, if text.is_empty() { "Schlage anhand der Fotos Materialien vor." } else { &text }));
    if !photos.is_empty() {
        user.push_str(&format!("\n\n{} Foto(s) der echten Wohnung angehängt.", photos.len()));
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let run = crate::ai_agent::Run { llm, project: project.clone(), cx, user_text: user, photos };
        match crate::ai_agent::run(run, &tx).await {
            Ok(o) => {
                let ops = commands::diff(&project, &o.work);
                for a in o.cx.assets.iter().filter(|a| virtual_ids.contains(&a.id)) {
                    if ops.iter().any(|op| matches!(op, Op::PutFurniture { item } if item.asset == a.id)) {
                        if let Err(e) = crate::assets::store_asset(&st, a).await {
                            tracing::warn!("Basisform nicht speicherbar: {:?}", e.body);
                        }
                    }
                }
                tracing::info!(ms = started.elapsed().as_millis() as u64, ops = ops.len(), "KI-Agent fertig");
                let _ = tx.send(json!({"type": "result", "answer": o.answer, "ops": ops, "changes": o.changes, "warnings": o.warnings, "refs": o.refs}));
            }
            Err(e) => {
                tracing::warn!("KI-Agent fehlgeschlagen: {e:#}");
                let _ = tx.send(json!({"type": "error", "error": format!("KI-Anfrage fehlgeschlagen: {e:#}")}));
            }
        }
    });
    Ok(crate::ai_agent::ndjson(rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> Project {
        // two rooms side by side: 0..4000 | 4000..7000, height 3000
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = &mut p.floors[0];
        let mut n = |x, y| {
            let id = Uuid::new_v4();
            f.nodes.push(Node { id, x_mm: x, y_mm: y });
            id
        };
        let (a, b, c, d, e, g) = (n(0, 0), n(4000, 0), n(7000, 0), n(7000, 3000), n(4000, 3000), n(0, 3000));
        for (x, y, k) in [(a, b, WallKind::Exterior), (b, c, WallKind::Exterior), (c, d, WallKind::Exterior), (d, e, WallKind::Exterior), (e, g, WallKind::Exterior), (g, a, WallKind::Exterior), (b, e, WallKind::Interior)] {
            f.walls.push(Wall::new(Uuid::new_v4(), x, y, if k == WallKind::Exterior { 300 } else { 100 }, k));
        }
        f.rooms.push(RoomMeta { id: Uuid::new_v4(), anchor: [2000, 1500], name: "Wohnen".into(), floor_material: "wood:oak".into(), ceiling_material: "paint:white".into(), open_ceiling: false });
        p
    }

    fn ctx(p: &Project) -> Ctx {
        build_ctx(p, vec![], vec![("paint:sage".into(), "Salbei".into()), ("tile:grey".into(), "Fliese".into())])
    }

    #[test]
    fn state_has_rooms_and_sides() {
        let p = flat();
        let cx = ctx(&p);
        let s = state_json(&p, &cx);
        assert_eq!(s["rooms"].as_array().unwrap().len(), 2);
        let inner = s["walls"].as_array().unwrap().iter().find(|w| w["kind"] == "interior").unwrap();
        assert!(inner["left_room"].is_string() && inner["right_room"].is_string());
    }

    #[test]
    fn grows_room_by_moving_interior_wall() {
        let p = flat();
        let cx = ctx(&p);
        let s = state_json(&p, &cx);
        let wohnen = s["rooms"].as_array().unwrap().iter().find(|r| r["name"] == "Wohnen").unwrap()["id"].as_str().unwrap().to_string();
        let inner = s["walls"].as_array().unwrap().iter().find(|w| w["kind"] == "interior").unwrap()["id"].as_str().unwrap().to_string();
        let v = json!({"answer": "ok", "actions": [{"type": "move_wall", "wall": inner, "distance_mm": 500, "away_from_room": wohnen}], "refs": [wohnen]});
        let out = translate(&p, &cx, &v);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let mut q = p.clone();
        apply_batch(&mut q, &out.ops).unwrap();
        let w = derive_rooms(&q.floors[0]).into_iter().find(|r| r.meta.is_some()).unwrap();
        // net: (4000 − 150 − 50 + 500) × (3000 − 300)
        assert!((w.area_m2 - 4.3 * 2.7).abs() < 0.05, "{}", w.area_m2);
        assert_eq!(out.refs[0]["kind"], "room");
    }

    #[test]
    fn paints_only_sides_facing_the_room_and_rejects_unknown_material() {
        let p = flat();
        let cx = ctx(&p);
        let v = json!({"answer": "", "actions": [
            {"type": "paint_room_walls", "room": "r1", "material": "paint:sage"},
            {"type": "set_room", "room": "r1", "floor_material": "wood:unobtainium"}
        ]});
        let out = translate(&p, &cx, &v);
        assert_eq!(out.warnings.len(), 1);
        let mut q = p.clone();
        apply_batch(&mut q, &out.ops).unwrap();
        let painted: usize = q.floors[0].walls.iter().map(|w| (w.finish_left == "paint:sage") as usize + (w.finish_right == "paint:sage") as usize).sum();
        assert_eq!(painted, 4, "one side of each of the room's 4 walls");
    }

    #[test]
    fn builds_kitchen_from_primitives() {
        let p = flat();
        let assets = vec![crate::assets::primitive_template(PrimitiveShape::Box), crate::assets::primitive_template(PrimitiveShape::Plate)];
        let cx = build_ctx(&p, assets, vec![("wood:oak-light".into(), "Eiche hell".into())]);
        let v = json!({"answer": "", "actions": [
            {"type": "add_primitive", "shape": "box", "name": "Unterschrank", "width_mm": 2400, "depth_mm": 600, "height_mm": 870, "x_mm": 1400, "y_mm": 450, "rotation_deg": 0, "material": "wood:oak-light"},
            {"type": "add_primitive", "shape": "plate", "name": "Abdeckung", "width_mm": 2400, "depth_mm": 620, "height_mm": 40, "x_mm": 1400, "y_mm": 460, "z_mm": 870},
            {"type": "add_primitive", "shape": "rocket", "x_mm": 0, "y_mm": 0}
        ]});
        let out = translate(&p, &cx, &v);
        assert_eq!(out.changes.len(), 2, "{:?}", out.warnings);
        assert_eq!(out.warnings.len(), 1);
        let mut q = p.clone();
        apply_batch(&mut q, &out.ops).unwrap();
        let top = q.furniture.iter().find(|f| f.name == "Abdeckung").unwrap();
        assert_eq!((top.pos_mm[2], top.dims_mm), (870, [2400, 620, 40]));
    }

    #[test]
    fn adds_door_centred_and_question_without_actions() {
        let p = flat();
        let cx = ctx(&p);
        let inner = cx.ids.s(p.floors[0].walls[6].id);
        let out = translate(&p, &cx, &json!({"answer": "x", "actions": [{"type": "add_opening", "wall": inner, "opening_type": "door", "width_mm": 900}]}));
        let mut q = p.clone();
        apply_batch(&mut q, &out.ops).unwrap();
        let o = &q.floors[0].openings[0];
        assert_eq!((o.offset_mm, o.width_mm), (1050, 900));
        let out = translate(&p, &cx, &json!({"answer": "Ja, 3 m breit.", "actions": []}));
        assert!(out.ops.is_empty() && out.answer.starts_with("Ja"));
    }
}
