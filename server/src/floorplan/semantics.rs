//! LLM vision stage: JSON schema, prompt and lenient parsing of the semantic plan reading.
//! All coordinates are pixels of the image sent to the model; `Semantics::scaled` maps them
//! to the rectified image.

use serde_json::{json, Value};

use super::P;

pub const TOOL_NAME: &str = "report_floor_plan";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Semantics {
    pub rooms: Vec<SemRoom>,
    pub doors: Vec<SemDoor>,
    pub windows: Vec<SemWindow>,
    pub dims: Vec<SemDim>,
    /// Direction of the north arrow, degrees clockwise from image-up.
    pub north_deg: Option<f64>,
    pub scale_text: Option<String>,
    pub walls: Vec<SemWall>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemRoom {
    pub name: String,
    pub pos: P,
    pub area_m2: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemDoor {
    pub pos: P,
    pub width: f64,
    pub hinge: Option<P>,
    pub opens_into: Option<P>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemWindow {
    pub pos: P,
    pub width: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemDim {
    pub value_mm: f64,
    pub a: P,
    pub b: P,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemWall {
    pub a: P,
    pub b: P,
    pub thickness: Option<f64>,
    pub exterior: Option<bool>,
}

pub fn schema() -> Value {
    let pt = |desc: &str| json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": desc});
    json!({
        "type": "object",
        "properties": {
            "rooms": {"type": "array", "description": "One entry per room label on the plan.", "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Room name exactly as written on the plan (German), e.g. 'Wohnen/Essen', 'Zimmer', 'Bad', 'Küche'."},
                    "label": pt("[x, y] pixel position of the centre of the room's name label."),
                    "area_m2": {"type": ["number", "null"], "description": "Area if printed next to the name (e.g. '18.5 m²'), else null."}
                },
                "required": ["name", "label"]
            }},
            "doors": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "center": pt("[x, y] centre of the door opening, on the wall centre line."),
                    "width_px": {"type": "number", "description": "Clear width of the opening along the wall, in pixels."},
                    "hinge": pt("[x, y] position of the hinge (centre of the swing arc), at one end of the opening."),
                    "opens_into": pt("[x, y] a point inside the room the door leaf swings into (on the arc side).")
                },
                "required": ["center", "width_px"]
            }},
            "windows": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "center": pt("[x, y] centre of the window, on the wall centre line."),
                    "width_px": {"type": "number", "description": "Width of the window along the wall, in pixels."}
                },
                "required": ["center", "width_px"]
            }},
            "dimensions": {"type": "array", "description": "Dimension annotations (Masslinien/Kotierungen) with a readable number.", "items": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "The dimension text exactly as printed, e.g. '4.25', '425', '4.25 m'."},
                    "value_mm": {"type": "number", "description": "The value converted to millimetres (Swiss/German plans: '4.25' = metres → 4250; '425' usually centimetres → 4250; '24' with superscript '5' (24⁵) = 24.5 cm → 245)."},
                    "start": pt("[x, y] start point of the measured distance (where the dimension line meets its first extension line / tick)."),
                    "end": pt("[x, y] end point of the measured distance (second extension line / tick).")
                },
                "required": ["value_mm", "start", "end"]
            }},
            "north_arrow_deg": {"type": ["number", "null"], "description": "If a north arrow is drawn: the direction it points to, in degrees clockwise from image-up (0 = up, 90 = right). Else null."},
            "scale_text": {"type": ["string", "null"], "description": "Printed scale such as '1:100', else null."},
            "walls": {"type": "array", "description": "Coarse wall list: centre lines of all walls (exterior and interior). Split walls at corners and T-junctions. Draw walls continuous across door and window openings.", "items": {
                "type": "object",
                "properties": {
                    "start": pt("[x, y] start of the wall centre line."),
                    "end": pt("[x, y] end of the wall centre line."),
                    "thickness_px": {"type": "number"},
                    "exterior": {"type": "boolean"}
                },
                "required": ["start", "end"]
            }}
        },
        "required": ["rooms", "doors", "windows", "dimensions", "walls"]
    })
}

pub const SYSTEM: &str = "You are an expert architectural draughtsman who reads apartment floor plans \
(Swiss/German conventions) and reports their contents precisely as structured data. \
You never guess wildly: omit elements you cannot see. Coordinates are always image pixels.";

pub fn user_prompt(w: u32, h: u32) -> String {
    format!(
        "Analyse this floor plan image. The image is exactly {w} × {h} pixels. Use a pixel coordinate system with origin \
at the top-left corner, x to the right (0..{w}) and y downwards (0..{h}). All positions and lengths you report must be in \
this pixel space; be as precise as you can (walls are the thick dark strokes; thin lines are furniture, dimensions or text).\n\
Report:\n\
1. rooms: every room label, with the room name exactly as written on the plan (keep the German wording, umlauts and \
abbreviations, e.g. 'Wohnen', 'Zi.', 'Bad/WC', 'Reduit') and the pixel position of the label centre.\n\
2. doors: centre of each door opening, its clear width in px, the hinge point (centre of the swing arc) and a point in the \
room the leaf opens into. Sliding doors: omit hinge/opens_into.\n\
3. windows: centre and width in px.\n\
4. dimensions: every readable dimension (Masslinie) with its value converted to millimetres and the exact pixel endpoints of \
the measured distance. Prefer long overall dimensions; include at least the main ones.\n\
5. north_arrow_deg and scale_text if present.\n\
6. walls: a coarse list of wall centre lines with approximate thickness in px and whether each is an exterior wall.\n\
Call the tool {TOOL_NAME} with the result."
    )
}

// ---- lenient parsing --------------------------------------------------------------------------

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().replace(',', ".").parse().ok(),
        _ => None,
    }
    .filter(|x: &f64| x.is_finite())
}

fn point(v: Option<&Value>) -> Option<P> {
    let v = v?;
    match v {
        Value::Array(a) if a.len() >= 2 => Some([num(&a[0])?, num(&a[1])?]),
        Value::Object(o) => Some([num(o.get("x")?)?, num(o.get("y")?)?]),
        _ => None,
    }
}

fn arr<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(|x| x.as_array()).map(|a| a.iter()).into_iter().flatten()
}

pub fn parse(v: &Value) -> Semantics {
    let mut s = Semantics::default();
    for r in arr(v, "rooms") {
        let (Some(name), Some(pos)) = (r.get("name").and_then(|n| n.as_str()), point(r.get("label").or(r.get("position")))) else {
            continue;
        };
        let name = name.trim().replace('ß', "ss");
        if name.is_empty() {
            continue;
        }
        s.rooms.push(SemRoom { name, pos, area_m2: r.get("area_m2").and_then(num) });
    }
    for d in arr(v, "doors") {
        let (Some(pos), Some(width)) = (point(d.get("center")), d.get("width_px").and_then(num)) else { continue };
        if width <= 0.0 {
            continue;
        }
        s.doors.push(SemDoor { pos, width, hinge: point(d.get("hinge")), opens_into: point(d.get("opens_into")) });
    }
    for d in arr(v, "windows") {
        let (Some(pos), Some(width)) = (point(d.get("center")), d.get("width_px").and_then(num)) else { continue };
        if width <= 0.0 {
            continue;
        }
        s.windows.push(SemWindow { pos, width });
    }
    for d in arr(v, "dimensions") {
        let (Some(value_mm), Some(a), Some(b)) = (d.get("value_mm").and_then(num), point(d.get("start")), point(d.get("end"))) else {
            continue;
        };
        if value_mm <= 0.0 || super::dist(a, b) < 5.0 {
            continue;
        }
        let text = d.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
        s.dims.push(SemDim { value_mm, a, b, text });
    }
    for w in arr(v, "walls") {
        let (Some(a), Some(b)) = (point(w.get("start")), point(w.get("end"))) else { continue };
        if super::dist(a, b) < 2.0 {
            continue;
        }
        s.walls.push(SemWall { a, b, thickness: w.get("thickness_px").and_then(num).filter(|t| *t > 0.0), exterior: w.get("exterior").and_then(|e| e.as_bool()) });
    }
    s.north_deg = v.get("north_arrow_deg").and_then(num).map(|d| d.rem_euclid(360.0));
    s.scale_text = v.get("scale_text").and_then(|t| t.as_str()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    s
}

impl Semantics {
    /// Map coordinates from the LLM image (factor = rectified px / llm px).
    pub fn scaled(mut self, f: f64) -> Self {
        let m = |p: &mut P| {
            p[0] *= f;
            p[1] *= f;
        };
        for r in &mut self.rooms {
            m(&mut r.pos);
        }
        for d in &mut self.doors {
            m(&mut d.pos);
            d.width *= f;
            if let Some(h) = &mut d.hinge {
                m(h);
            }
            if let Some(o) = &mut d.opens_into {
                m(o);
            }
        }
        for w in &mut self.windows {
            m(&mut w.pos);
            w.width *= f;
        }
        for d in &mut self.dims {
            m(&mut d.a);
            m(&mut d.b);
        }
        for w in &mut self.walls {
            m(&mut w.a);
            m(&mut w.b);
            if let Some(t) = &mut w.thickness {
                *t *= f;
            }
        }
        self
    }

    /// Apply a per-axis affine correction x' = sx·x + ox, y' = sy·y + oy.
    pub fn map_axes(&mut self, sx: f64, ox: f64, sy: f64, oy: f64) {
        let m = |p: &mut P| {
            p[0] = sx * p[0] + ox;
            p[1] = sy * p[1] + oy;
        };
        for r in &mut self.rooms {
            m(&mut r.pos);
        }
        for d in &mut self.doors {
            m(&mut d.pos);
            if let Some(h) = &mut d.hinge {
                m(h);
            }
            if let Some(o) = &mut d.opens_into {
                m(o);
            }
        }
        for w in &mut self.windows {
            m(&mut w.pos);
        }
        for d in &mut self.dims {
            m(&mut d.a);
            m(&mut d.b);
        }
        for w in &mut self.walls {
            m(&mut w.a);
            m(&mut w.b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lenient() {
        let v = json!({
            "rooms": [{"name": "Wohnen", "label": [100, 200]}, {"name": "", "label": [1, 1]}, {"name": "Bad", "label": {"x": 5, "y": "6"}}],
            "doors": [{"center": [10, 10], "width_px": 40, "hinge": [0, 10], "opens_into": [10, 30]}, {"center": [1, 1]}],
            "windows": [{"center": [50, 0], "width_px": "60"}],
            "dimensions": [{"text": "4.25", "value_mm": 4250, "start": [0, 0], "end": [425, 0]}],
            "north_arrow_deg": -90,
            "scale_text": "1:100",
            "walls": [{"start": [0, 0], "end": [100, 0], "thickness_px": 10, "exterior": true}]
        });
        let s = parse(&v);
        assert_eq!(s.rooms.len(), 2);
        assert_eq!(s.rooms[1].pos, [5.0, 6.0]);
        assert_eq!(s.doors.len(), 1);
        assert_eq!(s.windows[0].width, 60.0);
        assert_eq!(s.dims[0].value_mm, 4250.0);
        assert_eq!(s.north_deg, Some(270.0));
        assert_eq!(s.walls[0].exterior, Some(true));
        let s2 = s.scaled(2.0);
        assert_eq!(s2.dims[0].b, [850.0, 0.0]);
        assert!(schema()["properties"]["walls"].is_object());
    }
}
