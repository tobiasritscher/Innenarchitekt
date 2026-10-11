//! Agentic KI mode: Opus works in several tool-using steps on a working copy of the project –
//! inspect a room's exact geometry, apply actions (with feedback on collisions), look at a wall
//! elevation image, correct, and finally `finish`. The result is still only a proposal (ops diff);
//! progress is streamed to the client as NDJSON events.

use std::collections::HashSet;
use std::convert::Infallible;

use axum::body::{Body, Bytes};
use axum::http::header;
use axum::response::Response;
use image::{Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_line_segment_mut};
use imageproc::rect::Rect;
use plan_core::furniture::{furniture_footprint, overlaps};
use plan_core::geom::*;
use plan_core::walls::footprints;
use plan_core::*;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::ai::{light_json, map_refs, r1, room_name, side_room, txt, Ctx, Tr, CONVENTIONS};
use crate::llm::LlmClient;

const MAX_STEPS: usize = 18;

const AGENT_RULES: &str = "Working mode: you act in steps with tools on a working copy; nothing is saved until the user accepts \
your final proposal.\n\
1. Questions: answer with `finish` (call `inspect_room` first if you need exact geometry). No changes unless asked.\n\
2. Changes: call `inspect_room` for every room you touch (exact inner wall faces, openings, furniture). Then `apply_actions` in \
sensible batches (e.g. base cabinets; worktop + appliances; upper cabinets; table, chairs, lamps). Use the ids returned for new items \
when you correct them later (move_furniture / set_furniture / delete_furniture).\n\
3. After furnishing a wall, call `view_wall` and check the image: gaps, overlaps, wrong heights, blocked doors/windows, items floating. \
Fix what is wrong. Also react to the `issues` returned by apply_actions.\n\
4. When asked to replicate something (e.g. a kitchen from a photo) be detailed and faithful: individual cabinet units (600 mm wide, \
600 deep, 870 high incl. plinth), worktop (plate 40 mm, 620 deep, z 870), sink (plate metal:steel, 2 mm above the worktop), cooktop \
(plate #222222), oven / dishwasher fronts, tall units with fridge, range hood, backsplash (box 10 mm deep), upper cabinets (350 deep, \
z ≈ 1450), handles (thin boxes, metal:steel or metal:black), dining table, chairs, pendant lamp (add_light above the table). \
Match colours/materials of the photo (hex colours allowed for fronts). 20–60 elements are fine.\n\
5. Placement maths for a piece of depth d against a wall face (face p0→p1, into_room normal n): centre = point on the face + n·d/2; \
rotation_deg = atan2(−n.x, n.y) in degrees (front = local +y points into the room). Positions along the face come from the face \
coordinates; keep door and window ranges free unless asked.\n\
6. Lighting: light_type uplight = spot pointing up (indirect light, on the floor or on top of tall furniture, ~600 lm). \
LED strips = plate primitive (height 10 mm, depth 12 mm) with glow (face down under wall cabinets, front on walls), \
~900 lm per metre, 3000 K. Living areas 2700 K. A lamp can be built from primitives: cylinder base + thin cylinder stem + \
cylinder shade with glow {face: all}. Attached lamps: add_light with attach_to and local x/y/z.\n\
7. End with exactly one `finish` call: short German summary for the user (rules below) and refs. Use at most ~15 tool calls.\n\n\
Rules for `answer`: friendly German addressing the user with «du» (Swiss: «ss», never «ß»; metres like 2.40 m). Plain text, no \
Markdown; lines starting with «–» are fine. Never mention internal ids (w14, f3 …) – name things like a person would.\n";

fn tool(name: &str, description: &str, params: Value) -> Value {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": params}})
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "inspect_room",
            "Exact geometry of one room: inner wall faces (coordinates, into-room normal, length, height, surface), openings with their \
             start/end points on the face, furniture and lights in the room.",
            json!({"type": "object", "required": ["room"], "properties": {"room": {"type": "string", "description": "room id, e.g. r3"}}}),
        ),
        tool(
            "apply_actions",
            "Apply actions to the working copy. Returns per-action results, ids of new elements and issues (overlaps, items outside rooms).",
            json!({"type": "object", "required": ["actions"], "properties": {
                "note": {"type": "string", "description": "very short German progress note shown to the user, e.g. «Unterschränke setzen»"},
                "actions": {"type": "array", "items": crate::ai::action_schema()}
            }}),
        ),
        tool(
            "view_wall",
            "Render an elevation of one wall as seen from inside the room (wall surface, openings, furniture in front of it, 0.5 m grid). \
             The image follows as the next user message, together with a legend.",
            json!({"type": "object", "required": ["room", "wall"], "properties": {"room": {"type": "string"}, "wall": {"type": "string"}}}),
        ),
        tool(
            "finish",
            "End the task with the answer for the user.",
            json!({"type": "object", "required": ["answer"], "properties": {
                "answer": {"type": "string"},
                "refs": {"type": "array", "items": {"type": "string"}}
            }}),
        ),
    ]
}

pub(crate) struct Run {
    pub llm: std::sync::Arc<LlmClient>,
    pub project: Project,
    pub cx: Ctx,
    pub user_text: String,
    pub photos: Vec<(String, Vec<u8>)>,
}

pub(crate) struct Outcome {
    pub answer: String,
    pub work: Project,
    pub cx: Ctx,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
    pub refs: Vec<Value>,
}

type Emit = UnboundedSender<Value>;

fn step(tx: &Emit, text: impl Into<String>) {
    let _ = tx.send(json!({"type": "step", "text": text.into()}));
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine;
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

pub(crate) async fn run(r: Run, tx: &Emit) -> anyhow::Result<Outcome> {
    let Run { llm, project, mut cx, user_text, photos } = r;
    let system = format!("{CONVENTIONS}{AGENT_RULES}");
    let mut content = vec![json!({"type": "text", "text": user_text})];
    for (mime, b) in &photos {
        content.push(json!({"type": "image_url", "image_url": {"url": data_url(mime, b)}}));
    }
    let mut messages = vec![json!({"role": "system", "content": system}), json!({"role": "user", "content": content})];
    let tools = tools();
    let mut work = project.clone();
    let mut changes: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for i in 0..MAX_STEPS {
        let msg = llm.chat(&messages, &tools).await?;
        if let Some(t) = msg.get("content").and_then(|c| c.as_str()).map(str::trim).filter(|t| !t.is_empty()) {
            step(tx, t.chars().take(240).collect::<String>());
        }
        let calls = msg.get("tool_calls").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        messages.push(msg.clone());
        if calls.is_empty() {
            let answer = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
            return Ok(Outcome { answer, work, cx, changes, warnings, refs: vec![] });
        }
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        let mut finish: Option<Value> = None;
        for c in &calls {
            let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = c.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
            let args: Value = match c.pointer("/function/arguments") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
                Some(v @ Value::Object(_)) => v.clone(),
                _ => json!({}),
            };
            let result = match name {
                "inspect_room" => {
                    let rid = txt(&args, "room").unwrap_or("");
                    match cx.room(rid) {
                        Some(room) => {
                            step(tx, format!("Raum «{}» vermessen", room_name(&work.floors[0], room).unwrap_or_else(|| "Raum".into())));
                            inspect_room(&work, &cx, rid)
                        }
                        None => json!({"error": format!("unknown room {rid}")}),
                    }
                }
                "apply_actions" => {
                    let acts = args.get("actions").and_then(|a| a.as_array()).cloned().unwrap_or_default();
                    step(tx, txt(&args, "note").map(str::to_string).unwrap_or_else(|| format!("{} Änderungen anwenden", acts.len())));
                    let before: HashSet<Uuid> = work.furniture.iter().map(|f| f.id).chain(work.lights.iter().map(|l| l.id)).collect();
                    let mut results = Vec::new();
                    let mut tr = Tr { p: work.clone(), cx: &cx };
                    for a in &acts {
                        match tr.action(a) {
                            Ok(d) => {
                                changes.push(d.clone());
                                results.push(json!({"ok": true, "change": d}));
                            }
                            Err(e) => results.push(json!({"ok": false, "error": e})),
                        }
                    }
                    work = tr.p;
                    cx.refresh(&work);
                    let new_items: Vec<Value> = work
                        .furniture
                        .iter()
                        .filter(|f| !before.contains(&f.id))
                        .map(|f| json!({"id": cx.ids.s(f.id), "name": f.name}))
                        .chain(work.lights.iter().filter(|l| !before.contains(&l.id)).map(|l| json!({"id": cx.ids.s(l.id), "name": l.name})))
                        .collect();
                    json!({"results": results, "new_items": new_items, "issues": issues(&project, &work, &cx)})
                }
                "view_wall" => {
                    let (rid, wid) = (txt(&args, "room").unwrap_or(""), txt(&args, "wall").unwrap_or(""));
                    match elevation(&work, &cx, rid, wid) {
                        Ok((png, legend, label)) => {
                            step(tx, format!("Wandansicht «{label}» prüfen"));
                            images.push((legend.clone(), png));
                            json!({"ok": true, "legend": legend, "note": "image follows in the next user message"})
                        }
                        Err(e) => json!({"error": e}),
                    }
                }
                "finish" => {
                    finish = Some(args.clone());
                    json!({"ok": true})
                }
                other => json!({"error": format!("unknown tool {other}")}),
            };
            messages.push(json!({"role": "tool", "tool_call_id": id, "content": result.to_string()}));
        }
        if let Some(f) = finish {
            let refs = map_refs(&work, &cx, f.get("refs").and_then(|r| r.as_array()).map(|a| a.as_slice()).unwrap_or(&[]));
            let answer = txt(&f, "answer").unwrap_or("").to_string();
            return Ok(Outcome { answer, work, cx, changes, warnings, refs });
        }
        if !images.is_empty() {
            let mut content = Vec::new();
            for (legend, png) in &images {
                content.push(json!({"type": "text", "text": format!("Wandansicht: {legend}")}));
                content.push(json!({"type": "image_url", "image_url": {"url": data_url("image/png", png)}}));
            }
            messages.push(json!({"role": "user", "content": content}));
        }
        if i + 2 == MAX_STEPS {
            messages.push(json!({"role": "user", "content": "Step limit reached: call `finish` now."}));
        }
    }
    warnings.push("Die KI hat das Schrittlimit erreicht – Vorschlag eventuell unvollständig.".into());
    Ok(Outcome { answer: "Ich bin nicht ganz fertig geworden – hier der bisherige Stand.".into(), work, cx, changes, warnings, refs: vec![] })
}

/// Overlaps and misplaced items among the elements changed in this session.
fn issues(orig: &Project, work: &Project, cx: &Ctx) -> Vec<String> {
    let changed: HashSet<Uuid> = work.furniture.iter().filter(|f| !orig.furniture.contains(f)).map(|f| f.id).collect();
    let mut out = Vec::new();
    let ov: HashSet<Uuid> = overlaps(work).into_iter().collect();
    let rooms = &cx.rooms;
    let ceiling = work.floors[0].ceiling_height_mm as i64;
    for f in work.furniture.iter().filter(|f| changed.contains(&f.id)) {
        let label = format!("{} ({})", f.name, cx.ids.s(f.id));
        if ov.contains(&f.id) {
            // name the partners to make fixing easy
            let fp = furniture_footprint(f);
            let partners: Vec<String> = work
                .furniture
                .iter()
                .filter(|g| g.id != f.id && ov.contains(&g.id) && !f.overlap_ok && !g.overlap_ok)
                .filter(|g| {
                    let (za, zb) = ((f.pos_mm[2], f.pos_mm[2] + f.dims_mm[2] as i32), (g.pos_mm[2], g.pos_mm[2] + g.dims_mm[2] as i32));
                    za.0 < zb.1 - 1 && zb.0 < za.1 - 1 && convex_overlap(&fp, &furniture_footprint(g), 1.0)
                })
                .map(|g| format!("{} ({})", g.name, cx.ids.s(g.id)))
                .collect();
            out.push(if partners.is_empty() { format!("{label} steckt in einer Wand") } else { format!("{label} überschneidet {}", partners.join(", ")) });
        }
        let c = [f.pos_mm[0] as f64, f.pos_mm[1] as f64];
        if !rooms.iter().any(|r| point_in_polygon(c, &r.polygon)) {
            out.push(format!("{label} steht ausserhalb der Räume"));
        }
        if f.pos_mm[2] as i64 + f.dims_mm[2] as i64 > ceiling {
            out.push(format!("{label} ragt über die Decke"));
        }
    }
    out
}

/// Inner face of `w` on the side of room index `ri`: (p0, p1, into-room normal).
fn face_for(f: &Floor, cx: &Ctx, w: &Wall, ri: usize) -> Option<(P, P, P)> {
    let fp = footprints(f).into_iter().find(|fp| fp.wall == w.id)?;
    let (a, b) = f.wall_points(w)?;
    let n = left_normal(norm(sub(b, a)));
    if side_room(f, &cx.rooms, w, 1.0) == Some(ri) {
        Some((fp.left[0], fp.left[1], n))
    } else if side_room(f, &cx.rooms, w, -1.0) == Some(ri) {
        Some((fp.right[0], fp.right[1], mul(n, -1.0)))
    } else {
        None
    }
}

fn pt(p: P) -> Value {
    json!([p[0].round() as i64, p[1].round() as i64])
}

fn inspect_room(p: &Project, cx: &Ctx, rid: &str) -> Value {
    let f = &p.floors[0];
    let Some(ri) = cx.room_ids.iter().position(|r| r == rid) else { return json!({"error": "unknown room"}) };
    let room = &cx.rooms[ri];
    let meta = room.meta.and_then(|m| f.rooms.iter().find(|x| x.id == m));
    let mut walls = Vec::new();
    for wid in &room.walls {
        let Some(w) = f.wall(*wid) else { continue };
        let Some((p0, p1, n)) = face_for(f, cx, w, ri) else { continue };
        let (a, b) = f.wall_points(w).unwrap();
        let d = norm(sub(b, a));
        let half = w.thickness_mm as f64 / 2.0;
        let on_face = |q: P| add(q, mul(n, half));
        let finish = if side_room(f, &cx.rooms, w, 1.0) == Some(ri) { &w.finish_left } else { &w.finish_right };
        let openings: Vec<Value> = f
            .openings_on(w.id)
            .map(|o| {
                let s = on_face(add(a, mul(d, o.offset_mm as f64)));
                let e = on_face(add(a, mul(d, (o.offset_mm + o.width_mm) as f64)));
                let ty = match o.kind {
                    OpeningKind::Door { .. } => "door",
                    OpeningKind::Window { .. } => "window",
                    OpeningKind::Passage => "passage",
                };
                json!({"id": cx.ids.s(o.id), "type": ty, "from": pt(s), "to": pt(e), "width": o.width_mm, "sill": o.kind.sill_mm(), "height": o.height_mm})
            })
            .collect();
        walls.push(json!({
            "id": cx.ids.s(w.id), "kind": if w.kind == WallKind::Exterior { "exterior" } else { "interior" },
            "glazed": w.glazed, "face": [pt(p0), pt(p1)], "into_room": [r1(n[0]), r1(n[1])],
            "length": dist(p0, p1).round(), "height": w.height(f.ceiling_height_mm), "surface": finish,
            "openings": openings,
        }));
    }
    let inside = |x: i32, y: i32| point_in_polygon([x as f64, y as f64], &room.polygon);
    let furniture: Vec<Value> = p
        .furniture
        .iter()
        .filter(|it| inside(it.pos_mm[0], it.pos_mm[1]))
        .map(|it| {
            json!({"id": cx.ids.s(it.id), "name": it.name, "x": it.pos_mm[0], "y": it.pos_mm[1], "z": it.pos_mm[2],
                   "rotation_deg": r1(it.rotation_deg as f64), "width": it.dims_mm[0], "depth": it.dims_mm[1], "height": it.dims_mm[2],
                   "material": it.material})
        })
        .collect();
    let lights: Vec<Value> = p
        .lights
        .iter()
        .filter(|l| match l.attached_to {
            None => inside(l.pos_mm[0], l.pos_mm[1]),
            Some(a) => p.furniture.iter().any(|f| f.id == a && inside(f.pos_mm[0], f.pos_mm[1])),
        })
        .map(|l| light_json(l, &cx.ids))
        .collect();
    json!({
        "room": rid, "name": meta.map(|m| m.name.clone()), "area_m2": r1(room.area_m2),
        "ceiling_height": f.ceiling_height_mm, "open_ceiling": meta.map(|m| m.open_ceiling).unwrap_or(false),
        "floor_material": meta.map(|m| m.floor_material.clone()),
        "inner_polygon": room.inner.iter().map(|q| pt(*q)).collect::<Vec<_>>(),
        "walls": walls, "furniture": furniture, "lights": lights,
    })
}

fn hex(c: &str) -> Option<Rgb<u8>> {
    let c = c.strip_prefix('#')?;
    if c.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(c, 16).ok()?;
    Some(Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
}

fn color_of(cx: &Ctx, m: Option<&str>, fallback: Rgb<u8>) -> Rgb<u8> {
    match m {
        Some(m) if m.starts_with('#') => hex(m).unwrap_or(fallback),
        Some(m) => cx.colors.get(m).and_then(|c| hex(c)).unwrap_or(fallback),
        None => fallback,
    }
}

/// Elevation of a wall seen from inside the room. Returns (PNG, legend, human label).
fn elevation(p: &Project, cx: &Ctx, rid: &str, wid: &str) -> Result<(Vec<u8>, String, String), String> {
    let f = &p.floors[0];
    let ri = cx.room_ids.iter().position(|r| r == rid).ok_or("unknown room")?;
    let wall_id = cx.ids.get(wid).ok_or("unknown wall")?;
    let w = f.wall(wall_id).ok_or("unknown wall")?;
    let (p0, p1, n) = face_for(f, cx, w, ri).ok_or("wall does not bound this room")?;
    // viewer looks along −n; right-hand direction on screen (y down): r = (n.y, −n.x)
    let r = [n[1], -n[0]];
    let (u0, u1) = (dot(p0, r), dot(p1, r));
    let (umin, len) = (u0.min(u1), (u1 - u0).abs());
    let ceil = f.ceiling_height_mm as f64;
    let k = (1100.0 / len.max(ceil)).min(0.25); // px per mm
    let m = 20.0;
    let (wpx, hpx) = ((len * k + 2.0 * m).ceil() as u32, (ceil * k + 2.0 * m).ceil() as u32);
    let mut img = RgbImage::from_pixel(wpx, hpx, Rgb([255, 255, 255]));
    let x = |u: f64| (m + (u - umin) * k) as f32;
    let y = |z: f64| (m + (ceil - z) * k) as f32;
    let rect = |img: &mut RgbImage, ua: f64, ub: f64, za: f64, zb: f64, fill: Rgb<u8>, outline: Option<Rgb<u8>>| {
        let (xa, xb) = (x(ua.min(ub)).round() as i32, x(ua.max(ub)).round() as i32);
        let (ya, yb) = (y(za.max(zb)).round() as i32, y(za.min(zb)).round() as i32);
        let (w, h) = ((xb - xa).max(1) as u32, (yb - ya).max(1) as u32);
        draw_filled_rect_mut(img, Rect::at(xa, ya).of_size(w, h), fill);
        if let Some(o) = outline {
            draw_hollow_rect_mut(img, Rect::at(xa, ya).of_size(w, h), o);
        }
    };
    let finish = if side_room(f, &cx.rooms, w, 1.0) == Some(ri) { &w.finish_left } else { &w.finish_right };
    let wall_h = w.height(f.ceiling_height_mm) as f64;
    rect(&mut img, umin, umin + len, 0.0, wall_h, if w.glazed { Rgb([214, 234, 242]) } else { color_of(cx, Some(finish), Rgb([242, 240, 235])) }, Some(Rgb([90, 90, 90])));
    // grid every 0.5 m
    let mut g = 0.0;
    while g <= len {
        let c = if (g / 1000.0).fract().abs() < 1e-6 { Rgb([160, 160, 160]) } else { Rgb([205, 205, 205]) };
        draw_line_segment_mut(&mut img, (x(umin + g), y(0.0)), (x(umin + g), y(ceil)), c);
        g += 500.0;
    }
    let mut z = 0.0;
    while z <= ceil {
        let c = if (z / 1000.0).fract().abs() < 1e-6 { Rgb([160, 160, 160]) } else { Rgb([205, 205, 205]) };
        draw_line_segment_mut(&mut img, (x(umin), y(z)), (x(umin + len), y(z)), c);
        z += 500.0;
    }
    let mut legend = vec![format!(
        "Wand von innen, links→rechts {:.2} m, Höhe {:.2} m, Raster 0.5 m. u = Abstand vom linken Wandende (mm).",
        len / 1000.0,
        ceil / 1000.0
    )];
    let (a, b) = f.wall_points(w).ok_or("wall")?;
    let d = norm(sub(b, a));
    for o in f.openings_on(w.id) {
        let (ua, ub) = (dot(add(a, mul(d, o.offset_mm as f64)), r), dot(add(a, mul(d, (o.offset_mm + o.width_mm) as f64)), r));
        let sill = o.kind.sill_mm() as f64;
        let (fill, name) = match o.kind {
            OpeningKind::Window { .. } => (Rgb([150, 200, 235]), "Fenster"),
            OpeningKind::Door { .. } => (Rgb([120, 120, 120]), "Tür"),
            OpeningKind::Passage => (Rgb([70, 70, 70]), "Durchgang"),
        };
        rect(&mut img, ua, ub, sill, sill + o.height_mm as f64, fill, Some(Rgb([40, 40, 40])));
        legend.push(format!("{name}: u {:.0}–{:.0}, z {:.0}–{:.0}", ua.min(ub) - umin, ua.max(ub) - umin, sill, sill + o.height_mm as f64));
    }
    // furniture in front of the wall (within 2 m), far ones first
    let mut items: Vec<(f64, &FurnitureInstance, f64, f64)> = p
        .furniture
        .iter()
        .filter_map(|it| {
            let fp = furniture_footprint(it);
            let depths: Vec<f64> = fp.iter().map(|q| dot(sub(*q, p0), n)).collect();
            let near = depths.iter().cloned().fold(f64::MAX, f64::min);
            if near < -50.0 || near > 2000.0 {
                return None;
            }
            let us: Vec<f64> = fp.iter().map(|q| dot(*q, r)).collect();
            let (ua, ub) = (us.iter().cloned().fold(f64::MAX, f64::min), us.iter().cloned().fold(f64::MIN, f64::max));
            (ub > umin + 1.0 && ua < umin + len - 1.0).then_some((near, it, ua, ub))
        })
        .collect();
    items.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap());
    for (near, it, ua, ub) in &items {
        let z0 = it.pos_mm[2] as f64;
        let col = color_of(cx, it.material.as_deref(), Rgb([201, 196, 187]));
        rect(&mut img, ua.max(umin), ub.min(umin + len), z0, z0 + it.dims_mm[2] as f64, col, Some(Rgb([30, 30, 30])));
        legend.push(format!(
            "{} ({}): u {:.0}–{:.0}, z {:.0}–{:.0}, Abstand zur Wand {:.0}",
            it.name,
            cx.ids.s(it.id),
            ua - umin,
            ub - umin,
            z0,
            z0 + it.dims_mm[2] as f64,
            near
        ));
    }
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let label = crate::ai::human_label(p, cx, w.id);
    Ok((png.into_inner(), legend.join("\n"), label))
}

/// NDJSON response body fed by the agent task.
pub(crate) fn ndjson(rx: tokio::sync::mpsc::UnboundedReceiver<Value>) -> Response {
    let stream = UnboundedReceiverStream::new(rx).map(|v| Ok::<_, Infallible>(Bytes::from(format!("{v}\n"))));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kitchen() -> (Project, Ctx) {
        let mut p = Project::new(Uuid::new_v4(), "t");
        let f = &mut p.floors[0];
        let mut n = |x, y| {
            let id = Uuid::new_v4();
            f.nodes.push(Node { id, x_mm: x, y_mm: y });
            id
        };
        let (a, b, c, d) = (n(0, 0), n(4000, 0), n(4000, 3000), n(0, 3000));
        for (x, y) in [(a, b), (b, c), (c, d), (d, a)] {
            f.walls.push(Wall::new(Uuid::new_v4(), x, y, 200, WallKind::Exterior));
        }
        let w0 = f.walls[0].id;
        f.openings.push(Opening { id: Uuid::new_v4(), wall: w0, kind: OpeningKind::Window { sill_mm: 900 }, offset_mm: 2500, width_mm: 1000, height_mm: 1200 });
        let assets = vec![crate::assets::primitive_template(plan_core::primitives::PrimitiveShape::Box)];
        let cx = crate::ai::build_ctx(&p, assets, vec![("wood:oak".into(), "Eiche".into())]);
        (p, cx)
    }

    #[test]
    fn inspect_gives_inner_faces_and_openings() {
        let (p, cx) = kitchen();
        let v = inspect_room(&p, &cx, "r1");
        let walls = v["walls"].as_array().unwrap();
        assert_eq!(walls.len(), 4);
        let north = walls.iter().find(|w| !w["openings"].as_array().unwrap().is_empty()).unwrap();
        // inner face of the north wall at y = 100, normal pointing south (+y)
        assert_eq!(north["face"][0][1], 100);
        assert_eq!(north["into_room"], json!([0.0, 1.0]));
        assert_eq!(north["openings"][0]["from"][0], 2500);
    }

    #[test]
    fn elevation_renders_and_issues_flag_overlap() {
        let (p, mut cx) = kitchen();
        let wid = cx.ids.s(p.floors[0].walls[0].id);
        let mut tr = Tr { p: p.clone(), cx: &cx };
        // two cabinets on top of each other at the north wall
        for x in [600, 900] {
            tr.action(&json!({"type": "add_primitive", "shape": "box", "name": "Schrank", "width_mm": 600, "depth_mm": 600, "height_mm": 870, "x_mm": x, "y_mm": 400}))
                .unwrap();
        }
        let work = tr.p;
        cx.refresh(&work);
        let iss = issues(&p, &work, &cx);
        assert!(iss.iter().any(|i| i.contains("überschneidet")), "{iss:?}");
        let (png, legend, _) = elevation(&work, &cx, "r1", &wid).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        // seen from inside facing north: left end = inner corner at x = 100 → window x 2500–3500 = u 2400–3400
        assert!(legend.contains("Fenster: u 2400–3400"), "{legend}");
        assert_eq!(legend.matches("Schrank").count(), 2);
    }
}
